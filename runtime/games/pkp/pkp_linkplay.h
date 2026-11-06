// SPDX-License-Identifier: GPL-3.0-or-later
// Pokémon Pokopia, Link Play "Interact everywhere" (pkp_linkplay.cpp, [pkp] link_interact_everywhere): an experimental opt-in that gives
// a Link Play session in the story areas Palette Town's Spectator-Off permissions (visitors can use moves and items, pick up, place and
// build). Every player of a session must have the same setting: it is sent as the ldn session variant (services_api.h ldnapi), so a peer
// whose setting differs is refused when it joins.
#pragma once
#include <cstdint>
#include <string>

namespace nxrt::games::pkp {

// One supported main build: the permission-mask builder `fn`, the site in it and the instruction word expected there (offsets in main).
struct InteractSite {
    const char* build;   // main's build id, lowercase hex (a prefix of Module::build_id())
    uint64_t fn, site;
    uint32_t word;
};
enum class InteractCheck { Ok, UnknownBuild, NotLifted, WordDiffers };

constexpr uint8_t kInteractVariant = 1;                   // the ldn session variant while the hook is armed
constexpr const char* kInteractName = "Interact everywhere";

// Pure: the site of main build `build_id` (null when none is known).
const InteractSite* interact_site(const std::string& build_id);
// Pure: whether the hook may be armed for this main: a known build, the site lifted as its own function (the lifted libs carry the
// midhook) and the expected instruction word at it. *site: the build's site, or null.
InteractCheck interact_check(const std::string& build_id, bool site_lifted, uint32_t word, const InteractSite** site);
// Pure: the register change at the site (x = x0..x30): while the session's Spectator flag (x22 bit 0) is clear, the Spectator pattern in
// x13 becomes the normal online one (0x5ffffffe; the movk at the site then adds bit 32, online). True when x13 changed.
bool interact_apply(uint64_t* x);

// Registers the midhook at main+site when `on` (play mode's install: main is not loaded yet, so nothing is checked here) and logs the
// choice. The hook does nothing until linkplay_arm() accepted main.
void linkplay_install(uint64_t base, bool on);
// Once per frame (play mode's frame: main is loaded): the first call checks main (its build id, the lifted site, the instruction word)
// and arms the hook or removes it; logs once either way.
void linkplay_frame();
// The decision of linkplay_frame() with main's facts given (tests): arms the hook and sets the ldn session variant on Ok, else removes
// the hook and clears the variant. Returns the check. Does nothing (returns Ok) when the hook was not installed.
InteractCheck linkplay_arm(const std::string& build_id, bool site_lifted, uint32_t word);
void linkplay_remove();
bool linkplay_armed();
uint64_t linkplay_changes();   // times the hook lifted the Spectator restriction

}  // namespace nxrt::games::pkp
