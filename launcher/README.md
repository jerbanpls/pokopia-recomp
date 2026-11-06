# Pokopia Launcher (Linux)

Tauri 2 launcher for Pokopia Recomp, built as a native Linux program (Tauri draws it with WebKitGTK). It ships no game files and
never changes yours: it reads the game files dumped from your own copy, compiles the game from them (`pkp.exe`), builds the
shader cache from your romfs, writes `pkp.ini` and starts the game through Proton.

## First run

1. **Game files**: pick the dump folder (`<dump>/base/{exefs,romfs}` and/or `<dump>/updates/<version>/{exefs,romfs}`) or a
   `.nup` (unpacked into `nup_dump/`). The newest complete update is used, layered over the base game's romfs when the update
   holds only the files it changes. "Use these game files" writes `[fs] exefs`, `[fs] romfs` and `[fs] romfs_base`.
2. **Set up**, after the checks below (disk space, write access, Vulkan, memory, processor, Proton):
   - **Build tools** (`src-tauri/src/host.rs`, once): Python 3.13 from python-build-standalone with the lz4 and capstone
     wheels, and Zig 0.17.0, into `tools/linux/`, each pinned by size and SHA-256.
   - **Game build** (`src-tauri/src/gamebuild.rs`): `tools/build/pkp_build.py` verifies the ExeFS by hash, lifts the four
     modules, compiles them with Zig and links `pkp.exe` against `pkp_rt.dll`. The script caps parallel compiles at the free
     memory (`/proc/meminfo`) divided by 2 GiB.
   - **Shaders** (`src-tauri/src/shaders.rs`): `tools/shaders/extract_corpus.py` writes the distinct shader blocks of the romfs
     and `exefs/main` to `shader_cache/corpus/` (about 10 GB, deleted at the end); `tools/spirv_harness/spirv_harness.exe`
     translates them through Proton in chunks of 5000 into `shader_cache/spirv/` (`<hash>.spv` + `results.tsv`, about 26 GB).
   - **Hidden first run**: the game plays 60 s without a window or sound, with a save folder of its own, to record the shaders
     it builds while it runs; they are translated before the first real start.

   Pause stops the running tool and everything it started (its process group, Proton's wine processes included); Set up
   again continues where it stopped. Everything Set up does goes to `setup.log` in the game folder ("Open setup log").
3. **Play**: starts `pkp.exe` through Proton with no paths on the command line; a `pkp.ini` next to it makes its folder the
   working directory, and every path comes from the ini. Before every start the run-time shaders seen so far are translated,
   and after the game exits the new ones.

## Release model (b): pkp.exe built on the player's PC

The release holds no `pkp.exe`. A built one counts only while `build/done.json` matches the tools of the release in use
(`pkp_build.py --check`: lifter, `main.sites`, compile flags, SDK files, `pkp_rt.lib`), so an update that changes any of them
turns Play off with "This update changes the game code" until Set up rebuilds `pkp.exe`; the shader cache is kept.
Headless check of the whole first run: `cargo run --release --example first_run -- <game dir> <exefs> <romfs> <tools dir>
[shader limit] [jobs]`.

## Running the programs it starts (`src-tauri/src/host.rs`)

- **Proton:** Steam's Proton Experimental, else the newest Proton N.N Steam installed, started inside the Steam Linux Runtime
  its `toolmanifest.vdf` asks for (as Steam starts a game), with `STEAM_COMPAT_DATA_PATH` =
  `~/.local/share/pokopia-recomp/proton`. `PKP_PROTON=<folder>` picks another Proton build. Without Steam: `umu-run`, then
  `wine`. Paths handed to programs run through it are `Z:\...`.
- **Environment:** every Proton run gets driver shader caches of the game's own that are never trimmed
  (`~/.local/share/pokopia-recomp/shadercache/{nvidia,mesa}`, as Steam sets them up for its games). Inside an AppImage,
  `host::command()` removes the variables the AppImage runtime points into its mount (`GTK_PATH`, `XDG_DATA_DIRS` entries,
  `APPDIR`, `APPIMAGE`, ...) from the environment of every program the launcher starts.
- **Speed:** Set up writes `[nvn] pool_mirror = off` and `tex_write_watch = false` unless they are set: under Proton their
  write-watching makes the GPU driver map the game's memory again after each caught write (measured: 107 stalls of 60-180 ms in
  a 24-minute session; none with both off). `pool_mirror` in `pkp.ini` outranks the F1 page's "Faster GPU memory".
- **Desktop:** the checks read statvfs, `/proc` and `libvulkan.so.1`; Link Play lists addresses from getifaddrs and
  `/proc/net/route`; save import looks in `~/.local/share/eden` and `~/.config/Ryujinx`; files open with `xdg-open`; on NVIDIA
  the launcher sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` (a blank window otherwise).
- **Shortcuts:** `PokopiaLauncher --play [--game <folder>]` does what Play does without the window and waits for the game.
- **Where it looks:** `tools/` next to the launcher, or next to the `.AppImage` file (`$APPIMAGE`) when it runs as one. The
  game folder is the same unless set in Settings.

## Release folder

```
PokopiaLauncher                  this launcher (native; needs WebKitGTK 4.1 and glibc 2.35+)
PokopiaLauncher-x86_64.AppImage  the same launcher with its own libraries
install-shortcut.sh              adds the launcher to the app menu and the desktop
pkp_rt.dll, astcdec-*.dll        the runtime and its texture decoders (Pokopia Recomp release)
upscalers/, assets/, fonts/      from the Pokopia Recomp release
tools/
  build/                         pkp_build.py (this repository's), the lifter, the SDK files, dump_hashes.json
  shaders/extract_corpus.py
  spirv_harness/                 the shader translator (run through Proton)
  linux/                         Python and Zig, fetched by the first Set up
```

`linux/package.sh` makes this folder from the official release zip and the launchers built in `linux/Containerfile`.

## Development

- `cargo test` in `src-tauri` (the Linux tests are the `linux_tests` modules and `host.rs`'s tests).
- `npm ci && npm run tauri build -- --no-bundle` builds `src-tauri/target/release/pokopia-launcher` with the system's toolchain
  (Rust, Node.js, and the webkit2gtk-4.1, gtk3 and libsoup3 development packages; on Bazzite in a Fedora toolbox:
  `distrobox create -n pokopia-build -i registry.fedoraproject.org/fedora-toolbox:45`, then `sudo dnf install rust cargo nodejs
  npm gcc webkit2gtk4.1-devel gtk3-devel libsoup3-devel javascriptcoregtk4.1-devel`).
- `cargo run --example shader_setup -- <game dir> <romfs> <exefs/main> <tools dir> [limit] [jobs]` runs the shader steps without
  the window; `limit` translates only that many blocks.
