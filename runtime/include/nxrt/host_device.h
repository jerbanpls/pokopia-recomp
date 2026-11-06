// SPDX-License-Identifier: GPL-3.0-or-later
// The machine the runtime runs on, where it changes a default (src/host_device.cpp).
#pragma once

namespace nxrt {

// A gaming handheld played with its own controls: a Steam Deck (LCD or OLED) or a Lenovo Legion Go S, under SteamOS/Proton or
// Windows. Detected from Steam's SteamDeck=1 in the environment or the board (SMBIOS product name Jupiter or Galileo, family
// "Legion Go S ..."), read from the registry's BIOS key (Windows, and Wine fills it from SMBIOS) or Wine's Z:\sys\class\dmi.
// [host] handheld = auto (default) | on | off overrides it. What it changes: [display] render_scale auto renders at the window's
// size instead of at least 1920x1080 (a handheld's GPU), and [prompts] style auto keeps controller prompts when the trackpad,
// touchscreen or on-screen keyboard send mouse or keyboard input.
bool host_is_handheld();

// A Steam Deck (LCD or OLED) only: Steam's SteamDeck=1 or the board Jupiter / Galileo (same sources as above; [host] handheld does not
// change it). For defaults tied to the Deck's fixed-refresh panel, such as [display] fps_snap auto.
bool host_is_steam_deck();

// Running under Wine (Proton, SteamOS): ntdll exports wine_get_version. Read once; any thread.
bool host_is_wine();

}  // namespace nxrt
