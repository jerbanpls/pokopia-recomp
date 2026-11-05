<h1 align="center">🏝️ Pokopia Recomp (Bazzite Fork)</h1>

<p align="center">
  <b>Pokémon Pokopia (Switch 2) recompiled to run natively on Windows, with multiplayer support.</b>
</p>

<p align="center">
  <a href="https://github.com/featureJosh/pokopia-recomp/releases/latest"><img src="https://img.shields.io/github/v/release/featureJosh/pokopia-recomp?label=latest&color=d8b46a&labelColor=1b2740" alt="Latest release"></a>
  <a href="https://github.com/featureJosh/pokopia-recomp/releases/latest"><img src="https://img.shields.io/github/downloads/featureJosh/pokopia-recomp/latest/total?label=latest%20downloads&color=3fb950&labelColor=1b2740" alt="Latest release downloads"></a>
  <a href="https://github.com/featureJosh/pokopia-recomp/releases"><img src="https://img.shields.io/github/downloads/featureJosh/pokopia-recomp/total?label=downloads&color=3fb950&labelColor=1b2740" alt="Total downloads"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2010%2F11-0078d6?logo=windows&logoColor=white&labelColor=1b2740" alt="Platform: Windows 10/11">
  <a href="https://github.com/featureJosh/pokopia-recomp/commits/main"><img src="https://img.shields.io/github/last-commit/featureJosh/pokopia-recomp/main?label=last%20commit&color=58a6ff&labelColor=1b2740" alt="Last commit on main"></a>
  <a href="https://github.com/featureJosh/pokopia-recomp/issues"><img src="https://img.shields.io/github/issues/featureJosh/pokopia-recomp?label=open%20issues&color=db61a2&labelColor=1b2740" alt="Open issues"></a>
</p>

<p align="center">
  <a href="https://github.com/featureJosh/pokopia-recomp/releases/latest"><img src="https://img.shields.io/badge/%E2%AC%87_Download-latest_for_Windows-d8b46a?style=for-the-badge&labelColor=1b2740" alt="Download the latest release for Windows"></a>
</p>

<p align="center">
  <b><a href="https://github.com/featureJosh/pokopia-recomp/releases">Releases</a></b> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="CONTRIBUTING.md">Contributing</a> ·
  <a href="CREDITS.md">Credits</a> ·
  <a href="SECURITY.md">Security</a> ·
  <a href="LEGAL.md">Legal</a> ·
  <a href="https://github.com/featureJosh/pokopia-recomp/issues">Issues</a>
</p>

> [!IMPORTANT]
> **No game files are included.** You need your own extracted copy of the game, version **2.0.1a**. This project provides no game files, keys or dumping tools. Not affiliated with Nintendo, The Pokémon Company or Koei Tecmo; Pokémon is a trademark of Nintendo, Creatures Inc. and GAME FREAK inc. (see [LEGAL.md](LEGAL.md)).

> [!NOTE]
> **Built with AI, supervised by a human.** This project is developed primarily with AI, with me as the supervisor. I have a handful of years of development experience, so it's not all slop. Every commit is reviewed, verified and tested by me before it lands.

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/assets/screenshots/title-screen.svg" alt="Title screen (screenshot placeholder)"><br><sub>Title screen</sub></td>
    <td width="50%"><img src="docs/assets/screenshots/island-gameplay.svg" alt="Island gameplay (screenshot placeholder)"><br><sub>Building your island</sub></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/assets/screenshots/item-placement.svg" alt="Item placement (screenshot placeholder)"><br><sub>Placing items with Joy-Con 2 mouse mode</sub></td>
    <td width="50%"><img src="docs/assets/screenshots/link-play.svg" alt="Link Play (screenshot placeholder)"><br><sub>Link Play with a friend (work in progress)</sub></td>
  </tr>
</table>

## Quick start

1. Download the zip from the [latest release](https://github.com/featureJosh/pokopia-recomp/releases/latest) and extract it anywhere.
2. Run `PokopiaLauncher.exe`. If SmartScreen warns, choose **More info → Run anyway**. Pick your dump folder and press **Set up**: it compiles the game on your PC (about 7 minutes), then builds the shader cache (1 to 2 hours, you can pause and continue later).
3. Press **Play**.

F1 opens the settings in game. Middle mouse toggles Joy-Con 2 mouse mode for placing items.

## Updating from an older version

Your saves, settings and shaders stay.

1. Download the new zip from the [latest release](https://github.com/featureJosh/pokopia-recomp/releases/latest). Close the game and the launcher.
2. Extract it over your install, into the same folder. The zip holds no saves, settings, shader cache or game files, so extracting does not replace them.
3. Run `PokopiaLauncher.exe`. If the update changes the game code, the launcher says so: open **Game files** and press **Set up** again. It rebuilds `pkp.exe` (about 7 minutes). The old `pkp.exe` stays until the new one is ready.
4. Press **Play**.

Copy the `save` folder somewhere first if you want a backup. A save that a newer version wrote may not load in an older one.

## Common problems

- **The window is black, or the game stutters a lot, on the first launch.** The game makes some shaders while it runs. The launcher translates the ones it has seen before every start and again after you quit, so the first runs are rough and later ones are better. Give it a minute and keep playing. If the setup stopped half way, open **Game files** and press **Set up** to continue. It keeps finished work.
- **The launcher says the game files are a different version.** This release needs version 2.0.1a. Dump the game again with the update installed. The base game alone is not enough.
- **The launcher says it found no usable game files, or Set up says a file does not match.** Pick the folder that holds `base` and `updates` (for example `Pokopia_v2.0.1a`). Each needs `exefs` (`main`, `sdk`, `subsdk0`, `rtld`, `main.npdm`) and `romfs`. A file that does not match is damaged. Dump it again, or copy it again.
- **You copied the dump with FTP.** Set the transfer type to **Binary** (FileZilla: Transfer, Transfer type, Binary). In text mode some programs change the bytes of `main`, `sdk` and `subsdk0`, and Set up stops with a "does not match" error.
- **Your dump is one file** (`.nsp`, `.xci`). The launcher takes the game as folders, or as a `.nup`. Extract the `.nsp` or `.xci` to folders with your dumping tool.
- **Setup stops with an error about a file that cannot be opened or removed.** Put the folder somewhere plain, like `C:\Pokopia`, not in a folder that OneDrive syncs. Your antivirus may be scanning the files it is building: wait for it, or allow the folder. Setup needs about 36 GB free while it runs.
- **Low frame rate, or a blurry picture.** The picture size is fixed when the game starts. It is the size of the window or screen at that moment (**Settings**, **Display**).
  - Start in **Borderless** on a 4K screen and the game draws at 4K. That is heavy. Lower **Render scale**.
  - Start **Windowed** (1280x720 by default) and press Alt+Enter, and the picture stays 720p, scaled up. Set **Window mode** before you press **Play**.
  - Set **Frame rate limit** to what your screen can show.
- **The game slows down when you are not looking at it.** Windows 11 can slow a window that is minimised or hidden. Keep the game window in front.
- **The controller is not detected.** Open the game with F1 and look at the **Gamepad** line on the Controls page. It says "No gamepad open" when the game sees none. Connect it before you press **Play**, then try the other cable or port. Close other programs that may be holding the pad.

If it is none of these, see [Reporting problems](#reporting-problems).

## Where your saves, logs and settings are

All of these are in the game folder: the folder with `PokopiaLauncher.exe` and `pkp.exe`, unless you chose another one in **Settings**, **Game folder**.

| What | Where |
|---|---|
| Saves | `save` (**Settings** can point it elsewhere: **Save folder**) |
| Settings | `pkp.ini` and `controls.ini` (F1 in game writes `controls.ini`) |
| Log | `pkp.log`, written on every start from the launcher. A crash also writes `pkp_crash.txt`, `pkp_freeze.txt` or `pkp_fatal.txt`. |
| Translated shaders | `shader_cache` (about 26 GB) |
| Build files | `build` (the setup's work folder) |
| Unpacked `.nup` | `nup_dump`, if you picked a `.nup` |
| Launcher settings | `%APPDATA%\org.pkp-recomp.launcher\launcher.json` (your dump folder and game folder) |

Never attach game files, the `build` or `shader_cache` folders, or saves to an issue. `pkp.log` is enough.

## Reporting problems

[Open an issue](https://github.com/featureJosh/pokopia-recomp/issues/new/choose) with `pkp.log` from the session where it happened, your GPU and your Windows version.

## License

GPL-3.0. Third-party code keeps its own licence, see [CREDITS.md](CREDITS.md).
