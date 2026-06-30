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

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

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

/// A file's timestamps — the cross-backend stand-in for `std::fs::Metadata`, pared
/// to the two times awl reads (the go-to "last edited" recency and the HUD's
/// "file created" date). Each is `Option` because not every platform / backend
/// records both (`created` is famously absent on some filesystems).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    /// Creation time, if the backend records it.
    pub created: Option<SystemTime>,
    /// Last-modification time, if the backend records it.
    pub modified: Option<SystemTime>,
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

    /// The created/modified timestamps of `path` (go-to recency, HUD date).
    fn metadata(&self, path: &Path) -> io::Result<Metadata>;
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
            created: md.created().ok(),
            modified: md.modified().ok(),
        })
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
    /// path → (bytes, created, modified).
    files: std::collections::BTreeMap<PathBuf, MemFile>,
    /// known directories (every created/implied dir).
    dirs: std::collections::BTreeSet<PathBuf>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct MemFile {
    bytes: Vec<u8>,
    created: SystemTime,
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
        let now = SystemTime::now();
        let mut state = self.inner.write().unwrap();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                InMemoryFs::insert_dirs(&mut state, parent);
            }
        }
        let created = state.files.get(path).map(|f| f.created).unwrap_or(now);
        state.files.insert(
            path.to_path_buf(),
            MemFile {
                bytes: data.to_vec(),
                created,
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
                created: Some(f.created),
                modified: Some(f.modified),
            })
        } else if state.dirs.contains(path) {
            Ok(Metadata { created: None, modified: None })
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "no such file"))
        }
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

/// Serializes EVERY test that swaps the global FS — the backend is process-wide, so
/// two tests installing different fakes must not race. Mirrors `fps::TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run `body` with `fs` installed as the active backend, restoring the previous
/// backend (normally [`NativeFs`]) afterwards — so an fs-touching test runs against
/// the fake without leaking it into sibling tests. Holds [`TEST_LOCK`] for the
/// duration. Test-only.
#[cfg(test)]
pub(crate) fn with_fs<T>(fs: Arc<dyn FileSystem>, body: impl FnOnce() -> T) -> T {
    let _guard = FsGuard::install(fs);
    body()
}

/// An RAII alternative to [`with_fs`] for a MULTI-STATEMENT test that can't easily
/// wrap its whole body in a closure (e.g. a setup helper that returns a fake +
/// keeps it installed for the rest of the test). Holds [`TEST_LOCK`] and restores
/// the previous backend when dropped. Test-only.
#[cfg(test)]
pub(crate) struct FsGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    prev: Arc<dyn FileSystem>,
}

#[cfg(test)]
impl FsGuard {
    /// Install `fs` as the active backend, returning a guard that restores the
    /// prior backend on drop. The shared [`TEST_LOCK`] is held for the guard's life.
    pub(crate) fn install(fs: Arc<dyn FileSystem>) -> Self {
        let lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_is_the_default_backend() {
        // Without any swap the global is the native disk backing.
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    fn in_memory_metadata_has_times() {
        let fs = InMemoryFs::new().with_file("/a.md", "x");
        let md = fs.metadata(Path::new("/a.md")).unwrap();
        assert!(md.created.is_some() && md.modified.is_some());
        // A missing file errors.
        assert!(fs.metadata(Path::new("/nope")).is_err());
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
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(active().read_to_string(Path::new("/cfg.toml")).is_err());
    }
}
