// SPDX-License-Identifier: GPL-3.0-or-later
//! Validates a decrypted game dump folder. Read-only: nothing here writes to the dump.
//!
//! Expected layout: `<dump>/base/{exefs,romfs}` and `<dump>/updates/<version>/{exefs,romfs}`.
//! The highest valid update is used; with none, the base. A folder that holds `exefs/` and `romfs/` directly
//! (an `updates/<version>` picked by hand, or a flattened dump) works too, and so does picking `exefs/` itself.
//! Text-mode FTP copies (CRLF-mangled NSOs) and older game versions get their own message.

use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const EXEFS_FILES: [&str; 5] = ["main", "sdk", "subsdk0", "rtld", "main.npdm"];
/// The NSO modules among them (main.npdm is a different format).
const NSO_FILES: [&str; 4] = ["main", "sdk", "subsdk0", "rtld"];
/// Main build ID (first 16 bytes) of the older game version. Their sources disagree on its number, so player text says "an older version".
const OLDER_MAIN_BUILD: &str = "816c5358e8178d802fcec4a7ce5a741a";
/// A folder every real RomFS has; a copy that stopped early lacks it.
const ROMFS_MARKER_DIR: [&str; 2] = ["motor_asset_re", "KIDS_PLATFORM_OUNCE"];
const TEXT_MODE_LABEL: &str = "copied in text mode";
const OLDER_LABEL: &str = "older version";

#[derive(Serialize, Clone)]
pub struct Check {
    pub label: String,
    pub ok: bool,
}

#[derive(Serialize, Clone)]
pub struct Candidate {
    /// "base", "update", or "folder" (exefs/ and romfs/ directly in the picked folder)
    pub kind: String,
    /// Update version folder name; empty for the base.
    pub version: String,
    pub path: String,
    pub exefs: String,
    pub romfs: String,
    pub ok: bool,
    pub checks: Vec<Check>,
}

#[derive(Serialize, Clone)]
pub struct DumpReport {
    pub path: String,
    pub exists: bool,
    pub errors: Vec<String>,
    pub candidates: Vec<Candidate>,
    pub chosen: Option<Candidate>,
}

pub fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The build this release supports, from `tools/build/dump_hashes.json` next to the launcher: (main build ID, version text).
/// None in a dev checkout without the file: then any version passes here and the build script still checks the hashes.
pub(crate) fn supported() -> Option<(String, String)> {
    let p = crate::exe_dir().join("tools").join("build").join("dump_hashes.json");
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(p).ok()?).ok()?;
    Some((v["main_build_id"].as_str()?.to_lowercase(), v["version"].as_str()?.to_string()))
}

/// Build ID of a decrypted NSO (20 bytes at 0x40), lower-case hex; None if the file is missing, short or not an NSO.
pub(crate) fn main_build_id(main: &Path) -> Option<String> {
    let mut h = [0u8; 0x54];
    fs::File::open(main).ok()?.read_exact(&mut h).ok()?;
    if &h[..4] != b"NSO0" {
        return None;
    }
    Some(h[0x40..0x54].iter().map(|b| format!("{:02x}", b)).collect())
}

/// How many bytes an NSO lacks against the segments its header lists (0 = complete); None if the header cannot be read.
/// From pokopia-native (MIT), nxs2.py nso_short @7c3655d. A text-mode FTP copy drops a byte from every CR LF pair.
fn nso_short(p: &Path) -> Option<u64> {
    let mut h = [0u8; 0x70];
    fs::File::open(p).ok()?.read_exact(&mut h).ok()?;
    if &h[..4] != b"NSO0" {
        return None;
    }
    let u32at = |o: usize| u32::from_le_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]) as u64;
    let flags = u32at(0xC);
    let end = (0..3usize)
        .map(|i| u32at(0x10 + 0x10 * i) + if flags & (1 << i) != 0 { u32at(0x60 + 4 * i) } else { u32at(0x18 + 0x10 * i) })
        .max()
        .unwrap_or(0);
    Some(end.saturating_sub(fs::metadata(p).ok()?.len()))
}

fn dir_has_entries(p: &Path) -> bool {
    fs::read_dir(p).map(|mut rd| rd.next().is_some()).unwrap_or(false)
}

fn check_candidate(kind: &str, version: &str, dir: &Path, want: Option<&(String, String)>) -> Candidate {
    let exefs = dir.join("exefs");
    let romfs = dir.join("romfs");
    let mut checks = Vec::new();
    for f in EXEFS_FILES {
        checks.push(Check { label: format!("exefs/{}", f), ok: exefs.join(f).is_file() });
    }
    checks.push(Check { label: "romfs/".to_string(), ok: romfs.is_dir() });
    if romfs.is_dir() {
        let filled = dir_has_entries(&romfs);
        checks.push(Check { label: "romfs/ is not empty".to_string(), ok: filled });
        if filled {
            let marker = ROMFS_MARKER_DIR.iter().fold(romfs.clone(), |a, b| a.join(b));
            checks.push(Check { label: "romfs/ holds the game data".to_string(), ok: dir_has_entries(&marker) });
        }
    }
    for f in NSO_FILES {
        if let Some(short) = nso_short(&exefs.join(f)) {
            if short > 0 {
                checks.push(Check { label: format!("exefs/{} {} ({} bytes short)", f, TEXT_MODE_LABEL, short), ok: false });
            }
        }
    }
    // the right game version, up front: an older or newer update would otherwise be chosen and fail minutes into Set up
    if let Some((id, text)) = want {
        if exefs.join("main").is_file() {
            let got = main_build_id(&exefs.join("main"));
            let ok = got.as_deref() == Some(id.as_str());
            checks.push(Check { label: format!("version {}", text), ok });
            if !ok && got.as_deref().map_or(false, |g| g.starts_with(OLDER_MAIN_BUILD)) {
                checks.push(Check { label: OLDER_LABEL.to_string(), ok: false });
            }
        }
    }
    let ok = checks.iter().all(|c| c.ok);
    Candidate {
        kind: kind.to_string(),
        version: version.to_string(),
        path: fwd(dir),
        exefs: fwd(&exefs),
        romfs: fwd(&romfs),
        ok,
        checks,
    }
}

/// Newest first: numeric versions by value, then anything non-numeric by name.
fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => y.cmp(&x),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(_), Err(_)) => b.cmp(a),
    }
}

pub fn scan(path: &str) -> DumpReport {
    scan_with(path, supported())
}

fn scan_with(path: &str, want: Option<(String, String)>) -> DumpReport {
    let root = PathBuf::from(path.trim());
    let mut report = DumpReport {
        path: fwd(&root),
        exists: root.is_dir(),
        errors: Vec::new(),
        candidates: Vec::new(),
        chosen: None,
    };
    if path.trim().is_empty() {
        report.errors.push("No folder chosen yet.".to_string());
        return report;
    }
    if !report.exists {
        report.errors.push("That folder does not exist or cannot be read.".to_string());
        return report;
    }
    let base = root.join("base");
    let updates = root.join("updates");
    // exefs/ and romfs/ directly in the picked folder, or the picked folder is exefs/ itself: use its parent
    let direct = if root.join("exefs").is_dir() {
        Some(root.clone())
    } else if root.join("main").is_file() && root.join("main.npdm").is_file() {
        root.parent().filter(|p| p.join("romfs").is_dir()).map(|p| p.to_path_buf())
    } else {
        None
    };
    if !base.is_dir() && !updates.is_dir() && direct.is_none() {
        report.errors.push(
            "This folder has no base/ or updates/ subfolder, and no exefs/ and romfs/ either. Pick the folder that contains them (for example Pokopia_v2.0.1a).".to_string(),
        );
        return report;
    }

    if let Some(d) = &direct {
        report.candidates.push(check_candidate("folder", "", d, want.as_ref()));
    }
    if updates.is_dir() {
        let mut versions: Vec<String> = fs::read_dir(&updates)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        versions.sort_by(|a, b| version_order(a, b));
        for v in versions {
            report.candidates.push(check_candidate("update", &v, &updates.join(&v), want.as_ref()));
        }
    }
    if base.is_dir() {
        report.candidates.push(check_candidate("base", "", &base, want.as_ref()));
    }

    report.chosen = report.candidates.iter().find(|c| c.ok).cloned();
    if report.chosen.is_none() {
        let failed = |pat: &str| report.candidates.iter().any(|c| c.checks.iter().any(|k| k.label.contains(pat) && !k.ok));
        let mut msg = String::new();
        if failed(TEXT_MODE_LABEL) {
            msg = "Some game files are shorter than they should be: the dump was copied in text mode, which damages them. Copy the exefs and romfs folders again in binary mode (FileZilla: Transfer > Transfer type > Binary).".to_string();
        } else if let Some((_, text)) = &want {
            let version = text.split_whitespace().next().unwrap_or(text);
            let update_hint = format!("Update the game on your Switch 2 to {}, then dump again.", version);
            let has_update = report.candidates.iter().any(|c| c.kind == "update");
            if failed(OLDER_LABEL) {
                msg = format!("These game files are from an older version. {}", update_hint);
            } else if failed("version ") && !has_update && direct.is_none() {
                msg = format!("This dump holds the base game only. {}", update_hint);
            } else if failed("version ") {
                msg = format!("These game files are a different version. This release needs {}.", text);
            }
        }
        if msg.is_empty() && failed("romfs/ ") {
            msg = "The romfs folder is empty or incomplete. Dump the game again, or copy the romfs folder again in binary mode.".to_string();
        }
        if msg.is_empty() {
            msg = "No usable game files found. Each of base/ or an updates/<version>/ folder needs exefs/ (main, sdk, subsdk0, rtld, main.npdm) and romfs/.".to_string();
        }
        report.errors.push(msg);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_id_of_an_nso_header() {
        let dir = std::env::temp_dir().join(format!("pkp_dump_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut h = vec![0u8; 0x60];
        h[..4].copy_from_slice(b"NSO0");
        h[0x40] = 0x76;
        h[0x53] = 0x0e;
        fs::write(dir.join("main"), &h).unwrap();
        assert_eq!(main_build_id(&dir.join("main")).unwrap(), format!("76{}0e", "00".repeat(18)));
        h[0] = b'X';
        fs::write(dir.join("main"), &h).unwrap();
        assert_eq!(main_build_id(&dir.join("main")), None);   // encrypted or not an NSO
        fs::write(dir.join("main"), b"NSO0").unwrap();
        assert_eq!(main_build_id(&dir.join("main")), None);   // short
        assert_eq!(main_build_id(&dir.join("nope")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    fn nso(total: usize, build: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; total.max(0x70)];
        h[..4].copy_from_slice(b"NSO0");
        for i in 0..3usize {
            let off = 0x100 + 0x20 * i as u32;
            h[0x10 + 0x10 * i..0x14 + 0x10 * i].copy_from_slice(&off.to_le_bytes());
            h[0x18 + 0x10 * i..0x1c + 0x10 * i].copy_from_slice(&0x20u32.to_le_bytes());
        }
        h[0x40..0x40 + build.len()].copy_from_slice(build);
        h.resize(total, 0);
        h
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pkp_dump_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_set(dir: &Path, build: &[u8], nso_len: usize, romfs: bool) {
        let ex = dir.join("exefs");
        fs::create_dir_all(&ex).unwrap();
        for f in NSO_FILES {
            fs::write(ex.join(f), nso(nso_len, build)).unwrap();
        }
        fs::write(ex.join("main.npdm"), b"META").unwrap();
        let ro = dir.join("romfs");
        fs::create_dir_all(&ro).unwrap();
        if romfs {
            let m = ROMFS_MARKER_DIR.iter().fold(ro.clone(), |a, b| a.join(b));
            fs::create_dir_all(&m).unwrap();
            fs::write(m.join("x.fdata"), b"1").unwrap();
        }
    }

    fn want() -> Option<(String, String)> {
        Some(("11".repeat(20), "2.0.1 (update 655360)".to_string()))
    }

    #[test]
    fn nso_short_counts_missing_bytes() {
        let d = tmp("short");
        fs::write(d.join("a"), nso(0x160, &[1])).unwrap();
        assert_eq!(nso_short(&d.join("a")), Some(0));
        fs::write(d.join("b"), nso(0x150, &[1])).unwrap();
        assert_eq!(nso_short(&d.join("b")), Some(0x10));
        fs::write(d.join("c"), b"nope").unwrap();
        assert_eq!(nso_short(&d.join("c")), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn direct_folder_and_picked_exefs_are_accepted() {
        let d = tmp("direct");
        make_set(&d, &[0x11; 20], 0x160, true);
        let r = scan_with(&d.to_string_lossy(), want());
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.chosen.unwrap().kind, "folder");
        let r = scan_with(&d.join("exefs").to_string_lossy(), want());
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn text_mode_copy_is_named() {
        let d = tmp("text");
        make_set(&d, &[0x11; 20], 0x160, true);
        fs::write(d.join("exefs").join("sdk"), nso(0x15c, &[0x11; 20])).unwrap();
        let r = scan_with(&d.to_string_lossy(), want());
        assert!(r.chosen.is_none());
        assert!(r.errors[0].contains("text mode") && r.errors[0].contains("binary mode"), "{:?}", r.errors);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn empty_or_unmarked_romfs_fails() {
        let d = tmp("romfs");
        make_set(&d, &[0x11; 20], 0x160, false);
        let r = scan_with(&d.to_string_lossy(), want());
        assert!(r.chosen.is_none());
        assert!(r.errors[0].contains("romfs"), "{:?}", r.errors);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn older_version_and_base_only_say_update() {
        let older: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&OLDER_MAIN_BUILD[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let d = tmp("older");
        make_set(&d.join("updates").join("327680"), &older, 0x160, true);
        let r = scan_with(&d.to_string_lossy(), want());
        assert!(r.errors[0].contains("older version") && r.errors[0].contains("2.0.1") && r.errors[0].contains("dump again"), "{:?}", r.errors);
        assert!(!r.errors[0].contains("2.0.0") && !r.errors[0].contains("1.0."), "{:?}", r.errors);
        let _ = fs::remove_dir_all(&d);

        let d = tmp("baseonly");
        make_set(&d.join("base"), &[0x22; 20], 0x160, true);
        let r = scan_with(&d.to_string_lossy(), want());
        assert!(r.errors[0].contains("base game only") && r.errors[0].contains("2.0.1"), "{:?}", r.errors);
        let _ = fs::remove_dir_all(&d);

        let d = tmp("other");
        make_set(&d.join("updates").join("9"), &[0x22; 20], 0x160, true);
        let r = scan_with(&d.to_string_lossy(), want());
        assert!(r.errors[0].contains("different version"), "{:?}", r.errors);
        let _ = fs::remove_dir_all(&d);
    }
}
