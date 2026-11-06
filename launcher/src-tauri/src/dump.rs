// SPDX-License-Identifier: GPL-3.0-or-later
//! Validates a decrypted game dump folder. Read-only: nothing here writes to the dump.
//!
//! Expected layout: `<dump>/base/{exefs,romfs}` and `<dump>/updates/<version>/{exefs,romfs}`.
//! The highest valid update is used; with none, the base. A folder that holds `exefs/` and `romfs/` directly
//! (an `updates/<version>` picked by hand, or a flattened dump) works too, and so does picking `exefs/` itself.
//! Text-mode FTP copies (CRLF-mangled NSOs) and older game versions get their own message.
//! The romfs must hold at least the files and bytes of the supported version (`dump_hashes.json` "romfs"): an update dumped without
//! the base game's files under it has the right exefs and only the files the update changes (fork #128: 22 of 36 files), and the game
//! then crashes at start (code 4). When the dump also has base/romfs and the two together are complete, the base is layered under the
//! update (`romfs_base`, served by pkp's `[fs] romfs_base`), as the .nup unpacker does.

use serde::Serialize;
use std::fs;
use std::collections::BTreeMap;
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
const INCOMPLETE_LABEL: &str = "romfs/ is complete";

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
    /// The base game's romfs to layer under `romfs` (its files fill in what the update lacks); empty: none needed.
    pub romfs_base: String,
    /// Files and bytes in the romfs as it will be served (with romfs_base under it when that is set).
    pub romfs_files: u64,
    pub romfs_bytes: u64,
    pub ok: bool,
    pub checks: Vec<Check>,
}

/// The least a complete romfs of the supported version holds (extra files such as desktop.ini pass).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RomfsNeed {
    pub files: u64,
    pub bytes: u64,
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

/// The romfs size of the supported version, from `dump_hashes.json` "romfs" {files, bytes}; None without it (no check).
pub(crate) fn supported_romfs() -> Option<RomfsNeed> {
    let p = std::env::current_exe().ok()?.parent()?.join("tools").join("build").join("dump_hashes.json");
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(p).ok()?).ok()?;
    Some(RomfsNeed { files: v["romfs"]["files"].as_u64()?, bytes: v["romfs"]["bytes"].as_u64()? })
}

/// Every file below `root` as (path relative to it with '/', size). Unreadable folders count as empty.
fn romfs_files(root: &Path) -> BTreeMap<String, u64> {
    fn walk(dir: &Path, prefix: &str, depth: u32, out: &mut BTreeMap<String, u64>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() && depth < 32 {
                walk(&e.path(), &format!("{}{}/", prefix, name), depth + 1, out);
            } else if ft.is_file() {
                out.insert(format!("{}{}", prefix, name), e.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, "", 0, &mut out);
    out
}

fn totals(files: &BTreeMap<String, u64>) -> (u64, u64) {
    (files.len() as u64, files.values().sum())
}

fn gb(b: u64) -> f64 {
    b as f64 / 1e9
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

/// `base_romfs`: the base game's romfs of the same dump (for an update), layered under this romfs when this one alone is incomplete.
fn check_candidate(kind: &str, version: &str, dir: &Path, want: Option<&(String, String)>, need: Option<RomfsNeed>, base_romfs: Option<&Path>) -> Candidate {
    let exefs = dir.join("exefs");
    let romfs = dir.join("romfs");
    let mut romfs_base = String::new();
    let (mut romfs_files_n, mut romfs_bytes_n) = (0u64, 0u64);
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
        let mut files = romfs_files(&romfs);
        (romfs_files_n, romfs_bytes_n) = totals(&files);
        if let Some(n) = need {
            let complete = |f: u64, b: u64| f >= n.files && b >= n.bytes;
            let mut ok = complete(romfs_files_n, romfs_bytes_n);
            if !ok {
                // an update that only holds the files it changes: the base game's romfs underneath (the update's files win)
                if let Some(b) = base_romfs.filter(|b| b.is_dir()) {
                    for (rel, size) in romfs_files(b) {
                        files.entry(rel).or_insert(size);
                    }
                    let (f, by) = totals(&files);
                    if complete(f, by) {
                        ok = true;
                        romfs_base = fwd(b);
                        (romfs_files_n, romfs_bytes_n) = (f, by);
                        checks.push(Check { label: format!("romfs/ completed from {}", fwd(b)), ok: true });
                    }
                }
            }
            checks.push(Check {
                label: if ok {
                    INCOMPLETE_LABEL.to_string()
                } else {
                    format!("{} ({} of {} files, {:.2} of {:.2} GB)", INCOMPLETE_LABEL, romfs_files_n, n.files, gb(romfs_bytes_n), gb(n.bytes))
                },
                ok,
            });
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
        romfs_base,
        romfs_files: romfs_files_n,
        romfs_bytes: romfs_bytes_n,
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
    scan_need(path, supported(), supported_romfs())
}

fn scan_need(path: &str, want: Option<(String, String)>, need: Option<RomfsNeed>) -> DumpReport {
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

    let base_romfs = base.join("romfs");
    if let Some(d) = &direct {
        // updates/<version> picked by hand: the base game is at ../../base
        let up = d.parent().filter(|p| p.file_name().map_or(false, |n| n.eq_ignore_ascii_case("updates")));
        let near = up.and_then(|p| p.parent()).map(|p| p.join("base").join("romfs"));
        report.candidates.push(check_candidate("folder", "", d, want.as_ref(), need, near.as_deref()));
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
            report.candidates.push(check_candidate("update", &v, &updates.join(&v), want.as_ref(), need, Some(&base_romfs)));
        }
    }
    if base.is_dir() {
        report.candidates.push(check_candidate("base", "", &base, want.as_ref(), need, None));
    }

    report.chosen = report.candidates.iter().find(|c| c.ok).cloned();
    if report.chosen.is_none() {
        let failed = |pat: &str| report.candidates.iter().any(|c| c.checks.iter().any(|k| k.label.contains(pat) && !k.ok));
        let mut msg = String::new();
        let bad = |c: &Candidate, pat: &str| c.checks.iter().any(|k| k.label.starts_with(pat) && !k.ok);
        // the right version with game data missing: said before the version messages (the base of a base+update dump never matches)
        let incomplete = report.candidates.iter().find(|c| bad(c, INCOMPLETE_LABEL) && !bad(c, "version ") && !c.checks.iter().any(|k| k.label.contains(TEXT_MODE_LABEL)));
        if let (Some(n), Some(c)) = (need, incomplete) {
            msg = format!(
                "The game data (romfs) in {} is incomplete: {} of {} files, {:.2} of {:.2} GB. This happens when the update is dumped without the \
                 base game's files under it (an update only holds the files it changes), or when copying the romfs folder stopped early. Put the \
                 base game (base/ with exefs and romfs) next to updates/ and pick the folder that holds both: the launcher then adds the missing \
                 files from it. Otherwise dump the game again with the update merged into the base game.",
                c.romfs, c.romfs_files, n.files, gb(c.romfs_bytes), gb(n.bytes)
            );
        } else if failed(TEXT_MODE_LABEL) {
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
