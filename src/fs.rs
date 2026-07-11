//! src/fs.rs — the FILESYSTEM SEAM: awl's file I/O behind one swappable trait.
//!
//! awl is a native editor today (std::fs, the real disk), but the SAME editor is
//! meant to run later in a SANDBOXED browser, where there is no `std::fs` — storage
//! is OPFS / IndexedDB reached through a worker's synchronous access handles. That
//! future backend is NOT built here; this module only carves the SEAM it plugs into,
//! so the day it arrives no call site has to change.
//!
//! The shape:
//!   * [`FileSystem`] — the trait every file op routes through. SYNC by design: awl
//!     stays sync, and an OPFS worker's sync-access-handles let the browser backend
//!     honour these signatures without going async. Minimal — only the ops awl
//!     actually performs (read/write text + bytes, dir create/list, rename, remove,
//!     exists, metadata times).
//!   * [`NativeFs`] — the std::fs backing. BEHAVIOUR-PRESERVING: each method does
//!     EXACTLY what the inlined `std::fs::…` call did (same paths, same `io::Result`
//!     errors, same bytes), so a native capture is byte-identical to before.
//!   * [`InMemoryFs`] — a HashMap-backed fake for tests: no real disk, deterministic,
//!     so fs-touching unit tests prove the seam works without littering temp dirs.
//!
//! INJECTION follows awl's existing process-global idiom (mirroring `page`/`fps`/
//! `caret` — one app-wide setting reached without threading a handle through every
//! function): a single process-global FS, [`active`], returns the live backend
//! (default [`NativeFs`]). Production code calls `fs::active().read_to_string(p)`.
//! Tests swap in an [`InMemoryFs`] via [`set_active`] / [`with_fs`] under a shared
//! lock, so the seam is testable without plumbing `&dyn FileSystem` through the
//! whole call graph.

use crate::clock::SystemTime;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// One entry of a directory listing — the cross-backend stand-in for
/// `std::fs::DirEntry`. The walk / browse code needs only the leaf NAME, the full
/// PATH, and whether the entry is a dir or a file, so that is all this carries
/// (a `read_dir` consumer never re-stats it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// The entry's full path (`<dir>/<name>`), as the native `entry.path()` gave.
    pub path: PathBuf,
    /// The leaf file name (`entry.file_name()`), lossily-stringable by the caller.
    pub name: String,
    /// True for a directory entry.
    pub is_dir: bool,
    /// True for a regular file entry.
    pub is_file: bool,
}

/// A file's stat — the cross-backend stand-in for `std::fs::Metadata`, pared to
/// what awl reads: the "last edited" recency (go-to) and the byte length (the
/// autosave clobber guard's same-tick tie-breaker — an external edit landing
/// within our last stat's mtime tick still moves the size). Each field is an
/// `Option` because not every platform / backend records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    /// Last-modification time, if the backend records it.
    pub modified: Option<SystemTime>,
    /// Byte length of the file, if the backend records it.
    pub len: Option<u64>,
}

/// The FILESYSTEM SEAM: every file op awl performs, behind one sync trait so the
/// `std::fs` dependency is swappable for a future sandboxed (OPFS/IndexedDB)
/// backend. SYNC on purpose (awl is sync; an OPFS worker uses sync-access-handles).
/// Kept MINIMAL — exactly the surface the inventoried call sites need, no more.
pub trait FileSystem: Send + Sync {
    /// Read the whole file at `path` as a UTF-8 string (config load, buffer open).
    fn read_to_string(&self, path: &Path) -> io::Result<String>;

    /// Read the whole file at `path` as raw bytes (the `AWL_FONT` face load).
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Write `data` to `path`, creating or truncating it (config + buffer save).
    fn write(&self, path: &Path, data: &[u8]) -> io::Result<()>;

    /// Create `path` and every missing parent (note/config dir seeding).
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Rename / move `from` → `to` (note rename + C-x m move; a true move).
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// True if `path` exists (collision scans, git-root + notes-root probes).
    fn exists(&self, path: &Path) -> bool;

    /// True if `path` is a directory (the launch-file root probe in `main.rs`).
    fn is_dir(&self, path: &Path) -> bool;

    /// List ONE directory level at `path` (the index walk + browse navigator).
    /// Order is unspecified (callers sort), matching `std::fs::read_dir`.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>>;

    /// The last-modified timestamp of `path` (go-to recency).
    fn metadata(&self, path: &Path) -> io::Result<Metadata>;

    /// Remove a single file at `path`. Used by the corrupt-backup pruner
    /// ([`crate::durable::preserve_corrupt`]) to cap how many `.corrupt-*`
    /// siblings a store keeps. Best-effort at every call site (a failed prune
    /// just means one extra sibling lingers, never fatal).
    fn remove_file(&self, path: &Path) -> io::Result<()>;
}

// --- Native backend -------------------------------------------------------

/// The std::fs backing — the native disk. Every method is a THIN, behaviour-
/// preserving wrapper over the exact `std::fs::…` call the code used to inline, so
/// the native editor reads and writes precisely as before (same paths, same
/// `io::Result` errors, byte-identical results).
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeFs;

impl FileSystem for NativeFs {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        std::fs::write(path, data)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let mut out = Vec::new();
        // Mirror the inlined `read_dir(...).flatten()` + per-entry `file_type()`
        // pattern exactly: an entry whose type can't be read is SKIPPED (as the
        // call sites' `let Ok(ft) = entry.file_type() else { continue }` did), so
        // the visible result is identical.
        for entry in std::fs::read_dir(path)? {
            let Ok(entry) = entry else { continue };
            let Ok(ft) = entry.file_type() else { continue };
            out.push(DirEntry {
                path: entry.path(),
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: ft.is_dir(),
                is_file: ft.is_file(),
            });
        }
        Ok(out)
    }

    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        let md = std::fs::metadata(path)?;
        Ok(Metadata {
            modified: md.modified().ok(),
            len: Some(md.len()),
        })
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

// --- In-memory backend (tests) --------------------------------------------
//
// Test-only: the fake + its helpers exist solely so fs-touching unit tests run
// against an in-memory backend (no real disk). Gated behind `#[cfg(test)]` so a
// release build doesn't carry — or warn about — never-constructed test scaffolding.

/// A HashMap-backed fake filesystem for tests: files + their bytes live in memory,
/// directories are tracked as a set, so fs-touching logic runs deterministically
/// with NO real disk (no temp-dir litter, no cross-test interference). Paths are
/// stored verbatim (the keys callers pass), and the ops model the native ones
/// closely enough that the inventoried code behaves identically. Cloneable +
/// shareable (`Arc<RwLock<…>>`) so a test can seed it, install it, then assert.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct InMemoryFs {
    inner: Arc<RwLock<MemState>>,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct MemState {
    /// path → (bytes, modified).
    files: std::collections::BTreeMap<PathBuf, MemFile>,
    /// known directories (every created/implied dir).
    dirs: std::collections::BTreeSet<PathBuf>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct MemFile {
    bytes: Vec<u8>,
    modified: SystemTime,
}

#[cfg(test)]
impl InMemoryFs {
    /// A fresh, empty in-memory filesystem (root `/` implicitly present).
    pub fn new() -> Self {
        let fs = InMemoryFs::default();
        fs.inner.write().unwrap().dirs.insert(PathBuf::from("/"));
        fs
    }

    /// Seed a text file (creating its parent dirs), for arranging a test. Returns
    /// `self` so seeds can chain.
    pub fn with_file(self, path: impl AsRef<Path>, contents: &str) -> Self {
        self.write(path.as_ref(), contents.as_bytes()).unwrap();
        self
    }

    /// Seed a directory (and its parents), for arranging a test.
    pub fn with_dir(self, path: impl AsRef<Path>) -> Self {
        self.create_dir_all(path.as_ref()).unwrap();
        self
    }

    /// Record `path` and every ancestor as a known directory.
    fn insert_dirs(state: &mut MemState, path: &Path) {
        let mut cur = Some(path);
        while let Some(p) = cur {
            state.dirs.insert(p.to_path_buf());
            cur = p.parent();
        }
    }
}

#[cfg(test)]
impl FileSystem for InMemoryFs {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.inner
            .read()
            .unwrap()
            .files
            .get(path)
            .map(|f| f.bytes.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }

    fn write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        let now = crate::clock::system_now();
        let mut state = self.inner.write().unwrap();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                InMemoryFs::insert_dirs(&mut state, parent);
            }
        }
        state.files.insert(
            path.to_path_buf(),
            MemFile {
                bytes: data.to_vec(),
                modified: now,
            },
        );
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let mut state = self.inner.write().unwrap();
        InMemoryFs::insert_dirs(&mut state, path);
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut state = self.inner.write().unwrap();
        let file = state
            .files
            .remove(from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))?;
        if let Some(parent) = to.parent() {
            if !parent.as_os_str().is_empty() {
                InMemoryFs::insert_dirs(&mut state, parent);
            }
        }
        state.files.insert(to.to_path_buf(), file);
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        let state = self.inner.read().unwrap();
        state.files.contains_key(path) || state.dirs.contains(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.inner.read().unwrap().dirs.contains(path)
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let state = self.inner.read().unwrap();
        if !state.dirs.contains(path) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such directory"));
        }
        let mut out = Vec::new();
        let is_child = |p: &Path| p.parent() == Some(path);
        for f in state.files.keys().filter(|p| is_child(p)) {
            out.push(DirEntry {
                path: f.clone(),
                name: leaf_name(f),
                is_dir: false,
                is_file: true,
            });
        }
        for d in state.dirs.iter().filter(|p| p.as_path() != path && is_child(p)) {
            out.push(DirEntry {
                path: d.clone(),
                name: leaf_name(d),
                is_dir: true,
                is_file: false,
            });
        }
        Ok(out)
    }

    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        let state = self.inner.read().unwrap();
        if let Some(f) = state.files.get(path) {
            Ok(Metadata {
                modified: Some(f.modified),
                len: Some(f.bytes.len() as u64),
            })
        } else if state.dirs.contains(path) {
            Ok(Metadata { modified: None, len: None })
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "no such file"))
        }
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let mut state = self.inner.write().unwrap();
        state
            .files
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }
}

/// The leaf file name of `path` as an owned lossy string (matches what the native
/// `entry.file_name().to_string_lossy()` yields for a `read_dir` entry).
#[cfg(test)]
fn leaf_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

// --- First-load seed samples (shared, platform-agnostic) -------------------
//
// The web build's FIRST-LOAD seed set: a small, CURATED welcome for a
// first-time visitor, not a dumping ground for every dev fixture that has
// ever lived under `samples/`. Kept here (unconditional — NOT `cfg(wasm32)`)
// so the list + its write-if-absent LAW are unit-testable on native via
// [`InMemoryFs`], never only exercised inside a browser sandbox.
//
// Curation note: `samples/longwrap.md` (soft-wrap stress fixture) and
// `samples/spellcheck.md` (squiggle-demo fixture) are deliberately EXCLUDED
// here — real files, still used by the capture harness and docs, just not
// what should greet a first-time visitor. `samples/tour.md` is the new
// markdown showcase; `samples/prose.md` and `samples/japanese.md` (the
// bundled-JP-face beauty moment) carry over unchanged in shape.
//
// `cfg(any(test, wasm32))`: the only consumers are `mod web` (wasm-only) and
// this file's own native unit tests — a plain native `cargo build` has no use
// for any of the three items below, so they'd otherwise warn `dead_code`.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) const SEED_SAMPLES: &[(&str, &str)] = &[
    ("/welcome.md", include_str!("../samples/welcome.md")),
    ("/tour.md", include_str!("../samples/tour.md")),
    ("/prose.md", include_str!("../samples/prose.md")),
    ("/japanese.md", include_str!("../samples/japanese.md")),
];

/// The seed-generation sentinel key. Bumped `awlfs:seeded` -> `awlfs:seeded:v2`
/// alongside the curated seed-list change above, so an already-seeded browser
/// (which only ever wrote the OLD key) re-runs seeding exactly once more under
/// the new key — picking up `/tour.md` and dropping `/longwrap.md`+
/// `/spellcheck.md` from the seed set — while [`seed_write_if_absent`]'s own
/// per-file law means it can NEVER clobber bytes the visitor already has. The
/// old `awlfs:seeded` key is simply left inert in `localStorage` for a
/// returning visitor — not read, not cleaned up; a stray unused key costs
/// nothing and a migration pass isn't worth the complexity for one flag.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) const SEED_SENTINEL_KEY: &str = "awlfs:seeded:v2";

/// Seed [`SEED_SAMPLES`] into `fs`, WRITE-IF-ABSENT per file: a path that
/// already exists is left completely untouched (never overwritten), so a
/// returning visitor who has edited `/welcome.md` — or still has an old
/// `/longwrap.md` / `/spellcheck.md` from a prior seed generation — keeps
/// every byte; they only ever GAIN newly-seeded paths. Generic over
/// `&dyn FileSystem` (not `WebFs`-specific) so this is unit-testable on
/// native with an [`InMemoryFs`] — the sentinel-gating (localStorage-
/// specific, "have I seeded THIS generation yet") stays the caller's job.
///
/// CONVENTION-TRUTHFUL SURFACES ROUND: `SEED_SAMPLES`' text carries
/// `{{key:slug}}` chord tokens (see `keytoken.rs`) — each file's content is
/// rendered through [`crate::keytoken::render_key_tokens`] for `convention`/
/// `platform` BEFORE it's written, so a Linux-web visitor's seeded welcome
/// note says `Ctrl+P` (or the web-alternate chord, where the native one is
/// browser-reserved) and a Mac-web visitor's says `⌘P` — never a hand-typed
/// literal that could drift from what actually fires. `convention`/`platform`
/// are EXPLICIT parameters (the same testability pattern
/// `resolved_native_label_truthful` uses); the one real call site
/// ([`web::WebFs::seed_samples`]) passes both `::current()`.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn seed_write_if_absent(fs: &dyn FileSystem, convention: crate::convention::Convention, platform: crate::commands::Platform) {
    let _ = fs.create_dir_all(Path::new("/"));
    for (p, content) in SEED_SAMPLES {
        let path = Path::new(p);
        if fs.exists(path) {
            continue; // never clobber a visitor's own edits (or an old fixture)
        }
        let rendered = crate::keytoken::render_key_tokens(content, convention, platform);
        let _ = fs.write(path, rendered.as_bytes());
    }
}

// --- Web backend (browser localStorage) -----------------------------------
//
// The SANDBOXED browser backing the seam doc promised. There is no `std::fs` on
// `wasm32-unknown-unknown`, so awl's file ops route through the browser's
// `localStorage` — a synchronous, origin-scoped, reload-persistent key→string
// store, which fits this SYNC trait exactly (no OPFS worker / async handles
// needed for a single-user notes editor). Gated to wasm so the native build is
// byte-identical (the whole module vanishes on a native compile).
//
// MAPPING. localStorage is a FLAT string map, so a tiny virtual filesystem is
// laid over it with TYPE-PREFIXED keys (all under the `awlfs:` namespace so a
// host page's own keys never collide):
//   * `awlfs:F:<path>` → a file's UTF-8 contents.
//   * `awlfs:D:<path>` → a directory MARKER (value unused) so empty dirs exist.
//   * `awlfs:M:<path>` → a file's modified millis (best-effort time; the browser
//     has no inode, so it is recorded on write rather than read from a real stat).
//   * `awlfs:seeded:v2` → the SEED-generation sentinel (see `seed_samples`,
//     [`super::SEED_SENTINEL_KEY`]) — bumped from the v1 `awlfs:seeded` key
//     when the curated seed set changed; the old key is left inert, unread.
// `read_dir` enumerates the `F:`/`D:` keys and keeps the ones whose PARENT is the
// queried dir — the same parent-match `InMemoryFs` uses — so the index walk and
// the go-to / browse pickers see the seeded notes. Binary `read`/`write` round-
// trip through `String::from_utf8_lossy`: awl only ever writes UTF-8 rope text,
// and the only byte reader (the `AWL_FONT` face load) never runs on the web.
#[cfg(target_arch = "wasm32")]
mod web {
    use super::{DirEntry, FileSystem, Metadata};
    use crate::clock::SystemTime;
    use std::io;
    use std::path::Path;
    use std::time::Duration;

    const FILE_PREFIX: &str = "awlfs:F:";
    const DIR_PREFIX: &str = "awlfs:D:";
    const MTIME_PREFIX: &str = "awlfs:M:";

    /// The browser-`localStorage` filesystem. A ZERO-SIZE handle: the `Storage`
    /// object is fetched fresh per call (cheap — it's a live binding to the one
    /// origin store), so the struct stays `Send + Sync` for the `dyn FileSystem`
    /// global despite `Storage` itself being a non-`Send` JS handle.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct WebFs;

    /// The origin's `localStorage`, or `None` if the page has no window / the API
    /// is blocked (private-mode lockdowns). Callers degrade gracefully (a read
    /// becomes `NotFound`, a write a benign error) exactly like a headless native
    /// run with no disk.
    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    /// An io-flavoured error when the JS side throws (e.g. `QuotaExceeded` on a
    /// write, or the API being unavailable).
    fn js_err(what: &str) -> io::Error {
        io::Error::other(format!("localStorage {what} failed"))
    }

    /// Now, as whole milliseconds since the Unix epoch, via `crate::clock` (the JS
    /// clock on wasm — std's `SystemTime::now()` PANICS on `wasm32-unknown-unknown`,
    /// no platform clock).
    fn now_millis() -> u64 {
        crate::clock::system_now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// A stored-millis stamp back as a `SystemTime`, built by ADDING to the const
    /// `UNIX_EPOCH` (no clock read) so it never trips the wasm panic. The
    /// `Metadata` times cross module boundaries as `crate::clock::SystemTime`.
    fn millis_to_system_time(ms: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_millis(ms)
    }

    impl WebFs {
        fn key(prefix: &str, path: &Path) -> String {
            format!("{prefix}{}", path.to_string_lossy())
        }

        /// Record `path` and every ancestor as a directory marker (mirrors the
        /// `InMemoryFs` `insert_dirs` contract so `create_dir_all` is idempotent).
        fn insert_dirs(s: &web_sys::Storage, path: &Path) {
            let mut cur = Some(path);
            while let Some(p) = cur {
                let _ = s.set_item(&Self::key(DIR_PREFIX, p), "");
                cur = p.parent();
            }
        }

        /// SEED the sample docs on FIRST load (sentinel-gated on
        /// [`super::SEED_SENTINEL_KEY`], so a reload of an already-seeded
        /// generation is a no-op). Called once at startup by
        /// [`super::install_web_fs`]; the bundled samples are embedded via
        /// `include_str!` (see [`super::SEED_SAMPLES`]), so seeding needs no
        /// network. The actual per-file write-if-absent law lives in the
        /// shared, platform-agnostic [`super::seed_write_if_absent`] — this
        /// method only owns the localStorage-specific sentinel check.
        pub fn seed_samples(&self) {
            let Some(s) = storage() else { return };
            if s.get_item(super::SEED_SENTINEL_KEY).ok().flatten().is_some() {
                return; // already seeded this generation — preserve existing notes
            }
            // The UA-detected convention MUST already be set by the time this
            // runs — `main::wasm_start` calls `set_web_convention_from_ua`
            // BEFORE `fs::install_web_fs()` for exactly this reason (see that
            // ordering note there).
            super::seed_write_if_absent(self, crate::convention::Convention::current(), crate::commands::Platform::current());
            let _ = s.set_item(super::SEED_SENTINEL_KEY, "1");
        }
    }

    impl FileSystem for WebFs {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            storage()
                .and_then(|s| s.get_item(&Self::key(FILE_PREFIX, path)).ok().flatten())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
        }

        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.read_to_string(path).map(String::into_bytes)
        }

        fn write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
            let s = storage().ok_or_else(|| js_err("unavailable"))?;
            let text = String::from_utf8_lossy(data);
            s.set_item(&Self::key(FILE_PREFIX, path), &text)
                .map_err(|_| js_err("write"))?;
            // Stamp the modification time on every write.
            let now = now_millis().to_string();
            let _ = s.set_item(&Self::key(MTIME_PREFIX, path), &now);
            // The containing dir (and its ancestors) now exist.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    Self::insert_dirs(&s, parent);
                }
            }
            Ok(())
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            let s = storage().ok_or_else(|| js_err("unavailable"))?;
            Self::insert_dirs(&s, path);
            Ok(())
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            let s = storage().ok_or_else(|| js_err("unavailable"))?;
            let content = s
                .get_item(&Self::key(FILE_PREFIX, from))
                .ok()
                .flatten()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))?;
            let _ = s.remove_item(&Self::key(FILE_PREFIX, from));
            let _ = s.remove_item(&Self::key(MTIME_PREFIX, from));
            s.set_item(&Self::key(FILE_PREFIX, to), &content)
                .map_err(|_| js_err("rename"))?;
            let _ = s.set_item(&Self::key(MTIME_PREFIX, to), &now_millis().to_string());
            if let Some(parent) = to.parent() {
                if !parent.as_os_str().is_empty() {
                    Self::insert_dirs(&s, parent);
                }
            }
            Ok(())
        }

        fn exists(&self, path: &Path) -> bool {
            storage()
                .map(|s| {
                    s.get_item(&Self::key(FILE_PREFIX, path)).ok().flatten().is_some()
                        || s.get_item(&Self::key(DIR_PREFIX, path)).ok().flatten().is_some()
                })
                .unwrap_or(false)
        }

        fn is_dir(&self, path: &Path) -> bool {
            storage()
                .and_then(|s| s.get_item(&Self::key(DIR_PREFIX, path)).ok().flatten())
                .is_some()
        }

        fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
            let s = storage()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no storage"))?;
            let len = s.length().map_err(|_| js_err("length"))?;
            let mut out = Vec::new();
            for i in 0..len {
                let Ok(Some(k)) = s.key(i) else { continue };
                // Each stored path is a FILE or a DIR marker; recover its virtual
                // path and keep the entry iff its parent IS the queried directory.
                let (rest, is_dir) = if let Some(r) = k.strip_prefix(FILE_PREFIX) {
                    (r, false)
                } else if let Some(r) = k.strip_prefix(DIR_PREFIX) {
                    (r, true)
                } else {
                    continue;
                };
                let child = Path::new(rest);
                if child.parent() != Some(path) || child == path {
                    continue;
                }
                out.push(DirEntry {
                    path: child.to_path_buf(),
                    name: child
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    is_dir,
                    is_file: !is_dir,
                });
            }
            Ok(out)
        }

        fn metadata(&self, path: &Path) -> io::Result<Metadata> {
            let s = storage()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no storage"))?;
            let read_ms = |prefix: &str| -> Option<SystemTime> {
                s.get_item(&Self::key(prefix, path))
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(millis_to_system_time)
            };
            // A file the store knows (it has content) reports its recorded times +
            // byte length (the stored UTF-8 string's length); a bare directory has
            // none; an unknown path errors like a native stat.
            let content = s.get_item(&Self::key(FILE_PREFIX, path)).ok().flatten();
            let is_dir = s.get_item(&Self::key(DIR_PREFIX, path)).ok().flatten().is_some();
            if let Some(content) = content {
                Ok(Metadata {
                    modified: read_ms(MTIME_PREFIX),
                    len: Some(content.len() as u64),
                })
            } else if is_dir {
                Ok(Metadata { modified: None, len: None })
            } else {
                Err(io::Error::new(io::ErrorKind::NotFound, "no such file"))
            }
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            let s = storage().ok_or_else(|| js_err("unavailable"))?;
            let key = Self::key(FILE_PREFIX, path);
            // Mirror the native/InMemory contract: removing an unknown path is
            // a NotFound error, not a silent success.
            if s.get_item(&key).ok().flatten().is_none() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
            }
            let _ = s.remove_item(&key);
            let _ = s.remove_item(&Self::key(MTIME_PREFIX, path));
            Ok(())
        }
    }
}

/// Install the browser [`web::WebFs`] (localStorage) as the active backend and
/// SEED the bundled sample docs on first load. The wasm entrypoint
/// (`main.rs::wasm_start`) calls this once before `app::run`, so the editor opens
/// on a seeded, reload-persistent virtual filesystem instead of the default
/// `NativeFs` (which has no real disk to reach in the sandbox).
#[cfg(target_arch = "wasm32")]
pub fn install_web_fs() {
    let webfs = web::WebFs;
    webfs.seed_samples();
    set_active(Arc::new(webfs));
}

// --- Shared write / path helpers (both backends) ---------------------------

/// ATOMIC WRITE through the active backend: write `data` to a hidden temp
/// sibling (`.<name>.awl-tmp`, same directory so the rename never crosses a
/// filesystem), then `rename` it over `path`. On the native backend a same-dir
/// rename is POSIX-atomic, so a crash mid-save leaves either the OLD file or the
/// NEW one — never a truncated half-write. Uses ONLY the trait's `write` +
/// `rename`, so `InMemoryFs` and `WebFs` model it too (wasm keeps compiling).
/// Used by every buffer save (manual and autosave), the scratch stash, and —
/// after this round's audit — every other durable app-owned store.
///
/// **`AWL_FAULT_DELAY_MS` (DEV-ONLY, native-only, no CLI flag — mirrors
/// `AWL_CJK_FORCE`'s "total no-op unless set" contract):** when set to a
/// valid integer, sleeps that many milliseconds AFTER the tmp write and
/// BEFORE the rename — artificially widening the pre-rename window so the
/// kill-9 fault harness (`tests/fault_kill9.rs`) can reliably land a SIGKILL
/// INSIDE it and assert the target file still holds its OLD content (the
/// rename never happened, so nothing was torn). Unset in every normal run —
/// including every other test in this suite — so this is a genuine zero-cost
/// no-op the rest of the time; reading the env var on every call is cheap
/// enough that a `#[cfg(test)]` gate isn't worth the code-path divergence
/// between test and release builds this primitive most needs to stay honest.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let fs = active();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());
    let tmp_name = format!(".{name}.awl-tmp");
    let tmp = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join(tmp_name),
        _ => PathBuf::from(tmp_name),
    };
    fs.write(&tmp, data)?;
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(ms) = std::env::var("AWL_FAULT_DELAY_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }
    fs.rename(&tmp, path)
}

/// awl's PERSISTENT DATA root — where the local-history store and the scratch
/// stash live. Native honours the XDG data dir (`$XDG_DATA_HOME/awl`, else
/// `~/.local/share/awl`), with a relative last-resort so the function is total
/// when no HOME is set. On the web the path is virtual (WebFs maps it onto
/// `localStorage` keys), so a fixed root suffices. `history::history_root()`
/// nests under this (`<data_root>/history`).
pub fn data_root() -> PathBuf {
    #[cfg(target_arch = "wasm32")]
    {
        PathBuf::from("/awl")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(x).join("awl");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".local").join("share").join("awl");
        }
        PathBuf::from("awl-data")
    }
}

/// Where the PERSISTENT SCRATCH BUFFER stashes across quits: the no-path launch
/// buffer is written here (atomic, on the same autosave triggers + quit) and
/// restored on the next no-argument launch. Web-safe via the trait (WebFs).
pub fn scratch_stash_path() -> PathBuf {
    data_root().join("scratch.md")
}

/// THE WEB CONFIG PATH — where `config.toml` lives inside the virtual `WebFs`
/// root (a `localStorage` key, `awlfs:F:/awl/config.toml`), closing WEB.md's
/// former "no config file on the web" gap. Beside the scratch stash under
/// [`data_root`] (the SAME `/awl` virtual-root convention `scratch.md` already
/// uses for machine-owned state), deliberately NOT under the seeded content
/// root `/` (which holds the user's own documents). `main::wasm_start` is the
/// ONE caller (native's `config::config_path` resolves an OS path instead and
/// is never reached on wasm); every `Config` write door (`write_pref`/
/// `write_binding`/`write_default`) already routes through
/// `crate::fs::active()` + [`write_atomic`], so a `Config` loaded from THIS
/// path just works over `WebFs` with zero further plumbing.
#[cfg(target_arch = "wasm32")]
pub fn web_config_path() -> PathBuf {
    data_root().join("config.toml")
}

// --- The process-global active backend ------------------------------------

/// The app-wide filesystem. DEFAULT is [`NativeFs`] (the real disk); tests swap in
/// an [`InMemoryFs`]. Behind an `RwLock<Arc<…>>` (not a plain static) so a test can
/// install a fake without `unsafe` and without threading a handle through every
/// I/O function — the same one-app-wide-setting idiom as `page`/`fps`/`caret`.
fn global() -> &'static RwLock<Arc<dyn FileSystem>> {
    use std::sync::OnceLock;
    static FS: OnceLock<RwLock<Arc<dyn FileSystem>>> = OnceLock::new();
    FS.get_or_init(|| RwLock::new(Arc::new(NativeFs)))
}

/// The ACTIVE filesystem backend. Production code routes EVERY file op through this
/// (`fs::active().read_to_string(p)`), so swapping the global swaps the backend
/// everywhere. Returns an `Arc` clone (cheap) so the caller holds no lock across
/// the actual I/O.
pub fn active() -> Arc<dyn FileSystem> {
    global().read().unwrap().clone()
}

/// Install `fs` as the active backend (the future browser entrypoint would call
/// this once with its OPFS backend; tests call it to inject an [`InMemoryFs`]).
#[allow(dead_code)]
pub fn set_active(fs: Arc<dyn FileSystem>) {
    *global().write().unwrap() = fs;
}

/// Run `body` with `fs` installed as the active backend, restoring the previous
/// backend (normally [`NativeFs`]) afterwards — so an fs-touching test runs against
/// the fake without leaking it into sibling tests. Holds the shared
/// [`crate::testlock`] guard for the duration. Test-only.
#[cfg(test)]
pub(crate) fn with_fs<T>(fs: Arc<dyn FileSystem>, body: impl FnOnce() -> T) -> T {
    let _guard = FsGuard::install(fs);
    body()
}

/// An RAII alternative to [`with_fs`] for a MULTI-STATEMENT test that can't easily
/// wrap its whole body in a closure (e.g. a setup helper that returns a fake +
/// keeps it installed for the rest of the test). Holds the shared
/// [`crate::testlock`] guard and restores the previous backend when dropped.
/// Test-only.
#[cfg(test)]
pub(crate) struct FsGuard {
    _lock: crate::testlock::SerialGuard,
    prev: Arc<dyn FileSystem>,
}

#[cfg(test)]
impl FsGuard {
    /// Install `fs` as the active backend, returning a guard that restores the
    /// prior backend on drop. The shared [`crate::testlock`] guard is held for the guard's life.
    pub(crate) fn install(fs: Arc<dyn FileSystem>) -> Self {
        let lock = crate::testlock::serial();
        let prev = active();
        set_active(fs);
        FsGuard { _lock: lock, prev }
    }
}

#[cfg(test)]
impl Drop for FsGuard {
    fn drop(&mut self) {
        set_active(self.prev.clone());
    }
}

/// An RAII helper that chdirs the process into `dir` for the guard's life,
/// restoring the ORIGINAL cwd on drop — even if the test body panics or an
/// assertion fails, so one failing test never stably strands every sibling
/// test (including ones that just read `current_dir()`, like
/// `main::run::tests::resolve_root_absent_sticky_reproduces_todays_default`)
/// in the wrong directory. The process cwd is a global like the fs backend, so
/// this holds the shared [`crate::testlock`] guard (reentrant) for its whole
/// life — ONE owner for every process-global, no cross-lock order left to
/// invert. Test-only.
#[cfg(test)]
pub(crate) struct CwdGuard {
    _lock: crate::testlock::SerialGuard,
    prev: PathBuf,
}

#[cfg(test)]
impl CwdGuard {
    /// Chdir into `dir`, panicking if either the current cwd can't be read
    /// (so it could be restored later) or `dir` can't be entered — both are
    /// setup failures, not something a test should silently limp past.
    pub(crate) fn enter(dir: &Path) -> Self {
        let lock = crate::testlock::serial();
        let prev = std::env::current_dir().expect("current dir must be readable");
        std::env::set_current_dir(dir).expect("chdir into test dir");
        CwdGuard { _lock: lock, prev }
    }
}

#[cfg(test)]
impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.prev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_is_the_default_backend() {
        // Without any swap the global is the native disk backing.
        let _g = crate::testlock::serial();
        // A read of a path that surely doesn't exist returns NotFound, not a fake.
        let err = active()
            .read_to_string(Path::new("/awl/definitely/not/here.toml"))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn in_memory_round_trips_text() {
        let fs = InMemoryFs::new();
        fs.write(Path::new("/n/a.md"), b"hello").unwrap();
        assert_eq!(fs.read_to_string(Path::new("/n/a.md")).unwrap(), "hello");
        // The parent dir is implied by the write.
        assert!(fs.exists(Path::new("/n")));
        assert!(fs.exists(Path::new("/n/a.md")));
        assert!(!fs.exists(Path::new("/n/b.md")));
    }

    #[test]
    fn in_memory_read_dir_levels_and_types() {
        let fs = InMemoryFs::new()
            .with_file("/r/readme.md", "r")
            .with_dir("/r/src")
            .with_file("/r/src/main.rs", "m");
        let mut names: Vec<String> = fs
            .read_dir(Path::new("/r"))
            .unwrap()
            .into_iter()
            .map(|e| format!("{}:{}", e.name, if e.is_dir { "d" } else { "f" }))
            .collect();
        names.sort();
        assert_eq!(names, vec!["readme.md:f".to_string(), "src:d".to_string()]);
        // Descend: only main.rs.
        let sub = fs.read_dir(Path::new("/r/src")).unwrap();
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0].name, "main.rs");
        assert!(sub[0].is_file);
    }

    #[test]
    fn in_memory_rename_moves_bytes() {
        let fs = InMemoryFs::new().with_file("/a.md", "body");
        fs.rename(Path::new("/a.md"), Path::new("/sub/b.md")).unwrap();
        assert!(!fs.exists(Path::new("/a.md")));
        assert_eq!(fs.read_to_string(Path::new("/sub/b.md")).unwrap(), "body");
    }

    #[test]
    fn in_memory_remove_file_deletes_and_errors_on_a_missing_path() {
        let fs = InMemoryFs::new().with_file("/a.md", "body").with_file("/b.md", "other");
        fs.remove_file(Path::new("/a.md")).unwrap();
        assert!(!fs.exists(Path::new("/a.md")), "removed file is gone");
        assert!(fs.exists(Path::new("/b.md")), "a sibling file is untouched");
        assert_eq!(
            fs.remove_file(Path::new("/a.md")).unwrap_err().kind(),
            io::ErrorKind::NotFound,
            "removing an already-gone (or never-existed) file errors NotFound, never panics"
        );
    }

    #[test]
    fn in_memory_metadata_has_times() {
        let fs = InMemoryFs::new().with_file("/a.md", "x");
        let md = fs.metadata(Path::new("/a.md")).unwrap();
        assert!(md.modified.is_some());
        // A missing file errors.
        assert!(fs.metadata(Path::new("/nope")).is_err());
    }

    #[test]
    fn write_atomic_replaces_content_and_leaves_no_tmp() {
        // The atomic write lands the exact bytes AND leaves no `.awl-tmp` sibling
        // behind (the temp file is renamed over the target, not copied). Both a
        // fresh create and an overwrite go through the same tmp+rename dance.
        let fake = Arc::new(InMemoryFs::new().with_dir("/docs"));
        with_fs(fake.clone(), || {
            write_atomic(Path::new("/docs/a.md"), b"first").unwrap();
            assert_eq!(fake.read_to_string(Path::new("/docs/a.md")).unwrap(), "first");
            write_atomic(Path::new("/docs/a.md"), b"second").unwrap();
            assert_eq!(fake.read_to_string(Path::new("/docs/a.md")).unwrap(), "second");
            let names: Vec<String> = fake
                .read_dir(Path::new("/docs"))
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert_eq!(names, vec!["a.md".to_string()], "no tmp residue: {names:?}");
        });
    }

    #[test]
    fn data_root_and_scratch_path_shapes() {
        // Pure SUFFIX asserts (no env mutation, so this can't race the config
        // env tests): whatever XDG/HOME arm resolves, the data root's leaf is
        // `awl` (or the total-function fallback `awl-data`), and the scratch
        // stash is `scratch.md` directly under it.
        let root = data_root();
        let leaf = root.file_name().map(|n| n.to_string_lossy().into_owned());
        assert!(
            leaf.as_deref() == Some("awl") || leaf.as_deref() == Some("awl-data"),
            "data root leaf is awl[-data]: {root:?}"
        );
        let stash = scratch_stash_path();
        assert_eq!(
            stash.file_name().map(|n| n.to_string_lossy().into_owned()).as_deref(),
            Some("scratch.md")
        );
    }

    #[test]
    fn with_fs_installs_and_restores() {
        let fake = Arc::new(InMemoryFs::new().with_file("/cfg.toml", "zoom = 1.0"));
        with_fs(fake, || {
            assert_eq!(
                active().read_to_string(Path::new("/cfg.toml")).unwrap(),
                "zoom = 1.0"
            );
        });
        // Restored to native: the fake's file is gone.
        let _g = crate::testlock::serial();
        assert!(active().read_to_string(Path::new("/cfg.toml")).is_err());
    }

    // --- Web first-load seed curation ---------------------------------

    /// THE CURATED SEED LIST, pinned exactly — the four paths a first-time
    /// web visitor sees, in seed order, and nothing else. `/longwrap.md` and
    /// `/spellcheck.md` (dev fixtures — soft-wrap + squiggle stress tests)
    /// are deliberately NOT in the seed set anymore (the files themselves
    /// still live under `samples/` for the capture harness); a regression
    /// that re-adds either — or drops `/tour.md` — fails this test.
    #[test]
    fn seed_sample_list_is_exactly_the_curated_four() {
        let paths: Vec<&str> = SEED_SAMPLES.iter().map(|(p, _)| *p).collect();
        assert_eq!(paths, vec!["/welcome.md", "/tour.md", "/prose.md", "/japanese.md"]);
        assert!(
            !paths.contains(&"/longwrap.md") && !paths.contains(&"/spellcheck.md"),
            "dev fixtures must never re-enter the first-load seed set: {paths:?}"
        );
        // Every seeded doc actually carries content (a bare include_str! typo
        // would otherwise silently seed an empty file).
        for (p, content) in SEED_SAMPLES {
            assert!(!content.trim().is_empty(), "{p} seeds non-empty content");
        }
    }

    /// The sentinel bumped generations: `awlfs:seeded` (v1) -> `awlfs:seeded:v2`,
    /// so an already-seeded browser (which only ever wrote the v1 key) re-runs
    /// seeding exactly once more under the new key.
    #[test]
    fn seed_sentinel_is_bumped_to_v2() {
        assert_eq!(SEED_SENTINEL_KEY, "awlfs:seeded:v2");
        assert_ne!(SEED_SENTINEL_KEY, "awlfs:seeded", "must differ from the v1 key");
    }

    /// THE WRITE-IF-ABSENT LAW: seeding a fresh filesystem writes exactly the
    /// curated four paths, TOKEN-RENDERED for the pinned convention/platform
    /// (see `keytoken.rs`) — never the raw `{{key:..}}`-bearing source text.
    #[test]
    fn seed_write_if_absent_seeds_the_curated_set_on_a_fresh_fs() {
        let fs = InMemoryFs::new();
        seed_write_if_absent(&fs, crate::convention::Convention::Mac, crate::commands::Platform::Web);
        for (p, content) in SEED_SAMPLES {
            let rendered = crate::keytoken::render_key_tokens(content, crate::convention::Convention::Mac, crate::commands::Platform::Web);
            assert_eq!(fs.read_to_string(Path::new(p)).unwrap(), rendered);
            // The seeded text is fully rendered — no stray token/unknown-slug marker survives.
            assert!(!fs.read_to_string(Path::new(p)).unwrap().contains("{{key:"), "{p} still carries a raw token");
        }
    }

    /// THE WRITE-IF-ABSENT LAW, the returning-visitor half: a path that
    /// already exists — whether it's one of the curated seed paths the
    /// visitor has since edited, or an unrelated leftover from an OLDER seed
    /// generation (`/longwrap.md`, `/spellcheck.md`) — is left BYTE-FOR-BYTE
    /// untouched by a re-seed. This is the actual guarantee behind "a
    /// returning visitor with edits keeps every byte; they just gain the new
    /// files."
    #[test]
    fn seed_write_if_absent_never_clobbers_an_existing_path() {
        let fs = InMemoryFs::new()
            .with_file("/welcome.md", "my own edited welcome, thanks")
            .with_file("/longwrap.md", "an old dev-fixture leftover, untouched")
            .with_file("/spellcheck.md", "another old leftover, untouched");
        seed_write_if_absent(&fs, crate::convention::Convention::Mac, crate::commands::Platform::Web);

        // The edited existing seed path survives verbatim.
        assert_eq!(
            fs.read_to_string(Path::new("/welcome.md")).unwrap(),
            "my own edited welcome, thanks"
        );
        // The two dropped-from-seeding dev fixtures are left alone too, not
        // deleted and not overwritten — seeding never touches a path it
        // didn't itself write.
        assert_eq!(
            fs.read_to_string(Path::new("/longwrap.md")).unwrap(),
            "an old dev-fixture leftover, untouched"
        );
        assert_eq!(
            fs.read_to_string(Path::new("/spellcheck.md")).unwrap(),
            "another old leftover, untouched"
        );
        // Meanwhile every OTHER curated path — absent before — gets seeded,
        // token-rendered same as the fresh-fs case above.
        for (p, content) in SEED_SAMPLES {
            let content = &crate::keytoken::render_key_tokens(content, crate::convention::Convention::Mac, crate::commands::Platform::Web);
            if *p == "/welcome.md" {
                continue;
            }
            assert_eq!(fs.read_to_string(Path::new(p)).unwrap(), *content);
        }
    }
}
