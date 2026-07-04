//! The MULTI-BUFFER REGISTRY — the identity + eviction policy shared by the
//! live [`crate::app::App`] and the headless `--keys` replay
//! ([`crate::main::run::replay_keys`]), so "open a file that's already open
//! switches to its live buffer" behaves IDENTICALLY in both (same behavior ⇒
//! same code, per CLAUDE.md's engineering principles — no aligned copies).
//!
//! The ACTIVE buffer is never held here: it stays exactly where it always
//! lived (`App::buffer` / the replay's `buffer` local), so that seam's name +
//! type stay stable and the diff introducing this module is reviewable. This
//! module owns only the BACKGROUNDED buffers — the other N-1 open files —
//! keyed by a stable identity so re-opening one finds its live state instead
//! of re-reading disk.
//!
//! V1 SCOPE: the registry is the state model, not chrome (no tab strip / no
//! session restore / no daemon — see the arc's later items). It is generic
//! over a small `extra` payload (`T`) so the live App can carry its own
//! per-buffer bookkeeping (scroll / spell cache / autosave versions — see
//! `app::files::BufferExtra`) while the headless replay carries none (`()`).

use std::path::{Component, Path, PathBuf};

use crate::buffer::Buffer;

/// A buffer's stable identity for registry lookups. A SAVED file is keyed by
/// its bound path (NORMALIZED — absolutized + canonicalized where possible,
/// see [`BufferKey::path`]); the ONE
/// pathless "scratch" writing surface (the launch buffer, or the persistent
/// stash it restores from) is keyed by the `Scratch` sentinel — there is only
/// ever one such identity, mirroring the one persistent scratch stash
/// (`fs::scratch_stash_path`). A pathless QUICK NOTE that hasn't been named
/// yet has NO stable identity and is deliberately never registered (see
/// [`BufferKey::of`]).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BufferKey {
    Path(NormPath),
    Scratch,
}

/// A normalized path (see [`normalize_path`]) — the identity payload of
/// `BufferKey::Path`.
/// Rust always makes an enum variant's fields as visible as the enum itself
/// (there is no way to mark `BufferKey::Path`'s field private while keeping
/// `BufferKey` public), so normalization is instead enforced by wrapping the
/// `PathBuf` in this newtype with a PRIVATE field: the only way to build one
/// is [`NormPath::of`] (private to this module), routed from
/// [`BufferKey::path`]. A sibling module (`app::files`, `main::run`, or any
/// future caller) structurally CANNOT construct `BufferKey::Path(..)` from a
/// raw, un-normalized path — the bypass this module's doc warns about doesn't
/// just rely on convention, it fails to compile.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NormPath(PathBuf);

impl NormPath {
    fn of(p: &Path) -> Self {
        NormPath(normalize_path(p))
    }
}

impl BufferKey {
    /// Build a PATH identity, routed through [`normalize_path`] so the SAME
    /// file is recognized as the same registry entry no matter which of its
    /// (possibly several) textual spellings produced the path — e.g. a CLI
    /// file argument typed with no directory component (`cd project && awl
    /// a.txt`, staying relative) versus that same file's later ROOT-JOINED
    /// spelling (`index::resolve`, always absolute — every Goto-picker
    /// candidate). Without this, the two spellings hash to different keys: the
    /// buffer opened under the first spelling gets parked under it, a later
    /// Goto to the second spelling never finds it, silently re-reads the file
    /// from disk (discarding the live edit), and leaves the first spelling's
    /// entry orphaned in the registry forever (never evictable once dirty).
    /// THE ONE constructor every `BufferKey::Path` site must go through
    /// (`BufferKey::of` below, plus the Goto-accept sites in
    /// `app::files::load_path` / `main::run::replay_keys`) — same behavior ⇒
    /// same code, per CLAUDE.md.
    pub fn path(p: &Path) -> Self {
        BufferKey::Path(NormPath::of(p))
    }

    /// The registry identity for `buffer`, or `None` for a buffer that has no
    /// stable identity worth keeping: an unnamed, still-empty QUICK NOTE. By
    /// the time a note carries real content, the autosave engine
    /// (`App::flush_note` -> `autosave_note`) has already derived it a path
    /// from its first line, so in practice this arm is only ever hit on a
    /// truly empty note — which is fine to drop (nothing would be lost:
    /// an empty note is never written to disk either).
    pub fn of(buffer: &Buffer) -> Option<Self> {
        match buffer.path() {
            Some(p) => Some(BufferKey::path(p)),
            None if !buffer.is_note() => Some(BufferKey::Scratch),
            None => None,
        }
    }
}

/// Normalize `p` to a stable, comparable form: make it ABSOLUTE (joined
/// against the process's current directory when relative), then resolve it
/// through [`std::fs::canonicalize`] — which ALSO collapses `.`/`..` and
/// follows symlinks, so a symlinked directory in the path resolves to the
/// SAME identity as the real one (two spellings of one file must be one
/// registry entry, full stop; tracking the symlink's own name would defeat
/// the entire point of normalizing). `canonicalize` requires every component
/// to exist, which a freshly-typed CLI argument for a NOT-YET-CREATED file
/// never does — so on failure, [`canonicalize_lenient`] walks UP to the
/// deepest EXISTING ancestor, canonicalizes that instead, and re-joins the
/// remaining (lexically pre-collapsed) tail — so the new file's key
/// normalizes identically once it exists, matching whatever spelling of its
/// existing parent directory was used to reach it. See [`BufferKey::path`]
/// for why this matters.
fn normalize_path(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    let clean = lexically_collapse(&abs);
    std::fs::canonicalize(&clean).unwrap_or_else(|_| canonicalize_lenient(&clean))
}

/// Lexically collapse `.` / `..` components without touching disk (the pure
/// building block [`normalize_path`] runs BEFORE attempting canonicalize, so
/// the un-canonicalizable-tail fallback in [`canonicalize_lenient`] never has
/// to re-strip a stray `..` out of its already-clean tail components).
fn lexically_collapse(abs: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in abs.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Fallback for a path `std::fs::canonicalize` rejected outright (typically:
/// the path, or some component of it, doesn't exist yet). Walks UP from
/// `clean` (already lexically collapsed) until an ancestor DOES canonicalize,
/// then re-joins the remaining tail components onto that resolved ancestor —
/// so a not-yet-existing file's key still tracks its real (symlink-resolved)
/// parent directory. A real filesystem's root always exists, so this
/// terminates; the pathological case where NOT EVEN THE ROOT canonicalizes
/// (e.g. the cwd itself was unreadable) degrades to the lexically-collapsed
/// path as-is — the same best-effort fallback `normalize_path` always used,
/// never a panic.
fn canonicalize_lenient(clean: &Path) -> PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut ancestor = clean;
    loop {
        let Some(parent) = ancestor.parent() else {
            return clean.to_path_buf();
        };
        if let Some(name) = ancestor.file_name() {
            tail.push(name.to_os_string());
        }
        ancestor = parent;
        if let Ok(canon_ancestor) = std::fs::canonicalize(ancestor) {
            let mut out = canon_ancestor;
            for comp in tail.iter().rev() {
                out.push(comp);
            }
            return out;
        }
    }
}

/// Max simultaneously-open buffers (the active one + everything backgrounded
/// here), a modest cap so a long session doesn't grow memory unboundedly.
/// PRODUCT CALL (flagged, taking the calm default): past the cap, the
/// LEAST-RECENTLY-USED CLEAN (unedited-since-open) backgrounded buffer is
/// evicted; a DIRTY buffer is NEVER evicted — the cap is silently exceeded
/// rather than discarding unsaved work. A future UI could surface "N buffers
/// open, M unsaved" (out of scope here — no tab strip in v1).
pub const MAX_OPEN_BUFFERS: usize = 16;

/// One BACKGROUNDED buffer's saved state: the [`Buffer`] itself (cursor,
/// selection anchor, undo/redo history, and dirty flag already live ON
/// `Buffer` — nothing to duplicate there) plus an opaque `extra` payload the
/// caller attaches for its OWN per-buffer bookkeeping.
pub struct Entry<T> {
    pub buffer: Buffer,
    pub extra: T,
}

/// MRU-ordered registry of backgrounded buffers (index 0 = most recently
/// backgrounded = the eviction LAST-resort), keyed by [`BufferKey`]. Generic
/// over the caller's per-buffer payload `T`.
pub struct BufferRegistry<T> {
    entries: Vec<(BufferKey, Entry<T>)>,
    /// Latches once the over-cap-all-dirty notice (see `park`) has fired, so a
    /// user who keeps opening dirty files past the cap gets ONE calm stderr
    /// line instead of a re-print on every subsequent open (code review nit:
    /// the un-latched version was harmless but noisy). Clears the instant a
    /// clean eviction succeeds again — i.e. it tracks "are we CURRENTLY stuck
    /// over cap with nothing evictable", not "has this ever happened".
    over_cap_warned: bool,
}

impl<T> Default for BufferRegistry<T> {
    fn default() -> Self {
        Self { entries: Vec::new(), over_cap_warned: false }
    }
}

impl<T> BufferRegistry<T> {
    /// How many buffers are parked here (NOT counting the caller's active one).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when `key` names a currently-backgrounded buffer. Exercised by the
    /// registry's own tests; kept `pub` as the natural companion of `park`/
    /// `take` for a future caller (e.g. a tab-strip chrome, out of v1 scope).
    #[allow(dead_code)]
    pub fn contains(&self, key: &BufferKey) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// Park `entry` under `key` at the MRU front, evicting the LRU CLEAN
    /// backgrounded entry (if any) while doing so would push the total open
    /// count (this registry + 1 active) past [`MAX_OPEN_BUFFERS`]. Replaces
    /// any existing entry under the same key (should not normally happen —
    /// the caller only parks the buffer it is LEAVING).
    pub fn park(&mut self, key: BufferKey, entry: Entry<T>) {
        self.entries.retain(|(k, _)| k != &key);
        self.entries.insert(0, (key, entry));
        while self.entries.len() + 1 > MAX_OPEN_BUFFERS {
            // Evict the LEAST-recently-used (last in MRU order) CLEAN entry.
            match self.entries.iter().rposition(|(_, e)| !e.buffer.is_dirty()) {
                Some(pos) => {
                    self.entries.remove(pos);
                    self.over_cap_warned = false;
                }
                None => {
                    // Every backgrounded buffer is dirty: never discard unsaved
                    // work — exceed the cap instead (see the module doc). Fire
                    // the notice once per "stuck over cap" spell, not once per
                    // subsequent open (see `over_cap_warned`'s doc).
                    if !self.over_cap_warned {
                        eprintln!(
                            "awl: buffer registry over cap ({} open, all dirty) — keeping all",
                            self.entries.len() + 1
                        );
                        self.over_cap_warned = true;
                    }
                    break;
                }
            }
        }
    }

    /// Remove and return the entry for `key` (a buffer being brought back to
    /// the foreground), or `None` if it isn't backgrounded (first time open).
    pub fn take(&mut self, key: &BufferKey) -> Option<Entry<T>> {
        let pos = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(pos).1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(path: &str) -> BufferKey {
        BufferKey::path(Path::new(path))
    }

    #[test]
    fn park_then_take_round_trips_the_same_buffer() {
        let mut reg: BufferRegistry<()> = BufferRegistry::default();
        let mut b = Buffer::scratch();
        b.set_text("hello");
        reg.park(keyed("/a.txt"), Entry { buffer: b, extra: () });
        assert_eq!(reg.len(), 1);
        assert!(reg.contains(&keyed("/a.txt")));
        let entry = reg.take(&keyed("/a.txt")).expect("parked entry");
        assert_eq!(entry.buffer.text(), "hello");
        assert_eq!(reg.len(), 0);
        assert!(!reg.contains(&keyed("/a.txt")));
    }

    #[test]
    fn take_of_unknown_key_is_none() {
        let mut reg: BufferRegistry<()> = BufferRegistry::default();
        assert!(reg.take(&keyed("/nope.txt")).is_none());
    }

    #[test]
    fn buffer_key_path_normalizes_a_relative_path_against_the_cwd() {
        // REGRESSION (code review): a relative path (e.g. an un-directoried CLI
        // file argument) must key IDENTICALLY to its cwd-joined absolute form —
        // the same file reached two different ways must be the same registry
        // entry.
        let cwd = std::env::current_dir().unwrap();
        let rel = BufferKey::path(Path::new("some_never_created_test_file.rs"));
        let abs = BufferKey::path(&cwd.join("some_never_created_test_file.rs"));
        assert_eq!(rel, abs, "relative and cwd-joined-absolute must key the same");
    }

    #[test]
    fn buffer_key_path_collapses_dot_and_dotdot_components() {
        let messy = PathBuf::from("/a/b/x/../c/./file.rs");
        let clean = PathBuf::from("/a/b/c/file.rs");
        assert_eq!(BufferKey::path(&messy), BufferKey::path(&clean));
    }

    #[test]
    #[cfg(unix)]
    fn buffer_key_path_resolves_a_symlinked_directory_to_the_real_path() {
        // REGRESSION (code review, scenario c): a path reached THROUGH a
        // symlinked directory must key IDENTICALLY to the path reached via
        // the real directory it points at — a symlink is just another
        // spelling of the same file, and `normalize_path` now resolves it
        // (real `std::fs::canonicalize`, not just lexical `.`/`..` collapse)
        // rather than tracking the symlink's own name.
        let base =
            std::env::temp_dir().join(format!("awl-buffers-symlink-{}", std::process::id()));
        let real_dir = base.join("real");
        let link_dir = base.join("link");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::write(real_dir.join("a.txt"), "alpha\n").unwrap();
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        let via_real = BufferKey::path(&real_dir.join("a.txt"));
        let via_link = BufferKey::path(&link_dir.join("a.txt"));
        assert_eq!(
            via_real, via_link,
            "the symlinked spelling must key identically to the real path"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    #[cfg(unix)]
    fn buffer_key_path_resolves_a_not_yet_existing_file_under_a_symlinked_directory() {
        // The ancestor-canonicalize fallback (`canonicalize_lenient`) must
        // ALSO resolve a symlinked ancestor directory for a file that doesn't
        // exist yet — a new file's key must match its real (not symlink)
        // parent identically whether reached via the link or the target, so
        // it normalizes the same before and after the file is created.
        let base =
            std::env::temp_dir().join(format!("awl-buffers-symlink-new-{}", std::process::id()));
        let real_dir = base.join("real");
        let link_dir = base.join("link");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        let via_real = BufferKey::path(&real_dir.join("new.txt"));
        let via_link = BufferKey::path(&link_dir.join("new.txt"));
        assert_eq!(
            via_real, via_link,
            "a not-yet-existing file under a symlinked ancestor still keys identically"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn buffer_key_of_scratch_and_path_and_unnamed_note() {
        let scratch = Buffer::scratch();
        assert_eq!(BufferKey::of(&scratch), Some(BufferKey::Scratch));

        let file = Buffer::from_file(std::path::Path::new("/does/not/exist/x.rs"));
        assert_eq!(
            BufferKey::of(&file),
            Some(BufferKey::path(Path::new("/does/not/exist/x.rs")))
        );

        let mut note = Buffer::scratch();
        note.set_note_dir(PathBuf::from("/notes"));
        assert_eq!(BufferKey::of(&note), None, "an unnamed empty note has no stable identity");
    }

    #[test]
    fn park_evicts_lru_clean_entry_over_cap() {
        let mut reg: BufferRegistry<()> = BufferRegistry::default();
        // Fill to exactly the cap (MAX_OPEN_BUFFERS - 1 backgrounded + 1 active).
        for i in 0..(MAX_OPEN_BUFFERS - 1) {
            reg.park(keyed(&format!("/f{i}.txt")), Entry { buffer: Buffer::scratch(), extra: () });
        }
        assert_eq!(reg.len(), MAX_OPEN_BUFFERS - 1);
        // One more push would exceed the cap: the LRU (last-in, i.e. `/f0.txt`,
        // parked first and never re-touched) is evicted.
        reg.park(keyed("/new.txt"), Entry { buffer: Buffer::scratch(), extra: () });
        assert_eq!(reg.len(), MAX_OPEN_BUFFERS - 1, "cap holds steady");
        assert!(!reg.contains(&keyed("/f0.txt")), "the LRU clean entry was evicted");
        assert!(reg.contains(&keyed("/new.txt")));
    }

    #[test]
    fn park_never_evicts_a_dirty_buffer() {
        let mut reg: BufferRegistry<()> = BufferRegistry::default();
        // Fill to the cap with DIRTY buffers (an edit marks dirty).
        for i in 0..(MAX_OPEN_BUFFERS - 1) {
            let mut b = Buffer::scratch();
            b.set_text("x");
            reg.park(keyed(&format!("/f{i}.txt")), Entry { buffer: b, extra: () });
        }
        assert_eq!(reg.len(), MAX_OPEN_BUFFERS - 1);
        // The newly-parked buffer is ALSO dirty, so there is truly no clean
        // victim anywhere in the registry.
        let mut newest = Buffer::scratch();
        newest.set_text("y");
        reg.park(keyed("/new.txt"), Entry { buffer: newest, extra: () });
        // Nothing dirty could be evicted, so the registry is left OVER cap
        // rather than discarding unsaved work.
        assert_eq!(reg.len(), MAX_OPEN_BUFFERS, "over cap: no dirty buffer was evicted");
        for i in 0..(MAX_OPEN_BUFFERS - 1) {
            assert!(reg.contains(&keyed(&format!("/f{i}.txt"))), "dirty entry {i} survives");
        }
        assert!(reg.contains(&keyed("/new.txt")), "the new dirty entry survives too");
    }

    #[test]
    fn park_evicts_the_newest_clean_entry_when_it_is_the_only_clean_one() {
        // A subtler shape of the same law: eviction picks ANY clean victim over
        // NO eviction, even if the only clean buffer happens to be the one just
        // parked (the incoming buffer is not specially protected — only DIRTY
        // buffers are).
        let mut reg: BufferRegistry<()> = BufferRegistry::default();
        for i in 0..(MAX_OPEN_BUFFERS - 1) {
            let mut b = Buffer::scratch();
            b.set_text("x");
            reg.park(keyed(&format!("/f{i}.txt")), Entry { buffer: b, extra: () });
        }
        reg.park(keyed("/clean.txt"), Entry { buffer: Buffer::scratch(), extra: () });
        assert_eq!(reg.len(), MAX_OPEN_BUFFERS - 1, "cap holds: the one clean entry was evicted");
        assert!(!reg.contains(&keyed("/clean.txt")));
        for i in 0..(MAX_OPEN_BUFFERS - 1) {
            assert!(reg.contains(&keyed(&format!("/f{i}.txt"))));
        }
    }
}
