//! The editor buffer: a ropey `Rope` plus a cursor, plus all the pure editing
//! and motion logic. This module has NO rendering and NO winit dependency, so it
//! is unit-testable in isolation (see the `tests` module at the bottom). The
//! keymap turns key events into method calls on this type.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use ropey::Rope;

/// A buffer's line-ending discipline — the VS Code model. The rope is ALWAYS
/// stored purely `\n`-based (CRLF is normalized to LF on load, see
/// [`Buffer::from_file`]); `Eol` remembers what the FILE used so a save can
/// restore it byte-for-byte. New / no-path buffers default to [`Eol::Lf`].
///
/// This is deliberately a two-value enum: awl recognizes exactly the two endings
/// a real editor round-trips — Unix `\n` and Windows `\r\n`. A lone `\r`, NEL,
/// LS or PS is treated as ordinary CONTENT (never a line break, never an EOL
/// style), matching VS Code — see the "Line endings" section of CLAUDE.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Eol {
    /// Unix `\n`. The default for a fresh / scratch / note buffer.
    #[default]
    Lf,
    /// Windows `\r\n`.
    Crlf,
}

impl Eol {
    /// Detect a file's DOMINANT line ending from its raw (pre-normalization)
    /// bytes-as-str. The rule (documented, VS Code-leaning): CRLF iff the file's
    /// `\r\n` pairs OUTNUMBER its lone `\n` breaks — i.e. `\r\n` is the majority
    /// ending. A pure-LF file (no `\r\n`) is `Lf`; a pure-CRLF file is `Crlf`; a
    /// MIXED file follows the majority (ties, incl. the empty / newline-free file,
    /// fall to `Lf` — the conservative default). Only `\r\n` counts toward CRLF; a
    /// lone `\r` is content and is ignored here.
    pub fn detect(s: &str) -> Eol {
        let total_lf = s.bytes().filter(|&b| b == b'\n').count();
        // Every '\n' immediately preceded by a '\r' is a CRLF pair.
        let crlf = s.match_indices("\r\n").count();
        let lone_lf = total_lf - crlf;
        if crlf > lone_lf {
            Eol::Crlf
        } else {
            Eol::Lf
        }
    }

    /// Encode a PURELY `\n`-based buffer string into this ending's on-disk form:
    /// `Lf` returns it untouched (byte-identical to today); `Crlf` rewrites every
    /// `\n` to `\r\n`. The rope's invariant is pure-`\n`, but text can reach here
    /// already holding a `\r\n` if it entered by a door OTHER than [`from_file`]
    /// (a pasted CRLF clipboard value, a git-history restore); so the `Crlf` arm
    /// COLLAPSES to the pure-`\n` base FIRST ([`normalize_eol`]) rather than assume
    /// it, making the `\n`→`\r\n` rewrite IDEMPOTENT (never `\r\n`→`\r\r\n`). A lone
    /// `\r` that was content is left alone (only `\r\n`/`\n` is touched).
    /// Allocation-light: `Lf`, or a `Crlf` string with no `\n`, borrows.
    ///
    /// [`from_file`]: Buffer::from_file
    pub fn encode<'a>(&self, lf_text: &'a str) -> Cow<'a, str> {
        match self {
            Eol::Lf => Cow::Borrowed(lf_text),
            Eol::Crlf if lf_text.contains('\n') => {
                Cow::Owned(normalize_eol(lf_text).replace('\n', "\r\n"))
            }
            Eol::Crlf => Cow::Borrowed(lf_text),
        }
    }

    /// The short UI label for this ending — `"LF"` / `"CRLF"` — shown by the held
    /// stats HUD's LINE ENDINGS row and named in the capture sidecar's `hud.eol`
    /// field. A pure function, so it is deterministic and capture-safe.
    pub fn label(&self) -> &'static str {
        match self {
            Eol::Lf => "LF",
            Eol::Crlf => "CRLF",
        }
    }

    /// The OTHER ending — the target of the "Line endings…" toggle
    /// (`Lf`↔`Crlf`). awl recognizes exactly two, so a toggle is total.
    pub fn toggled(&self) -> Eol {
        match self {
            Eol::Lf => Eol::Crlf,
            Eol::Crlf => Eol::Lf,
        }
    }
}

/// Normalize a freshly-read file string to the buffer's pure-`\n` model: strip the
/// `\r` from every `\r\n` pair so no CRLF ever enters the rope. A LONE `\r` (or
/// NEL / LS / PS) is left untouched — it is ordinary content, not a line break
/// (the VS Code model). Allocation-light: a file with no `\r` at all borrows.
fn normalize_eol(s: &str) -> Cow<'_, str> {
    if s.contains('\r') {
        Cow::Owned(s.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(s)
    }
}

/// A character classification used for word motion (M-f / M-b). "Word"
/// characters are alphanumeric or underscore; everything else is punctuation or
/// whitespace, matching mg/emacs default word syntax closely enough for v1.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The ONE owner of the backward word-delete boundary (⌥⌫ / M-Backspace): from
/// char index `cursor`, first consume any trailing WHITESPACE run LEFT, then
/// delete exactly ONE token — a run of WORD chars if a word char now sits before
/// the caret, else a run of PUNCTUATION (non-word, non-whitespace) chars. Return
/// the char index the deletion should stop at.
///
/// This matches native macOS Option-Delete (verified 2026-07-22): a caret sitting
/// after a punctuation run deletes only that run, NOT the word before it — so
/// `abc ...⎸` leaves `abc ` (the word survives), while `abc def⎸` still deletes
/// only `def` and `abc def ⎸` deletes `def ` (a word plus the space that
/// introduced it). Punctuation and a word are DISTINCT token classes and never
/// delete together in one stroke; only leading whitespace folds into the token it
/// precedes. The old rule (skip-nonword-then-word) over-deleted the word after a
/// trailing punctuation run — the reported `abc ...⎸` bug.
///
/// `char_at(i)` yields the char at 0-based char index `i` (`i < cursor` always).
/// Abstract over the storage so the rope-backed [`Buffer::delete_word_backward`]
/// and the overlay minibuffer (a `String`) share this rule instead of duplicating
/// it.
pub(crate) fn word_delete_backward_boundary(cursor: usize, char_at: impl Fn(usize) -> char) -> usize {
    let mut i = cursor;
    // 1. Fold any trailing whitespace into the deletion (a word/punct token to the
    //    left "owns" the space that introduced it, mirroring macOS word motion).
    while i > 0 && char_at(i - 1).is_whitespace() {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    // 2. Delete exactly one token — the class of the char now before the caret.
    if is_word_char(char_at(i - 1)) {
        while i > 0 && is_word_char(char_at(i - 1)) {
            i -= 1;
        }
    } else {
        // Punctuation: non-word AND non-whitespace (whitespace was consumed above
        // and a word char ends the run), so this never crosses into an adjacent word.
        while i > 0 && !is_word_char(char_at(i - 1)) && !char_at(i - 1).is_whitespace() {
            i -= 1;
        }
    }
    i
}

/// The ONE owner of the forward word-delete boundary (⌥+forward-Delete /
/// DeleteWordForward): from char index `cursor`, first consume any LEADING
/// WHITESPACE run to the RIGHT, then delete exactly ONE token — a run of WORD
/// chars if a word char now sits at the caret, else a run of PUNCTUATION
/// (non-word, non-whitespace) chars. Return the char index the deletion should
/// stop at.
///
/// The exact forward mirror of [`word_delete_backward_boundary`]: ONE token
/// class per stroke, so `⎸... abc` removes only the `...` run (the word `abc`
/// survives, leaving ` abc`), exactly as `... abc⎸` ⌥⌫ removes only `abc`.
/// Punctuation and a word are DISTINCT classes that never delete together; only
/// the whitespace that INTRODUCES a token folds into it. The old rule
/// (skip-nonword-then-word) over-deleted BOTH the punct run and the word after
/// it in one stroke — the forward twin of the backward bug item 3(a) fixed.
///
/// `char_at(i)` yields the char at 0-based char index `i` (`cursor <= i < len`).
pub(crate) fn word_delete_forward_boundary(
    cursor: usize,
    len: usize,
    char_at: impl Fn(usize) -> char,
) -> usize {
    let mut j = cursor;
    // 1. Fold any leading whitespace into the deletion (the token to the RIGHT
    //    "owns" the space that introduced it, mirroring macOS word motion).
    while j < len && char_at(j).is_whitespace() {
        j += 1;
    }
    if j == len {
        return len;
    }
    // 2. Delete exactly one token — the class of the char now at the caret.
    if is_word_char(char_at(j)) {
        while j < len && is_word_char(char_at(j)) {
            j += 1;
        }
    } else {
        // Punctuation: non-word AND non-whitespace (whitespace was consumed above
        // and a word char ends the run), so this never crosses into an adjacent word.
        while j < len && !is_word_char(char_at(j)) && !char_at(j).is_whitespace() {
            j += 1;
        }
    }
    j
}

/// One recorded edit, the unit of undo. We store the CHANGE (op-based history),
/// not a whole-document snapshot, so memory is proportional to what was edited.
/// At char index `start`, the text `removed` was replaced by the text `inserted`.
/// `cursor_before` is where the cursor sat before the edit (restored on undo);
/// `cursor_after` is where it landed after (restored on redo). Inverting an edit
/// (undo) re-inserts `removed` in place of `inserted` and restores `cursor_before`.
#[derive(Clone, Debug)]
struct Edit {
    start: usize,
    removed: String,
    inserted: String,
    cursor_before: usize,
    cursor_after: usize,
}

/// The direction of the last recorded edit, used for coalescing. An insertion
/// run and a deletion run never merge into the same group.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
}

/// The text buffer + cursor. The cursor is stored as an absolute char index into
/// the rope; line/column are derived on demand. A "goal column" is remembered so
/// that vertical motion (C-n/C-p) keeps a stable column across short lines.
pub struct Buffer {
    rope: Rope,
    /// Absolute char index of the cursor, in `0..=rope.len_chars()`.
    cursor: usize,
    /// Remembered visual column for vertical motion; `None` means "recompute".
    goal_col: Option<usize>,
    /// Remembered VISUAL goal-x (pixels, in the layout oracle's TEXT_LEFT-relative
    /// space) for VISUAL-line vertical motion: it keeps the caret under the same
    /// screen column across consecutive up/down moves through soft-wrapped rows —
    /// the wrap-aware analogue of `goal_col`. The buffer carries it opaquely (it
    /// owns no layout itself); `apply_core`'s oracle reads/writes it via
    /// [`Self::goal_x`] + [`Self::set_cursor_visual`]. `None` means "recompute from
    /// the caret's current visual x". Cleared by every non-vertical motion and edit
    /// (through `clear_kill_flag` / `set_cursor` / `apply_edit`), so it only ever
    /// survives a RUN of consecutive visual vertical moves.
    goal_x: Option<f32>,
    /// CARET WRAP AFFINITY: which visual row the caret RENDERS on when its column
    /// sits exactly on a SHARED soft-wrap boundary (see [`crate::caret::Affinity`]).
    /// `Upstream` (upper row's trailing edge) is set ONLY by a visual line-end
    /// motion (C-e / End / Cmd-Right); every other motion / edit clears it back to
    /// `Downstream`, exactly like `goal_x`'s lifecycle — so it only survives on a
    /// caret parked at a visual-row end. The buffer carries it opaquely; the render
    /// pipeline reads it (via [`Self::affinity`]) to disambiguate the two legit
    /// renders of the boundary column.
    affinity: crate::caret::Affinity,
    /// The file this buffer is bound to (for Cmd-S). `None` for scratch.
    path: Option<PathBuf>,
    /// This buffer's line-ending discipline (the VS Code model): the rope is
    /// ALWAYS pure-`\n`, and `eol` remembers the file's original ending so a save
    /// restores it byte-for-byte ([`Self::disk_bytes`]). Detected on load
    /// ([`Self::from_file`]); [`Eol::Lf`] for a fresh / scratch / note buffer.
    eol: Eol,
    /// QUICK NOTE target directory: set when this buffer is a freshly-summoned
    /// scrap note (C-x n) that has not been named yet. While `path` is `None` and
    /// this is `Some`, the first `save()` DERIVES the filename from the buffer's
    /// first non-empty line (slugified) under this directory — "capture first,
    /// name later". Stays set after the first save so the windowed app keeps
    /// auto-saving the note; the filename then LOCKS (save writes the bound path).
    /// `None` for ordinary files and scratch buffers (which never auto-name).
    note_dir: Option<PathBuf>,
    /// Kill buffer (C-k / C-y). Appended to by consecutive kills.
    kill: String,
    /// Whether the previous command was a kill, so consecutive C-k appends.
    last_was_kill: bool,
    /// Dirty flag (unsaved changes).
    dirty: bool,
    /// Selection mark: the anchor char index. The selection is the span between
    /// `anchor` and `cursor`. `None` means no active selection. Set by C-Space
    /// (set-mark) or a Shift+motion / mouse drag; cleared by C-g or a plain
    /// motion that does not extend.
    anchor: Option<usize>,
    /// Monotonic edit version, bumped on every mutation of the rope CONTENT. Lets
    /// callers (the view sync / eager spell rescan) detect "did the text change?" with
    /// a cheap `u64` compare instead of cloning + comparing the whole rope string
    /// each keystroke. Cursor/selection-only changes do NOT bump it.
    version: u64,
    /// Undo stack: completed (and the in-progress) edit groups, oldest first.
    /// Each group is a run of coalesced [`Edit`]s applied together; one undo pops
    /// and inverts the whole top group. A fresh edit may extend the top group (see
    /// coalescing rules in [`record_edit`]) or push a new one.
    undo_stack: Vec<Vec<Edit>>,
    /// Redo stack: groups popped by undo, ready to re-apply. Cleared by any NEW
    /// edit (linear, modern-editor history — undo is not itself undoable).
    redo_stack: Vec<Vec<Edit>>,
    /// True when the top undo group is "open" and a contiguous same-direction edit
    /// may coalesce into it. Sealed (set false) by [`seal_undo_group`] after any
    /// non-edit command, and internally when a group-breaking edit occurs.
    undo_group_open: bool,
    /// The direction of the last recorded edit, for coalescing decisions.
    last_edit_kind: Option<EditKind>,
    /// COLLAPSED SECTIONS (view state, never file content): the set of ATX heading
    /// LOGICAL LINES whose sections are folded. Pure in-memory render state for the
    /// app run — it survives a buffer switch (the whole `Buffer` parks in the
    /// registry) but is NOT serialized to disk / session and is NOT on the undo
    /// timeline (undo replays rope `Edit`s, never this field). Empty for the
    /// overwhelming common case, so every fold read short-circuits to a no-op. The
    /// section extent + auto-expand rules live in [`crate::fold`]; this buffer owns
    /// only the set + the caret-relative gestures over it.
    folds: std::collections::BTreeSet<usize>,
}

impl Buffer {
    /// Empty scratch buffer (no file).
    pub fn scratch() -> Self {
        Self::from_rope(Rope::new(), None)
    }

    /// Load a file into a buffer. A missing file yields an empty buffer bound to
    /// that path (so the first Cmd-S creates it), matching mg behavior.
    ///
    /// LINE ENDINGS (VS Code model): the file's DOMINANT ending is detected
    /// ([`Eol::detect`]) and remembered, then every `\r\n` is normalized to `\n`
    /// ([`normalize_eol`]) BEFORE the text enters the rope — so the buffer is
    /// purely `\n`-based and agrees with the `\n`-only renderer by construction.
    /// A save restores the remembered ending ([`Self::disk_bytes`]), so a CRLF
    /// file round-trips byte-for-byte. A missing file defaults to [`Eol::Lf`].
    pub fn from_file(path: &Path) -> Self {
        let (rope, eol) = match crate::fs::active().read_to_string(path) {
            Ok(s) => (Rope::from_str(&normalize_eol(&s)), Eol::detect(&s)),
            Err(_) => (Rope::new(), Eol::Lf),
        };
        let mut buf = Self::from_rope(rope, Some(path.to_path_buf()));
        buf.eol = eol;
        buf
    }

    /// Build directly from a string (used in tests and scratch construction).
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        Self::from_rope(Rope::from_str(s), None)
    }

    fn from_rope(rope: Rope, path: Option<PathBuf>) -> Self {
        Self {
            rope,
            cursor: 0,
            goal_col: None,
            goal_x: None,
            affinity: crate::caret::Affinity::Downstream,
            path,
            eol: Eol::Lf,
            note_dir: None,
            kill: String::new(),
            last_was_kill: false,
            dirty: false,
            anchor: None,
            version: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_group_open: false,
            last_edit_kind: None,
            folds: std::collections::BTreeSet::new(),
        }
    }

    /// The current edit version (bumped on every content mutation). A cheap change
    /// token for the view-sync / spell-rescan hot path: equal versions ⇒ the
    /// rope text is unchanged, so a full-string compare can be skipped.
    pub fn version(&self) -> u64 {
        self.version
    }

    // --- Accessors --------------------------------------------------------

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn line_count(&self) -> usize {
        // ropey counts a trailing newline as ending a line; for display we want
        // at least one line.
        self.rope.len_lines().max(1)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// This buffer's line-ending discipline (see [`Eol`]). The rope is always
    /// pure-`\n`; this reports what a save will restore.
    pub fn eol(&self) -> Eol {
        self.eol
    }

    /// Switch this buffer's line-ending discipline (the palette's "Convert Line
    /// Endings" command calls this). The rope is UNCHANGED — it is always pure
    /// `\n`; only the on-disk encoding differs — so this is metadata, not a text
    /// edit. Design choice (documented): EOL is NOT part of the undo history, and
    /// Cmd-Z does not restore it (mirroring VS Code, where the ending is a
    /// document-level setting, not an undoable edit; the rope content is
    /// byte-identical either way, so there is nothing in the text for undo to
    /// restore). A real change bumps `version` + marks the buffer dirty so the
    /// autosave engine rewrites the file with the new ending on the next flush; a
    /// no-op switch (same ending) leaves everything untouched.
    pub fn set_eol(&mut self, eol: Eol) {
        if self.eol == eol {
            return;
        }
        self.eol = eol;
        // The DISK bytes changed even though the rope content did not; bump the
        // content version so the autosave engine (which keys on `version`) picks
        // the rewrite up on the next idle/blur/switch/quit, and mark dirty.
        self.dirty = true;
        self.version += 1;
    }

    /// The buffer's content encoded to its ON-DISK byte form: the pure-`\n` rope
    /// string with this buffer's [`Eol`] restored ([`Eol::encode`]). The ONE owner
    /// of "buffer content → disk bytes" — every save path routes through it (manual
    /// [`Self::save`], the autosave engine, the scratch stash), so a CRLF file is
    /// rewritten with `\r\n` and an LF file is byte-identical to today. Distinct
    /// from [`Self::text`], which is the internal pure-`\n` view every other reader
    /// (spell / search / markdown / render) wants.
    pub fn disk_bytes(&self) -> Vec<u8> {
        let text = self.rope.to_string();
        match self.eol.encode(&text) {
            // Lf (or a `\n`-free Crlf buffer): reuse the rope string's own buffer.
            Cow::Borrowed(_) => text.into_bytes(),
            // Crlf with real `\n`s: the freshly-rewritten `\r\n` string.
            Cow::Owned(s) => s.into_bytes(),
        }
    }

    /// The buffer's DISPLAY NAME for the page-mode orientation gutter: the bound
    /// file's name (`notes.md`) for a saved file, else the name a quick note WOULD
    /// derive on its first save — the slugified first non-empty line plus `.md`, or
    /// the `"scratch"` placeholder for an empty / untitled buffer. So a scratch
    /// surface or an unsaved note still shows a stable, save-consistent name in the
    /// gutter BEFORE it is ever written (matching [`Self::save`]'s naming).
    pub fn display_name(&self) -> String {
        if let Some(p) = &self.path {
            return p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "scratch".to_string());
        }
        let stem = match first_nonempty_line(&self.rope.to_string()) {
            Some(line) => note_stem(line),
            None => "scratch".to_string(),
        };
        format!("{stem}.md")
    }

    /// True when this buffer is a MARKDOWN document. awl is a prose-first writing
    /// app, so the rule is unified and prose-leaning: a buffer with NO path — the
    /// bare SCRATCH launch surface or an unsaved QUICK NOTE — defaults to markdown,
    /// styling `# title` / **bold** as you type on the blank writing surface; a
    /// SAVED file is markdown only by its `.md` / `.markdown` extension (case-
    /// insensitive). So a `.rs` / `.txt` / `.env` file (a path with a non-markdown
    /// extension) stays NOT markdown — code/.env files always open WITH a path, so
    /// they are unaffected. (The no-path arm subsumes [`Self::is_note`] — a note
    /// is always unsaved-then-`.md` — and is what makes a note read as markdown
    /// from the first keystroke, before its first save derives a `.md` path.)
    /// Gates the renderer's markdown styling pass. Syntax highlighting stays
    /// path-based ([`Self::syntax_lang`]), so a no-path buffer reports no code
    /// language and is never code-highlighted — markdown and code remain mutually
    /// exclusive even for the scratch surface.
    pub fn is_markdown(&self) -> bool {
        match self.path.as_deref() {
            // Scratch (the blank writing surface) or an unsaved note: prose-first,
            // so the writing surface defaults to markdown.
            None => true,
            // A saved file is markdown only by a `.md` / `.markdown` extension.
            Some(p) => p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
                .unwrap_or(false),
        }
    }

    /// The CODE language for syntax highlighting, or `None` when this buffer must
    /// NOT be highlighted — decided purely by the file extension via
    /// [`crate::syntax::Lang::from_path`]. The gate excludes `.env`, `.md`/
    /// `.markdown` (own markdown styling), `.txt`, and any unrecognized / scratch
    /// buffer, so those render byte-identically. Markdown and code are mutually
    /// exclusive: a `.md` buffer is [`Self::is_markdown`] with no `syntax_lang`.
    pub fn syntax_lang(&self) -> Option<crate::syntax::Lang> {
        self.path.as_deref().and_then(crate::syntax::Lang::from_path)
    }

    /// Which STICKY page-width CLASS this buffer draws its measure from — see
    /// [`crate::page::PageClass`]. Delegates to the ONE classifier
    /// (`PageClass::of_syntax`), driven by [`Self::syntax_lang`], so it can
    /// never disagree with the syntax-highlighting gate: a recognized CODE
    /// file is `Code`; markdown / the no-path scratch-or-note surface / an
    /// unrecognized plain-text file is `Prose`.
    pub fn page_class(&self) -> crate::page::PageClass {
        crate::page::PageClass::of_syntax(self.syntax_lang())
    }

    /// Re-point the buffer at a new file path. Future saves write here. Used by a
    /// note's first auto-save (once its filename is derived) and by C-x m MOVE
    /// (so editing continues at the moved path). The app keeps its own `file`
    /// notion in sync alongside this.
    pub fn set_path(&mut self, p: PathBuf) {
        self.path = Some(p);
    }

    /// Mark this buffer as a freshly-summoned QUICK NOTE living under `dir`: it
    /// has no filename yet; the first non-empty line names it on the first save.
    pub fn set_note_dir(&mut self, dir: PathBuf) {
        self.note_dir = Some(dir);
    }

    /// True when this buffer is a QUICK NOTE (auto-saved; auto-named on first save
    /// from its first line). Ordinary files and scratch buffers are not notes.
    pub fn is_note(&self) -> bool {
        self.note_dir.is_some()
    }

    /// Reset this buffer to a fresh, EMPTY, unsaved quick note bound to `dir`
    /// (no file yet). Used by C-x n to start capturing immediately; the filename
    /// is derived from the first non-empty line on the first save.
    pub fn start_note(&mut self, dir: PathBuf) {
        *self = Self::from_rope(Rope::new(), None);
        self.note_dir = Some(dir);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn kill_buffer(&self) -> &str {
        &self.kill
    }

    /// Pure setter for the kill ring's top entry. Used by the app's clipboard
    /// bridge to load an external OS-clipboard value before a yank. Overwrites
    /// (does not append) and MUST NOT touch `last_was_kill`: loading an external
    /// value is not a kill, so a subsequent C-k must start a fresh kill rather
    /// than chaining onto this. No winit/gpu/arboard here — buffer stays pure.
    ///
    /// NORMALIZES any `\r\n` the external source used to the rope's pure-`\n`
    /// invariant ([`normalize_eol`], matching [`Self::from_file`]) — a pasted
    /// Windows/CRLF clipboard value can therefore never introduce a real `\r\n`
    /// into the rope on yank (which a `Crlf` save would otherwise double-encode).
    /// A lone `\r` stays content (the established lone-CR decision).
    pub fn set_kill(&mut self, s: &str) {
        self.kill.clear();
        self.kill.push_str(&normalize_eol(s));
    }

    /// Cursor as (line, column) both 0-based, column measured in chars.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.rope.char_to_line(self.cursor);
        let line_start = self.rope.line_to_char(line);
        (line, self.cursor - line_start)
    }

    // --- Folds (collapsed sections; view state — see the `folds` field + `fold`) -

    /// The set of folded heading LOGICAL LINES (read-only). Empty when nothing is
    /// collapsed.
    pub fn folds(&self) -> &std::collections::BTreeSet<usize> {
        &self.folds
    }

    /// True when there is at least one collapsed section. The cheap short-circuit
    /// every render / reveal path checks first so an unfolded document pays nothing.
    pub fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }

    /// The per-logical-line HIDDEN mask for the current fold set — `true` where a
    /// line is collapsed inside a folded section (never the heading line itself).
    /// Empty-of-`true` when nothing is folded. The render builds its fold-filtered
    /// text from this ([`crate::fold::Filter`]).
    pub fn hidden_lines(&self) -> Vec<bool> {
        if self.folds.is_empty() {
            return Vec::new();
        }
        let levels = self.heading_levels();
        crate::fold::hidden_lines(&levels, &self.folds)
    }

    /// This buffer's per-logical-line heading levels ([`crate::fold::heading_levels`]
    /// over the current text, gated by markdown-ness) — the one input the fold logic
    /// reads. Recomputed per gesture (fold ops are rare, not a hot path).
    fn heading_levels(&self) -> Vec<u8> {
        crate::fold::heading_levels(&self.text(), self.is_markdown())
    }

    /// The "… N lines" TAIL data for the current fold set — one
    /// `(full-doc heading line, hidden line count)` per VISIBLE folded heading,
    /// ascending ([`crate::fold::fold_tails`]). Empty when nothing is folded. The
    /// render remaps each heading into filtered space to hang its quiet tail glyph.
    pub fn fold_tails(&self) -> Vec<(usize, usize)> {
        if self.folds.is_empty() {
            return Vec::new();
        }
        let levels = self.heading_levels();
        crate::fold::fold_tails(&levels, &self.folds)
    }

    /// Map a VISIBLE (fold-filtered) line index — what a pointer hit-test returns,
    /// since the render shapes the filtered document — back to its FULL-document line.
    /// The IDENTITY when nothing is folded, so every unfolded click is byte-identical.
    /// ([`crate::fold::visible_to_full`].)
    pub fn visible_line_to_full(&self, visible_line: usize) -> usize {
        if self.folds.is_empty() {
            return visible_line;
        }
        let levels = self.heading_levels();
        let hidden = crate::fold::hidden_lines(&levels, &self.folds);
        crate::fold::visible_to_full(&hidden, visible_line)
    }

    /// CLICK-TO-EXPAND hit test: given a pointer's VISIBLE `(line, col)` (as the
    /// render's hit-test yields), return the FULL-document heading line to EXPAND when
    /// the click landed on a collapsed heading's affordance — the "… N lines" tail /
    /// chevron cluster, which hangs to the RIGHT of the heading text (so `col` at or
    /// past the heading's own character length). `None` when nothing is folded, the
    /// clicked visible line is not a collapsed heading, or the click is ON the heading
    /// text (which places the caret for editing, unchanged). The affordance region is
    /// "past the heading text" — the tail + chevron both live there, so one rule covers
    /// both without pixel geometry.
    pub fn fold_tail_hit(&self, visible_line: usize, col: usize) -> Option<usize> {
        if self.folds.is_empty() {
            return None;
        }
        let full = self.visible_line_to_full(visible_line);
        if !self.folds.contains(&full) {
            return None; // not a collapsed heading's row
        }
        // The affordance sits past the heading's own text. `col` is a CHAR column on
        // the logical line; a click to the right of the last glyph maps to the line's
        // char length (the tail/chevron region), so `col >= len` is the hit.
        (col >= self.line_len(full)).then_some(full)
    }

    /// Expand (unfold) the section headed by `heading_line`, parking the caret on that
    /// heading — the click-to-expand affordance's action. Returns `true` when that line
    /// was folded (and is now open), `false` (no-op) otherwise.
    pub fn unfold_at(&mut self, heading_line: usize) -> bool {
        if self.folds.remove(&heading_line) {
            self.set_cursor(self.line_start(heading_line));
            true
        } else {
            false
        }
    }

    /// Toggle the fold on the heading enclosing the caret (fold ⇄ unfold). No-op on
    /// a non-markdown buffer or a caret with no enclosing heading. Returns the
    /// toggled heading line, or `None` when nothing was toggled. On a FOLD (not an
    /// unfold), the caret is parked on the heading line so it is not left inside the
    /// section it just collapsed (which the auto-expand would immediately reveal) —
    /// the standard fold gesture leaves you on the collapsed heading.
    pub fn toggle_fold_at_cursor(&mut self) -> Option<usize> {
        let levels = self.heading_levels();
        let (line, _) = self.cursor_line_col();
        let h = crate::fold::toggle_at(&levels, &mut self.folds, line)?;
        if self.folds.contains(&h) {
            self.set_cursor(self.line_start(h));
        }
        Some(h)
    }

    /// "Collapse other sections": fold every heading except the caret's section and
    /// its enclosing chain (the daily-notes gesture). No-op on a non-markdown buffer.
    pub fn collapse_other_sections(&mut self) {
        if !self.is_markdown() {
            return;
        }
        let levels = self.heading_levels();
        let (line, _) = self.cursor_line_col();
        self.folds = crate::fold::collapse_others(&levels, line);
    }

    /// Unfold everything.
    #[allow(dead_code)] // used by the render increment + palette "Unfold all"
    pub fn unfold_all(&mut self) {
        self.folds.clear();
    }

    /// AUTO-EXPAND: reveal any fold that hides the caret line (and prune stale
    /// entries whose heading was edited away). Cheap no-op when nothing is folded,
    /// so it is safe to call after every action. Returns true when the fold set
    /// changed. See [`crate::fold::expand_containing`].
    pub fn reveal_cursor(&mut self) -> bool {
        if self.folds.is_empty() {
            return false;
        }
        let levels = self.heading_levels();
        let mut changed = crate::fold::prune_stale(&levels, &mut self.folds);
        let (line, _) = self.cursor_line_col();
        changed |= crate::fold::expand_containing(&levels, &mut self.folds, line);
        changed
    }

    /// AUTO-EXPAND: reveal any fold the active selection would span INVISIBLY, so a
    /// selection never crosses hidden lines. No-op when nothing is folded or there
    /// is no selection. See [`crate::fold::expand_range`].
    pub fn reveal_selection(&mut self) -> bool {
        if self.folds.is_empty() {
            return false;
        }
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        let lo = self.rope.char_to_line(start);
        let hi = self.rope.char_to_line(end);
        let levels = self.heading_levels();
        crate::fold::expand_range(&levels, &mut self.folds, lo, hi)
    }

    #[allow(dead_code)]
    pub fn cursor_char(&self) -> usize {
        self.cursor
    }

    /// The cursor's absolute BYTE offset into the document text (`text()`), the
    /// coordinate [`crate::markdown::link_at`] and other byte-indexed span readers
    /// want. Derived from the char cursor via the rope's char→byte map, so it
    /// agrees with the `\n`-only byte offsets `markdown::spans` produces.
    pub fn cursor_byte(&self) -> usize {
        self.rope.char_to_byte(self.cursor)
    }

    /// The absolute BYTE offset of an arbitrary CHAR index (`text()` coordinates) —
    /// the same rope char→byte map [`Self::cursor_byte`] uses, for a byte-indexed
    /// span reader (e.g. [`crate::markdown::link_at`]) that needs a HIT-TESTED char,
    /// not the cursor's. Clamped to the document length so an off-the-end index is
    /// safe.
    pub fn char_to_byte(&self, ch: usize) -> usize {
        self.rope.char_to_byte(ch.min(self.rope.len_chars()))
    }

    // --- Internal line geometry helpers -----------------------------------

    /// Char index of the start of `line`.
    fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    /// Number of chars on `line` EXCLUDING the trailing newline (if any).
    fn line_len(&self, line: usize) -> usize {
        let total_lines = self.rope.len_lines();
        if line >= total_lines {
            return 0;
        }
        let start = self.rope.line_to_char(line);
        let end = if line + 1 < total_lines {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };
        let mut len = end - start;
        // Trim a single trailing '\n' from the count.
        if len > 0 {
            let last = self.rope.char(end - 1);
            if last == '\n' {
                len -= 1;
            }
        }
        len
    }

    fn clear_kill_flag(&mut self) {
        self.last_was_kill = false;
        // A non-vertical motion ends any visual vertical run, so the sticky
        // visual goal-x is recomputed on the next C-n/C-p. (The visual vertical
        // path uses `set_cursor_visual`, which bypasses this and KEEPS goal_x.)
        self.goal_x = None;
        // A plain motion / edit also drops any caret wrap-affinity: the caret is no
        // longer parked at a visual-row END, so at a shared boundary it renders on
        // the LOWER row again (the default bias). The visual line-END motion re-sets
        // `Upstream` AFTER its `set_cursor`, so only that survives (see
        // `crate::caret::Affinity`).
        self.affinity = crate::caret::Affinity::Downstream;
    }

    /// The caret's current wrap AFFINITY (which visual row it renders on at a
    /// shared soft-wrap boundary — see [`crate::caret::Affinity`]). Read by the
    /// render pipeline's caret placement; `Downstream` for any caret not parked at
    /// a visual-row end.
    pub fn affinity(&self) -> crate::caret::Affinity {
        self.affinity
    }

    /// Mark the caret's wrap AFFINITY. Called ONLY by the visual line-END motion
    /// (C-e / End / Cmd-Right) with `Upstream`, AFTER `set_cursor` has parked the
    /// caret at the boundary column (so it survives that call's clear). Every other
    /// motion / edit resets it to `Downstream` through `clear_kill_flag` /
    /// `set_cursor_visual` / `apply_edit`.
    pub fn set_affinity(&mut self, affinity: crate::caret::Affinity) {
        self.affinity = affinity;
    }

    // --- Word / line bounds (for double / triple click) -------------------

    /// The char range `[start, end)` of the word containing or adjacent to
    /// `idx`. If `idx` is on a word char, returns that whole word; otherwise the
    /// run of non-word chars under it. Used by double-click select-word.
    pub fn word_bounds(&self, idx: usize) -> (usize, usize) {
        let len = self.rope.len_chars();
        if len == 0 {
            return (0, 0);
        }
        let idx = idx.min(len);
        // Decide which class we are selecting: prefer the char AT idx, else the
        // char before it (so a click at end-of-word still grabs the word).
        let class_at = |i: usize| -> Option<bool> {
            if i < len {
                Some(is_word_char(self.rope.char(i)))
            } else {
                None
            }
        };
        let want = class_at(idx)
            .or_else(|| if idx > 0 { class_at(idx - 1) } else { None })
            .unwrap_or(true);
        let mut start = idx;
        while start > 0 && is_word_char(self.rope.char(start - 1)) == want {
            start -= 1;
        }
        let mut end = idx;
        while end < len && is_word_char(self.rope.char(end)) == want {
            end += 1;
        }
        (start, end)
    }

    /// The char range `[start, end)` of the line containing `idx`, INCLUDING the
    /// trailing newline if present (so triple-click selects the whole line).
    pub fn line_bounds(&self, idx: usize) -> (usize, usize) {
        let idx = idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        let start = self.line_start(line);
        let total_lines = self.rope.len_lines();
        let end = if line + 1 < total_lines {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };
        (start, end)
    }

    /// Replace the ENTIRE buffer contents with `new` as ONE atomic, undoable edit,
    /// then seal the group so it is its own undo step. The cursor lands at the end
    /// of the inserted text (callers that care reposition it afterward). Used by
    /// find-and-replace, which computes the post-replace document wholesale; a
    /// no-op replacement (identical text) is the caller's to skip.
    pub fn set_text(&mut self, new: &str) {
        self.clear_kill_flag();
        self.goal_col = None;
        self.anchor = None;
        let before = self.cursor;
        let len = self.rope.len_chars();
        let after = new.chars().count();
        self.apply_edit(0, len, new, before, after);
        self.seal_undo_group();
    }

    /// Select an explicit char range: set the mark at `start` and the cursor at
    /// `end` (both clamped). Used by double/triple-click and the `--sel` hook.
    pub fn select_range(&mut self, start: usize, end: usize) {
        self.clear_kill_flag();
        self.goal_col = None;
        let max = self.rope.len_chars();
        self.anchor = Some(start.min(max));
        self.cursor = end.min(max);
    }

    // --- Files ------------------------------------------------------------

    /// Save to the bound path. For a QUICK NOTE that has not been named yet
    /// (`path` is None but `note_dir` is set), DERIVE the filename from the first
    /// non-empty line — slugified, collision-suffixed — under `note_dir`, bind it,
    /// and write there; an EMPTY note bails (no file written, no litter). Returns
    /// Err if there is no path and no name can be derived.
    pub fn save(&mut self) -> anyhow::Result<()> {
        if self.path.is_none() {
            if let Some(dir) = self.note_dir.clone() {
                let text = self.rope.to_string();
                match first_nonempty_line(&text) {
                    // A non-empty first line names the file. A single word counts
                    // ("foo" -> foo.md). A first line with no alphanumeric content
                    // (e.g. punctuation-only) yields no slug, so FALL BACK to the
                    // "scratch" placeholder (scratch.md / scratch-2.md / …).
                    Some(line) => {
                        let stem = note_stem(line);
                        crate::fs::active().create_dir_all(&dir)?;
                        let path = unique_path(&dir, &stem, "md");
                        self.path = Some(path);
                    }
                    // A truly empty note (no non-whitespace anywhere) is NEVER
                    // written — no litter.
                    None => anyhow::bail!("empty note: nothing to save yet"),
                }
            }
        }
        match &self.path {
            Some(p) => {
                // ATOMIC: temp sibling + rename, so a crash mid-save leaves the
                // old file or the new one — never a truncated half-write. The
                // buffer's remembered line ending is restored here ([`disk_bytes`]),
                // so a CRLF file round-trips byte-for-byte.
                crate::fs::write_atomic(p, &self.disk_bytes())?;
                self.dirty = false;
                Ok(())
            }
            None => anyhow::bail!("no file bound to this buffer (scratch)"),
        }
    }

    /// SAVE-FEEDBACK round: manual save on the TRUE scratch surface (no path,
    /// never named as a note) converts it into a real note FIRST, then saves —
    /// reusing the exact auto-name recipe [`Self::set_note_dir`] + [`Self::save`]
    /// already give a C-x n note (the same one `App::ensure_note_named_before_paste`
    /// established for the paste-image door, generalized here to manual save). A
    /// buffer that is ALREADY a note (named or not) or already pathed is left
    /// untouched — this only ever promotes a true scratch buffer, and only once
    /// (`is_note()` is true from then on, so a second call is a plain `save()`).
    /// `notes_root` need not exist yet: creating it is best-effort (mirroring
    /// `App::new_note`); if it truly can't be created or written to, that failure
    /// surfaces as the same `Err` `save` already returns, for the caller to turn
    /// into a calm notice — never a terminal print.
    pub fn save_as_note(&mut self, notes_root: &Path) -> anyhow::Result<()> {
        if !self.is_note() {
            let _ = crate::fs::active().create_dir_all(notes_root);
            self.set_note_dir(notes_root.to_path_buf());
        }
        self.save()
    }
}

/// SELECTION + CURSOR PLACEMENT — the mark / region model and the raw cursor
/// setters (`set_cursor` / `set_cursor_visual` / `delete_selection` / `kill_region`
/// …). Inherent methods on [`Buffer`], carved out verbatim.
mod selection;

/// CURSOR MOTION — the non-mutating caret movements (char / line / buffer / word).
/// Inherent methods on [`Buffer`], carved out verbatim.
mod motion;

/// UNDO / REDO ENGINE — the `apply_edit` mutation choke point + the op-based
/// history (coalescing) + undo / redo / seal. Inherent methods on [`Buffer`],
/// carved out verbatim; `apply_edit` is `pub(super)` for the edit / selection
/// modules + this root.
mod undo;

/// TEXT EDITING OPS — self-insert / tab / delete / kill-line / yank, all routed
/// through [`Buffer::apply_edit`]. Inherent methods on [`Buffer`], carved out
/// verbatim — plus the free [`is_url`] URL-shape helper (paste-URL-over-selection
/// → markdown-link), re-exported so its call sites + tests resolve it bare.
mod edit;
// Public URL-shape helper (used bare by `yank` inside `edit`; re-exported for its
// tests + future clipboard call sites). Allowed-unused: the binary itself only
// reaches it through `edit`'s own module-local name.
#[allow(unused_imports)]
pub use edit::is_url;

/// QUICK-NOTE NAMING + FILE MOVES — the pure title-slug + no-clobber move / rename
/// helpers. Glob re-exported so the `crate::buffer::note_stem` / `first_nonempty_line`
/// / `move_file` / `rename_to_stem` (and the in-module `save`) call sites resolve
/// them by their bare names.
mod notes;
pub use notes::*;

#[cfg(test)]
mod tests;
