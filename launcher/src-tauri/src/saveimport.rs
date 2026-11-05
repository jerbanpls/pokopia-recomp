// SPDX-License-Identifier: GPL-3.0-or-later
// Save import: brings a Pokopia save from Eden, Ryujinx or any folder holding the game's save layout into the save the runtime plays.
// The finder and the layout test follow pokopia-native (MIT), nxs2.py eden_saves() / import_save() @7c3655d; the Ryujinx scan, the
// committed-volume placement and the staging/rollback are new.
//
// The runtime keeps an account save at <save_dir>/account/<uid bytes as 32 hex>/ (the COMMITTED folder of a journalled volume,
// runtime/src/services/fs_volume.cpp) with <same>.meta beside it (size/journal/owner/attr, services/fs.cpp). A running game keeps
// work folders (<root>.work.<pid>), journals (<root>.journal.<pid>) and, for a commit in flight, <root>.commit next to it.
// Import: stage a copy of B00/G00/S00 beside the root, move the root and its leftovers to <save_dir>.before-<time>/ (never deleted),
// rename the staged copy into place. The folder is then a plain committed state with no marker: the runtime opens it as is.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

/// Program id of the game (little-endian in the save attribute).
const PROGRAM_ID: u64 = 0x0400_A3A0_1EE4_C000;
const TITLE_DIR: &str = "0400A3A01EE4C000";
/// The runtime's built-in local user (games/pkp/pkp_core.cpp): [account] uid is hi then lo; the save folder is each u64 little-endian.
const DEFAULT_UID: &str = "000000000000000141434e4852656370";
/// What the folder is called for that user.
#[cfg(test)]
const DEFAULT_KEY: &str = "010000000000000070636552484e4341";
const SAVE_PARTS: [&str; 3] = ["B00", "G00", "S00"];
/// Size and journal size the runtime gives the account save (games/pkp/pkp_core.cpp account_save_size).
const ACCOUNT_SAVE_SIZE: u64 = 0x400_0000;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Source {
    /// "Eden", "Ryujinx"
    pub label: String,
    pub path: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Report {
    pub source: String,
    pub files: u64,
    /// Where the previous save went (None when there was none).
    pub backup: Option<String>,
    pub target: String,
}

fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// A folder is a Pokopia save when it holds the game's G00 and S00 folders (nxs2.py import_save).
pub fn is_save_folder(d: &Path) -> bool {
    d.join("G00").is_dir() && d.join("S00").is_dir()
}

/// The game's own file names, to tell its save from another title's in an emulator's folders (nxs2.py eden_saves).
fn has_game_files(d: &Path) -> bool {
    d.join("G00").join("CMNDAT.BIN").is_file() && d.join("S00").join("SYDAT.BIN").is_file()
}

/// A picked folder, or the save/ folder inside it.
pub fn resolve_picked(p: &Path) -> Option<PathBuf> {
    if is_save_folder(p) {
        Some(p.to_path_buf())
    } else if is_save_folder(&p.join("save")) {
        Some(p.join("save"))
    } else {
        None
    }
}

fn newest_mtime(d: &Path) -> SystemTime {
    let mut best = SystemTime::UNIX_EPOCH;
    let mut stack = vec![d.to_path_buf()];
    while let Some(x) = stack.pop() {
        let Ok(rd) = fs::read_dir(&x) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                stack.push(e.path());
            } else if let Ok(m) = md.modified() {
                best = best.max(m);
            }
        }
    }
    best
}

fn subdirs(d: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(d).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    v.sort();
    v
}

/// Eden's saves of the game under <data>/nand/user/save/0000000000000000/<user>/<title id> (or the title-0 folder it uses when it cannot
/// read the title id from an extracted dump). Only read.
pub fn eden_saves(data: &Path) -> Vec<PathBuf> {
    let root = data.join("nand").join("user").join("save").join("0000000000000000");
    let mut out = vec![];
    for user in subdirs(&root) {
        for name in [TITLE_DIR, "0000000000000000"] {
            let d = user.join(name);
            if has_game_files(&d) {
                out.push(d);
            }
        }
    }
    out
}

/// Ryujinx keeps a save at <data>/bis/user/save/<save id>/0 (and a second copy in 1). The save id comes from its imkvdb.arc index, which
/// is not parsed here: every save folder is tested for the game's own files instead (CMNDAT.BIN and SYDAT.BIN do not occur in other titles).
pub fn ryujinx_saves(data: &Path) -> Vec<PathBuf> {
    let root = data.join("bis").join("user").join("save");
    let mut out = vec![];
    for id in subdirs(&root) {
        for slot in ["0", "1"] {
            let d = id.join(slot);
            if has_game_files(&d) {
                out.push(d);
            }
        }
    }
    out
}

/// Everything found automatically, newest first.
pub fn find_sources(appdata: &Path, eden_dir: Option<&Path>, ryujinx_dir: Option<&Path>) -> Vec<Source> {
    let eden = eden_dir.map(Path::to_path_buf).unwrap_or_else(|| appdata.join("eden"));
    let ryu = ryujinx_dir.map(Path::to_path_buf).unwrap_or_else(|| appdata.join("Ryujinx"));
    let mut all: Vec<(SystemTime, Source)> = vec![];
    for (label, list) in [("Eden", eden_saves(&eden)), ("Ryujinx", ryujinx_saves(&ryu))] {
        for d in list {
            all.push((newest_mtime(&d), Source { label: label.into(), path: fwd(&d) }));
        }
    }
    all.sort_by(|a, b| b.0.cmp(&a.0));
    all.into_iter().map(|(_, s)| s).collect()
}

pub fn find_sources_here() -> Vec<Source> {
    let env_dir = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    #[cfg(windows)]
    let (appdata, eden, ryujinx) = (PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()), env_dir("EDEN_DIR"), env_dir("RYUJINX_DIR"));
    // Linux: Eden keeps its data in $XDG_DATA_HOME/eden, Ryujinx in $XDG_CONFIG_HOME/Ryujinx
    #[cfg(not(windows))]
    let (appdata, eden, ryujinx) = {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let data = env_dir("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"));
        let config = env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        (PathBuf::new(), env_dir("EDEN_DIR").or(Some(data.join("eden"))), env_dir("RYUJINX_DIR").or(Some(config.join("Ryujinx"))))
    };
    find_sources(&appdata, eden.as_deref(), ryujinx.as_deref())
}

// ---- where the runtime keeps the save ------------------------------------------------------------------------------------------------

/// The runtime's [fs] save_dir: relative to the game folder; empty is the launcher's default `save`.
pub fn save_dir_of(game_dir: &Path, ini_value: &str) -> PathBuf {
    let v = ini_value.trim();
    if v.is_empty() {
        return game_dir.join("save");
    }
    let p = Path::new(v);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        game_dir.join(p)
    }
}

/// [account] uid (32 hex digits: hi then lo) to the folder name the runtime uses: each u64 as 8 little-endian bytes in hex.
pub fn uid_key(uid_hex: &str) -> Option<String> {
    let h = uid_hex.trim();
    if h.len() != 32 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let le = |s: &str| -> String {
        let v = u64::from_str_radix(s, 16).unwrap_or(0);
        v.to_le_bytes().iter().map(|b| format!("{:02x}", b)).collect()
    };
    Some(format!("{}{}", le(&h[..16]), le(&h[16..])))
}

fn app_hex() -> String {
    PROGRAM_ID.to_le_bytes().iter().map(|b| format!("{:02x}", b)).collect()
}

fn meta_attr_matches(meta: &Path) -> bool {
    let Ok(text) = fs::read_to_string(meta) else { return false };
    let Some(attr) = text.lines().find_map(|l| l.strip_prefix("attr=")) else { return false };
    // attr: application id (8 bytes), uid (16), system id (8), type (1 byte at 32; 1 = account)
    attr.len() >= 66 && attr[..16].eq_ignore_ascii_case(&app_hex()) && &attr[64..66] == "01"
}

/// The account save folder name to import into: the game's existing one (its .meta names the game and the account type), else the
/// folder of [account] uid, else the runtime's built-in user.
pub fn target_key(save_dir: &Path, ini_uid: &str) -> String {
    let acc = save_dir.join("account");
    let mut metas: Vec<PathBuf> = fs::read_dir(&acc)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "meta") && meta_attr_matches(p)).collect())
        .unwrap_or_default();
    metas.sort_by_key(|m| std::cmp::Reverse(fs::metadata(m).and_then(|x| x.modified()).ok()));
    if let Some(stem) = metas.first().and_then(|m| m.file_stem()).and_then(|s| s.to_str()) {
        return stem.to_string();
    }
    uid_key(ini_uid).or_else(|| uid_key(DEFAULT_UID)).unwrap()
}

fn meta_text(key: &str) -> String {
    // attr = app id, uid, system id 0, type 1, rank 0, index 0, padding: 64 bytes, as the runtime's create_save writes it
    let attr = format!("{}{}{}01{}", app_hex(), key, "00".repeat(8), "00".repeat(31));
    format!("size={0}\njournal={0}\nowner={1:x}\nattr={2}\n", ACCOUNT_SAVE_SIZE, PROGRAM_ID, attr)
}

// ---- the import ----------------------------------------------------------------------------------------------------------------------

fn copy_tree(src: &Path, dst: &Path, n: &mut u64) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let t = e.file_type()?;
        let to = dst.join(e.file_name());
        if t.is_dir() {
            copy_tree(&e.path(), &to, n)?;
        } else if t.is_file() {
            fs::copy(e.path(), &to)?;
            *n += 1;
        }
    }
    Ok(())
}

/// UTC time as yyyymmdd-hhmmss (no time-zone crate needed).
fn stamp() -> String {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86400, secs % 86400);
    // civil-from-days (H. Hinnant)
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

fn put_back(old: &[PathBuf], bacc: &Path) {
    for p in old {
        let back = bacc.join(p.file_name().unwrap());
        if back.exists() && !p.exists() {
            let _ = fs::rename(&back, p);
        }
    }
}

/// Brings `src` (a folder holding G00 and S00, optionally B00) in as the game's committed account save. The source is only read. The
/// caller has checked that the game is not running.
pub fn import(save_dir: &Path, key: &str, src: &Path) -> Result<Report, String> {
    if !is_save_folder(src) {
        return Err(format!("{} holds no Pokopia save (no G00 and S00 folders).", fwd(src)));
    }
    let acc = save_dir.join("account");
    let root = acc.join(key);
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if canon(src).starts_with(canon(save_dir)) {
        return Err(format!("{} is inside the save folder the game plays. Pick a copy outside it.", fwd(src)));
    }
    fs::create_dir_all(&acc).map_err(|e| format!("Cannot create {}: {}", fwd(&acc), e))?;
    let t = stamp();

    // 1. stage the copy beside the root; a failure here changes nothing
    let stage = acc.join(format!("{}.import-{}", key, t));
    let _ = fs::remove_dir_all(&stage);
    let mut files = 0u64;
    let staged = (|| -> io::Result<()> {
        fs::create_dir_all(&stage)?;
        for part in SAVE_PARTS {
            if src.join(part).is_dir() {
                copy_tree(&src.join(part), &stage.join(part), &mut files)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = staged {
        let _ = fs::remove_dir_all(&stage);
        return Err(format!("Copying the save failed ({}). Your current save was not touched.", e));
    }
    if files == 0 {
        let _ = fs::remove_dir_all(&stage);
        return Err("The folder holds no save files.".into());
    }

    // 2. move the old save and its commit leftovers (.work.<pid>, .journal.<pid>, .commit) aside; never deleted
    let mut old: Vec<PathBuf> = vec![];
    let stem = format!("{}.", key);
    if let Ok(rd) = fs::read_dir(&acc) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if ["work.", "journal."].iter().any(|k| name.strip_prefix(&stem).map_or(false, |r| r.starts_with(k))) || name == format!("{}commit", stem) {
                old.push(e.path());
            }
        }
    }
    if root.exists() {
        old.push(root.clone());
    }
    let mut backup: Option<PathBuf> = None;
    if !old.is_empty() {
        let base = save_dir.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
        let mut b = PathBuf::from(format!("{}.before-{}", base, t));
        let mut i = 2;
        while b.exists() {
            b = PathBuf::from(format!("{}.before-{}-{}", base, t, i));
            i += 1;
        }
        let bacc = b.join("account");
        let moved = (|| -> io::Result<()> {
            fs::create_dir_all(&bacc)?;
            for p in &old {
                fs::rename(p, bacc.join(p.file_name().unwrap()))?;
            }
            let meta = acc.join(format!("{}.meta", key));
            if meta.is_file() {
                fs::copy(&meta, bacc.join(format!("{}.meta", key)))?;
            }
            Ok(())
        })();
        if let Err(e) = moved {
            put_back(&old, &bacc);
            let _ = fs::remove_dir_all(&stage);
            return Err(format!("Could not move the old save aside ({}). Nothing was changed.", e));
        }
        backup = Some(b);
    }

    // 3. the staged copy becomes the committed folder
    if let Err(e) = fs::rename(&stage, &root) {
        if let Some(b) = &backup {
            put_back(&old, &b.join("account"));
        }
        let _ = fs::remove_dir_all(&stage);
        return Err(format!("Could not put the imported save in place ({}). The old save was put back.", e));
    }
    // 4. the .meta the runtime needs to see the save (an existing one is kept: it carries the game's size and owner)
    let meta = acc.join(format!("{}.meta", key));
    if !meta.is_file() {
        let tmp = acc.join(format!("{}.meta.tmp", key));
        if fs::write(&tmp, meta_text(key)).and_then(|_| fs::rename(&tmp, &meta)).is_err() {
            let _ = fs::remove_file(&tmp);
            return Err(format!("The save was copied to {} but its .meta could not be written. Start the game once to let it create one.", fwd(&root)));
        }
    }
    Ok(Report { source: fwd(src), files, backup: backup.map(|b| fwd(&b)), target: fwd(&root) })
}

/// True while a pkp.exe process runs (the runtime holds the save open and would overwrite the import at its next commit).
#[cfg(windows)]
pub fn game_running() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq pkp.exe", "/NH"])
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("pkp.exe"))
        .unwrap_or(false)
}
/// Linux: a process named pkp.exe (Wine names a Windows program's process after its exe), also one Steam started.
#[cfg(not(windows))]
pub fn game_running() -> bool {
    fs::read_dir("/proc")
        .map(|rd| rd.flatten().any(|e| fs::read_to_string(e.path().join("comm")).map(|c| c.trim() == "pkp.exe").unwrap_or(false)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pkp_saveimp_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_save(d: &Path, tag: &str) {
        for (dir, file) in [("G00", "CMNDAT.BIN"), ("S00", "SYDAT.BIN")] {
            fs::create_dir_all(d.join(dir)).unwrap();
            fs::write(d.join(dir).join(file), tag).unwrap();
        }
    }

    fn cmn(d: &Path) -> String {
        fs::read_to_string(d.join("G00/CMNDAT.BIN")).unwrap()
    }

    #[test]
    fn uid_folder_names() {
        assert_eq!(uid_key(DEFAULT_UID).as_deref(), Some(DEFAULT_KEY));
        assert_eq!(uid_key("short"), None);
        assert_eq!(uid_key("zz000000000000014143 4e4852656370"), None);
    }

    #[test]
    fn eden_and_ryujinx_are_found() {
        let ad = tmp("find");
        make_save(&ad.join("eden/nand/user/save/0000000000000000/USER1").join(TITLE_DIR), "eden");
        make_save(&ad.join("eden/nand/user/save/0000000000000000/USER1/0000000000000000"), "eden0");
        make_save(&ad.join("Ryujinx/bis/user/save/0000000000000007/0"), "ryu");
        // another title's save has other file names
        fs::create_dir_all(ad.join("Ryujinx/bis/user/save/0000000000000008/0/G00")).unwrap();
        fs::write(ad.join("Ryujinx/bis/user/save/0000000000000008/0/G00/other.bin"), "x").unwrap();
        let found = find_sources(&ad, None, None);
        assert_eq!(found.len(), 3);
        assert_eq!(found.iter().filter(|s| s.label == "Ryujinx").count(), 1);
        assert!(find_sources(&ad.join("nowhere"), None, None).is_empty());
        let _ = fs::remove_dir_all(&ad);
    }

    #[test]
    fn newest_save_is_listed_first() {
        let ad = tmp("order");
        let old = ad.join("eden/nand/user/save/0000000000000000/U/0400A3A01EE4C000");
        make_save(&old, "old");
        std::thread::sleep(std::time::Duration::from_millis(50));
        make_save(&ad.join("Ryujinx/bis/user/save/0000000000000001/0"), "new");
        let found = find_sources(&ad, None, None);
        assert_eq!(found[0].label, "Ryujinx");
        let _ = fs::remove_dir_all(&ad);
    }

    #[test]
    fn picked_folder_may_be_the_save_or_hold_one() {
        let d = tmp("pick");
        assert!(resolve_picked(&d).is_none());
        make_save(&d.join("save"), "a");
        assert_eq!(resolve_picked(&d), Some(d.join("save")));
        make_save(&d, "b");
        assert_eq!(resolve_picked(&d), Some(d.clone()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn import_into_an_empty_save_dir_writes_meta() {
        let d = tmp("fresh");
        let src = d.join("src");
        make_save(&src, "new");
        fs::create_dir_all(src.join("B00")).unwrap();
        fs::write(src.join("B00/b.bin"), "b").unwrap();
        fs::write(src.join("junk.txt"), "not a save part").unwrap();
        let sd = d.join("save");
        let key = target_key(&sd, "");
        assert_eq!(key, DEFAULT_KEY);
        let r = import(&sd, &key, &src).unwrap();
        assert_eq!(r.files, 3);
        assert!(r.backup.is_none());
        let root = sd.join("account").join(&key);
        assert_eq!(cmn(&root), "new");
        assert!(!root.join("junk.txt").exists());
        let meta = sd.join("account").join(format!("{}.meta", key));
        assert!(fs::read_to_string(&meta).unwrap().starts_with("size=67108864\njournal=67108864\nowner=400a3a01ee4c000\nattr=00c0e41ea0a30004"));
        assert!(meta_attr_matches(&meta));
        assert_eq!(target_key(&sd, "11111111111111112222222222222222"), key);   // now the existing save names the target
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn import_moves_the_old_save_and_leftovers_aside() {
        let d = tmp("replace");
        let sd = d.join("save");
        let key = target_key(&sd, "");
        let acc = sd.join("account");
        make_save(&acc.join(&key), "old");
        fs::write(acc.join(format!("{}.meta", key)), meta_text(&key)).unwrap();
        fs::create_dir_all(acc.join(format!("{}.work.4242", key)).join("G00")).unwrap();
        fs::write(acc.join(format!("{}.journal.4242", key)), "j").unwrap();
        fs::write(acc.join(format!("{}.commit", key)), "W x\n").unwrap();
        make_save(&acc.join("0300000000000000"), "other");   // another save stays where it is
        let src = d.join("eden");
        make_save(&src, "new");
        let r = import(&sd, &key, &src).unwrap();
        let b = PathBuf::from(r.backup.clone().unwrap());
        assert_eq!(cmn(&b.join("account").join(&key)), "old");
        for n in ["commit", "journal.4242", "meta"] {
            assert!(b.join("account").join(format!("{}.{}", key, n)).is_file(), "{}", n);
        }
        assert!(b.join("account").join(format!("{}.work.4242", key)).is_dir());
        assert_eq!(cmn(&acc.join(&key)), "new");
        assert!(!acc.join(format!("{}.commit", key)).exists());
        assert!(!acc.join(format!("{}.work.4242", key)).exists());
        assert!(!acc.join(format!("{}.journal.4242", key)).exists());
        assert!(acc.join(format!("{}.meta", key)).is_file());
        assert_eq!(cmn(&acc.join("0300000000000000")), "other");
        // a second import in the same second gets its own backup folder; the first backup is untouched
        let r2 = import(&sd, &key, &src).unwrap();
        assert_ne!(r.backup, r2.backup);
        assert_eq!(cmn(&PathBuf::from(r2.backup.unwrap()).join("account").join(&key)), "new");
        assert_eq!(cmn(&b.join("account").join(&key)), "old");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn existing_game_save_names_the_target() {
        let d = tmp("key");
        let acc = d.join("save/account");
        fs::create_dir_all(&acc).unwrap();
        // another title's account save is ignored
        fs::write(acc.join("aa.meta"), format!("attr=ffffffffffffffff{}01\n", "00".repeat(24))).unwrap();
        let k = "1100000000000000".to_string() + "2200000000000000";
        fs::write(acc.join(format!("{}.meta", k)), meta_text(&k)).unwrap();
        assert_eq!(target_key(&d.join("save"), ""), k);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn ini_uid_names_the_target_when_no_save_exists() {
        let d = tmp("uid");
        assert_eq!(target_key(&d, "00000000000000010000000000000002"), "0100000000000000".to_string() + "0200000000000000");
        assert_eq!(target_key(&d, "nonsense"), DEFAULT_KEY);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn refuses_bad_sources_and_leaves_the_save_alone() {
        let d = tmp("bad");
        let sd = d.join("save");
        let key = target_key(&sd, "");
        make_save(&sd.join("account").join(&key), "keep");
        let empty = d.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(import(&sd, &key, &empty).is_err());
        assert!(import(&sd, &key, &sd.join("account").join(&key)).is_err());   // the game's own save
        let hollow = d.join("hollow");   // G00/S00 without files
        fs::create_dir_all(hollow.join("G00")).unwrap();
        fs::create_dir_all(hollow.join("S00")).unwrap();
        assert!(import(&sd, &key, &hollow).is_err());
        assert_eq!(cmn(&sd.join("account").join(&key)), "keep");
        assert_eq!(fs::read_dir(sd.join("account")).unwrap().count(), 1);   // no staging folder left
        assert_eq!(fs::read_dir(&d).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains("before")).count(), 0);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn save_dir_follows_the_ini() {
        let g = Path::new("C:/game");
        assert_eq!(save_dir_of(g, ""), g.join("save"));
        assert_eq!(save_dir_of(g, "saves"), g.join("saves"));
        let abs = if cfg!(windows) { "D:/s" } else { "/mnt/s" };
        assert_eq!(save_dir_of(g, abs), PathBuf::from(abs));
    }
}
