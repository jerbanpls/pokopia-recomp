// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dlss;
mod dump;
mod gamebuild;
mod host;
mod ini;
mod lan;
mod nup;
mod play;
mod preflight;
mod relay;
mod saveimport;
mod settings;
mod setuplog;
mod shaders;
mod update;
mod writecheck;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// The launcher's own options, stored in the user's app config folder.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct Config {
    /// Root of the decrypted dump (the folder with base/ and updates/).
    dump_dir: String,
    /// Folder with pkp.exe. Empty: the folder the launcher is in.
    game_dir: String,
    minimize_on_play: bool,
    /// Set up with fewer cores at idle priority (preflight.rs jobs_for)
    gentle_build: bool,
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    Ok(dir.join("launcher.json"))
}

fn load_config(app: &AppHandle) -> Config {
    config_path(app)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The launcher's folder, where `tools/` is. An AppImage runs from a temporary mount: there it is the folder of the
/// .AppImage file itself ($APPIMAGE, set by the AppImage runtime).
fn exe_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("APPIMAGE").map(PathBuf::from).and_then(|p| p.parent().map(Path::to_path_buf)) {
        return dir;
    }
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

fn game_dir_of(cfg: &Config) -> PathBuf {
    if cfg.game_dir.trim().is_empty() {
        exe_dir()
    } else {
        PathBuf::from(cfg.game_dir.trim())
    }
}

#[derive(Serialize)]
struct Overview {
    game_dir: String,
    default_game_dir: String,
    exe_exists: bool,
}

#[tauri::command]
fn get_config(app: AppHandle) -> Config {
    load_config(&app)
}

#[tauri::command]
fn save_config(app: AppHandle, config: Config) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    settings::write_atomic(&config_path(&app)?, &text).map_err(|e| format!("cannot save launcher settings: {}", e))
}

/// Background update check (see update.rs). None: no update, a dev build, or any error.
#[tauri::command]
async fn check_update(app: AppHandle) -> Option<update::UpdateInfo> {
    #[allow(unused_mut)]
    let mut version = app.package_info().version.to_string();
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("PKP_LAUNCHER_UPDATE_CHECK_AS") {
        version = v; // debug builds only: pretend to be this version (dev builds otherwise skip the check)
    }
    if update::is_dev_version(&version) {
        return None;
    }
    tauri::async_runtime::spawn_blocking(move || update::check(&version)).await.ok().flatten()
}

/// Opens a release page in the default browser.
#[tauri::command]
fn open_release_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://github.com/") || url.chars().any(|c| c.is_whitespace() || c == '"' || c == '&') {
        return Err("That link cannot be opened from here.".into());
    }
    play::open_url(&url)
}

#[tauri::command]
fn get_overview(app: AppHandle) -> Overview {
    let cfg = load_config(&app);
    let dir = game_dir_of(&cfg);
    Overview {
        game_dir: dump::fwd(&dir),
        default_game_dir: dump::fwd(&exe_dir()),
        exe_exists: dir.join("pkp.exe").is_file(),
    }
}

#[tauri::command]
async fn pick_folder(app: AppHandle, title: String, start: Option<String>) -> Option<String> {
    let mut dlg = app.dialog().file().set_title(title);
    if let Some(s) = start.filter(|s| PathBuf::from(s).is_dir()) {
        dlg = dlg.set_directory(s);
    }
    dlg.blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| dump::fwd(&p))
}

#[tauri::command]
async fn pick_nup(app: AppHandle, start: Option<String>) -> Option<String> {
    let mut dlg = app.dialog().file().set_title("Choose your Pokopia .nup package").add_filter("NUP package", &["nup"]);
    if let Some(s) = start.filter(|s| PathBuf::from(s).is_dir()) {
        dlg = dlg.set_directory(s);
    }
    dlg.blocking_pick_file().and_then(|p| p.into_path().ok()).map(|p| dump::fwd(&p))
}

#[derive(Serialize, Clone)]
struct NupProgress {
    done: u64,
    total: u64,
    file: String,
}

/// Unpacks a .nup into `<game folder>/nup_dump` (base/ and updates/<version>/, the same layout as a dump folder) and
/// returns that folder. Emits "nup-progress". A finished unpack of the same package is reused.
#[tauri::command]
async fn import_nup(app: AppHandle, state: State<'_, Arc<SetupState>>, path: String) -> Result<String, String> {
    if state.progress.lock().unwrap().state == "running" {
        return Err("Setup is running. Pause it first.".into());
    }
    let game_dir = game_dir_of(&load_config(&app));
    let out = game_dir.join("nup_dump");
    // a blocked folder fails here, in words, instead of halfway through with an OS error (writecheck.rs)
    writecheck::check(&game_dir).map_err(|e| e.message())?;
    let src = PathBuf::from(path.trim());
    let emitter = app.clone();
    let st = state.inner().clone();
    st.nup_cancel.store(false, Ordering::SeqCst);
    let res = tauri::async_runtime::spawn_blocking(move || {
        let mut last = 0u64;
        nup::extract(
            &src,
            &out,
            &mut |done, total, file| {
                // the UI only needs ~100 updates over the whole unpack
                if total == 0 || done == total || done - last >= total / 100 {
                    last = done;
                    let _ = emitter.emit("nup-progress", NupProgress { done, total, file: file.to_string() });
                }
            },
            &|| st.nup_cancel.load(Ordering::SeqCst),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(dump::fwd(&res))
}

/// Stops a running .nup unpack (the call to `import_nup` then returns an error).
#[tauri::command]
fn cancel_nup(state: State<Arc<SetupState>>) {
    state.nup_cancel.store(true, Ordering::SeqCst);
}

/// [fs] romfs_base for pkp.ini: the base romfs (relative to the game folder when inside it), or empty for none.
fn base_value(game_dir: &Path, base: &str) -> String {
    if base.is_empty() { String::new() } else { settings::portable(game_dir, base) }
}

#[tauri::command]
fn validate_dump(path: String) -> dump::DumpReport {
    dump::scan(&path)
}

/// Validates, stores the dump folder, and points `[fs] romfs` in pkp.ini at the chosen romfs.
#[tauri::command]
fn save_dump(app: AppHandle, path: String) -> Result<dump::DumpReport, String> {
    let report = dump::scan(&path);
    let chosen = report.chosen.clone().ok_or_else(|| report.errors.join(" "))?;
    let mut cfg = load_config(&app);
    cfg.dump_dir = report.path.clone();
    save_config(app.clone(), cfg.clone())?;
    settings::apply(
        &game_dir_of(&cfg),
        &[
            settings::Edit { file: "pkp".into(), section: "fs".into(), key: "exefs".into(), value: settings::portable(&game_dir_of(&cfg), &chosen.exefs) },
            settings::Edit { file: "pkp".into(), section: "fs".into(), key: "romfs".into(), value: settings::portable(&game_dir_of(&cfg), &chosen.romfs) },
            settings::Edit { file: "pkp".into(), section: "fs".into(), key: "romfs_base".into(), value: base_value(&game_dir_of(&cfg), &chosen.romfs_base) },
        ],
    )?;
    Ok(report)
}

#[tauri::command]
fn read_settings(app: AppHandle) -> settings::SettingsDump {
    settings::read(&game_dir_of(&load_config(&app)))
}

#[tauri::command]
fn write_settings(app: AppHandle, edits: Vec<settings::Edit>) -> Result<(), String> {
    settings::apply(&game_dir_of(&load_config(&app)), &edits)
}

#[tauri::command]
fn play(app: AppHandle) -> Result<(), String> {
    let cfg = load_config(&app);
    if cfg.dump_dir.trim().is_empty() {
        return Err("Choose your game files first (Setup).".into());
    }
    // a .nup package is played as it is: pkp reads exefs and romfs straight from it
    let (exefs, romfs, romfs_base) = if cfg.dump_dir.trim().ends_with(".nup") && Path::new(cfg.dump_dir.trim()).is_file() {
        (cfg.dump_dir.trim().to_string(), cfg.dump_dir.trim().to_string(), String::new())
    } else {
        let report = dump::scan(&cfg.dump_dir);
        let chosen = report.chosen.ok_or_else(|| format!("The game files at {} are not usable: {}", cfg.dump_dir, report.errors.join(" ")))?;
        (chosen.exefs, chosen.romfs, chosen.romfs_base)
    };
    let dir = game_dir_of(&cfg);
    let tools_dir = exe_dir().join("tools");
    if gamebuild::is_package_b(&tools_dir) {
        match gamebuild::built_state(&dir, &tools_dir) {
            gamebuild::Built::Current => {}
            gamebuild::Built::Stale => return Err(gamebuild::REBUILD_MESSAGE.into()),
            gamebuild::Built::No => return Err("Build the game first (Game files, Set up).".into()),
        }
    }
    if !shaders::is_complete(&dir) {
        return Err("Finish the shader setup first (Game files, Set up).".into());
    }
    // the base game's files now fill in an update that lacks them (#128): its shaders were never extracted, so Set up runs once more
    if !romfs_base.is_empty() {
        let key = shaders::Job { game_dir: dir.clone(), romfs: PathBuf::from(&romfs), romfs_base: Some(PathBuf::from(&romfs_base)), main: PathBuf::new(), jobs: 1, idle: false, log: None, limit: 0 }.romfs_key();
        if shaders::read_marker(&dir).romfs != key {
            return Err("Your update holds only the files it changes, so the launcher now adds the missing ones from the base game. Run Set up again once (Game files, Set up) so their shaders are prepared too.".into());
        }
    }
    settings::ensure_play_paths(&dir, &exefs, &romfs, &romfs_base)?;
    play::start(&app, &dir, cfg.minimize_on_play)
}

/// Shader setup: the running job's progress and its cancel flag.
#[derive(Default)]
struct SetupState {
    progress: Mutex<shaders::Progress>,
    cancel: AtomicBool,
    /// Cancel flag of a running .nup unpack.
    nup_cancel: AtomicBool,
}

#[derive(Serialize)]
struct SetupStatus {
    progress: shaders::Progress,
    /// setup.json says finished and the cache is there
    complete: bool,
    /// an earlier setup started (Set up continues it)
    started: bool,
    /// pkp.exe is from an older release whose game code this one changes: Set up rebuilds it
    rebuild: bool,
}

#[tauri::command]
fn setup_status(app: AppHandle, state: State<Arc<SetupState>>) -> SetupStatus {
    let dir = game_dir_of(&load_config(&app));
    let mut progress = state.progress.lock().unwrap().clone();
    if progress.state.is_empty() {
        progress.state = "idle".into();
    }
    let tools_dir = exe_dir().join("tools");
    let state = if gamebuild::is_package_b(&tools_dir) { gamebuild::built_state(&dir, &tools_dir) } else { gamebuild::Built::Current };
    let started = !shaders::read_marker(&dir).romfs.is_empty() || dir.join("build").join("state.json").is_file();
    SetupStatus {
        progress,
        complete: state == gamebuild::Built::Current && shaders::is_complete(&dir),
        started,
        rebuild: state == gamebuild::Built::Stale,
    }
}

/// First-run setup: (a) the pkp.ini paths, (b) in a model (b) release, pkp.exe built from the player's dump
/// (gamebuild.rs), (c) the shader cache in shader_cache/spirv, on a worker thread with "setup-status" events.
#[tauri::command]
fn start_setup(app: AppHandle, state: State<Arc<SetupState>>, gentle: Option<bool>) -> Result<(), String> {
    let cfg = load_config(&app);
    let gentle = gentle.unwrap_or(cfg.gentle_build);
    let report = dump::scan(&cfg.dump_dir);
    let chosen = report.chosen.ok_or_else(|| "Choose usable game files first.".to_string())?;
    let dir = game_dir_of(&cfg);
    let tools_dir = exe_dir().join("tools");
    let package_b = gamebuild::is_package_b(&tools_dir);
    if !package_b && !dir.join("pkp.exe").is_file() {
        return Err(format!("pkp.exe was not found in {}. Put the launcher next to pkp.exe or set the game folder in Settings.", dump::fwd(&dir)));
    }
    // the build, the shader tools and pkp.ini all write in the game folder: a blocked one fails here, in words (writecheck.rs)
    writecheck::check(&dir).map_err(|e| e.message())?;
    {
        let mut p = state.progress.lock().unwrap();
        if p.state == "running" {
            return Err("Setup is already running.".into());
        }
        *p = shaders::Progress { state: "running".into(), phase: "check".into(), message: "Checking the setup tools...".into(), ..Default::default() };
    }
    settings::ensure_play_paths(&dir, &chosen.exefs, &chosen.romfs, &chosen.romfs_base)?;
    let st = state.inner().clone();
    st.cancel.store(false, Ordering::SeqCst);
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(3);
    let (jobs, build_jobs) = preflight::jobs_for(cpus, gentle);
    let slog = Arc::new(setuplog::SetupLog::open(&dir));
    let log: shaders::LogFn = {
        let l = slog.clone();
        Arc::new(move |t: &str| l.line(t))
    };
    log(&format!(
        "=== Set up started: launcher {}, {} threads, {} jobs{}",
        env!("CARGO_PKG_VERSION"),
        cpus,
        jobs,
        if gentle { ", gentle (idle priority)" } else { "" }
    ));
    log(&format!("game files: {}", dump::fwd(Path::new(&chosen.exefs))));
    let build_job = gamebuild::Job { game_dir: dir.clone(), exefs: PathBuf::from(&chosen.exefs), jobs: build_jobs, idle: gentle, log: Some(log.clone()) };
    let romfs_base = (!chosen.romfs_base.is_empty()).then(|| PathBuf::from(&chosen.romfs_base));
    let job = shaders::Job { game_dir: dir, romfs: PathBuf::from(&chosen.romfs), romfs_base, main: PathBuf::from(&chosen.exefs).join("main"), jobs, idle: gentle, log: Some(log.clone()), limit: 0 };
    std::thread::spawn(move || {
        let mut last = (String::new(), 0u64, String::new());
        let mut emit = |p: shaders::Progress| {
            if setuplog::progress_worth_logging(&mut last, &p.phase, p.done, p.total, &p.message) && p.state == "running" {
                log(&format!("[{}] {}", p.phase, p.message));
            }
            *st.progress.lock().unwrap() = p.clone();
            let _ = app.emit("setup-status", p);
        };
        let result = (|| {
            // Linux: the native Python and Zig that the build and the shader extractor run with, fetched once
            let tools_ready = host::prepare_tools(&tools_dir, &st.cancel, &mut |done, total, message| {
                emit(shaders::Progress { state: "running".into(), phase: "tools".into(), done, total, message })
            })?;
            if !tools_ready {
                return Ok(false);
            }
            // Linux: the translator and the hidden run need Proton; say so now, not after the build
            #[cfg(not(windows))]
            log(&format!("Windows programs run with: {}", host::runner()?.label));
            // no build yet, or one from an older release (stale tools stamp): build pkp.exe (again)
            if package_b && !gamebuild::is_built(&build_job.game_dir, &tools_dir) {
                let tools = gamebuild::find_tools(&tools_dir)?;
                let built = gamebuild::run(&build_job, &tools, &st.cancel, |t| {
                    emit(shaders::Progress { state: "running".into(), phase: t.phase, done: t.done, total: t.total, message: t.message })
                })?;
                if !built {
                    return Ok(false);
                }
            }
            let tools = shaders::find_tools(&tools_dir)?;
            // a finished cache of the same romfs has nothing left to translate (re-extracting would only find that out)
            let same = shaders::read_marker(&job.game_dir).romfs == job.romfs_key();
            let done = same && shaders::is_complete(&job.game_dir);
            if !done && !shaders::run(&job, &tools, &st.cancel, |p| emit(p))? {
                return Ok(false);
            }
            // the shaders the game builds while it runs: one hidden run collects those of the opening; later ones are
            // collected by every normal run, and every Play checks them all (play.rs)
            emit(shaders::Progress {
                state: "running".into(),
                phase: "warmup".into(),
                message: format!("Running the game once in the background to collect the shaders it builds while it runs (about {} s)...", shaders::WARMUP_SECONDS),
                ..Default::default()
            });
            Ok(shaders::prepare(&job.game_dir, &tools_dir, &st.cancel)?.is_some())
        })();
        let end = match result {
            Ok(true) => shaders::Progress { state: "done".into(), phase: "finish".into(), message: "Setup finished.".into(), ..Default::default() },
            Ok(false) => shaders::Progress { state: "cancelled".into(), message: "Setup paused. Set up again continues where it stopped.".into(), ..Default::default() },
            Err(e) => shaders::Progress { state: "error".into(), message: e, ..Default::default() },
        };
        slog.line(&format!("=== Set up ended: {}{}", end.state, if end.message.is_empty() { String::new() } else { format!(": {}", end.message) }));
        emit(end);
    });
    Ok(())
}

/// The checks shown before Set up starts (preflight.rs). Reads the disk and loads Vulkan, so it runs off the UI thread.
#[tauri::command]
async fn preflight(app: AppHandle) -> Result<preflight::Report, String> {
    let dir = game_dir_of(&load_config(&app));
    let tools_dir = exe_dir().join("tools");
    tauri::async_runtime::spawn_blocking(move || {
        let build_pending = gamebuild::is_package_b(&tools_dir) && !gamebuild::is_built(&dir, &tools_dir);
        let facts = preflight::probe(&dir, &preflight::remaining_of(&dir, build_pending));
        let report = preflight::evaluate(&facts);
        // into the setup log too: a failed setup report then shows what the PC looked like
        let log = setuplog::SetupLog::open(&dir);
        for c in &report.checks {
            log.line(&format!("preflight {:?}: {}: {}", c.level, c.title, c.detail));
        }
        report
    })
    .await
    .map_err(|e| e.to_string())
}

/// Opens setup.log (next to pkp.exe) in the default editor.
#[tauri::command]
fn open_setup_log(app: AppHandle) -> Result<(), String> {
    let path = setuplog::path_of(&game_dir_of(&load_config(&app)));
    if !path.is_file() {
        return Err("There is no setup log yet: it is written when Set up starts.".into());
    }
    play::open_with_shell(&path)
}

#[tauri::command]
fn cancel_setup(state: State<Arc<SetupState>>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn dlss_status(app: AppHandle) -> dlss::Status {
    dlss::status(&game_dir_of(&load_config(&app)), &exe_dir().join("tools"))
}

/// Downloads NVIDIA's nvngx_dlss.dll (see dlss.rs) on a worker thread, with "dlss-progress" events.
#[tauri::command]
async fn fetch_dlss(app: AppHandle) -> Result<(), String> {
    let dir = game_dir_of(&load_config(&app));
    let tools_dir = exe_dir().join("tools");
    tauri::async_runtime::spawn_blocking(move || {
        dlss::fetch(&dir, &tools_dir, |p| {
            let _ = app.emit("dlss-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Where the runtime keeps the save and which account folder the game uses, from pkp.ini.
fn save_paths(app: &AppHandle) -> (PathBuf, String) {
    let dir = game_dir_of(&load_config(app));
    let pkp = settings::read(&dir).pkp;
    let get = |k: &str| pkp.get(k).cloned().unwrap_or_default();
    // NXRT_FS_SAVE_DIR overrides the ini in the runtime, so it does here
    let env = std::env::var("NXRT_FS_SAVE_DIR").unwrap_or_default();
    let sd = saveimport::save_dir_of(&dir, &if env.trim().is_empty() { get("fs.save_dir") } else { env });
    let key = saveimport::target_key(&sd, &get("account.uid"));
    (sd, key)
}

/// Eden and Ryujinx saves of the game found on this PC, newest first.
#[tauri::command]
fn save_sources() -> Vec<saveimport::Source> {
    saveimport::find_sources_here()
}

/// Imports a save: `source` is a picked folder, or None for the newest one found in Eden / Ryujinx.
#[tauri::command]
fn import_save(app: AppHandle, state: State<play::PlayState>, source: Option<String>) -> Result<saveimport::Report, String> {
    if state.0.lock().unwrap().state == "running" || saveimport::game_running() {
        return Err("The game is running. Close it first, then import the save.".into());
    }
    let src = match source.filter(|s| !s.trim().is_empty()) {
        Some(p) => saveimport::resolve_picked(Path::new(p.trim()))
            .ok_or_else(|| format!("{} holds no Pokopia save (it needs the G00 and S00 folders).", p.trim()))?,
        None => {
            let found = saveimport::find_sources_here();
            let first = found.first().ok_or("No Pokopia save found in Eden or Ryujinx. Use \"Pick a folder\" instead.")?;
            PathBuf::from(&first.path)
        }
    };
    let (sd, key) = save_paths(&app);
    saveimport::import(&sd, &key, &src)
}

#[tauri::command]
fn open_save_dir(app: AppHandle) -> Result<(), String> {
    let (sd, _) = save_paths(&app);
    fs::create_dir_all(&sd).map_err(|e| e.to_string())?;
    play::open_with_shell(&sd)
}

#[tauri::command]
fn game_status(state: State<play::PlayState>) -> play::Status {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
fn open_game_file(app: AppHandle, name: String) -> Result<(), String> {
    if !play::REPORT_FILES.contains(&name.as_str()) {
        return Err("That file cannot be opened from here.".into());
    }
    let path = game_dir_of(&load_config(&app)).join(&name);
    if !path.is_file() {
        return Err(format!("{} does not exist.", name));
    }
    play::open_with_shell(&path)
}

#[tauri::command]
fn open_game_dir(app: AppHandle) -> Result<(), String> {
    play::open_with_shell(&game_dir_of(&load_config(&app)))
}


/// Test runs only (PKP_LAUNCHER_EVAL_FILE set): the page writes a result next to the eval file, <file>.out.
#[tauri::command]
fn test_report(text: String) -> Result<(), String> {
    let path = std::env::var("PKP_LAUNCHER_EVAL_FILE").map_err(|_| "not a test run".to_string())?;
    std::fs::write(format!("{path}.out"), text).map_err(|e| e.to_string())
}

/// The relay server the launcher runs on a thread (Host button, phase 4). None: not running.
#[derive(Default)]
struct RelayState(Mutex<Option<relay::RelayServer>>);

/// Starts the relay on 0.0.0.0:port (0 or none: the default port). A running one is stopped first.
#[tauri::command]
fn relay_start(state: State<RelayState>, port: Option<u16>, password: Option<String>) -> Result<relay::Status, String> {
    let mut slot = state.0.lock().unwrap();
    if let Some(mut old) = slot.take() {
        old.stop();
    }
    let port = port.filter(|p| *p != 0).unwrap_or(relay::DEFAULT_PORT);
    let cfg = relay::Config { password: password.filter(|p| !p.is_empty()), ..Default::default() };
    let srv = relay::RelayServer::start(("0.0.0.0", port), cfg).map_err(|e| format!("Cannot open port {}: {}", port, e))?;
    let status = srv.status();
    *slot = Some(srv);
    Ok(status)
}

#[tauri::command]
fn relay_stop(state: State<RelayState>) {
    // take it out first: stopping joins threads, the lock is not held meanwhile
    let srv = state.0.lock().unwrap().take();
    if let Some(mut srv) = srv {
        srv.stop();
    }
}

#[tauri::command]
fn relay_status(state: State<RelayState>) -> Option<relay::Status> {
    state.0.lock().unwrap().as_ref().map(|s| s.status())
}

/// LAN address to give visitors, other addresses, and the Windows Public-network warning (read only).
#[tauri::command]
fn lan_report() -> lan::LanReport {
    lan::report()
}

/// A new room code for a host, XXXX-XXXX-XXXX-XXXX (80 bits from the OS random source). The launcher writes it to pkp.ini
/// `[net] room_code` so the game hosts with it and the page can show it before the game starts.
#[tauri::command]
fn room_code_new() -> Result<String, String> {
    relay::room_code::new_code().map(|c| relay::room_code::format(&c)).map_err(|e| format!("Cannot make a room code: {}", e))
}

/// A visitor's typed code: the display form, or the error text to show.
#[tauri::command]
fn room_code_check(code: String) -> Result<String, String> {
    relay::room_code::normalize(&code)
        .map(|c| relay::room_code::format(&c))
        .ok_or_else(|| "A room code has 16 letters and digits: XXXX-XXXX-XXXX-XXXX.".to_string())
}

/// The room code the running game uses, from `<game folder>/room-code.txt` (written by the game when it starts local wireless
/// with `[net] room_code` set, removed when it runs without one). None: no file or no valid code in it.
#[tauri::command]
fn room_code_host(app: AppHandle) -> Option<String> {
    let text = fs::read_to_string(game_dir_of(&load_config(&app)).join("room-code.txt")).ok()?;
    relay::room_code::normalize(text.trim()).map(|c| relay::room_code::format(&c))
}

/// `--host [--port N] [--password P] [--no-udp]`: the relay alone, no window, log lines on stdout, until Ctrl+C.
/// `--no-udp`: game datagrams only over TCP (no UDP socket on the port).
fn run_headless(args: &[String]) -> i32 {
    #[cfg(windows)]
    win_console::attach_and_hook_ctrl_c();
    let mut port = relay::DEFAULT_PORT;
    let mut password = None;
    let mut udp = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--host" => {}
            "--no-udp" => udp = false,
            "--port" => match it.next().and_then(|p| p.parse::<u16>().ok()) {
                Some(p) => port = p,
                None => {
                    eprintln!("--port needs a number (1-65535)");
                    return 2;
                }
            },
            "--password" => match it.next() {
                Some(p) => password = Some(p.clone()).filter(|p| !p.is_empty()),
                None => {
                    eprintln!("--password needs a value");
                    return 2;
                }
            },
            other => {
                eprintln!("unknown argument {}. Usage: PokopiaLauncher --host [--port N] [--password P] [--no-udp]", other);
                return 2;
            }
        }
    }
    let log: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|line: &str| println!("{}", line));
    let cfg = relay::Config { password, log: Some(log), udp, ..Default::default() };
    let mut srv = match relay::RelayServer::start(("0.0.0.0", port), cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open port {}: {}", port, e);
            return 1;
        }
    };
    println!("Pokopia relay running on {}. Ctrl+C stops it.", srv.local_addr());
    for line in lan::log_lines(&lan::report(), srv.local_addr().port()) {
        println!("{}", line);
    }
    while !win_console::CTRL_C.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    srv.stop();
    0
}

/// A GUI-subsystem exe has no console: attach to the terminal that started it, and turn Ctrl+C into a clean stop.
mod win_console {
    use std::sync::atomic::AtomicBool;
    pub static CTRL_C: AtomicBool = AtomicBool::new(false);

    #[cfg(windows)]
    pub fn attach_and_hook_ctrl_c() {
        use std::sync::atomic::Ordering;
        const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
        extern "system" {
            fn AttachConsole(pid: u32) -> i32;
            fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
        }
        unsafe extern "system" fn on_ctrl(_kind: u32) -> i32 {
            CTRL_C.store(true, Ordering::SeqCst);
            1 // handled: main() stops the server and returns
        }
        // fails harmlessly when there is no parent console or the exe already has one (debug builds)
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
            SetConsoleCtrlHandler(Some(on_ctrl), 1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--host") {
        std::process::exit(run_headless(&args));
    }
    // `--play [--game <folder>]`: starts the game as Play does, without the window (a Steam or Sunshine shortcut). The game
    // folder is the launcher's own unless given: the one set in Settings lives in the window's config, not read here.
    if args.iter().any(|a| a == "--play") {
        #[cfg(windows)]
        win_console::attach_and_hook_ctrl_c();
        let game = args.iter().position(|a| a == "--game").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(exe_dir);
        std::process::exit(play::run_cli(&game, &exe_dir().join("tools")));
    }
    // WebKitGTK's DMA-BUF renderer leaves the window blank or kills it ("Error 71 (Protocol error)") on NVIDIA's driver
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() && Path::new("/sys/module/nvidia").exists() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(play::PlayState::default())
        .manage(Arc::new(SetupState::default()))
        .manage(RelayState::default())
        .setup(|app| {
            // Test runs drive the page without input: PKP_LAUNCHER_EVAL_FILE=<path>. A JS file written there is run in the page once and
            // deleted; it reports back through document.title (the window title). WebView2's DevTools port does not open for Tauri apps here.
            if let Ok(path) = std::env::var("PKP_LAUNCHER_EVAL_FILE") {
                if let Some(win) = app.get_webview_window("main") {
                    std::thread::spawn(move || loop {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        if let Ok(js) = std::fs::read_to_string(&path) {
                            let _ = std::fs::remove_file(&path);
                            let _ = win.eval(&js);
                        }
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            test_report,
            get_config,
            check_update,
            open_release_url,
            save_config,
            get_overview,
            pick_folder,
            validate_dump,
            pick_nup,
            import_nup,
            cancel_nup,
            save_dump,
            read_settings,
            write_settings,
            play,
            setup_status,
            start_setup,
            preflight,
            open_setup_log,
            cancel_setup,
            dlss_status,
            fetch_dlss,
            game_status,
            save_sources,
            import_save,
            open_save_dir,
            open_game_file,
            open_game_dir,
            relay_start,
            relay_stop,
            relay_status,
            lan_report,
            room_code_new,
            room_code_check,
            room_code_host,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the launcher");
}
