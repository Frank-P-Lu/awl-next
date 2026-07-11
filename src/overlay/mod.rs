//! The SUMMONED, TRANSIENT navigation overlay (go-to file / switch project /
//! one-level browse).
//!
//! The overlay is NOT a sidebar/tree/tabs: it appears, is used, and VANISHES on
//! pick. While it is `Some`, typed chars edit the overlay QUERY (never the
//! buffer), Up/Down move the selection, Enter opens the highlighted item, and
//! Esc/C-g cancels. All of that is driven through `actions::apply_core`, so the
//! `--keys` headless replay can open it, type to filter, move, and accept — the
//! whole flow stays agent-verifiable and serializable to the capture sidecar.
//!
//! Three kinds share the one card:
//!   * `Goto`    — the active project's flat file index (fuzzy jump).
//!   * `Project` — a real, navigable FILE EXPLORER for picking the active root.
//!     It starts at the `--workspace` dir but navigates by ABSOLUTE path. It is a
//!     PROJECT PICKER first: Enter PICKS the highlighted folder as the new root
//!     (the synthetic `.` row picks the CURRENT directory). Right DESCENDS into a
//!     folder to pick a subfolder; Left / Backspace ASCENDS (even ABOVE the
//!     workspace). Git folders carry a dim `git` tag in the row's secondary column.
//!   * `Browse`  — ONE directory level at a time for the active root. Enter on a
//!     FOLDER descends (the list becomes that folder's children); Left/Backspace
//!     ASCENDS; Enter on a FILE opens it and closes. Git folders are marked. It
//!     is still summoned + transient — it vanishes on open/cancel, never a tree.

mod build;
mod capture;
mod facet;
mod kind;
mod nav;
mod state;

pub use build::{browse_level, build, elide_path, row_split, BuildCtx};
pub use capture::{Capture, CaptureStage, LinkEdit, LinkEditMode, RenameEdit, ValueEdit};
#[allow(unused_imports)] // used by overlay::tests (format_hint/HintAction directly; PIN_TAG below)
pub use kind::{format_hint, AcceptDisposition, HintAction, OverlayKind, HINT_SEP, PIN_TAG};
pub use state::OverlayState;

#[cfg(test)]
mod tests;
