// SPDX-License-Identifier: GPL-3.0-or-later
//! What the launcher does differently on Linux (SteamOS, the Steam Deck, Bazzite, ...). The game stays a Windows program.
//!
//! On Windows the release's Windows tools run and pkp.exe starts directly. On Linux the launcher is a native program (Tauri
//! draws it with WebKitGTK, so it needs no WebView2), the tools that only prepare files run as Linux programs from
//! `tools/linux`: Python 3.13 with lz4 and capstone (pkp_build.py, the lifter, extract_corpus.py) and Zig 0.17.0, the version
//! the release bundles for Windows, which cross-compiles the same pkp.exe. `prepare_tools` fetches them once, each pinned by
//! size and SHA-256. The two Windows programs, the shader translator and pkp.exe, run through `runner`: Steam's Proton inside
//! the Steam Linux Runtime, as Steam starts a game (PKP_PROTON picks another Proton folder), else umu-run, else Wine. There
//! the Linux root is drive Z:, so paths handed to them go through `win_path`.
//! Steam's Proton is found as tinfoil-chewer/pokopia-recomp-linux (GPL-3.0) linux/pkp_linux.py `find_proton` finds it.
//! No crate-local imports, so the examples can build this file alone next to gamebuild.rs and shaders.rs.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::AtomicBool;

/// The Python that runs the release's scripts (pkp_build.py, extract_corpus.py, fetch_dlss.py).
pub fn python(tools_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        tools_dir.join("python").join("python.exe")
    } else {
        tools_dir.join("linux").join("python").join("bin").join("python3")
    }
}

/// The Zig pkp_build.py compiles and links with (the script finds the same path on its own).
pub fn zig(tools_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        tools_dir.join("build").join("zig").join("zig.exe")
    } else {
        tools_dir.join("linux").join("zig").join("zig")
    }
}

/// A command for a program of this system (Python, tar, Proton, xdg-open). In an AppImage, whose runtime points
/// variables such as GTK_PATH, GIO_EXTRA_MODULES or XDG_DATA_DIRS into the AppImage for the launcher's own libraries, the
/// program gets the environment without those entries (and without APPDIR, APPIMAGE, ARGV0, OWD); elsewhere nothing
/// changes. After tinfoil-chewer/pokopia-recomp-linux (GPL-3.0) launcher/src-tauri/src/linux.rs `host_env`.
pub fn command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    let appdir = std::env::var("APPDIR").unwrap_or_default();
    let vars = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)));
    for (k, v) in appimage_env_fixes(&appdir, vars) {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    cmd
}

/// What `command` changes for `appdir` (empty: not an AppImage, nothing): (variable, Some(new value) or None = removed).
fn appimage_env_fixes(appdir: &str, vars: impl Iterator<Item = (String, String)>) -> Vec<(String, Option<String>)> {
    let appdir = appdir.trim_end_matches('/');
    if appdir.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (k, v) in vars {
        if matches!(k.as_str(), "APPDIR" | "APPIMAGE" | "ARGV0" | "OWD") {
            out.push((k, None));
        } else if v.contains(appdir) {
            // a list keeps its other entries; a single path into the AppImage goes
            let kept: Vec<&str> = v.split(':').filter(|e| !e.is_empty() && !e.contains(appdir)).collect();
            out.push((k, (v.contains(':') && !kept.is_empty()).then(|| kept.join(":"))));
        }
    }
    out
}

/// A command that runs the Windows program `exe`: itself on Windows, through `runner` on Linux.
pub fn windows_program(exe: &Path) -> Result<Command, String> {
    #[cfg(windows)]
    {
        Ok(Command::new(exe))
    }
    #[cfg(not(windows))]
    {
        let r = runner()?;
        let mut cmd = command(&r.argv[0]);
        cmd.args(&r.argv[1..]).arg(exe);
        for (k, v) in &r.env {
            cmd.env(k, v);
        }
        Ok(cmd)
    }
}

/// `p` as a Windows program sees it: unchanged on Windows, `Z:\...` under Proton and Wine (Z: is the Linux root there).
pub fn win_path(p: &Path) -> OsString {
    #[cfg(windows)]
    {
        p.as_os_str().to_os_string()
    }
    #[cfg(not(windows))]
    {
        let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf()) };
        OsString::from(format!("Z:{}", abs.to_string_lossy().replace('/', "\\")))
    }
}

/// Stops a tool and what it started: on Linux the compilers, or Proton's wine processes (`lower_priority` made it the
/// leader of its own process group).
pub fn kill(child: &mut Child) {
    #[cfg(not(windows))]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}

/// Linux: the tool runs niced (10, idle 19; its children inherit it) in a process group of its own, for `kill`.
#[cfg(not(windows))]
pub fn lower_priority(cmd: &mut Command, idle: bool) {
    use std::os::unix::process::CommandExt;
    let n = if idle { 19 } else { 10 };
    cmd.process_group(0);
    // SAFETY: nice(2) is async-signal-safe, which is all pre_exec asks of the closure
    unsafe {
        cmd.pre_exec(move || {
            libc::nice(n);
            Ok(())
        });
    }
}

/// Free bytes on the file system that holds `dir` (or its nearest existing parent).
#[cfg(not(windows))]
pub fn free_bytes(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let mut d = dir;
    while !d.exists() {
        d = d.parent()?;
    }
    let c = std::ffi::CString::new(d.as_os_str().as_bytes()).ok()?;
    // SAFETY: c is NUL-terminated and s is a valid out pointer for the call
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0).then(|| s.f_bavail as u64 * s.f_frsize as u64)
}

/// Opens a file in its default app, a folder in the file manager, or a link in the browser.
pub fn open(target: &std::ffi::OsStr) -> Result<(), String> {
    let opener = if cfg!(windows) { "explorer.exe" } else { "xdg-open" };
    let mut child = command(opener).arg(target).spawn().map_err(|e| format!("Could not open {}: {}", target.to_string_lossy(), e))?;
    // reaped, so no exited opener lingers as a zombie while the launcher runs
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// How the Windows programs run on Linux: `argv` before the program's path, extra environment, and a name for the setup
/// log and the checks before Set up.
#[cfg(not(windows))]
pub struct Runner {
    pub argv: Vec<OsString>,
    pub env: Vec<(&'static str, OsString)>,
    pub label: String,
}

/// `$XDG_DATA_HOME/pokopia-recomp` (~/.local/share/pokopia-recomp): the Proton and Wine prefixes, kept out of the game folder.
#[cfg(not(windows))]
fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("pokopia-recomp")
}

#[cfg(not(windows))]
fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(not(windows))]
fn which(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(exe)).find(|f| f.is_file()))
}

/// The GPU driver's shader caches, set up as Steam sets them up for a game it starts: a folder of the game's own that the
/// driver never trims. The shared default (~/.cache/nvidia, ~/.cache/mesa_shader_cache) is trimmed to a size limit by every
/// program using it, and pipelines trimmed away are compiled again at the draw (20-40 ms each). A variable already set wins.
#[cfg(not(windows))]
fn shader_cache_env() -> Vec<(&'static str, OsString)> {
    let dir = data_dir().join("shadercache");
    let (nvidia, mesa) = (dir.join("nvidia"), dir.join("mesa"));
    let _ = std::fs::create_dir_all(&nvidia);
    let _ = std::fs::create_dir_all(&mesa);
    [
        ("__GL_SHADER_DISK_CACHE_PATH", nvidia.into_os_string()),
        ("__GL_SHADER_DISK_CACHE_SKIP_CLEANUP", "1".into()),
        ("MESA_SHADER_CACHE_DIR", mesa.into_os_string()),
        ("MESA_SHADER_CACHE_MAX_SIZE", "10G".into()),
    ]
    .into_iter()
    .filter(|(k, _)| std::env::var_os(k).is_none())
    .collect()
}

/// Steam's Proton, else umu-run, else Wine, each with `shader_cache_env`. Err names what to install.
#[cfg(not(windows))]
pub fn runner() -> Result<Runner, String> {
    let mut r = find_runner()?;
    r.env.extend(shader_cache_env());
    Ok(r)
}

#[cfg(not(windows))]
fn find_runner() -> Result<Runner, String> {
    if let Some(r) = steam_proton() {
        return Ok(r);
    }
    let prefix = |name: &str| {
        let p = data_dir().join(name);
        let _ = std::fs::create_dir_all(&p);
        p.into_os_string()
    };
    if let Some(umu) = which("umu-run") {
        // umu picks (and fetches) its own Proton unless PROTONPATH says which
        return Ok(Runner { argv: vec![umu.into()], env: vec![("WINEPREFIX", prefix("umu")), ("GAMEID", "0".into())], label: "umu-run".into() });
    }
    if let Some(wine) = which("wine") {
        // mscoree stays on: Wine needs it to load the shader translator's .NET assemblies
        let env = vec![("WINEPREFIX", prefix("wine")), ("WINEDEBUG", "-all".into()), ("WINEDLLOVERRIDES", "mshtml=".into())];
        return Ok(Runner { argv: vec![wine.into()], env, label: "Wine".into() });
    }
    Err("Nothing to run the game's Windows programs with was found. Install Steam and its Proton (in Steam: Library, Tools, Proton Experimental), umu-launcher, or Wine.".into())
}

/// The values of `"key" "value"` lines in a Valve KeyValues file (libraryfolders.vdf, toolmanifest.vdf, appmanifest_*.acf).
#[cfg(not(windows))]
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let quoted = format!("\"{}\"", key);
    text.lines()
        .filter_map(|l| l.trim().strip_prefix(quoted.as_str()))
        .filter_map(|rest| rest.trim().strip_prefix('"').and_then(|v| v.strip_suffix('"')))
        .map(|v| v.replace("\\\\", "\\"))
        .collect()
}

/// (Steam root, its library folders) from libraryfolders.vdf. None without Steam.
#[cfg(not(windows))]
fn steam_libraries() -> Option<(PathBuf, Vec<PathBuf>)> {
    for r in [".local/share/Steam", ".steam/steam", ".steam/root", ".var/app/com.valvesoftware.Steam/.local/share/Steam"] {
        let Ok(root) = std::fs::canonicalize(home().join(r)) else { continue };
        let Ok(text) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) else { continue };
        let mut libs: Vec<PathBuf> = Vec::new();
        for lib in std::iter::once(root.clone()).chain(vdf_values(&text, "path").into_iter().map(PathBuf::from)) {
            let Ok(lib) = std::fs::canonicalize(lib) else { continue };
            if lib.join("steamapps").is_dir() && !libs.contains(&lib) {
                libs.push(lib);
            }
        }
        return Some((root, libs));
    }
    None
}

/// PKP_PROTON (a folder with `proton`), else Proton Experimental, else the newest "Proton N.N" Steam installed.
#[cfg(not(windows))]
fn pick_proton(libs: &[PathBuf]) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PKP_PROTON").map(PathBuf::from) {
        return p.join("proton").is_file().then_some(p);
    }
    let rank = |name: &str| -> (u8, Vec<u32>) {
        if name.contains("Experimental") {
            return (2, vec![]);
        }
        let nums: Vec<u32> = name.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse().ok()).collect();
        (if nums.is_empty() { 0 } else { 1 }, nums)
    };
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for lib in libs {
        let Ok(rd) = std::fs::read_dir(lib.join("steamapps").join("common")) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("Proton") && e.path().join("proton").is_file() {
                found.push((name, e.path()));
            }
        }
    }
    found.into_iter().max_by_key(|(n, _)| rank(n)).map(|(_, p)| p)
}

/// Steam's Proton, inside the Steam Linux Runtime container its toolmanifest.vdf asks for when that is installed (as Steam
/// starts a game). Its prefix is `data_dir()/proton` (STEAM_COMPAT_DATA_PATH).
#[cfg(not(windows))]
fn steam_proton() -> Option<Runner> {
    let (root, libs) = steam_libraries()?;
    let pdir = pick_proton(&libs)?;
    let mut argv: Vec<OsString> = vec![pdir.join("proton").into(), "waitforexitandrun".into()];
    let mut label = pdir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Proton".into());
    let manifest = std::fs::read_to_string(pdir.join("toolmanifest.vdf")).unwrap_or_default();
    if let Some(appid) = vdf_values(&manifest, "require_tool_appid").first() {
        for lib in &libs {
            let acf = std::fs::read_to_string(lib.join("steamapps").join(format!("appmanifest_{}.acf", appid))).unwrap_or_default();
            let Some(dir) = vdf_values(&acf, "installdir").into_iter().next() else { continue };
            let entry = lib.join("steamapps").join("common").join(&dir).join("_v2-entry-point");
            if entry.is_file() {
                argv.splice(0..0, [entry.into_os_string(), "--verb=waitforexitandrun".into(), "--".into()]);
                label = format!("{} in {}", label, dir);
                break;
            }
        }
    }
    let data = data_dir().join("proton");
    let _ = std::fs::create_dir_all(&data);
    Some(Runner { argv, env: vec![("STEAM_COMPAT_DATA_PATH", data.into()), ("STEAM_COMPAT_CLIENT_INSTALL_PATH", root.into())], label })
}

/// One download of `prepare_tools`, checked against the size and SHA-256 its publisher lists for it.
struct Pinned {
    file: &'static str,
    url: &'static str,
    size: u64,
    sha256: &'static str,
}

// python-build-standalone 20261013 (its SHA256SUMS), the PyPI wheels the Windows release bundles, Zig 0.17.0 (ziglang.org
// download index). Bump together with the Windows release's tools/python and tools/build/zig.
const PYTHON: Pinned = Pinned {
    file: "cpython-3.13.16+20261013-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz",
    url: "https://github.com/astral-sh/python-build-standalone/releases/download/20261013/cpython-3.13.16%2B20261013-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz",
    size: 35_099_487,
    sha256: "d0080766b6b74aeb81e1949ef0d9a20e6c17f7b160fb295a69b936ccb17660be",
};
const WHEELS: [Pinned; 2] = [
    Pinned {
        file: "lz4-4.4.5-cp313-cp313-manylinux2014_x86_64.manylinux_2_17_x86_64.manylinux_2_28_x86_64.whl",
        url: "https://files.pythonhosted.org/packages/d2/fb/ba9256c48266a09012ed1d9b0253b9aa4fe9cdff094f8febf5b26a4aa2a2/lz4-4.4.5-cp313-cp313-manylinux2014_x86_64.manylinux_2_17_x86_64.manylinux_2_28_x86_64.whl",
        size: 1_368_257,
        sha256: "451039b609b9a88a934800b5fc6ee401c89ad9c175abf2f4d9f8b2e4ef1afc64",
    },
    Pinned {
        file: "capstone-5.0.9-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
        url: "https://files.pythonhosted.org/packages/d1/39/17747862222bb062e86b501f1f148d5ff589b77908b080d30f7f085cbfb7/capstone-5.0.9-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
        size: 1_485_316,
        sha256: "273fd8d747d2e35c88f91450be51a603ecfaafb00d96d9f315dcb8689c86193e",
    },
];
const ZIG: Pinned = Pinned {
    file: "zig-x86_64-linux-0.17.0.tar.xz",
    url: "https://ziglang.org/download/0.17.0/zig-x86_64-linux-0.17.0.tar.xz",
    size: 57_332_648,
    sha256: "1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026",
};

/// The Linux Python can run the release's scripts.
fn python_ready(py: &Path) -> bool {
    let mut c = command(py);
    c.args(["-I", "-c", "import lz4.block, capstone"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c.status().map(|s| s.success()).unwrap_or(false)
}

/// Linux: fetches the native build tools into `tools/linux` (python/ with lz4 and capstone, zig/) unless they are there.
/// `report(done, total, text)` gets the download progress. Ok(false) = cancelled. Windows: nothing to do.
pub fn prepare_tools(tools_dir: &Path, cancel: &AtomicBool, report: &mut dyn FnMut(u64, u64, String)) -> Result<bool, String> {
    if cfg!(windows) {
        return Ok(true);
    }
    let linux = tools_dir.join("linux");
    let (py, zig) = (python(tools_dir), zig(tools_dir));
    let py_ok = python_ready(&py);
    if py_ok && zig.is_file() {
        return Ok(true);
    }
    let dl = linux.join("downloads");
    std::fs::create_dir_all(&dl).map_err(|e| format!("cannot create {}: {}", dl.display(), e))?;
    let mut need: Vec<&Pinned> = Vec::new();
    if !py_ok {
        need.push(&PYTHON);
        need.extend(WHEELS.iter());
    }
    if !zig.is_file() {
        need.push(&ZIG);
    }
    let total: u64 = need.iter().map(|p| p.size).sum();
    let mut done = 0u64;
    for p in &need {
        let mb = |b: u64| b as f64 / (1u64 << 20) as f64;
        let ok = fetch(p, &dl, cancel, &mut |n| {
            report(done + n, total, format!("Downloading the Linux build tools: {:.0} of {:.0} MB", mb(done + n), mb(total)))
        })?;
        if !ok {
            return Ok(false);
        }
        done += p.size;
    }
    if !py_ok {
        report(total, total, "Installing the Linux build tools: Python...".into());
        unpack(&dl.join(PYTHON.file), &linux, "python")?;
        let mut cmd = command(&py);
        cmd.args(["-I", "-m", "pip", "install", "-q", "--no-index", "--no-deps", "--no-compile", "--disable-pip-version-check", "--no-warn-script-location"]);
        cmd.args(WHEELS.iter().map(|w| dl.join(w.file)));
        let out = cmd.output().map_err(|e| format!("could not start {}: {}", py.display(), e))?;
        if !out.status.success() || !python_ready(&py) {
            return Err(format!("Installing lz4 and capstone into {} failed: {}", py.display(), String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    if !zig.is_file() {
        report(total, total, "Installing the Linux build tools: Zig...".into());
        unpack(&dl.join(ZIG.file), &linux, "zig")?;
    }
    let _ = std::fs::remove_dir_all(&dl);
    Ok(true)
}

/// Downloads `p` into `dir` unless it is there. Ok(false) = cancelled. A file that does not match its pin is never kept.
fn fetch(p: &Pinned, dir: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<bool, String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let dst = dir.join(p.file);
    let hex = |h: &[u8]| h.iter().map(|b| format!("{:02x}", b)).collect::<String>();
    if std::fs::read(&dst).map(|b| b.len() as u64 == p.size && hex(&Sha256::digest(&b)) == p.sha256).unwrap_or(false) {
        progress(p.size);
        return Ok(true);
    }
    let part = dir.join(format!("{}.part", p.file));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent(&format!("PokopiaRecomp-Launcher/{}", env!("CARGO_PKG_VERSION")))
        .build();
    let resp = agent.get(p.url).call().map_err(|e| format!("Could not download {}: {}", p.url, e))?;
    let mut body = resp.into_reader().take(p.size + 1);
    let mut f = std::fs::File::create(&part).map_err(|e| format!("cannot write {}: {}", part.display(), e))?;
    let (mut h, mut n, mut buf) = (Sha256::new(), 0u64, vec![0u8; 1 << 16]);
    loop {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            drop(f);
            let _ = std::fs::remove_file(&part);
            return Ok(false);
        }
        let k = body.read(&mut buf).map_err(|e| format!("Downloading {} failed: {}", p.url, e))?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        f.write_all(&buf[..k]).map_err(|e| format!("cannot write {}: {}", part.display(), e))?;
        n += k as u64;
        progress(n);
    }
    drop(f);
    let got = hex(&h.finalize());
    if n != p.size || got != p.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{} does not match its pinned size and SHA-256 ({} bytes, {}): not used. Try Set up again.", p.file, n, got));
    }
    std::fs::rename(&part, &dst).map_err(|e| format!("cannot move {}: {}", dst.display(), e))?;
    Ok(true)
}

/// Unpacks a tarball with one top-level folder (python/, zig-x86_64-linux-0.17.0/) as `parent/name`, replacing an older one.
fn unpack(archive: &Path, parent: &Path, name: &str) -> Result<(), String> {
    let tmp = parent.join(format!(".unpack-{}", name));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("cannot create {}: {}", tmp.display(), e))?;
    let out = command("tar").arg("-xf").arg(archive).arg("-C").arg(&tmp).output().map_err(|e| format!("could not start tar: {}", e))?;
    if !out.status.success() {
        return Err(format!("Unpacking {} failed: {}", archive.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    let top: Vec<PathBuf> = std::fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    let [only] = top.as_slice() else {
        return Err(format!("{} does not hold one folder", archive.display()));
    };
    let dst = parent.join(name);
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::rename(only, &dst).map_err(|e| format!("cannot move {}: {}", dst.display(), e))?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn appimage_entries_are_dropped_for_host_programs() {
        let v = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>().into_iter();
        let env = v(&[
            ("APPDIR", "/tmp/.mount_PokoX"),
            ("APPIMAGE", "/home/me/Games/PokopiaLauncher.AppImage"),
            ("XDG_DATA_DIRS", "/tmp/.mount_PokoX/usr/share:/usr/local/share:/usr/share"),
            ("GTK_PATH", "/tmp/.mount_PokoX/usr/lib/gtk-3.0"),
            ("HOME", "/home/me"),
            ("PATH", "/usr/bin:/bin"),
        ]);
        let mut got = appimage_env_fixes("/tmp/.mount_PokoX/", env);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("APPDIR".to_string(), None),
                ("APPIMAGE".to_string(), None),
                ("GTK_PATH".to_string(), None),
                ("XDG_DATA_DIRS".to_string(), Some("/usr/local/share:/usr/share".to_string())),
            ]
        );
        // not in an AppImage: nothing changes
        assert!(appimage_env_fixes("", v(&[("XDG_DATA_DIRS", "/usr/share"), ("APPIMAGE", "x")])).is_empty());
    }

    #[test]
    fn win_path_maps_the_root_to_z() {
        assert_eq!(win_path(Path::new("/home/deck/Games/pkp/shader_cache/stage")), OsString::from(r"Z:\home\deck\Games\pkp\shader_cache\stage"));
    }

    #[test]
    fn vdf_values_reads_quoted_pairs() {
        let text = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/me/.local/share/Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games\\\\lib\"\n\t}\n}\n";
        assert_eq!(vdf_values(text, "path"), vec!["/home/me/.local/share/Steam".to_string(), "/mnt/games\\lib".to_string()]);
        assert_eq!(vdf_values("\"require_tool_appid\" \"1628350\"", "require_tool_appid"), vec!["1628350".to_string()]);
    }

    #[test]
    fn unpack_moves_the_one_top_folder() {
        let d = std::env::temp_dir().join(format!("pkp-host-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src").join("zig-x86_64-linux-0.17.0")).unwrap();
        std::fs::write(d.join("src").join("zig-x86_64-linux-0.17.0").join("zig"), b"#!").unwrap();
        let st = Command::new("tar").arg("-czf").arg(d.join("z.tar.gz")).arg("-C").arg(d.join("src")).arg("zig-x86_64-linux-0.17.0").status().unwrap();
        assert!(st.success());
        unpack(&d.join("z.tar.gz"), &d, "zig").unwrap();
        assert!(d.join("zig").join("zig").is_file());
        assert!(!d.join(".unpack-zig").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
