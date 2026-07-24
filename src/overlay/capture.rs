//! The Keybindings-menu CAPTURE sub-state (`Capture`/`CaptureStage`) and the
//! Settings-menu inline VALUE-EDIT sub-state (`ValueEdit`), plus the
//! `OverlayState` methods that drive both state machines. Split out of the
//! former `overlay.rs` monolith (2026-07 code-organization pass); every
//! item's path is unchanged -- only the file it lives in moved.

use super::{OverlayKind, OverlayState};
use crate::textbox::TextBox;

/// Which phase of a Keybindings CAPTURE we are in (carried by [`Capture`]). Drives
/// what the next key does and what the card prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStage {
    /// Just after Enter on a command: a two-row choice of KEY vs CHORD (Up/Down
    /// toggles, Enter confirms the mode and begins recording).
    ChooseMode,
    /// Recording presses. KEY mode finishes on the FIRST combo; CHORD mode collects
    /// successive combos (capped at the keymap's 2-deep limit) until Enter finishes.
    Recording,
    /// The finished binding clashes with another command; Enter COMMITS anyway,
    /// Esc aborts. `conflict` names the command already bound.
    Confirm,
}

/// The live CAPTURE sub-state of the Keybindings menu: which command is being
/// rebound, the phase, the KEY-vs-CHORD mode, and the combos captured so far. Pure
/// + serialisable so the capture flows into the sidecar and is unit-testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// The VISIBLE-CORPUS (`commands::visible()`) index of the command being rebound —
    /// the Keybindings corpus is built from `commands::visible_names()`, so this is the
    /// selected row index into THAT filtered view, not a raw `commands::COMMANDS` index
    /// (see `commands.rs`'s "PLATFORM-SCOPED COMMANDS" section).
    pub cmd_index: usize,
    /// The command's display name (for the prompt + conflict notices).
    pub cmd_name: String,
    pub stage: CaptureStage,
    /// In `ChooseMode`: 0 = KEY row, 1 = CHORD row. Records the chosen mode after.
    pub mode_sel: usize,
    /// `false` = KEY (single combo), `true` = CHORD (a sequence). Set when leaving
    /// `ChooseMode`.
    pub chord_mode: bool,
    /// The combos captured so far (KEY: 0–1; CHORD: up to 2), each a canonical chord
    /// spec (`"C-t"`, `"C-x"`). Joined by spaces, this is the binding being written.
    pub captured: Vec<String>,
    /// `Confirm` stage only: the command this binding already belongs to.
    pub conflict: Option<String>,
}

impl Capture {
    /// The binding SPEC being built — the captured combos joined by spaces
    /// (`"C-x C-s"`). Empty until the first combo is recorded.
    pub fn binding(&self) -> String {
        self.captured.join(" ")
    }

    /// The dim PROMPT line the card shows for this capture phase, surfaced to the
    /// sidecar so the flow is agent-verifiable.
    pub fn prompt(&self) -> String {
        match self.stage {
            CaptureStage::ChooseMode => {
                let key = if self.mode_sel == 0 { "[Key]" } else { "Key" };
                let chord = if self.mode_sel == 1 { "[Chord]" } else { "Chord" };
                format!("Rebind {} — {key} / {chord}   Enter choose   Esc cancel", self.cmd_name)
            }
            CaptureStage::Recording => {
                let so_far = self.binding();
                if self.chord_mode {
                    format!("press the sequence… {so_far}   Enter done   Esc cancel")
                } else {
                    format!("press a key… {so_far}   Esc cancel")
                }
            }
            CaptureStage::Confirm => {
                let who = self.conflict.as_deref().unwrap_or("another command");
                format!("{} already bound to {who} — Enter rebind   Esc cancel", self.binding())
            }
        }
    }
}

/// The live inline VALUE-EDIT sub-state of the Settings menu (Enter on a
/// [`crate::settings::SettingKind::Value`] row): which row is being edited, the
/// config key its commit writes, the text typed so far, and the ORIGINAL cell value
/// to restore on cancel. Pure + serialisable, mirroring [`Capture`]. While it is
/// `Some`, the Settings overlay OWNS every key at the intercept level (digits build
/// the value, Enter commits, Esc cancels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueEdit {
    /// The settings-table (corpus) index of the Value row being edited — also the
    /// index into `bindings` whose cell shows the live typed value.
    pub row: usize,
    /// The row's display name (for the prompt / logging).
    pub name: String,
    /// The config key the commit writes ("page_width_prose"/"page_width_code"/"zoom").
    pub key: String,
    /// ITEM 10 — the text typed so far (digits, plus a single `.`/`%` for zoom)
    /// plus its CHAR-index caret, one shared [`TextBox`]. Seeded from the row's
    /// current value cell (caret at END) so a typed edit starts from the shown
    /// value. The digit/`.`/`%` FILTER stays here in [`OverlayState::value_edit_push`]
    /// — `TextBox::insert` itself accepts any char.
    pub input: TextBox,
    /// The cell value at edit start, restored verbatim on cancel (the core can't
    /// re-gather the config, so it stashes the original here).
    pub orig: String,
}

/// NOTES VERBS round: the live RENAME minibuffer sub-state — the current typed
/// filename plus the original (for the prompt's "unchanged" no-op check). Pure +
/// serialisable, mirroring [`ValueEdit`]'s exact shape but WITHOUT the numeric/`.`/`%`
/// filter (a filename accepts any character except the path separator `/`, which
/// would let a typed name silently escape into a different directory). While it is
/// `Some`, the Rename overlay OWNS every key at the intercept level (any printable
/// char extends `input`, Backspace deletes, Enter commits, Esc cancels) — see
/// [`super::overlay_nav`]'s `rename_edit`-first check (`actions/overlay_nav.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameEdit {
    /// ITEM 10 — the text typed so far + its CHAR-index caret, seeded (caret at
    /// END) from the file's current name. The `/`-REJECT filter stays here in
    /// [`OverlayState::rename_edit_push`] — `TextBox::insert` itself accepts any char.
    pub input: TextBox,
    /// The name at edit start (unused by the core beyond equality — the CALLER's
    /// own "unchanged input is a no-op" gate reads it via `Effect::RenameNoteCommit`
    /// naming the typed value; kept here so a future cancel-restore path, or a test,
    /// never has to re-derive it).
    pub orig: String,
}

impl RenameEdit {
    /// The dim PROMPT line the card shows while renaming, surfaced to the sidecar's
    /// `overlay.hint` via [`OverlayState::foot_hint`] — exactly the seam the
    /// Keybindings capture's own `Capture::prompt` rides, so the minibuffer's typing
    /// state is `--keys`-verifiable with ZERO new sidecar plumbing.
    pub fn prompt(&self) -> String {
        format!("rename to: {}   Enter commit   Esc cancel", self.input.text())
    }
}

/// LINKS V2: what the committed URL is APPLIED to — decided once, purely, from
/// buffer state the instant Cmd-K is pressed (`actions/link.rs`'s dispatch), then
/// carried untouched through the whole minibuffer flow so the commit at Enter is
/// a single pure text-build, no buffer re-inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEditMode {
    /// WRAP or REWRITE: replace the CHAR range `[start, end)` with
    /// `[{text}]({url})` — `text` is the selection being wrapped into a NEW link,
    /// or the EXISTING link's own visible text being carried over into a rewrite
    /// (Cmd-K with the caret already inside a link). The two cases are the exact
    /// same edit shape, so one variant covers both.
    WithText { start: usize, end: usize, text: String },
    /// INSERT empty markup `[](url)` at char position `at` — no selection, no
    /// existing link under the caret. The caret lands BETWEEN the brackets after
    /// commit, ready to type the link text.
    Empty { at: usize },
}

/// LINKS V2: the live Cmd-K minibuffer sub-state (`Some` only for
/// [`OverlayKind::InsertLink`], armed the instant the overlay is BUILT by
/// [`OverlayState::new_link_edit`] — mirrors [`RenameEdit`]'s "nothing to browse
/// before typing starts" shape exactly, but with NO character filter: a URL
/// legitimately contains `/`, unlike a filename, so every printable char is
/// accepted (the one difference from `RenameEdit::push`'s `/`-rejection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEdit {
    /// ITEM 10 — the text typed so far + its CHAR-index caret, seeded (caret at
    /// END) from the prefill (existing link's URL in EDIT mode; else the
    /// clipboard/kill head IF it looks like a URL; else empty). NO character
    /// filter — `TextBox::insert` accepts any char, a URL legitimately contains `/`.
    pub input: TextBox,
    pub mode: LinkEditMode,
}

impl LinkEdit {
    /// The dim PROMPT line the card shows while typing, surfaced to the sidecar's
    /// `overlay.hint` via [`OverlayState::foot_hint`] — the exact seam
    /// [`RenameEdit::prompt`] rides, so the URL-typing state is `--keys`-verifiable
    /// with ZERO new sidecar plumbing.
    pub fn prompt(&self) -> String {
        format!("link to: {}   Enter commit   Esc cancel", self.input.text())
    }
}

/// NAMED SAVE POINTS: the live "Keep version…" minibuffer sub-state (`Some` only
/// for [`OverlayKind::KeepName`], armed the instant the overlay is BUILT by
/// [`OverlayState::new_keep_name`] — mirrors [`LinkEdit`]'s "nothing to browse
/// before typing starts" shape exactly, with NO character filter (a name is free
/// display text, never a path or URL). The typed name is OPTIONAL by design:
/// Enter on an empty input commits the plain (nameless) keep — today's zero-
/// friction behavior — while Enter with text commits a NAMED point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepEdit {
    /// ITEM 10 — the text typed so far + its CHAR-index caret, one shared
    /// [`TextBox`]. Seeded EMPTY (a fresh point is being marked — there is no
    /// old name to edit). NO character filter, mirroring [`LinkEdit::input`].
    pub input: TextBox,
}

impl KeepEdit {
    /// The dim PROMPT line the card shows while typing, surfaced to the sidecar's
    /// `overlay.hint` via [`OverlayState::foot_hint`] — the exact seam
    /// [`RenameEdit::prompt`]/[`LinkEdit::prompt`] ride, so the naming flow is
    /// `--keys`-verifiable with ZERO new sidecar plumbing. "Enter keep" holds for
    /// an empty input too (a blank Enter IS the plain keep).
    pub fn prompt(&self) -> String {
        format!("name this version: {}   Enter keep   Esc cancel", self.input.text())
    }
}

impl OverlayState {
    /// REBIND MENU: begin a capture for the highlighted command (catalog index). A
    /// no-op when no row matches the filter. Opens in `ChooseMode` with KEY preselected.
    pub fn start_capture(&mut self) {
        let Some(i) = self.selected_corpus_index() else {
            return;
        };
        self.notice.clear();
        self.capture = Some(Capture {
            cmd_index: i,
            cmd_name: crate::commands::visible_name_of(i).to_string(),
            stage: CaptureStage::ChooseMode,
            mode_sel: 0,
            chord_mode: false,
            captured: Vec::new(),
            conflict: None,
        });
    }

    /// REBIND MENU: in `ChooseMode`, move the KEY/CHORD selection (`delta` &lt; 0 → KEY,
    /// &gt; 0 → CHORD). Other phases ignore it.
    pub fn capture_move_mode(&mut self, delta: isize) {
        if let Some(cap) = self.capture.as_mut() {
            if cap.stage == CaptureStage::ChooseMode {
                cap.mode_sel = if delta < 0 { 0 } else { 1 };
            }
        }
    }

    /// REBIND MENU: leave `ChooseMode` — lock in KEY vs CHORD and begin `Recording`.
    pub fn capture_begin_recording(&mut self) {
        if let Some(cap) = self.capture.as_mut() {
            if cap.stage == CaptureStage::ChooseMode {
                cap.chord_mode = cap.mode_sel == 1;
                cap.stage = CaptureStage::Recording;
            }
        }
    }

    /// REBIND MENU: record one captured `combo` (a canonical chord spec) while
    /// `Recording`. Returns `true` when the binding is now COMPLETE — KEY mode after
    /// the first combo (finishes instantly), or CHORD mode once the 2-deep cap is hit
    /// — so the caller can finalise it; `false` while a CHORD still awaits more (Enter).
    /// A no-op outside `Recording`.
    pub fn capture_record(&mut self, combo: String) -> bool {
        let Some(cap) = self.capture.as_mut() else {
            return false;
        };
        if cap.stage != CaptureStage::Recording {
            return false;
        }
        if cap.chord_mode {
            if cap.captured.len() < 2 {
                cap.captured.push(combo);
            }
            // CHORD: a full 2-deep sequence is complete; otherwise wait for Enter.
            cap.captured.len() >= 2
        } else {
            cap.captured = vec![combo];
            true // KEY: one combo finishes instantly.
        }
    }

    /// REBIND MENU: the (slug, binding-spec) for the in-progress capture, or `None`
    /// when nothing has been captured yet. The slug keys the `[keys]` entry; the
    /// binding is the captured combos joined by spaces.
    pub fn capture_target(&self) -> Option<(String, String)> {
        let cap = self.capture.as_ref()?;
        if cap.captured.is_empty() {
            return None;
        }
        Some((crate::commands::visible_slug_of(cap.cmd_index), cap.binding()))
    }

    /// REBIND MENU: move the capture into the `Confirm` phase (a clash was found),
    /// remembering `conflict` (the command already bound) for the prompt.
    pub fn capture_into_confirm(&mut self, conflict: String) {
        if let Some(cap) = self.capture.as_mut() {
            cap.stage = CaptureStage::Confirm;
            cap.conflict = Some(conflict);
        }
    }

    /// REBIND MENU: cancel any in-progress capture, returning to the command list.
    pub fn capture_abort(&mut self) {
        self.capture = None;
    }

    /// SETTINGS: begin inline VALUE editing of the highlighted row. Seeds the typed
    /// `input` from the row's CURRENT value cell (`rows[ci].secondary`) so the edit
    /// starts from the shown value (backspace to change it), and stashes it as
    /// `orig` for a clean cancel. `key`/`name` come from the single-owner
    /// [`crate::settings::value_key`] map. A no-op if no row matches the filter.
    pub fn start_value_edit(&mut self, key: String, name: String) {
        let Some(row) = self.selected_corpus_index() else {
            return;
        };
        let orig = self.rows.get(row).map(|r| r.secondary.clone()).unwrap_or_default();
        self.value_edit = Some(ValueEdit { row, name, key, input: TextBox::seeded(&orig), orig });
    }

    /// SETTINGS VALUE EDIT: mirror the row's field text into its own display cell —
    /// the tail every value-edit mutator below shares (push/pop/pop_word/motion all
    /// end by re-showing the live typed value). A no-op when no value edit is active.
    fn value_edit_mirror(&mut self) {
        let Some(ve) = self.value_edit.as_ref() else {
            return;
        };
        let (row, text) = (ve.row, ve.input.text().to_string());
        if let Some(r) = self.rows.get_mut(row) {
            r.secondary = text;
        }
    }

    /// SETTINGS VALUE EDIT: insert `c` at the caret when it is valid — a digit
    /// always, or a SINGLE `.`/`%` (zoom) — and mirror the new text into the row's own
    /// value cell so the edit is visible. Any other char is ignored (calm); the
    /// FILTER stays here — [`TextBox::insert`] itself accepts any char. A no-op
    /// when no value edit is active.
    pub fn value_edit_push(&mut self, c: char) {
        let Some(ve) = self.value_edit.as_mut() else {
            return;
        };
        let text = ve.input.text();
        let ok = c.is_ascii_digit()
            || (c == '.' && !text.contains('.'))
            || (c == '%' && !text.contains('%'));
        if ok {
            ve.input.insert(c);
        }
        self.value_edit_mirror();
    }

    /// SETTINGS VALUE EDIT: delete the char before the caret, mirroring the change
    /// into the row's cell. A no-op when no value edit is active.
    pub fn value_edit_pop(&mut self) {
        let Some(ve) = self.value_edit.as_mut() else {
            return;
        };
        ve.input.delete_back();
        self.value_edit_mirror();
    }

    /// SETTINGS VALUE EDIT: ⌥⌫ word-delete — drop the trailing word (the word-DELETE
    /// rule, [`TextBox::delete_word_back`]), mirroring the change into the row's
    /// cell. A no-op when no value edit is active.
    pub fn value_edit_pop_word(&mut self) {
        let Some(ve) = self.value_edit.as_mut() else {
            return;
        };
        ve.input.delete_word_back();
        self.value_edit_mirror();
    }

    /// ITEM 10 — SETTINGS VALUE EDIT char/word motion + forward word-delete: move
    /// (or forward-delete from) the caret WITHOUT editing backward, mirroring the
    /// cell afterward (motion never changes the text, but the mirror stays cheap
    /// + uniform with the edits above). A no-op when no value edit is active.
    pub fn value_edit_char_left(&mut self) {
        if let Some(ve) = self.value_edit.as_mut() {
            ve.input.char_left();
        }
    }
    pub fn value_edit_char_right(&mut self) {
        if let Some(ve) = self.value_edit.as_mut() {
            ve.input.char_right();
        }
    }
    pub fn value_edit_word_left(&mut self) {
        if let Some(ve) = self.value_edit.as_mut() {
            ve.input.word_left();
        }
    }
    pub fn value_edit_word_right(&mut self) {
        if let Some(ve) = self.value_edit.as_mut() {
            ve.input.word_right();
        }
    }
    pub fn value_edit_delete_word_forward(&mut self) {
        let Some(ve) = self.value_edit.as_mut() else {
            return;
        };
        ve.input.delete_word_forward();
        self.value_edit_mirror();
    }

    /// SETTINGS VALUE EDIT commit target: the `(config key, typed value)` to persist,
    /// consumed when Enter commits. `None` when no value edit is active.
    pub fn value_edit_target(&self) -> Option<(String, String)> {
        self.value_edit.as_ref().map(|v| (v.key.clone(), v.input.text().to_string()))
    }

    /// SETTINGS VALUE EDIT cancel: drop the edit and RESTORE the row's cell to the
    /// value it showed before editing (the core has no config to re-gather, so the
    /// stashed `orig` is the source of truth). A no-op when no value edit is active.
    pub fn value_edit_cancel(&mut self) {
        if let Some(ve) = self.value_edit.take() {
            if let Some(r) = self.rows.get_mut(ve.row) {
                r.secondary = ve.orig;
            }
        }
    }

    /// NOTES VERBS round: RENAME the current file — build the fresh minibuffer
    /// state, pre-filled with `current_name` (which becomes the single editable
    /// row's primary cell too — corpus and `rename_edit.input` start in lockstep so
    /// the very first frame already shows the seeded name, not an empty row).
    pub fn new_rename(current_name: String) -> Self {
        let mut s = Self::new_marked(
            OverlayKind::Rename,
            vec![current_name.clone()],
            vec![false],
            vec![false],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.rename_edit = Some(RenameEdit { input: TextBox::seeded(&current_name), orig: current_name });
        s
    }

    /// RENAME MINIBUFFER: mirror the typed filename into `corpus[0]` — RENAME has
    /// no separate `bindings` secondary column, so the live-typed name IS the
    /// primary cell. The tail every mutator below shares. A no-op when no rename
    /// edit is active.
    fn rename_edit_mirror(&mut self) {
        let Some(re) = self.rename_edit.as_ref() else {
            return;
        };
        let text = re.input.text().to_string();
        if let Some(row) = self.rows.get_mut(0) {
            row.accept = text;
        }
    }

    /// RENAME MINIBUFFER: insert `c` at the caret UNLESS it is `/` — a path
    /// separator would let a typed name silently escape into a different
    /// directory, which this verb (a same-directory rename) never does; every other
    /// character is accepted (unlike `value_edit_push`'s digit-only filter — a
    /// filename is free text). The FILTER stays here — `TextBox::insert` itself
    /// accepts any char. A no-op when no rename edit is active.
    pub fn rename_edit_push(&mut self, c: char) {
        let Some(re) = self.rename_edit.as_mut() else {
            return;
        };
        if c != '/' {
            re.input.insert(c);
        }
        self.rename_edit_mirror();
    }

    /// RENAME MINIBUFFER: ⌥⌫ word-delete — drop the trailing word (the word-DELETE
    /// rule), mirroring the change into `corpus[0]`. A no-op when no rename edit is
    /// active.
    pub fn rename_edit_pop_word(&mut self) {
        let Some(re) = self.rename_edit.as_mut() else {
            return;
        };
        re.input.delete_word_back();
        self.rename_edit_mirror();
    }

    /// RENAME MINIBUFFER: delete the char before the caret, mirroring the change
    /// into `corpus[0]`. A no-op when no rename edit is active.
    pub fn rename_edit_pop(&mut self) {
        let Some(re) = self.rename_edit.as_mut() else {
            return;
        };
        re.input.delete_back();
        self.rename_edit_mirror();
    }

    /// ITEM 10 — RENAME MINIBUFFER char/word motion + forward word-delete. A
    /// no-op when no rename edit is active.
    pub fn rename_edit_char_left(&mut self) {
        if let Some(re) = self.rename_edit.as_mut() {
            re.input.char_left();
        }
    }
    pub fn rename_edit_char_right(&mut self) {
        if let Some(re) = self.rename_edit.as_mut() {
            re.input.char_right();
        }
    }
    pub fn rename_edit_word_left(&mut self) {
        if let Some(re) = self.rename_edit.as_mut() {
            re.input.word_left();
        }
    }
    pub fn rename_edit_word_right(&mut self) {
        if let Some(re) = self.rename_edit.as_mut() {
            re.input.word_right();
        }
    }
    pub fn rename_edit_delete_word_forward(&mut self) {
        let Some(re) = self.rename_edit.as_mut() else {
            return;
        };
        re.input.delete_word_forward();
        self.rename_edit_mirror();
    }

    /// RENAME MINIBUFFER commit target: the typed filename, consumed when Enter
    /// commits. `None` when no rename edit is active.
    pub fn rename_edit_target(&self) -> Option<String> {
        self.rename_edit.as_ref().map(|re| re.input.text().to_string())
    }

    /// LINKS V2: summon the Cmd-K minibuffer — build the fresh overlay, pre-filled
    /// with `prefill` (which becomes the single editable row's primary cell too,
    /// mirroring [`Self::new_rename`]'s lockstep seeding). `mode` was already
    /// decided by the caller from buffer state at press time (see
    /// [`LinkEditMode`]'s own doc).
    pub fn new_link_edit(prefill: String, mode: LinkEditMode) -> Self {
        let mut s = Self::new_marked(
            OverlayKind::InsertLink,
            vec![prefill.clone()],
            vec![false],
            vec![false],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.link_edit = Some(LinkEdit { input: TextBox::seeded(&prefill), mode });
        s
    }

    /// LINK MINIBUFFER: mirror the typed URL into `corpus[0]`. A no-op when no
    /// link edit is active.
    fn link_edit_mirror(&mut self) {
        let Some(le) = self.link_edit.as_ref() else {
            return;
        };
        let text = le.input.text().to_string();
        if let Some(row) = self.rows.get_mut(0) {
            row.accept = text;
        }
    }

    /// LINK MINIBUFFER: insert `c` at the caret — NO character filter (unlike
    /// [`Self::rename_edit_push`]'s `/`-rejection: a URL legitimately contains `/`).
    /// Mirrors the change into `corpus[0]`. A no-op when no link edit is active.
    pub fn link_edit_push(&mut self, c: char) {
        let Some(le) = self.link_edit.as_mut() else {
            return;
        };
        le.input.insert(c);
        self.link_edit_mirror();
    }

    /// LINK MINIBUFFER: delete the char before the caret, mirroring the change
    /// into `corpus[0]`. A no-op when no link edit is active.
    pub fn link_edit_pop(&mut self) {
        let Some(le) = self.link_edit.as_mut() else {
            return;
        };
        le.input.delete_back();
        self.link_edit_mirror();
    }

    /// LINK MINIBUFFER: ⌥⌫ word-delete — drop the trailing word of the URL (the
    /// word-DELETE rule), mirroring the change into `corpus[0]`. A no-op when no
    /// link edit is active.
    pub fn link_edit_pop_word(&mut self) {
        let Some(le) = self.link_edit.as_mut() else {
            return;
        };
        le.input.delete_word_back();
        self.link_edit_mirror();
    }

    /// ITEM 10 — LINK MINIBUFFER char/word motion + forward word-delete. A no-op
    /// when no link edit is active.
    pub fn link_edit_char_left(&mut self) {
        if let Some(le) = self.link_edit.as_mut() {
            le.input.char_left();
        }
    }
    pub fn link_edit_char_right(&mut self) {
        if let Some(le) = self.link_edit.as_mut() {
            le.input.char_right();
        }
    }
    pub fn link_edit_word_left(&mut self) {
        if let Some(le) = self.link_edit.as_mut() {
            le.input.word_left();
        }
    }
    pub fn link_edit_word_right(&mut self) {
        if let Some(le) = self.link_edit.as_mut() {
            le.input.word_right();
        }
    }
    pub fn link_edit_delete_word_forward(&mut self) {
        let Some(le) = self.link_edit.as_mut() else {
            return;
        };
        le.input.delete_word_forward();
        self.link_edit_mirror();
    }

    /// LINK MINIBUFFER commit target: the typed URL + the mode it applies to,
    /// consumed when Enter commits. `None` when no link edit is active.
    pub fn link_edit_target(&self) -> Option<(String, LinkEditMode)> {
        self.link_edit
            .as_ref()
            .map(|le| (le.input.text().to_string(), le.mode.clone()))
    }

    /// NAMED SAVE POINTS: summon the "Keep version…" minibuffer — build the
    /// fresh overlay with its single editable row EMPTY (no old name to seed,
    /// unlike [`Self::new_rename`]'s pre-fill; the lockstep corpus[0] ↔
    /// `keep_edit.input` convention is otherwise identical).
    pub fn new_keep_name() -> Self {
        let mut s = Self::new_marked(
            OverlayKind::KeepName,
            vec![String::new()],
            vec![false],
            vec![false],
            Vec::new(),
            Vec::new(),
            None,
        );
        s.keep_edit = Some(KeepEdit { input: TextBox::new() });
        s
    }

    /// KEEP-VERSION MINIBUFFER: mirror the typed name into `corpus[0]`. A no-op
    /// when no keep edit is active.
    fn keep_edit_mirror(&mut self) {
        let Some(ke) = self.keep_edit.as_ref() else {
            return;
        };
        let text = ke.input.text().to_string();
        if let Some(row) = self.rows.get_mut(0) {
            row.accept = text;
        }
    }

    /// KEEP-VERSION MINIBUFFER: insert `c` at the caret — NO character filter (a
    /// name is free display text; even `/` is fine — it never becomes a path,
    /// unlike [`Self::rename_edit_push`]'s rejection). Mirrors the change into
    /// `corpus[0]`. A no-op when no keep edit is active.
    pub fn keep_edit_push(&mut self, c: char) {
        let Some(ke) = self.keep_edit.as_mut() else {
            return;
        };
        ke.input.insert(c);
        self.keep_edit_mirror();
    }

    /// KEEP-VERSION MINIBUFFER: delete the char before the caret, mirroring the
    /// change into `corpus[0]`. A no-op when no keep edit is active.
    pub fn keep_edit_pop(&mut self) {
        let Some(ke) = self.keep_edit.as_mut() else {
            return;
        };
        ke.input.delete_back();
        self.keep_edit_mirror();
    }

    /// KEEP-VERSION MINIBUFFER: ⌥⌫ word-delete — drop the trailing word (the
    /// word-DELETE rule), mirroring the change into `corpus[0]`. A no-op when no
    /// keep edit is active.
    pub fn keep_edit_pop_word(&mut self) {
        let Some(ke) = self.keep_edit.as_mut() else {
            return;
        };
        ke.input.delete_word_back();
        self.keep_edit_mirror();
    }

    /// ITEM 10 — KEEP-VERSION MINIBUFFER char/word motion + forward word-delete.
    /// A no-op when no keep edit is active.
    pub fn keep_edit_char_left(&mut self) {
        if let Some(ke) = self.keep_edit.as_mut() {
            ke.input.char_left();
        }
    }
    pub fn keep_edit_char_right(&mut self) {
        if let Some(ke) = self.keep_edit.as_mut() {
            ke.input.char_right();
        }
    }
    pub fn keep_edit_word_left(&mut self) {
        if let Some(ke) = self.keep_edit.as_mut() {
            ke.input.word_left();
        }
    }
    pub fn keep_edit_word_right(&mut self) {
        if let Some(ke) = self.keep_edit.as_mut() {
            ke.input.word_right();
        }
    }
    pub fn keep_edit_delete_word_forward(&mut self) {
        let Some(ke) = self.keep_edit.as_mut() else {
            return;
        };
        ke.input.delete_word_forward();
        self.keep_edit_mirror();
    }

    /// KEEP-VERSION MINIBUFFER commit target: the OPTIONAL typed name, consumed
    /// when Enter commits — `Some(trimmed)` for real text, `None` for an
    /// empty/whitespace-only input (the plain, nameless keep). Outer `None`
    /// when no keep edit is active at all.
    pub fn keep_edit_target(&self) -> Option<Option<String>> {
        let ke = self.keep_edit.as_ref()?;
        let trimmed = ke.input.text().trim();
        Some((!trimmed.is_empty()).then(|| trimmed.to_string()))
    }
}
