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
# --reclaim / CI=true — GitHub mac-runner disk-exhaustion hardening:
#   After the workflow's two `cargo build --release --target ...` + `lipo`
#   steps, `target/<triple>/release/deps` (the per-arch object-file cache —
#   several GB, dead weight once the universal binary is lipo'd) is deleted
#   before assembling/DMGing. Gated so a LOCAL run never loses incremental
#   build state: fires only when `--reclaim` is passed explicitly OR the
#   ambient `CI=true` (GitHub Actions sets this natively on every job) — the
#   release workflow passes `--reclaim` explicitly too, belt-and-suspenders,
#   so the behavior doesn't depend on an inherited env var alone. Never
#   touches `target/release` (the ordinary, non-cross-compiled profile a dev
#   iterates on locally) and never fires in `--mas` mode (which builds
#   in-place, no per-arch split to reclaim).
#
# DMG staging + hdiutil TMPDIR — the other half of the same hardening: DMG
# staging now happens in a directory UNDER the caller's own <output-dir>
# (which the release workflow points at a workspace-relative path,
# `dist-mac`) instead of the system `mktemp -d`/$TMPDIR default — on GitHub
# mac runners the system temp volume has been observed nearly full while the
# workspace volume has room. `TMPDIR` is exported (workspace-local) around
# the `hdiutil create` invocation specifically, since that's the documented
# lever hdiutil honors for its own scratch/temp work (there is no `-tmpdir`
# flag); the explicit `cp -R`-populated staging dir was already a `mktemp -d`
# consumer of the same default, so both move together.
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
  export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
fi

MAS=0
RECLAIM=0
POSITIONAL=()
for arg in "$@"; do
  case "$arg" in
    --mas) MAS=1 ;;
    --reclaim) RECLAIM=1 ;;
    -h|--help)
      sed -n '2,68p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) POSITIONAL+=("$arg") ;;
  esac
done
[ "${CI:-}" = "true" ] && RECLAIM=1

if [[ "$MAS" -eq 1 ]]; then
  command -v cargo >/dev/null 2>&1 || { echo "!! cargo not found on PATH" >&2; exit 1; }
  echo "==> building MAS (--features mas) release binary..."
  # with-remap.sh strips the builder's $HOME/registry paths from this shipped
  # release binary (rustc bakes compile-time source locations otherwise).
  (cd "$ROOT" && "$ROOT/scripts/with-remap.sh" cargo build --release --features mas)
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

# --- Reclaim headroom (CI only — see the module doc's "--reclaim" note) ----
# By the time we're assembling, `lipo` has already merged the two per-arch
# release binaries into the universal $BIN_PATH; each `target/<triple>/
# release/deps` directory (every dependency crate's compiled object files)
# is dead weight from here on, and on a GitHub mac runner it's several GB of
# headroom we want back before hdiutil ever runs.
if [[ "$MAS" -eq 0 && "$RECLAIM" -eq 1 ]]; then
  for triple in aarch64-apple-darwin x86_64-apple-darwin; do
    DEPS="$ROOT/target/$triple/release/deps"
    if [ -d "$DEPS" ]; then
      SIZE_BEFORE="$(du -sh "$DEPS" 2>/dev/null | cut -f1)"
      echo "==> reclaim: removing $DEPS ($SIZE_BEFORE — dead post-lipo build artifacts)"
      rm -rf "$DEPS"
    fi
  done
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
# Stage BOTH the DMG source-folder copy AND hdiutil's own scratch/temp work
# under $OUT_DIR — a directory the caller controls (in CI, a workspace-
# relative path, e.g. `dist-mac`) — instead of the system `mktemp -d`/
# $TMPDIR default. `TMPDIR` is the documented lever for redirecting hdiutil's
# OWN scratch work (there is no `-tmpdir` flag), so it's exported
# workspace-local only around the `hdiutil create` call below. Cheap
# precautionary hardening either way, kept even though it turned out NOT to
# be this round's actual root cause (see below) — it can only help.
DMG_WORK="$OUT_DIR/.dmg-work"
rm -rf "$DMG_WORK"
mkdir -p "$DMG_WORK/staging" "$DMG_WORK/tmp"
trap 'rm -rf "$DMG_WORK"' EXIT
DMG_STAGING="$DMG_WORK/staging"
cp -R "$APP" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"
# Same licensing docs, also visible at the DMG's top level (alongside the
# .app + the Applications shortcut) — the common "read this before you drag
# it over" placement, redundant with the copies already inside the bundle.
for doc in LICENSE NOTICE CREDITS.md THIRD-PARTY-LICENSES.md; do
  [ -f "$ROOT/$doc" ] && cp "$ROOT/$doc" "$DMG_STAGING/$doc"
done

echo "==> disk space before hdiutil (self-diagnostic for the 'No space left on device' failure class):"
df -h

# THE ACTUAL ROOT CAUSE (found live, this round): the two prior CI failures
# were NOT genuine disk exhaustion — a `df -h` added right before this exact
# hdiutil call on the failing runner showed 96 GiB free on EVERY volume, and
# the failure text ("could not access .../MacOS/awl - No space left on
# device") names the bundled BINARY specifically. `hdiutil create -srcfolder`
# (no `-size`) auto-estimates its scratch image's size and can undersize it
# in ways that have nothing to do with real host disk pressure — reproduced
# LOCALLY with a synthetic sparse file (`truncate -s 500m` over a 4 KiB real
# payload: tiny on-disk allocation, huge apparent size), which hit the exact
# same failure text on a machine with hundreds of GB free (a `lipo`-produced
# universal binary CAN be sparse on APFS from inter-slice alignment padding —
# the originally-suspected trigger; CHECKED against this app's OWN real
# `lipo -create` output, though, and it was NOT measurably sparse — apparent
# size and on-disk allocation agreed to within one block). So the exact
# trigger inside GitHub's virtualized macOS runner is unconfirmed, but the
# SYMPTOM (auto-size undershoot despite ample real free space) is real and
# reproducible, and matches the standard community workaround reported
# across multiple `actions/runner-images` hdiutil issues: pass an explicit
# `-size` and sidestep hdiutil's own estimate entirely, whatever throws it
# off. Sized off the staging folder's APPARENT bytes (`stat -f%z`, NOT `du`,
# which would repeat the same kind of undercount if the source ever IS
# sparse) with a generous 2x + 64 MiB margin — cheap insurance, since the
# image is compressed down to real content size in the FINAL (UDZO) output
# regardless; the margin only costs a briefly-larger temp scratch file.
APPARENT_BYTES="$(find "$DMG_STAGING" -type f -exec stat -f%z {} + | awk '{sum+=$1} END{print sum+0}')"
DMG_SIZE_MB=$(( (APPARENT_BYTES * 2 / 1024 / 1024) + 64 ))
echo "==> sizing DMG scratch image: ${DMG_SIZE_MB}m (from ${APPARENT_BYTES} apparent bytes staged)"

TMPDIR="$DMG_WORK/tmp" hdiutil create -volname "Awl" -srcfolder "$DMG_STAGING" -size "${DMG_SIZE_MB}m" -ov -format UDZO "$OUT_DIR/Awl.dmg"
echo "==> $OUT_DIR/Awl.dmg created"
