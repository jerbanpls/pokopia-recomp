// SPDX-License-Identifier: GPL-3.0-or-later
//! Can setup write in the game folder? Windows Security's Controlled folder access (ransomware protection) and some
//! antivirus programs block writes in Documents, Desktop, Downloads and Pictures for programs they do not know. The game
//! folder defaults to the launcher's own folder, which is often one of those, and the build and shader tools then fail
//! halfway with errors that read like a missing file. The pre-flight check, Set up and the .nup unpack try a test file
//! first: create, write, read back, delete. The wording follows the OS error code.
//!
//! Idea from pokopia-native (MIT), nxs2.py `check_writable` @5f0b08f.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// The folders setup writes in, under the game folder: pkp.ini, setup.log and pkp.exe in the folder itself, the game
/// build (`build`, its zig cache inside), the shader cache, and the unpacked .nup (`nup_dump`). Subfolders that do not
/// exist yet are created inside the game folder, so testing the game folder covers them.
pub const SUBFOLDERS: [&str; 4] = ["build", "shader_cache", "shader_cache/spirv", "nup_dump"];

/// Which part of the test failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Folder,
    Create,
    Write,
    ReadBack,
    Delete,
}

impl Step {
    fn what(self) -> &'static str {
        match self {
            Step::Folder => "creating the folder",
            Step::Create => "creating a test file",
            Step::Write => "writing a test file",
            Step::ReadBack => "reading the test file back",
            Step::Delete => "deleting the test file",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// access denied, or a file just written is gone or changed: Controlled folder access or an antivirus
    Blocked,
    /// a write-protected drive
    ReadOnly,
    DiskFull,
    Other,
}

#[derive(Clone, Debug)]
pub struct WriteError {
    pub dir: PathBuf,
    pub step: Step,
    /// the raw OS error code (GetLastError on Windows), None when the file read back wrong; `cause` is made from it and
    /// `os` (the OS text) ends with it
    #[allow(dead_code)]
    pub code: Option<i32>,
    pub os: String,
    pub cause: Cause,
}

/// The cause, from the raw OS error code alone.
pub fn cause_of(step: Step, code: Option<i32>) -> Cause {
    let Some(c) = code else {
        // the file was written but read back different: something rewrote it
        return Cause::Blocked;
    };
    #[cfg(windows)]
    {
        // ERROR_ACCESS_DENIED 5, ERROR_VIRUS_INFECTED 225, ERROR_VIRUS_DELETED 226, ERROR_ACCESS_DISABLED_BY_POLICY 1260
        // ERROR_FILE_NOT_FOUND 2 / ERROR_PATH_NOT_FOUND 3 on a file in a folder that exists: Controlled folder access makes
        // the new file look missing (what Python reported in pokopia-native's case). Not for the folder itself: a
        // missing drive gives the same codes.
        match c {
            5 | 225 | 226 | 1260 => Cause::Blocked,
            2 | 3 if step != Step::Folder => Cause::Blocked,
            19 => Cause::ReadOnly,     // ERROR_WRITE_PROTECT
            39 | 112 => Cause::DiskFull, // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
            _ => Cause::Other,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = step;
        match c {
            1 | 13 => Cause::Blocked, // EPERM, EACCES
            30 => Cause::ReadOnly,    // EROFS
            28 => Cause::DiskFull,    // ENOSPC
            _ => Cause::Other,
        }
    }
}

impl WriteError {
    fn from_io(dir: &Path, step: Step, e: &io::Error) -> WriteError {
        let code = e.raw_os_error();
        WriteError { dir: dir.to_path_buf(), step, code, os: e.to_string(), cause: cause_of(step, code) }
    }

    pub fn title(&self) -> &'static str {
        match self.cause {
            Cause::Blocked => "Windows is blocking writes in the game folder",
            Cause::ReadOnly => "The game folder's drive is write-protected",
            Cause::DiskFull => "The game folder's drive is full",
            Cause::Other => "Cannot write in the game folder",
        }
    }

    /// What happened and what to do, for the pre-flight list and the error of Set up and the .nup unpack.
    pub fn message(&self) -> String {
        let dir = self.dir.to_string_lossy().replace('/', "\\");
        let dir = dir.strip_prefix(r"\\?\").unwrap_or(&dir).to_string();
        let what = format!("{} in {} failed: {}", self.step.what(), dir, self.os);
        match self.cause {
            Cause::Blocked => format!(
                "Windows is blocking writes in this folder ({}). This is most likely Windows Security's Controlled folder access or an antivirus, which block programs in Documents, Desktop, Downloads and Pictures. Allow PokopiaLauncher.exe, pkp.exe and the tools it runs (python.exe, zig.exe and spirv_harness.exe in the tools folder) in Windows Security > Virus & threat protection > Ransomware protection > Allow an app through Controlled folder access, or in your antivirus. Or move the game folder somewhere like C:\\Games.",
                what
            ),
            Cause::ReadOnly => format!("The drive is write-protected ({}). Move the game folder to another drive, somewhere like C:\\Games.", what),
            Cause::DiskFull => format!("The drive is full ({}). Free some space, or move the game folder to a bigger drive.", what),
            Cause::Other => format!("Setup cannot write in this folder ({}). Check that the folder is not read-only, or move the game folder somewhere like C:\\Games.", what),
        }
    }
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Creates `dir` if needed, then creates, writes, reads back and deletes a uniquely named test file in it.
pub fn probe(dir: &Path) -> Result<(), WriteError> {
    fs::create_dir_all(dir).map_err(|e| WriteError::from_io(dir, Step::Folder, &e))?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let name = format!(".pkp-write-test-{}-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed), nanos);
    let path = dir.join(&name);
    let body = format!("Pokopia launcher write test {}\n", name);
    let fail = |step: Step, e: io::Error| WriteError::from_io(dir, step, &e);
    let res = (|| {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| fail(Step::Create, e))?;
        f.write_all(body.as_bytes()).map_err(|e| fail(Step::Write, e))?;
        drop(f);
        let back = fs::read(&path).map_err(|e| fail(Step::ReadBack, e))?;
        if back != body.as_bytes() {
            return Err(WriteError {
                dir: dir.to_path_buf(),
                step: Step::ReadBack,
                code: None,
                os: format!("the file read back as {} bytes, not the {} written", back.len(), body.len()),
                cause: cause_of(Step::ReadBack, None),
            });
        }
        fs::remove_file(&path).map_err(|e| fail(Step::Delete, e))
    })();
    if res.is_err() {
        let _ = fs::remove_file(&path);
    }
    res
}

/// The game folder and those of its setup subfolders that exist.
pub fn folders(game_dir: &Path) -> Vec<PathBuf> {
    std::iter::once(game_dir.to_path_buf()).chain(SUBFOLDERS.iter().map(|s| game_dir.join(s)).filter(|p| p.is_dir())).collect()
}

/// Probes every folder of `folders`; the first failure.
pub fn check(game_dir: &Path) -> Result<(), WriteError> {
    folders(game_dir).iter().try_for_each(|d| probe(d))
}
