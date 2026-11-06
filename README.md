<h1 align="center">🏝️ Pokopia Recomp</h1>

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
  <a href="https://github.com/xFractals/pokopia-native">Parent: pokopia-native</a> ·
  <a href="SECURITY.md">Security</a> ·
  <a href="LEGAL.md">Legal</a> ·
  <a href="https://github.com/featureJosh/pokopia-recomp/issues">Issues</a>
</p>

> [!IMPORTANT]
> **No game files are included.** You need your own extracted copy of the game, version **2.0.1a**. This project provides no game files, keys or dumping tools. Not affiliated with Nintendo, The Pokémon Company or Koei Tecmo; Pokémon is a trademark of Nintendo, Creatures Inc. and GAME FREAK inc. (see [LEGAL.md](LEGAL.md)).

> [!NOTE]
> **Parent project: [pokopia-native](https://github.com/xFractals/pokopia-native).** This project follows pokopia-native: whenever it ships an update, the parts that apply here are brought over in a tested build (see [CONTRIBUTING.md](CONTRIBUTING.md#keeping-up-with-pokopia-native)). It started as a fork of [Pokopia Recomp](https://github.com/drakolordx7/pokopia-recomp) 0.1.1.

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

## Features

### Play together (Link Play)
- **Visit a friend's island, or invite them to yours**, on the same Wi-Fi or over the internet.
- Set it up in the launcher's **Link Play** page: press **Host** and share the address and code it shows, or press **Visit** and type them in. No files to edit.
- A **room code** keeps strangers out.
- Step-by-step guide: [Playing together](#playing-together).

### Import your saves
- **Already playing on Eden or Ryujinx?** One button in **Settings** finds your save and brings it over. Your emulator's copy isn't touched.
- **Got a save somewhere else?** Pick its folder (the one with `B00`, `G00` and `S00` inside).
- **Your current save is never deleted.** It's kept as a backup folder first.
- If a save can't be written (a full disk, OneDrive or antivirus holding the file), your last good save is kept and the game tells you.

### Easy setup
- **Use your dump as it comes:** a folder (with `base` and `updates`) or a single `.nup` file.
- **Clear messages if something is wrong with your dump:** the wrong game version, a missing update, or files damaged while copying. The launcher tells you what to do.
- **Checks before it starts:** enough disk space, a working graphics driver, enough memory.
- **Use fewer cores** keeps your PC usable while it builds. You can pause and carry on later.
- **Updates are quick:** only the parts that changed are rebuilt.
- The launcher tells you when a new version is out.

### Controls
- **Any controller:** Xbox, PlayStation, Switch Pro and more, with rumble and motion. Buttons follow the Switch layout by default.
- **Keyboard and mouse.** The mouse also works in menus: click to pick, scroll to move.
- **Change any button** on the F1 **Controls** page.
- **Joy-Con 2 mouse mode** for placing items: press the middle mouse button.
- **On-screen button icons match what you're holding:** keyboard, Xbox, PlayStation or Switch.
- **Typing your name** works with a controller, mouse or keyboard.
- **Quit without Alt+F4:** hold Select and Start (View and Menu on an Xbox pad), or use **Quit game** in F1.

### Steam Deck and handhelds
- Steam Deck, Legion Go S and other handhelds are recognised: controller icons stay on screen and the picture size suits the screen.
- 16:10 screens are filled edge to edge, with no black bars.
- Runs through Proton on SteamOS, with handheld speed-ups (about 26 to 53 fps on a Steam Deck and 55 to 75 fps on a Legion Go S, measured by players).
- The launcher doesn't run under Proton yet. Set up on a Windows PC, then copy the game folder over.

### Graphics
- **Play at any resolution**, 4K and above. Lower **Render scale** if it runs slowly.
- **Quality presets** (Switch 2, High, Ultra) set how far you can see and how sharp textures are.
- **HDR** on HDR screens.
- **Window, borderless or full screen.** Choose the monitor, cap the frame rate, show an FPS counter. Alt+Enter switches modes while playing.
- **Upscalers:** FSR 1 and NVIDIA Image Scaling work.
- **FSR 3 and DLSS / DLAA are there but don't look good yet** and need rework. Leave them off for now.

### In-game settings
Press **F1** in the game to change graphics, controls and sound (volume, output device, mute when the window is in the background). Most changes apply straight away.

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

## Playing together

Link Play puts two players on one island. Every player needs this release, and **each player needs their own save**: a copy of someone else's save cannot join its original.

**To host**
1. In the launcher, open **Link Play** and pick **Host on this PC**, then **Start hosting**.
2. Give your friends one of the addresses it shows (the first one is for your own network) and the room code. Use **Copy** for both.
3. Keep the launcher open while you play, and press **Play**.

**To visit**
1. In the launcher, open **Link Play**, pick **Visit**, type the host's address and room code, and press **Save**.
2. Press **Play**.

**Before you start (Windows):** the first time, Windows Firewall asks about the launcher: allow **Private networks**. Your network must be Private too (Settings, Network & internet, your connection, Network profile type: **Private**). On a Public network Windows keeps the other PCs out; the host page warns you.

**In the game**, open the PC in your first house, then **Link Play** (it unlocks after Environment Level 2 in the Withered Wasteland) and pick **Local connection**. (**Online connection** is Nintendo's own service and does not work here.) The host picks **Invite Others to Visit**, the visitor **Visit a Friend**.

**Over the internet:** the easy way is [Tailscale](https://tailscale.com) or [ZeroTier](https://www.zerotier.com) (both free for personal use) on every PC: no router settings, and the traffic between you is encrypted. The host page lists that address too. Without one, the host forwards port **39660, TCP and UDP**, to their PC on the router's port forwarding page, and gives out their internet address (search "what is my IP" on the host's network). TCP alone works, but UDP keeps a lossy link from stalling. Always use a room code then, and remove the forward when you are done: the relay checks the code but does not encrypt the game's traffic.

Link Play is still being tested: joining and walking around together work, long sessions and more than two players are not tested yet. See [docs/multiplayer](docs/multiplayer/README.md) for how it works and the `pkp.ini` keys.

## Common problems

- **The window is black, or the game stutters a lot, on the first launch.** The game makes some shaders while it runs. The launcher translates the ones it has seen before every start and again after you quit, so the first runs are rough and later ones are better. Give it a minute and keep playing. If the setup stopped half way, open **Game files** and press **Set up** to continue. It keeps finished work.
- **The launcher says the game files are from an older version, or that the update is missing.** This release needs version 2.0.1a. Dump the game again with the update installed. The base game alone is not enough.
- **The launcher says it found no usable game files, or Set up says a file does not match.** Pick the folder that holds `base` and `updates` (for example `Pokopia_v2.0.1a`). Each needs `exefs` (`main`, `sdk`, `subsdk0`, `rtld`, `main.npdm`) and `romfs`. A file that does not match is damaged. Dump it again, or copy it again.
- **The launcher says the game data (romfs) is incomplete, or the game closed at start with "pkp.exe exited with code 4".** Your dumping tool wrote the update's `romfs` without the base game's files under it (an update only holds the files it changes). Keep the base game in the same dump (`base` next to `updates`, each with `exefs` and `romfs`) and pick the folder that holds both: the launcher adds the missing files from `base` itself. Then press **Set up** once more. Otherwise dump the game again with the update merged into the base game.
- **The launcher says the dump was copied in text mode.** You copied it with FTP in text mode, which damages `main`, `sdk` and `subsdk0`. Copy the `exefs` and `romfs` folders again with the transfer type set to **Binary** (FileZilla: Transfer, Transfer type, Binary).
- **Your dump is one file** (`.nsp`, `.xci`). The launcher takes the game as folders, or as a `.nup`. Extract the `.nsp` or `.xci` to folders with your dumping tool.
- **Setup stops with an error about a file that cannot be opened or removed.** Put the folder somewhere plain, like `C:\Pokopia`, not in a folder that OneDrive syncs. Your antivirus may be scanning the files it is building: wait for it, or allow the folder. **Open setup log** shows where it stopped.
- **Set up warns before it starts.** It checks free disk space (about 36 GB while it runs), Vulkan, memory and the processor first. On an Intel 13th or 14th generation desktop CPU, update the BIOS before you set up: without Intel's fix these CPUs can crash under long full load. If the PC gets hot or hard to use while it builds, tick **Use fewer cores**.
- **Low frame rate, or a blurry picture.** The picture size is fixed when the game starts. It is the size of the window or screen at that moment (**Settings**, **Display**).
  - Start in **Borderless** on a 4K screen and the game draws at 4K. That is heavy. Lower **Render scale**.
  - Start **Windowed** (1280x720 by default) and press Alt+Enter, and the picture stays 720p, scaled up. Set **Window mode** before you press **Play**.
  - Set **Frame rate limit** to what your screen can show.
- **Link Play: "Communication error" (9005) when you visit.** You are both playing copies of the same save. Each player needs their own save.
- **Link Play: the visitor cannot find the host.** Check that the host pressed **Start hosting** and still has the launcher open, that both typed the same room code, and that both networks are Private in Windows. Over the internet, check the port forward (39660, TCP and UDP) or use Tailscale or ZeroTier. Every player needs the same release. Changes on the Link Play page apply the next time the game starts.
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
