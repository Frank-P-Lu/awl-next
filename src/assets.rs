//! ASSET CLEANER: the SUMMONED "Clean unused assets…" command's engine — the pure
//! SCAN that finds unreferenced image files under the active project, plus the
//! TRASH SEAM its picker's Enter routes through.
//!
//! **The model (user-decided 2026-07-09).** A project keeps its images in the
//! per-doc-dir `assets/` convention (the same folder `paste_image` writes into —
//! Typora/Obsidian style). Over time some of those images stop being referenced by
//! any document. This module answers ONE question, purely: *which image files
//! under an `assets/` directory does no document reference?* — the ORPHANS.
//!   1. Candidate assets = image-extension files ([`IMAGE_EXTS`]) living under ANY
//!      `assets/` directory in the project (an ancestor path component is `assets`).
//!   2. References = every `![alt](path)` a markdown document holds, parsed with the
//!      REAL parser ([`crate::markdown::image_refs`] → [`crate::markdown::parse_image_source`],
//!      never a regex) and resolved against THAT doc's own directory ([`resolve_ref`],
//!      the path-only mirror of the renderer's `resolve_image_path`).
//!   3. ORPHAN = a candidate no resolved reference lands on.
//! The scan is a PURE function of `(root, corpus)` over the [`crate::fs`] seam, so it
//! is fully testable against an [`crate::fs::InMemoryFs`] with no real disk.
//!
//! **Recoverability is the safety net.** An orphan is never deleted with `rm` — the
//! picker's Enter moves it to the macOS TRASH ([`TrashCan::trash`], the NSFileManager
//! door), so a false-orphan (e.g. a rare reference-style image the parser can't
//! resolve) is one Finder ⌘-Z away. The trash call goes behind the [`TrashCan`] seam
//! (a process-global backend, mirroring [`crate::fs::active`]) so a test injects a
//! FAKE that records the paths without touching a real Trash, and the real call stays
//! live-only.
//!
//! **Native / macOS scope (logged).** The scan compiles + runs on every native
//! target; the default TRASH backend only succeeds on macOS (NSFileManager) and
//! returns a calm `Err` elsewhere. The whole feature is a no-op under the headless
//! capture harness for the TRASH half (a `--keys` replay's `Effect::TrashAsset` is a
//! documented no-op — see `main/run.rs`), so a default `--screenshot` is byte-identical.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The image file EXTENSIONS a candidate asset must carry (lower-cased leaf ext).
/// The raster set the bundled `image` crate decodes, plus `svg` (a common markdown
/// asset the renderer can't decode but the user still keeps in `assets/`). Matching
/// on extension — not a magic-byte sniff — is deliberate: a file the DOCUMENTS refer
/// to by an image path IS an image for this scan's purpose, whatever its bytes.
pub const IMAGE_EXTS: &[&str] =
    &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico", "avif", "tif", "tiff", "heic"];

/// One UNREFERENCED image asset — a row in the "Clean unused assets…" picker. `rel`
/// is the project-root-relative path (forward-slashed; the picker's accept/trash key
/// AND its fuzzy corpus), `name` the leaf file name (the picker's PRIMARY cell),
/// `parent` the directory the file sits in (root-relative; the SECONDARY cell beside
/// the human size), and `size` its byte length (`None` when the backend can't stat it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    /// Root-relative path, forward-slashed — the accept/trash key + fuzzy corpus.
    pub rel: String,
    /// The leaf file name — the picker's primary cell.
    pub name: String,
    /// The parent directory (root-relative) — half the picker's secondary cell.
    pub parent: String,
    /// Byte length, or `None` when unavailable.
    pub size: Option<u64>,
}

/// A compact human byte size: `"0 B"`, `"742 B"`, `"12.3 KB"`, `"4.1 MB"`, `"1.0 GB"`.
/// One decimal place past a kilobyte, none below it. PURE — no locale, no clock — so
/// the picker's secondary column is deterministic + unit-testable.
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// The picker's SECONDARY (right-column) label for an orphan: its human size, then
/// its parent directory when it has one (`"12.3 KB · notes/assets"`; just the size
/// when the file sits at the root). An unknown size falls back to the parent alone,
/// or an empty label when neither is known. The size YIELDS first under width
/// pressure (it rides the recessive column, [`crate::render::rowlayout`]).
pub fn secondary_label(o: &Orphan) -> String {
    let size = o.size.map(human_size);
    match (size, o.parent.is_empty()) {
        (Some(s), false) => format!("{s} · {}", o.parent),
        (Some(s), true) => s,
        (None, false) => o.parent.clone(),
        (None, true) => String::new(),
    }
}

/// True when a root-relative path's LEAF name carries an image extension ([`IMAGE_EXTS`]).
fn is_image(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rsplit_once('.') {
        Some((_, ext)) => IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// True when SOME ANCESTOR directory of a root-relative path is literally `assets`
/// (`assets/x.png`, `notes/assets/x.png` → true; `x.png`, a top-level `assets.png`
/// file → false). The leaf name is excluded, so a file *named* `assets` never counts.
fn under_assets_dir(rel: &str) -> bool {
    let mut comps: Vec<&str> = rel.split('/').collect();
    comps.pop(); // drop the leaf file name — only ANCESTORS decide
    comps.iter().any(|c| *c == "assets")
}

/// True when a root-relative path is a markdown document (`.md` / `.markdown`,
/// case-insensitive) — the only files the scan reads for image references.
fn is_markdown(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rsplit_once('.') {
        Some((_, ext)) => matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown"),
        None => false,
    }
}

/// Split a root-relative path into `(leaf_name, parent_dir)` — both forward-slashed,
/// `parent_dir` empty for a top-level file.
fn split_rel(rel: &str) -> (String, String) {
    match rel.rsplit_once('/') {
        Some((dir, name)) => (name.to_string(), dir.to_string()),
        None => (rel.to_string(), String::new()),
    }
}

/// LEXICALLY normalize a path — collapse `.` and `..` components WITHOUT touching the
/// filesystem (no `canonicalize`, so it works over the in-memory backend + never
/// resolves symlinks). A leading `..` with nothing to pop is kept (it can't be
/// resolved lexically). This is what makes a candidate's `root/notes/assets/x.png`
/// and a reference's `root/notes/../notes/assets/x.png` compare EQUAL.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop a real segment; keep a leading `..` (nothing to pop) verbatim.
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve a document-relative image reference `path` against the document's own
/// directory `doc_dir`, normalized for comparison — the PATH-ONLY mirror of
/// `render::TextPipeline::resolve_image_path` (the semantics the renderer trusts): an
/// ABSOLUTE ref is used verbatim; a RELATIVE ref joins `doc_dir`. Lexically
/// normalized so `..` hops resolve to the same canonical form a candidate asset's
/// path takes.
fn resolve_ref(doc_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    let joined = if p.is_absolute() { p.to_path_buf() } else { doc_dir.join(p) };
    normalize(&joined)
}

/// THE SCAN. Given the project `root` and its root-relative file `corpus` (the exact
/// list the go-to picker fuzzy-matches — [`crate::index::build_index`], already
/// honouring the git / junk-dir rules), find every ORPHAN image: a candidate asset
/// (an image-extension file under an `assets/` directory) that no markdown document
/// references. Reads each document's text + each candidate's size through the
/// [`crate::fs`] seam, so the whole scan runs against an in-memory backend in tests.
/// Dot-hidden entries ([`crate::index::is_hidden_entry`]) are skipped on BOTH sides,
/// matching the picker's own dotfolder convention. The result is sorted by `rel` for
/// a stable, deterministic list.
pub fn scan(root: &Path, corpus: &[String]) -> Vec<Orphan> {
    let fs = crate::fs::active();
    // (1) Every image path any document references, resolved to a normalized absolute
    // path. Being INCLUSIVE here is the safe direction (a missed reference would
    // false-orphan a used file); the Trash's recoverability backs the rest up.
    let mut referenced: HashSet<PathBuf> = HashSet::new();
    for rel in corpus {
        if !is_markdown(rel) || crate::index::is_hidden_entry(rel) {
            continue;
        }
        let abs = root.join(rel);
        let Ok(text) = fs.read_to_string(&abs) else {
            continue;
        };
        let doc_dir = abs.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
        for img in crate::markdown::image_refs(&text) {
            referenced.insert(resolve_ref(&doc_dir, &img.path));
        }
    }
    // (2) Candidate assets minus the referenced set → the orphans.
    let mut orphans: Vec<Orphan> = Vec::new();
    for rel in corpus {
        if crate::index::is_hidden_entry(rel) || !under_assets_dir(rel) || !is_image(rel) {
            continue;
        }
        let abs = normalize(&root.join(rel));
        if referenced.contains(&abs) {
            continue;
        }
        let size = fs.metadata(&root.join(rel)).ok().and_then(|m| m.len);
        let (name, parent) = split_rel(rel);
        orphans.push(Orphan { rel: rel.clone(), name, parent, size });
    }
    orphans.sort_by(|a, b| a.rel.cmp(&b.rel));
    orphans
}

// --- The TRASH seam --------------------------------------------------------

/// The RECOVERABLE delete backend: move a file to the OS trash (never `rm`). Behind a
/// trait so a test injects a FAKE that records the request without touching a real
/// Trash — the same swap-a-backend idiom as [`crate::fs::FileSystem`]. The production
/// backend ([`SystemTrash`]) is macOS's `NSFileManager trashItemAtURL:` (via
/// `crate::mac_chrome`); other native targets return a calm `Err`.
pub trait TrashCan: Send + Sync {
    /// Move `path` to the OS trash, or `Err(message)` on failure (a missing file, an
    /// unsupported platform, an OS refusal). Never panics.
    fn trash(&self, path: &Path) -> Result<(), String>;
}

/// The platform trash — macOS `NSFileManager` (the single objc2 surface,
/// `crate::mac_chrome::trash_path`); every other native target reports unsupported.
struct SystemTrash;

impl TrashCan for SystemTrash {
    fn trash(&self, path: &Path) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            crate::mac_chrome::trash_path(path)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            Err("moving to Trash is only supported on macOS".to_string())
        }
    }
}

/// The process-global active trash backend (default [`SystemTrash`]); tests swap in a
/// fake. Behind an `RwLock<Arc<…>>` exactly like [`crate::fs`]'s backend, so no handle
/// threads through the apply seam.
fn global() -> &'static RwLock<Arc<dyn TrashCan>> {
    use std::sync::OnceLock;
    static TRASH: OnceLock<RwLock<Arc<dyn TrashCan>>> = OnceLock::new();
    TRASH.get_or_init(|| RwLock::new(Arc::new(SystemTrash)))
}

/// The ACTIVE trash backend — the live App routes `Effect::TrashAsset` through this.
pub fn active_trash() -> Arc<dyn TrashCan> {
    global().read().unwrap().clone()
}

/// Serializes every test that swaps the global trash backend (process-wide, like the
/// fs backend). Test-only.
#[cfg(test)]
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run `body` with `trash` installed as the active backend, restoring the previous
/// one afterwards (holding [`TEST_LOCK`]). Test-only — the fake-trash injection door.
#[cfg(test)]
pub(crate) fn with_trash<T>(trash: Arc<dyn TrashCan>, body: impl FnOnce() -> T) -> T {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = active_trash();
    *global().write().unwrap() = trash;
    let out = body();
    *global().write().unwrap() = prev;
    out
}

/// A recording FAKE trash for tests: appends every `trash`ed path to a shared list and
/// reports success, so a test can assert exactly which files were sent to the Trash
/// without touching a real one. Test-only.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct FakeTrash {
    pub trashed: std::sync::Mutex<Vec<PathBuf>>,
}

#[cfg(test)]
impl TrashCan for FakeTrash {
    fn trash(&self, path: &Path) -> Result<(), String> {
        self.trashed.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::InMemoryFs;
    use std::sync::Arc;

    /// A tiny in-memory project: markdown docs + asset files, returned with the
    /// root-relative corpus [`scan`] takes.
    fn project(files: &[(&str, &str)]) -> (Arc<InMemoryFs>, PathBuf, Vec<String>) {
        let root = PathBuf::from("/proj");
        let mut fs = InMemoryFs::new();
        let mut corpus = Vec::new();
        for (rel, body) in files {
            fs = fs.with_file(root.join(rel), body);
            corpus.push((*rel).to_string());
        }
        corpus.sort();
        (Arc::new(fs), root, corpus)
    }

    #[test]
    fn human_size_reads_calmly_across_magnitudes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(742), "742 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(12_600), "12.3 KB");
        assert_eq!(human_size(4_300_000), "4.1 MB");
        assert_eq!(human_size(1_100_000_000), "1.0 GB");
    }

    #[test]
    fn under_assets_dir_needs_an_ancestor_named_assets() {
        assert!(under_assets_dir("assets/x.png"));
        assert!(under_assets_dir("notes/assets/x.png"));
        assert!(under_assets_dir("a/b/assets/c/x.png"));
        assert!(!under_assets_dir("x.png"));
        assert!(!under_assets_dir("assets.png")); // a FILE named assets, not a dir
        assert!(!under_assets_dir("notes/pic.png"));
    }

    #[test]
    fn normalize_collapses_dot_and_dotdot() {
        assert_eq!(normalize(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("/a/./b")), PathBuf::from("/a/b"));
        assert_eq!(
            resolve_ref(Path::new("/proj/notes"), "../notes/assets/x.png"),
            PathBuf::from("/proj/notes/assets/x.png")
        );
        assert_eq!(
            resolve_ref(Path::new("/proj/notes"), "assets/x.png"),
            PathBuf::from("/proj/notes/assets/x.png")
        );
        // Absolute ref used verbatim (normalized).
        assert_eq!(
            resolve_ref(Path::new("/proj/notes"), "/proj/img/y.png"),
            PathBuf::from("/proj/img/y.png")
        );
    }

    #[test]
    fn scan_flags_only_the_unreferenced_asset() {
        let (fs, root, corpus) = project(&[
            ("doc.md", "text\n![a](assets/used.png)\nmore"),
            ("assets/used.png", "PNGDATA"),
            ("assets/orphan.png", "PNGDATA-longer"),
        ]);
        let orphans = crate::fs::with_fs(fs, || scan(&root, &corpus));
        assert_eq!(orphans.len(), 1, "exactly one orphan");
        assert_eq!(orphans[0].rel, "assets/orphan.png");
        assert_eq!(orphans[0].name, "orphan.png");
        assert_eq!(orphans[0].parent, "assets");
        assert_eq!(orphans[0].size, Some("PNGDATA-longer".len() as u64));
    }

    #[test]
    fn scan_resolves_nested_doc_references_with_dotdot_hops() {
        // A doc in notes/ referencing an asset via `../img/assets/` must NOT orphan it.
        let (fs, root, corpus) = project(&[
            ("notes/journal.md", "![p](../shared/assets/keep.png)"),
            ("shared/assets/keep.png", "DATA"),
            ("shared/assets/drop.png", "DATA"),
        ]);
        let orphans = crate::fs::with_fs(fs, || scan(&root, &corpus));
        let rels: Vec<&str> = orphans.iter().map(|o| o.rel.as_str()).collect();
        assert_eq!(rels, vec!["shared/assets/drop.png"]);
    }

    #[test]
    fn scan_counts_references_across_multiple_docs_and_ignores_loose_images() {
        let (fs, root, corpus) = project(&[
            ("a.md", "![x](assets/one.png)"),
            ("sub/b.md", "![y](assets/two.png)"), // resolves to sub/assets/two.png
            ("assets/one.png", "1"),
            ("sub/assets/two.png", "2"),
            ("assets/three.png", "3"),   // orphan (under assets/)
            ("loose.png", "4"),          // NOT under assets/ → never a candidate
            ("assets/note.txt", "hi"),   // not an image ext → never a candidate
        ]);
        let orphans = crate::fs::with_fs(fs, || scan(&root, &corpus));
        let rels: Vec<&str> = orphans.iter().map(|o| o.rel.as_str()).collect();
        assert_eq!(rels, vec!["assets/three.png"], "only the unreferenced assets/ image");
    }

    #[test]
    fn scan_skips_dot_hidden_docs_and_assets() {
        let (fs, root, corpus) = project(&[
            (".hidden/assets/x.png", "DATA"), // in a dotfolder → skipped as a candidate
            ("assets/y.png", "DATA"),         // a real orphan
        ]);
        let orphans = crate::fs::with_fs(fs, || scan(&root, &corpus));
        let rels: Vec<&str> = orphans.iter().map(|o| o.rel.as_str()).collect();
        assert_eq!(rels, vec!["assets/y.png"]);
    }

    #[test]
    fn scan_is_empty_when_every_asset_is_used() {
        let (fs, root, corpus) = project(&[
            ("doc.md", "![a](assets/a.png)\n![b](assets/b.png)"),
            ("assets/a.png", "A"),
            ("assets/b.png", "B"),
        ]);
        let orphans = crate::fs::with_fs(fs, || scan(&root, &corpus));
        assert!(orphans.is_empty());
    }

    #[test]
    fn secondary_label_pairs_size_and_parent() {
        let o = Orphan {
            rel: "notes/assets/x.png".into(),
            name: "x.png".into(),
            parent: "notes/assets".into(),
            size: Some(12_600),
        };
        assert_eq!(secondary_label(&o), "12.3 KB · notes/assets");
        let top = Orphan { rel: "x.png".into(), name: "x.png".into(), parent: String::new(), size: Some(5) };
        assert_eq!(secondary_label(&top), "5 B");
    }

    #[test]
    fn fake_trash_records_without_touching_disk() {
        let fake = Arc::new(FakeTrash::default());
        let f2 = fake.clone();
        with_trash(fake, || {
            active_trash().trash(Path::new("/proj/assets/x.png")).unwrap();
        });
        assert_eq!(f2.trashed.lock().unwrap().as_slice(), &[PathBuf::from("/proj/assets/x.png")]);
    }
}
