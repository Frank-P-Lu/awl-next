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

use std::path::PathBuf;

use crate::buffer::Buffer;

/// A buffer's stable identity for registry lookups. A SAVED file is keyed by
/// its bound path; the ONE pathless "scratch" writing surface (the launch
/// buffer, or the persistent stash it restores from) is keyed by the
/// `Scratch` sentinel — there is only ever one such identity, mirroring the
/// one persistent scratch stash (`fs::scratch_stash_path`). A pathless QUICK
/// NOTE that hasn't been named yet has NO stable identity and is deliberately
/// never registered (see [`BufferKey::of`]).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BufferKey {
    Path(PathBuf),
    Scratch,
}

impl BufferKey {
    /// The registry identity for `buffer`, or `None` for a buffer that has no
    /// stable identity worth keeping: an unnamed, still-empty QUICK NOTE. By
    /// the time a note carries real content, the autosave engine
    /// (`App::flush_note` -> `autosave_note`) has already derived it a path
    /// from its first line, so in practice this arm is only ever hit on a
    /// truly empty note — which is fine to drop (nothing would be lost:
    /// an empty note is never written to disk either).
    pub fn of(buffer: &Buffer) -> Option<Self> {
        match buffer.path() {
            Some(p) => Some(BufferKey::Path(p.to_path_buf())),
            None if !buffer.is_note() => Some(BufferKey::Scratch),
            None => None,
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
}

impl<T> Default for BufferRegistry<T> {
    fn default() -> Self {
        Self { entries: Vec::new() }
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
                }
                None => {
                    // Every backgrounded buffer is dirty: never discard unsaved
                    // work — exceed the cap instead (see the module doc).
                    eprintln!(
                        "awl: buffer registry over cap ({} open, all dirty) — keeping all",
                        self.entries.len() + 1
                    );
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
        BufferKey::Path(PathBuf::from(path))
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
    fn buffer_key_of_scratch_and_path_and_unnamed_note() {
        let scratch = Buffer::scratch();
        assert_eq!(BufferKey::of(&scratch), Some(BufferKey::Scratch));

        let file = Buffer::from_file(std::path::Path::new("/does/not/exist/x.rs"));
        assert_eq!(BufferKey::of(&file), Some(BufferKey::Path(PathBuf::from("/does/not/exist/x.rs"))));

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
