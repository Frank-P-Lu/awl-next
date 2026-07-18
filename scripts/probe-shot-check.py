#!/usr/bin/env python3
"""probe-shot-check.py — assert a LIVE probe shot matches its HEADLESS reference.

The live-probe harness (`scripts/live-probe.sh` + `awl --live-script`) captures
what the REAL app actually put on screen (window-server image, or the presented
frame mirror). The offscreen `--screenshot` capture of the same state is proven
correct by the law suite — so it serves as the EXPECTED image. This checker
compares the two with block arithmetic (CLAUDE.md: appearance is asserted over
pixels, never inferred from state):

  * The live shot may be at a higher DPI scale (2x retina) and may include OS
    window chrome above the surface; blocks are mapped bottom-anchored through
    the width ratio, so both backends and both scales check identically.
  * The reference is cut into fixed blocks; only blocks that are NEAR-UNIFORM
    in the reference (grounds, page surface, bars, placards) are asserted —
    text antialiasing legitimately differs between 1x and downscaled 2x, but a
    vanished/stale/blank surface floods the uniform blocks with huge diffs.
  * `--region page` restricts the sweep to the page column (x >= 20% of the
    frame) for shots where an AMBIENT world's margins legitimately animate
    live (lava lamp / twinkling stars) while the headless phase is frozen.

Exit 0 = every asserted block within tolerance (PASS printed with numbers);
exit 1 = defect (worst offenders printed with coordinates + expected/actual);
exit 2 = usage/decoding error. Pure stdlib (zlib PNG decode) — no PIL/numpy.
"""

import sys
import struct
import zlib


def decode_png(path):
    """Minimal PNG decoder: 8-bit RGB/RGBA, non-interlaced. -> (w, h, [rows of RGB tuples])."""
    raw = open(path, "rb").read()
    if raw[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path}: not a PNG")
    pos, w, h, depth, ctype, idat = 8, 0, 0, 0, 0, []
    while pos < len(raw):
        (length,) = struct.unpack(">I", raw[pos : pos + 4])
        tag = raw[pos + 4 : pos + 8]
        data = raw[pos + 8 : pos + 8 + length]
        if tag == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", data)
            if depth != 8 or ctype not in (2, 6) or interlace != 0:
                raise ValueError(f"{path}: unsupported PNG (depth={depth} ctype={ctype})")
        elif tag == b"IDAT":
            idat.append(data)
        elif tag == b"IEND":
            break
        pos += 12 + length
    bpp = 4 if ctype == 6 else 3
    stream = zlib.decompress(b"".join(idat))
    stride = w * bpp
    rows, prev = [], bytearray(stride)
    p = 0
    for _ in range(h):
        filt = stream[p]
        line = bytearray(stream[p + 1 : p + 1 + stride])
        p += 1 + stride
        if filt == 1:  # Sub
            for i in range(bpp, stride):
                line[i] = (line[i] + line[i - bpp]) & 0xFF
        elif filt == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif filt == 3:  # Average
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 0xFF
        elif filt == 4:  # Paeth
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                pp = a + b - c
                pa, pb, pc = abs(pp - a), abs(pp - b), abs(pp - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 0xFF
        elif filt != 0:
            raise ValueError(f"{path}: bad filter {filt}")
        rows.append(bytes(line))
        prev = line
    return w, h, bpp, rows


def block_mean(rows, bpp, x0, y0, x1, y1, step=1):
    r = g = b = n = 0
    for y in range(y0, y1, step):
        row = rows[y]
        for x in range(x0, x1, step):
            o = x * bpp
            r += row[o]
            g += row[o + 1]
            b += row[o + 2]
            n += 1
    return (r / n, g / n, b / n) if n else (0.0, 0.0, 0.0)


def block_mad(rows, bpp, x0, y0, x1, y1, mean):
    """Mean absolute deviation from `mean` (max over channels)."""
    dr = dg = db = n = 0
    for y in range(y0, y1):
        row = rows[y]
        for x in range(x0, x1):
            o = x * bpp
            dr += abs(row[o] - mean[0])
            dg += abs(row[o + 1] - mean[1])
            db += abs(row[o + 2] - mean[2])
            n += 1
    return max(dr, dg, db) / n if n else 0.0


def main():
    args = sys.argv[1:]
    opts = {"--tol": 30.0, "--uniform": 6.0, "--block": 40, "--region": "all"}
    coarse = False
    paths = []
    i = 0
    while i < len(args):
        if args[i] == "--coarse":
            # MID-TRANSITION shots (e.g. mid font-debounce: destination colors
            # applied, source font metrics still shaping) legitimately differ
            # from the settled reference in layout detail — assert only the
            # REGION-WIDE mean color, which still catches a vanished/blank/
            # stale surface (a dark-vs-light swing is a ~200 diff).
            coarse = True
            i += 1
        elif args[i] in opts:
            opts[args[i]] = args[i + 1] if args[i] == "--region" else float(args[i + 1])
            i += 2
        else:
            paths.append(args[i])
            i += 1
    if len(paths) != 2:
        print("usage: probe-shot-check.py LIVE.png REF.png [--tol N] [--uniform N] [--block N] [--region all|page] [--coarse]")
        return 2

    live_path, ref_path = paths
    lw, lh, lbpp, live = decode_png(live_path)
    rw, rh, rbpp, ref = decode_png(ref_path)
    scale = lw / rw
    if scale < 1.0 or abs(scale - round(scale)) > 0.01:
        print(f"DEFECT {live_path}: live width {lw} is not an integer multiple of ref width {rw}")
        return 1
    scale = round(scale)
    # Bottom-anchor: the live shot may carry OS chrome ABOVE the surface.
    y_off = lh - rh * scale
    if y_off < 0:
        print(f"DEFECT {live_path}: live {lw}x{lh} shorter than ref {rw}x{rh} at scale {scale}")
        return 1

    block = int(opts["--block"])
    tol, uniform = opts["--tol"], opts["--uniform"]
    x_min = int(rw * 0.20) if opts["--region"] == "page" else 0

    if coarse:
        rm = block_mean(ref, rbpp, x_min, 0, rw, rh, step=2)
        lm = block_mean(live, lbpp, x_min * scale, y_off, lw, lh, step=2 * scale)
        diff = max(abs(lm[c] - rm[c]) for c in range(3))
        if diff > tol:
            print(
                f"DEFECT {live_path}: coarse region mean rgb({lm[0]:.0f},{lm[1]:.0f},{lm[2]:.0f})"
                f" differs from expected rgb({rm[0]:.0f},{rm[1]:.0f},{rm[2]:.0f}) by {diff:.0f} (> {tol})"
            )
            return 1
        print(
            f"PASS {live_path}: coarse region mean rgb({lm[0]:.0f},{lm[1]:.0f},{lm[2]:.0f})"
            f" within {tol} of expected rgb({rm[0]:.0f},{rm[1]:.0f},{rm[2]:.0f}) (diff {diff:.0f})"
        )
        return 0

    checked = skipped = 0
    fails = []
    for by in range(0, rh - block + 1, block):
        for bx in range(x_min, rw - block + 1, block):
            rm = block_mean(ref, rbpp, bx, by, bx + block, by + block)
            if block_mad(ref, rbpp, bx, by, bx + block, by + block, rm) > uniform:
                skipped += 1
                continue
            lm = block_mean(
                live, lbpp,
                bx * scale, y_off + by * scale,
                (bx + block) * scale, y_off + (by + block) * scale,
                step=max(1, scale),
            )
            diff = max(abs(lm[c] - rm[c]) for c in range(3))
            checked += 1
            if diff > tol:
                fails.append((diff, bx, by, rm, lm))

    if fails:
        fails.sort(reverse=True)
        print(f"DEFECT {live_path}: {len(fails)}/{checked} uniform blocks differ > {tol} (worst first):")
        for diff, bx, by, rm, lm in fails[:5]:
            print(
                f"  block ({bx},{by})+{block}: expected rgb({rm[0]:.0f},{rm[1]:.0f},{rm[2]:.0f})"
                f" got rgb({lm[0]:.0f},{lm[1]:.0f},{lm[2]:.0f}) diff {diff:.0f}"
            )
        return 1
    print(
        f"PASS {live_path}: {checked} uniform blocks within {tol} of {ref_path}"
        f" ({skipped} text/varied blocks skipped, scale {scale}x, y-offset {y_off}px)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
