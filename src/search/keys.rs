//! The ONE search/replace KEY-INTERCEPTION seam. While the isearch panel is
//! open, EVERY key belongs to the search surface — printable chars extend the
//! focused field, Backspace shortens it, C-s/C-r/arrows step matches, M-c
//! toggles case, Tab/Cmd-R move between the find and replace fields, Enter
//! accepts / replaces, Cmd-Enter replaces all, Esc/C-g aborts — and nothing
//! ever reaches the keymap. BOTH drivers route through [`intercept`]: the live
//! window's `App::handle_search_key` (a thin delegate) and the headless
//! `--keys` replay's search guard (`main/run.rs::replay_keys_mode`), so live
//! editing and captured replay cannot drift (merge, don't align — this seam
//! retired the documented "isearch-input gap" where a replayed char landed in
//! the BUFFER instead of the query). Renderer-independent by construction: it
//! touches only the pure [`SearchState`] model and the [`Buffer`], and returns
//! the one live-only consequence (a caret recoil) for the windowed caller to
//! animate. The step/jump/abort/replace helpers are module-private — the only
//! door in is `intercept`.

use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::{Direction, SearchState, StepOutcome};
use crate::buffer::Buffer;
use crate::caret::RecoilDir;

/// Route one key press to the active search surface. Only meaningful while
/// `*search` is `Some` (both callers gate on that); a `None` search is a no-op.
/// Consumes EVERY key. Mutates the search state + the buffer (cursor follows
/// the current match; a replace writes the document back; accept/abort close
/// the panel by clearing `*search`). Returns `Some(dir)` when a boundary step
/// RECOILED — the Emacs failing-I-search feedback — so the LIVE caller can bump
/// the visual caret ([`crate::caret::CaretAnim::recoil`]); the headless replay
/// ignores it (no clock, no animation), exactly like `Effect::Recoil`.
pub fn intercept(
    search: &mut Option<SearchState>,
    buffer: &mut Buffer,
    logical: &Key,
    mods: ModifiersState,
) -> Option<RecoilDir> {
    let ctrl = mods.contains(ModifiersState::CONTROL);
    let alt = mods.contains(ModifiersState::ALT);
    let sup = mods.contains(ModifiersState::SUPER);
    let shift = mods.contains(ModifiersState::SHIFT);
    // Which field a self-insert / Backspace edits: the replacement (true) or
    // the search query (false). A bool copy, so the immutable borrow is dropped
    // before the arms below take a mutable borrow of `*search`.
    let editing_replacement = search
        .as_ref()
        .map(|s| s.is_editing_replacement())
        .unwrap_or(false);

    match logical {
        Key::Character(s) => {
            let Some(c) = s.chars().next() else { return None };
            // Cmd-based Find/Replace chords WITHIN the panel: Cmd-F skips to the
            // next match, Cmd-Shift-F the previous (so you can pass a match without
            // replacing it), Cmd-Option-F reveals+toggles the replace field, Cmd-R
            // focuses the replace field (the headline door — a fresh Cmd-R opened
            // the panel on the find field), and Cmd-G / Cmd-Shift-G MIRROR Cmd-F /
            // Cmd-Shift-F's plain step (P2 — the deeper macOS find-next/previous
            // idiom, alongside Cmd-F's own in-panel step; Cmd-Option-G has no
            // Option-toggle counterpart, so it is simply consumed, no-op). Other
            // Super combos are consumed.
            if sup && !ctrl {
                if c.eq_ignore_ascii_case(&'f') {
                    if alt {
                        if let Some(st) = search.as_mut() {
                            st.toggle_replace();
                        }
                    } else if shift {
                        return step(search, buffer, Direction::Backward);
                    } else {
                        return step(search, buffer, Direction::Forward);
                    }
                } else if c.eq_ignore_ascii_case(&'g') && !alt {
                    if shift {
                        return step(search, buffer, Direction::Backward);
                    } else {
                        return step(search, buffer, Direction::Forward);
                    }
                } else if c.eq_ignore_ascii_case(&'r') && !alt {
                    if let Some(st) = search.as_mut() {
                        st.focus_replacement();
                    }
                }
                return None;
            }
            if ctrl && !alt {
                match c.to_ascii_lowercase() {
                    's' => return step(search, buffer, Direction::Forward),
                    'r' => return step(search, buffer, Direction::Backward),
                    'g' => abort(search, buffer),
                    _ => {} // other ctrl combos: consumed, no-op
                }
            } else if alt && !ctrl {
                if matches!(c, 'c' | 'C') {
                    // M-c / Alt+c toggles case sensitivity.
                    let hay = buffer.text();
                    if let Some(st) = search.as_mut() {
                        st.toggle_case(&hay);
                    }
                    jump_to_current(search, buffer);
                }
            } else if !c.is_control() {
                // Self-insert into the FOCUSED field. The replacement is not
                // searched, so typing it never moves a match; query edits do.
                if editing_replacement {
                    if let Some(st) = search.as_mut() {
                        st.push_replace_char(c);
                    }
                } else {
                    let hay = buffer.text();
                    if let Some(st) = search.as_mut() {
                        st.push_char(c, &hay);
                    }
                    jump_to_current(search, buffer);
                }
            }
        }
        // Tab is the one FIELD-SWITCH key: flip focus find↔replace (revealing the
        // replace row the first time). No longer overloaded — Enter replaces, Tab
        // only moves between the two fields of the one warm panel.
        Key::Named(NamedKey::Tab) => {
            if let Some(st) = search.as_mut() {
                st.toggle_replace();
            }
        }
        // Down / Up SKIP to the next / previous match without replacing (alongside
        // Cmd-F / Cmd-Shift-F), so you can pass over a match you don't want changed.
        Key::Named(NamedKey::ArrowDown) => return step(search, buffer, Direction::Forward),
        Key::Named(NamedKey::ArrowUp) => return step(search, buffer, Direction::Backward),
        Key::Named(NamedKey::Backspace) => {
            if editing_replacement {
                if let Some(st) = search.as_mut() {
                    st.pop_replace_char();
                }
            } else {
                let hay = buffer.text();
                if let Some(st) = search.as_mut() {
                    st.pop_char(&hay);
                }
                jump_to_current(search, buffer);
            }
        }
        Key::Named(NamedKey::Enter) => {
            // The clarified core loop: once replace is active, Enter ALWAYS
            // replaces the current match + advances to the next (regardless of
            // which field has focus) — Cmd-Enter replaces ALL. In a PLAIN find
            // (no replace row), Enter ACCEPTS (closes, leaving the cursor on the
            // current match). Esc / C-g is the "done" door out of replace.
            let replace_active = search
                .as_ref()
                .map(|s| s.is_replace_active())
                .unwrap_or(false);
            if sup && replace_active {
                replace_all(search, buffer);
            } else if replace_active {
                replace_current(search, buffer);
            } else {
                // ACCEPT: remember the query (P2) before closing, so a
                // LATER bare Cmd-G re-finds it.
                if let Some(st) = search.as_ref() {
                    super::set_last_query(st.query());
                }
                *search = None;
                buffer.seal_undo_group();
            }
        }
        Key::Named(NamedKey::Space) if !ctrl && !alt && !sup => {
            // Space arrives as a Named key (not a Character), so without this
            // arm it would fall through to the no-op below and never reach the
            // focused field. Ctrl/Alt/Cmd+Space stay no-ops.
            if editing_replacement {
                if let Some(st) = search.as_mut() {
                    st.push_replace_char(' ');
                }
            } else {
                let hay = buffer.text();
                if let Some(st) = search.as_mut() {
                    st.push_char(' ', &hay);
                }
                jump_to_current(search, buffer);
            }
        }
        Key::Named(NamedKey::Escape) => abort(search, buffer),
        _ => {} // any other named key: consumed, no-op
    }
    None
}

/// C-s / C-r (and arrows / the Cmd-F family) while searching: advance to the
/// next/previous match (the Emacs two-press wrap) and move the real cursor onto
/// it. A step that FAILS at the boundary does NOT advance — it returns the
/// recoil direction (forward travels toward the end → bump UP; backward →
/// DOWN), mirroring the blocked-motion recoil, and arms the two-press wrap.
fn step(
    search: &mut Option<SearchState>,
    buffer: &mut Buffer,
    dir: Direction,
) -> Option<RecoilDir> {
    let outcome = search.as_mut().map(|st| st.step(dir));
    let recoil = match outcome {
        Some(StepOutcome::RecoiledAtBoundary(d)) => Some(match d {
            Direction::Forward => RecoilDir::Up,
            Direction::Backward => RecoilDir::Down,
        }),
        _ => None,
    };
    jump_to_current(search, buffer);
    recoil
}

/// Move the real buffer cursor onto the current match (if any) so the amber
/// document caret lands on it. No-op (cursor unchanged) when there is no
/// current match — we don't jump on a no-match query.
fn jump_to_current(search: &Option<SearchState>, buffer: &mut Buffer) {
    if let Some(st) = search.as_ref() {
        if let Some(m) = st.current_match() {
            buffer.set_cursor(m.start);
        }
    }
}

/// Esc / C-g: restore the cursor to where search began and close the panel.
/// REMEMBERS the query first (P2) — a non-empty abandoned search still
/// survives the close so a later bare Cmd-G re-finds it.
fn abort(search: &mut Option<SearchState>, buffer: &mut Buffer) {
    if let Some(st) = search.as_ref() {
        super::set_last_query(st.query());
        let origin = st.origin();
        buffer.set_cursor(origin);
    }
    buffer.clear_mark();
    *search = None;
}

/// REPLACE-CURRENT (Enter in the replace field): swap the active match for the
/// replacement text, write the new document back as one atomic edit, and ADVANCE
/// the search to the next match (the cursor follows). The panel stays open so a
/// repeated Enter walks forward replacing. A no-op unless replace mode is active
/// and there is a current match.
fn replace_current(search: &mut Option<SearchState>, buffer: &mut Buffer) {
    let hay = buffer.text();
    let new_text = match search.as_mut() {
        Some(st) if st.is_replace_active() => st.replace_current_text(&hay),
        _ => return,
    };
    if let Some(t) = new_text {
        buffer.set_text(&t);
        jump_to_current(search, buffer);
    }
}

/// REPLACE-ALL (Cmd-Enter): swap EVERY current-query match for the replacement
/// in one atomic, undoable edit, then re-anchor the (now usually empty) match
/// set at the search origin. A no-op unless replace mode is active and the text
/// actually changes.
fn replace_all(search: &mut Option<SearchState>, buffer: &mut Buffer) {
    let hay = buffer.text();
    let (new_text, origin) = match search.as_ref() {
        Some(st) if st.is_replace_active() => (st.replace_all_text(&hay), st.origin()),
        _ => return,
    };
    if new_text == hay {
        return;
    }
    buffer.set_text(&new_text);
    let new_hay = buffer.text();
    if let Some(st) = search.as_mut() {
        st.refind(origin, &new_hay);
    }
    jump_to_current(search, buffer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::SmolStr;

    fn ch(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    fn named(k: NamedKey) -> Key {
        Key::Named(k)
    }

    const NONE: ModifiersState = ModifiersState::empty();

    /// Open a search over `text` anchored at char 0 and return (search, buffer).
    fn open(text: &str) -> (Option<SearchState>, Buffer) {
        let buffer = Buffer::from_str(text);
        let search = Some(SearchState::start(0, Direction::Forward));
        (search, buffer)
    }

    /// Feed a bare printable string char-by-char through the seam.
    fn type_str(search: &mut Option<SearchState>, buffer: &mut Buffer, s: &str) {
        for c in s.chars() {
            let key = if c == ' ' { named(NamedKey::Space) } else { ch(&c.to_string()) };
            intercept(search, buffer, &key, NONE);
        }
    }

    /// THE SEARCH-TYPING REGRESSION (the retired "isearch-input gap"): with the
    /// panel open, printable keys extend the QUERY — the buffer text is never
    /// touched — and the cursor lands on the current match.
    #[test]
    fn typing_extends_the_query_never_the_buffer() {
        let (mut search, mut buffer) = open("alpha beta alpha");
        type_str(&mut search, &mut buffer, "beta");
        assert_eq!(search.as_ref().unwrap().query(), "beta");
        assert_eq!(buffer.text(), "alpha beta alpha", "the document is untouched");
        assert_eq!(buffer.cursor_char(), 6, "the caret sits on the match");
        // Space through the Named-key arm joins the query too.
        type_str(&mut search, &mut buffer, " a");
        assert_eq!(search.as_ref().unwrap().query(), "beta a");
        assert_eq!(buffer.text(), "alpha beta alpha");
    }

    #[test]
    fn backspace_pops_the_focused_field() {
        let (mut search, mut buffer) = open("abc abd");
        type_str(&mut search, &mut buffer, "abc");
        assert_eq!(search.as_ref().unwrap().hit_count(), 1);
        intercept(&mut search, &mut buffer, &named(NamedKey::Backspace), NONE);
        let st = search.as_ref().unwrap();
        assert_eq!(st.query(), "ab");
        assert_eq!(st.hit_count(), 2);
        // With the replace field focused, Backspace edits the REPLACEMENT.
        intercept(&mut search, &mut buffer, &named(NamedKey::Tab), NONE);
        type_str(&mut search, &mut buffer, "xy");
        intercept(&mut search, &mut buffer, &named(NamedKey::Backspace), NONE);
        let st = search.as_ref().unwrap();
        assert_eq!(st.replacement(), "x");
        assert_eq!(st.query(), "ab", "the query is untouched by replace-field edits");
    }

    #[test]
    fn steps_advance_and_recoil_at_the_boundary() {
        let (mut search, mut buffer) = open("x.x.x");
        type_str(&mut search, &mut buffer, "x");
        assert_eq!(buffer.cursor_char(), 0);
        // Every step door advances: C-s, ArrowDown, Cmd-F, Cmd-G.
        assert_eq!(intercept(&mut search, &mut buffer, &ch("s"), ModifiersState::CONTROL), None);
        assert_eq!(buffer.cursor_char(), 2);
        assert_eq!(intercept(&mut search, &mut buffer, &named(NamedKey::ArrowDown), NONE), None);
        assert_eq!(buffer.cursor_char(), 4);
        // First forward press at the last match: recoil UP, cursor stays put.
        assert_eq!(
            intercept(&mut search, &mut buffer, &ch("f"), ModifiersState::SUPER),
            Some(RecoilDir::Up)
        );
        assert_eq!(buffer.cursor_char(), 4);
        // Second press wraps to the first match.
        assert_eq!(intercept(&mut search, &mut buffer, &ch("g"), ModifiersState::SUPER), None);
        assert_eq!(buffer.cursor_char(), 0);
        // Backward from the first match: recoil DOWN, then C-r/ArrowUp step back.
        assert_eq!(
            intercept(&mut search, &mut buffer, &ch("r"), ModifiersState::CONTROL),
            Some(RecoilDir::Down)
        );
        assert_eq!(buffer.cursor_char(), 0);
        // Cmd-Shift-F / Cmd-Shift-G mirror the backward step (post-recoil wrap).
        assert_eq!(
            intercept(&mut search, &mut buffer, &ch("F"), ModifiersState::SUPER | ModifiersState::SHIFT),
            None
        );
        assert_eq!(buffer.cursor_char(), 4, "armed backward step wrapped to the last match");
    }

    #[test]
    fn alt_c_toggles_case_sensitivity() {
        let (mut search, mut buffer) = open("Hello HELLO hello");
        type_str(&mut search, &mut buffer, "hello");
        assert_eq!(search.as_ref().unwrap().hit_count(), 3);
        intercept(&mut search, &mut buffer, &ch("c"), ModifiersState::ALT);
        let st = search.as_ref().unwrap();
        assert!(st.is_case_sensitive());
        assert_eq!(st.hit_count(), 1);
        intercept(&mut search, &mut buffer, &ch("C"), ModifiersState::ALT);
        assert!(!search.as_ref().unwrap().is_case_sensitive());
    }

    /// Tab reveals the replace row then flips focus; Cmd-R forces focus into the
    /// replacement; Cmd-Option-F rides the same toggle — the affordances the
    /// retired `apply_core` search intercept used to cover at the Action level.
    #[test]
    fn tab_and_cmd_r_move_between_the_two_fields() {
        let (mut search, mut buffer) = open("alpha beta alpha");
        intercept(&mut search, &mut buffer, &named(NamedKey::Tab), NONE);
        {
            let st = search.as_ref().unwrap();
            assert!(st.is_replace_active());
            assert!(st.is_editing_replacement());
        }
        intercept(&mut search, &mut buffer, &named(NamedKey::Tab), NONE);
        assert!(!search.as_ref().unwrap().is_editing_replacement());
        intercept(&mut search, &mut buffer, &ch("r"), ModifiersState::SUPER);
        assert!(search.as_ref().unwrap().is_editing_replacement());
        // Cmd-Option-F toggles back to the find field.
        intercept(&mut search, &mut buffer, &ch("f"), ModifiersState::SUPER | ModifiersState::ALT);
        assert!(!search.as_ref().unwrap().is_editing_replacement());
        // None of the field motion leaked a char anywhere.
        assert_eq!(buffer.text(), "alpha beta alpha");
    }

    #[test]
    fn enter_accepts_a_plain_find_and_remembers_the_query() {
        let _g = crate::testlock::serial();
        crate::search::clear_last_query();
        let (mut search, mut buffer) = open("alpha beta alpha");
        type_str(&mut search, &mut buffer, "beta");
        intercept(&mut search, &mut buffer, &named(NamedKey::Enter), NONE);
        assert!(search.is_none(), "plain-find Enter closes the panel");
        assert_eq!(buffer.cursor_char(), 6, "the cursor stays on the accepted match");
        assert_eq!(crate::search::last_query(), "beta");
        crate::search::clear_last_query();
    }

    #[test]
    fn enter_replaces_current_and_cmd_enter_replaces_all() {
        let (mut search, mut buffer) = open("x.x.x");
        type_str(&mut search, &mut buffer, "x");
        intercept(&mut search, &mut buffer, &named(NamedKey::Tab), NONE);
        type_str(&mut search, &mut buffer, "Y");
        // Enter in replace mode: swap ONE match, advance, panel stays open.
        intercept(&mut search, &mut buffer, &named(NamedKey::Enter), NONE);
        assert_eq!(buffer.text(), "Y.x.x");
        assert!(search.is_some(), "replace-current keeps the panel open");
        assert_eq!(buffer.cursor_char(), 2, "cursor advanced to the next match");
        // Cmd-Enter: swap EVERY remaining match in one edit.
        intercept(&mut search, &mut buffer, &named(NamedKey::Enter), ModifiersState::SUPER);
        assert_eq!(buffer.text(), "Y.Y.Y");
        assert!(search.is_some());
        assert_eq!(search.as_ref().unwrap().hit_count(), 0, "no needle remains");
    }

    #[test]
    fn escape_aborts_restoring_the_origin_cursor() {
        let _g = crate::testlock::serial();
        crate::search::clear_last_query();
        let mut buffer = Buffer::from_str("alpha beta alpha");
        buffer.set_cursor(3);
        let mut search = Some(SearchState::start(3, Direction::Forward));
        type_str(&mut search, &mut buffer, "beta");
        assert_eq!(buffer.cursor_char(), 6, "the search moved the cursor");
        intercept(&mut search, &mut buffer, &named(NamedKey::Escape), NONE);
        assert!(search.is_none());
        assert_eq!(buffer.cursor_char(), 3, "abort restores the origin");
        assert_eq!(crate::search::last_query(), "beta", "an abandoned query is still remembered");
        crate::search::clear_last_query();
    }

    /// EVERY key is consumed while the panel is open: a C-x never arms the
    /// keymap prefix (it isn't even seen by the keymap), an unbound Super combo
    /// and a stray named key are quiet no-ops, and none of them leak into the
    /// buffer or close the panel.
    #[test]
    fn unhandled_chords_are_consumed_no_ops() {
        let (mut search, mut buffer) = open("alpha beta alpha");
        type_str(&mut search, &mut buffer, "beta");
        for (key, mods) in [
            (ch("x"), ModifiersState::CONTROL),          // the live C-x prefix chord
            (ch("p"), ModifiersState::SUPER),            // Cmd-P: palette stays shut
            (named(NamedKey::Home), NONE),               // stray named key
            (named(NamedKey::Space), ModifiersState::CONTROL), // modified Space
        ] {
            assert_eq!(intercept(&mut search, &mut buffer, &key, mods), None);
            let st = search.as_ref().expect("the panel stays open");
            assert_eq!(st.query(), "beta", "the query is unchanged");
        }
        assert_eq!(buffer.text(), "alpha beta alpha");
    }
}
