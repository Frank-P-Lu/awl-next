//! DOCUMENT EXPORT — `.docx` (Word), standalone `.html`, and native `.pdf`, from
//! awl's plain-text markdown. ONE exporter core ([`model::parse`], a single
//! walk of `pulldown-cmark`'s events into a neutral [`model::Document`] tree),
//! with shared DOCX/HTML emitters and a native-only PDF emitter. The file on disk
//! stays plain text; export just projects it into a rich, portable document.
//!
//! Coverage: headings, bold/italic/strikethrough, `==highlight==`, inline +
//! fenced code, links (real hyperlinks), bullet/numbered/task lists,
//! blockquotes, thematic rules, GFM tables, and embedded images. Frontmatter is
//! excluded (it never renders); footnotes are out of scope.
//!
//! Every emitter is PURE + DETERMINISTIC: a function of the markdown text plus a
//! caller-supplied [`model::ImageSource`] (the live App reads the doc's `assets/`
//! off disk; tests hand in a fixed map), so the same document always exports the
//! same bytes — the golden-file gate depends on it. The DOCX container is built
//! by the hand-rolled STORED-ZIP writer ([`zip`], no new runtime deps, byte-
//! stable); the model, DOCX, and HTML paths compile identically on native and
//! wasm, while the PDF shaping/emitter stack is native-only.

mod docx;
mod html;
mod model;
#[cfg(not(target_arch = "wasm32"))]
mod pdf;
mod zip;

// Native-only: the export tests read on-disk golden files at runtime
// (`CARGO_MANIFEST_DIR`/`std::fs`) and build the fixture PNG through the
// native-only `paste_image` encoder — neither exists on wasm. The export
// logic itself (model/docx/html) still compiles on every target.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

pub use model::{ExportImage, ImageSource};

/// The export format a command targets — drives the sibling-file extension + the
/// emitter chosen. Kept here (not in `commands.rs`) so the whole export surface
/// lives in one module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Microsoft Word `.docx` (OOXML).
    Docx,
    /// Standalone print-tuned `.html`.
    Html,
    /// Self-contained A4 `.pdf` (native only).
    #[cfg(not(target_arch = "wasm32"))]
    Pdf,
}

impl Format {
    /// The file extension (no dot) a sibling export writes.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Docx => "docx",
            Format::Html => "html",
            #[cfg(not(target_arch = "wasm32"))]
            Format::Pdf => "pdf",
        }
    }

    /// The MIME type of the produced bytes (the web download shim's Blob type).
    /// Only read on the wasm export path (`trigger_download_bytes`); native
    /// writes the bytes to disk with no MIME involved.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn mime(self) -> &'static str {
        match self {
            Format::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
            Format::Html => "text/html",
            #[cfg(not(target_arch = "wasm32"))]
            Format::Pdf => "application/pdf",
        }
    }
}

/// Export `markdown` to `.docx` bytes, resolving images through `images`.
pub fn to_docx(markdown: &str, images: &dyn ImageSource) -> Vec<u8> {
    let doc = model::parse(markdown);
    docx::emit(&doc, images)
}

/// Export `markdown` to a standalone HTML string, resolving images through
/// `images`.
pub fn to_html(markdown: &str, images: &dyn ImageSource) -> String {
    let doc = model::parse(markdown);
    html::emit(&doc, images)
}

/// Export `markdown` to a native, self-contained A4 PDF. The browser build does
/// not compile the shaping/emitter stack and deliberately has no PDF API.
#[cfg(not(target_arch = "wasm32"))]
pub fn to_pdf(markdown: &str, images: &dyn ImageSource) -> Vec<u8> {
    let doc = model::parse(markdown);
    pdf::emit(&doc, images)
}

/// The native `--bench-perf` harness's fixed PDF-font coverage probe.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pdf_glyph_probe() -> usize {
    pdf::glyph_probe()
}

/// Export `markdown` in `format`, returning the raw bytes to write/download.
pub fn to_bytes(markdown: &str, format: Format, images: &dyn ImageSource) -> Vec<u8> {
    match format {
        Format::Docx => to_docx(markdown, images),
        Format::Html => to_html(markdown, images).into_bytes(),
        #[cfg(not(target_arch = "wasm32"))]
        Format::Pdf => to_pdf(markdown, images),
    }
}

/// The image resolver the live App uses: resolve each markdown `src` against the
/// document's directory (relative refs) or take it as-is (absolute — the scratch
/// buffer's `assets/` path), read the bytes through the filesystem seam, and
/// sniff its intrinsic dimensions. A remote URL, a missing/unreadable file, or
/// an unsupported encoding yields `None`, so the image gracefully degrades to
/// its alt text. Works on every platform through [`crate::fs::active`]; on wasm
/// (localStorage-backed) a real disk asset simply won't be found, which is the
/// correct no-op.
pub struct FsImages {
    /// The directory the document lives in, for resolving relative `src`s. `None`
    /// for a path-less scratch buffer (only absolute refs resolve).
    pub doc_dir: Option<std::path::PathBuf>,
}

impl ImageSource for FsImages {
    fn resolve(&self, src: &str) -> Option<ExportImage> {
        // Remote references are never fetched (the zero-network invariant).
        let lower = src.to_ascii_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("data:")
        {
            return None;
        }
        let path = std::path::Path::new(src);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.doc_dir.as_ref()?.join(path)
        };
        let bytes = crate::fs::active().read(&resolved).ok()?;
        let (width, height, mime) = model::sniff_image(&bytes)?;
        Some(ExportImage {
            bytes,
            width,
            height,
            mime,
        })
    }
}
