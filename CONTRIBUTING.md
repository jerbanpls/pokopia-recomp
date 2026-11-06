# Contributing

- Open pull requests against `main` and say what they change.
- Code for this game goes in `recomp/games/pkp/` and `runtime/games/pkp/`; the rest is shared with other games.

## Keeping up with pokopia-native

[pokopia-native](https://github.com/xFractals/pokopia-native) is this project's parent. Every update it publishes gets mirrored here:

1. Read its commits since the last reviewed one (named at the top of [CHANGELOG.md](CHANGELOG.md)): `git remote add native https://github.com/xFractals/pokopia-native.git`, `git fetch native`, `git log <last reviewed>..native/main`.
2. For each change that touches the game, the runtime, the builder or the launcher, decide: we already have it, take it, or skip it (Mac/iPhone-only, or not how this project works). Its runtime is different code, so taking a change usually means writing the same fix in ours.
3. One pull request per change taken, saying "Based on pokopia-native (MIT) `<path>` @`<commit>`"; copied code gets a header comment naming the file and commit.
4. Only a version that works is merged; behaviour changes for all players go behind their own ini key unless they are plain bug fixes.
5. Move the "last reviewed" commit at the top of the changelog, and the commit named in `runtime/THIRD_PARTY_NOTICES.md` if code was copied.

## Building from source

Needs Visual Studio 2022 Build Tools or newer (with the C++ x64 tools and CMake), LLVM 23, PowerShell 7, Python 3.13 with [uv](https://docs.astral.sh/uv/), sccache, .NET 10 SDK, the Vulkan SDK (validation layers), vcpkg with `libvpx:x64-windows-static` (`C:\vcpkg` or `VCPKG_ROOT`), Rust and Node.js. The scripts find Visual Studio with vswhere.

First build, from the repository root:

1. `uv venv tools\.venv --python 3.13` and `uv pip install --python tools\.venv -r recomp\requirements.txt`
2. `pwsh -File tools\textures\scripts\fetch_third_party.ps1`, then `pwsh -File tools\textures\scripts\build.ps1` (the runtime links its `astc2bc7.lib`)
3. Put the dump in `game\dump\` as the launcher expects it (`updates\<version>\exefs` and `romfs`) or the exefs files in `game\dump\exefs\`, then `recomp\scripts\build_game.cmd pkp`
4. `pwsh -File game-build\scripts\build.ps1`

- Shaders: [tools/shaders/README.md](tools/shaders/README.md). Launcher: `npm install`, `npm run tauri build` in `launcher\`.
