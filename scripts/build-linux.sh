#!/usr/bin/env bash
# build-linux.sh — cross-build the x86_64 Linux binary ON THE MAC (the powerful
# machine), so the weak Linux laptop never has to compile wgpu/glyphon itself.
# Uses Dockerfile.linux (Debian bookworm, glibc 2.36) via qemu emulation and
# drops the fresh binary into dist/awl, alongside the existing run.sh + samples.
#
#   scripts/build-linux.sh        # rebuild dist/awl
#
# Then copy the whole dist/ folder to the laptop and run ./run.sh there.
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

command -v docker >/dev/null 2>&1 || { echo "!! docker not found on PATH" >&2; exit 1; }

echo "==> cross-building x86_64 Linux binary (emulated; first build is slow)..."
DOCKER_BUILDKIT=1 docker build \
  --platform linux/amd64 \
  -f Dockerfile.linux \
  --target export \
  --output type=local,dest=dist \
  .

chmod +x dist/awl
echo "==> done: $(ls -lh dist/awl | awk '{print $5}')  $(file -b dist/awl | cut -d, -f1-2)"
echo "    next: copy dist/ to the laptop, then run ./run.sh there."
