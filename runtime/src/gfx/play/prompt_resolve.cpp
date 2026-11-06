// SPDX-License-Identifier: GPL-3.0-or-later
#include "prompt_resolve.h"
#include "nxrt/host_device.h"

#include <cctype>
#include <vector>

#include "nxrt/game.h"
#include "nxrt/input_source.h"
#include "play/pad_map.h"

namespace nxrt::play {

namespace {

std::string lower_ascii(std::string s) {
    for (char& c : s) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return s;
}

// The Switch button bit an icon stands for; the d-pad icon stands for "up" (any direction will do for the glyph of the whole cross).
uint64_t button_bit(Btn b) {
    using namespace hid;
    switch (b) {
        case Btn::A: return btn(kBtnA);
        case Btn::B: return btn(kBtnB);
        case Btn::X: return btn(kBtnX);
        case Btn::Y: return btn(kBtnY);
        case Btn::L: return btn(kBtnL);
        case Btn::R: return btn(kBtnR);
        case Btn::ZL: return btn(kBtnZL);
        case Btn::ZR: return btn(kBtnZR);
        case Btn::Plus: return btn(kBtnPlus);
        case Btn::Minus: return btn(kBtnMinus);
        case Btn::DPad: return btn(kBtnUp);
        default: return 0;
    }
}

// The pad source that presses Switch button `bit`, in this order: the lowest source whose remap is exactly that bit; the lowest source
// with a remap that contains it; the source whose built-in button it is, if that source is not remapped. False: nothing presses it.
bool source_for_button(const PadSettings& pad, bool label_abxy, uint64_t bit, PadSrc* out) {
    for (int i = 0; i < kPadSrcCount; ++i) {
        if (pad.remap[i] == bit) {
            *out = static_cast<PadSrc>(i);
            return true;
        }
    }
    for (int i = 0; i < kPadSrcCount; ++i) {
        if (pad.remap[i] != kPadDefault && (pad.remap[i] & bit)) {
            *out = static_cast<PadSrc>(i);
            return true;
        }
    }
    for (int i = 0; i < kPadSrcCount; ++i) {
        if (pad.remap[i] == kPadDefault && pad_default_buttons(static_cast<PadSrc>(i), label_abxy) == bit) {
            *out = static_cast<PadSrc>(i);
            return true;
        }
    }
    return false;
}

// Pad icons: the Xbox or PlayStation button that presses the Switch button. The icon names the physical button (South is the Xbox A,
// cross), so it is the same whatever the label question says; the label question only decides which source a Switch button comes from.
// Stick clicks and the d-pad have no icon of their own here: "" (the game keeps its icon). The d-pad icon is always the d-pad.
const char* pad_name(PromptStyle style, const PadSettings& pad, bool label_abxy, Btn b) {
    const bool xbox = style == PromptStyle::Xbox;
    if (b == Btn::DPad) return xbox ? "xbox/dpad" : "ps/dpad";
    PadSrc src = PadSrc::South;
    if (!source_for_button(pad, label_abxy, button_bit(b), &src)) return "";
    // Indexed by PadSrc: the four face buttons, the shoulders, the triggers, Start, Back; the sticks and the d-pad have no icon.
    static const char* const kXbox[kPadSrcCount] = {
        "xbox/a", "xbox/b", "xbox/x", "xbox/y", "xbox/lb", "xbox/rb", "xbox/lt", "xbox/rt", "xbox/menu", "xbox/view",
        "", "", "", "", "", "",
    };
    static const char* const kPs[kPadSrcCount] = {
        "ps/cross", "ps/circle", "ps/square", "ps/triangle", "ps/l1", "ps/r1", "ps/l2", "ps/r2", "ps/options", "ps/create",
        "", "", "", "", "", "",
    };
    return (xbox ? kXbox : kPs)[static_cast<int>(src)];
}

// Does binding b apply in context c? A binding of c applies; an `all` binding applies unless the same input has a binding of c itself.
bool binding_applies(const Settings& s, const Binding& b, Ctx c) {
    if (b.act == Act::None) return false;
    if (c == Ctx::All) return b.ctx == Ctx::All;
    if (b.ctx == c) return true;
    if (b.ctx != Ctx::All) return false;
    for (const Binding& o : s.bindings)
        if (o.ctx == c && o.in == b.in) return false;
    return true;
}

}  // namespace

const PromptGame* prompt_game() { return game::info().prompts; }

namespace {
const PromptIcon* icon_of(Icon i) {
    const PromptGame* g = prompt_game();
    return g && g->icons ? &g->icons[static_cast<int>(i)] : nullptr;
}
}  // namespace

Btn icon_button(Icon i) { const PromptIcon* p = icon_of(i); return p ? p->btn : Btn::None; }
bool icon_pressed(Icon i) { const PromptIcon* p = icon_of(i); return p && p->pressed; }
const char* icon_asset_name(Icon i) { const PromptIcon* p = icon_of(i); return p ? p->asset : ""; }
const char* icon_asset_uuid(Icon i) { const PromptIcon* p = icon_of(i); return p ? p->uuid : ""; }
const char* icon_pak_file(Icon i) { const PromptIcon* p = icon_of(i); return p ? p->pak : ""; }
const char* icon_front_uuid(Icon i) { const PromptIcon* p = icon_of(i); return p ? p->front_uuid : nullptr; }

std::string kbm_pack_name(const InputId& id) {
    const std::string n = lower_ascii(input_name(id));
    return n.empty() ? std::string() : "kbm/" + n;
}

bool input_for_button(const Settings& s, Ctx ctx, Btn b, InputId* out) {
    const uint64_t bit = button_bit(b);
    if (!bit) return false;
    // actions that do exactly this button: the named game actions first, the raw btn_ ones last
    std::vector<Act> cands;
    for (int pass = 0; pass < 2; ++pass)
        for (int a = 1; a < kActCount; ++a) {
            const bool raw = a >= static_cast<int>(Act::BtnA);
            if ((pass == 0) == raw) continue;
            const Output& o = s.out[a];
            if (o.buttons == bit && o.lx == 0 && o.ly == 0 && o.rx == 0 && o.ry == 0) cands.push_back(static_cast<Act>(a));
        }
    // the current context first, then the others (a menu button is still worth showing while playing)
    const Ctx order[] = {ctx, Ctx::Menu, Ctx::Play, Ctx::Alt, Ctx::Map, Ctx::Inspect, Ctx::All};
    for (Ctx c : order)
        for (Act a : cands)
            for (const Binding& bd : s.bindings)
                if (bd.act == a && binding_applies(s, bd, c)) {
                    *out = bd.in;
                    return true;
                }
    return false;
}

std::string resolve_icon(const ResolveIn& in, Icon icon) {
    // The map's stick icons: the mouse buttons that drive the map (left drag rotates, middle drag moves). Only keyboard style on the map.
    if (icon == Icon::LStick || icon == Icon::RStick) {
        if (in.style != PromptStyle::Keyboard || in.ctx != Ctx::Map) return {};
        return icon == Icon::LStick ? "kbm/mouse1" : "kbm/mouse2";
    }
    const Btn b = icon_button(icon);
    const PadSettings no_pad{};
    const PadSettings& pad = in.settings ? in.settings->pad : no_pad;
    std::string base;
    switch (in.style) {
        case PromptStyle::Xbox:
        case PromptStyle::PlayStation:
            base = pad_name(in.style, pad, in.label_abxy, b);
            break;
        case PromptStyle::Keyboard: {
            if (!in.settings) return {};
            InputId id;
            if (!input_for_button(*in.settings, in.ctx, b, &id)) return {};
            base = kbm_pack_name(id);
            break;
        }
        default:
            return {};
    }
    if (base.empty()) return {};
    return icon_pressed(icon) ? base + "_pressed" : base;
}

std::string pad_pack_name(PromptStyle style, const PadSettings& pad, bool label_abxy, uint64_t switch_bit) {
    if (style != PromptStyle::Xbox && style != PromptStyle::PlayStation) return {};
    if (switch_bit == 0 || (switch_bit & (switch_bit - 1)) != 0) return {};   // exactly one button bit
    using namespace hid;
    // one d-pad direction: the cross with that arm lit (the prompt names a direction); pad_pack_fallback() gives the whole cross
    const char* dir = switch_bit == btn(kBtnUp) ? "_up" : switch_bit == btn(kBtnDown) ? "_down" : switch_bit == btn(kBtnLeft) ? "_left"
                    : switch_bit == btn(kBtnRight) ? "_right" : nullptr;
    if (dir) return std::string(pad_name(style, pad, label_abxy, Btn::DPad)) + dir;
    Btn b = Btn::None;
    for (int i = 0; i < static_cast<int>(Btn::kCount); ++i)
        if (button_bit(static_cast<Btn>(i)) == switch_bit) {
            b = static_cast<Btn>(i);
            break;
        }
    if (b == Btn::None) return {};
    return pad_name(style, pad, label_abxy, b);
}

std::string pad_pack_fallback(const std::string& name) {
    for (const std::string dir : {"_up", "_down", "_left", "_right"}) {
        const std::string tail = "/dpad" + dir;
        if (name.size() > tail.size() && name.compare(name.size() - tail.size(), tail.size(), tail) == 0)
            return name.substr(0, name.size() - dir.size());
    }
    return {};
}

PromptStyle auto_style(LastDevice last, int last_pad_kind, int connected_pad_kind) {
    auto from_kind = [](int k) {
        switch (k) {
            case 2: return PromptStyle::PlayStation;
            case 3: return PromptStyle::Switch;
            default: return PromptStyle::Xbox;     // Xbox pads and every pad that is neither a PlayStation nor a Switch pad
        }
    };
    // A handheld is played with its own controls: its trackpad, touchscreen and on-screen keyboard reach the game as mouse and keyboard
    // input, which would switch the prompts to keyboard keys (the title screen showed Enter for A on a Legion Go S). There the built-in
    // pad (Xbox layout) decides; [prompts] style keyboard still shows keys.
    if (host_is_handheld()) return last == LastDevice::Pad ? from_kind(last_pad_kind) : connected_pad_kind >= 0 ? from_kind(connected_pad_kind) : PromptStyle::Xbox;
    switch (last) {
        case LastDevice::KeyboardMouse: return PromptStyle::Keyboard;
        case LastDevice::Pad: return from_kind(last_pad_kind);
        default: return connected_pad_kind >= 0 ? from_kind(connected_pad_kind) : PromptStyle::Keyboard;
    }
}

}  // namespace nxrt::play
