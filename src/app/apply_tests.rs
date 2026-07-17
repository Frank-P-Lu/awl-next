use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::buffer::Buffer;
use crate::config::Config;
use crate::fs::{FileSystem, InMemoryFs};

use super::App;

#[test]
fn pdf_export_writes_saved_sibling_and_scratch_notes_root_without_other_formats() {
    let saved_fs = InMemoryFs::new().with_dir("/docs");
    let mut saved = App::new_hermetic(None, PathBuf::from("/docs"), Config::empty());
    saved.buffer = Buffer::from_str("# Saved PDF\n\nSibling export body.\n");
    saved.buffer.set_path(PathBuf::from("/docs/draft.md"));
    crate::fs::with_fs(Arc::new(saved_fs.clone()), || {
        saved.export_document(crate::export::Format::Pdf);
        let pdf = saved_fs.read(Path::new("/docs/draft.pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.7\n"));
        assert!(!saved_fs.exists(Path::new("/docs/draft.docx")));
        assert!(!saved_fs.exists(Path::new("/docs/draft.html")));
        assert_eq!(saved.notice.as_deref(), Some("exported draft.pdf"));
    });

    let scratch_fs = InMemoryFs::new().with_dir("/notes");
    let mut scratch = App::new_hermetic(None, PathBuf::from("/project"), Config::empty());
    scratch.notes_root = PathBuf::from("/notes");
    scratch.buffer = Buffer::from_str("# Scratch PDF\n\nNotes-root export body.\n");
    crate::fs::with_fs(Arc::new(scratch_fs.clone()), || {
        scratch.export_document(crate::export::Format::Pdf);
        let target = Path::new("/notes/scratch-pdf.pdf");
        let pdf = scratch_fs.read(target).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.7\n"));
        assert_eq!(
            scratch.notice.as_deref(),
            Some("exported /notes/scratch-pdf.pdf")
        );
    });
}
