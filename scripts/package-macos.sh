#!/usr/bin/env bash
#
# package-macos.sh — build the native macOS release binary, either the
# ORDINARY flavor (default) or the MAS (Mac App Store / App Sandbox) flavor
# (`--mas`), and drop it into dist/.
#
# STANDALONE NOTE: at the time this script was written, no RELEASING.md /
# scripts/package-macos.sh existed yet on this worktree's base (a concurrent
# infra round may still be landing a fuller packaging pipeline — icon,
# codesigning, notarization, DMG, the works). This script is deliberately
# SCOPED to what the MAS-flavor round actually needs to hand a verifier: a
# built `--features mas` binary, wrapped in a minimal-but-real `.app` bundle
# (so entitlements/Info.plist have somewhere to live), with the entitlements
# plist alongside for a human to `codesign --entitlements` with their own
# Developer ID / Apple Development identity. If a fuller RELEASING.md /
# packaging script lands later, this file's `--mas` mode should fold into it
# rather than staying a second parallel script.
#
# Usage:
#   scripts/package-macos.sh          # ordinary release build -> dist/awl
#   scripts/package-macos.sh --mas    # `--features mas` build -> dist/Awl.app
#                                      #   (unsigned; entitlements alongside)
#
# SIGNING IS DELIBERATELY OUT OF SCOPE (per this round's brief): this script
# never calls `codesign`. Dev/ad-hoc signing for local verification, and real
# Developer-ID/App-Store distribution signing + notarization, are a future
# submission round's job — this script only gets you to an unsigned .app a
# human can then run `codesign --entitlements packaging/mas/entitlements.plist
# --sign <identity> --deep dist/Awl.app` against.
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Pin this Mac's toolchain so cargo is findable regardless of cwd / a bare shell.
if ! command -v cargo >/dev/null 2>&1; then
  export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || { echo "!! cargo not found on PATH" >&2; exit 1; }

MAS=0
for arg in "$@"; do
  case "$arg" in
    --mas) MAS=1 ;;
    -h|--help)
      sed -n '2,26p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "!! unknown argument: $arg (see --help)" >&2; exit 1 ;;
  esac
done

mkdir -p dist

if [[ "$MAS" -eq 0 ]]; then
  echo "==> building ordinary release binary..."
  cargo build --release
  cp target/release/awl dist/awl
  chmod +x dist/awl
  echo "==> done: dist/awl ($(ls -lh dist/awl | awk '{print $5}'))"
  exit 0
fi

echo "==> building MAS (--features mas) release binary..."
cargo build --release --features mas

APP="dist/Awl.app"
CONTENTS="$APP/Contents"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"

cp target/release/awl "$CONTENTS/MacOS/awl"
chmod +x "$CONTENTS/MacOS/awl"

# A minimal Info.plist — just enough for the bundle to be a real, launchable
# .app (CFBundleExecutable lowercase so the CLI binary name stays `awl`, per
# CLAUDE.md's menu-bar-title finding: only a real bundle gets AppKit to stop
# forcibly substituting the App-menu title with the raw process name). The
# icon (CFBundleIconFile) is a PLACEHOLDER PATH ONLY — no .icns shipped yet,
# out of scope per this round's brief (see the module doc above).
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
  <string>com.awl.editor</string>
  <key>CFBundleVersion</key>
  <string>$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')</string>
  <key>CFBundleShortVersionString</key>
  <string>$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <!-- PLACEHOLDER — no .icns bundled yet; out of scope this round. -->
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

cp packaging/mas/entitlements.plist "$CONTENTS/Resources/entitlements.plist"

echo "==> done: $APP (unsigned)"
echo "    entitlements: packaging/mas/entitlements.plist (also copied into the bundle's Resources/)"
echo "    next (out of scope here): codesign --entitlements packaging/mas/entitlements.plist \\"
echo "         --sign <your identity> --deep --force \"$APP\""
