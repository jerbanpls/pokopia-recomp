// SPDX-License-Identifier: GPL-3.0-or-later
// Pokémon Pokopia: the whole game module as one nxrt::game::Info (pkp_game.h). Play-mode hooks come here as they are found.
#include "pkp_game.h"
#include "pkp_linkplay.h"
#include "pkp_menu.h"
#include "pkp_play.h"
#include "pkp_quality.h"
#include "pkp_taa.h"

#include <cstring>

#include "nxrt/config.h"
#include "play_game.h"
#include "play_types.h"

namespace nxrt::games::pkp {

namespace {

// The main loop (0x7100882c60, called from nnMain's startup) calls 0x7101fdcde4 once per iteration on the main thread: the frame boundary
// that pumps window events and samples the pad/keyboard for play mode (needs `entryhook 0x7101fdcde4` in recomp/games/pkp/main.sites).
constexpr uint64_t kFrameUpdate = 0x1fdcde4;

// play mode's install / remove / status, plus the menu mouse (pkp_menu.cpp) and Link Play's Interact everywhere (pkp_linkplay.cpp)
void install_all(uint64_t base, play::GameHost* host, const play::Settings& s) {
    play_install(base, host, s);
    menu_install(base);
    quality_install(base, s);
    taa_install(base);
    linkplay_install(base, config_get_bool("pkp", "link_interact_everywhere", false));
}
void frame_all(bool in_game, double step_seconds, play::GameFrame* out) {
    linkplay_frame();   // the first frame checks main (loaded by now) for Interact everywhere
    play_frame(in_game, step_seconds, out);
}
void settings_changed_all(const play::Settings& s) {
    play_settings_changed(s);
    quality_settings_changed(s);
}
void remove_all() {
    linkplay_remove();
    taa_remove();
    quality_remove();
    menu_remove();
    play_remove();
}
void status_all(char* buf, size_t n) {
    play_status_text(buf, n);
    const size_t k = std::strlen(buf);
    if (k + 1 < n) menu_status_text(buf + k, n - k);
    const size_t k2 = std::strlen(buf);
    if (k2 + 1 < n) quality_status_text(buf + k2, n - k2);
    const size_t k3 = std::strlen(buf);
    if (k3 + 2 < n) {
        buf[k3] = ' ';
        taa_status_text(buf + k3 + 1, n - k3 - 1);
    }
}

const play::GamePlay& game_play() {
    static const play::GamePlay t = [] {
        play::GamePlay p;
        p.frame_update = kFrameUpdate;
        p.display_options = play::kDisplayOptQuality;   // view distance and the quality preset (pkp_quality.cpp)
        p.quality_preset = &quality_preset;
        p.install_render = &play_install_render;
        p.install = &install_all;
        p.remove = &remove_all;
        p.frame = &frame_all;
        p.settings_changed = &settings_changed_all;
        p.display_live = &play_display_live;
        p.display_hook_calls = &play_display_calls;
        p.status_text = &status_all;
        p.menu_frame = &menu_frame;
        p.menu_over_value = &menu_over_value;
        p.menu_active = &menu_active;
        p.menu_in_game = &menu_in_game;
        return p;
    }();
    return t;
}

}  // namespace

const game::Info& full_info() {
    static const game::Info info = [] {
        game::Info g = core_info();
        g.play = &game_play();
        return g;
    }();
    return info;
}

}  // namespace nxrt::games::pkp

void nxrt_register_game_pkp() { nxrt::game::set_current(&nxrt::games::pkp::full_info()); }
