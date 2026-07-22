//! THE SELECTION-BAND GLIDE-ANCHOR LAW (grep-law test, mirrors
//! `theme_caps_law.rs`'s scanner shape, incl. its `tests/`-directory skip) —
//! neighborhood audit, 2026-07-22 (queue item 19, Trigger-3 sweep over item
//! 8's overlay/selection class).
//!
//! `TextPipeline::retarget_band` (`pipeline_overlay.rs`) is THE ONE owner of
//! the "chase from the band's true drawn position, never a stale previous
//! target" fix (the 2026-07-22 user-reported selection-desync bug — see that
//! fn's own doc). Its two callers, `overlay_band_drawn` and
//! `living_band_phase`, are the ONLY doors any summoned surface's RENDER path
//! may use to animate a selection highlight. This law bans a SECOND
//! production call site: if the command palette, the theme picker, the
//! rebind menu, or any future navigable overlay ever grew its OWN chase-from-
//! target selection animator instead of routing through these two, it could
//! silently reintroduce the exact stale-anchor bug `retarget_band` closed —
//! independently, in code this scanner would otherwise never see.
//!
//! (Both fns are `pub(in crate::render)`, unlike `set_float_quads` — which is
//! module-private and so structurally unreachable from `render::tests::*` —
//! so, mirroring `theme_caps_law`'s reason for its own `tests/`-dir skip
//! rather than `float_surface_law`'s narrower per-file allowlist, the WHOLE
//! `tests/` directory is exempt here: that's exactly where the pure-fn
//! regression coverage for the chase formula itself legitimately calls these
//! two directly, e.g.
//! `living_band_phase_chains_from_the_actual_drawn_position_not_the_stale_target`
//! in `tests/firetail_showcase.rs`.)
//!
//! THE OTHER HALF THE AUDIT NAMED (search / spell popup / which-key /
//! menu-bar): none of those four surfaces carries ANY selection-glide
//! animator at all (confirmed by inspection — `chrome/panel.rs`'s search
//! card has no candidate list to highlight, `chrome/whichkey.rs` and
//! `chrome/menubar.rs` draw static rows with no per-row band, and the
//! contextual spell POPUP is the format popover's `prepare_popover`, which
//! never calls `overlay_band_drawn`/`living_band_phase` either) — they are
//! structurally immune to this bug class, not merely untested against it.
//! This law is what keeps that true: a stray call from any of those
//! PRODUCTION files (or a brand-new summoned surface) trips it immediately.
//!
//! Mirrors `theme_caps_law`'s doc-comment skip: a `///`/`//!`/plain `//` line
//! naming the two fn names in prose (this file included) is exempt — only a
//! real code reference trips the law.

/// The ONE production file allowed to call `overlay_band_drawn(`/
/// `living_band_phase(` directly: `chrome/overlay_rows.rs`, the shared
/// Pane-family row-fill seam every `OverlayKind`'s render passes through
/// (see `living_covered_rows`/`living_probe_geom` and the
/// `BandResponse::Slide` fill arm). `pipeline_overlay.rs` itself (the two
/// fns' own definitions) is scanned too but excluded by NAME below, mirroring
/// `float_surface_law`'s owner-file treatment.
const ALLOWED_DIRECT_CALLER: &str = "chrome/overlay_rows.rs";

/// True iff `line` (real code, not a comment) calls `overlay_band_drawn(` or
/// `living_band_phase(`.
fn line_violates(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return false; // doc comment or plain comment — prose, not code.
    }
    line.contains("overlay_band_drawn(") || line.contains("living_band_phase(")
}

/// Walk `dir` (relative to `base`), skipping any `tests` subdirectory (the
/// exact exemption `theme_caps_law.rs` uses, for the same reason: that's
/// where the pure-fn chase-formula regression coverage legitimately lives)
/// and `pipeline_overlay.rs` itself (the two fns' own definitions — checked
/// separately below, non-vacuously), collecting `(rel_path, line)` violations.
fn scan_dir(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, usize)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("tests") {
                continue;
            }
            scan_dir(base, &path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("pipeline_overlay.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        for (i, line) in text.lines().enumerate() {
            if line_violates(line) {
                out.push((rel.clone(), i + 1));
            }
        }
    }
}

#[test]
fn glide_anchor_has_no_bypass_among_summoned_surfaces() {
    let render_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("render");
    let mut hits = Vec::new();
    scan_dir(&render_root, &render_root, &mut hits);
    assert!(
        hits.iter().all(|(f, _)| f == ALLOWED_DIRECT_CALLER),
        "only `{ALLOWED_DIRECT_CALLER}` (the shared Pane-family row-fill seam) may call \
         `overlay_band_drawn`/`living_band_phase` — a second call site could reintroduce \
         the 2026-07-22 stale-anchor selection-desync bug independently. offending lines:\n{}",
        hits.iter()
            .filter(|(f, _)| f != ALLOWED_DIRECT_CALLER)
            .map(|(f, l)| format!("  {f}:{l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // NON-VACUOUS: `chrome/overlay_rows.rs` — the ONE legitimate production
    // caller — really does carry calls (if a refactor ever routed the fill
    // through a different file without updating the allowlist, this pins the
    // expectation so the law's target stays explicit rather than silently
    // trivial).
    assert!(
        hits.iter().any(|(f, _)| f == ALLOWED_DIRECT_CALLER),
        "chrome/overlay_rows.rs must be the real Pane-family row-fill seam \
         (found no calls there at all — the scan itself may be broken)"
    );

    // NON-VACUOUS: `pipeline_overlay.rs` (excluded by name above) still
    // carries the two fns' own definitions — if either were ever deleted
    // outright this count would drop and the law would go quiet without
    // ever having exercised its ban.
    let owner = render_root.join("pipeline_overlay.rs");
    let text = std::fs::read_to_string(&owner).expect("pipeline_overlay.rs must exist");
    let real_hits = text.lines().filter(|l| line_violates(l)).count();
    assert_eq!(
        real_hits, 2,
        "expected exactly the two fn definitions (`overlay_band_drawn`/`living_band_phase`); found {real_hits}"
    );
}

#[test]
fn line_violates_catches_both_calls_and_skips_comments() {
    assert!(line_violates("        self.overlay_band_drawn(target)"));
    assert!(line_violates("let (from, to, t) = self.living_band_phase(force, target, lh);"));
    assert!(!line_violates("/// calls `overlay_band_drawn` — history note"));
    assert!(!line_violates("// mentions living_band_phase( in prose"));
    assert!(!line_violates("let x = theme.render_caps.elevation;"));
}

#[test]
fn scan_dir_exempts_the_tests_directory() {
    // The pure-fn chase-formula regression coverage in `tests/firetail_showcase.rs`
    // calls both fns directly (legitimate — see the module doc); a scan that
    // failed to skip the `tests/` subdirectory would flag them as violations.
    let render_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("render");
    let mut hits = Vec::new();
    scan_dir(&render_root, &render_root, &mut hits);
    assert!(
        !hits.iter().any(|(f, _)| f.starts_with("tests/")),
        "the scanner must skip the whole `tests/` directory: {hits:?}"
    );
}
