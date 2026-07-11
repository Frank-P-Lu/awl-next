//! CHORD TOKENS — the CONVENTION-TRUTHFUL SURFACES round's write side for the
//! starting docs (`samples/welcome.md`, `samples/tour.md`, `GUIDE.md`).
//!
//! A literal chord glyph baked into a doc (`⌘P`) is a LIE the instant it's read
//! under a different convention or platform — a Linux visitor sees a mac glyph
//! that doesn't fire; a web visitor on a browser-reserved chord (New note,
//! Switch theme…) sees a chord that's silently eaten by the browser chrome
//! before the page ever gets it. `{{key:slug}}` is the fix: a token substituted
//! at the RIGHT moment for each surface (seed time for welcome/tour — see
//! `fs::seed_write_if_absent` — open time for GUIDE.md — see `App::open_guide` /
//! `main::run`'s headless arm) through the SAME truthful label owner every other
//! chord surface reads (`commands::resolved_native_label_truthful`), so a doc
//! can never show a chord that doesn't actually fire.
//!
//! `slug` is a catalog command's own config slug (`commands::slug`) for the vast
//! majority of tokens; [`SYNTHETIC`] covers the two chords that are real,
//! fixed, non-rebindable presses with NO catalog row at all (the command
//! palette's own dedicated Cmd-P, the held stats HUD's Option-Cmd-I) — both are
//! the most-taught chords in the onboarding docs, so they still need a token
//! even though `commands::COMMANDS` has no entry to hang it on.

use crate::commands::{self, Platform};
use crate::convention::Convention;

/// The token delimiters: `{{key:` ... `}}`.
const OPEN: &str = "{{key:";
const CLOSE: &str = "}}";

/// Non-catalog synthetic slugs: `(slug, mac spec, linux spec)`, each spec in
/// the same terse form [`crate::keyspec::parse_chord`] accepts. See the module
/// doc for why these two exist outside the catalog.
const SYNTHETIC: &[(&str, &str, &str)] = &[
    // The command palette: its own dedicated Cmd-P/Ctrl-P, matched directly in
    // `keymap.rs::resolve` (never a catalog row, never rebindable via `[keys]`).
    ("command_palette", "Cmd-P", "C-p"),
    // The held stats HUD: Option-Cmd-I / Ctrl-Alt-I, matched directly in
    // `keymap.rs::resolve_named`'s `native && alt` arm (deliberately NOT a
    // catalog row — see `commands.rs`'s "Held stats HUD" doc: a discrete
    // palette selection has no key-release to dismiss a hold-only panel with).
    ("stats_hud", "Cmd-M-i", "C-M-i"),
];

/// Resolve `slug_want`'s chord LABEL for `convention`+`platform`: first the
/// catalog, through [`commands::resolved_native_label_truthful`] (the ONE
/// owner every other chord surface reads — a token can therefore never show a
/// chord that doesn't actually fire, web-reserved/Linux-displaced/web-alternate
/// included), then [`SYNTHETIC`] for the two dedicated chords with no catalog
/// row. `None` for an unknown slug.
pub fn key_token_label(slug_want: &str, convention: Convention, platform: Platform) -> Option<String> {
    if let Some(c) = commands::COMMANDS.iter().find(|c| commands::slug(c.name) == slug_want) {
        return Some(commands::resolved_native_label_truthful(c, convention, platform));
    }
    SYNTHETIC.iter().find(|(s, _, _)| *s == slug_want).map(|(_, mac, linux)| match convention {
        Convention::Mac => crate::keyspec::mac_glyph_chord(mac),
        Convention::Linux => crate::keyspec::linux_glyph_chord(linux),
    })
}

/// Replace every `{{key:slug}}` token in `text` with [`key_token_label`]'s
/// resolved chord for `convention`+`platform`. An UNKNOWN slug is left as a
/// visible `[[unknown-key:slug]]` marker — never panics, never silently
/// vanishes — so a typo'd token is obvious in the rendered doc even outside
/// the build-time law test (`tests::every_key_token_in_the_starting_docs_resolves`)
/// that actually guards against one ever shipping. An unterminated `{{key:`
/// (missing `}}`) is likewise left verbatim rather than eating the rest of the
/// document.
pub fn render_key_tokens(text: &str, convention: Convention, platform: Platform) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        match after.find(CLOSE) {
            Some(end) => {
                let slug_want = &after[..end];
                match key_token_label(slug_want, convention, platform) {
                    Some(label) => out.push_str(&label),
                    None => out.push_str(&format!("[[unknown-key:{slug_want}]]")),
                }
                rest = &after[end + CLOSE.len()..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const WELCOME: &str = include_str!("../samples/welcome.md");
    const TOUR: &str = include_str!("../samples/tour.md");
    const GUIDE: &str = include_str!("../GUIDE.md");

    fn extract_token_slugs(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find(OPEN) {
            let after = &rest[start + OPEN.len()..];
            match after.find(CLOSE) {
                Some(end) => {
                    out.push(after[..end].to_string());
                    rest = &after[end + CLOSE.len()..];
                }
                None => break,
            }
        }
        out
    }

    fn strip_generated_table(doc: &str) -> String {
        const BEGIN: &str = "<!-- GENERATED:keys-reference:BEGIN -->";
        const END: &str = "<!-- GENERATED:keys-reference:END -->";
        match (doc.find(BEGIN), doc.find(END)) {
            (Some(s), Some(e)) => format!("{}{}", &doc[..s], &doc[e + END.len()..]),
            _ => doc.to_string(),
        }
    }

    #[test]
    fn render_key_tokens_substitutes_a_known_slug() {
        let out = render_key_tokens("press {{key:save}} to save", Convention::Mac, Platform::Native);
        assert_eq!(out, "press \u{2318}S to save");
    }

    #[test]
    fn render_key_tokens_flags_an_unknown_slug_rather_than_vanishing() {
        let out = render_key_tokens("{{key:nope}}", Convention::Mac, Platform::Native);
        assert_eq!(out, "[[unknown-key:nope]]");
    }

    #[test]
    fn render_key_tokens_leaves_an_unterminated_token_verbatim() {
        let out = render_key_tokens("abc {{key:save no close", Convention::Mac, Platform::Native);
        assert_eq!(out, "abc {{key:save no close");
    }

    #[test]
    fn synthetic_tokens_resolve_on_both_conventions() {
        assert_eq!(key_token_label("command_palette", Convention::Mac, Platform::Native).as_deref(), Some("\u{2318}P"));
        assert_eq!(key_token_label("command_palette", Convention::Linux, Platform::Native).as_deref(), Some("Ctrl+P"));
        assert!(key_token_label("stats_hud", Convention::Mac, Platform::Native).is_some());
        assert!(key_token_label("stats_hud", Convention::Linux, Platform::Native).is_some());
    }

    /// THE ONE LAW that guards `render_key_tokens`'s unknown-slug fallback from
    /// ever actually shipping: every `{{key:slug}}` token used in the STARTING
    /// docs must resolve, on every convention x platform combination.
    #[test]
    fn every_key_token_in_the_starting_docs_resolves() {
        for (name, doc) in [("welcome.md", WELCOME), ("tour.md", TOUR), ("GUIDE.md", GUIDE)] {
            let slugs = extract_token_slugs(doc);
            assert!(!slugs.is_empty(), "{name}: expected at least one {{{{key:..}}}} token");
            for slug_want in slugs {
                for convention in [Convention::Mac, Convention::Linux] {
                    for platform in [Platform::Native, Platform::Web] {
                        assert!(
                            key_token_label(&slug_want, convention, platform).is_some(),
                            "{name}: unknown key token slug {slug_want:?} under {convention:?}/{platform:?}"
                        );
                    }
                }
            }
        }
    }

    /// THE GREP-LAW: no literal chord glyph survives in the STARTING docs'
    /// PROSE — every specific chord mention must be a `{{key:slug}}` token, so
    /// it can never silently drift from what actually fires. Two literal forms
    /// are banned: the mac ⌘ glyph, and the generated table's own `Ctrl+<key>`
    /// word form. Exempted: the ONE generated keys-reference fence in
    /// GUIDE.md (dual-column BY DESIGN — see `commands::generate_keys_
    /// reference_markdown`'s doc) and a short, individually curated allowlist
    /// of lines that talk ABOUT the two-convention split itself (both
    /// conventions named explicitly in the SAME breath, so there is no single
    /// per-viewer truth a token could substitute without erasing the other).
    #[test]
    fn no_literal_chord_glyphs_survive_outside_tokens_and_the_generated_table() {
        // Curated, honest allowlist: lines that name BOTH conventions in the
        // same breath (never claim a single truth a token could replace).
        const ALLOWED_MAC_GLYPH_SUBSTRINGS: &[&str] = &[
            "**slot 1 is native** (\u{2318} on",
            "**The hold-\u{2318} peek.** Hold the arming modifier alone for a beat (\u{2318} on",
        ];
        // The Omarchy/Hyprland recipe: `Ctrl+C/X/V` here names the LITERAL
        // signal the compositor forwards to every app (a hardware/OS fact,
        // not an awl chord label) — explicitly out of scope, per the round's
        // own "curate honestly" instruction.
        const ALLOWED_CTRL_WORD_SUBSTRINGS: &[&str] = &["as Ctrl+C/X/V for the system clipboard"];
        for (name, doc) in [("welcome.md", WELCOME), ("tour.md", TOUR), ("GUIDE.md", GUIDE)] {
            let body = strip_generated_table(doc);
            for line in body.lines() {
                if line.contains('\u{2318}') && !ALLOWED_MAC_GLYPH_SUBSTRINGS.iter().any(|a| line.contains(a)) {
                    panic!("{name}: literal \u{2318} glyph outside a token/allowlist: {line:?}");
                }
                if line.contains("Ctrl+") && !ALLOWED_CTRL_WORD_SUBSTRINGS.iter().any(|a| line.contains(a)) {
                    panic!("{name}: literal Ctrl+ word-form outside the generated table/allowlist: {line:?}");
                }
            }
        }
    }
}
