#!/usr/bin/env bash
# run-linux.sh — one-shot bootstrap to build & run awl-next on Linux.
# Installs the system libs winit/wgpu/cosmic-text need, ensures Rust, then runs.
#
#   ./run-linux.sh                 # open samples/welcome.md
#   ./run-linux.sh path/to/file.md # open a specific file
#   ./run-linux.sh --release [f]   # optimized build (slower first compile, smoother)
#   SKIP_DEPS=1 ./run-linux.sh     # skip the system-package install step
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

PROFILE=()
if [[ "${1:-}" == "--release" ]]; then PROFILE=(--release); shift; fi
FILE="${1:-samples/welcome.md}"

install_deps() {
  [[ "${SKIP_DEPS:-0}" == "1" ]] && { echo "SKIP_DEPS=1 -> skipping system packages"; return; }
  echo "==> installing system dependencies (uses sudo)..."
  if command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq
    sudo apt-get install -y --no-install-recommends \
      build-essential pkg-config curl ca-certificates \
      libfontconfig1-dev libxkbcommon-dev libwayland-dev \
      libx11-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
      libvulkan1 mesa-vulkan-drivers vulkan-tools
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y \
      gcc gcc-c++ make pkgconf-pkg-config curl \
      fontconfig-devel libxkbcommon-devel wayland-devel libX11-devel libxcb-devel \
      vulkan-loader mesa-vulkan-drivers vulkan-tools
  elif command -v pacman >/dev/null 2>&1; then
    sudo pacman -S --needed --noconfirm \
      base-devel pkgconf curl \
      fontconfig libxkbcommon wayland libx11 libxcb vulkan-icd-loader mesa vulkan-tools
  elif command -v zypper >/dev/null 2>&1; then
    sudo zypper install -y \
      gcc gcc-c++ make pkg-config curl \
      fontconfig-devel libxkbcommon-devel wayland-devel libX11-devel libxcb-devel \
      vulkan-loader Mesa-vulkan-device-driver vulkan-tools
  else
    echo "!! Unknown package manager. Install manually: a C toolchain, pkg-config," >&2
    echo "   fontconfig, libxkbcommon, wayland, X11/xcb dev libs, the Vulkan loader," >&2
    echo "   and a Mesa Vulkan driver, then re-run with SKIP_DEPS=1." >&2
  fi
}

ensure_rust() {
  command -v cargo >/dev/null 2>&1 || { [[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"; }
  if ! command -v cargo >/dev/null 2>&1; then
    echo "==> installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
  fi
  echo "==> $(cargo --version)"
}

install_deps
ensure_rust

echo "==> building + launching awl (${PROFILE[*]:-debug}) on: $FILE"
echo "    first build compiles wgpu/glyphon (a few minutes); later runs are instant."
exec cargo run "${PROFILE[@]}" -- "$FILE"
