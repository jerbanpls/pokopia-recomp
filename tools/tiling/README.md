# tools/tiling: Tegra block-linear tile / untile

C++20 library (`include/tegra_tiling.h`, `src/tegra_tiling.cpp`), and a small CLI (`tegra_tile`). It converts
between the Switch GPU's block-linear texture layout and plain rows, for every level, array layer and 3D slice.

## What it covers

- Bytes per block 1, 2, 4, 8, 12, 16; block footprints 1x1 (plain), 4x4 (BCn), 4x4 to 12x12 (ASTC).
- Block height of level 0 from 1 to 32 GOBs and block depth from 1 to 32 (3D), both powers of two. Small mips shrink the block height
  the way the hardware does (`gob_blocks_y` halves while the level fits in half a block); 3D mips halve the depth block once per level.
- Mip chains, array layers (2D), 3D textures with depth, partial GOBs at the right and bottom edges.
- Linear buffer layout identical to Ryujinx's (levels back to back, then layers, then z, then rows padded to 4 bytes);
  `linear_stride_align = 1` gives tight rows.
- Block height is an input. Packaged (BNTX) textures carry their own value (BRTI +0x34, `1 << (layout & 7)`), which is
  often one step smaller than `default_gob_blocks_y()`; use the file's (see `refs\SuperMarioWonderRecomp\docs\nvn\TILING.md`).
- Not covered: tile width above 1 (`gobBlocksInTileX`), 3D arrays, pitch-linear textures. The code is single-threaded.

```cpp
tegra::TextureDesc d;  d.width = 1920; d.height = 1080; d.levels = 11; d.bytes_per_block = 16; d.block_w = d.block_h = 4;
d.gob_blocks_y = 16;
tegra::Layout l = tegra::compute_layout(d);        // per-level offsets, sizes, effective block heights
tegra::untile(d, l, tiled, linear);                // or tile(); untile_level()/tile_level() for one level
```

CLI: `tegra_tile untile in.bin out.bin --width W --height H [--depth D] [--levels N] [--layers L] --bpb B [--block WxH] --gob-y G [--gob-z G] [--tight]`.

## Build

```powershell
pwsh -File tools\tiling\scripts\build.ps1                       # CMake + Ninja + clang-cl -> build\release
```

## Provenance and licences

| Files | From | Licence |
|---|---|---|
| `include/`, `src/`, `cli/` | written for this project from the layout rules below | GPL-3.0-or-later |
| layout rules | `Ryujinx.Graphics.Texture` (`SizeCalculator.cs`, `LayoutConverter.cs`, `BlockLinearLayout.cs`), `refs\deko3d`, Wonder `docs/nvn/TILING.md` | read as documentation; no code copied |

`ref/LAYOUT_NOTES.md` records the exact rules and the one place the first draft was wrong (`docs/debug/003.md`).
