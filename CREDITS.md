# Credits

## Where this comes from

- **[Pokopia Recomp by featureJosh](https://github.com/featureJosh/pokopia-recomp)**: the game build, the runtime, the launcher
  and the recompiler setup this Linux edition is based on (0.6.0, build J: Jaguar). It follows its parent project
  [pokopia-native](https://github.com/xFractals/pokopia-native) (MIT; its `ampere-lower/` is GPL-3.0-or-later).
- **[Pokopia Recomp by drakolordx7](https://github.com/drakolordx7/pokopia-recomp)**: the original project (0.1.1), built on
  drakolordx7's [nxrecomp](https://github.com/drakolordx7/nxrecomp).
- **[pokopia-recomp-linux by tinfoil-chewer](https://github.com/tinfoil-chewer/pokopia-recomp-linux)** (GPL-3.0): how to run
  Steam's Proton inside the Steam Linux Runtime from a launcher, cleaning an AppImage's environment for the programs it starts,
  and the pinned Linux Zig; credited where used in `launcher/src-tauri/src/host.rs`.
- **The Linux edition** (this repository): jerbanpls, developed with an AI assistant (Claude).

## Code and assets used

Code from [Ryujinx](https://github.com/Ryubing/Ryujinx), [Mesa](https://gitlab.freedesktop.org/mesa/mesa),
[pokopia-native](https://github.com/xFractals/pokopia-native), smo-recomp, ScarletNX, ssbu-recomp, SuperMarioWonderRecomp, yuzu,
Eden, libnx, [astc-encoder](https://github.com/ARM-software/astc-encoder), [bc7enc_rdo](https://github.com/richgel999/bc7enc_rdo),
[libvpx](https://chromium.googlesource.com/webm/libvpx), AMD FidelityFX, NVIDIA Image Scaling, [SDL3](https://github.com/libsdl-org/SDL),
[Tauri](https://tauri.app), [Zig](https://ziglang.org) and Python, plus [Kenney input prompts](https://kenney.nl/assets/input-prompts)
and DejaVu fonts. Design ideas (no code) from totk-pc-recomp (shadow storage for guest memory) and ultratears (memory pool copies).

The Linux launcher downloads at setup, each pinned by SHA-256: Python from
[python-build-standalone](https://github.com/astral-sh/python-build-standalone) (PSF), the [lz4](https://pypi.org/project/lz4/)
and [capstone](https://pypi.org/project/capstone/) wheels (BSD-3-Clause) and [Zig](https://ziglang.org) 0.17.0 (MIT).

Licences are in `runtime/THIRD_PARTY_NOTICES.md`, `recomp/THIRD_PARTY.md`, `tools/shaders/licenses/` and, in the release
download, `licenses/`.
