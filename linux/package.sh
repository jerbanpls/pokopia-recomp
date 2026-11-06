#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds the Linux release download: linux/build/dist/PokopiaRecomp-<version>.tar.gz and its .sha256.
#   linux/package.sh
# 1. Both launchers, the native one and the AppImage, in the Ubuntu 22.04 container of linux/Containerfile (glibc 2.35).
# 2. The official Pokopia Recomp release zip (UPSTREAM_VERSION), checked against its pinned SHA-256: its runtime, tools and
#    assets, without what Linux does not use (that release's own launcher and start script, its bundled Python and Zig: Set up
#    fetches the Linux ones).
# 3. This repository's pkp_build.py, the launchers, install-shortcut.sh, README.txt and CHANGELOG.md.
# Nothing from a game dump is involved; a final check refuses a package that holds game-made files.
# Needs podman, curl, python3, tar, sha256sum.
set -euo pipefail

UPSTREAM_VERSION=0.6.0
UPSTREAM_ZIP=PokopiaRecomp-$UPSTREAM_VERSION-win64.zip
UPSTREAM_SHA256=b0dc82c9a2b7a7669eff9bb0e271d2f9d2fa287d3b12831c72d05680fb4def2d
UPSTREAM_URL=https://github.com/featureJosh/pokopia-recomp/releases/download/v$UPSTREAM_VERSION/$UPSTREAM_ZIP

ROOT=$(cd "$(dirname "$0")/.." && pwd)
VERSION=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' "$ROOT/launcher/src-tauri/tauri.conf.json")
OUT=$ROOT/linux/build
CACHE=$OUT/cache
STAGE=$OUT/stage
DIST=$OUT/dist
IMAGE=pokopia-linux-build
TARGET=launcher/src-tauri/target/ubuntu       # its own target folder, apart from builds with the system's toolchain
mkdir -p "$CACHE/cargo-registry" "$DIST"

echo "== 1/4 launchers $VERSION (Ubuntu 22.04 container)"
podman build -q -t "$IMAGE" -f "$ROOT/linux/Containerfile" "$ROOT/linux" >/dev/null
rm -rf "${ROOT:?}/$TARGET/release/bundle"
# APPIMAGE_EXTRACT_AND_RUN: the AppImage tools run without FUSE in the container; NO_STRIP: linuxdeploy's strip fails on newer ELF files
podman run --rm --security-opt label=disable \
    -v "$ROOT:/src" -v "$CACHE/cargo-registry:/usr/local/cargo/registry" \
    -e CARGO_TARGET_DIR="/src/$TARGET" -e APPIMAGE_EXTRACT_AND_RUN=1 -e NO_STRIP=true -w /src/launcher "$IMAGE" \
    sh -c 'npm ci --no-audit --no-fund >/dev/null && npm run tauri build -- --config "{\"bundle\":{\"active\":true,\"targets\":[\"appimage\"]}}"'
BIN=$ROOT/$TARGET/release/pokopia-launcher
APPIMAGE=$(find "$ROOT/$TARGET/release/bundle/appimage" -maxdepth 1 -name '*.AppImage' | head -1)
[ -x "$BIN" ] && [ -n "$APPIMAGE" ] || { echo "the launcher build made no binary or AppImage" >&2; exit 1; }

echo "== 2/4 Pokopia Recomp $UPSTREAM_VERSION release files"
ZIP=$CACHE/$UPSTREAM_ZIP
if ! echo "$UPSTREAM_SHA256  $ZIP" | sha256sum -c --status 2>/dev/null; then
    curl -fL --retry 3 -o "$ZIP.part" "$UPSTREAM_URL"
    echo "$UPSTREAM_SHA256  $ZIP.part" | sha256sum -c -
    mv "$ZIP.part" "$ZIP"
fi
rm -rf "${STAGE:?}"
mkdir -p "$STAGE"
python3 -I -m zipfile -e "$ZIP" "$STAGE"
TOP=$(find "$STAGE" -mindepth 1 -maxdepth 1)
[ "$(printf '%s\n' "$TOP" | wc -l)" = 1 ] && [ -d "$TOP" ] || { echo "the release zip does not hold one folder" >&2; exit 1; }
PKG=$STAGE/PokopiaRecomp
mv "$TOP" "$PKG"
rm -rf "$PKG/PokopiaLauncher.exe" "$PKG/Play.bat" "$PKG/README.txt" "$PKG/CHANGELOG.txt" "$PKG/tools/python" "$PKG/tools/build/zig"

echo "== 3/4 the Linux edition's files"
install -m 755 "$BIN" "$PKG/PokopiaLauncher"
install -m 755 "$APPIMAGE" "$PKG/PokopiaLauncher-x86_64.AppImage"
install -m 755 "$ROOT/linux/package/install-shortcut.sh" "$PKG/install-shortcut.sh"
install -m 644 "$ROOT/launcher/src-tauri/icons/128x128@2x.png" "$PKG/assets/pokopia-launcher.png"
install -m 644 "$ROOT/game-build/release_b/pkp_build.py" "$PKG/tools/build/pkp_build.py"
install -m 644 "$ROOT/CHANGELOG.md" "$PKG/CHANGELOG.md"
sed "s/@VERSION@/$VERSION/g; s/@UPSTREAM@/$UPSTREAM_VERSION/g" "$ROOT/linux/package/README.txt" > "$PKG/README.txt"
# no game-made or player-made file, and nothing that may not be passed on (NVIDIA's DLSS file is fetched per player)
bad=$(cd "$PKG" && find . \( -name pkp.exe -o -name shader_cache -o -name save -o -name nup_dump -o -name 'pkp.ini' -o -name 'controls.ini' \
    -o -name '*.log' -o -name 'pipeline_*.bin' -o -name nvngx_dlss.dll -o -path ./build -o -path ./tools/linux \) -print)
[ -z "$bad" ] || { echo "refusing to package: $bad" >&2; exit 1; }

echo "== 4/4 archive"
NAME=PokopiaRecomp-$VERSION
tar -C "$STAGE" --owner=0 --group=0 --numeric-owner -czf "$DIST/$NAME.tar.gz" PokopiaRecomp
(cd "$DIST" && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
echo "$(du -h "$DIST/$NAME.tar.gz" | cut -f1)  $DIST/$NAME.tar.gz"
cat "$DIST/$NAME.tar.gz.sha256"
