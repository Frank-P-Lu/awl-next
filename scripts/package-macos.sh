#!/usr/bin/env bash
#
# package-macos.sh — assemble Awl.app, either the ORDINARY flavor (from an
# already-built universal `awl` binary — the release workflow's job, so it
# can build aarch64 + x86_64 separately and cache each target) or the MAS
# (Mac App Store / App Sandbox) flavor (`--mas`, which builds the
# `--features mas` binary itself and bundles the sandbox entitlements
# alongside for a human to sign against). One script, two modes — folded
# together deliberately: a standalone MAS packaging script briefly existed
# on a concurrent worktree before this merge; its `--mas` behavior now lives
# here instead of as a second parallel script.
#
# Usage:
#   scripts/package-macos.sh <path-to-universal-binary> <output-dir>
#     Ordinary flavor: assemble (does NOT build or lipo the binary itself).
#
#   scripts/package-macos.sh --mas [output-dir]
#     MAS flavor: builds `cargo build --release --features mas` itself
#     (output-dir defaults to `dist`) and assembles Awl.app with
#     `packaging/mas/entitlements.plist` copied into Resources/.
#
# Produces (ordinary):
#   <output-dir>/Awl.app/Contents/{MacOS/awl, Info.plist, Resources/}
#   <output-dir>/Awl.dmg          (only if `hdiutil` succeeds — see below)
#
# Produces (--mas):
#   <output-dir>/Awl.app/Contents/{MacOS/awl, Info.plist, Resources/}
#   <output-dir>/Awl.app/Contents/Resources/entitlements.plist
#   No DMG (Mac App Store submissions never ship a DMG).
#
# Env overrides (all optional):
#   AWL_BUNDLE_ID      reverse-DNS bundle identifier (default below)
#   AWL_VERSION        CFBundleShortVersionString (default: Cargo.toml's
#                       package.version, read via `cargo metadata` if cargo
#                       is on PATH, else "0.0.0")
#   AWL_SKIP_DMG=1      skip DMG creation (bundle-only; --mas never makes one
#                       regardless of this flag)
#
# SIGNING IS DELIBERATELY OUT OF SCOPE in both modes: this script never calls
# `codesign` — it only PRINTS the command a human should run next (with their
# own Developer ID / Apple Development / Apple Distribution identity),
# mirroring the ordinary flavor's own release workflow (gated on secrets,
# kept separate so this script stays runnable standalone with no Apple
# developer account at all).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Pin this Mac's toolchain so cargo is findable regardless of cwd / a bare
# shell (only matters for --mas, which invokes cargo itself).
if ! command -v cargo >/dev/null 2>&1; then
  export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
fi

MAS=0
POSITIONAL=()
for arg in "$@"; do
  case "$arg" in
    --mas) MAS=1 ;;
    -h|--help)
      sed -n '2,42p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) POSITIONAL+=("$arg") ;;
  esac
done

if [[ "$MAS" -eq 1 ]]; then
  command -v cargo >/dev/null 2>&1 || { echo "!! cargo not found on PATH" >&2; exit 1; }
  echo "==> building MAS (--features mas) release binary..."
  (cd "$ROOT" && cargo build --release --features mas)
  BIN_PATH="$ROOT/target/release/awl"
  OUT_DIR="${POSITIONAL[0]:-$ROOT/dist}"
else
  BIN_PATH="${POSITIONAL[0]:?usage: package-macos.sh <path-to-universal-binary> <output-dir> (or --mas [output-dir])}"
  OUT_DIR="${POSITIONAL[1]:?usage: package-macos.sh <path-to-universal-binary> <output-dir> (or --mas [output-dir])}"
fi

if [ ! -f "$BIN_PATH" ]; then
  echo "error: binary not found at $BIN_PATH" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

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
echo "==> assembling $APP  (version $AWL_VERSION, bundle id $AWL_BUNDLE_ID)$([ "$MAS" -eq 1 ] && echo '  [MAS]')"

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

# LICENSING: LICENSE (GPL-3.0 full text), CREDITS.md (the human-readable
# thank-you), and THIRD-PARTY-LICENSES.md (the generated crate inventory)
# ride into Contents/Resources/ — the standard macOS bundle home for
# license-adjacent docs (see also Apple's own apps' Resources/ folders).
# Missing files are a loud warning, never a hard failure (this script must
# stay runnable standalone against an older checkout too).
for doc in LICENSE NOTICE CREDITS.md THIRD-PARTY-LICENSES.md; do
  if [ -f "$ROOT/$doc" ]; then
    cp "$ROOT/$doc" "$CONTENTS/Resources/$doc"
  else
    echo "warning: $ROOT/$doc not found — skipping (bundle built without it)" >&2
  fi
done

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

if [[ "$MAS" -eq 1 ]]; then
  # MAS SANDBOX ENTITLEMENTS: copied into the bundle's own Resources/ for
  # reference, and named again below so a human knows exactly what to
  # `codesign --entitlements` with. Never signed here (see the module doc).
  cp "$ROOT/packaging/mas/entitlements.plist" "$CONTENTS/Resources/entitlements.plist"
  echo "==> done: $APP (unsigned, MAS entitlements at packaging/mas/entitlements.plist)"
  echo "    next (out of scope here): codesign --entitlements packaging/mas/entitlements.plist \\"
  echo "         --sign <your Apple Distribution identity> --deep --force \"$APP\""
  # Mac App Store submissions never ship a DMG (App Store Connect / Transporter
  # takes the signed .app / .pkg directly) — exit before the DMG step below.
  exit 0
fi

if [ "${AWL_SKIP_DMG:-0}" = "1" ]; then
  exit 0
fi

echo "==> creating Awl.dmg"
DMG_STAGING="$(mktemp -d)"
trap 'rm -rf "$DMG_STAGING"' EXIT
cp -R "$APP" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"
# Same licensing docs, also visible at the DMG's top level (alongside the
# .app + the Applications shortcut) — the common "read this before you drag
# it over" placement, redundant with the copies already inside the bundle.
for doc in LICENSE NOTICE CREDITS.md THIRD-PARTY-LICENSES.md; do
  [ -f "$ROOT/$doc" ] && cp "$ROOT/$doc" "$DMG_STAGING/$doc"
done
hdiutil create -volname "Awl" -srcfolder "$DMG_STAGING" -ov -format UDZO "$OUT_DIR/Awl.dmg"
echo "==> $OUT_DIR/Awl.dmg created"
