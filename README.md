<h1 align="center">🏝️ Pokopia Recomp for Linux</h1>

<p align="center">
  <b>Pokémon Pokopia (Switch 2) recompiled to run on Linux: Bazzite, SteamOS, Fedora, Arch, Ubuntu and more.</b><br>
  A native launcher sets everything up and starts the game through Proton.
</p>

<p align="center">
  <a href="https://github.com/jerbanpls/pokopia-recomp/releases/latest"><img src="https://img.shields.io/github/v/release/jerbanpls/pokopia-recomp?label=latest&color=d8b46a&labelColor=1b2740" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/platform-Linux-fcc624?logo=linux&logoColor=white&labelColor=1b2740" alt="Platform: Linux">
  <img src="https://img.shields.io/badge/runs%20through-Proton-1a9fff?logo=steam&logoColor=white&labelColor=1b2740" alt="Runs through Proton">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue?labelColor=1b2740" alt="License: GPL-3.0"></a>
</p>

<p align="center">
  <a href="https://github.com/jerbanpls/pokopia-recomp/releases/latest"><img src="https://img.shields.io/badge/%E2%AC%87_Download-latest_for_Linux-d8b46a?style=for-the-badge&labelColor=1b2740" alt="Download the latest release for Linux"></a>
</p>

<p align="center">
  <a href="#how-to-install">How to install</a> ·
  <a href="CHANGELOG.md">What changed</a> ·
  <a href="#common-problems">Common problems</a> ·
  <a href="CONTRIBUTING.md">Contributing</a> ·
  <a href="CREDITS.md">Credits</a> ·
  <a href="LEGAL.md">Legal</a> ·
  <a href="https://github.com/jerbanpls/pokopia-recomp/issues">Issues</a>
</p>

> [!IMPORTANT]
> **No game files are included.** You need your own copy of the game, dumped from a console you own: the **base game and the
> 2.0.1 update (update 655360)**. This project provides no game files, keys or dumping tools. Not affiliated with Nintendo, The
> Pokémon Company or Koei Tecmo; Pokémon is a trademark of Nintendo, Creatures Inc. and GAME FREAK inc. (see [LEGAL.md](LEGAL.md)).

> [!NOTE]
> **The Linux edition of [featureJosh's Pokopia Recomp](https://github.com/featureJosh/pokopia-recomp)** (0.6.0, build J: Jaguar),
> which grew from [drakolordx7's Pokopia Recomp](https://github.com/drakolordx7/pokopia-recomp). The game, the runtime and the
> recompiler are theirs; this edition adds a launcher that runs natively on Linux and makes the whole setup work there. See
> [CHANGELOG.md](CHANGELOG.md) for what changed and [CREDITS.md](CREDITS.md) for everyone whose work this builds on. The Linux
> work was developed with an AI assistant (Claude) and tested on real hardware.

## What you get

- **A launcher that runs natively on Linux.** It checks your PC, unpacks and checks your game dump, compiles the game on your PC,
  prepares its shaders, and starts it. One window, no terminal needed.
- **Everything it needs is fetched for you.** On the first Set up the launcher downloads its build tools (Python and Zig, about
  95 MB), each checked against a pinned checksum. Nothing to install by hand besides Steam's Proton.
- **Runs through Proton the way Steam runs games:** Proton Experimental inside the Steam Linux Runtime. No Steam shortcut needed.
- **Tuned for Proton:** the memory settings that cost 60-180 ms stalls under Proton are off, and the game keeps its own GPU
  shader cache that is never trimmed, so pipelines built once stay built.
- **Two launchers in one download:** a small native program, and an AppImage that brings its own libraries for distributions
  (and SteamOS) that lack them.
- **Shortcuts:** a script adds the launcher to your app menu and desktop; `PokopiaLauncher --play` starts the game directly for
  Steam, Sunshine/Moonlight or any other shortcut.
- **Everything from Pokopia Recomp 0.6.0:** Link Play with friends, save import from Eden and Ryujinx, controller, keyboard and
  mouse support with remapping, device presets, HDR, any resolution, and the 0.6.0 fixes (the world drawn in the right place at
  1440p and 16:10, no ghost lamp lights, frame limits above 60 fps, the real time of day).

## Requirements

| | |
|---|---|
| **System** | 64-bit Linux (x86-64). Tested on Bazzite 45. |
| **Graphics** | A Vulkan 1.3 GPU with a current driver. Tested on NVIDIA (RTX 4070 Ti). |
| **Proton** | Steam with **Proton Experimental** installed (Steam: Library, search "Proton Experimental", Install). Without Steam: [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher) or Wine. |
| **Disk** | About 40 GB free during setup (about 30 GB afterwards). |
| **Memory** | 16 GB recommended. |
| **Your game** | The base game plus update 655360 (2.0.1), as a folder (`base/` and `updates/655360/`, each with `exefs/` and `romfs/`) or as one `.nup` file. |
| **Native launcher** | WebKitGTK 4.1 and glibc 2.35 or newer (Bazzite, Fedora, Arch, CachyOS, Ubuntu 22.04+ and most desktops have them). If it does not start, use the AppImage. |

## How to install

1. **Download** `PokopiaRecomp-0.6.0-linux.1.tar.gz` from the [latest release](https://github.com/jerbanpls/pokopia-recomp/releases/latest).
2. **Extract it** where you keep games. In a file manager: right-click, Extract. Or in a terminal:
   ```sh
   mkdir -p ~/Games && tar -xzf ~/Downloads/PokopiaRecomp-0.6.0-linux.1.tar.gz -C ~/Games
   ```
   This makes `~/Games/PokopiaRecomp`.
3. **Make sure Proton Experimental is installed** in Steam (see Requirements). The launcher finds it by itself.
4. **Start the launcher:** double-click `PokopiaLauncher` in that folder. If nothing opens, double-click
   `PokopiaLauncher-x86_64.AppImage` instead.
5. **Optional, add shortcuts:** run `install-shortcut.sh` in the folder (double-click it, or `./install-shortcut.sh`). It adds
   "Pokopia Launcher" to your app menu and desktop, with a right-click "Play Pokopia" that starts the game directly.

## First-time setup

1. In the launcher, open **Game files** and pick your dump folder (the one holding `base` and `updates`), or press **.nup** and
   pick your `.nup` file. The launcher checks the game version and the files before anything big happens.
2. Press **Set up**. It checks your PC first (disk space, Vulkan, memory, Proton, write access), then:
   - downloads the build tools, once (about 95 MB);
   - compiles the game from your dump into `pkp.exe` (about 12 minutes on a 12-thread desktop);
   - translates the game's 99,012 shaders through Proton (about 17 minutes on the same PC);
   - runs the game once in the background for a minute to collect the shaders it builds while it plays.

   You can **pause** and continue later; finished work is kept. **Use fewer cores** keeps the PC usable meanwhile.
3. Press **Play**.

On a Steam Deck the setup takes much longer. You can also set up on a desktop PC and copy the whole folder to the Deck (for
example with `rsync -a`). Keep your dump inside the game folder (or use a `.nup`, which unpacks there), so the copied folder
still finds it.

## Playing

- **From the launcher:** press **Play**. F1 opens the game's settings; middle mouse toggles the Joy-Con 2 mouse mode for placing
  items.
- **From the app menu or desktop:** "Pokopia Launcher" after `install-shortcut.sh`, or right-click it, "Play Pokopia", to skip
  the launcher window.
- **As a Steam shortcut:** add `PokopiaLauncher` as a non-Steam game and put `--play` in its launch options. Leave its
  compatibility (Proton) setting off: the launcher is a Linux program and starts Proton itself.
- **Streaming to another device (Sunshine and Moonlight):** add an app whose command is
  `/path/to/PokopiaRecomp/PokopiaLauncher --play`. The stream ends when you quit the game.

## Settings worth knowing

- **Upscaler: use FSR 1** (or none). FSR 3 and DLSS/DLAA are there but do not look right yet (the image shakes and lights flash).
- **Adaptive sync (FreeSync/G-Sync):** set VSync off and a frame rate limit a little below your screen's refresh rate.
- **Device presets** (F1, Graphics) set view distance, texture sharpness, render scale and frame limit in one pick.
- **If it runs slowly:** lower **Render scale**. The picture size is fixed when the game starts.

## Controllers

Xbox, PlayStation and Switch Pro controllers, keyboard and mouse all work, with rumble, and buttons can be remapped on the F1
Controls page. The **Steam Controller (2026)** works as an Xbox-style controller; its gyro, touchpads and grip sense are not
passed to the game yet.

## Playing together

Link Play puts two players on one island: open **Link Play** in the launcher, press **Host** and share the address and room code
it shows, or **Visit** and type them in. Each player needs their own save. Over the internet, [Tailscale](https://tailscale.com)
or [ZeroTier](https://www.zerotier.com) is the easy way; the host page lists their addresses too. If visitors cannot connect,
your firewall may block the port (39660, TCP and UDP). With firewalld (Fedora, Bazzite):

```sh
sudo firewall-cmd --add-port=39660/tcp --add-port=39660/udp
```

More in [docs/multiplayer](docs/multiplayer/README.md).

## Updating

1. Download the new release and close the game and the launcher.
2. Extract it over your folder. The download holds no saves, settings, shaders or game files, so nothing of yours is replaced.
3. Start the launcher. If the update changes the game code, it says so: open **Game files** and press **Set up** again (it
   rebuilds `pkp.exe`, about 12 minutes; your shaders and saves are kept).

The launcher also tells you when a new release is out.

## Common problems

- **The launcher does not open.** Your system lacks WebKitGTK 4.1 or has an older glibc: use `PokopiaLauncher-x86_64.AppImage`.
  If the AppImage does not start either, your system lacks FUSE 2: run it as
  `./PokopiaLauncher-x86_64.AppImage --appimage-extract-and-run`.
- **The launcher window is blank.** On NVIDIA the launcher works around WebKitGTK's DMA-BUF renderer by itself; on other GPUs,
  start it with `WEBKIT_DISABLE_DMABUF_RENDERER=1 ./PokopiaLauncher`.
- **"No Proton found" before Set up.** Install Proton Experimental in Steam (or umu-launcher, or Wine), then press Set up again.
- **The game version is wrong, or the update is missing.** This release needs the 2.0.1 update (655360) together with the base
  game. A dump labelled 2.0.1 can hold an older update; the launcher names what it found.
- **"This update only holds the files it changes and needs the base game".** Your dump has the update without the base game.
  Dump the base game too and keep both (`base` next to `updates`).
- **Stutter or missing effects on the very first runs.** The game builds some shaders while it plays; the launcher translates them
  before every start and after you quit, so it gets smoother after the first sessions.
- **The image shakes or lights flash.** The upscaler is set to FSR 3 or DLSS: switch it to FSR 1 (F1, Graphics).
- **Setup stops with an error.** **Open setup log** on the Game files page shows where it stopped.
- **The controller is not detected.** Connect it before pressing **Play**. The F1 Controls page shows which controller the game
  sees.

## Where your files are

In the game folder (the folder with `PokopiaLauncher`):

| What | Where |
|---|---|
| Saves | `save` |
| Settings | `pkp.ini` and `controls.ini` (F1 in game writes `controls.ini`) |
| Logs | `pkp.log` (every start), `setup.log` (Set up); a crash also writes `pkp_crash.txt`, `pkp_freeze.txt` or `pkp_fatal.txt` |
| Translated shaders | `shader_cache` (about 26 GB) |
| Build files | `build`, and the build tools in `tools/linux` |
| Unpacked `.nup` | `nup_dump` |

Outside it: the launcher's own settings in `~/.config/org.pkp-recomp.launcher/launcher.json`, and Proton's prefix and the GPU
shader cache in `~/.local/share/pokopia-recomp`. Deleting the Proton prefix is safe; it is made again on the next start.

## Reporting problems

[Open an issue](https://github.com/jerbanpls/pokopia-recomp/issues/new/choose) with `pkp.log` (or `setup.log`) from the session
where it happened, your GPU and driver, your distribution and your Proton version. Never attach game files, the `build` or
`shader_cache` folders, or saves.

## Building from source

`linux/package.sh` builds the release download: both launchers in an Ubuntu 22.04 container (so they need only glibc 2.35), the
official Pokopia Recomp 0.6.0 files it is based on (checked by SHA-256), and this repository's Linux changes. It needs podman.
Details in [launcher/README.md](launcher/README.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## License

GPL-3.0, like the projects it builds on. Third-party code keeps its own licence, see [CREDITS.md](CREDITS.md).
