//! src/guide.rs — the embedded GUIDE.md text, `include_str!`'d at build time
//! (zero network, mirroring `credits.rs`'s exact pattern).
//!
//! Summoned via Cmd-P → "Guide" (`Action::OpenGuide` / `Effect::OpenGuide`, see
//! `commands.rs` + `actions.rs`), which opens this text into the buffer exactly
//! like Credits opens `CREDITS.md` — see `App::open_guide` (`app/files.rs`) for
//! why it is written to a real on-disk path (under `fs::data_root()`, refreshed
//! to the embedded text on every open) rather than left path-less: a path-less
//! buffer is indistinguishable from the SCRATCH surface to the autosave engine
//! (`App::autosave_flush`'s `buffer.path().is_none()` arm stashes it as scratch),
//! which would silently clobber the user's real scratch stash the next time
//! autosave flushes. Routing through a real path keeps Guide an ordinary,
//! harmlessly-editable buffer instead.
//!
//! **The generated keys reference (the drift-proof centerpiece):** GUIDE.md's
//! "Keys" section carries a fenced table, between
//! `<!-- GENERATED:keys-reference:BEGIN -->` / `<!-- GENERATED:keys-reference:END
//! -->` markers, listing every catalog command's resolved default chord under
//! each convention (mac glyphs / Linux words). That table is produced by
//! `commands::generate_keys_reference_markdown` (a `#[cfg(test)]` generator, since
//! it is only ever needed to REGENERATE the checked-in doc, never at runtime) and
//! is never hand-typed. [`tests::generated_keys_reference_matches_catalog`] is the
//! LAW TEST: it re-runs the generator against the LIVE catalog and diffs the
//! result byte-for-byte against what's checked into `GUIDE.md` — a catalog change
//! (a new command, a changed default chord) fails this test until the doc is
//! regenerated and pasted back in, so the shipped reference can never silently
//! drift from what the app actually does.

/// The full text of the repo's `GUIDE.md`, embedded at compile time.
pub const GUIDE_MD: &str = include_str!("../GUIDE.md");

#[cfg(test)]
mod tests {
    use super::*;

    const BEGIN: &str = "<!-- GENERATED:keys-reference:BEGIN -->";
    const END: &str = "<!-- GENERATED:keys-reference:END -->";

    /// Slice out exactly the checked-in generated table (the text strictly
    /// between the two markers, trimmed of the single blank line on each side
    /// the markdown fencing naturally carries) — the ONE extraction both the law
    /// test and a future maintainer reading this file should trust.
    fn extract_generated_section(md: &str) -> &str {
        let start = md.find(BEGIN).expect("BEGIN marker present in GUIDE.md") + BEGIN.len();
        let end = md.find(END).expect("END marker present in GUIDE.md");
        md[start..end].trim_matches('\n')
    }

    #[test]
    fn guide_text_is_nonempty_and_mentions_the_doors() {
        assert!(!GUIDE_MD.is_empty());
        // The doors a docs writer actually reaches for: the palette, the
        // config file, the notes model's verbs, the theme picker.
        assert!(GUIDE_MD.contains("Settings"));
        assert!(GUIDE_MD.contains("Rename note"));
        assert!(GUIDE_MD.contains("notes_root"));
        assert!(GUIDE_MD.to_lowercase().contains("wysiwyg"));
    }

    /// THE CENTERPIECE: GUIDE.md's checked-in keys-reference table must be
    /// byte-identical to what the live catalog generates RIGHT NOW. A drift
    /// (new command, changed default chord, a Linux-displacement change) fails
    /// this test with a clear diff and a regeneration pointer, rather than
    /// letting the shipped doc quietly lie about what the app does.
    #[test]
    fn generated_keys_reference_matches_catalog() {
        let checked_in = extract_generated_section(GUIDE_MD);
        let fresh = crate::commands::generate_keys_reference_markdown();
        let fresh = fresh.trim_matches('\n');
        assert_eq!(
            checked_in, fresh,
            "GUIDE.md's generated keys reference has drifted from the live \
             command catalog — regenerate with `cargo test --bin awl \
             guide::tests::print_generated_keys_reference -- --ignored \
             --nocapture` and paste the printed table between the \
             <!-- GENERATED:keys-reference:BEGIN/END --> markers, byte for byte."
        );
    }

    /// Not a real test — a REGENERATION TOOL, run explicitly (`--ignored`) and
    /// read via stdout (`--nocapture`). Prints exactly what belongs between the
    /// markers in `GUIDE.md`.
    #[test]
    #[ignore]
    fn print_generated_keys_reference() {
        print!("{}", crate::commands::generate_keys_reference_markdown());
    }
}
