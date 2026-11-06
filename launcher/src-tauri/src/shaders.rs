// SPDX-License-Identifier: GPL-3.0-or-later
//! First-run shader setup: builds `shader_cache/spirv` next to pkp.exe from the user's own romfs and main.
//!
//! Two existing tools do the work, both shipped in the launcher's `tools/` folder:
//!   1. `tools/shaders/extract_corpus.py` (Python 3 with the `lz4` package) finds every GLSLC output in the romfs and in
//!      exefs/main and writes the distinct shader blocks to `shader_cache/corpus` (code/, ctrl/, glslc/, unique.tsv);
//!   2. `tools/spirv_harness/spirv_harness.exe` (self-contained .NET publish) translates them to `<hash>.spv` plus the
//!      `results.tsv` rows the runtime reads.
//!
//! Resumable: the full block list is kept as `corpus/unique_all.tsv`; translation runs in chunks (the harness reads
//! `corpus/unique.tsv`, rewritten per chunk) into `shader_cache/stage`, and a chunk counts only once its `.spv` files are
//! moved into `spirv/` and its rows appended to `spirv/results.tsv`. A block with a row there is never translated again.
//! `shader_cache/setup.json` records what is done. The tools run at below-normal priority.
//! On Linux the extractor runs with the Linux Python and the translator (a Windows program) through Proton (host.rs).
//! Its only crate-local import is host.rs, which has none, so `examples/shader_setup.rs` can build the two files alone.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Folder (in the game folder) the runtime opens by default; also what setup writes as `[nvn] shader_cache`.
pub const SPIRV_REL: &str = "shader_cache/spirv";
/// `[nvn] dump_missing`: the runtime writes the blocks the cache has no translation for here (the game builds some while
/// it runs, so the romfs corpus cannot have them). The launcher translates them after the game exits.
pub const MISSING_REL: &str = "shader_cache/missing";
/// Every run-time block seen so far, kept with its code (manifest.tsv, code_<hash>.bin, ctrl_<hash>.bin), so each Play
/// can check that all of them are translated without running the game.
pub const RUNTIME_REL: &str = "shader_cache/runtime";
/// Blocks per translator run. Under Proton every run also starts Proton (a few seconds), so Linux runs fewer, larger chunks.
const CHUNK: usize = if cfg!(windows) { 1000 } else { 5000 };

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Marker {
    /// romfs the corpus was extracted from
    pub romfs: String,
    pub extracted: bool,
    pub blocks: usize,
    pub complete: bool,
}

#[derive(Serialize, Clone, Default)]
pub struct Progress {
    /// "idle", "running", "done", "error", "cancelled"
    pub state: String,
    /// "check", "extract", "translate", "finish"
    pub phase: String,
    pub done: u64,
    pub total: u64,
    pub message: String,
}

pub struct Tools {
    pub python: PathBuf,
    pub extract: PathBuf,
    pub harness: PathBuf,
}

/// Where setup writes its log lines (the launcher's setup.log); a closure so this file needs no crate-local import.
pub type LogFn = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

pub struct Job {
    pub game_dir: PathBuf,
    pub romfs: PathBuf,
    /// The base game's romfs layered under `romfs` (an update dumped without the base game's files under it); None: romfs alone.
    pub romfs_base: Option<PathBuf>,
    pub main: PathBuf,
    pub jobs: usize,
    /// "Gentle build": the tools run at idle priority (the default is below normal).
    pub idle: bool,
    pub log: Option<LogFn>,
    /// Test runs: translate at most this many blocks (0 = all); the setup is then never marked complete.
    pub limit: usize,
}

impl Job {
    /// What the marker records as the extracted romfs: the romfs, and the base romfs under it when there is one.
    pub fn romfs_key(&self) -> String {
        let f = |p: &Path| p.to_string_lossy().replace('\\', "/");
        match &self.romfs_base {
            Some(b) => format!("{} over {}", f(&self.romfs), f(b)),
            None => f(&self.romfs),
        }
    }
}

pub fn cache_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("shader_cache")
}

pub fn read_marker(game_dir: &Path) -> Marker {
    fs::read_to_string(cache_dir(game_dir).join("setup.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_marker(game_dir: &Path, m: &Marker) -> Result<(), String> {
    // through a temporary file: a marker cut short by a crash would read back as no progress and start the hours-long setup over
    let p = cache_dir(game_dir).join("setup.json");
    let tmp = cache_dir(game_dir).join("setup.json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(m).unwrap_or_default()).map_err(|e| format!("cannot write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, &p).map_err(|e| format!("cannot replace {}: {}", p.display(), e))
}

/// True when a finished setup's cache is in place.
pub fn is_complete(game_dir: &Path) -> bool {
    read_marker(game_dir).complete && game_dir.join(SPIRV_REL).join("results.tsv").is_file()
}

fn quiet(cmd: &mut Command, idle: bool) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS, or IDLE_PRIORITY_CLASS when idle (the tools' own child processes inherit it)
        cmd.creation_flags(0x0800_0000 | if idle { 0x0000_0040 } else { 0x0000_4000 });
    }
    #[cfg(not(windows))]
    crate::host::lower_priority(cmd, idle);
}

fn python_has_lz4(py: &Path) -> bool {
    let mut c = crate::host::command(py);
    c.args(["-I", "-c", "import lz4.block"]).stdout(Stdio::null()).stderr(Stdio::null());
    quiet(&mut c, false);
    c.status().map(|s| s.success()).unwrap_or(false)
}

fn on_path(exe: &str) -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).filter(|f| f.is_file()).collect()).unwrap_or_default()
}

/// Finds the tools in `tools_dir` (the launcher's `tools/` folder). Python: `tools/python/python.exe` (Linux:
/// `tools/linux/python/bin/python3`), else the first python.exe (python3) on PATH that has `lz4`. Every missing piece is named
/// in the error.
pub fn find_tools(tools_dir: &Path) -> Result<Tools, String> {
    let mut missing = Vec::new();
    let harness = tools_dir.join("spirv_harness").join("spirv_harness.exe");
    if !harness.is_file() {
        missing.push(format!("the shader translator ({})", harness.display()));
    }
    let extract = tools_dir.join("shaders").join("extract_corpus.py");
    if !extract.is_file() {
        missing.push(format!("the shader extractor ({})", extract.display()));
    }
    let bundled = crate::host::python(tools_dir);
    let mut candidates = vec![bundled.clone()];
    candidates.extend(on_path(if cfg!(windows) { "python.exe" } else { "python3" }));
    let python = candidates.into_iter().filter(|p| p.is_file()).find(|p| python_has_lz4(p));
    if python.is_none() {
        missing.push(format!(
            "Python 3 with the lz4 package ({}, or python.exe on PATH with `pip install lz4`)",
            bundled.display()
        ));
    }
    if !missing.is_empty() {
        return Err(format!("Shader setup cannot run, missing: {}. Re-extract the launcher's release folder so tools/ is complete.", missing.join("; ")));
    }
    Ok(Tools { python: python.unwrap(), extract, harness })
}

/// A progress line of a tool ("scan: 12 of 99", "  500/1000  3.2 s"): counted by the progress bar, not worth a log line.
fn is_progress_line(l: &str) -> bool {
    l.starts_with("scan: ") || l.trim().split('/').next().map_or(false, |x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()) && l.contains('/'))
}

/// Runs a tool, handing each stdout line to `line`; kills it when `cancel` is set. Ok(true) = exit 0, Ok(false) = cancelled.
/// `log` gets the command line, the tool's stderr and non-progress stdout, and on failure the end of its output.
/// `limit`: killed after that long, an error.
fn run_tool(mut cmd: Command, cancel: &AtomicBool, idle: bool, log: Option<&LogFn>, limit: Option<Duration>, mut line: impl FnMut(&str)) -> Result<bool, String> {
    quiet(&mut cmd, idle);
    let name = cmd.get_program().to_string_lossy().to_string();
    let say = |t: &str| {
        if let Some(f) = log {
            f(t)
        }
    };
    say(&format!("run: {} {}", name, cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>().join(" ")));
    let mut child: Child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not start {}: {}", name, e))?;
    let (tx, rx) = mpsc::channel::<String>();
    let out = child.stdout.take().unwrap();
    let tx2 = tx.clone();
    std::thread::spawn(move || BufReader::new(out).lines().map_while(Result::ok).for_each(|l| drop(tx2.send(l))));
    let err = child.stderr.take().unwrap();
    let (etx, erx) = mpsc::channel::<String>();
    let elog = log.cloned();
    std::thread::spawn(move || {
        let mut tail: Vec<String> = Vec::new();
        for l in BufReader::new(err).lines().map_while(Result::ok) {
            if let Some(f) = &elog {
                f(&format!("stderr: {}", l));
            }
            tail.push(l);
            if tail.len() > 12 {
                tail.remove(0);
            }
        }
        let _ = etx.send(tail.join("\n"));
    });
    drop(tx);
    let deadline = limit.map(|d| std::time::Instant::now() + d);
    let mut out_tail: Vec<String> = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(l) => {
                if !is_progress_line(&l) {
                    say(&format!("out: {}", l));
                }
                out_tail.push(l.clone());
                if out_tail.len() > 12 {
                    out_tail.remove(0);
                }
                line(&l)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // stdout closed: the process is ending. Polled, so Pause and the time limit still apply while it does (Proton's
                // wrapper waits for every wine process of the prefix)
                if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
                    if cancel.load(Ordering::SeqCst) {
                        say(&format!("stopped: {} (paused by the player)", name));
                        return Ok(false);
                    }
                    if st.success() {
                        say(&format!("done: {}", name));
                        return Ok(true);
                    }
                    let tail = erx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
                    say(&format!("FAILED: {} ({}). Last output:\n{}\nLast error output:\n{}", name, st, out_tail.join("\n"), tail));
                    return Err(format!("{} failed ({}). {}", name, st, tail.trim()));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        if cancel.load(Ordering::SeqCst) {
            crate::host::kill(&mut child);
            let _ = child.wait();
            say(&format!("stopped: {} (paused by the player)", name));
            return Ok(false);
        }
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            crate::host::kill(&mut child);
            let _ = child.wait();
            say(&format!("stopped: {} (still running after {} s)", name, limit.unwrap_or_default().as_secs()));
            return Err(format!("{} did not end in time", name));
        }
    }
}

/// Header + rows of a tsv file (rows without the trailing newline).
fn read_tsv(p: &Path) -> Result<(String, Vec<String>), String> {
    let text = fs::read_to_string(p).map_err(|e| format!("cannot read {}: {}", p.display(), e))?;
    let mut it = text.lines();
    let header = it.next().unwrap_or("").to_string();
    Ok((header, it.filter(|l| !l.is_empty()).map(|l| l.to_string()).collect()))
}

fn first_col(row: &str) -> &str {
    row.split('\t').next().unwrap_or("")
}

/// Moves a harness run's .spv files from `stage` into `spirv`, then appends its rows to spirv/results.tsv (the rows are
/// what make a block count as done, so they go last). Returns the number of rows.
fn commit_stage(stage: &Path, spirv: &Path) -> Result<usize, String> {
    let results = spirv.join("results.tsv");
    let (rheader, rrows) = read_tsv(&stage.join("results.tsv"))?;
    for r in &rrows {
        let h = first_col(r);
        let src = stage.join(format!("{}.spv", h));
        if src.is_file() {
            let dst = spirv.join(format!("{}.spv", h));
            let _ = fs::remove_file(&dst);
            fs::rename(&src, &dst).map_err(|e| format!("cannot move {}: {}", src.display(), e))?;
        }
    }
    let new_file = !results.is_file();
    let mut out = fs::OpenOptions::new().create(true).append(true).open(&results).map_err(|e| format!("cannot write {}: {}", results.display(), e))?;
    let mut text = String::new();
    if new_file {
        text.push_str(&rheader);
        text.push('\n');
    }
    for r in &rrows {
        text.push_str(r);
        text.push('\n');
    }
    out.write_all(text.as_bytes()).map_err(|e| format!("cannot write {}: {}", results.display(), e))?;
    Ok(rrows.len())
}

fn col(row: &str, k: usize) -> &str {
    row.split('\t').nth(k).unwrap_or("")
}

/// True when no run-time block was ever collected for this cache: the warm-up run has not happened yet.
pub fn needs_warm_up(game_dir: &Path) -> bool {
    !game_dir.join(RUNTIME_REL).join("manifest.tsv").is_file()
}

/// Before every start: moves what the game dumped to MISSING_REL (by any run, also one started without the launcher)
/// into RUNTIME_REL, then translates every block listed there that has no .spv in the cache. Ok(n) = blocks translated.
pub fn translate_missing(game_dir: &Path, tools_dir: &Path) -> Result<usize, String> {
    let missing = game_dir.join(MISSING_REL);
    let runtime = game_dir.join(RUNTIME_REL);
    let list = runtime.join("manifest.tsv");
    // manifest columns (runtime nvn_shaders.cpp dump_missing_block): 10 = hash, 11 = code file, 12 = control file
    if let Ok((header, rows)) = read_tsv(&missing.join("manifest.tsv")) {
        fs::create_dir_all(&runtime).map_err(|e| format!("cannot create {}: {}", runtime.display(), e))?;
        let mut known: HashSet<String> = read_tsv(&list).map(|(_, r)| r.iter().map(|x| col(x, 10).to_string()).collect()).unwrap_or_default();
        let mut text = if fs::metadata(&list).map(|m| m.len() == 0).unwrap_or(true) { format!("{}\n", header) } else { String::new() };
        for r in &rows {
            // the same block twice (several runs appended to one manifest before it was merged): the first one counts
            if !known.insert(col(r, 10).to_string()) {
                continue;
            }
            for f in [col(r, 11), col(r, 12)] {
                fs::rename(missing.join(f), runtime.join(f)).map_err(|e| format!("cannot move {}: {}", f, e))?;
            }
            text.push_str(r);
            text.push('\n');
        }
        let mut out = fs::OpenOptions::new().create(true).append(true).open(&list).map_err(|e| format!("cannot write {}: {}", list.display(), e))?;
        out.write_all(text.as_bytes()).map_err(|e| format!("cannot write {}: {}", list.display(), e))?;
        let _ = fs::remove_dir_all(&missing);
    }

    let spirv = game_dir.join(SPIRV_REL);
    let Ok((header, rows)) = read_tsv(&list) else { return Ok(0) };
    // ponytail: a block the translator fails on has no .spv and is tried again at every start (a second or two)
    let todo: Vec<&String> = rows.iter().filter(|r| !spirv.join(format!("{}.spv", col(r, 10))).is_file()).collect();
    if todo.is_empty() {
        return Ok(0);
    }
    // the harness reads the code and control files relative to its manifest: absolute paths (as the translator sees them)
    // point it at RUNTIME_REL
    let input = cache_dir(game_dir).join("runtime_todo");
    let stage = cache_dir(game_dir).join("stage_missing");
    let _ = fs::remove_dir_all(&input);
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&input).map_err(|e| format!("cannot create {}: {}", input.display(), e))?;
    let mut text = format!("{}\n", header);
    for r in &todo {
        let mut f: Vec<String> = r.split('\t').map(String::from).collect();
        for k in [11, 12] {
            if let Some(v) = f.get_mut(k) {
                *v = crate::host::win_path(&runtime.join(&*v)).to_string_lossy().replace('\\', "/");
            }
        }
        text.push_str(&f.join("\t"));
        text.push('\n');
    }
    fs::write(input.join("manifest.tsv"), text).map_err(|e| format!("cannot write {}: {}", input.display(), e))?;
    let jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).to_string();
    let mut cmd = crate::host::windows_program(&tools_dir.join("spirv_harness").join("spirv_harness.exe"))?;
    cmd.arg("--batch").arg(crate::host::win_path(&input)).arg("--out").arg(crate::host::win_path(&stage)).args(["--jobs", &jobs, "--no-val"]);
    cmd.env("DOTNET_gcServer", "1");
    if !run_tool(cmd, &AtomicBool::new(false), false, None, None, |_| {})? {
        return Err("the shader translator stopped".into());
    }
    let n = commit_stage(&stage, &spirv)?;
    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&input);
    Ok(n)
}

/// Seconds the warm-up run plays. The opening asks for its run-time blocks about 3 to 25 s after start.
pub const WARMUP_SECONDS: u32 = 60;

/// Plays the game once without a window or sound, for WARMUP_SECONDS, so the blocks it builds while it runs (the romfs
/// corpus cannot have them) are collected and translated before a real start. A fresh save folder in the cache, so the
/// player's own save is never touched. Ok(None) = cancelled, else how many blocks were translated.
pub fn warm_up(game_dir: &Path, tools_dir: &Path, cancel: &AtomicBool) -> Result<Option<usize>, String> {
    let scratch = cache_dir(game_dir).join("warmup");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(scratch.join("save")).map_err(|e| format!("cannot create {}: {}", scratch.display(), e))?;
    let mut cmd = crate::host::windows_program(&game_dir.join("pkp.exe"))?;
    cmd.current_dir(game_dir).arg("--headless").args(["--max-seconds", &WARMUP_SECONDS.to_string()]).arg("--log").arg(crate::host::win_path(&scratch.join("warmup.log")));
    // environment variables win over pkp.ini
    cmd.env("NXRT_FS_SAVE_DIR", crate::host::win_path(&scratch.join("save"))).env("NXRT_NVN_DUMP_MISSING", MISSING_REL).env("NXRT_AUDIO_SINK", "null");
    // it ends with exit code 124 when the time is up: any end is fine, a crash only means fewer blocks were collected. One
    // still running 4 minutes later (a first Proton start also makes its prefix) is stopped.
    if let Ok(false) = run_tool(cmd, cancel, false, None, Some(Duration::from_secs(WARMUP_SECONDS as u64 + 240)), |_| {}) {
        return Ok(None);
    }
    let n = translate_missing(game_dir, tools_dir)?;
    // an empty list still records that the warm-up ran: a run that collects nothing is not repeated at every start
    let list = game_dir.join(RUNTIME_REL).join("manifest.tsv");
    if !list.is_file() {
        let _ = fs::create_dir_all(game_dir.join(RUNTIME_REL));
        fs::write(&list, "").map_err(|e| format!("cannot write {}: {}", list.display(), e))?;
    }
    let _ = fs::remove_dir_all(&scratch);
    Ok(Some(n))
}

/// What every start and every setup does: the warm-up run once, afterwards only the check of the collected blocks.
pub fn prepare(game_dir: &Path, tools_dir: &Path, cancel: &AtomicBool) -> Result<Option<usize>, String> {
    if needs_warm_up(game_dir) {
        warm_up(game_dir, tools_dir, cancel)
    } else {
        translate_missing(game_dir, tools_dir).map(Some)
    }
}

/// The whole setup. `report` gets every progress change. Ok(true) = finished (or the limit reached), Ok(false) = cancelled.
pub fn run(job: &Job, tools: &Tools, cancel: &AtomicBool, mut report: impl FnMut(Progress)) -> Result<bool, String> {
    let cache = cache_dir(&job.game_dir);
    let corpus = cache.join("corpus");
    let stage = cache.join("stage");
    let spirv = job.game_dir.join(SPIRV_REL);
    fs::create_dir_all(&spirv).map_err(|e| format!("cannot create {}: {}", spirv.display(), e))?;
    let romfs_s = job.romfs_key();
    let mut marker = read_marker(&job.game_dir);
    if marker.romfs != romfs_s {
        marker = Marker { romfs: romfs_s.clone(), ..Default::default() };
    }
    marker.complete = false;
    write_marker(&job.game_dir, &marker)?;
    let jobs = job.jobs.max(1).to_string();
    let p = |phase: &str, done: u64, total: u64, message: String| Progress { state: "running".into(), phase: phase.into(), done, total, message };

    // 1. extract (skipped when this romfs was already extracted)
    let all = corpus.join("unique_all.tsv");
    if !(marker.extracted && all.is_file()) {
        report(p("extract", 0, 0, "Finding the game's shaders in your game files...".into()));
        let _ = fs::remove_file(corpus.join("unique.tsv"));
        let mut cmd = crate::host::command(&tools.python);
        cmd.arg("-I").arg(&tools.extract).arg("--romfs").arg(&job.romfs).arg("--main").arg(&job.main).arg("--out").arg(&corpus);
        if let Some(b) = &job.romfs_base {
            cmd.arg("--romfs-base").arg(b);
        }
        cmd.args(["--jobs", &jobs, "--progress"]);
        let ok = run_tool(cmd, cancel, job.idle, job.log.as_ref(), None, |l| {
            if let Some(rest) = l.strip_prefix("scan: ") {
                let mut it = rest.split_whitespace().filter_map(|x| x.parse::<u64>().ok());
                if let (Some(d), Some(t)) = (it.next(), it.next()) {
                    report(p("extract", d, t, format!("Scanning game files: {} of {}", d, t)));
                }
            }
        })?;
        if !ok {
            return Ok(false);
        }
        fs::rename(corpus.join("unique.tsv"), &all).map_err(|e| format!("the extractor wrote no block list: {}", e))?;
        marker.extracted = true;
        write_marker(&job.game_dir, &marker)?;
    }

    // 2. translate what spirv/results.tsv does not list yet
    let (uheader, urows) = read_tsv(&all)?;
    marker.blocks = urows.len();
    write_marker(&job.game_dir, &marker)?;
    let results = spirv.join("results.tsv");
    let done: HashSet<String> = if results.is_file() { read_tsv(&results)?.1.iter().map(|r| first_col(r).to_string()).collect() } else { HashSet::new() };
    let mut todo: Vec<&String> = urows.iter().filter(|r| !done.contains(first_col(r))).collect();
    let limited = job.limit > 0 && todo.len() > job.limit;
    if job.limit > 0 {
        todo.truncate(job.limit);
    }
    let total = urows.len() as u64;
    let mut finished = (urows.len() - urows.iter().filter(|r| !done.contains(first_col(r))).count()) as u64;
    report(p("translate", finished, total, format!("Translating shaders: {} of {}", finished, total)));
    for chunk in todo.chunks(CHUNK) {
        let mut f = fs::File::create(corpus.join("unique.tsv")).map_err(|e| e.to_string())?;
        writeln!(f, "{}", uheader).map_err(|e| e.to_string())?;
        for r in chunk {
            writeln!(f, "{}", r).map_err(|e| e.to_string())?;
        }
        drop(f);
        let _ = fs::remove_dir_all(&stage);
        let mut cmd = crate::host::windows_program(&tools.harness)?;
        cmd.arg("--batch").arg(crate::host::win_path(&corpus)).arg("--out").arg(crate::host::win_path(&stage)).args(["--jobs", &jobs, "--no-val"]);
        // server GC: with the default workstation GC each collection stops every worker (about 4x slower)
        cmd.env("DOTNET_gcServer", "1");
        let base = finished;
        let ok = run_tool(cmd, cancel, job.idle, job.log.as_ref(), None, |l| {
            // "  <n>/<count>  <s> s" every 500 blocks
            if let Some(n) = l.trim().split('/').next().and_then(|x| x.parse::<u64>().ok()) {
                report(p("translate", base + n, total, format!("Translating shaders: {} of {}", base + n, total)));
            }
        })?;
        if !ok {
            return Ok(false);
        }
        commit_stage(&stage, &spirv)?;
        finished += chunk.len() as u64;
        report(p("translate", finished, total, format!("Translating shaders: {} of {}", finished, total)));
    }
    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_file(corpus.join("unique.tsv"));
    if limited {
        return Ok(true);
    }

    // 3. done: the corpus is only needed to translate, drop it (several GB)
    report(p("finish", total, total, "Cleaning up...".into()));
    let _ = fs::remove_dir_all(&corpus);
    marker.extracted = false;
    marker.complete = true;
    write_marker(&job.game_dir, &marker)?;
    Ok(true)
}
