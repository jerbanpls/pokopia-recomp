#!/usr/bin/env python3
"""Build the MPIP prompt-icon pack from the Kenney "Input Prompts" pack (CC0).

Build (run from the prompts project root):
  python -I scripts/build_prompt_pack.py --kenney <pack folder or .zip> --out assets/prompts/prompt_icons.pak [--preview <png>]

Check an existing pack:
  python -I scripts/build_prompt_pack.py --verify assets/prompts/prompt_icons.pak

Add the derived entries (the d-pad direction icons) to an existing pack without the Kenney pack; other entries are kept byte for byte:
  python -I scripts/build_prompt_pack.py --derive assets/prompts/prompt_icons.pak

Pack format, little endian:
  header (24 bytes): char magic[4]="MPIP", u32 version=1, u32 count, u32 icon_w=36, u32 icon_h=36, u32 reserved=0
  count directory entries (48 bytes each): char name[40] (ASCII, NUL padded), u32 offset, u32 size (=5184)
  pixel blocks: raw RGBA8, 36x36, straight alpha, sRGB values, row 0 = top, no padding.
  Entries are sorted by name in plain byte order.

Key names follow SDL3 scancode names (lower-cased), taken from SDL 3.4.18 SDL_scancode_names[]
in src/events/SDL_keymap.c of the build tree (runtime/build/release/_deps/sdl3-src).
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import struct
import sys
import tempfile
import zipfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ICON = 36
BOX = 33                    # icon content is fitted into a BOX x BOX square
BLOCK = ICON * ICON * 4     # 5184 bytes per icon
MAGIC = b"MPIP"
VERSION = 1
HEADER_FMT = "<4sIIIII"
ENTRY_FMT = "<40sII"
HEADER_SIZE = struct.calcsize(HEADER_FMT)   # 24
ENTRY_SIZE = struct.calcsize(ENTRY_FMT)     # 48
NAME_FIELD = 40

SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_FONT = SCRIPT_DIR.parent / "assets" / "fonts" / "DejaVuSans.ttf"
RESAMPLE = getattr(Image, "Resampling", Image).LANCZOS

XBOX_DIR = "Xbox Series"
PS_DIR = "PlayStation Series"
KBM_DIR = "Keyboard & Mouse"

XBOX_MAP = {
    "xbox/a": "xbox_button_color_a",
    "xbox/b": "xbox_button_color_b",
    "xbox/x": "xbox_button_color_x",
    "xbox/y": "xbox_button_color_y",
    "xbox/lb": "xbox_lb",
    "xbox/rb": "xbox_rb",
    "xbox/lt": "xbox_lt",
    "xbox/rt": "xbox_rt",
    "xbox/menu": "xbox_button_menu",
    "xbox/view": "xbox_button_view",
    "xbox/dpad": "xbox_dpad",
}

PS_MAP = {
    "ps/cross": "playstation_button_color_cross",
    "ps/circle": "playstation_button_color_circle",
    "ps/square": "playstation_button_color_square",
    "ps/triangle": "playstation_button_color_triangle",
    "ps/l1": "playstation_trigger_l1",
    "ps/r1": "playstation_trigger_r1",
    "ps/l2": "playstation_trigger_l2",
    "ps/r2": "playstation_trigger_r2",
    "ps/options": "playstation5_button_options",
    "ps/create": "playstation5_button_create",
    "ps/dpad": "playstation_dpad",
}

# One d-pad direction each, for prompts that name a direction (Pokopia's left/right/up/down glyphs): <style>/dpad_<dir>, drawn from the
# whole cross <style>/dpad by dpad_direction(). Value: the dead centre the arms leave out, in pixels from the icon centre (the Xbox cross
# is solid there; the PlayStation arms are four pieces split along the diagonals).
DPAD_DIRS = ("up", "down", "left", "right")
DPAD_CORE = {"xbox/dpad": 6.5, "ps/dpad": 0.0}
DPAD_DIM = 0.4              # the arms not pressed: the cross's luminance x this (a grey cross), alpha kept

# Mouse plus the last-resort generic key.
OTHER_KBM_MAP = {
    "kbm/mouse1": "mouse_left",
    "kbm/mouse2": "mouse_scroll",
    "kbm/mouse3": "mouse_right",
    "kbm/mouse4": "mouse_side_back",
    "kbm/mouse5": "mouse_side_forward",
    "kbm/wheelup": "mouse_scroll_up",
    "kbm/wheeldown": "mouse_scroll_down",
    "kbm/any": "keyboard_any",
}

# SDL3 scancode names for scancodes 4..284, in scancode order, NULL/empty entries skipped.
# "Return" appears twice (scancodes 40 and 158); the build keeps one entry per name.
SDL_SCANCODE_NAMES = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "0",
    "Return", "Escape", "Backspace", "Tab", "Space", "-", "=", "[", "]", "\\", "#", ";", "'", "`", ",", ".", "/", "CapsLock",
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    "PrintScreen", "ScrollLock", "Pause", "Insert", "Home", "PageUp", "Delete", "End", "PageDown",
    "Right", "Left", "Down", "Up", "Numlock",
    "Keypad /", "Keypad *", "Keypad -", "Keypad +", "Keypad Enter",
    "Keypad 1", "Keypad 2", "Keypad 3", "Keypad 4", "Keypad 5", "Keypad 6", "Keypad 7", "Keypad 8", "Keypad 9", "Keypad 0",
    "Keypad .", "NonUSBackslash", "Application", "Power", "Keypad =",
    "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
    "Execute", "Help", "Menu", "Select", "Stop", "Again", "Undo", "Cut", "Copy", "Paste", "Find", "Mute", "VolumeUp", "VolumeDown",
    "Keypad ,", "Keypad = (AS400)",
    "International 1", "International 2", "International 3", "International 4", "International 5",
    "International 6", "International 7", "International 8", "International 9",
    "Language 1", "Language 2", "Language 3", "Language 4", "Language 5", "Language 6", "Language 7", "Language 8", "Language 9",
    "AltErase", "SysReq", "Cancel", "Clear", "Prior", "Return", "Separator", "Out", "Oper", "Clear / Again", "CrSel", "ExSel", "Front",
    "Keypad 00", "Keypad 000", "ThousandsSeparator", "DecimalSeparator", "CurrencyUnit", "CurrencySubUnit",
    "Keypad (", "Keypad )", "Keypad {", "Keypad }", "Keypad Tab", "Keypad Backspace",
    "Keypad A", "Keypad B", "Keypad C", "Keypad D", "Keypad E", "Keypad F",
    "Keypad XOR", "Keypad ^", "Keypad %", "Keypad <", "Keypad >", "Keypad &", "Keypad &&", "Keypad |", "Keypad ||",
    "Keypad :", "Keypad #", "Keypad Space", "Keypad @", "Keypad !",
    "Keypad MemStore", "Keypad MemRecall", "Keypad MemClear", "Keypad MemAdd", "Keypad MemSubtract", "Keypad MemMultiply", "Keypad MemDivide",
    "Keypad +/-", "Keypad Clear", "Keypad ClearEntry", "Keypad Binary", "Keypad Octal", "Keypad Decimal", "Keypad Hexadecimal",
    "Left Ctrl", "Left Shift", "Left Alt", "Left GUI", "Right Ctrl", "Right Shift", "Right Alt", "Right GUI",
    "ModeSwitch", "Sleep", "Wake", "ChannelUp", "ChannelDown", "MediaPlay", "MediaPause", "MediaRecord",
    "MediaFastForward", "MediaRewind", "MediaTrackNext", "MediaTrackPrevious", "MediaStop", "Eject", "MediaPlayPause", "MediaSelect",
    "AC New", "AC Open", "AC Close", "AC Exit", "AC Save", "AC Print", "AC Properties", "AC Search", "AC Home", "AC Back", "AC Forward",
    "AC Stop",
]

# SDL key name (lower-case) -> Kenney Double image stem, where Kenney has one.
KEY_IMAGE = {c: f"keyboard_{c}" for c in "abcdefghijklmnopqrstuvwxyz"}
KEY_IMAGE.update({str(d): f"keyboard_{d}" for d in range(10)})
KEY_IMAGE.update({f"f{i}": f"keyboard_f{i}" for i in range(1, 13)})
KEY_IMAGE.update({
    "return": "keyboard_return",
    "escape": "keyboard_escape",
    "backspace": "keyboard_backspace",
    "tab": "keyboard_tab",
    "space": "keyboard_space",
    "-": "keyboard_minus",
    "=": "keyboard_equals",
    "[": "keyboard_bracket_open",
    "]": "keyboard_bracket_close",
    "\\": "keyboard_slash_back",
    ";": "keyboard_semicolon",
    "'": "keyboard_apostrophe",
    "`": "keyboard_tilde",
    ",": "keyboard_comma",
    ".": "keyboard_period",
    "/": "keyboard_slash_forward",
    "capslock": "keyboard_capslock",
    "printscreen": "keyboard_printscreen",
    "scrolllock": "keyboard_scroll_lock",
    "pause": "keyboard_pause",
    "insert": "keyboard_insert",
    "home": "keyboard_home",
    "pageup": "keyboard_page_up",
    "delete": "keyboard_delete",
    "end": "keyboard_end",
    "pagedown": "keyboard_page_down",
    "up": "keyboard_arrow_up",
    "down": "keyboard_arrow_down",
    "left": "keyboard_arrow_left",
    "right": "keyboard_arrow_right",
    "numlock": "keyboard_numlock",
    "keypad +": "keyboard_numpad_plus",
    "keypad enter": "keyboard_numpad_enter",
    "left ctrl": "keyboard_ctrl",
    "right ctrl": "keyboard_ctrl",
    "left shift": "keyboard_shift",
    "right shift": "keyboard_shift",
    "left alt": "keyboard_alt",
    "right alt": "keyboard_alt",
    "left gui": "keyboard_win",
    "right gui": "keyboard_win",
})

# Short labels for keys without a Kenney image (lower-case SDL name -> label).
# Names not listed here fall back to label_for().
BAKED_LABELS = {
    "#": "#",
    "nonusbackslash": "ISO",
    "application": "APP",
    "power": "PWR",
    "execute": "EXE",
    "help": "HELP",
    "menu": "MENU",
    "select": "SEL",
    "stop": "STOP",
    "again": "AGN",
    "undo": "UNDO",
    "cut": "CUT",
    "copy": "CPY",
    "paste": "PST",
    "find": "FIND",
    "mute": "MUTE",
    "volumeup": "VOL+",
    "volumedown": "VOL-",
    "keypad = (as400)": "KP=",
    "keypad memstore": "MS",
    "keypad memrecall": "MR",
    "keypad memclear": "MC",
    "keypad memadd": "M+",
    "keypad memsubtract": "M-",
    "keypad memmultiply": "M*",
    "keypad memdivide": "M/",
    "keypad +/-": "+/-",
    "keypad clear": "CLR",
    "keypad clearentry": "CE",
    "keypad binary": "BIN",
    "keypad octal": "OCT",
    "keypad decimal": "DEC",
    "keypad hexadecimal": "HEX",
    "keypad xor": "XOR",
    "keypad tab": "TAB",
    "keypad backspace": "BKSP",
    "keypad space": "SPC",
    "thousandsseparator": "TSEP",
    "decimalseparator": "DSEP",
    "currencyunit": "CUR",
    "currencysubunit": "CSUB",
    "alterase": "ALTE",
    "sysreq": "SYS",
    "cancel": "CNCL",
    "clear": "CLR",
    "prior": "PRIO",
    "separator": "SEP",
    "out": "OUT",
    "oper": "OPER",
    "clear / again": "CLRA",
    "crsel": "CRSL",
    "exsel": "EXSL",
    "front": "FRNT",
    "modeswitch": "MODE",
    "sleep": "SLP",
    "wake": "WAKE",
    "channelup": "CH+",
    "channeldown": "CH-",
    "mediaplay": "PLAY",
    "mediapause": "PAUS",
    "mediarecord": "REC",
    "mediafastforward": "FF",
    "mediarewind": "REW",
    "mediatracknext": "NEXT",
    "mediatrackprevious": "PREV",
    "mediastop": "MSTP",
    "eject": "EJCT",
    "mediaplaypause": "P/P",
    "mediaselect": "MSEL",
    "ac new": "NEW",
    "ac open": "OPEN",
    "ac close": "CLS",
    "ac exit": "EXIT",
    "ac save": "SAVE",
    "ac print": "PRNT",
    "ac properties": "PROP",
    "ac search": "SRCH",
    "ac home": "HOME",
    "ac back": "BACK",
    "ac forward": "FWD",
    "ac stop": "ASTP",
    "ac refresh": "REFR",
}


def label_for(lname: str, sdl: str) -> str:
    if lname in BAKED_LABELS:
        return BAKED_LABELS[lname]
    m = re.fullmatch(r"(international|language) (\d)", lname)
    if m:
        return ("INT" if m.group(1) == "international" else "LNG") + m.group(2)
    base = "KP" + sdl[len("Keypad "):] if lname.startswith("keypad ") else sdl
    return re.sub(r"\s+", "", base).upper()[:5]


# ---------------------------------------------------------------- source loading

def find_pack_root(p: Path) -> Path:
    if (p / XBOX_DIR).is_dir():
        return p
    for dirpath, dirnames, _ in os.walk(p):
        if XBOX_DIR in dirnames:
            return Path(dirpath)
    raise SystemExit(f"'{XBOX_DIR}' folder not found under {p}")


def open_kenney(arg: str, tmp_dirs: list[Path]) -> Path:
    p = Path(arg)
    if p.is_file() and zipfile.is_zipfile(p):
        dest = Path(tempfile.mkdtemp(prefix="kenney_prompts_"))
        tmp_dirs.append(dest)
        dest_resolved = dest.resolve()
        with zipfile.ZipFile(p) as zf:
            for m in zf.infolist():
                target = (dest / m.filename).resolve()
                if dest_resolved not in target.parents:
                    raise SystemExit(f"unsafe path in zip: {m.filename}")
            zf.extractall(dest)
        p = dest
    if not p.is_dir():
        raise SystemExit(f"not a folder or zip: {arg}")
    return find_pack_root(p)


def load_src(root: Path, folder: str, stem: str) -> Image.Image:
    f = root / folder / "Double" / f"{stem}.png"
    if not f.is_file():
        raise SystemExit(f"missing Kenney image: {f}")
    return Image.open(f).convert("RGBA")


# ---------------------------------------------------------------- image processing

def fit_icon(src: Image.Image) -> Image.Image:
    """Crop to alpha > 8, fit into BOX x BOX (aspect kept), centre on a 36x36 transparent canvas."""
    mask = src.getchannel("A").point(lambda v: 255 if v > 8 else 0)
    box = mask.getbbox()
    if box is None:
        raise SystemExit("icon has no visible pixels")
    crop = src.crop(box)
    w, h = crop.size
    s = BOX / max(w, h)
    nw = min(BOX, max(1, round(w * s)))
    nh = min(BOX, max(1, round(h * s)))
    # "RGBa" is Pillow's premultiplied-alpha mode: resample there (no dark fringes),
    # then convert back to straight RGBA, which un-premultiplies.
    scaled = crop.convert("RGBa").resize((nw, nh), RESAMPLE).convert("RGBA")
    canvas = Image.new("RGBA", (ICON, ICON), (0, 0, 0, 0))
    canvas.paste(scaled, ((ICON - nw) // 2, (ICON - nh) // 2))
    return canvas


def pressed_variant(icon: Image.Image) -> Image.Image:
    """Gold, slightly brighter copy: luminance x (1.0, 0.72, 0.18) x 1.15, alpha kept."""
    a = icon.getchannel("A")
    lum = icon.convert("L")

    def chan(k: float) -> Image.Image:
        return lum.point(lambda v: min(255, int(round(v * k * 1.15))))

    return Image.merge("RGBA", (chan(1.0), chan(0.72), chan(0.18), a))


def dpad_direction(cross: Image.Image, d: str, core: float) -> Image.Image:
    """The whole d-pad cross with one arm left as it is and the rest dimmed to grey. A pixel is in the arm of `d` when it lies beyond the
    diagonal through the centre of the drawn cross on that side and more than `core` from the centre along the arm."""
    box = cross.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
    if box is None:
        raise SystemExit("d-pad icon has no visible pixels")
    cx, cy = (box[0] + box[2]) / 2, (box[1] + box[3]) / 2
    src = cross.load()
    out = cross.copy()
    px = out.load()
    for y in range(ICON):
        for x in range(ICON):
            dx, dy = x + 0.5 - cx, y + 0.5 - cy
            along = {"up": -dy, "down": dy, "left": -dx, "right": dx}[d]
            across = abs(dx) if d in ("up", "down") else abs(dy)
            if along > across and along > core:
                continue
            r, g, b, a = src[x, y]
            px[x, y] = (round(r * DPAD_DIM), round(g * DPAD_DIM), round(b * DPAD_DIM), a)
    return out


def add_dpad_directions(icons: dict[str, Image.Image]) -> list[str]:
    """Adds <style>/dpad_<dir> for every whole cross in DPAD_CORE (no _pressed twins). Returns the added names."""
    added = []
    for cross, core in DPAD_CORE.items():
        for d in DPAD_DIRS:
            icons[f"{cross}_{d}"] = dpad_direction(icons[cross], d, core)
            added.append(f"{cross}_{d}")
    return added


def read_pak(path: Path) -> dict[str, Image.Image]:
    data = path.read_bytes()
    magic, ver, count, w, h, _ = struct.unpack_from(HEADER_FMT, data, 0)
    if magic != MAGIC or ver != VERSION or (w, h) != (ICON, ICON):
        raise SystemExit(f"not an MPIP v{VERSION} {ICON}x{ICON} pack: {path}")
    icons = {}
    for i in range(count):
        raw, off, size = struct.unpack_from(ENTRY_FMT, data, HEADER_SIZE + ENTRY_SIZE * i)
        icons[raw.rstrip(b"\0").decode("ascii")] = Image.frombytes("RGBA", (ICON, ICON), data[off:off + size])
    return icons


def derive(path: Path) -> int:
    icons = read_pak(path)
    added = add_dpad_directions(icons)
    for n in added:
        icons[n + "_pressed"] = pressed_variant(icons[n])
    size = write_pak(path, icons)
    print(f"wrote {path} ({size} bytes), {len(icons)} entries; derived: {', '.join(added)} (+ _pressed)")
    return 0


_font_cache: dict[tuple[str, int], ImageFont.FreeTypeFont] = {}


def _font(path: Path, size: int) -> ImageFont.FreeTypeFont:
    key = (str(path), size)
    if key not in _font_cache:
        _font_cache[key] = ImageFont.truetype(str(path), size)
    return _font_cache[key]


def baked_key(cap: Image.Image, label: str, font_path: Path) -> Image.Image:
    """Draw a short label in the middle of the blank key cap (128 px source), largest size that fits."""
    im = cap.copy()
    x0, y0, x1, y1 = im.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
    cw, ch = x1 - x0, y1 - y0
    draw = ImageDraw.Draw(im)
    size = int(ch * 0.6)
    while True:
        f = _font(font_path, size)
        l, t, r, b = draw.textbbox((0, 0), label, font=f, anchor="mm")
        if (r - l) <= cw * 0.8 and (b - t) <= ch * 0.5 or size <= 6:
            break
        size -= 1
    draw.text(((x0 + x1) / 2, (y0 + y1) / 2), label, font=f, fill=(30, 30, 30, 255), anchor="mm")
    return im


# ---------------------------------------------------------------- pack I/O

def write_pak(path: Path, icons: dict[str, Image.Image]) -> int:
    names = sorted(icons, key=lambda n: n.encode("ascii"))
    count = len(names)
    data_start = HEADER_SIZE + ENTRY_SIZE * count
    directory = bytearray()
    blocks = bytearray()
    for n in names:
        nb = n.encode("ascii")
        if len(nb) >= NAME_FIELD:
            raise SystemExit(f"name too long for the 40-byte field: {n}")
        img = icons[n]
        if img.mode != "RGBA" or img.size != (ICON, ICON):
            raise SystemExit(f"bad icon {n}: {img.mode} {img.size}")
        directory += struct.pack(ENTRY_FMT, nb, data_start + len(blocks), BLOCK)
        blocks += img.tobytes()
    header = struct.pack(HEADER_FMT, MAGIC, VERSION, count, ICON, ICON, 0)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(bytes(header) + bytes(directory) + bytes(blocks))
    return path.stat().st_size


def verify(path: Path) -> int:
    data = path.read_bytes()
    problems: list[str] = []
    if len(data) < HEADER_SIZE:
        print(f"FAIL: file too short ({len(data)} bytes)")
        return 1
    magic, ver, count, w, h, res = struct.unpack_from(HEADER_FMT, data, 0)
    print(f"header: magic={magic!r} version={ver} count={count} icon={w}x{h} reserved={res}")
    if magic != MAGIC:
        problems.append("bad magic")
    if ver != VERSION:
        problems.append(f"bad version {ver}")
    if (w, h) != (ICON, ICON):
        problems.append(f"bad icon size {w}x{h}")
    if res != 0:
        problems.append("reserved not 0")
    dir_end = HEADER_SIZE + ENTRY_SIZE * count
    if dir_end > len(data):
        problems.append("directory runs past end of file")
        names, entries = [], []
    else:
        entries = []
        for i in range(count):
            raw, off, size = struct.unpack_from(ENTRY_FMT, data, HEADER_SIZE + ENTRY_SIZE * i)
            nm = raw.rstrip(b"\0")
            if b"\0" in nm or raw[len(nm):].strip(b"\0"):
                problems.append(f"entry {i}: name not NUL padded")
            try:
                entries.append((nm.decode("ascii"), off, size))
            except UnicodeDecodeError:
                problems.append(f"entry {i}: non-ASCII name")
        names = [e[0] for e in entries]
        if names != sorted(names, key=lambda n: n.encode("ascii")):
            problems.append("names not sorted")
        if len(set(names)) != len(names):
            problems.append("duplicate names")
        for name, off, size in entries:
            if size != BLOCK:
                problems.append(f"{name}: size {size} != {BLOCK}")
            if off < dir_end or off + size > len(data):
                problems.append(f"{name}: offset {off} out of range")
        if len(data) != dir_end + BLOCK * count:
            problems.append(f"file size {len(data)} != header + directory + {count} blocks")

    s = set(names)
    for p in ("xbox/", "ps/", "kbm/"):
        print(f"{p:8s} {sum(1 for n in names if n.startswith(p)):4d} entries")
    other = [n for n in names if not n.startswith(("xbox/", "ps/", "kbm/"))]
    if other:
        problems.append(f"unexpected prefixes: {other[:5]}")
    bases = [n for n in names if not n.endswith("_pressed")]
    print(f"base {len(bases)}, _pressed {len(names) - len(bases)}")
    for p in ("xbox/", "ps/", "kbm/"):
        missing = [n for n in bases if n.startswith(p) and n + "_pressed" not in s]
        orphan = [n for n in names if n.endswith("_pressed") and n[: -len("_pressed")] not in s]
        print(f"{p:8s} pressed twins missing: {len(missing)}, orphan pressed: {len(orphan)}")
        if p in ("xbox/", "ps/") and missing:
            problems.append(f"{p} base names without _pressed twin: {missing}")
        if orphan:
            problems.append(f"orphan _pressed entries: {orphan[:5]}")
    print("RESULT:", "PASS" if not problems else "FAIL")
    for pr in problems:
        print("  problem:", pr)
    return 0 if not problems else 1


def write_preview(path: Path, icons: dict[str, Image.Image], font_path: Path) -> None:
    CELL, LABEL_H, COLS, PAD, HEAD = 80, 16, 20, 12, 28
    bg = (96, 16, 16)
    font = ImageFont.truetype(str(font_path), 10)
    head_font = ImageFont.truetype(str(font_path), 18)
    groups = []
    for prefix in ("xbox/", "ps/", "kbm/"):
        base = sorted((n for n in icons if n.startswith(prefix) and not n.endswith("_pressed")),
                      key=lambda n: n.encode("ascii"))
        groups.append((f"{prefix} base", base))
        groups.append((f"{prefix} _pressed", [n + "_pressed" for n in base]))
    total_h = PAD
    for _, names in groups:
        rows = -(-len(names) // COLS)
        total_h += HEAD + rows * (CELL + LABEL_H + PAD)
    sheet = Image.new("RGB", (COLS * CELL + 2 * PAD, total_h), bg)
    draw = ImageDraw.Draw(sheet)
    y = PAD
    for title, names in groups:
        draw.text((PAD, y), title, font=head_font, fill=(255, 230, 200))
        y += HEAD
        for i, n in enumerate(names):
            x = PAD + (i % COLS) * CELL
            yy = y + (i // COLS) * (CELL + LABEL_H + PAD)
            big = icons[n].resize((ICON * 2, ICON * 2), Image.NEAREST)
            sheet.paste(big, (x + (CELL - ICON * 2) // 2, yy), big)
            draw.text((x + CELL // 2, yy + ICON * 2 + 2), n.split("/", 1)[1][:13],
                      font=font, fill=(240, 220, 200), anchor="mt")
        rows = -(-len(names) // COLS)
        y += rows * (CELL + LABEL_H + PAD)
    path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(path)


# ---------------------------------------------------------------- build

def build(args: argparse.Namespace) -> int:
    tmp_dirs: list[Path] = []
    try:
        root = open_kenney(args.kenney, tmp_dirs)
        font_path = Path(args.font)
        if not font_path.is_file():
            raise SystemExit(f"font not found: {font_path}")

        icons: dict[str, Image.Image] = {}
        for name, stem in XBOX_MAP.items():
            icons[name] = fit_icon(load_src(root, XBOX_DIR, stem))
        for name, stem in PS_MAP.items():
            icons[name] = fit_icon(load_src(root, PS_DIR, stem))
        for name, stem in OTHER_KBM_MAP.items():
            icons[name] = fit_icon(load_src(root, KBM_DIR, stem))
        add_dpad_directions(icons)

        cap = load_src(root, KBM_DIR, "keyboard")
        baked: list[tuple[str, str]] = []
        seen: set[str] = set()
        for sdl in SDL_SCANCODE_NAMES:
            lname = sdl.lower()
            if lname in seen:
                continue
            seen.add(lname)
            key = "kbm/" + lname
            if lname in KEY_IMAGE:
                icons[key] = fit_icon(load_src(root, KBM_DIR, KEY_IMAGE[lname]))
            else:
                label = label_for(lname, sdl)
                icons[key] = fit_icon(baked_key(cap, label, font_path))
                baked.append((sdl, label))

        for name in list(icons):
            icons[name + "_pressed"] = pressed_variant(icons[name])

        out = Path(args.out)
        size = write_pak(out, icons)

        base = [n for n in icons if not n.endswith("_pressed")]
        print(f"wrote {out} ({size} bytes), {len(icons)} entries ({len(base)} base + {len(base)} _pressed)")
        for p in ("xbox/", "ps/", "kbm/"):
            print(f"  {p:8s} {sum(1 for n in base if n.startswith(p))} base")
        print(f"baked text keys (no Kenney image), {len(baked)}:")
        print("  " + ", ".join(f"{sdl} -> {label}" for sdl, label in baked))

        if args.preview:
            write_preview(Path(args.preview), icons, font_path)
            print(f"wrote preview {args.preview}")
        return 0
    finally:
        for d in tmp_dirs:
            shutil.rmtree(d, ignore_errors=True)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Build the MPIP prompt icon pack from the Kenney Input Prompts pack.")
    ap.add_argument("--kenney", help="unzipped Kenney pack folder, or the pack .zip")
    ap.add_argument("--out", default="assets/prompts/prompt_icons.pak", help="output .pak path")
    ap.add_argument("--preview", help="also write a contact-sheet PNG of all entries here")
    ap.add_argument("--font", default=str(DEFAULT_FONT), help="TTF used for baked key labels and the preview")
    ap.add_argument("--verify", metavar="PAK", help="check an existing pak and exit")
    ap.add_argument("--derive", metavar="PAK", help="add the derived d-pad direction entries to an existing pak in place and exit")
    args = ap.parse_args(argv)
    if args.verify:
        return verify(Path(args.verify))
    if args.derive:
        return derive(Path(args.derive))
    if not args.kenney:
        ap.error("--kenney is required")
    return build(args)


if __name__ == "__main__":
    sys.exit(main())
