//! src/docs_catalog_law.rs — THE DOCS-VS-CATALOG LAW for `site/guide.html`,
//! the marketing site's static, hand-mirrored copy of `GUIDE.md`.
//!
//! `GUIDE.md` (and `samples/welcome.md` / `samples/tour.md`) already carry a
//! COMPLETE, runtime-rendered version of this law: every chord/command-name
//! citation in those docs is a `{{key:slug}}` / `{{cmd:slug}}` token (see
//! `keytoken.rs`), substituted from the live catalog at open/seed time and
//! checked by `keytoken::tests::every_key_token_in_the_starting_docs_resolves`
//! / `every_cmd_token_in_the_starting_docs_resolves`. `site/guide.html` has no
//! such seam — it is a plain static file the deploy step copies verbatim, so
//! its chord glyphs and command names are literal, hand-typed text (its own
//! header comment already names this an accepted, LOGGED drift risk). This
//! module is the harvest-and-resolve law for THAT surface: two checks, both
//! reading the SAME canonical truth [`commands::generate_keys_reference_markdown`]
//! already produces (the generator `guide::tests::generated_keys_reference_
//! matches_catalog` verifies byte-for-byte against `GUIDE.md`'s checked-in
//! table every run).
//!
//! 1. [`site_guide_html_keys_table_matches_catalog`] — a STRUCTURED harvest:
//!    every row of the page's own keys-reference `<table>` is parsed into
//!    `(name, macOS, Linux)`, joined by [`commands::slug`] against the
//!    canonical rows, and asserted equal. A command cited by a name the live
//!    catalog no longer has (renamed/retired) or a chord cell that no longer
//!    matches fails here with the exact command named.
//! 2. [`site_guide_html_prose_glyph_chords_resolve`] — the harvest for a bare
//!    mac-glyph chord mention OUTSIDE the table (`"⌘P"`, `"⌘⇧H"` in running
//!    prose). HARVESTING CONVENTION: a contiguous run of one-or-more Mac
//!    modifier glyphs (⌘ ⌥ ⇧ ⌃) immediately followed by an ASCII
//!    alphanumeric "key" run (letter/digit ONLY — see [`is_key_char`]'s doc
//!    for why awl's own punctuation-keyed chords like `⌘,` are deliberately
//!    excluded here) is treated as a cited chord label. This is the
//!    LEAST-FRAGILE convention available for a plain static HTML file (no
//!    build step to thread an explicit marker through, unlike the
//!    render-time `{{key:}}` seam): the four glyphs are drawn from a tiny,
//!    closed Unicode block that never otherwise appears in English prose, so
//!    the scan carries essentially ZERO false-positive risk — the exact fact
//!    `keytoken::tests::no_literal_chord_glyphs_survive_outside_tokens_and_
//!    the_generated_table` already leans on for the markdown docs. The valid
//!    set is derived from the SAME canonical rows (via the SAME harvester,
//!    applied to the canonical text) plus [`keytoken::synthetic_mac_glyphs`],
//!    so nothing here can drift from the table check above.
//!
//! Both checks are pure string scanning over checked-in files + the static
//! catalog — no clock, no filesystem beyond `include_str!` (a compile-time
//! embed, not a runtime read), no randomness.
//!
//! SIBLINGS CHECKED, NOT COVERED: `site/credits.html`, `site/index.html`, and
//! `site/check.html` were greped by hand for chord glyphs / `Ctrl+` word
//! forms / command-palette citations while building this law and carry NONE
//! today (`site/credits.html`/`CREDITS.md` carry the SAME kind of accepted
//! hand-mirror drift risk, just never for a chord or command name).
//! [`sibling_site_pages_carry_no_chord_glyphs`] pins that fact so a FUTURE
//! stray chord mention in one of them fails loudly instead of shipping
//! unverified — the same "grep-law" shape as the markdown docs' own glyph
//! ban. `site/editor/index.html` is the Trunk-generated wasm app shell (no
//! hand-written prose) and is out of scope entirely.
#![cfg(test)]

use std::collections::{HashMap, HashSet};

use crate::commands;

/// One keys-reference ROW: `(name, macOS label, Linux label)`, already
/// trimmed/decoded plain text — the shape both the canonical generator and
/// the HTML harvester produce, so they can be compared directly.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    name: String,
    mac: String,
    linux: String,
}

/// The CANONICAL rows: `commands::generate_keys_reference_markdown()`'s own
/// pipe-table, parsed back into `Row`s. This is the exact text
/// `guide::tests::generated_keys_reference_matches_catalog` already verifies
/// byte-for-byte against `GUIDE.md`'s checked-in table — reusing it here (
/// rather than re-deriving from `commands::COMMANDS` a second way) means a
/// mismatch can only ever be site/guide.html's fault, never a second,
/// possibly-diverging derivation of "the truth."
fn canonical_rows() -> Vec<Row> {
    let md = commands::generate_keys_reference_markdown();
    md.lines()
        .skip(2) // "| Command | macOS | Linux |" header + "|---|---|---|" separator
        .filter_map(|line| {
            let trimmed = line.trim();
            let cells: Vec<&str> = trimmed.trim_matches('|').split('|').map(str::trim).collect();
            match cells.as_slice() {
                [name, mac, linux] => Some(Row { name: name.to_string(), mac: mac.to_string(), linux: linux.to_string() }),
                _ => None,
            }
        })
        .collect()
}

/// Narrow HTML-entity decoder — deliberately not a general one, just the
/// handful that could plausibly appear in this specific table's cells.
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&nbsp;", " ")
}

/// Harvest the keys-reference `<table>`'s data rows out of `site/guide.html`
/// — the ONE table whose header cells are exactly `Command`/`macOS`/`Linux`
/// (the page carries a second table further down, the desktop/browser
/// feature comparison, which this marker deliberately does not match).
fn html_table_rows(html: &str) -> Vec<Row> {
    const HEADER: &str = "<th>Command</th><th>macOS</th><th>Linux</th>";
    const BODY_OPEN: &str = "<tbody>";
    const BODY_CLOSE: &str = "</tbody>";
    let Some(head) = html.find(HEADER) else { return Vec::new() };
    let after_head = &html[head..];
    let Some(body_start) = after_head.find(BODY_OPEN) else { return Vec::new() };
    let Some(body_end) = after_head.find(BODY_CLOSE) else { return Vec::new() };
    let body = &after_head[body_start + BODY_OPEN.len()..body_end];

    let mut rows = Vec::new();
    let mut rest = body;
    while let Some(tr_start) = rest.find("<tr>") {
        let after = &rest[tr_start + "<tr>".len()..];
        let Some(tr_end) = after.find("</tr>") else { break };
        let row_html = &after[..tr_end];
        let cells: Vec<String> = row_html
            .split("<td>")
            .skip(1)
            .map(|c| decode_entities(c.split("</td>").next().unwrap_or("").trim()))
            .collect();
        if let [name, mac, linux] = cells.as_slice() {
            rows.push(Row { name: name.clone(), mac: mac.clone(), linux: linux.clone() });
        }
        rest = &after[tr_end + "</tr>".len()..];
    }
    rows
}

/// The MAC-CONVENTION modifier glyphs: ⌘ (U+2318) ⌥ (U+2325) ⇧ (U+21E7)
/// ⌃ (U+2303). See the module doc for why a run of these is a safe harvest
/// anchor.
const MAC_GLYPHS: &[char] = &['\u{2318}', '\u{2325}', '\u{21E7}', '\u{2303}'];

/// Is `c` a plausible KEY character trailing a glyph run? ASCII alphanumeric
/// only — deliberately NOT awl's punctuation-keyed chords (`⌘,` Settings,
/// `⌘=`/`⌘-` zoom, `⌘⇧.` toggle hidden files, `⌘;` spell suggestions): every
/// one of those is cited ONLY inside the structured keys-reference table
/// (checked separately, exactly, by [`site_guide_html_keys_table_matches_
/// catalog`]), never as a bare prose mention — and a trailing comma/period is
/// indistinguishable from ordinary ENGLISH sentence punctuation right after a
/// parenthetical chord (`"⌘P, type..."`), which a punctuation-greedy key
/// charset would wrongly swallow into the token. Letters/digits carry no such
/// ambiguity.
fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// Harvest every `<glyph-run><key-run>` chord token from `text` (mac glyphs
/// immediately followed by one-or-more key characters, no space between —
/// exactly the shape [`crate::keyspec::mac_glyph_chord`] emits). Used BOTH to
/// harvest from `site/guide.html`'s prose AND to derive the valid-token set
/// from the canonical rows' own mac-column text (so both sides of the check
/// go through the identical scanner).
fn harvest_glyph_chords(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if MAC_GLYPHS.contains(&chars[i]) {
            let start = i;
            while i < chars.len() && MAC_GLYPHS.contains(&chars[i]) {
                i += 1;
            }
            let key_start = i;
            while i < chars.len() && is_key_char(chars[i]) {
                i += 1;
            }
            if i > key_start {
                out.push(chars[start..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The full set of Mac-glyph chord tokens the live catalog can actually
/// produce today: every glyph token inside a canonical row's `mac` column
/// (via [`harvest_glyph_chords`] — this naturally skips a row's terse emacs
/// half, like the `C-s` in `"⌘F · C-s"`, since it carries no mac glyph) plus
/// the two [`crate::keytoken::synthetic_mac_glyphs`] (command palette, stats
/// HUD) that have no catalog row to hang a column value on.
fn valid_mac_chord_tokens() -> HashSet<String> {
    let mut set: HashSet<String> = canonical_rows().iter().flat_map(|r| harvest_glyph_chords(&r.mac)).collect();
    set.extend(crate::keytoken::synthetic_mac_glyphs());
    set
}

/// THE TABLE LAW: `site/guide.html`'s own keys-reference table, row for row,
/// must match what the live catalog generates. A row citing a command name
/// the catalog no longer has (renamed/retired), or a chord cell that no
/// longer matches, fails with the offending command named. A command
/// present in the catalog but simply ABSENT from the HTML table is not a
/// violation — the page is a hand-curated subset, not required to mirror
/// every row; only what IS cited must be accurate (mirrors the docs-vs-
/// catalog law's own framing: citations must resolve, not "every command
/// must be cited").
#[test]
fn site_guide_html_keys_table_matches_catalog() {
    let canonical = canonical_rows();
    assert!(!canonical.is_empty(), "sanity: the generator produced no rows at all");
    let canon_by_slug: HashMap<String, &Row> = canonical.iter().map(|r| (commands::slug(&r.name), r)).collect();

    let harvested = html_table_rows(crate::embedded_docs::SITE_GUIDE_HTML);
    assert!(!harvested.is_empty(), "expected to find the keys-reference table in site/guide.html");

    let mut problems = Vec::new();
    for row in &harvested {
        let key = commands::slug(&row.name);
        match canon_by_slug.get(&key) {
            None => problems.push(format!(
                "site/guide.html cites {:?}, which no longer exists in the live command \
                 catalog (renamed or retired command — regenerate GUIDE.md's table and \
                 re-paste the row, or drop it)",
                row.name
            )),
            Some(canon) => {
                if canon.mac != row.mac {
                    problems.push(format!(
                        "site/guide.html: {:?}'s macOS chord is {:?}, but the live catalog \
                         resolves it to {:?}",
                        row.name, row.mac, canon.mac
                    ));
                }
                if canon.linux != row.linux {
                    problems.push(format!(
                        "site/guide.html: {:?}'s Linux chord is {:?}, but the live catalog \
                         resolves it to {:?}",
                        row.name, row.linux, canon.linux
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "site/guide.html's keys-reference table has drifted from the live catalog:\n{}",
        problems.join("\n")
    );
}

/// THE PROSE LAW: every bare mac-glyph chord mention in `site/guide.html`
/// (outside the table, checked separately above — this scan covers the whole
/// page, so it re-checks the table's own cells too, harmlessly) must match a
/// chord the live catalog actually produces. A stale/renamed chord glyph —
/// the exact "welcome-doc taught a retired chord" failure, one surface over —
/// fails here, naming the unresolved token(s).
#[test]
fn site_guide_html_prose_glyph_chords_resolve() {
    let valid = valid_mac_chord_tokens();
    let harvested = harvest_glyph_chords(crate::embedded_docs::SITE_GUIDE_HTML);
    assert!(!harvested.is_empty(), "expected at least one glyph chord mention in site/guide.html");

    let mut unknown: Vec<String> = harvested.into_iter().filter(|t| !valid.contains(t)).collect();
    unknown.sort();
    unknown.dedup();
    assert!(
        unknown.is_empty(),
        "site/guide.html cites a chord glyph that doesn't match any live catalog chord \
         (retired/renamed default?): {unknown:?}"
    );
}

/// THE SIBLINGS GREP-LAW: `site/credits.html` / `site/index.html` /
/// `site/check.html` carry NO chord-glyph or `Ctrl+`-word-form citations
/// today (verified by hand while building this law — see the module doc). A
/// future stray chord mention in one of them has no harvester to catch it
/// (unlike `site/guide.html`'s two checks above), so this pins the CURRENT
/// absence loudly: growing a chord mention in one of these pages must add it
/// to a real harvester (or extend this module's table/prose checks) rather
/// than silently shipping unverified, mirroring the markdown docs' own
/// `no_literal_chord_glyphs_survive_outside_tokens_and_the_generated_table`.
#[test]
fn sibling_site_pages_carry_no_chord_glyphs() {
    let credits = include_str!("../site/credits.html");
    let index = include_str!("../site/index.html");
    let check = include_str!("../site/check.html");
    for (name, text) in [("site/credits.html", credits), ("site/index.html", index), ("site/check.html", check)] {
        assert!(
            !text.chars().any(|c| MAC_GLYPHS.contains(&c)),
            "{name} now carries a mac chord glyph — extend this module's harvest-and-resolve \
             checks to cover it (see site_guide_html_prose_glyph_chords_resolve)"
        );
        assert!(
            !text.contains("Ctrl+"),
            "{name} now carries a literal Ctrl+ chord label — extend this module's \
             harvest-and-resolve checks to cover it"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harvest_glyph_chords_finds_every_token_and_skips_bare_prose() {
        let text = "The palette (\u{2318}P) and \u{2318}\u{21E7}H, but not the word command alone.";
        assert_eq!(harvest_glyph_chords(text), vec!["\u{2318}P".to_string(), "\u{2318}\u{21E7}H".to_string()]);
    }

    /// A punctuation-keyed chord (`⌘,` Settings, `⌘⇧.` toggle hidden files)
    /// is deliberately NOT captured by the prose scanner — see
    /// [`is_key_char`]'s doc for why (indistinguishable from a trailing
    /// English comma/period). These chords are only ever cited inside the
    /// structured table, which [`html_table_rows`] parses exactly instead.
    #[test]
    fn harvest_glyph_chords_ignores_trailing_sentence_punctuation() {
        assert_eq!(harvest_glyph_chords("Settings\u{2026} (\u{2318},)"), Vec::<String>::new());
        assert_eq!(harvest_glyph_chords("Toggle hidden files (\u{2318}\u{21E7}.)"), Vec::<String>::new());
        // The exact real-world case that motivated the restriction: a bare
        // chord immediately followed by a sentence comma, not part of the chord.
        assert_eq!(harvest_glyph_chords("\u{2318}P, type \"rename\", Enter."), vec!["\u{2318}P".to_string()]);
    }

    #[test]
    fn html_table_rows_parses_a_minimal_fixture() {
        let html = format!(
            "<table>\
               <thead><tr><th>Command</th><th>macOS</th><th>Linux</th></tr></thead>\
               <tbody>\
               <tr><td>Save</td><td>{glyph}S</td><td>Ctrl+S</td></tr>\
               <tr><td>Quit</td><td></td><td></td></tr>\
               </tbody>\
             </table>",
            glyph = '\u{2318}'
        );
        let rows = html_table_rows(&html);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Save");
        assert_eq!(rows[0].mac, "\u{2318}S");
        assert_eq!(rows[1].name, "Quit");
        assert_eq!(rows[1].mac, "");
    }

    #[test]
    fn decode_entities_handles_the_narrow_set() {
        assert_eq!(decode_entities("A &amp; B &lt;tag&gt; &quot;q&quot;&nbsp;x"), "A & B <tag> \"q\" x");
    }

    #[test]
    fn canonical_rows_is_nonempty_and_matches_row_count() {
        let rows = canonical_rows();
        assert_eq!(rows.len(), commands::COMMANDS.len());
    }
}
