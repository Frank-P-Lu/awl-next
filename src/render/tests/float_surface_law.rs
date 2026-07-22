//! THE FLOAT-SURFACE PRIMITIVE LAW (grep-law test, mirrors `theme_caps_law`'s
//! scanner shape): `chrome::mod::set_float_quads` is the private quad-math fn
//! behind every summoned "small floating card, no scrim" surface. Some owners
//! (the HUD card, the which-key panel, the menu-bar dropdown) legitimately
//! carry their OWN dedicated elevation trio and call it directly — those are
//! OUT OF SCOPE for this round, an intentional allowlist. What this law bans
//! is a SECOND direct caller among the files THIS round unified: the caret-
//! style preview panel, the search panel, the contextual SPELL popup, and the
//! format popover (the "mouse-highlight popover" — it rides the selection a
//! mouse-drag makes) must ALL reach it ONLY through
//! `TextPipeline::prepare_float_panel`, never inline.
//!
//! THE BUG THIS CLOSED (overlay/chrome polish round): the format popover used
//! to build its OWN `popover_shadow`/`popover_border`/`popover_card` trio and
//! call `set_float_quads` inline — the exact "same behavior, different code"
//! duplication the spell popup's own call (already routed through
//! `prepare_float_panel`) had already solved once. A future summoned
//! micro-panel among this SAME family (a link preview, a thesaurus popup) can
//! no longer quietly reinvent its own elevation call either.
//!
//! Mirrors `theme_caps_law`'s doc-comment skip: a `///`/`//!`/plain `//` line
//! naming `set_float_quads(` in prose (this file included) is exempt — only a
//! real code reference trips the law.

/// Files allowed to call `set_float_quads(` DIRECTLY: `chrome/mod.rs` (the
/// owner — its own definition, plus `prepare_float_panel`'s one call), and the
/// three float-panel families this round deliberately left alone (each keeps
/// its own dedicated elevation trio, never shared — a bigger unification than
/// this round's scope).
const ALLOWED_DIRECT_CALLERS: &[&str] =
    &["chrome/mod.rs", "chrome/hud.rs", "chrome/menubar.rs", "chrome/whichkey.rs"];

/// True iff `line` (real code, not a comment) calls `set_float_quads(`.
fn line_violates(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return false; // doc comment or plain comment — prose, not code.
    }
    line.contains("set_float_quads(")
}

/// Walk `dir`, collecting `(rel_path, line)` violations — every `.rs` file
/// under `src/render/` whose relative path is NOT in [`ALLOWED_DIRECT_CALLERS`].
fn scan_dir(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, usize)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(base, &path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        if ALLOWED_DIRECT_CALLERS.contains(&rel.as_str()) {
            continue;
        }
        // This scanner's OWN source necessarily embeds the pattern string it
        // searches for (the doc comments above, `ALLOWED_DIRECT_CALLERS`'s own
        // doc) — self-exempt by name, mirroring `theme_caps_law`'s `tests.rs`
        // skip (a whole test file, not a line-level heuristic, is the right
        // grain for "this file IS the scanner").
        if path.file_name().and_then(|n| n.to_str()) == Some("float_surface_law.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        for (i, line) in text.lines().enumerate() {
            if line_violates(line) {
                out.push((rel.clone(), i + 1));
            }
        }
    }
}

#[test]
fn float_surface_primitive_has_no_bypass_among_the_unified_family() {
    let render_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("render");
    let mut hits = Vec::new();
    scan_dir(&render_root, &render_root, &mut hits);
    assert!(
        hits.is_empty(),
        "the caret-preview panel / search panel / spell popup / format popover must \
         route through `TextPipeline::prepare_float_panel`, never call \
         `set_float_quads` directly — offending lines:\n{}",
        hits.iter().map(|(f, l)| format!("  {f}:{l}")).collect::<Vec<_>>().join("\n")
    );

    // NON-VACUOUS: the owner file itself still carries the expected hits — the
    // fn's own `fn set_float_quads(` definition, plus its three IN-MODULE
    // callers (`prepare_float_panel`, and the unrelated `prepare_diff_panel` /
    // `prepare_panel_card_elevation`, which own their OWN dedicated buffer
    // trios and are out of this round's scope) — if `prepare_float_panel`
    // were ever deleted outright this count would drop and the law would go
    // quiet without ever having exercised its ban.
    let owner = render_root.join("chrome/mod.rs");
    let text = std::fs::read_to_string(&owner).expect("chrome/mod.rs must exist");
    let real_hits = text.lines().filter(|l| line_violates(l)).count();
    assert_eq!(
        real_hits, 4,
        "expected the fn definition + its three in-module callers; found {real_hits}"
    );

    // NON-VACUOUS (the popover-specific regression): `chrome/popover.rs` — the
    // file this round's fix touched — carries ZERO direct calls; if a future
    // edit reintroduced one, `hits` above would catch it, but this pins the
    // exact file the bug lived in so the law's target is explicit.
    let popover = render_root.join("chrome/popover.rs");
    let popover_text = std::fs::read_to_string(&popover).expect("chrome/popover.rs must exist");
    let popover_hits = popover_text.lines().filter(|l| line_violates(l)).count();
    assert_eq!(popover_hits, 0, "chrome/popover.rs must never call set_float_quads directly");
}

#[test]
fn line_violates_catches_the_call_and_skips_comments() {
    assert!(line_violates("        set_float_quads("));
    assert!(line_violates("fn set_float_quads("));
    assert!(!line_violates("/// calls `set_float_quads` — history note"));
    assert!(!line_violates("// mentions set_float_quads( in prose"));
    assert!(!line_violates("let x = theme.render_caps.elevation;"));
}

#[test]
fn scan_dir_exempts_this_scanners_own_file_by_name() {
    // This file's OWN source embeds `set_float_quads(` in prose/patterns
    // (this test's assertions above included) — `scan_dir` must skip it by
    // filename, never by trying to out-clever the substring match, so this
    // test module itself never becomes a false positive.
    let render_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("render");
    let mut hits = Vec::new();
    scan_dir(&render_root, &render_root, &mut hits);
    assert!(
        !hits.iter().any(|(f, _)| f.ends_with("float_surface_law.rs")),
        "the scanner must never flag its own source: {hits:?}"
    );
}
