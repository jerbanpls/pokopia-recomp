// SPDX-License-Identifier: GPL-3.0-or-later
// Pokémon Pokopia, Link Play "Interact everywhere" (pkp_linkplay.h). Based on pokopia-native (MIT) windows/src/coop/coop_interact.c and
// windows/docs/coop-interact.md @555813a.
//
// The game keeps a session's permissions in a 36-bit mask (GameMgr+0x8c; bits 32-35, the byte at +0x90, are online / host / visitor) and
// rebuilds it in one function, 0xb0f60 of the 2.0.1 main, when hosting starts, when a town loads and when the session ends. Offline every
// bit but 31 is set. Online in a story area (stage type 1) the player always gets the Spectator pattern (bits 23-26 and 31: "certain moves
// and items can't be used"), whatever the host chose; online in Palette Town with Spectator Mode Off the normal one (bits 1-28 and 30). At
// 0xb1030, `movk x13, #1, lsl #32` in the story-area online block, x13 holds the pattern's bits 0-31 (0x87800000, set at 0xb1020) and the
// movk adds bit 32; x22 is the session's Spectator flag (w3 of the call, clear in every story area). The midhook there (recomp/games/pkp/
// main.sites) swaps the pattern for the normal one while the flag is clear: the mask is then exactly Palette Town's with Spectator Mode Off
// (pokopia-native measured fe ff ff 5f 03 for a host, fe ff ff 5f 05 for a visitor). Palette Town, the other type-3 towns, offline play, the
// host's Spectator setting, the session flag and the advertise data are not touched. Known gaps (pokopia-native): the visitor's join notice
// and the host's settings row still say Spectator Mode; a visitor's bag goes home with it as in Palette Town.
#include "pkp_linkplay.h"

#include <atomic>
#include <cstring>
#include <vector>

#include "nxrt/dispatch.h"
#include "nxrt/hooks.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/module.h"
#include "nxrt/services_api.h"

namespace nxrt::games::pkp {

namespace {

constexpr const char* kTag = "pkp";

// Pokopia 2.0.1 main (build 76a489f3...): the mask builder and its story-area online block.
constexpr InteractSite kSites[] = {
    {"76a489f3bd3642fa5e18bd83d32c98ba", 0xb0f60, 0xb1030, 0xf2c0002du},
};
constexpr uint64_t kSpectatorFlagReg = 22, kPatternReg = 13;
constexpr uint64_t kNormalPattern = 0x5ffffffeull;   // bits 1-28 and 30: Palette Town, Spectator Mode Off

struct State {
    uint64_t base = 0;
    uint32_t hook = 0;                     // 0: not installed
    std::atomic<bool> armed{false};        // main checked: the hook may change the mask
    std::atomic<bool> checked{false};      // linkplay_frame() made its decision
    std::atomic<bool> logged_change{false};
    std::atomic<uint64_t> changes{0};
};
State& st() {
    static State s;
    return s;
}

int hook_mask(::Ctx* c, uint64_t, void*) {
    State& s = st();
    if (!s.armed.load(std::memory_order_acquire)) return 0;
    if (interact_apply(c->x)) {
        s.changes.fetch_add(1, std::memory_order_relaxed);
        if (!s.logged_change.exchange(true))
            NXLOG_I(kTag, "Interact everywhere: this story area's Link Play session has Palette Town's permissions (Spectator Mode lifted)");
    }
    return 0;
}

const char* check_str(InteractCheck c) {
    switch (c) {
        case InteractCheck::Ok: return "ok";
        case InteractCheck::UnknownBuild: return "main is not a supported build (2.0.1 only)";
        case InteractCheck::NotLifted: return "the lifted game libraries lack the hook site (rebuild them)";
        case InteractCheck::WordDiffers: return "the instruction at the site is not the expected one";
    }
    return "?";
}

}  // namespace

const InteractSite* interact_site(const std::string& build_id) {
    for (const InteractSite& s : kSites)
        if (build_id.compare(0, std::strlen(s.build), s.build) == 0) return &s;
    return nullptr;
}

InteractCheck interact_check(const std::string& build_id, bool site_lifted, uint32_t word, const InteractSite** site) {
    const InteractSite* s = interact_site(build_id);
    if (site) *site = s;
    if (!s) return InteractCheck::UnknownBuild;
    if (!site_lifted) return InteractCheck::NotLifted;
    if (word != s->word) return InteractCheck::WordDiffers;
    return InteractCheck::Ok;
}

bool interact_apply(uint64_t* x) {
    if (x[kSpectatorFlagReg] & 1) return false;           // the session's own Spectator flag: left as the game has it
    if (x[kPatternReg] == kNormalPattern) return false;
    x[kPatternReg] = kNormalPattern;                      // the movk that follows adds bit 32 (online)
    return true;
}

void linkplay_install(uint64_t base, bool on) {
    State& s = st();
    if (s.hook) return;
    s.base = base;
    s.armed.store(false);
    s.checked.store(false);
    s.logged_change.store(false);
    s.changes.store(0);
    services::ldnapi::set_session_variant(0, kInteractName);
    if (!on) {
        NXLOG_I(kTag, "Interact everywhere ([pkp] link_interact_everywhere): off, Link Play outside Palette Town keeps the game's Spectator Mode");
        return;
    }
    s.hook = hook_add_mid(base + kSites[0].site, &hook_mask);
    NXLOG_I(kTag, "Interact everywhere ([pkp] link_interact_everywhere): on, hook at main+0x%llx, main is checked once the game runs",
            static_cast<unsigned long long>(kSites[0].site));
}

InteractCheck linkplay_arm(const std::string& build_id, bool site_lifted, uint32_t word) {
    State& s = st();
    if (!s.hook) return InteractCheck::Ok;
    s.checked.store(true);
    const InteractSite* site = nullptr;
    const InteractCheck c = interact_check(build_id, site_lifted, word, &site);
    const std::string shown = build_id.substr(0, 32);
    if (c == InteractCheck::Ok) {
        services::ldnapi::set_session_variant(kInteractVariant, kInteractName);
        s.armed.store(true, std::memory_order_release);
        NXLOG_I(kTag, "Interact everywhere: armed at main+0x%llx (build %s); Link Play sessions are refused to players without it (session variant %u)",
                static_cast<unsigned long long>(site->site), shown.c_str(), kInteractVariant);
        return c;
    }
    s.armed.store(false);
    hook_remove(s.hook);
    s.hook = 0;
    services::ldnapi::set_session_variant(0, kInteractName);
    if (c == InteractCheck::WordDiffers)
        NXLOG_W(kTag, "Interact everywhere: not applied: the word at main+0x%llx is %08x, expected %08x (build %s): Link Play keeps the game's rules",
                static_cast<unsigned long long>(site->site), word, site->word, shown.c_str());
    else
        NXLOG_W(kTag, "Interact everywhere: not applied: %s (build %s): Link Play keeps the game's rules", check_str(c),
                shown.empty() ? "unknown" : shown.c_str());
    return c;
}

void linkplay_frame() {
    State& s = st();
    if (!s.hook || s.checked.load(std::memory_order_relaxed)) return;
    const Module* m = module_at(s.base);
    const std::string build = m ? m->build_id() : std::string();
    const InteractSite* site = interact_site(build);
    bool lifted = false;
    uint32_t word = 0;
    if (site) {
        const uint64_t a = s.base + site->site;
        lifted = fn_lookup(a) != nullptr;   // a midhook makes its address a lifted function of its own
        if (mem::is_mapped(a, 4)) std::memcpy(&word, reinterpret_cast<const void*>(a), 4);
    }
    linkplay_arm(build, lifted, word);
}

void linkplay_remove() {
    State& s = st();
    s.armed.store(false);
    if (s.hook) hook_remove(s.hook);
    s.hook = 0;
    s.checked.store(false);
    services::ldnapi::set_session_variant(0, kInteractName);
}

bool linkplay_armed() { return st().armed.load(); }
uint64_t linkplay_changes() { return st().changes.load(); }

}  // namespace nxrt::games::pkp
