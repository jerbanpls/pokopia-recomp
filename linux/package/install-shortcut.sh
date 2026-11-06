#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Adds "Pokopia Launcher" to the app menu and the desktop, for the launcher in this folder. Run it again after moving the folder.
#   ./install-shortcut.sh            add (or update) the shortcuts
#   ./install-shortcut.sh --remove   remove them
set -eu
DIR=$(cd "$(dirname "$0")" && pwd)
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
APPS=$DATA/applications
ICONS=$DATA/icons/hicolor/256x256/apps
DESKTOP=$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")
ENTRY=org.pkp-recomp.launcher.desktop

if [ "${1:-}" = --remove ]; then
    rm -f "$APPS/$ENTRY" "$DESKTOP/$ENTRY" "$ICONS/pokopia-launcher.png"
    echo "Removed the Pokopia Launcher shortcuts."
    exit 0
fi

case "$DIR" in
    *[\"\`\$\\%]*) echo "This folder's path holds a character a shortcut cannot carry (\" \` \$ \\ %): move the folder and run this again." >&2; exit 1 ;;
esac

# the small native launcher when this system has what it needs (WebKitGTK 4.1, glibc 2.35+), else the AppImage
LAUNCHER=$DIR/PokopiaLauncher
if ! ldd "$LAUNCHER" >/dev/null 2>&1 || ldd "$LAUNCHER" 2>&1 | grep -q 'not found'; then
    LAUNCHER=$DIR/PokopiaLauncher-x86_64.AppImage
fi

mkdir -p "$APPS" "$ICONS"
cp "$DIR/assets/pokopia-launcher.png" "$ICONS/pokopia-launcher.png"
cat > "$APPS/$ENTRY" <<EOF
[Desktop Entry]
Type=Application
Name=Pokopia Launcher
Comment=Set up and play Pokopia Recomp
Exec="$LAUNCHER"
Path=$DIR
Icon=pokopia-launcher
Terminal=false
Categories=Game;
StartupWMClass=org.pkp-recomp.launcher
Actions=play;

[Desktop Action play]
Name=Play Pokopia
Exec="$LAUNCHER" --play
EOF
chmod +x "$APPS/$ENTRY"
where="the app menu"
if [ -d "$DESKTOP" ]; then
    cp "$APPS/$ENTRY" "$DESKTOP/$ENTRY"
    chmod +x "$DESKTOP/$ENTRY"
    where="the app menu and the desktop"
fi
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
echo "Added \"Pokopia Launcher\" to $where (it starts $(basename "$LAUNCHER"))."
