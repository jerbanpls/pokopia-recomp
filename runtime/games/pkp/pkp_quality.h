// SPDX-License-Identifier: GPL-3.0-or-later
// Pokémon Pokopia, PC picture quality above the console's (pkp_quality.cpp): the camera's view distance and a quality preset that sets it
// together with the renderer's texture sharpness ([display] lod_bias, anisotropy). Wired into play mode by pkp_game.cpp.
#pragma once
#include <cstddef>
#include <cstdint>

#include "play_types.h"

namespace nxrt::games::pkp {

void quality_install(uint64_t base, const play::Settings& s);
void quality_remove();
void quality_settings_changed(const play::Settings& s);
// F1 Graphics page: the values of a preset written into d. "switch2", "high", "ultra": view distance, texture LOD bias, anisotropy;
// "steamdeck", "legion_go_s", "laptop", "desktop": those and the render scale and frame limit. False for an unknown name (and "custom").
bool quality_preset(const char* name, play::DisplaySettings& d);
void quality_status_text(char* buf, size_t n);

}  // namespace nxrt::games::pkp
