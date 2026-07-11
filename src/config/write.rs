//! src/config/write.rs — FORMAT-PRESERVING WRITES: the "write" seam of the
//! 2026-07 code-organization split (see `config/mod.rs` for the full module
//! doc). Every function here mutates `config.toml` on disk while leaving
//! comments, unrelated keys, and formatting untouched — a hand-edited file
//! stays hand-edited. [`DEFAULT_TEMPLATE`] is the commented starter file
//! seeded on the first write to a missing config. Reading/parsing lives in
//! `config::model`; the launch-time process-global APPLY lives in
//! `config::apply`.
//!
//! **DATA-SAFETY HARDENING round:** every write here now routes through
//! [`crate::fs::write_atomic`] (was a bare `fs.write` before this round) —
//! the durability half of the fix, so a crash mid-write leaves the OLD
//! config or the NEW one, never a torn file. The RECOVERY half
//! (`crate::durable::preserve_corrupt`'s `.corrupt-*` sibling-backup-on-
//! parse-failure contract, applied to `session`/`stats`/`recents`/
//! `mas::GrantStore`/the history log/the scratch stash) is DELIBERATELY NOT
//! applied here: `config.toml` is user-authored, and `Config::load`'s
//! existing behavior on a parse failure — keep the prior in-memory values +
//! show a notice, per `config::model` — is already the right response to a
//! typo, not data loss (the user's own editor buffer + undo history still
//! has their intended text). See `crate::durable`'s module doc for the full
//! reasoning.

use super::Config;
use std::path::Path;

/// The commented template written on the FIRST Settings-open when no config
/// exists, so the user lands in a self-documenting file rather than a blank one.
pub const DEFAULT_TEMPLATE: &str = "\
# awl config — edit as text, then Cmd-S to save (live-reloads keys + folders).
#
# notes_root : where Cmd-N quick-notes live          (default: ~/notes)
# workspace  : the parent dir whose children Cmd-Shift-P switches between
#                                                     (default: the project's parent)
#
# [keys] : rebind a command. The ACTION NAME is the command-palette name
#   lower-cased with spaces as underscores (go_to_file, switch_theme, save,
#   new_note, ...). Every command takes UP TO 2 bindings — slot 1 = NATIVE
#   (macOS Cmd), slot 2 = EMACS — and BOTH fire, so a value is a LIST of up to
#   two chords. A single string is the one-chord form. A CHORD is a key spec:
#   \"Cmd-S\", \"C-t\", \"M-g\", or \"C-x g\" (the C-x prefix plus one key) —
#   modifiers: Cmd-/s- = Super, C- = Ctrl, M-/Option- = Meta, S- = Shift. A bad
#   chord is ignored and the default kept. Open Cmd-P to see each command's name
#   + both effective chords, or Cmd-P -> \"Keybindings…\" to rebind by PRESSING the
#   key (it writes this table for you).
#
# On LINUX (and a web build detected as non-Mac), slot 1 reads as Ctrl-chords
# instead of Cmd-chords (e.g. Save is Ctrl-S, not Cmd-S) — labels everywhere
# resolve automatically, nothing to configure. Where a Ctrl-native chord collides
# with a quiet emacs default (Ctrl-S save vs the emacs C-s search-forward, Ctrl-P
# palette vs C-p previous-line, Ctrl-F search vs C-f forward-char, Ctrl-A select-all
# vs C-a line-start, Ctrl-E inline-code vs C-e line-end, Ctrl-N new-note vs C-n
# next-line, Ctrl-W finish-file vs C-w cut, Ctrl-R replace vs C-r search-backward,
# Ctrl-B bold vs C-b backward-char, Ctrl-G find-next vs C-g cancel, Ctrl-K insert
# link vs C-k kill-line, Ctrl-C copy vs the bare C-c prefix, Ctrl-X cut vs the bare
# C-x prefix, Ctrl-V paste vs C-v page-down), the NATIVE chord wins and the
# displaced emacs default goes quietly empty on Linux — still one `[keys]` line
# away (a `[keys]` chord is ALWAYS
# consulted before the static defaults, on every convention, so it can reclaim any
# displaced chord — e.g. `search_forward = \"C-s\"` gets Ctrl-S back for Search
# forward, which then in turn takes it away from Save's own native slot; there is
# only one physical chord to hand out per collision, whichever command you rebind
# onto it).
#
# linux_keep_emacs : a shorter Linux-only door to the SAME fix — list the bare
#   chords you want to KEEP their emacs meaning, and ONLY that chord's native
#   collision is suppressed (its native command stays reachable by palette/menu/
#   its other chord). On Mac this key is simply ignored. Example, an emacs-hands
#   setup that wants back the whole bare-control nav cluster:
#     linux_keep_emacs = [\"C-f\", \"C-b\", \"C-n\", \"C-p\", \"C-a\", \"C-e\"]
#   The trade: those letters' NATIVE meanings on Linux (Find/Bold/New note/
#   Command palette/Select all/Inline code) fall back to the palette or their
#   other chord instead of a bare Ctrl-letter. LINKS V2 (Cmd-K / Ctrl-K, Insert
#   link…) added a new letter to the same displaced set — an emacs hand who
#   wants C-k kill-line back too just adds it to the list:
#     linux_keep_emacs = [\"C-f\", \"C-b\", \"C-n\", \"C-p\", \"C-a\", \"C-e\", \"C-k\"]
#

# keymap : \"native\" (default) or \"emacs\" — a WHOLE-CATALOG preset over the
#   SAME per-chord door as linux_keep_emacs above, for a Linux emacs hand who
#   wants EVERY displaced bare-control chord back at once rather than naming
#   them one by one: keymap = \"emacs\" keeps every letter the native-wins
#   collision would otherwise claim (Find/Bold/New note/Command palette/Select
#   all/Inline code/Finish file/Replace/Backward char/Search backward/Insert
#   link/Copy/Cut/Paste), reachable instead by palette/menu/their other chord.
#   On Mac this key is inert (no collisions exist there to keep). A [keys] rebind ALWAYS
#   wins over the preset for that one chord — the carve-out an Omarchy/Hyprland
#   user needs, since that compositor forwards Super+C/X/V as Ctrl+C/X/V (the
#   system clipboard), so Copy/Cut/Paste must stay native even under the emacs
#   preset:
#     keymap = \"emacs\"
#     [keys]
#     copy = \"C-c\"
#     cut = \"C-x\"
#     paste = \"C-v\"
#   Also flippable live: Settings -> Keybindings -> Keymap.

# notes_root = \"~/notes\"
# workspace = \"~/code\"

# STICKY PREFERENCES — awl REMEMBERS these across launches and rewrites them here
# whenever you change them live (no settings menu; the action IS the setting). You
# can also hand-edit them. Absent = the built-in default.
#   theme      : the world to launch in (Tawny, Quokka, Gumtree, ...) — set by Cmd-T
#   zoom       : the launch zoom factor (default 0.8) — set by Cmd-= / Cmd--
#   page_mode  : centered page column on/off (default on) — toggled by its command
#   page_width_prose : the writing column MEASURE in characters for a PROSE buffer
#                (markdown / the scratch-or-note surface / an unrecognized plain-text
#                file) — default 70. Set by the Widen page / Narrow page commands
#                while a prose buffer is active.
#   page_width_code : the writing column MEASURE in characters for a CODE buffer
#                (a recognized syntax-highlighted file) — default 100 (rustfmt's own
#                max_width). Which one applies follows the ACTIVE buffer's own kind;
#                zoom is DECOUPLED from both — zoom sizes the glyphs, these size the
#                column.
#   caret_mode : caret look (block | morph | ibeam) — set by the Caret style… /
#                Toggle caret style commands
#   dictionary : spell-check dictionary (en_US | en_GB | en_AU) — default en_US;
#                set via Cmd-P -> \"Dictionary…\"
#   writing_nits : the quiet mechanical-typo underline highlighter on/off
#                (default on) — toggled by the \"Toggle writing nits\" palette command
#   spellcheck : the GLOBAL spell-check on/off (default on) — OFF silences every
#                squiggle (prose and code strings/comments alike) and turns the
#                spell-suggest picker into a calm no-op — toggled by the
#                \"Toggle spellcheck\" palette command
#   history    : automatic LOCAL SNAPSHOTS on save for LOOSE (non-git) files
#                (default on), pruned by the aged retention ladder (resolution
#                thins with age; memory is kept). A file inside a git repo is
#                never snapshotted — git owns its versioning; the timeline reads
#                git history instead.
#   autosave   : quietly SAVE the open file on idle (~1s after you stop typing),
#                window blur, file switch, and quit (default on). Writes are atomic
#                and never overwrite a file changed outside awl (a calm notice instead).
#                The unsaved scratch buffer stashes + restores across launches.
#   project_root : the project folder a BARE launch (no file argument) reopens —
#                set automatically by switch-project (C-x p); an explicit --root
#                flag always wins over this.
#   wysiwyg    : conceal markdown markup off the caret's line (default on) — a
#                heading's `#`, bold/italic `**`/`*`/`_`, inline `` ` `` backticks,
#                and `==highlight==` marks hide until the caret lands on that
#                line; a fenced code block's marker lines hide until the caret is
#                anywhere inside the block. Set false for today's always-visible
#                markup.
#   inline_images : render a markdown `![alt](img.png)` reference as the decoded
#                image in a tall fit-to-column row — its source concealing off the
#                caret's line — instead of plain text (default on, native only).
#                An Obsidian `![alt|300](img.png)` width hint sizes it.
#   code_ligatures : programming ligatures (-> => != >= :: |>) in CODE buffers on
#                the pitch-safe monos (JetBrains Mono, Iosevka) — default on. Set
#                false for ligature-free code. Prose fi/fl ligatures are always on.
#   cjk_priority : the Han-ambiguity tiebreak ladder (default [\"ja\", \"zh-Hans\",
#                \"zh-Hant\", \"ko\"]) — consulted ONLY when an untagged document's
#                CJK content is bare Han (kanji/hanzi with no kana/hangul/bopomofo
#                to disambiguate it); an unrecognized tag in the list is skipped.
#                Used by the write-back-once doc-language tagger on first open of
#                an untagged CJK document (adds a `---\\nlang: ..\\n---` frontmatter
#                block as one undoable edit) and by the per-run render ladder.
#   session_restore : reopen the previous session on a plain relaunch — every
#                open file, the active one, each file's cursor/scroll, and the
#                native window frame (default on). OFF disables both writing
#                the session file (on quit/blur) and reading it back.
#   outline    : the persistent margin table-of-contents (default on) — a faint
#                marginalia TOC that tracks the section you are in.
#   menu_bar   : the awl-rendered menu bar across the top (web/Linux only, default
#                on there; absent on macOS, which has the native menu bar).
#   typewriter_scroll : pin the caret's line centered so the document scrolls
#                under a stationary caret (default OFF, opt-in) — iA Writer-style;
#                the caret rides the doc edges naturally (no centering above the
#                top / below the bottom).
#   stats      : the lifetime stats odometer — chars typed, keystrokes, active-
#                writing time, files touched, caret travel, per-world time
#                (default on). LOCAL + PRIVATE, never uploaded. Native-only. OFF
#                disables all tracking and never writes stats.toml.
#   reduce_motion : settle every caret glide/flinch/pulse INSTANTLY instead of
#                easing (default absent = auto: follows the OS \"Reduce Motion\"
#                accessibility preference where one is reachable — macOS, the
#                web build — else off). Set true/false to override auto either
#                way; also toggleable from Settings -> Editor -> Reduce motion.
#   keymap     : \"native\" (default) or \"emacs\" — see the keymap section above;
#                also toggleable from Settings -> Keybindings -> Keymap.
# theme = \"Tawny\"
# zoom = 0.8
# page_mode = true
# page_width_prose = 70
# page_width_code = 100
# caret_mode = \"block\"
# dictionary = \"en_US\"
# writing_nits = true
# spellcheck = true
# history = true
# autosave = true
# project_root = \"~/code/my-project\"
# wysiwyg = true
# inline_images = true
# code_ligatures = true
# cjk_priority = [\"ja\", \"zh-Hans\", \"zh-Hant\", \"ko\"]
# session_restore = true
# outline = true
# menu_bar = true
# typewriter_scroll = false
# stats = true
# keymap = \"native\"

[keys]
# save = [\"Cmd-S\", \"C-x C-s\"]
# go_to_file = \"Cmd-O\"
# switch_theme = \"Cmd-T\"
# Motions are rebindable too — e.g. reclaim the emacs Option-letter word motion
# (off by default: macOS uses Option-letters for typing accents):
# forward_word = [\"M-Right\", \"M-f\"]
# backward_word = [\"M-Left\", \"M-b\"]
";

impl Config {
    /// Write the commented [`DEFAULT_TEMPLATE`] to `path`, creating parent dirs.
    /// Called by Settings-open when the file does not exist yet.
    pub fn write_default(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            crate::fs::active().create_dir_all(parent)?;
        }
        crate::fs::write_atomic(path, DEFAULT_TEMPLATE.as_bytes())
    }

    /// Merge a freshly-captured `binding` into a command's EXISTING config slots,
    /// honouring the 2-binding cap: the new binding goes FIRST (newest wins), prior
    /// slots follow, duplicates (compared CANONICALLY, so `Cmd-S` == `s-s`) drop, and
    /// the list is capped at 2. So rebinding a command twice keeps the two most recent
    /// custom chords; rebinding to an existing slot is idempotent. Pure — the rebind
    /// menu computes the new slot list with this, then persists it via [`write_binding`].
    pub fn merge_slot(existing: &[String], binding: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![binding.to_string()];
        for ch in existing {
            let dup = out.iter().any(|o| {
                crate::keyspec::canonical_binding(o) == crate::keyspec::canonical_binding(ch)
            });
            if !dup {
                out.push(ch.clone());
            }
        }
        out.truncate(2);
        out
    }

    /// PERSIST a `[keys]` rebind to `path`, format-PRESERVINGLY (comments + other
    /// settings survive): `chords = Some([...])` sets the command's slots, `None`
    /// REMOVES the entry (reset-to-default). The matching non-comment `slug = …` line
    /// is replaced in place; a new entry is inserted under the `[keys]` header (added
    /// if absent). A missing file is seeded from [`DEFAULT_TEMPLATE`] first so the
    /// user keeps the documented comments. Used by the rebind menu's commit + reset.
    pub fn write_binding(path: &Path, slug: &str, chords: Option<&[String]>) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            crate::fs::active().create_dir_all(parent)?;
        }
        let src = match crate::fs::active().read_to_string(path) {
            Ok(s) => s,
            Err(_) => DEFAULT_TEMPLATE.to_string(),
        };
        let new_line = chords.map(|cs| {
            let quoted: Vec<String> = cs.iter().map(|c| format!("\"{c}\"")).collect();
            format!("{slug} = [{}]", quoted.join(", "))
        });
        let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
        // An EXISTING uncommented `slug = …` line (whitespace-tolerant), if any.
        let existing = lines.iter().position(|l| {
            let t = l.trim_start();
            !t.starts_with('#')
                && t
                    .strip_prefix(slug)
                    .map(|r| r.trim_start().starts_with('='))
                    .unwrap_or(false)
        });
        match (existing, new_line) {
            // Replace an existing entry's value.
            (Some(i), Some(line)) => lines[i] = line,
            // Remove an existing entry (reset-to-default).
            (Some(i), None) => {
                lines.remove(i);
            }
            // Insert a new entry under [keys] (append the header if it is missing).
            (None, Some(line)) => {
                match lines.iter().position(|l| l.trim() == "[keys]") {
                    Some(h) => lines.insert(h + 1, line),
                    None => {
                        if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                            lines.push(String::new());
                        }
                        lines.push("[keys]".to_string());
                        lines.push(line);
                    }
                }
            }
            // Nothing to remove: leave the file untouched.
            (None, None) => return Ok(()),
        }
        let mut out = lines.join("\n");
        out.push('\n');
        crate::fs::write_atomic(path, out.as_bytes())
    }

    /// PERSIST a TOP-LEVEL scalar PREFERENCE (theme/zoom/page_mode/caret_mode) to
    /// `path`, format-PRESERVINGLY — the same surgical upsert as [`write_binding`]
    /// but for a top-level `key = value`, so comments + the `[keys]` table + the
    /// other prefs survive. `value` is the already-formatted RHS (a quoted string,
    /// a number, or `true`/`false`). This is the WRITE-ON-CHANGE seam: when the user
    /// switches theme / zooms / toggles page / changes caret, the live `App` calls
    /// this with the settled value (zoom DEBOUNCED in `app.rs`).
    ///
    /// A matching UNCOMMENTED top-level `key = …` line (one that precedes any
    /// `[table]` header, so it can't be a key nested inside `[keys]`) is replaced in
    /// place; otherwise the entry is INSERTED just before the first `[table]` header
    /// (keeping it in the top-level table — a top-level key written AFTER `[keys]`
    /// would parse as a member of that table), or appended if the file has no header.
    /// A missing file is seeded from [`DEFAULT_TEMPLATE`] first so the comments stay.
    pub fn write_pref(path: &Path, key: &str, value: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            crate::fs::active().create_dir_all(parent)?;
        }
        let src = match crate::fs::active().read_to_string(path) {
            Ok(s) => s,
            Err(_) => DEFAULT_TEMPLATE.to_string(),
        };
        let new_line = format!("{key} = {value}");
        let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
        // The first `[table]` header — top-level keys must stay strictly above it.
        let first_header = lines
            .iter()
            .position(|l| l.trim_start().starts_with('['));
        let existing = find_top_level_key(&lines, key);
        match existing {
            Some(i) => lines[i] = new_line,
            None => match first_header {
                // Insert just above the first table header so it stays top-level.
                Some(h) => lines.insert(h, new_line),
                // No header at all: append (optionally after a blank separator).
                None => {
                    if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                        lines.push(String::new());
                    }
                    lines.push(new_line);
                }
            },
        }
        let mut out = lines.join("\n");
        out.push('\n');
        crate::fs::write_atomic(path, out.as_bytes())
    }

    /// REMOVE a top-level scalar PREFERENCE entirely, format-preservingly — the
    /// RESET counterpart to [`write_pref`] for an action whose "built-in default"
    /// is expressed by the key's ABSENCE (`None`) rather than by writing the default
    /// value back, so a future default change flows through instead of pinning a
    /// stale value (used by "Reset page width": clearing `page_width_prose` /
    /// `page_width_code` — whichever matches the active buffer's kind — rather
    /// than writing that class's default back). Mirrors [`write_binding`]'s
    /// reset branch. A matching
    /// UNCOMMENTED top-level `key = …` line is deleted; a MISSING file or an ABSENT
    /// key is a silent no-op (nothing to remove) — never an error.
    pub fn remove_pref(path: &Path, key: &str) -> std::io::Result<()> {
        let Ok(src) = crate::fs::active().read_to_string(path) else {
            return Ok(()); // no file: nothing to remove
        };
        let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
        let Some(i) = find_top_level_key(&lines, key) else {
            return Ok(()); // key absent: nothing to remove
        };
        lines.remove(i);
        let mut out = lines.join("\n");
        out.push('\n');
        crate::fs::write_atomic(path, out.as_bytes())
    }
}

/// Locate an EXISTING uncommented top-level `key = …` line in `lines` — strictly
/// BEFORE any `[table]` header, so `key` can't collide with a same-named entry
/// nested inside e.g. `[keys]`. The shared lookup [`Config::write_pref`] (replace)
/// and [`Config::remove_pref`] (delete) both key off, so the two writers can never
/// disagree on what counts as "the same key" (merge, don't align).
fn find_top_level_key(lines: &[String], key: &str) -> Option<usize> {
    let first_header = lines.iter().position(|l| l.trim_start().starts_with('['));
    lines.iter().enumerate().position(|(i, l)| {
        if let Some(h) = first_header {
            if i >= h {
                return false;
            }
        }
        let t = l.trim_start();
        !t.starts_with('#')
            && t.strip_prefix(key)
                .map(|r| r.trim_start().starts_with('='))
                .unwrap_or(false)
    })
}
