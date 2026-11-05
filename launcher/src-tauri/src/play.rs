// SPDX-License-Identifier: GPL-3.0-or-later
//! Starts pkp.exe, watches it, and reports how it ended.

use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

/// Files the launcher may open for the user, by name, inside the game folder.
pub const REPORT_FILES: [&str; 4] = ["pkp_crash.txt", "pkp_freeze.txt", "pkp_fatal.txt", "pkp.log"];

#[derive(Serialize, Clone, Default)]
pub struct ReportFile {
    pub name: String,
    /// Written since this run started.
    pub fresh: bool,
}

#[derive(Serialize, Clone)]
pub struct Status {
    /// "idle", "running" or "exited"
    pub state: String,
    pub code: Option<i32>,
    pub crashed: bool,
    pub files: Vec<ReportFile>,
    pub message: String,
}

impl Default for Status {
    fn default() -> Self {
        Status { state: "idle".into(), code: None, crashed: false, files: vec![], message: String::new() }
    }
}

#[derive(Default)]
pub struct PlayState(pub Mutex<Status>);

fn set_status(app: &AppHandle, s: Status) {
    *app.state::<PlayState>().0.lock().unwrap() = s.clone();
    let _ = app.emit("game-status", s);
}

fn report_files(dir: &Path, started: SystemTime) -> Vec<ReportFile> {
    REPORT_FILES
        .iter()
        .filter_map(|n| {
            let md = std::fs::metadata(dir.join(n)).ok()?;
            let fresh = md.modified().map(|m| m + Duration::from_secs(2) >= started).unwrap_or(false);
            Some(ReportFile { name: n.to_string(), fresh })
        })
        .collect()
}

pub fn start(app: &AppHandle, game_dir: &Path, minimize: bool) -> Result<(), String> {
    {
        let st = app.state::<PlayState>();
        if st.0.lock().unwrap().state == "running" {
            return Err("The game is already running.".into());
        }
    }
    let exe = game_dir.join("pkp.exe");
    if !exe.is_file() {
        return Err(format!("pkp.exe was not found in {}. Put the launcher next to pkp.exe or set the game folder in Settings.", crate::dump::fwd(game_dir)));
    }

    let app2 = app.clone();
    let dir = game_dir.to_path_buf();
    let tools = crate::exe_dir().join("tools");
    let first = if crate::shaders::needs_warm_up(game_dir) {
        "Collecting the shaders the game builds while it runs (once, about a minute)..."
    } else {
        "Checking the shaders the game builds while it runs..."
    };
    set_status(app, Status { state: "running".into(), message: first.into(), ..Default::default() });
    std::thread::spawn(move || {
        // every start: the run-time shaders seen so far must all be translated (the warm-up run collects them the first
        // time, e.g. for a setup finished before it existed). A failure here is logged in the status, the game still starts.
        let prep = match crate::shaders::prepare(&dir, &tools, &std::sync::atomic::AtomicBool::new(false)) {
            Ok(Some(n)) if n > 0 => format!(" {} shaders were translated before the start.", n),
            Err(e) => format!(" Shaders were not prepared: {}.", e),
            _ => String::new(),
        };
        // Linux: through Proton (host.rs)
        let mut cmd = match crate::host::windows_program(&exe) {
            Ok(c) => c,
            Err(e) => {
                set_status(&app2, Status { state: "exited".into(), crashed: true, message: format!("Could not start pkp.exe: {}", e), ..Default::default() });
                return;
            }
        };
        // every path (exefs, romfs, shader cache) comes from pkp.ini next to pkp.exe
        cmd.current_dir(&dir).args(["--log", "pkp.log"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no stray console if pkp.exe is a console build
        }
        let started = SystemTime::now();
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                set_status(&app2, Status { state: "exited".into(), crashed: true, message: format!("Could not start pkp.exe: {}", e), ..Default::default() });
                return;
            }
        };
        set_status(&app2, Status { state: "running".into(), message: format!("pkp.exe started (process {})", child.id()), ..Default::default() });
        if minimize {
            if let Some(w) = app2.get_webview_window("main") {
                let _ = w.minimize();
            }
        }
        let result = child.wait();
        let (code, wait_err) = match result {
            Ok(s) => (s.code(), None),
            Err(e) => (None, Some(e.to_string())),
        };
        // shaders the game built while it ran: translated now, so the next run has them (the state stays "running" meanwhile)
        set_status(&app2, Status { state: "running".into(), message: "Translating the shaders the game built while it ran...".into(), ..Default::default() });
        let shaders = match crate::shaders::translate_missing(&dir, &tools) {
            Ok(0) => String::new(),
            Ok(n) => format!(" {} new shaders translated.", n),
            Err(e) => format!(" New shaders were not translated: {}.", e),
        };
        let files = report_files(&dir, started);
        let crash_file = files.iter().any(|f| f.fresh && f.name != "pkp.log");
        let crashed = crash_file || code.map_or(true, |c| c != 0);
        let message = match (&wait_err, code) {
            (Some(e), _) => format!("Lost track of pkp.exe: {}", e),
            (None, Some(0)) => "pkp.exe exited normally.".to_string(),
            (None, Some(c)) => format!("pkp.exe exited with code {} (0x{:08X}).", c, c as u32),
            (None, None) => "pkp.exe was ended by the system.".to_string(),
        };
        set_status(&app2, Status { state: "exited".into(), code, crashed, files, message: message + &prep + &shaders });
        if minimize {
            if let Some(w) = app2.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }
    });
    Ok(())
}

pub fn open_with_shell(path: &Path) -> Result<(), String> {
    // explorer (Linux: xdg-open) opens a file in its default app and a folder in a window
    crate::host::open(path.as_os_str())
}

/// Opens an http(s) link in the default browser (same route as open_with_shell).
pub fn open_url(url: &str) -> Result<(), String> {
    crate::host::open(std::ffi::OsStr::new(url))
}
