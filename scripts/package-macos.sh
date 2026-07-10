#!/usr/bin/env bash
#
# package-macos.sh — assemble Awl.app from an already-built universal
# `awl` binary. Does NOT build or lipo the binary itself (the release
# workflow does that, so it can build aarch64 + x86_64 separately and cache
# each target); this script only assembles the .app bundle shape + Info.plist
# and, optionally, a DMG.
#
# Usage:
#   scripts/package-macos.sh <path-to-universal-binary> <output-dir>
#
# Produces:
#   <output-dir>/Awl.app/Contents/{MacOS/awl, Info.plist, Resources/}
#   <output-dir>/Awl.dmg          (only if `hdiutil` succeeds — see below)
#
# Env overrides (all optional):
#   AWL_BUNDLE_ID      reverse-DNS bundle identifier (default below)
#   AWL_VERSION        CFBundleShortVersionString (default: Cargo.toml's
#                       package.version, read via `cargo metadata` if cargo
#                       is on PATH, else "0.0.0")
#   AWL_SKIP_DMG=1      skip DMG creation (bundle-only)
#
# This script never signs or notarizes anything — that's the release
# workflow's job (gated on secrets), kept separate so this script stays
# runnable standalone with no Apple developer account at all.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BIN_PATH="${1:?usage: package-macos.sh <path-to-universal-binary> <output-dir>}"
OUT_DIR="${2:?usage: package-macos.sh <path-to-universal-binary> <output-dir>}"

if [ ! -f "$BIN_PATH" ]; then
  echo "error: binary not found at $BIN_PATH" >&2
  exit 1
fi

# reverse-DNS identifier — a placeholder namespace, USER-CHANGEABLE. There is
# no registered organization domain for awl yet; this is a reasonable-looking
# default (dev.<author>.awl) that the user should replace with their own once
# they pick a real domain / Apple Developer Team identity. See RELEASING.md.
AWL_BUNDLE_ID="${AWL_BUNDLE_ID:-dev.franklu.awl}"

AWL_VERSION="${AWL_VERSION:-}"
if [ -z "$AWL_VERSION" ]; then
  if command -v cargo >/dev/null 2>&1; then
    AWL_VERSION="$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 2>/dev/null \
      | grep -o '"version":"[^"]*"' | head -1 | cut -d'"' -f4)"
  fi
  AWL_VERSION="${AWL_VERSION:-0.0.0}"
fi

APP="$OUT_DIR/Awl.app"
CONTENTS="$APP/Contents"
echo "==> assembling $APP  (version $AWL_VERSION, bundle id $AWL_BUNDLE_ID)"

rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"

cp "$BIN_PATH" "$CONTENTS/MacOS/awl"
chmod +x "$CONTENTS/MacOS/awl"

# ICON: a placeholder path, wired but commented out below — the user's icon
# (Awl.icns) is in progress. Drop it at assets/macos/Awl.icns and uncomment
# the two lines to wire it in; the bundle builds and runs fine without one
# (macOS just shows the generic app icon).
ICON_SRC="$ROOT/assets/macos/Awl.icns"
# if [ -f "$ICON_SRC" ]; then
#   cp "$ICON_SRC" "$CONTENTS/Resources/Awl.icns"
# fi

cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Awl</string>
  <key>CFBundleDisplayName</key>
  <string>Awl</string>
  <key>CFBundleExecutable</key>
  <string>awl</string>
  <key>CFBundleIdentifier</key>
  <string>${AWL_BUNDLE_ID}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${AWL_VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${AWL_VERSION}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSHumanReadableCopyright</key>
  <string>GPL-3.0-only</string>
PLIST

if [ -f "$ICON_SRC" ]; then
  cat >> "$CONTENTS/Info.plist" <<PLIST
  <key>CFBundleIconFile</key>
  <string>Awl.icns</string>
PLIST
fi

cat >> "$CONTENTS/Info.plist" <<PLIST
</dict>
</plist>
PLIST

echo "==> Awl.app assembled"

if [ "${AWL_SKIP_DMG:-0}" = "1" ]; then
  exit 0
fi

echo "==> creating Awl.dmg"
DMG_STAGING="$(mktemp -d)"
trap 'rm -rf "$DMG_STAGING"' EXIT
cp -R "$APP" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"
hdiutil create -volname "Awl" -srcfolder "$DMG_STAGING" -ov -format UDZO "$OUT_DIR/Awl.dmg"
echo "==> $OUT_DIR/Awl.dmg created"
