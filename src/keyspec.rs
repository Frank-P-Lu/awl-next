//! Parse a headless `--keys` spec — a space-separated list of emacs chords —
//! into the same `Action`s the live keymap produces. Each chord is turned into a
//! winit `(Key, Modifiers)` and fed THROUGH `KeymapState::resolve`, so replay
//! goes down the exact dispatch table live keys use (including the `C-x` two-key
//! prefix state, which is why one persistent `KeymapState` drives the whole
//! sequence). An unrecognized chord is a clear `anyhow::Error`, never a panic.
//!
//! Grammar:
//!   spec  := chord (WS+ chord)*
//!   chord := mod* key                # mods: "C-" Ctrl, "M-" Meta/Alt,
//!                                    #       "S-" Shift, "s-" Super/Cmd. Word-form
//!                                    #       aliases also parse: "Cmd-"/"Super-",
//!                                    #       "Ctrl-", "Option-"/"Opt-"/"Alt-"/"Meta-",
//!                                    #       "Shift-" (case-insensitive).
//!   key   := <named> | <single printable char>
//!
//! Named keys (case-insensitive): Left Right Up Down Home End Enter/Return/RET
//! Tab Backspace/DEL Delete Space/SPC Esc/Escape. Anything else of length one is
//! a self-insert / literal char (case + shifted glyphs like `<` `>` pass through
//! verbatim, matching how the keymap reads them).

use anyhow::{bail, Result};
use winit::event::Modifiers;
use winit::keyboard::{Key, ModifiersState, NamedKey, SmolStr};

use crate::keymap::{Action, KeymapState};

/// Parse a whole `--keys` spec into the resolved `Action` stream. Chords are
/// split on ASCII whitespace and resolved in order through ONE `KeymapState`, so
/// prefix sequences like `C-x C-s` compose into a single `Save`. Two action
/// kinds are dropped because they carry no work for the apply seam: `Ignore`
/// (lone modifiers / unbound combos) and `BeginPrefix` (the FIRST half of a
/// `C-x` sequence, whose only job is to flip the keymap's prefix state — already
/// captured because we resolve through one persistent `KeymapState`). Dropping
/// both keeps the replay stream tight and the unit tests readable.
/// Parse a `--keys` spec with the DEFAULT keymap (no config overrides). Retained
/// as the simple entry point + the unit-test surface; `main` uses
/// [`parse_keys_with`] to honour config rebinds.
#[allow(dead_code)]
pub fn parse_keys(spec: &str) -> Result<Vec<Action>> {
    parse_keys_through(spec, KeymapState::new())
}

/// Like [`parse_keys`], but resolve through a keymap carrying the config `[keys]`
/// OVERRIDES, so a `--keys` replay exercises rebound chords exactly as live editing
/// would. `cfg` supplies the `(action-name, chord)` rebinds; with an empty config
/// this is identical to [`parse_keys`].
pub fn parse_keys_with(spec: &str, cfg: &crate::config::Config) -> Result<Vec<Action>> {
    parse_keys_through(spec, KeymapState::with_overrides(&cfg.keys))
}

/// Shared core: resolve every chord in `spec` through one persistent `km` (so C-x
/// prefix state and any config rebinds compose across the sequence).
fn parse_keys_through(spec: &str, mut km: KeymapState) -> Result<Vec<Action>> {
    let mut actions = Vec::new();
    for chord in spec.split_whitespace() {
        let (key, mods) = parse_chord(chord)?;
        let action = km.resolve(&key, &mods);
        if !matches!(action, Action::Ignore | Action::BeginPrefix) {
            actions.push(action);
        }
    }
    Ok(actions)
}

/// WORD-form modifier prefixes (case-insensitive), an alternative to the terse
/// single-letter `C-`/`M-`/`S-`/`s-` spellings so a macOS-native binding reads as
/// `Cmd-S` / `Option-f`. Each entry carries its trailing '-'; the longest natural
/// spellings come first but matching is unambiguous (each begins distinctly).
const WORD_MODS: &[(&str, ModifiersState)] = &[
    ("cmd-", ModifiersState::SUPER),
    ("super-", ModifiersState::SUPER),
    ("ctrl-", ModifiersState::CONTROL),
    ("control-", ModifiersState::CONTROL),
    ("option-", ModifiersState::ALT),
    ("opt-", ModifiersState::ALT),
    ("alt-", ModifiersState::ALT),
    ("meta-", ModifiersState::ALT),
    ("shift-", ModifiersState::SHIFT),
];

/// Parse a single chord (e.g. `C-x`, `M->`, `Cmd-S`, `Left`, `a`) into a winit key
/// event. Modifier prefixes — terse (`C-`) or word-form (`Cmd-`) — are stripped
/// greedily and order-independently (they are just bitflags); the remainder is the
/// key token.
pub fn parse_chord(chord: &str) -> Result<(Key, Modifiers)> {
    let mut rest = chord;
    let mut state = ModifiersState::empty();

    // Strip leading modifier prefixes. TWO spellings are accepted, greedily and
    // order-independently (modifiers are just bitflags): the terse single-letter
    // "<m>-" form (`C-`, `M-`, `S-`, `s-`) AND the macOS-friendly WORD form
    // (`Cmd-`, `Option-`, ...). The word form is tried first so `Cmd-S` reads as
    // Super+`S` rather than as the literal letters. A bare "-" (or a 1-char
    // remainder) is never consumed as a prefix, so the literal key always survives.
    loop {
        // Word-form modifiers (case-insensitive). Each `pfx` includes its trailing
        // '-'; `len() > pfx.len()` guarantees at least one key char remains after it.
        if let Some((pfx, flag)) = WORD_MODS.iter().find(|(pfx, _)| {
            rest.len() > pfx.len() && rest.get(..pfx.len()).is_some_and(|h| h.eq_ignore_ascii_case(pfx))
        }) {
            state |= *flag;
            rest = &rest[pfx.len()..];
            continue;
        }
        let bytes = rest.as_bytes();
        if rest.len() >= 2 && bytes[1] == b'-' {
            let flag = match bytes[0] {
                b'C' => Some(ModifiersState::CONTROL),
                b'M' => Some(ModifiersState::ALT), // Meta == Alt (Option on mac)
                b'S' => Some(ModifiersState::SHIFT),
                b's' => Some(ModifiersState::SUPER), // Super == Cmd
                _ => None,
            };
            if let Some(f) = flag {
                state |= f;
                rest = &rest[2..];
                continue;
            }
        }
        break;
    }

    if rest.is_empty() {
        bail!("empty key in chord {chord:?}");
    }

    let key = parse_key_token(rest, chord)?;
    Ok((key, Modifiers::from(state)))
}

/// Format a `(Key, ModifiersState)` back into a CANONICAL terse chord string —
/// the inverse of [`parse_chord`], used by the REBIND MENU to turn a captured key
/// press into a config-storable, displayable spec (`"C-t"`, `"M-f"`, `"s-S-z"`,
/// `"Left"`). Modifiers emit in a FIXED order (`C- M- S- s-`) so two presses of the
/// same combo always produce the same string; a single ASCII letter is lower-cased
/// (the keymap folds case via `canon_key`), every other char passes through.
pub fn format_chord(key: &Key, mods: ModifiersState) -> String {
    let mut s = String::new();
    if mods.contains(ModifiersState::CONTROL) {
        s.push_str("C-");
    }
    if mods.contains(ModifiersState::ALT) {
        s.push_str("M-");
    }
    if mods.contains(ModifiersState::SHIFT) {
        s.push_str("S-");
    }
    if mods.contains(ModifiersState::SUPER) {
        s.push_str("s-");
    }
    s.push_str(&key_token(key));
    s
}

/// Format a chord spec with mac MODIFIER GLYPHS concatenated, no dashes — the
/// NATIVE (macOS) slot's display form: Cmd→⌘ (U+2318), Shift→⇧ (U+21E7),
/// Option/Meta→⌥ (U+2325), Ctrl→⌃ (U+2303), then the key (single letters
/// upper-cased so `Cmd-S` reads `⌘S`). `"Cmd-S-o"` → `"⌘⇧O"`, `"Cmd-F"` → `"⌘F"`,
/// `"Cmd-M-f"` → `"⌘⌥F"`. A whitespace-separated SEQUENCE formats each chord and
/// re-joins with a space; a token that fails to parse passes through verbatim. The
/// EMACS slot does NOT use this — it keeps its terse `C-`/`M-` text.
pub fn mac_glyph_chord(spec: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for tok in spec.split_whitespace() {
        match parse_chord(tok) {
            Ok((key, mods)) => out.push(mac_glyph_token(&key, mods.state())),
            Err(_) => out.push(tok.to_string()),
        }
    }
    out.join(" ")
}

/// One chord as mac glyphs: the modifier glyphs (⌘ ⇧ ⌥ ⌃, in that order) then the
/// key glyph. Helper for [`mac_glyph_chord`].
fn mac_glyph_token(key: &Key, mods: ModifiersState) -> String {
    let mut s = String::new();
    if mods.contains(ModifiersState::SUPER) {
        s.push('\u{2318}'); // ⌘ Command
    }
    if mods.contains(ModifiersState::SHIFT) {
        s.push('\u{21E7}'); // ⇧ Shift
    }
    if mods.contains(ModifiersState::ALT) {
        s.push('\u{2325}'); // ⌥ Option
    }
    if mods.contains(ModifiersState::CONTROL) {
        s.push('\u{2303}'); // ⌃ Control
    }
    s.push_str(&mac_key_token(key));
    s
}

/// The key glyph for the mac-glyph form: a single ASCII letter UPPER-cased (so
/// `⌘S` not `⌘s`); everything else reuses [`key_token`] (named keys keep their
/// spelling, symbol keys their glyph).
fn mac_key_token(key: &Key) -> String {
    if let Key::Character(s) = key {
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_alphabetic() {
                return c.to_ascii_uppercase().to_string();
            }
        }
    }
    key_token(key)
}

/// The bare key TOKEN for [`format_chord`] (no modifiers): a named key maps back to
/// its canonical spelling (`Left`, `Enter`, `Esc`, …), and a character key is its
/// glyph (single ASCII letters lower-cased so `C-T` and `C-t` agree).
fn key_token(key: &Key) -> String {
    match key {
        Key::Named(named) => match named {
            NamedKey::ArrowLeft => "Left",
            NamedKey::ArrowRight => "Right",
            NamedKey::ArrowUp => "Up",
            NamedKey::ArrowDown => "Down",
            NamedKey::Home => "Home",
            NamedKey::End => "End",
            NamedKey::Enter => "Enter",
            NamedKey::Tab => "Tab",
            NamedKey::Backspace => "Backspace",
            NamedKey::Delete => "Delete",
            NamedKey::Space => "Space",
            NamedKey::Escape => "Esc",
            _ => "?",
        }
        .to_string(),
        Key::Character(s) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => c.to_ascii_lowercase().to_string(),
                _ => s.to_string(),
            }
        }
        _ => String::new(),
    }
}

/// Canonicalise a CHORD SPEC (one chord or a `C-x <key>` sequence) into the stable
/// terse form [`format_chord`] emits, or `None` if any token fails to parse. Two
/// specs that mean the same chord (`"Cmd-S"` / `"s-s"`, `"C-x C-f"`) canonicalise
/// to the SAME string, so the rebind menu can compare bindings for CONFLICTS and
/// store a normalized value. Whitespace-separated tokens are canonicalised
/// individually and re-joined with a single space.
pub fn canonical_binding(spec: &str) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in spec.split_whitespace() {
        let (key, mods) = parse_chord(tok).ok()?;
        out.push(format_chord(&key, mods.state()));
    }
    if out.is_empty() {
        return None;
    }
    Some(out.join(" "))
}

/// Map the key token (after modifier stripping) to a winit `Key`. Named keys are
/// matched case-insensitively against a fixed table; otherwise the token must be
/// exactly one character, passed through verbatim as a `Character`.
fn parse_key_token(tok: &str, chord: &str) -> Result<Key> {
    if let Some(named) = named_key(tok) {
        return Ok(Key::Named(named));
    }
    // A single printable char is a self-insert / literal. Pass it through
    // verbatim (no case folding) so `Z` stays `Z` and shifted glyphs like `<`
    // `>` keep working in the keymap's Meta arms.
    if tok.chars().count() != 1 {
        bail!("unrecognized key {tok:?} in chord {chord:?} (not a named key or single char)");
    }
    Ok(Key::Character(SmolStr::new(tok)))
}

/// Named-key table (case-insensitive on the token). Returns `None` for tokens
/// that are not named keys, so the caller falls through to single-char handling.
fn named_key(tok: &str) -> Option<NamedKey> {
    // Case-insensitive compare without allocating for the common (already-cased)
    // tokens: only fold when the raw token misses.
    let lower = tok.to_ascii_lowercase();
    Some(match lower.as_str() {
        "left" => NamedKey::ArrowLeft,
        "right" => NamedKey::ArrowRight,
        "up" => NamedKey::ArrowUp,
        "down" => NamedKey::ArrowDown,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "enter" | "return" | "ret" => NamedKey::Enter,
        "tab" => NamedKey::Tab,
        "backspace" | "del" => NamedKey::Backspace,
        "delete" => NamedKey::Delete,
        "space" | "spc" => NamedKey::Space,
        "esc" | "escape" => NamedKey::Escape,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_motions_sequence() {
        assert_eq!(
            parse_keys("C-n C-n").unwrap(),
            vec![Action::NextLine, Action::NextLine]
        );
    }

    #[test]
    fn c_x_prefix_save() {
        assert_eq!(parse_keys("C-x C-s").unwrap(), vec![Action::Save]);
    }

    #[test]
    fn self_insert_two_chars() {
        assert_eq!(
            parse_keys("a b").unwrap(),
            vec![Action::InsertChar('a'), Action::InsertChar('b')]
        );
    }

    #[test]
    fn meta_buffer_end() {
        assert_eq!(parse_keys("M->").unwrap(), vec![Action::BufferEnd]);
    }

    #[test]
    fn meta_buffer_start() {
        assert_eq!(parse_keys("M-<").unwrap(), vec![Action::BufferStart]);
    }

    #[test]
    fn parse_keys_with_honours_config_rebind() {
        use crate::config::Config;
        // The config-rebind replay path `main` actually uses: a `[keys]` override
        // resolves a chord to the rebound Action while the spec still splits/prefixes
        // correctly. The default (no-override) path must NOT produce that Action.
        let mut cfg = Config::empty();
        cfg.keys.push(("toggle_fps".into(), vec!["C-j".into()]));
        assert_eq!(parse_keys_with("C-j", &cfg).unwrap(), vec![Action::ToggleFps]);
        assert_ne!(parse_keys("C-j").unwrap(), vec![Action::ToggleFps], "default C-j is not ToggleFps");
        // An empty config makes parse_keys_with identical to parse_keys, including a
        // C-x prefix sequence.
        let empty = Config::empty();
        for spec in ["C-n C-n", "C-x C-s", "a b"] {
            assert_eq!(
                parse_keys_with(spec, &empty).unwrap(),
                parse_keys(spec).unwrap(),
                "empty config == default for {spec:?}"
            );
        }
    }

    #[test]
    fn word_form_modifiers_parse_like_terse() {
        // The macOS-native word spellings resolve to the SAME chords as the terse
        // single-letter forms, so a config `Cmd-S` reaches the keymap as Super+S.
        let pairs = [("Cmd-s", "s-s"), ("Option-f", "M-f"), ("Ctrl-x", "C-x"), ("Cmd-S-z", "s-S-z")];
        for (word, terse) in pairs {
            assert_eq!(
                parse_chord(word).map(|(k, m)| (k, m.state())).unwrap(),
                parse_chord(terse).map(|(k, m)| (k, m.state())).unwrap(),
                "{word:?} should parse like {terse:?}"
            );
        }
        // Case-insensitive on the word, and "Cmd--" is Super + the literal '-' key.
        assert_eq!(parse_chord("CMD-=").unwrap().1.state(), ModifiersState::SUPER);
        let (k, m) = parse_chord("Cmd--").unwrap();
        assert_eq!(m.state(), ModifiersState::SUPER);
        assert_eq!(k, Key::Character(SmolStr::new("-")));
    }

    #[test]
    fn format_chord_round_trips_through_parse() {
        // A captured key press → canonical terse spec, in the FIXED modifier order,
        // that parses back to the SAME (key, mods). Covers letter-folding, named
        // keys, and the macOS `s-`/`M-` modifiers.
        for spec in ["C-t", "M-f", "s-s", "Left", "Enter", "Esc", "s-S-z", "C-x", "="] {
            let (k, m) = parse_chord(spec).unwrap();
            let formatted = format_chord(&k, m.state());
            let (k2, m2) = parse_chord(&formatted).unwrap();
            assert_eq!((canon(&k2), m2.state()), (canon(&k), m.state()), "{spec:?} → {formatted:?}");
        }
        // A shifted/upper letter folds to lower-case; the modifier order is fixed.
        assert_eq!(format_chord(&ch_key("T"), ModifiersState::CONTROL), "C-t");
        // Modifiers emit in the FIXED order C- M- S- s- (so Cmd-Shift-Z → "S-s-z").
        assert_eq!(
            format_chord(&ch_key("z"), ModifiersState::SUPER | ModifiersState::SHIFT),
            "S-s-z"
        );
    }

    #[test]
    fn mac_glyph_chord_renders_modifier_glyphs() {
        // The NATIVE slot's display form: mac modifier glyphs concatenated, no dashes,
        // the key upper-cased. Cmd→⌘ Shift→⇧ Option/Meta→⌥ Ctrl→⌃.
        assert_eq!(mac_glyph_chord("Cmd-S-o"), "⌘⇧O");
        assert_eq!(mac_glyph_chord("Cmd-F"), "⌘F");
        assert_eq!(mac_glyph_chord("Cmd-M-f"), "⌘⌥F"); // Replace: Cmd-Option-F
        assert_eq!(mac_glyph_chord("Cmd-S"), "⌘S"); // the trailing S is the KEY
        assert_eq!(mac_glyph_chord("Cmd-S-z"), "⌘⇧Z"); // Redo
        assert_eq!(mac_glyph_chord("Cmd-;"), "⌘;");
        assert_eq!(mac_glyph_chord("Cmd-="), "⌘=");
        assert_eq!(mac_glyph_chord("C-t"), "⌃T"); // a Ctrl chord → the ⌃ glyph
        // The terse `s-` super form glyphifies identically to the word form.
        assert_eq!(mac_glyph_chord("s-s"), mac_glyph_chord("Cmd-S"));
        // An unparseable token passes through verbatim (never panics).
        assert_eq!(mac_glyph_chord("C-frobnicate"), "C-frobnicate");
    }

    #[test]
    fn canonical_binding_unifies_equivalent_specs() {
        // The word-form and terse spellings of one chord canonicalise identically,
        // and a `C-x <key>` sequence canonicalises token-by-token.
        assert_eq!(canonical_binding("Cmd-S"), canonical_binding("s-s"));
        assert_eq!(canonical_binding("C-x C-f").as_deref(), Some("C-x C-f"));
        assert_eq!(canonical_binding("Ctrl-t").as_deref(), Some("C-t"));
        // A garbled token yields None (no panic), so a bad capture can't conflict.
        assert_eq!(canonical_binding("C-frobnicate"), None);
        assert_eq!(canonical_binding("   "), None);
    }

    fn ch_key(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    fn canon(k: &Key) -> Key {
        match k {
            Key::Character(s) => Key::Character(SmolStr::new(s.to_lowercase())),
            other => other.clone(),
        }
    }

    #[test]
    fn unknown_chord_errors() {
        // A multi-char token that is not a named key is an error, not a panic.
        assert!(parse_keys("frobnicate").is_err());
    }

    #[test]
    fn named_keys_and_modifiers() {
        assert_eq!(
            parse_keys("Left Right").unwrap(),
            vec![Action::BackwardChar, Action::ForwardChar]
        );
        // Alt+Right is word motion.
        assert_eq!(parse_keys("M-Right").unwrap(), vec![Action::ForwardWord]);
        // Enter / Tab / Backspace / Delete named keys.
        assert_eq!(
            parse_keys("Enter Tab Backspace Delete").unwrap(),
            vec![
                Action::Newline,
                Action::InsertTab,
                Action::DeleteBackward,
                Action::DeleteForward,
            ]
        );
    }

    #[test]
    fn c_space_sets_mark_then_motion() {
        // C-Space is a NAMED key (Space) + Ctrl in the keymap.
        assert_eq!(
            parse_keys("C-Space C-f").unwrap(),
            vec![Action::SetMark, Action::ForwardChar]
        );
    }

    #[test]
    fn shifted_literal_self_inserts() {
        // A bare '<' with no modifier is a literal self-insert.
        assert_eq!(parse_keys("<").unwrap(), vec![Action::InsertChar('<')]);
    }

    #[test]
    fn case_preserved_on_self_insert() {
        assert_eq!(
            parse_keys("Z").unwrap(),
            vec![Action::InsertChar('Z')]
        );
    }

    #[test]
    fn save_and_quit_via_prefix() {
        assert_eq!(
            parse_keys("C-x C-s C-x C-c").unwrap(),
            vec![Action::Save, Action::Quit]
        );
    }

    #[test]
    fn empty_spec_is_empty() {
        assert_eq!(parse_keys("   ").unwrap(), Vec::<Action>::new());
    }
}
