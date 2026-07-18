//! THE THEME-CAPABILITIES-AS-DATA LAW (grep-law test, mirrors
//! `println_audit.rs`'s scanner shape): structurally bans two patterns from
//! ever reappearing in `src/render/**`'s RUNTIME code — a bare
//! `Theme::is_one_bit()` read, and a hardcoded per-world NAME string literal
//! (`"Wagtail"`, or any of the other fourteen). Both were the pre-round shape
//! of a per-theme special case; every render decision they used to gate now
//! reads a declarative field on `Theme::render_caps` instead (see
//! `theme::model::RenderCaps`'s own module doc), so a FUTURE theme wanting
//! one of those same behaviors only ever has to set a field, never grow a new
//! branch here. `is_one_bit()` itself still exists as a pure derivation
//! helper in `theme::model` (used by identity-pinning law tests, e.g.
//! `theme::tests::wagtail_alone_is_one_bit`) — this law only reaches the
//! RENDERER, never `theme::` itself.
//!
//! Mirrors `println_audit::scan_file`'s cfg(test)-block skip (a stray
//! `#[cfg(test)]` fixture inside an otherwise-real file is exempt) and its
//! `tests/`-directory / `tests.rs`-file skip (a whole test module is exempt
//! outright — that's exactly where `is_one_bit()`/world-name comparisons are
//! still legitimate: pinning which world IS the one true 1-bit world, or
//! driving a capture fixture by name). Doc-comment lines (`///`/`//!`/plain
//! `//`) are also skipped — this file, and several others, still MENTION
//! `is_one_bit`/"Wagtail" in prose explaining the history; only a real code
//! reference trips the law.

const WORLD_NAMES: &[&str] = &[
    "Tawny", "Mopoke", "Currawong", "Potoroo", "Outback", "Undertow", "Kingfisher", "Gumtree",
    "Bilby", "Saltpan", "Quokka", "Mangrove", "Galah", "Magpie", "Wagtail", "Firetail", "Brolga",
    "Cassowary",
];

/// True iff `line` (already known to be OUTSIDE a skipped cfg(test) block)
/// contains a banned pattern: `.is_one_bit(` as a real call, or a quoted
/// per-world name.
fn line_violates(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return None; // doc comment or plain comment line — prose, not code.
    }
    if line.contains(".is_one_bit(") {
        return Some("calls Theme::is_one_bit() directly".to_string());
    }
    for name in WORLD_NAMES {
        let quoted = format!("\"{name}\"");
        if line.contains(&quoted) {
            return Some(format!("hardcodes the world name {quoted}"));
        }
    }
    None
}

/// Scan `text`, skipping every `#[cfg(test)...]`-gated item (brace-balanced,
/// identical shape to `println_audit::scan_file`), and collect every
/// violating line's 1-based line number + reason.
fn scan_file(text: &str) -> Vec<(usize, String)> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Normal,
        AfterCfgTest,
        InSkippedBlock(i32),
    }
    let mut state = State::Normal;
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        state = match state {
            State::Normal => {
                let t = line.trim_start();
                if t.starts_with("#[cfg(test)") || t.starts_with("#[cfg(all(test") {
                    State::AfterCfgTest
                } else {
                    if let Some(reason) = line_violates(line) {
                        hits.push((i + 1, reason));
                    }
                    State::Normal
                }
            }
            State::AfterCfgTest => {
                let t = line.trim_start();
                if t.starts_with("#[") {
                    State::AfterCfgTest
                } else if line.contains('{') {
                    let d = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                    if d <= 0 {
                        State::Normal
                    } else {
                        State::InSkippedBlock(d)
                    }
                } else if line.trim_end().ends_with(';') {
                    State::Normal
                } else {
                    State::AfterCfgTest
                }
            }
            State::InSkippedBlock(depth) => {
                let d = depth + line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if d <= 0 {
                    State::Normal
                } else {
                    State::InSkippedBlock(d)
                }
            }
        };
    }
    hits
}

/// Walk `dir` (relative to `base`) collecting `(rel_path, line, reason)`
/// violations, skipping any `tests` subdirectory and any file literally named
/// `tests.rs` — the exact exemption shape `println_audit.rs` uses, since
/// that's where a real per-world identity check legitimately lives.
fn scan_dir(
    base: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(String, usize, String)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("tests") {
                continue;
            }
            // The unified `--bench-suite` harness (`render/benchsuite/`) is
            // the SAME class of exemption as `framebench.rs`/`perfbench.rs`
            // below: a CLI bench DRIVER that legitimately cycles concrete
            // world names to force reshape-cost measurements (its theme
            // scenario pins a different-face, different-mono world pair),
            // never a per-theme render branch. Whole-dir skip, mirroring the
            // per-file skips.
            if path.file_name().and_then(|n| n.to_str()) == Some("benchsuite") {
                continue;
            }
            scan_dir(base, &path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("tests.rs") {
            continue;
        }
        // The CLI perf-bench harnesses (`--bench-frame`/`--bench-perf`, driven
        // by `main.rs`, never by `apply_core`/the live render decisions this
        // law is about) legitimately CYCLE by concrete world NAME to force a
        // font-reshape/switch-cost measurement — that's a bench DRIVER
        // picking which worlds to visit, not a per-theme render branch. Not a
        // `render_caps` law concern; exempt by name, same shape as
        // `println_audit.rs`'s per-file allowances.
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if fname == "framebench.rs" || fname == "perfbench.rs" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        for (line, reason) in scan_file(&text) {
            out.push((rel.clone(), line, reason));
        }
    }
}

#[test]
fn render_never_reads_is_one_bit_or_hardcodes_a_world_name() {
    let render_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("render");
    let mut hits = Vec::new();
    scan_dir(&render_root, &render_root, &mut hits);
    // `src/render.rs` itself (the GPU-core file, sibling to the `render/`
    // dir) is the crate::render module root too — scan it explicitly.
    let render_rs = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/render.rs");
    if let Ok(text) = std::fs::read_to_string(&render_rs) {
        for (line, reason) in scan_file(&text) {
            hits.push(("../render.rs".to_string(), line, reason));
        }
    }

    assert!(
        hits.is_empty(),
        "render code must read Theme::render_caps fields, never Theme::is_one_bit() or a \
         hardcoded world name — offending lines:\n{}",
        hits.iter()
            .map(|(f, l, r)| format!("  {f}:{l}: {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn line_violates_catches_both_patterns_and_skips_comments() {
    assert!(line_violates("if theme::active().is_one_bit() {").is_some());
    assert!(line_violates("theme::set_active_by_name(\"Wagtail\").unwrap();").is_some());
    assert!(line_violates("/// `Theme::is_one_bit` — history note").is_none());
    assert!(line_violates("// mentions \"Wagtail\" in prose").is_none());
    assert!(line_violates("let x = theme.render_caps.elevation;").is_none());
}
