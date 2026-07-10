//! src/credits.rs — the embedded CREDITS.md text, `include_str!`'d at build
//! time (zero network, mirroring every other bundled asset in this app).
//!
//! Summoned via Cmd-P → "Credits" (`Action::OpenCredits` / `Effect::OpenCredits`,
//! see `commands.rs` + `actions.rs`), which opens this text into the buffer
//! exactly like Settings opens the config file — see `App::open_credits`
//! (`app/files.rs`) for why it is written to a real on-disk path (under
//! `fs::data_root()`, refreshed to the embedded text on every open) rather than
//! left path-less: a path-less buffer is indistinguishable from the SCRATCH
//! surface to the autosave engine (`App::autosave_flush`'s `buffer.path().is_none()`
//! arm stashes it as scratch), which would silently clobber the user's real
//! scratch stash the next time autosave flushes. Routing through a real path
//! keeps Credits an ordinary, harmlessly-editable buffer instead.

/// The full text of the repo's `CREDITS.md`, embedded at compile time.
pub const CREDITS_MD: &str = include_str!("../CREDITS.md");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_text_is_nonempty_and_mentions_the_license_door() {
        assert!(!CREDITS_MD.is_empty());
        assert!(CREDITS_MD.contains("GPL-3.0"));
        assert!(CREDITS_MD.contains("THIRD-PARTY-LICENSES"));
    }
}
