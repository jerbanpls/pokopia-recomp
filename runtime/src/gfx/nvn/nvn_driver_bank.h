// SPDX-License-Identifier: GPL-3.0-or-later
// NVN's driver constant bank c[0] as the GLSLC code reads it, filled from the current bindings for one draw or dispatch (no Vulkan,
// no replay state: the replay hands in the ranges and handles).
//
// Switch 2 compute programs keep a packed per-program table there, slot n = NVN binding n:
//   0x00  16 header bytes (zero)
//   0x10  16 bytes per uniform block      {address lo, address hi, size, 0}   count: control byte +0xb1
//         16 bytes per storage buffer     {address lo, address hi, size, 0}   count: +0xb2
//          8 bytes per texture handle     {handle, 0}                         count: +0xb3
//          8 bytes per image handle       {handle, 0}                         count: +0xb6
// The translated shaders read the address words (GlobalToStorage: a global access is matched against [address, address + size)) and
// the size words as bounds checks (clustered light culling: `uint(i + n) > uint(c0[2].z >> 2)` gives 0 past the end). A bank of
// zeros fails every such check. The idea of filling it per dispatch is pokopia-native's drv_fill (MIT,
// windows/src/nxrt/vk/vk_replay.c @7c3655d); the offsets are checked against our translator's c[0] reads (results.tsv sb[c0+...]
// words) and spirv-cross output of the five compute shaders that read sizes.
//
// Graphics stages share the fixed Maxwell layout:
//   0x110 + 0x100 * hardware stage + 0x10 * n   storage buffer n {address lo, address hi, size} (vertex 0, tess control 1, tess
//                                               evaluation 2, geometry 3, fragment 4; storage_slot_for in nvn_replay.cpp)
//   0x610                                       the largest point size (float; the agl vertex shaders clamp gl_PointSize to it)
//   0x644                                       the draw's base instance, which GLSLC adds to the hardware instance ID: the DOF
//                                               bokeh and lens flare sprites read c0[100].y. Vertex modules get that read replaced by
//                                               Vulkan's BaseInstance (spirv_base_instance_from_c0), which also covers indirect draws;
//                                               the word here is the direct draw's value for any read the rewrite does not match.
#pragma once
#include <cstdint>

#include "nvn_shaders.h"

namespace nxrt::nvn::be {

// Bytes of c[0] the replay fills and binds per draw / dispatch. Everything NVN keeps in c[0] that a Pokopia shader reads lies
// below it (compute table rows 2..8, fragment row 0x55, vertex row 0x64); the SPIR-V declares vec4[4096], read only by constant
// index in those shaders.
constexpr uint32_t kDriverBankBytes = 0x800;

struct DriverRange {
    uint64_t addr = 0, size = 0;
};

// Byte offsets of the NVN2 compute table (see above). end = the table's size (equals control u16 +0xba in the corpus).
struct Nvn2ComputeTable {
    int ubo0 = 0x10, ssbo0 = 0x10, tex0 = 0x10, img0 = 0x10, end = 0x10;
};
Nvn2ComputeTable nvn2_compute_table(const ShaderControl& c);

// Fills d[0, kDriverBankBytes) for a Switch 2 compute program: zeros, then the table of `c` from the stage's bindings (ubo / ssbo:
// 16 slots, tex: 64, img: 16, as the replay keeps them; slots past those are left zero). Returns the bytes written from 0 (the
// table's end, clamped to kDriverBankBytes), so a caller can key a cache on d[0, n).
uint32_t fill_driver_bank_nvn2_compute(uint8_t* d, const ShaderControl& c, const DriverRange ubo[16], const DriverRange ssbo[16],
                                       const uint64_t tex[64], const uint64_t img[16]);

// c[0] offsets of the graphics layout above.
constexpr uint32_t kDriverBankPointSize = 0x610;
constexpr uint32_t kDriverBankBaseInstance = 0x644;
constexpr float kDriverBankMaxPointSize = 64.0f;   // what the shared bank has always held

// Fills d[0, kDriverBankBytes) for graphics stage `nvn_stage` (NVN numbering: 0 vertex, 1 fragment, 2 geometry, 3 tess control,
// 4 tess evaluation): zeros, its storage buffer table, the point size and the base instance. Returns the bytes up to the last word.
uint32_t fill_driver_bank_graphics(uint8_t* d, int nvn_stage, const DriverRange ssbo[16], uint32_t base_instance);

}  // namespace nxrt::nvn::be
