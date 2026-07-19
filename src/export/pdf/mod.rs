//! Native, in-process PDF export: the neutral export tree is shaped with the
//! same cosmic-text engine as awl, then painted as positioned TrueType glyphs
//! into deterministic A4 pages. No clock, host font, network, compression, or
//! platform API enters the byte stream.

mod flow;
mod fonts;
mod images;
mod inline;
mod layout;
mod manifest;
mod writer;

/// Fixed PDF font-coverage probe exposed only to the crate's performance harness.
pub(super) fn glyph_probe() -> usize {
    fonts::glyph_probe()
}

use super::model::{Document, ImageSource};

pub(super) fn emit(doc: &Document, images: &dyn ImageSource) -> Vec<u8> {
    let layout = layout::build(doc, images);
    let metadata = manifest::build(doc, &layout);
    writer::emit(&layout, &metadata)
}

#[cfg(test)]
mod tests;
