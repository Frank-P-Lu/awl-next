//! tests/bullet_blank_line_nit_pixels.rs — queue item 72: THE STRAY-MARK PIXEL LAW
//! for a blank line immediately after an OFF-CURSOR empty unordered bullet marker.
//!
//! ROOT CAUSE (see the fix, `render::rects::TextPipeline::nit_hidden_by_bullet_glyph`):
//! an off-cursor empty marker line (`"- "` alone) still carries a genuine
//! TRAILING-WHITESPACE writing-nit on its own required space (`crate::nits::
//! line_nits("- ") == [(1, 2)]`) — the marker's raw `"-"` + `" "` are only
//! DIM-inked when concealed (unlike an off-cursor image SOURCE, which shrinks to
//! near-zero width, see `IMAGE_CONCEAL_UNDERLINE_MIN_ADVANCE`), so the nit's own
//! muted underline quad is NOT masked by the depth-derived bullet glyph drawn on
//! top of the text. That quad sits "a hair below the cell bottom" of row 0 (per
//! `nit_underlines`'s row-bottom placement) — which, at a full-height row, lands
//! almost exactly on row 1's own TOP edge. With row 0's marker text visually
//! replaced by nothing but a lone bullet, there is nothing above the tick to
//! anchor it to: it reads as a small stray mark floating on the blank line below,
//! even though it is really row 0's own (now-orphaned) nit.
//!
//! This spawns the REAL `awl` binary (`CARGO_BIN_EXE_awl`, mirroring
//! `tests/frost_rail_pixels.rs` / `tests/hermetic_canary.rs`) and asserts, by
//! RGB pixel arithmetic over the capture PNG (never inferred from the sidecar —
//! CLAUDE.md's appearance-oracle rule), that the blank line's pixel band renders
//! BYTE-IDENTICAL to an ordinary blank line's — i.e. no ink at all — on both a
//! LIGHT world (Magpie) and a LAVA world (Firetail, a non-flat procedural
//! background, so the assertion diffs two REAL captures of the same background
//! function rather than assume a flat/uniform color anywhere).
//!
//! `ListStyle::Pane`/`Bars` (docs/render.md) is a PICKER/overlay row layout
//! axis — this bug lives entirely in ordinary document-body rendering, with no
//! overlay open, so that axis does not apply here.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh, uniquely-named tempdir under the OS temp root (mirrors
/// `tests/frost_rail_pixels.rs`'s `tmp_dir`).
fn tmp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("awl-bullet72-pixels-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The bug fixture: an off-cursor EMPTY unordered marker (`"- "` alone), a
/// blank second line, then ordinary content — the exact static shape the
/// wave-7 vision-smoke found this in, byte-for-byte (a genuine trailing space
/// after the dash, a genuinely empty second line).
const BUG_DOC: &str = "- \n\nsomething\n";

/// The control: an ORDINARY first line (no list marker at all — nothing to
/// conceal, nothing to nit at its own trailing edge), otherwise identical
/// shape (single-char first line, blank second, same third line) — so line 1's
/// pixel band is expected to be genuinely, unremarkably blank in both.
const CONTROL_DOC: &str = "a\n\nsomething\n";

/// Spawn the real binary for one `--screenshot` capture of `doc`, moving the
/// caret to line 2 ("something") first via `--keys "C-n C-n"` — OFF the first
/// line, so an unordered marker there conceals to its bullet glyph (the
/// precise state the vision-smoke and the fix target: reveal-on-cursor keeps
/// the marker's own line's nits showing on-cursor, matching the raw text still
/// being visible there — never the bug). Returns `false` iff no GPU adapter
/// was available (mirrors the suite's `adapter_available()` tolerance).
fn capture(out: &Path, doc: &Path, theme: &str) -> bool {
    let output = Command::new(env!("CARGO_BIN_EXE_awl"))
        .arg("--theme")
        .arg(theme)
        .arg("--screenshot")
        .arg(out)
        .arg("--keys")
        .arg("C-n C-n")
        .arg(doc)
        .env_remove("AWL_CJK_FORCE")
        .env_remove("AWL_CONFIG")
        .output()
        .expect("failed to spawn the awl binary under CARGO_BIN_EXE_awl");
    if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("no wgpu adapter for headless capture")
    {
        return false;
    }
    assert!(
        output.status.success(),
        "awl capture failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    out.exists()
}

/// Parse a capture's sidecar JSON (mirrors `tests/hermetic_canary.rs::sidecar`).
fn sidecar(png: &Path) -> serde_json::Value {
    let json = std::fs::read_to_string(png.with_extension("json")).expect("sidecar exists");
    serde_json::from_str(&json).expect("sidecar parses")
}

/// Decode a capture PNG to `(width, height, rgba_bytes)` (mirrors
/// `tests/frost_rail_pixels.rs::decode`).
fn decode(png: &Path) -> (u32, u32, Vec<u8>) {
    let img = image::open(png).unwrap_or_else(|e| panic!("decode {}: {e}", png.display())).to_rgba8();
    (img.width(), img.height(), img.into_raw())
}

/// The pipeline is inert without a GPU adapter; on a headless box with none,
/// skip rather than fail (mirrors `frost_rail_pixels.rs::adapter_or_skip`).
fn adapter_or_skip(dir: &Path, doc: &Path) -> bool {
    let probe = dir.join("probe.png");
    if capture(&probe, doc, "Magpie") {
        return true;
    }
    eprintln!("skipping bullet_blank_line_nit_pixels: no wgpu adapter (capture produced no PNG)");
    false
}

/// Every RGB pixel in `[x0,x1) x [y0,y1)` byte-identical between two
/// same-sized captures — the "a blank line renders identically regardless of
/// what precedes it" invariant. Diffing two REAL captures (rather than
/// asserting a hardcoded/flat background color) keeps this honest on a LAVA
/// world's non-flat procedural ground: the ONLY variable between the two
/// fixtures at this row is the (fixed) preceding line's content, so any pixel
/// difference in this band is the stray mark (or a future regression like it),
/// never legitimate background variation.
fn assert_band_identical(
    bug: &(u32, u32, Vec<u8>),
    control: &(u32, u32, Vec<u8>),
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    label: &str,
) {
    let (bw, bh, bug_px) = bug;
    let (cw, ch, ctrl_px) = control;
    assert_eq!((bw, bh), (cw, ch), "{label}: captures must share canvas dims");
    let w = *bw;
    let mut diffs: Vec<(u32, u32, [u8; 3], [u8; 3])> = Vec::new();
    for y in y0..y1.min(*bh) {
        for x in x0..x1.min(w) {
            let i = ((y * w + x) * 4) as usize;
            let bp = [bug_px[i], bug_px[i + 1], bug_px[i + 2]];
            let cp = [ctrl_px[i], ctrl_px[i + 1], ctrl_px[i + 2]];
            if bp != cp {
                diffs.push((x, y, bp, cp));
            }
        }
    }
    assert!(
        diffs.is_empty(),
        "{label}: {} pixel(s) differ in the blank-line band where item 72's stray mark \
         used to paint — first few (x, y, bug_rgb, control_rgb): {:?}",
        diffs.len(),
        &diffs[..diffs.len().min(8)]
    );
}

#[test]
fn blank_line_after_off_cursor_empty_bullet_matches_an_ordinary_blank_line() {
    let root = tmp_dir("main");
    let bug_doc = root.join("bug.md");
    let control_doc = root.join("control.md");
    std::fs::write(&bug_doc, BUG_DOC).unwrap();
    std::fs::write(&control_doc, CONTROL_DOC).unwrap();

    if !adapter_or_skip(&root, &bug_doc) {
        return;
    }

    // A light world (flat/pinstripe background) and a lava world (a non-flat
    // procedural background) — the two backdrop families the render caps span
    // (docs/render.md's `RenderCaps::backdrop`).
    for theme in ["Magpie", "Firetail"] {
        let bug_png = root.join(format!("{theme}_bug.png"));
        let ctrl_png = root.join(format!("{theme}_control.png"));
        assert!(capture(&bug_png, &bug_doc, theme), "{theme}: bug capture");
        assert!(capture(&ctrl_png, &control_doc, theme), "{theme}: control capture");

        let bug_side = sidecar(&bug_png);
        let ctrl_side = sidecar(&ctrl_png);
        // Sanity: the caret really landed OFF the first line (so the bullet
        // glyph — and the bug, pre-fix — actually engages), and both fixtures
        // keep the 3-plain-line shape the row-band math below assumes.
        assert_eq!(bug_side["cursor"]["line"], 2, "{theme}: bug caret should land on line 2");
        assert_eq!(ctrl_side["cursor"]["line"], 2, "{theme}: control caret should land on line 2");
        assert_eq!(bug_side["first_lines"][0], "- ", "{theme}: bug fixture's marker line");
        assert_eq!(bug_side["first_lines"][1], "", "{theme}: bug fixture's blank line");
        assert_eq!(ctrl_side["first_lines"][1], "", "{theme}: control fixture's blank line");

        let top = bug_side["text_origin"]["top"].as_f64().expect("text_origin.top");
        let line_h = bug_side["font"]["line_height"].as_f64().expect("font.line_height");
        let col_left = bug_side["page"]["column"]["left"].as_f64().expect("page.column.left");
        let col_w = bug_side["page"]["column"]["width"].as_f64().expect("page.column.width");
        assert_eq!(
            ctrl_side["text_origin"]["top"].as_f64(),
            Some(top),
            "{theme}: same font metrics"
        );
        assert_eq!(
            ctrl_side["font"]["line_height"].as_f64(),
            Some(line_h),
            "{theme}: same font metrics"
        );

        // Row 1 (the blank line)'s band, widened 4px UPWARD past its nominal
        // top: the pre-fix mark sat right at the row-0/row-1 seam (a
        // trailing-whitespace nit's "cell bottom + 1px" placement is nearly
        // the full row height below row 0's own top), 1-2px into what is
        // nominally still row 0's own pixel rows. [top + line_h - 4, top +
        // 2*line_h) stays clear of row 0's actual glyph ink (bullet / "a", both
        // end well above this band) and of row 2's ascenders on both fixtures,
        // so it safely catches the historical mark while covering the whole
        // blank row.
        let y0 = (top + line_h - 4.0).round() as u32;
        let y1 = (top + 2.0 * line_h).round() as u32;
        let x0 = col_left.round() as u32;
        let x1 = (col_left + col_w).round() as u32;

        let bug_px = decode(&bug_png);
        let ctrl_px = decode(&ctrl_png);

        // Meaningfulness guard: the two full captures must NOT be byte-
        // identical overall (row 0 genuinely differs — a bullet glyph vs a
        // plain "a") — otherwise a silently-broken capture could vacuously
        // pass the band-only comparison below.
        assert_ne!(
            bug_px.2, ctrl_px.2,
            "{theme}: bug and control captures must differ somewhere (row 0's content)"
        );

        assert_band_identical(
            &bug_px,
            &ctrl_px,
            x0,
            x1,
            y0,
            y1,
            &format!("{theme} blank-line band"),
        );
    }
}
