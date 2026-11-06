prompt_icons.pak: 556 RGBA 36x36 controller, keyboard and mouse prompt icons (xbox/, ps/, kbm/; each base icon also has a gold _pressed twin), built from the Kenney Input Prompts 1.5a pack (CC0, see LICENSE-Kenney-InputPrompts.txt).
Format: "MPIP" header, 48-byte directory entries (name, offset, size), then raw RGBA8 pixel blocks; entries sorted by name.
Rebuild: python -I scripts/build_prompt_pack.py --kenney <Kenney pack folder or zip> --out assets/prompts/prompt_icons.pak [--preview <png>]
Check: python -I scripts/build_prompt_pack.py --verify assets/prompts/prompt_icons.pak
Keys without a Kenney image (keypad digits, F13-F24, media keys, etc.) are drawn as text on the blank key cap ("keyboard" image) with DejaVu Sans.
The d-pad direction icons (xbox/dpad_up ... ps/dpad_right) are drawn from the whole d-pad (one arm kept, the rest dimmed); --derive adds them to an existing pack.
