// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads and edits the runtime's ini files next to pkp.exe. The key schema lives in the frontend;
//! this side only moves `section.key` values in and out, with the careful editor in ini.rs.

use crate::ini::Ini;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const PKP_INI: &str = "pkp.ini";
pub const CONTROLS_INI: &str = "controls.ini";

#[derive(Serialize)]
pub struct SettingsDump {
    pub game_dir: String,
    pub pkp_exists: bool,
    pub controls_exists: bool,
    pub pkp: HashMap<String, String>,
    pub controls: HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct Edit {
    /// "pkp" or "controls"
    pub file: String,
    pub section: String,
    pub key: String,
    pub value: String,
}

fn load(path: &Path) -> Option<Ini> {
    let bytes = fs::read(path).ok()?;
    // the files are UTF-8 (controls.ini has a non-ASCII header); a stray bad byte must not lose the file
    Some(Ini::parse(&String::from_utf8_lossy(&bytes)))
}

fn map_of(ini: &Option<Ini>) -> HashMap<String, String> {
    ini.as_ref().map(|i| i.all().into_iter().collect()).unwrap_or_default()
}

pub fn file_for(dir: &Path, which: &str) -> Result<PathBuf, String> {
    match which {
        "pkp" => Ok(dir.join(PKP_INI)),
        "controls" => Ok(dir.join(CONTROLS_INI)),
        other => Err(format!("unknown settings file '{}'", other)),
    }
}

pub fn read(dir: &Path) -> SettingsDump {
    let pkp = load(&dir.join(PKP_INI));
    let controls = load(&dir.join(CONTROLS_INI));
    SettingsDump {
        game_dir: crate::dump::fwd(dir),
        pkp_exists: pkp.is_some(),
        controls_exists: controls.is_some(),
        pkp: map_of(&pkp),
        controls: map_of(&controls),
    }
}

pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("cannot replace {}: {}", path.display(), e)
    })
}

/// Applies edits grouped per file. controls.ini is never created: the game writes it with its factory
/// bindings on first run, and a file with only a few keys would lose them.
pub fn apply(dir: &Path, edits: &[Edit]) -> Result<(), String> {
    for which in ["pkp", "controls"] {
        let mine: Vec<&Edit> = edits.iter().filter(|e| e.file == which).collect();
        if mine.is_empty() {
            continue;
        }
        let path = file_for(dir, which)?;
        let mut ini = match load(&path) {
            Some(i) => i,
            None if which == "pkp" => Ini::empty(),
            None => return Err(format!("{} does not exist yet. Start the game once so it creates it.", CONTROLS_INI)),
        };
        for e in mine {
            ini.set(&e.section, &e.key, &e.value);
        }
        write_atomic(&path, &ini.to_text())?;
    }
    Ok(())
}

/// Used by Play and setup: the ini keys the launcher owns. pkp.exe reads every path from pkp.ini (a pkp.ini next to it makes
/// the game folder its working directory), so it needs no command-line paths.
/// A path for pkp.ini: relative to the game folder (forward slashes) when it lies inside it, so the folder can be moved or copied to
/// another PC or to SteamOS/Proton with its dump and saves; absolute otherwise. The game resolves relative paths against its folder.
pub fn portable(dir: &Path, p: &str) -> String {
    let full = Path::new(p.trim());
    match (fs::canonicalize(dir), fs::canonicalize(full)) {
        (Ok(d), Ok(f)) => match f.strip_prefix(&d) {
            Ok(rel) if !rel.as_os_str().is_empty() => crate::dump::fwd(rel),
            _ => p.to_string(),
        },
        _ => p.to_string(),
    }
}

/// `romfs_base`: the base game's romfs to layer under `romfs` ([fs] romfs_base; empty: none). It follows the launcher's choice while
/// [fs] romfs is the launcher's own value; a romfs the user pointed elsewhere keeps the romfs_base the user set with it.
pub fn ensure_play_paths(dir: &Path, exefs: &str, romfs: &str, romfs_base: &str) -> Result<(), String> {
    let path = dir.join(PKP_INI);
    let mut ini = load(&path).unwrap_or_else(Ini::empty);
    let mut changed = !path.exists();
    let resolve = |v: &str| {
        let p = Path::new(v.trim());
        let full = if p.is_absolute() { p.to_path_buf() } else { dir.join(p) };
        fs::canonicalize(&full).unwrap_or(full)
    };
    let romfs_was_ours = ini.get("fs", "romfs").map_or(true, |cur| cur.trim().is_empty() || resolve(&cur) == resolve(romfs));

    // [fs] exefs and [fs] romfs: kept while they name an existing folder or .nup (the user may have pointed them elsewhere)
    for (key, value) in [("exefs", exefs), ("romfs", romfs)] {
        let cur = ini.get("fs", key).unwrap_or_default();
        let cur_path = Path::new(cur.trim());
        // relative paths in the ini are relative to the game folder, the working directory of the game
        let cur_full = if cur_path.is_absolute() { cur_path.to_path_buf() } else { dir.join(cur_path) };
        if cur.trim().is_empty() || !(cur_full.is_dir() || cur_full.is_file()) {
            ini.set("fs", key, &portable(dir, value));
            changed = true;
        }
    }
    let romfs_is_ours = ini.get("fs", "romfs").map_or(false, |cur| resolve(&cur) == resolve(romfs));
    if romfs_was_ours && romfs_is_ours {
        let want = if romfs_base.is_empty() { String::new() } else { portable(dir, romfs_base) };
        if ini.get("fs", "romfs_base").unwrap_or_default().trim() != want {
            ini.set("fs", "romfs_base", &want);
            changed = true;
        }
    }
    if ini.get("nvn", "shader_cache").unwrap_or_default().trim() != crate::shaders::SPIRV_REL {
        ini.set("nvn", "shader_cache", crate::shaders::SPIRV_REL);
        changed = true;
    }
    // shaders the game builds while it runs, for play.rs to translate after it exits (kept if the user set it, even empty)
    if ini.get("nvn", "dump_missing").is_none() {
        ini.set("nvn", "dump_missing", crate::shaders::MISSING_REL);
        changed = true;
    }
    // Linux: nothing write-watches the game's memory. Under Proton the pools are mapped into the GPU driver, which maps them
    // again after every write the page protection catches: 60-180 ms stalls with these on (kept if the user set them; on
    // Linux pool_mirror here outranks the F1 page's "Faster GPU memory")
    #[cfg(not(windows))]
    for (key, value) in [("pool_mirror", "off"), ("tex_write_watch", "false")] {
        if ini.get("nvn", key).is_none() {
            ini.set("nvn", key, value);
            changed = true;
        }
    }
    let cur_save = ini.get("fs", "save_dir").unwrap_or_default();
    if cur_save.trim().is_empty() {
        let _ = fs::create_dir_all(dir.join("save"));
        ini.set("fs", "save_dir", "save");   // relative: the saves move with the folder
        changed = true;
    }
    if changed {
        write_atomic(&path, &ini.to_text())?;
    }
    Ok(())
}

#[cfg(all(test, not(windows)))]
mod linux_tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn linux_play_paths_turn_off_write_watching_once() {
        let dir = std::env::temp_dir().join(format!("pkp_ww_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let d = crate::dump::fwd(&dir);
        ensure_play_paths(&dir, &d, &d, "").unwrap();
        let ini = load(&dir.join(PKP_INI)).unwrap();
        assert_eq!((ini.get("nvn", "pool_mirror").as_deref(), ini.get("nvn", "tex_write_watch").as_deref()), (Some("off"), Some("false")));
        // the user turned the mirror back on: kept
        let mut ini = load(&dir.join(PKP_INI)).unwrap();
        ini.set("nvn", "pool_mirror", "on");
        write_atomic(&dir.join(PKP_INI), &ini.to_text()).unwrap();
        ensure_play_paths(&dir, &d, &d, "").unwrap();
        assert_eq!(load(&dir.join(PKP_INI)).unwrap().get("nvn", "pool_mirror").as_deref(), Some("on"));
        let _ = fs::remove_dir_all(&dir);
    }
}
