# Pokopia Launcher

Tauri 2 launcher for the Pokopia PC port (`pkp.exe`). It ships no game files and never changes yours. It reads the game
files you extracted from your own copy, writes `pkp.ini` next to `pkp.exe` and builds the shader cache from your romfs.

## First run

1. **Game files**: pick the dump folder (`<dump>\base\{exefs,romfs}` and/or `<dump>\updates\<version>\{exefs,romfs}`).
   The newest complete update is used, otherwise the base. "Use these game files" writes `[fs] exefs` and `[fs] romfs`.
2. **Set up**: writes the `pkp.ini` paths (`[fs] exefs`, `[fs] romfs`, `[fs] save_dir`, `[nvn] shader_cache =
   shader_cache/spirv`) and builds `shader_cache\spirv\` next to `pkp.exe`:
   - Extract: `tools\shaders\extract_corpus.py` finds every shader in the romfs and in `exefs\main` and writes the
     distinct blocks to `shader_cache\corpus\` (about 10 GB, deleted when setup finishes).
   - Translate: `tools\spirv_harness\spirv_harness.exe --batch` in chunks of 1000 into `shader_cache\spirv\`
     (`<hash>.spv` + `results.tsv`, about 26 GB for the 99,012 blocks of 2.0.1).
   A progress bar shows each step. Pause stops the tools and Continue setup picks up where they stopped: a block listed
   in `results.tsv` is never translated twice, and extraction is skipped if it already ran for the same romfs. The tools
   run at below-normal priority.
   State: `shader_cache\setup.json`.
   Before it starts, Set up checks the free disk space on the game drive (it refuses to start without about 36 GB, less
   when resumed), Vulkan 1.3 (vulkan-1.dll and a GPU), RAM, and warns about 13th/14th generation Intel desktop CPUs
   whose microcode is older than 0x12B (a BIOS update fixes their crashes under long loads). Warnings can be continued
   past. "Use fewer cores" runs setup with a quarter of the threads (2 to 8) at idle priority.
   Everything setup does is appended to `setup.log` next to `pkp.exe` (UTC timestamps, the commands, the tools' error
   output; rotated to `setup.log.prev` at 2 MB). The Set up card has an "Open setup log" button; attach the file to
   bug reports about a failed setup.
3. **Play** turns on once setup has finished. The launcher starts pkp.exe with no paths on the command line. A `pkp.ini`
   next to it makes its folder the working directory, and every path comes from the ini (relative paths start there).

`release\Play.bat` starts the game the same way without the launcher (after setup).

## Release model (b): pkp.exe built on the player's PC

When `tools\build\pkp_build.py` is present (the package from the main project's release script), the release holds no
`pkp.exe`. Set up first builds it from the chosen dump (`src-tauri/src/gamebuild.rs`): verify the ExeFS by hash, lift,
compile with the bundled Zig, link against `pkp_rt.dll`; then the shader steps below run. Play stays off until both are
done. Pause works in every step and Continue setup resumes. A built pkp.exe counts only while `build\done.json` matches
the tools of the release in use (`pkp_build.py --check`: lifter, `main.sites`, compile flags, SDK files, `pkp_rt.lib`), so
an update extracted over an install that changes any of them turns Play off with "This update changes the game code"
until Set up rebuilds pkp.exe (the shader cache is kept). Headless check of the whole first run:
`cargo run --release --example first_run -- <game dir> <exefs> <romfs> <tools dir> [shader limit] [jobs]`.

## Release folder

The launcher looks for its tools in `tools\` next to its own exe. A release folder must contain:

```
pkp.exe                     the game (game-build, -Release)
PokopiaLauncher.exe         this launcher (npm run tauri build)
Play.bat                    release\Play.bat (optional)
tools\
  spirv_harness\            self-contained publish of nx tools/shaders/harness:
                              dotnet publish tools\shaders\harness\SpirvHarness.csproj -c Release -r win-x64
                                --self-contained true -o <release>\tools\spirv_harness     (~84 MB)
                            (stage the Ryujinx sources first: tools\shaders\scripts\setup_ryujinx.ps1)
  shaders\extract_corpus.py nx tools/shaders/ampere/extract_corpus.py
  python\python.exe         optional: Python 3 (for example the embeddable package) with the lz4 package installed.
                            Without it the first python.exe on PATH that can `import lz4.block` is used.
```

If a file is missing, setup stops and names it. `pkp.exe` can live in another folder (Settings, game folder), but
`tools\` always sits next to the launcher.

## Linux (Bazzite, SteamOS, other distributions)

The launcher also builds as a native Linux program (`npm run tauri build -- --no-bundle` on Linux). Tauri draws it with WebKitGTK
there, so it needs no WebView2 (which Proton lacks: the Windows launcher does not start under Proton). Put the binary in the
release folder next to `tools\` as `PokopiaLauncher`. The game stays the Windows pkp.exe; what differs is in `src-tauri/src/host.rs`:

- The build tools run natively from `tools/linux/`. Set up first downloads Python 3.13 (python-build-standalone) with the lz4 and
  capstone wheels, and Zig 0.17.0, each pinned by size and SHA-256. Off Windows `pkp_build.py` compiles with that Zig, reads free
  memory from `/proc/meminfo` and starts its tools without the Windows creation flags.
- The shader translator and pkp.exe (the hidden first run and Play) run through Proton: Steam's Proton Experimental, else the newest
  Proton N.N, inside the Steam Linux Runtime as Steam starts a game, with its prefix in `~/.local/share/pokopia-recomp/proton`.
  `PKP_PROTON=<folder>` picks another Proton build. Without Steam: `umu-run`, then `wine`. Paths handed to them are `Z:\...`.
  Linux translates in chunks of 5000 (every run also starts Proton), and Pause kills the tool's whole process group.
- The checks before Set up read statvfs, `/proc` and `libvulkan.so.1`, and Set up refuses to start without Proton. Link Play lists
  addresses from getifaddrs (Tailscale and ZeroTier as overlays, bridges and VPNs skipped); save import looks in
  `~/.local/share/eden` and `~/.config/Ryujinx`; files and links open with `xdg-open`. On NVIDIA the launcher sets
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` (a blank window otherwise).
- Building needs the webkit2gtk-4.1, gtk3 and libsoup3 development packages. On Bazzite, in a Fedora toolbox:
  `distrobox create -n pokopia-build -i registry.fedoraproject.org/fedora-toolbox:45`, then
  `sudo dnf install rust cargo nodejs npm gcc webkit2gtk4.1-devel gtk3-devel libsoup3-devel javascriptcoregtk4.1-devel`.

Tested on Bazzite 45 (RTX 4070 Ti, Wayland): `cargo test` (also `cargo check --target x86_64-pc-windows-gnu --all-targets`), the
window, the checks, the tool download, Proton detection, Link Play addresses, Eden save detection, `test_pkp_build.py`, and the
Linux Zig compiling `pkp_stub.cpp` and linking a PE with `pkp.rc` and `pkp_rt.lib`. Not yet run end to end, for want of a 2.0.1
(update 655360) dump with its base game: the game build, the shader translation under Proton, the hidden first run and Play.

## Development

- `cargo check` in `src-tauri`; `npm run tauri build` for the exe.
- `cargo run --example shader_setup -- <game dir> <romfs> <exefs/main> <tools dir> [limit] [jobs]` runs the setup's
  shader steps without the window (same code, `src-tauri/src/shaders.rs`); `limit` translates only that many blocks.
