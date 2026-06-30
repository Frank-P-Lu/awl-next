//! QUICK-NOTE NAMING + FILE MOVES — the pure helpers behind note auto-naming and
//! the C-x m / live-rename file operations: `first_nonempty_line` (a note's working
//! title), `slugify` / `note_stem` / `slug_core` (title -> filename stem), and
//! `unique_path` / `move_file` / `rename_to_stem` / `stem_matches_slug` (no-clobber
//! path selection + true renames over the filesystem seam). Free functions carved
//! out of `buffer.rs` verbatim; glob-re-exported from the module root so the
//! `crate::buffer::*` call sites resolve unchanged.

use std::path::{Path, PathBuf};

/// The first line of `text` with non-whitespace content (trimmed), or `None` when
/// the text is empty / all blank. This is a quick note's working TITLE.
pub fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().map(|l| l.trim()).find(|l| !l.is_empty())
}

/// Slugify a note's first line into a lowercase, dash-separated filename STEM:
/// runs of non-alphanumeric chars collapse to a single dash, edges trimmed
/// (e.g. "Japanese week 12" -> "japanese-week-12"). An empty/punctuation-only
/// line yields "note" so there is always a usable name. (The note save uses
/// [`slug_core`] directly with a "scratch" fallback; this stays for the slug
/// contract + its unit test.)
#[allow(dead_code)]
pub fn slugify(line: &str) -> String {
    let out = slug_core(line);
    if out.is_empty() {
        "note".to_string()
    } else {
        out
    }
}

/// The raw slug for `line`: lowercase alphanumerics with non-alphanumeric runs
/// collapsed to single dashes (edges trimmed). Returns an EMPTY string when the
/// line has no alphanumeric content, so the caller can decide a fallback (the
/// note save falls back to the "scratch" placeholder; [`slugify`] falls back to
/// "note"). A single word stays a single word ("foo" -> "foo").
/// The filename STEM a note's first `line` derives to: its [`slug_core`], or the
/// "scratch" placeholder when the line has no slug-able (alphanumeric) content.
/// Shared by the FIRST naming save and live-rename so both agree on the name.
pub fn note_stem(line: &str) -> String {
    let s = slug_core(line);
    if s.is_empty() {
        "scratch".to_string()
    } else {
        s
    }
}

fn slug_core(line: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in line.chars() {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            for lc in c.to_lowercase() {
                out.push(lc);
            }
        } else {
            pending_dash = true;
        }
    }
    out
}

/// MOVE the file at `old` into `dest_dir`, KEEPING its filename: create the
/// destination directory if needed, never clobber an existing same-named file
/// there (append a numeric suffix on collision), and `std::fs::rename` (a true
/// move, not a copy). Returns the new path; an already-in-place move is a no-op
/// returning `old`. This is the only file-WRITE the move feature performs, scoped
/// to the current note (the C-x m fence: create + move, nothing else).
pub fn move_file(old: &Path, dest_dir: &Path) -> std::io::Result<PathBuf> {
    crate::fs::active().create_dir_all(dest_dir)?;
    let filename = old
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    let natural = dest_dir.join(&filename);
    if natural == old {
        return Ok(old.to_path_buf()); // already there
    }
    let new_path = if crate::fs::active().exists(&natural) {
        let p = Path::new(&filename);
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = p.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        unique_path(dest_dir, &stem, &ext)
    } else {
        natural
    };
    crate::fs::active().rename(old, &new_path)?;
    Ok(new_path)
}

/// True when `cur` already represents a note titled `stem` — either the exact
/// slug or that slug plus a numeric collision suffix (`japanese-week-12`,
/// `japanese-week-12-2`, …). Live-rename uses this to AVOID churning a file
/// whose name only differs by the `-N` that disambiguated it from a same-titled
/// sibling: such a file already tracks its title and must be left alone.
fn stem_matches_slug(cur: &str, stem: &str) -> bool {
    if cur == stem {
        return true;
    }
    cur.strip_prefix(stem)
        .and_then(|rest| rest.strip_prefix('-'))
        .map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false)
}

/// LIVE-RENAME the note at `old` so its filename STEM becomes `stem`, keeping its
/// extension + directory. A no-op (returns `old`) when the name already tracks
/// `stem` ([`stem_matches_slug`]) — so a collision-suffixed note isn't churned.
/// Otherwise pick a NON-CLOBBERING `<stem>.<ext>` in the same dir and
/// `std::fs::rename` there (a true move, never a copy); creates the parent dir if
/// needed, mirroring [`move_file`]. Returns the new path. This is the only
/// file-WRITE live-rename performs (the same fence as the C-x m move).
pub fn rename_to_stem(old: &Path, stem: &str) -> std::io::Result<PathBuf> {
    let cur_stem = old
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if stem_matches_slug(&cur_stem, stem) {
        return Ok(old.to_path_buf()); // already named for this title
    }
    let dir = old.parent().map(Path::to_path_buf).unwrap_or_default();
    let ext = old
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    crate::fs::active().create_dir_all(&dir)?;
    // `old`'s stem differs from `stem` (we passed the guard above), so the
    // no-clobber scan never points back at `old` itself.
    let new_path = unique_path(&dir, stem, &ext);
    crate::fs::active().rename(old, &new_path)?;
    Ok(new_path)
}

/// A NON-CLOBBERING path in `dir` for `stem`.`ext` (`ext` empty = no extension):
/// returns `<dir>/<stem>.<ext>` if free, else the first free `<stem>-2.<ext>`,
/// `<stem>-3.<ext>`, … So a note title collision (or a move into a folder that
/// already holds a same-named file) appends a short numeric suffix rather than
/// overwriting.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let name = |suffix: Option<u32>| -> String {
        let base = match suffix {
            None => stem.to_string(),
            Some(n) => format!("{stem}-{n}"),
        };
        if ext.is_empty() {
            base
        } else {
            format!("{base}.{ext}")
        }
    };
    let mut candidate = dir.join(name(None));
    let mut n = 2u32;
    while crate::fs::active().exists(&candidate) {
        candidate = dir.join(name(Some(n)));
        n += 1;
    }
    candidate
}
