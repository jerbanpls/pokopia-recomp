Pokopia Recomp for Linux @VERSION@
(the Linux edition of Pokopia Recomp @UPSTREAM@ by featureJosh)

Nothing from the game is included. You need your own dump of the game: the base game and the 2.0.1 update (update 655360),
as a folder (base/ and updates/655360/, each with exefs/ and romfs/) or as one .nup file.

HOW TO START
1. Install Proton Experimental in Steam (Library, search "Proton Experimental", Install).
2. Start PokopiaLauncher (double-click it). If it does not open, start PokopiaLauncher-x86_64.AppImage instead.
3. Game files: pick your dump folder or .nup, then press Set up. It downloads its build tools once (about 95 MB), compiles the
   game on this PC and translates its shaders: about 30 minutes on a desktop, much longer on a Steam Deck. You can pause and
   continue later. About 40 GB must be free.
4. Press Play. F1 in the game opens its settings.

SHORTCUTS
- ./install-shortcut.sh adds "Pokopia Launcher" to the app menu and the desktop (right-click it for "Play Pokopia").
  ./install-shortcut.sh --remove takes them away again.
- PokopiaLauncher --play starts the game directly, for a Steam, Sunshine or other shortcut.

TIPS
- Use FSR 1 as the upscaler (F1, Graphics): FSR 3 and DLSS do not look right yet.
- With adaptive sync (FreeSync/G-Sync): VSync off, and a frame limit just below your screen's refresh rate.

UPDATING
Extract the new release over this folder: your saves, settings and shaders stay. If the launcher says the game code changed,
open Game files and press Set up again.

FILES
Saves: save/   Logs: pkp.log, setup.log   Settings: pkp.ini, controls.ini

Help, problems and what changed: https://github.com/jerbanpls/pokopia-recomp
Licence: GPL-3.0 (LICENSE.txt), third-party licences in licenses/.
Source code of this release: https://github.com/jerbanpls/pokopia-recomp/tree/v@VERSION@
