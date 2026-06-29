//! Ruby syntax lexer — STUB.
//!
//! Not yet implemented: [`spans`] returns an empty list, so a Ruby buffer renders
//! plain (the default ink everywhere) and `cargo` stays green. Implementing this is
//! a SELF-CONTAINED edit to THIS file only — copy the structure of the reference
//! lexer in [`crate::syntax::rust`] (and [`crate::syntax::python`]): a minimal
//! hand-written byte scanner that emits the four Alabaster roles
//! ([`SynKind::Comment`], [`SynKind::Str`], [`SynKind::Constant`],
//! [`SynKind::Definition`]) and leaves everything else as the default ink. Keep
//! the exact `spans` signature below; the dispatch in `mod.rs` already calls it.
//!
//! TODO(lang): implement Ruby highlighting (see rust.rs for the template).

use super::SynKind;
use std::ops::Range;

/// Syntax spans for Ruby source — see the module docs. STUB: returns no spans.
pub fn spans(_text: &str) -> Vec<(Range<usize>, SynKind)> {
    Vec::new()
}
