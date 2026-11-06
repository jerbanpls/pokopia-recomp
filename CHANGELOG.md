# Changelog

The Linux edition of Pokopia Recomp. Each release is based on a Pokopia Recomp release by featureJosh, named in its heading;
that project's own history, back to drakolordx7's original, is in [docs/UPSTREAM-CHANGELOG.md](docs/UPSTREAM-CHANGELOG.md).

## 0.6.0-linux.1

Based on Pokopia Recomp **0.6.0 (build J: Jaguar)**. The first release of the Linux edition.

### Runs on Linux
- **The launcher is a native Linux program.** It used to need Microsoft's WebView2 and did not start under Proton at all; built
  for Linux it draws its window with WebKitGTK.
- **Two launchers in the download:** `PokopiaLauncher` (small, uses the system's WebKitGTK 4.1) and
  `PokopiaLauncher-x86_64.AppImage` (brings its own libraries). Both are built on Ubuntu 22.04, so they need only glibc 2.35.
- **The build tools are fetched for you.** The first Set up downloads Python 3.13 (with lz4 and capstone) and Zig 0.17.0 into
  `tools/linux`, each checked against a pinned size and SHA-256. The game is still compiled with the same Zig version as before.
- **Proton is found by itself:** Steam's Proton Experimental (or the newest Proton) inside the Steam Linux Runtime, exactly as
  Steam starts games, with a prefix of its own in `~/.local/share/pokopia-recomp`. Without Steam: umu-run, then Wine.
  `PKP_PROTON=<folder>` picks another Proton build.
- **The game build runs on Linux:** `pkp_build.py` uses the Linux Zig, starts its compilers with process options Linux accepts
  (the old ones made it fail there), and limits parallel compiles by the free memory read from `/proc/meminfo` (before, a 16 GB
  PC could start 12 compiles of about 2 GB each).
- **Checks before Set up on Linux:** free disk space, Vulkan (through `libvulkan.so.1`), memory, the processor, and that Proton is
  there; Set up says so at the start, not after the 12-minute build.
- **Pause stops everything:** a paused shader translation also stops Proton's wine processes, and the hidden first run is stopped
  if it is still going four minutes after its minute is up.

### Faster and smoother under Proton
- **No more memory write-watch stalls.** Set up writes `[nvn] pool_mirror = off` and `tex_write_watch = false` (unless you set
  them): under Proton every write those features caught made the GPU driver map the game's memory again. Measured in a
  24-minute session on an RTX 4070 Ti: 107 stalls of 60-180 ms and a 5-second worst frame with them on; with them off, no such
  stall and a 190 ms worst frame.
- **A GPU shader cache of the game's own** (`~/.local/share/pokopia-recomp/shadercache`), never trimmed, as Steam gives every
  game: pipelines built in one session stay built for the next.
- **Shaders are translated in larger batches** (5,000 at a time), since every batch also starts Proton: the 99,012 shaders take
  about 17 minutes on a 12-thread desktop.

### Fits into a Linux desktop
- **Link Play** lists this PC's addresses on Linux too (your network first, Tailscale and ZeroTier as internet options, Docker,
  libvirt and VPN interfaces left out), and the firewall hint names firewalld.
- **Save import** finds Eden and Ryujinx saves in their Linux folders (`~/.local/share/eden`, `~/.config/Ryujinx`), and knows
  when the game is running under Proton.
- **Files and folders open** in your default apps (`xdg-open`).
- **NVIDIA:** the launcher turns off WebKitGTK's DMA-BUF renderer, which leaves the window blank on NVIDIA's driver.
- **`PokopiaLauncher --play`** starts the game as Play does, without the window, and waits for it: for Steam, Sunshine or desktop
  shortcuts. `install-shortcut.sh` adds the launcher to the app menu and the desktop, with a "Play Pokopia" action.
- **Updates:** the launcher's "Update available" banner follows this edition's releases.

### Included from Pokopia Recomp 0.6.0
- The world is drawn in the right place below 100% scene size at 1440p, ultrawide and 16:10.
- No more getting stuck on the Poké Ball wipe above 60 fps; frame limits above 60 and unlimited work again.
- Lamp lights no longer show a ghost copy away from 1920x1080.
- The in-game clock shows your real time.
- Device presets (F1, Graphics), a check that setup can write in the game folder, base and update game data combined
  automatically, Steam Deck anti-stutter work, and more: see [docs/UPSTREAM-CHANGELOG.md](docs/UPSTREAM-CHANGELOG.md).

### Known issues
- **Steam Controller (2026):** works as an Xbox-style controller only; gyro, touchpads and grip sense are not passed to the game
  yet.
- **FSR 3 and DLSS/DLAA** do not look right yet (the image shakes and lights flash, also upstream): use FSR 1.
- **Not tested yet:** AMD and Intel GPUs, the Steam Deck and SteamOS, the AppImage on other distributions, umu-run and Wine
  instead of Steam's Proton, the Steam and Sunshine shortcuts, and pausing in the middle of the shader translation. Reports
  welcome.

### For developers
- `src-tauri/src/host.rs` holds everything that differs on Linux: the build tools, the Proton runner, `Z:\` paths for the
  programs run through it, killing a tool with its process group, cleaning an AppImage's variables out of the environment of
  the programs the launcher starts.
- The launcher's Linux tests stay in `linux_tests` modules (upstream removed its unit tests in 0.6.0).
- `linux/package.sh` builds the release download; the upstream release workflow is gone.
