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

    /// The OTHER ending — the target of the "Convert Line Endings" toggle
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
    /// The file this buffer is bound to (for C-x C-s). `None` for scratch.
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
    /// callers (the view sync / spell debounce) detect "did the text change?" with
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
}

impl Buffer {
    /// Empty scratch buffer (no file).
    pub fn scratch() -> Self {
        Self::from_rope(Rope::new(), None)
    }

    /// Load a file into a buffer. A missing file yields an empty buffer bound to
    /// that path (so the first C-x C-s creates it), matching mg behavior.
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
        }
    }

    /// The current edit version (bumped on every content mutation). A cheap change
    /// token for the view-sync / spell-debounce hot path: equal versions ⇒ the
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
/// verbatim.
mod edit;

/// FOCUS-MODE UNIT BOUNDS — the pure `&str` paragraph / sentence helpers. Glob
/// re-exported so the `crate::buffer::paragraph_bounds_str` / `sentence_bounds_str`
/// call sites + the tests resolve them by their bare names.
mod focus;
pub use focus::*;

/// QUICK-NOTE NAMING + FILE MOVES — the pure title-slug + no-clobber move / rename
/// helpers. Glob re-exported so the `crate::buffer::note_stem` / `first_nonempty_line`
/// / `move_file` / `rename_to_stem` (and the in-module `save`) call sites resolve
/// them by their bare names.
mod notes;
pub use notes::*;

#[cfg(test)]
mod tests;
