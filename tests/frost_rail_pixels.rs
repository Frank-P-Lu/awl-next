//! tests/frost_rail_pixels.rs — THE RENDERED-PIXEL FROST REGRESSION (the missing
//! appearance oracle for the FROST RAIL round).
//!
//! Every OTHER frost test asserts over the pure-Rust shader MIRROR
//! (`lava::frost_field` / `frost_pixel` / pill-rect geometry) — never an actual
//! GPU-rendered pixel. That is exactly the standing tripwire CLAUDE.md names:
//! "appearance properties MUST be asserted over the PNG's pixels — arithmetic
//! over the bytes, never inferred from state." A future break of the GPU frost
//! UPLOAD or SHADER path (pills not written, a wrong uniform, a dropped bind)
//! would keep every mirror/law test GREEN while the render silently reverted to
//! the raw, unfrosted lamp. Nothing committed guarded that — this test does.
//!
//! It spawns the REAL `awl` binary (`CARGO_BIN_EXE_awl`, the same mechanism as
//! `tests/hermetic_canary.rs` / `tests/fault_kill9.rs`) and diffs a frost-ON
//! capture against a frost-OFF one (`AWL_LAVA_FROST=off`, the shipped dev A/B
//! knob) of the SAME frozen lava composition — so the ONLY variable is whether
//! the frost path ran. Three cells, sampled along the changed axis:
//!
//!   1. HEADED lava doc, Mangrove + Firetail, phase 0.7 (a blob intrudes the
//!      fixed top rail there). The frosted pills — the OUTLINE entries' rail pills
//!      PLUS the bottom-left GUTTER pill — DIM real pixels: thousands change
//!      (measured 16750 Mangrove / 16981 Firetail). Asserted as a SUBSTANTIAL
//!      changed-pixel count via `image`-decoded RGB arithmetic, not merely "the
//!      bytes differ".
//!   2. HEADING-LESS lava doc (no outline, but the page-mode GUTTER still draws)
//!      — the GUTTER frost pill ALONE dims real pixels (measured 2734 Mangrove /
//!      2791 Firetail). This is the direct GPU guard for the gutter de-uglify fix:
//!      the bottom-left corner is frosted (a softened lamp) rather than the old
//!      hard-carved dead-flat dark rectangle, even with no outline.
//!   3. NON-LAVA world (Tawny, no lava ground) — byte-IDENTICAL: frost is gated on
//!      the lava-ground CAPABILITY, so a flat-ground world is untouched (no
//!      outline pill AND no gutter pill).
//!
//! Cell 3 is the negative control that keeps cells 1 & 2 honest: it proves the
//! diff is the lava-gated frost path and nothing ambient. (The "frost only touches
//! pixels a pill covers" no-op is law-tested at the pure seam —
//! `theme::tests::gutter_frost_pill_keeps_ink_contrast_on_every_lava_world`
//! part (4): coverage is exactly 0 outside every pill.)

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh, uniquely-named tempdir under the OS temp root (no `tempfile` dep —
/// mirrors `tests/hermetic_canary.rs`).
fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("awl-frost-pixels-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A headed markdown doc: four headings a fair way apart so the margin OUTLINE
/// draws several stacked entries → several frost pills in the left rail.
const HEADED: &str = "\
# Frost Rail Notes

Ordinary opening paragraph before any subheading, long enough to wrap across
a couple of lines so the outline entry and the body text share the margin
edge in a natural way.

## First Section

Some body text under the first section heading, establishing a second
outline entry a fair way down the page.

## Second Section

More body text under a third heading further down, so the outline carries
several entries stacked in the left rail while the lava keeps moving in the
gaps between the frosted pills.

### A Nested Point

A final nested heading to give the outline some depth and a fourth pill to
inspect in the rail.
";

/// A heading-less doc: the outline stays empty, so no pill is ever placed.
const ORDINARY: &str = "\
Ordinary paragraph text with no heading markers anywhere in this file, so
the outline stays empty and the margin outline rail never draws.

A second ordinary paragraph, just prose, continuing on with no structure
beyond plain sentences and the occasional line break to fill out the page.

A third paragraph rounds things out so the document has enough visible
height to judge both margins at once, still without a single heading line.
";

/// Spawn the real binary for one `--screenshot` capture of `doc`, into `out`.
/// `frost` toggles the shipped default (`true`) vs. the A/B off knob (`false`).
/// `lava` (`Some("deepsea"|"warm")`) forces the frozen lava composition at phase
/// 0.7 via `AWL_LAVA` (the dev gallery knob — the ONLY way to pin a phase
/// headlessly); `None` leaves the world's own background stand (the non-lava
/// control). Returns `false` iff no GPU adapter was available (the capture
/// produced no PNG) — the caller then skips, mirroring the suite's
/// `adapter_available()` tolerance on a headless box.
fn capture(out: &Path, doc: &Path, theme: &str, lava: Option<&str>, frost: bool) -> bool {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_awl"));
    cmd.arg("--theme")
        .arg(theme)
        .arg("--screenshot")
        .arg(out)
        .arg(doc)
        // The frost path memoizes `AWL_LAVA_FROST` once per process, so each cell
        // is a fresh child. ON must NOT inherit an ambient `off` from the parent
        // suite env; OFF sets it explicitly. Same for `AWL_LAVA` below.
        .env_remove("AWL_CJK_FORCE")
        .env_remove("AWL_CONFIG");
    if frost {
        cmd.env_remove("AWL_LAVA_FROST");
    } else {
        cmd.env("AWL_LAVA_FROST", "off");
    }
    match lava {
        Some(spec) => {
            cmd.env("AWL_LAVA", format!("{spec}:0.7"));
        }
        None => {
            cmd.env_remove("AWL_LAVA");
        }
    }
    let output = cmd
        .output()
        .expect("failed to spawn the awl binary under CARGO_BIN_EXE_awl");
    assert!(
        output.status.success(),
        "awl capture failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    out.exists()
}

/// Decode a capture PNG to `(width, height, rgba_bytes)`.
fn decode(png: &Path) -> (u32, u32, Vec<u8>) {
    let img = image::open(png)
        .unwrap_or_else(|e| panic!("decode {}: {e}", png.display()))
        .to_rgba8();
    (img.width(), img.height(), img.into_raw())
}

/// Count pixels whose RGB differs between two same-sized RGBA buffers (alpha is a
/// constant 255 in a capture, so ignoring it keeps the count to VISIBLE change).
fn changed_rgb_pixels(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len(), "captures must share dimensions");
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(p, q)| p[0] != q[0] || p[1] != q[1] || p[2] != q[2])
        .count()
}

/// A conservative floor for cell 1: far above zero (so a dead frost path — which
/// would render byte-identical to OFF — fails loudly) yet far below the observed
/// 598/706 (so an honest tuning change of `FROST_DIM`/`FROST_BLUR_PX` never
/// false-fails this GPU-path guard; the exact contrast bounds are the mirror
/// law's job, not this test's).
const FROST_PIXEL_FLOOR: usize = 50;

/// The frost pipeline is inert without an adapter; on a headless CI box with no
/// GPU, skip rather than fail (mirrors `capture::tests::adapter_available`).
fn adapter_or_skip(root: &Path, doc: &Path) -> bool {
    let probe = root.join("probe.png");
    if capture(&probe, doc, "Mangrove", Some("deepsea"), true) {
        return true;
    }
    eprintln!("skipping frost_rail_pixels: no wgpu adapter (capture produced no PNG)");
    false
}

#[test]
fn frost_pills_dim_real_gpu_pixels_on_a_headed_lava_doc() {
    let root = tmp_dir("headed");
    let headed = root.join("headed.md");
    std::fs::write(&headed, HEADED).unwrap();

    if !adapter_or_skip(&root, &headed) {
        return;
    }

    // Cell 1a + 1b: on a HEADED lava doc, frost-ON dims hundreds of real pixels
    // vs. frost-OFF at the identical frozen composition — sampled on two lava
    // worlds (Mangrove/deepsea, Firetail/warm).
    for (theme, spec) in [("Mangrove", "deepsea"), ("Firetail", "warm")] {
        let on = root.join(format!("{theme}_on.png"));
        let off = root.join(format!("{theme}_off.png"));
        assert!(capture(&on, &headed, theme, Some(spec), true));
        assert!(capture(&off, &headed, theme, Some(spec), false));

        let (w1, h1, on_px) = decode(&on);
        let (w2, h2, off_px) = decode(&off);
        assert_eq!((w1, h1), (w2, h2), "{theme}: same canvas");
        let changed = changed_rgb_pixels(&on_px, &off_px);
        assert!(
            changed >= FROST_PIXEL_FLOOR,
            "{theme}: frost changed only {changed} px (floor {FROST_PIXEL_FLOOR}); \
             the GPU frost path may not be writing pills"
        );
    }
}

#[test]
fn frost_dims_the_gutter_corner_and_is_gated_on_the_lava_capability() {
    let root = tmp_dir("controls");
    let headed = root.join("headed.md");
    let ordinary = root.join("ordinary.md");
    std::fs::write(&headed, HEADED).unwrap();
    std::fs::write(&ordinary, ORDINARY).unwrap();

    if !adapter_or_skip(&root, &headed) {
        return;
    }

    // Cell 2: a HEADING-LESS lava doc has no outline, but the page-mode GUTTER
    // still draws (the filename/project stack) — so the GUTTER frost pill ALONE
    // dims real pixels vs. frost-OFF's raw lamp. This is the direct GPU guard for
    // the de-uglify fix: the bottom-left corner is a softened lamp, not the old
    // hard-carved dead-flat dark rectangle. Sampled on both lava worlds.
    for (theme, spec) in [("Mangrove", "deepsea"), ("Firetail", "warm")] {
        let ord_on = root.join(format!("{theme}_ord_on.png"));
        let ord_off = root.join(format!("{theme}_ord_off.png"));
        assert!(capture(&ord_on, &ordinary, theme, Some(spec), true));
        assert!(capture(&ord_off, &ordinary, theme, Some(spec), false));
        let (w1, h1, on_px) = decode(&ord_on);
        let (w2, h2, off_px) = decode(&ord_off);
        assert_eq!((w1, h1), (w2, h2), "{theme}: same canvas");
        let changed = changed_rgb_pixels(&on_px, &off_px);
        assert!(
            changed >= FROST_PIXEL_FLOOR,
            "{theme}: the gutter frost pill changed only {changed} px (floor {FROST_PIXEL_FLOOR}); \
             a heading-less lava doc's bottom-left corner must still be frosted"
        );
    }

    // Cell 3: a NON-LAVA world (Tawny, flat ground → no AWL_LAVA) has no lava
    // capability for frost to gate on. ON and OFF captures are byte-identical even
    // with the outline AND the gutter drawn (no outline pill, no gutter pill).
    let tw_on = root.join("tw_on.png");
    let tw_off = root.join("tw_off.png");
    assert!(capture(&tw_on, &headed, "Tawny", None, true));
    assert!(capture(&tw_off, &headed, "Tawny", None, false));
    assert_eq!(
        std::fs::read(&tw_on).unwrap(),
        std::fs::read(&tw_off).unwrap(),
        "non-lava world: frost must be byte-identical (no lava ground to frost)"
    );
}
