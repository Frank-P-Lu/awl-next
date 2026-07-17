//! Parse a headless `--keys` spec — a space-separated list of emacs chords —
//! into the winit `(Key, Modifiers)` CHORD stream the replay loop consumes
//! ([`parse_chords`] → [`Chord`]), and resolve each chord through the REAL
//! keymap one press at a time ([`ChordResolver`] wraps `KeymapState::resolve`,
//! including the `C-x` two-key prefix state — one persistent keymap drives the
//! whole sequence). Parsing and resolution are deliberately SEPARATE steps:
//! the replay loop interleaves the search guard between them, exactly like
//! live key dispatch, so a chord the open search panel consumes never reaches
//! the keymap. An unrecognized token is a clear `anyhow::Error`, never a panic.
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
//! Named keys (case-insensitive): Left Right Up Down Home End PageUp/PgUp
//! PageDown/PgDn Enter/Return/RET Tab Backspace/DEL Delete Space/SPC Esc/Escape.
//! Anything else of length one is
//! a self-insert / literal char (case + shifted glyphs like `<` `>` pass through
//! verbatim, matching how the keymap reads them).

use anyhow::{bail, Result};
use winit::event::Modifiers;
use winit::keyboard::{Key, ModifiersState, NamedKey, SmolStr};

use crate::keymap::{Action, KeymapState};

/// One parsed `--keys` CHORD — the raw token exactly as the spec spelled it
/// (kept so a strict refusal can name the offender verbatim) plus the winit
/// key + modifiers every downstream seam consumes. This is the replay's
/// CURRENCY: chords stay UNRESOLVED until the replay loop feeds them through
/// [`ChordResolver`] one press at a time, mirroring the live window's
/// per-press `KeymapState::resolve` — which is what lets the SEARCH GUARD
/// (`crate::search::keys::intercept`) consume a chord BEFORE the keymap ever
/// sees it, the live guard's exact position. A pre-resolved `Action` stream
/// could not express "the open panel ate this key" (`M-c` would drop as
/// unbound, `C-x` would falsely arm the prefix).
#[derive(Clone, Debug)]
pub struct Chord {
    /// The raw token as written in the spec (`"C-s"`, `"Cmd-S-f"`, `"h"`).
    pub spec: String,
    pub key: Key,
    pub mods: Modifiers,
}

/// Parse a whole `--keys` spec into its CHORD stream — STRUCTURAL validation
/// only (modifier prefixes + key tokens; an unrecognized token is a clear
/// `anyhow::Error`, never a panic). No keymap is consulted here: resolution
/// happens chord-by-chord inside the replay loop (see [`ChordResolver`]),
/// interleaved with the search guard, exactly like live key dispatch.
pub fn parse_chords(spec: &str) -> Result<Vec<Chord>> {
    spec.split_whitespace()
        .map(|tok| {
            let (key, mods) = parse_chord(tok)?;
            Ok(Chord { spec: tok.to_string(), key, mods })
        })
        .collect()
}

/// The stateful chord→action resolver the headless replay drives: ONE
/// persistent `KeymapState` across the whole stream (so `C-x` prefix sequences
/// and config rebinds compose exactly as live), plus the STRICT refusals —
/// which moved HERE from parse time when the search guard made resolution
/// replay-state-dependent (a chord the open search panel consumes — `M-c`, a
/// bare `C-x` — never reaches the keymap and must not be judged "unbound").
/// Two action kinds are dropped (`Ok(None)`) because they carry no work for
/// the apply seam: `Ignore` (unbound combos, permissive's silent drop) and
/// `BeginPrefix` (the first half of a `C-x` sequence, whose only job is to
/// flip the keymap's prefix state). Under STRICT, an unbound chord or a
/// non-cancel chord dangling off an armed prefix is an error NAMING that
/// exact chord (`Esc`/`C-g` stay a legal explicit prefix-cancel), so a typo'd
/// scenario aborts instead of replaying a fiction.
pub struct ChordResolver<'a> {
    km: &'a mut KeymapState,
    strict: bool,
    /// The chord that ARMED a still-pending prefix (`C-x`), for the strict
    /// dangling-sequence error's wording. `None` outside a prefix.
    pending_prefix: Option<String>,
}

impl<'a> ChordResolver<'a> {
    pub fn new(km: &'a mut KeymapState, strict: bool) -> Self {
        Self { km, strict, pending_prefix: None }
    }

    /// Resolve one chord through the persistent keymap. `Ok(Some(action))` is
    /// work for the apply seam; `Ok(None)` is a dropped `Ignore`/`BeginPrefix`;
    /// `Err` is a strict refusal naming the offending chord.
    pub fn resolve(&mut self, chord: &Chord) -> Result<Option<Action>> {
        let action = self.km.resolve(&chord.key, &chord.mods);
        if self.strict {
            if matches!(action, Action::Ignore) {
                bail!("strict replay: chord {:?} is unbound (resolves to no action)", chord.spec);
            }
            if let Some(pfx) = &self.pending_prefix {
                if matches!(action, Action::Cancel)
                    && !is_explicit_cancel(&chord.key, chord.mods.state())
                {
                    bail!(
                        "strict replay: chord {:?} does not complete the {pfx:?} prefix (unbound sequence)",
                        chord.spec
                    );
                }
            }
        }
        self.pending_prefix = matches!(action, Action::BeginPrefix).then(|| chord.spec.clone());
        Ok((!matches!(action, Action::Ignore | Action::BeginPrefix)).then_some(action))
    }
}

/// Parse a whole `--keys` spec straight into the resolved `Action` stream with
/// the DEFAULT keymap — [`parse_chords`] + [`ChordResolver`] composed. Retained
/// as the unit-test surface (this module's own resolution tests and the pinned
/// sibling below); the real replay resolves inside its own loop so the search
/// guard can intercept first.
#[allow(dead_code)]
pub fn parse_keys(spec: &str) -> Result<Vec<Action>> {
    parse_keys_through(spec, KeymapState::new())
}

/// TEST-ONLY: like [`parse_keys`], but resolve through a keymap PINNED to
/// `convention` rather than the ambient [`crate::convention::Convention::current`]
/// — so a test with a hardcoded MAC-literal spec (`"Cmd-S-h"`, `"s-p"`, a bare
/// `"C-n"`/`"C-x"` whose letter Linux's collision table displaces, …) stays
/// CONVENTION-PROOF: it resolves identically regardless of which convention
/// happens to be ambient when the test runs (a dev Mac vs. CI's linux runner —
/// see `keymap.rs`'s collision-table doc for the displacement this sidesteps).
/// Every real (non-test) caller — the actual `--keys` CLI replay — correctly
/// wants the ambient convention via [`parse_keys`] / the replay keymap; this
/// pinned sibling exists only so hardcoded-literal unit tests can say exactly
/// which convention they mean.
#[cfg(test)]
pub(crate) fn parse_keys_pinned(spec: &str, convention: crate::convention::Convention) -> Result<Vec<Action>> {
    parse_keys_through(spec, KeymapState::new_with_convention(convention))
}

/// Shared core: resolve every chord in `spec` through one persistent `km` (so C-x
/// prefix state and any config rebinds compose across the sequence).
fn parse_keys_through(spec: &str, km: KeymapState) -> Result<Vec<Action>> {
    parse_keys_mode(spec, km, false)
}

/// The one spec→actions loop the test doors share: [`parse_chords`] then a
/// [`ChordResolver`] driven chord-by-chord — the SAME two primitives the real
/// replay loop composes (with its search guard between them), so a resolution
/// asserted here is the resolution a replay performs.
fn parse_keys_mode(spec: &str, mut km: KeymapState, strict: bool) -> Result<Vec<Action>> {
    let chords = parse_chords(spec)?;
    let mut resolver = ChordResolver::new(&mut km, strict);
    let mut actions = Vec::new();
    for chord in &chords {
        if let Some(action) = resolver.resolve(chord)? {
            actions.push(action);
        }
    }
    Ok(actions)
}

/// Is this chord an EXPLICIT cancel (`Esc` / `C-g`)? Mid-prefix, the keymap
/// resolves EVERY unbound second key to a quiet `Cancel`; strict parsing only
/// forgives the ones a scenario can honestly MEAN as "cancel the prefix".
fn is_explicit_cancel(key: &Key, mods: ModifiersState) -> bool {
    match key {
        Key::Named(NamedKey::Escape) => true,
        Key::Character(s) => {
            s.as_str().eq_ignore_ascii_case("g") && mods.contains(ModifiersState::CONTROL)
        }
        _ => false,
    }
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
            NamedKey::PageUp => "PageUp",
            NamedKey::PageDown => "PageDown",
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

/// LITERAL TEXT → its chord stream, for a storyboard's `type` step: each char
/// becomes exactly the chord a `--keys` spec would spell for it (a bare
/// printable self-insert; whitespace via the NAMED keys — `Space` / `Enter` /
/// `Tab` — since a spec token cannot hold a literal space). Routed through
/// [`parse_chord`] so the token→key mapping has ONE owner and a typed char is
/// byte-for-byte the chord the replay loop already understands (the search
/// guard consumes it while the panel is open, the keymap self-inserts it
/// otherwise). A char `parse_chord` cannot express (none known — every single
/// char is a legal token) surfaces as its clear error rather than a skip.
pub fn text_chords(text: &str) -> Result<Vec<Chord>> {
    text.chars()
        .map(|ch| {
            let tok = match ch {
                ' ' => "Space".to_string(),
                '\n' => "Enter".to_string(),
                '\t' => "Tab".to_string(),
                c => c.to_string(),
            };
            let (key, mods) = parse_chord(&tok)?;
            Ok(Chord { spec: tok, key, mods })
        })
        .collect()
}

/// NAIVE Mac→Linux chord TRANSLATION: swap SUPER for CONTROL in every token's
/// modifiers (leaving ALT/SHIFT untouched), re-emitting the terse canonical form.
/// This is the DEFAULT half of the convention-resolution data design (see
/// `commands::resolved_native`'s doc for the full story): most native chords are
/// a plain Cmd→Ctrl swap (`Cmd-S` → `C-s`), so a per-command override table only
/// needs entries for the handful where that swap is WRONG (word motion, line/doc
/// start-end — see `commands::LINUX_NATIVE_OVERRIDE`). A token that fails to parse
/// passes through verbatim (never panics), mirroring [`mac_glyph_chord`]'s own
/// tolerance. Pure — no convention/global read; the caller decides WHEN to use it.
pub fn translate_native_for_linux(mac_spec: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for tok in mac_spec.split_whitespace() {
        match parse_chord(tok) {
            Ok((key, mods)) => {
                let mut state = mods.state();
                if state.contains(ModifiersState::SUPER) {
                    state.remove(ModifiersState::SUPER);
                    state.insert(ModifiersState::CONTROL);
                }
                out.push(format_chord(&key, state));
            }
            Err(_) => out.push(tok.to_string()),
        }
    }
    out.join(" ")
}

/// Format a chord spec as a LINUX/GTK-style label — `"Ctrl+Shift+P"`, modifiers
/// joined with `+` (Ctrl, Alt, Shift, Super — in that fixed order, matching how
/// GNOME/GTK apps present accelerators) and the key upper-cased for a single
/// letter, mirroring [`mac_glyph_chord`]'s structure but with WORD labels instead
/// of Apple's modifier glyphs (Linux/GTK conventionally has none). A token that
/// fails to parse passes through verbatim. This is the LINUX-convention sibling of
/// [`mac_glyph_chord`] — the two are the only doors [`crate::commands`]'s resolved
/// label owner calls.
pub fn linux_glyph_chord(spec: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for tok in spec.split_whitespace() {
        match parse_chord(tok) {
            Ok((key, mods)) => out.push(linux_glyph_token(&key, mods.state())),
            Err(_) => out.push(tok.to_string()),
        }
    }
    out.join(" ")
}

/// One chord as a Linux/GTK label: `"Ctrl+Shift+P"`. Helper for [`linux_glyph_chord`].
fn linux_glyph_token(key: &Key, mods: ModifiersState) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mods.contains(ModifiersState::CONTROL) {
        parts.push("Ctrl");
    }
    if mods.contains(ModifiersState::ALT) {
        parts.push("Alt");
    }
    if mods.contains(ModifiersState::SHIFT) {
        parts.push("Shift");
    }
    if mods.contains(ModifiersState::SUPER) {
        parts.push("Super");
    }
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(&mac_key_token(key)); // same "single letter upper-cased" rule
    s
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
        // PgUp/PgDn — the card-ful jump through a summoned picker. Named so
        // `--keys "PageDown"` exercises the same overlay paging the live NamedKey
        // does (the harness stays real); several spellings for muscle memory.
        "pageup" | "pgup" => NamedKey::PageUp,
        "pagedown" | "pgdn" | "pagedn" => NamedKey::PageDown,
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

    // CONVENTION-PROOF SHADOW: every hardcoded literal spec in this module
    // documents specifically MAC-native default behavior ("Cmd-S is the save
    // door", "s-Down = buffer end", a bare "C-n"/"C-x" resolving to its EMACS
    // default) — pinning is the honest fix rather than re-deriving a
    // per-convention expectation for each (Linux's own displacement/collision
    // behavior is separately, exhaustively law-tested in `keymap.rs`). These
    // local definitions SHADOW the module-level `parse_keys`/`parse_keys_with`
    // pulled in by `use super::*` (a local item always wins over a glob import
    // in Rust name resolution), so no individual call site below needed editing.
    fn parse_keys(spec: &str) -> Result<Vec<Action>> {
        super::parse_keys_pinned(spec, crate::convention::Convention::Mac)
    }
    fn parse_keys_with(spec: &str, cfg: &crate::config::Config) -> Result<Vec<Action>> {
        parse_keys_through(spec, KeymapState::with_overrides_and_convention(&cfg.keys, crate::convention::Convention::Mac))
    }

    #[test]
    fn ctrl_motions_sequence() {
        assert_eq!(
            parse_keys("C-n C-n").unwrap(),
            vec![Action::NextLine, Action::NextLine]
        );
    }

    #[test]
    fn native_save_and_c_x_retired() {
        // Cmd-S is the save door now; the emacs C-x C-s default is retired (the second
        // key of the bare, defaultless prefix cancels).
        assert_eq!(parse_keys("s-s").unwrap(), vec![Action::Save]);
        assert_eq!(parse_keys("C-x C-s").unwrap(), vec![Action::Cancel]);
    }

    #[test]
    fn self_insert_two_chars() {
        assert_eq!(
            parse_keys("a b").unwrap(),
            vec![Action::InsertChar('a'), Action::InsertChar('b')]
        );
    }

    #[test]
    fn native_buffer_end() {
        // Cmd-Down is the buffer-end door now; the emacs M-> default self-inserts '>'.
        assert_eq!(parse_keys("s-Down").unwrap(), vec![Action::BufferEnd]);
        assert_eq!(parse_keys("M->").unwrap(), vec![Action::InsertChar('>')]);
    }

    #[test]
    fn native_buffer_start() {
        // Cmd-Up is the buffer-start door now; the emacs M-< default self-inserts '<'.
        assert_eq!(parse_keys("s-Up").unwrap(), vec![Action::BufferStart]);
        assert_eq!(parse_keys("M-<").unwrap(), vec![Action::InsertChar('<')]);
    }

    #[test]
    fn parse_keys_with_honours_config_rebind() {
        use crate::config::Config;
        // The config-rebind replay path `main` actually uses: a `[keys]` override
        // resolves a chord to the rebound Action while the spec still splits/prefixes
        // correctly. The default (no-override) path must NOT produce that Action.
        let mut cfg = Config::empty();
        cfg.keys.push(("toggle_debug".into(), vec!["C-j".into()]));
        assert_eq!(parse_keys_with("C-j", &cfg).unwrap(), vec![Action::ToggleDebug]);
        assert_ne!(parse_keys("C-j").unwrap(), vec![Action::ToggleDebug], "default C-j is not ToggleDebug");
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
    fn translate_native_for_linux_swaps_super_for_control_only() {
        assert_eq!(translate_native_for_linux("Cmd-S"), "C-s");
        // `format_chord`'s FIXED modifier order is C- M- S- s-, so Control sorts
        // before Shift regardless of the input's own order.
        assert_eq!(translate_native_for_linux("Cmd-S-p"), "C-S-p");
        assert_eq!(translate_native_for_linux("Cmd-,"), "C-,");
        // ALT/SHIFT-only chords (no Super) pass through unchanged — the naive
        // translator never touches a modifier it didn't ask about.
        assert_eq!(translate_native_for_linux("M-Right"), "M-Right");
        // An unparseable token passes through verbatim (never panics).
        assert_eq!(translate_native_for_linux("C-frobnicate"), "C-frobnicate");
    }

    #[test]
    fn linux_glyph_chord_renders_word_labels() {
        assert_eq!(linux_glyph_chord("C-s"), "Ctrl+S");
        assert_eq!(linux_glyph_chord("S-C-p"), "Ctrl+Shift+P");
        assert_eq!(linux_glyph_chord("C-,"), "Ctrl+,");
        assert_eq!(linux_glyph_chord("M-Right"), "Alt+Right");
        // Fixed modifier order Ctrl, Alt, Shift, Super regardless of input order.
        assert_eq!(
            linux_glyph_chord(&format_chord(&ch_key("z"), ModifiersState::SUPER | ModifiersState::CONTROL)),
            "Ctrl+Super+Z"
        );
        // An unparseable token passes through verbatim (never panics).
        assert_eq!(linux_glyph_chord("C-frobnicate"), "C-frobnicate");
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
    fn pageup_pagedown_tokens_parse_and_page_the_picker() {
        // `--keys "PageUp"/"PageDown"` (several spellings) parse to the NamedKey the
        // live overlay reads, so the harness can exercise the card-ful picker paging
        // (the harness stays real).
        for tok in ["PageUp", "pageup", "PgUp"] {
            let chords = parse_chords(tok).unwrap();
            assert_eq!(chords[0].key, Key::Named(NamedKey::PageUp), "{tok} -> PageUp");
        }
        for tok in ["PageDown", "pagedown", "PgDn", "pagedn"] {
            let chords = parse_chords(tok).unwrap();
            assert_eq!(chords[0].key, Key::Named(NamedKey::PageDown), "{tok} -> PageDown");
        }
        // Round-trips through the canonical token spelling.
        assert_eq!(super::key_token(&Key::Named(NamedKey::PageUp)), "PageUp");
        assert_eq!(super::key_token(&Key::Named(NamedKey::PageDown)), "PageDown");
    }

    #[test]
    fn text_chords_spell_each_char_as_its_keys_token() {
        // A storyboard `type` step's text becomes the same chords a --keys spec
        // would spell: bare printables verbatim, whitespace via the NAMED keys.
        let chords = text_chords("Hi w,\n\t").unwrap();
        let specs: Vec<&str> = chords.iter().map(|c| c.spec.as_str()).collect();
        assert_eq!(specs, vec!["H", "i", "Space", "w", ",", "Enter", "Tab"]);
        assert_eq!(chords[0].key, Key::Character(SmolStr::new("H")));
        assert_eq!(chords[2].key, Key::Named(NamedKey::Space));
        assert_eq!(chords[5].key, Key::Named(NamedKey::Enter));
        assert_eq!(chords[6].key, Key::Named(NamedKey::Tab));
        // No modifiers on any typed char (replay is unshifted by design).
        assert!(chords.iter().all(|c| c.mods.state().is_empty()));
        // And the chars resolve through the REAL keymap to self-inserts.
        let mut km = KeymapState::new_with_convention(crate::convention::Convention::Mac);
        let mut resolver = ChordResolver::new(&mut km, true);
        assert_eq!(resolver.resolve(&chords[0]).unwrap(), Some(Action::InsertChar('H')));
        assert_eq!(resolver.resolve(&chords[2]).unwrap(), Some(Action::InsertChar(' ')));
    }

    // ── STRICT REPLAY TRUTHFULNESS: the strict parse door ──

    /// Strict parse pinned to Mac, mirroring the permissive shadow above (these
    /// tests document MAC-native defaults; Linux displacement is law-tested in
    /// `keymap.rs`).
    fn parse_keys_strict(spec: &str) -> Result<Vec<Action>> {
        parse_keys_mode(spec, KeymapState::new_with_convention(crate::convention::Convention::Mac), true)
    }

    #[test]
    fn strict_rejects_an_unbound_chord_naming_it() {
        // Cmd-L is deliberately unbound (see keymap.rs's own "Cmd-L stays
        // unbound" test): permissive drops it silently, strict names it.
        assert_eq!(parse_keys("s-l").unwrap(), Vec::<Action>::new());
        let err = parse_keys_strict("s-l").unwrap_err().to_string();
        assert!(err.contains("\"s-l\""), "names the exact chord: {err}");
        assert!(err.contains("unbound"), "says why: {err}");
        // The offender is named even mid-spec, after bound chords.
        let err = parse_keys_strict("C-n s-l C-p").unwrap_err().to_string();
        assert!(err.contains("\"s-l\""), "mid-spec offender named: {err}");
    }

    #[test]
    fn strict_rejects_a_dangling_prefix_sequence_naming_both_chords() {
        // The C-x defaults are retired, so `C-x C-s` resolves to a quiet Cancel
        // permissively (pinned above in `native_save_and_c_x_retired`); strict
        // refuses it, naming the failed second key AND the prefix it dangled off.
        let err = parse_keys_strict("C-x C-s").unwrap_err().to_string();
        assert!(err.contains("\"C-s\""), "names the dangling chord: {err}");
        assert!(err.contains("\"C-x\""), "names the prefix: {err}");
    }

    #[test]
    fn strict_allows_an_explicit_prefix_cancel() {
        // Esc / C-g mid-prefix MEAN "cancel the prefix" — a scenario can say
        // that honestly, so strict resolves them exactly like permissive.
        for spec in ["C-x Esc", "C-x C-g"] {
            assert_eq!(
                parse_keys_strict(spec).unwrap(),
                parse_keys(spec).unwrap(),
                "explicit cancel stays legal in strict: {spec:?}"
            );
        }
    }

    #[test]
    fn strict_matches_permissive_on_fully_bound_specs() {
        for spec in ["C-n C-n", "s-s", "a b C-Space Left", "Enter Tab Backspace", "s-Down M->"] {
            assert_eq!(
                parse_keys_strict(spec).unwrap(),
                parse_keys(spec).unwrap(),
                "strict is a pure gate, never a different resolution: {spec:?}"
            );
        }
    }

    #[test]
    fn strict_still_errors_on_an_unparseable_token() {
        // The unparseable-token error is shared with the permissive door (both
        // name the token); strict adds no second vocabulary for it.
        let err = parse_keys_strict("frobnicate").unwrap_err().to_string();
        assert!(err.contains("\"frobnicate\""), "{err}");
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
    fn save_and_quit_via_native() {
        // Save/Quit are native chords now (Cmd-S / Cmd-Q); the C-x prefix defaults are
        // retired, so the old `C-x C-s C-x C-c` sequence cancels twice.
        assert_eq!(
            parse_keys("s-s s-q").unwrap(),
            vec![Action::Save, Action::Quit]
        );
        assert_eq!(
            parse_keys("C-x C-s C-x C-c").unwrap(),
            vec![Action::Cancel, Action::Cancel]
        );
    }

    #[test]
    fn empty_spec_is_empty() {
        assert_eq!(parse_keys("   ").unwrap(), Vec::<Action>::new());
    }
}
