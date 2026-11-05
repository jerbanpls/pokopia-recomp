// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads a `.nup` package (https://github.com/a55uka/nup, format 1) and unpacks it into the folder layout the rest
//! of the launcher expects: `<out>/base/{exefs,romfs}` and `<out>/updates/<version>/{exefs,romfs}`.
//! Add-on (DLC) packages are skipped. Only the exefs and romfs subtrees of each package are written.
//!
//! Order of work, so a wrong or damaged package fails in seconds and not after 7 GB: header and table of contents
//! (size, SHA-256 of the TOC, every record's bounds); then every program's ExeFS (small) and the main build ID check; then
//! the free-space check; then the RomFS of the chosen program only. A non-standalone update gets the base's RomFS files
//! underneath its own (the update's files win). Every file is written to `<name>.part` and renamed once its size and
//! SHA-256 match the record.
//!
//! The checks (TOC hash, per-file hash, record validation, `.part` + rename, standalone layering, free space) follow
//! pokopia-native (MIT), nupread.py and nxs2.py nup_unpack @7c3655d.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const NO_DIR: u32 = 0xFFFF_FFFF;
const KIND_BASE: u8 = 0;
const KIND_UPDATE: u8 = 1;
const MARKER: &str = ".nup_done";
const WORKING: &str = ".unpacking";
/// Free space kept beyond the unpacked size.
const SPARE: u64 = 2 << 30;
pub const CANCELLED: &str = "Unpacking was cancelled.";
const DAMAGED: &str = "The .nup file is damaged or incomplete.";

struct Package {
    root_dir: u32,
    exefs_dir: u32,
    romfs_dir: u32,
    version: u32,
    kind: u8,
    /// Package flag bit 0: the RomFS is complete on its own. Without it an update only holds the files it changes.
    standalone: bool,
}

struct Dir {
    name: String,
    first_child: u32,
    child_count: u32,
}

struct FileNode {
    name: String,
    size: u64,
    extent_offset: u64,
    extent_size: u64,
    sha: [u8; 32],
    codec: u8,
}

struct Nup {
    uuid: [u8; 16],
    file_size: u64,
    packages: Vec<Package>,
    dirs: Vec<Dir>,
    files: Vec<FileNode>,
    children: Vec<u32>,
}

fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o.saturating_add(4)).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| DAMAGED.to_string())
}
fn u64_at(b: &[u8], o: usize) -> Result<u64, String> {
    b.get(o..o.saturating_add(8)).map(|s| u64::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| DAMAGED.to_string())
}

/// Names in a package are labels (checked when a folder is walked: package roots are nameless), never paths: no separators, no "." or "..", nothing Windows reads specially.
fn checked_name(n: &str) -> Result<&str, String> {
    if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\\', ':', '\0', '<', '>', '"', '|', '?', '*']) || n.chars().any(|c| (c as u32) < 0x20) {
        return Err(format!("The .nup contains a file name that cannot be used here: {:?}", n));
    }
    Ok(n)
}

fn parse(f: &mut File) -> Result<Nup, String> {
    let real_size = f.metadata().map_err(|e| e.to_string())?.len();
    let mut sb = [0u8; 0x80];
    f.read_exact(&mut sb).map_err(|_| "This file is too small to be a .nup package.".to_string())?;
    if &sb[0..4] != b"NUP\x01" {
        return Err("This is not a .nup package (wrong header).".into());
    }
    let header_size = u16::from_le_bytes([sb[4], sb[5]]) as u64;
    let toc_offset = u64_at(&sb, 0x08)?;
    let toc_size = u64_at(&sb, 0x10)?;
    let toc_hash = &sb[0x18..0x38];
    let blob_offset = u64_at(&sb, 0x38)?;
    let file_size = u64_at(&sb, 0x40)?;
    let pkg_count = u32_at(&sb, 0x48)? as usize;
    if u32_at(&sb, 0x4C)? != 0 {
        return Err("This .nup was made with a newer format than this launcher knows. Update the launcher.".into());
    }
    if header_size < 0x80
        || toc_offset != header_size
        || blob_offset % 16 != 0
        || toc_offset.checked_add(toc_size).map_or(true, |e| e > blob_offset)
        || file_size != real_size
        || toc_size > (1 << 30)
    {
        return Err("The .nup file is damaged or incomplete (it does not match its header). Download it again.".into());
    }
    if !(1..=4096).contains(&pkg_count) {
        return Err("The .nup package count is not valid.".into());
    }
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&sb[0x50..0x60]);

    let mut toc = vec![0u8; toc_size as usize];
    f.seek(SeekFrom::Start(toc_offset)).map_err(|e| e.to_string())?;
    f.read_exact(&mut toc).map_err(|_| DAMAGED.to_string())?;
    if Sha256::digest(&toc).as_slice() != toc_hash {
        return Err("The .nup table of contents is damaged (checksum mismatch). Download the file again.".into());
    }
    if toc.get(0..4) != Some(b"TOC\x01".as_slice()) {
        return Err("The .nup table of contents is invalid.".into());
    }
    let toc_pkgs = u32_at(&toc, 4)? as usize;
    let dir_count = u64_at(&toc, 8)?;
    let file_count = u64_at(&toc, 0x10)?;
    let pool_off = u64_at(&toc, 0x18)?;
    let pool_size = u64_at(&toc, 0x20)?;
    let toc_len = toc.len() as u64;
    // counts come from the file: bound them by the TOC size before allocating anything
    if toc_pkgs != pkg_count || dir_count > toc_len / 0x18 || file_count > toc_len / 0x50 {
        return Err("The .nup table of contents is inconsistent.".into());
    }
    let (dir_count, file_count) = (dir_count as usize, file_count as usize);
    let pool_end = pool_off.checked_add(pool_size).filter(|e| *e <= toc_len).ok_or("The .nup string table is out of bounds.")?;
    let pool = &toc[pool_off as usize..pool_end as usize];
    let name_at = |off: u32, len: u32| -> Result<String, String> {
        let (o, l) = (off as usize, len as usize);
        Ok(String::from_utf8_lossy(pool.get(o..o.saturating_add(l)).ok_or("A name in the .nup is out of bounds.")?).into_owned())
    };

    let pkg_at = 0x30usize;
    let dir_at = pkg_at + 0x40 * pkg_count;
    let file_at = dir_at + 0x18 * dir_count;
    let child_at = file_at + 0x50 * file_count;
    if child_at as u64 > pool_off {
        return Err("The .nup table of contents is inconsistent.".into());
    }
    let mut packages = Vec::with_capacity(pkg_count);
    for i in 0..pkg_count {
        let pos = pkg_at + 0x40 * i;
        let p = Package {
            root_dir: u32_at(&toc, pos + 0x08)?,
            exefs_dir: u32_at(&toc, pos + 0x0C)?,
            romfs_dir: u32_at(&toc, pos + 0x10)?,
            version: u32_at(&toc, pos + 0x14)?,
            kind: *toc.get(pos + 0x18).ok_or(DAMAGED)?,
            standalone: toc.get(pos + 0x19).ok_or(DAMAGED)? & 1 != 0,
        };
        if p.kind >= 5 {
            return Err(format!("The .nup holds a package type ({}) this launcher does not know.", p.kind));
        }
        for d in [p.root_dir, p.exefs_dir, p.romfs_dir] {
            if d != NO_DIR && d as usize >= dir_count {
                return Err("The .nup package tree is invalid.".into());
            }
        }
        packages.push(p);
    }
    let mut dirs = Vec::with_capacity(dir_count);
    let mut slots = 0u64;
    for i in 0..dir_count {
        let pos = dir_at + 0x18 * i;
        let d = Dir { name: name_at(u32_at(&toc, pos)?, u32_at(&toc, pos + 4)?)?, first_child: u32_at(&toc, pos + 8)?, child_count: u32_at(&toc, pos + 0xC)? };
        slots += d.child_count as u64;
        dirs.push(d);
    }
    let mut files = Vec::with_capacity(file_count);
    for i in 0..file_count {
        let pos = file_at + 0x50 * i;
        let mut sha = [0u8; 32];
        sha.copy_from_slice(toc.get(pos + 0x20..pos + 0x40).ok_or(DAMAGED)?);
        let f = FileNode {
            name: name_at(u32_at(&toc, pos)?, u32_at(&toc, pos + 4)?)?,
            size: u64_at(&toc, pos + 8)?,
            extent_offset: u64_at(&toc, pos + 0x10)?,
            extent_size: u64_at(&toc, pos + 0x18)?,
            sha,
            codec: *toc.get(pos + 0x40).ok_or(DAMAGED)?,
        };
        if f.codec > 1 {
            return Err(format!("{} uses a compression ({}) this launcher does not know.", f.name, f.codec));
        }
        if f.extent_size > 0 && (f.extent_offset < blob_offset || f.extent_offset.checked_add(f.extent_size).map_or(true, |e| e > real_size) || f.extent_offset % 16 != 0) {
            return Err(format!("{} points outside the .nup file.", f.name));
        }
        if f.codec == 0 && f.extent_size != f.size {
            return Err(format!("{} has the wrong stored size.", f.name));
        }
        files.push(f);
    }
    if child_at as u64 + 4 * slots > pool_off {
        return Err("The .nup folder listing runs past the table of contents.".into());
    }
    let children = (0..slots as usize).map(|i| u32_at(&toc, child_at + i * 4)).collect::<Result<Vec<_>, _>>()?;
    if dirs.iter().any(|d| d.first_child as u64 + d.child_count as u64 > slots) {
        return Err("The .nup folder listing is out of bounds.".into());
    }
    Ok(Nup { uuid, file_size: real_size, packages, dirs, files, children })
}

impl Nup {
    /// Every file below a folder as (relative path with '/', file index), in the container's order.
    fn walk(&self, d: u32, prefix: &str, depth: u32, out: &mut Vec<(String, usize)>) -> Result<(), String> {
        if depth > 64 {
            return Err("The .nup folder tree is too deep.".into());
        }
        let dir = self.dirs.get(d as usize).ok_or("The .nup refers to a folder that does not exist.")?;
        let dc = self.dirs.len() as u32;
        for slot in dir.first_child..dir.first_child + dir.child_count {
            let id = *self.children.get(slot as usize).ok_or("The .nup folder listing is out of bounds.")?;
            if id < dc {
                self.walk(id, &format!("{}{}/", prefix, checked_name(&self.dirs[id as usize].name)?), depth + 1, out)?;
            } else {
                let fi = (id - dc) as usize;
                let f = self.files.get(fi).ok_or("The .nup refers to a file that does not exist.")?;
                out.push((format!("{}{}", prefix, checked_name(&f.name)?), fi));
            }
        }
        Ok(())
    }

    fn tree(&self, d: u32) -> Result<Vec<(String, usize)>, String> {
        let mut v = Vec::new();
        if d != NO_DIR {
            self.walk(d, "", 0, &mut v)?;
        }
        Ok(v)
    }
}

/// Writes through to `inner` and hashes what passes.
struct HashWriter<W: Write> {
    inner: W,
    hash: Sha256,
    len: u64,
}
impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(b)?;
        self.hash.update(&b[..n]);
        self.len += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct Extractor<'a> {
    nup: &'a Nup,
    src: BufReader<File>,
    done: u64,
    total: u64,
    progress: &'a mut dyn FnMut(u64, u64, &str),
    cancel: &'a dyn Fn() -> bool,
}

impl Extractor<'_> {
    /// One file to `dest`: written to `<dest>.part`, renamed once its size and SHA-256 match the record.
    fn file(&mut self, fi: usize, dest: &Path) -> Result<(), String> {
        let f = &self.nup.files[fi];
        if (self.cancel)() {
            return Err(CANCELLED.into());
        }
        (self.progress)(self.done, self.total, &f.name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {}", parent.display(), e))?;
        }
        let mut tmp_name = dest.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(".part");
        let tmp = dest.with_file_name(tmp_name);
        let res = self.write_part(fi, &tmp);
        if res.is_err() {
            let _ = fs::remove_file(&tmp);
            return res;
        }
        fs::rename(&tmp, dest).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("Cannot write {}: {}", dest.display(), e)
        })?;
        self.done += f.size;
        Ok(())
    }

    fn write_part(&mut self, fi: usize, tmp: &Path) -> Result<(), String> {
        let nup = self.nup;
        let f = &nup.files[fi];
        let disk = |e: std::io::Error| format!("Cannot write {} (disk full or blocked?): {}", tmp.display(), e);
        let mut out = HashWriter { inner: std::io::BufWriter::with_capacity(1 << 20, File::create(tmp).map_err(disk)?), hash: Sha256::new(), len: 0 };
        if f.extent_size > 0 {
            self.src.seek(SeekFrom::Start(f.extent_offset)).map_err(|e| e.to_string())?;
            let part = (&mut self.src).take(f.extent_size);
            let (name, cancel) = (&f.name, self.cancel);
            // a file may not unpack larger than its record says (guards against decompression bombs)
            match f.codec {
                0 => copy(&mut part.take(f.size), &mut out, cancel, name)?,
                _ => {
                    let dec = zstd::stream::read::Decoder::new(part).map_err(|e| format!("{} could not be decompressed: {}", name, e))?;
                    copy(&mut dec.take(f.size + 1), &mut out, cancel, name)?
                }
            }
        }
        out.flush().map_err(disk)?;
        if out.len != f.size || out.hash.finalize().as_slice() != f.sha {
            return Err(format!("{} does not match its checksum: the .nup is damaged. Download it again.", f.name));
        }
        Ok(())
    }
}

fn copy(r: &mut dyn Read, w: &mut dyn Write, cancel: &dyn Fn() -> bool, name: &str) -> Result<(), String> {
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel() {
            return Err(CANCELLED.into());
        }
        let n = r.read(&mut buf).map_err(|e| format!("{} could not be read from the .nup: {}", name, e))?;
        if n == 0 {
            return Ok(());
        }
        w.write_all(&buf[..n]).map_err(|e| format!("Cannot write {} (disk full or blocked?): {}", name, e))?;
    }
}

fn uuid_hex(u: &[u8; 16]) -> String {
    u.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Free bytes on the drive that holds `path` (or its nearest existing parent). None when it cannot be told.
#[cfg(windows)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    let mut p = path;
    while !p.exists() {
        p = p.parent()?;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut avail = 0u64;
    let (mut total, mut free) = (0u64, 0u64);
    // SAFETY: wide is NUL-terminated and the out pointers are valid for the call
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
    (ok != 0).then_some(avail)
}
#[cfg(not(windows))]
pub fn free_space(path: &Path) -> Option<u64> {
    crate::host::free_bytes(path)
}

/// Unpacks `nup_path` into `out`. A finished unpack of the same package is reused. `progress(done, total, current file)`
/// is called per file; `cancel` is polled during the unpack. Returns `out`.
pub fn extract(nup_path: &Path, out: &Path, progress: &mut dyn FnMut(u64, u64, &str), cancel: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    let want = crate::dump::supported().map(|(id, _)| id);
    extract_with(nup_path, out, progress, cancel, want.as_deref(), &free_space)
}

fn extract_with(
    nup_path: &Path,
    out: &Path,
    progress: &mut dyn FnMut(u64, u64, &str),
    cancel: &dyn Fn() -> bool,
    want_build: Option<&str>,
    free: &dyn Fn(&Path) -> Option<u64>,
) -> Result<PathBuf, String> {
    let mut file = File::open(nup_path).map_err(|e| format!("Cannot open {}: {}", nup_path.display(), e))?;
    let nup = parse(&mut file)?;
    let stamp = format!("{}:{}", uuid_hex(&nup.uuid), nup.file_size);
    if fs::read_to_string(out.join(MARKER)).map(|s| s.trim() == stamp).unwrap_or(false) {
        return Ok(out.to_path_buf());
    }
    // programs, newest update first, the base last; add-ons and system packages are not used by the game build
    let mut progs: Vec<&Package> = nup.packages.iter().filter(|p| (p.kind == KIND_BASE || p.kind == KIND_UPDATE) && p.exefs_dir != NO_DIR).collect();
    progs.sort_by_key(|p| std::cmp::Reverse((p.kind == KIND_UPDATE, p.version)));
    if progs.is_empty() {
        return Err("This .nup has no base game or update in it.".into());
    }

    // only a folder this code made is ever removed
    if out.exists() {
        let ours = out.join(MARKER).is_file() || out.join(WORKING).is_file() || fs::read_dir(out).map_err(|e| e.to_string())?.next().is_none();
        if !ours {
            return Err(format!("{} already exists and was not made by the launcher. Move or delete it, then try again.", out.display()));
        }
        fs::remove_dir_all(out).map_err(|e| format!("Cannot clear {}: {}", out.display(), e))?;
    }
    fs::create_dir_all(out).map_err(|e| format!("Cannot create {}: {}", out.display(), e))?;
    fs::write(out.join(WORKING), &stamp).map_err(|e| e.to_string())?;

    let mut ex = Extractor { nup: &nup, src: BufReader::with_capacity(1 << 20, file), done: 0, total: 0, progress, cancel };
    let root_of = |p: &Package| if p.kind == KIND_BASE { out.join("base") } else { out.join("updates").join(p.version.to_string()) };

    // ExeFS of every program first (small), then the version check: nothing big is written for a wrong .nup
    let mut chosen: Option<&Package> = None;
    let mut seen = Vec::new();
    for &p in &progs {
        let root = root_of(p);
        for (rel, fi) in nup.tree(p.exefs_dir)? {
            ex.file(fi, &root.join("exefs").join(&rel))?;
        }
        let bid = crate::dump::main_build_id(&root.join("exefs").join("main"));
        let label = if p.kind == KIND_BASE { "base game".to_string() } else { format!("update {}", p.version) };
        seen.push(format!("{} (build {})", label, bid.as_deref().unwrap_or("none")));
        if chosen.is_none() && (want_build.is_none() || bid.as_deref() == want_build) {
            chosen = Some(p);
        }
    }
    let p = chosen.ok_or_else(|| {
        format!("This .nup is not the game version this launcher supports (build {}). It holds: {}.", want_build.unwrap_or("?"), seen.join(", "))
    })?;

    // RomFS of the chosen program; a non-standalone update only holds what it changes, so the base's files go under it
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    if p.romfs_dir != NO_DIR {
        files.extend(nup.tree(p.romfs_dir)?);
    }
    if p.kind == KIND_UPDATE && !p.standalone {
        let base = nup.packages.iter().find(|q| q.kind == KIND_BASE && q.romfs_dir != NO_DIR);
        let Some(base) = base else {
            return Err("This update only holds the files it changes and needs the base game, which is not in this .nup. Use a .nup that contains the base game too.".into());
        };
        for (rel, fi) in nup.tree(base.romfs_dir)? {
            files.entry(rel).or_insert(fi);
        }
    }
    if files.is_empty() {
        return Err("The game program in this .nup has no RomFS (game data).".into());
    }
    let need: u64 = files.values().map(|&fi| nup.files[fi].size).sum();
    if let Some(avail) = free(out) {
        if avail < need.saturating_add(SPARE) {
            let gb = |b: u64| (b as f64 / (1u64 << 30) as f64).ceil() as u64;
            return Err(format!(
                "Not enough free disk space to unpack the game: about {} GB are needed on the drive of {}, {} GB are free.",
                gb(need.saturating_add(SPARE)),
                out.display(),
                avail >> 30
            ));
        }
    }
    ex.total = need;
    ex.done = 0;
    let romfs = root_of(p).join("romfs");
    fs::create_dir_all(&romfs).map_err(|e| format!("Cannot create {}: {}", romfs.display(), e))?;
    for (rel, fi) in &files {
        ex.file(*fi, &romfs.join(rel))?;
    }
    (ex.progress)(need, need, "");
    fs::write(out.join(MARKER), stamp).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(out.join(WORKING));
    Ok(out.to_path_buf())
}

#[cfg(test)]
mod tests {
    //! Synthetic containers built byte by byte as the format lays them out (ported from pokopia-native (MIT),
    //! tools/test_nupread.py @7c3655d). No game data.
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    enum Node {
        File(Vec<u8>),
        Dir(Vec<(&'static str, Node)>),
    }
    use Node::{Dir as D, File as F};

    fn f(b: &[u8]) -> Node {
        F(b.to_vec())
    }

    /// (title, version, kind, package flags, tree)
    struct Pkg(u64, u32, u8, u8, Vec<(&'static str, Node)>);

    struct B {
        dirs: Vec<(String, Vec<(bool, usize)>)>,
        files: Vec<(String, Vec<u8>)>,
    }
    impl B {
        fn add_dir(&mut self, name: &str, tree: &[(&'static str, Node)]) -> usize {
            let i = self.dirs.len();
            self.dirs.push((name.to_string(), vec![]));
            let mut sorted: Vec<&(&str, Node)> = tree.iter().collect();
            sorted.sort_by_key(|(k, _)| k.as_bytes().to_vec());
            for (k, v) in sorted {
                let e = match v {
                    D(sub) => (true, self.add_dir(k, sub)),
                    F(b) => {
                        self.files.push((k.to_string(), b.clone()));
                        (false, self.files.len() - 1)
                    }
                };
                self.dirs[i].1.push(e);
            }
            i
        }
    }

    fn make_nup(path: &Path, pkgs: &[Pkg], zstd_level: Option<i32>) {
        let mut b = B { dirs: vec![], files: vec![] };
        let mut recs = Vec::new();
        for Pkg(title, ver, kind, pflags, tree) in pkgs {
            let root = b.add_dir("", tree);
            let sub = |n: &str| b.dirs[root].1.iter().find(|(d, i)| *d && b.dirs[*i].0 == n).map_or(NO_DIR, |(_, i)| *i as u32);
            recs.push((*title, root as u32, sub("exefs"), sub("romfs"), *ver, *kind, *pflags));
        }
        let (nd, nf) = (b.dirs.len(), b.files.len());
        let mut pool = Vec::new();
        let mut s = |name: &str| {
            let off = pool.len() as u32;
            pool.extend_from_slice(name.as_bytes());
            (off, name.len() as u32)
        };
        let mut children: Vec<u32> = vec![];
        let mut dir_recs = vec![];
        for (name, kids) in &b.dirs {
            let (off, ln) = s(name);
            for v in [off, ln, children.len() as u32, kids.len() as u32] {
                dir_recs.extend_from_slice(&v.to_le_bytes());
            }
            dir_recs.extend_from_slice(&0u64.to_le_bytes());
            children.extend(kids.iter().map(|&(is_dir, i)| if is_dir { i as u32 } else { (nd + i) as u32 }));
        }
        let names: Vec<(u32, u32)> = b.files.iter().map(|(n, _)| s(n)).collect();
        let head = 0x30 + 0x40 * recs.len() + 0x18 * nd + 0x50 * nf + 4 * children.len();
        let toc_size = head + pool.len();
        let blob_off = (0x80 + toc_size + 15) & !15;
        let mut blob: Vec<u8> = vec![];
        let mut file_recs = vec![];
        for ((_, data), (off, ln)) in b.files.iter().zip(&names) {
            let (mut stored, mut codec) = (data.clone(), 0u8);
            if let (Some(l), false) = (zstd_level, data.is_empty()) {
                let z = zstd::encode_all(&data[..], l).unwrap();
                if z.len() < data.len() {
                    stored = z;
                    codec = 1;
                }
            }
            let at = if stored.is_empty() { 0 } else { (blob_off + blob.len()) as u64 };
            file_recs.extend_from_slice(&off.to_le_bytes());
            file_recs.extend_from_slice(&ln.to_le_bytes());
            file_recs.extend_from_slice(&(data.len() as u64).to_le_bytes());
            file_recs.extend_from_slice(&at.to_le_bytes());
            file_recs.extend_from_slice(&(stored.len() as u64).to_le_bytes());
            file_recs.extend_from_slice(&Sha256::digest(data));
            file_recs.push(codec);
            file_recs.extend_from_slice(&[0u8; 15]);
            blob.extend_from_slice(&stored);
            blob.resize(blob.len().div_ceil(16) * 16, 0);
        }
        let mut pkg_recs = vec![];
        for (t, r, e, ro, v, k, fl) in &recs {
            let mut p = vec![0u8; 0x40];
            p[0..8].copy_from_slice(&t.to_le_bytes());
            p[8..12].copy_from_slice(&r.to_le_bytes());
            p[12..16].copy_from_slice(&e.to_le_bytes());
            p[16..20].copy_from_slice(&ro.to_le_bytes());
            p[20..24].copy_from_slice(&v.to_le_bytes());
            p[24] = *k;
            p[25] = *fl;
            pkg_recs.extend_from_slice(&p);
        }
        let mut toc = b"TOC\x01".to_vec();
        toc.extend_from_slice(&(recs.len() as u32).to_le_bytes());
        for v in [nd as u64, nf as u64, head as u64, pool.len() as u64, 0] {
            toc.extend_from_slice(&v.to_le_bytes());
        }
        toc.extend(pkg_recs);
        toc.extend(dir_recs);
        toc.extend(file_recs);
        for c in &children {
            toc.extend_from_slice(&c.to_le_bytes());
        }
        toc.extend_from_slice(&pool);
        assert_eq!(toc.len(), toc_size);
        let size = blob_off + blob.len();
        let mut sb = b"NUP\x01".to_vec();
        sb.extend_from_slice(&0x80u16.to_le_bytes());
        sb.extend_from_slice(&1u16.to_le_bytes());
        sb.extend_from_slice(&0x80u64.to_le_bytes());
        sb.extend_from_slice(&(toc_size as u64).to_le_bytes());
        sb.extend_from_slice(&Sha256::digest(&toc));
        sb.extend_from_slice(&(blob_off as u64).to_le_bytes());
        sb.extend_from_slice(&(size as u64).to_le_bytes());
        sb.extend_from_slice(&(recs.len() as u32).to_le_bytes());
        sb.extend_from_slice(&0u32.to_le_bytes());
        sb.extend_from_slice(&Sha256::digest(&toc)[..16]);
        sb.resize(0x80, 0);
        let mut all = sb;
        all.extend(toc);
        all.resize(blob_off, 0);
        all.extend(blob);
        fs::write(path, all).unwrap();
    }

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!("nup-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn base() -> Vec<(&'static str, Node)> {
        vec![
            ("exefs", D(vec![("main", f(b"base main")), ("main.npdm", f(b"npdm"))])),
            ("romfs", D(vec![("Data", D(vec![("a.bin", F(vec![b'A'; 5000])), ("only_base.bin", f(b"base"))])), ("top.txt", f(b"top"))])),
        ]
    }
    fn update() -> Vec<(&'static str, Node)> {
        vec![
            ("exefs", D(vec![("main", f(b"update main")), ("sdk", f(b"sdk"))])),
            ("romfs", D(vec![("Data", D(vec![("a.bin", F(vec![b'B'; 7000])), ("new.bin", f(b""))])), ("top.txt", f(b"top v2"))])),
        ]
    }
    fn pkgs(update_flags: u8) -> Vec<Pkg> {
        vec![Pkg(0x0400A3A01EE4C000, 0, 0, 0, base()), Pkg(0x0400A3A01EE4C800, 655360, 1, update_flags, update())]
    }
    fn no_progress(_: u64, _: u64, _: &str) {}
    fn plenty(_: &Path) -> Option<u64> {
        Some(u64::MAX)
    }
    /// Stand-in for the build ID: `main_build_id` needs a real NSO, so tests run with no wanted build (newest wins).
    fn run(nup: &Path, out: &Path, want: Option<&str>) -> Result<PathBuf, String> {
        extract_with(nup, out, &mut no_progress, &|| false, want, &plenty)
    }
    fn read(p: PathBuf) -> Vec<u8> {
        fs::read(p).unwrap()
    }
    fn no_part(dir: &Path) -> bool {
        fn walk(d: &Path) -> bool {
            fs::read_dir(d).unwrap().all(|e| {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p)
                } else {
                    p.extension().map_or(true, |x| x != "part")
                }
            })
        }
        walk(dir)
    }

    fn layered(level: Option<i32>) {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &pkgs(0), level);
        run(&n, &out, None).unwrap();
        let r = out.join("updates/655360");
        assert_eq!(read(r.join("romfs/Data/a.bin")), vec![b'B'; 7000], "the update's own file wins");
        assert_eq!(read(r.join("romfs/Data/only_base.bin")), b"base", "the base's file sits under it");
        assert_eq!(read(r.join("romfs/top.txt")), b"top v2");
        assert_eq!(read(r.join("romfs/Data/new.bin")), b"");
        assert_eq!(read(r.join("exefs/main")), b"update main");
        assert_eq!(read(out.join("base/exefs/main")), b"base main");
        assert!(!out.join("base/romfs").exists(), "only the chosen program's RomFS is unpacked");
        assert!(no_part(&out));
    }

    #[test]
    fn raw_non_standalone_update_is_layered_over_base() {
        layered(None);
    }

    #[test]
    fn zstd_round_trip() {
        layered(Some(3));
        let t = Tmp::new();
        make_nup(&t.0.join("g.nup"), &pkgs(0), Some(3));
        let mut file = File::open(t.0.join("g.nup")).unwrap();
        assert!(parse(&mut file).unwrap().files.iter().any(|x| x.codec == 1), "the repetitive files compress");
    }

    #[test]
    fn standalone_update_is_not_layered() {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &pkgs(1), None);
        run(&n, &out, None).unwrap();
        let r = out.join("updates/655360/romfs");
        assert_eq!(read(r.join("top.txt")), b"top v2");
        assert!(!r.join("Data/only_base.bin").exists());
    }

    #[test]
    fn non_standalone_update_without_base_is_refused() {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &[Pkg(2, 655360, 1, 0, update())], None);
        let e = run(&n, &out, None).unwrap_err();
        assert!(e.contains("needs the base game"), "{e}");
        assert!(!out.join(MARKER).exists());
    }

    #[test]
    fn same_nup_is_not_unpacked_twice() {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &pkgs(0), None);
        run(&n, &out, None).unwrap();
        let top = out.join("updates/655360/romfs/top.txt");
        fs::write(&top, b"touched").unwrap();
        run(&n, &out, None).unwrap();
        assert_eq!(read(top), b"touched");
    }

    #[test]
    fn wrong_version_fails_before_any_romfs_is_written() {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &pkgs(0), None);
        let e = run(&n, &out, Some("00ff")).unwrap_err();
        assert!(e.contains("not the game version"), "{e}");
        assert!(!out.join("updates/655360/romfs").exists() && !out.join("base/romfs").exists());
    }

    #[test]
    fn free_space_is_checked_before_romfs() {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &pkgs(0), None);
        let e = extract_with(&n, &out, &mut no_progress, &|| false, None, &|_| Some(1 << 30)).unwrap_err();
        assert!(e.contains("Not enough free disk space"), "{e}");
        assert!(!out.join("updates/655360/romfs").exists());
        // exactly SPARE + the data is enough
        let need = 7010u64;
        let ok = extract_with(&n, &out, &mut no_progress, &|| false, None, &|_| Some(SPARE + need + 10_000));
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn cancel_stops_and_leaves_no_part_file() {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &pkgs(0), None);
        let calls = std::cell::Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() > 8
        };
        let e = extract_with(&n, &out, &mut no_progress, &cancel, None, &plenty).unwrap_err();
        assert_eq!(e, CANCELLED);
        assert!(no_part(&out));
        assert!(!out.join(MARKER).exists(), "a cancelled unpack is not reused");
        // and a later run clears the half-done folder and finishes
        run(&n, &out, None).unwrap();
        assert!(out.join(MARKER).is_file());
    }

    #[test]
    fn damaged_files_are_rejected() {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &pkgs(0), None);
        let good = fs::read(&n).unwrap();
        let bad = |data: Vec<u8>, want: &str| {
            let p = t.0.join("bad.nup");
            fs::write(&p, data).unwrap();
            let e = run(&p, &t.0.join("bad_out"), None).unwrap_err();
            assert!(e.contains(want), "{e} (wanted {want})");
        };
        let mut wrong_magic = good.clone();
        wrong_magic[0..4].copy_from_slice(b"NOPE");
        bad(wrong_magic, "not a .nup");
        bad(good[..good.len() - 16].to_vec(), "damaged or incomplete");
        bad(good[..10].to_vec(), "too small");
        let mut toc = good.clone();
        toc[0x80 + 0x40] ^= 1;
        bad(toc, "table of contents is damaged");
        let mut flags = good.clone();
        flags[0x4C] = 2;
        bad(flags, "newer format");
        let blob_off = u64::from_le_bytes(good[0x38..0x40].try_into().unwrap()) as usize;
        let mut body = good.clone();
        body[blob_off] ^= 0xFF;
        let p = t.0.join("body.nup");
        fs::write(&p, body).unwrap();
        let e = run(&p, &out, None).unwrap_err();
        assert!(e.contains("does not match its checksum"), "{e}");
        assert!(no_part(&out));
    }

    #[test]
    fn unsafe_names_are_rejected() {
        let t = Tmp::new();
        for bad in ["..", "a:b", "x\u{1}", "q?", "a<b", "s*"] {
            let n = t.0.join("n.nup");
            make_nup(&n, &[Pkg(1, 0, 0, 0, vec![("exefs", D(vec![("main", f(b"m"))])), ("romfs", D(vec![(bad, f(b"x"))]))])], None);
            let e = run(&n, &t.0.join("o"), None).unwrap_err();
            assert!(e.contains("cannot be used"), "{bad:?}: {e}");
        }
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let t = Tmp::new();
        let n = t.0.join("k.nup");
        make_nup(&n, &[Pkg(1, 0, 7, 0, vec![("exefs", D(vec![("main", f(b"m"))]))])], None);
        assert!(run(&n, &t.0.join("o"), None).unwrap_err().contains("package type (7)"));
    }

    #[test]
    fn foreign_folder_is_never_deleted() {
        let t = Tmp::new();
        let (n, out) = (t.0.join("g.nup"), t.0.join("out"));
        make_nup(&n, &pkgs(0), None);
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("mine.txt"), b"keep").unwrap();
        let e = run(&n, &out, None).unwrap_err();
        assert!(e.contains("not made by the launcher"), "{e}");
        assert_eq!(read(out.join("mine.txt")), b"keep");
    }
}
