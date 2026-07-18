//! `card` — the ONE summoned float-card mechanism shared by awl's three summoned
//! cards: the About card (`about.rs`), the Lifetime-stats card (`lifetime.rs`),
//! and the hold-⌘ shortcut peek (`peek.rs`). Each is a THIN instance over this
//! one owner (same-behavior-same-code), so the three copies of the open-flag +
//! dismiss intercept collapse to a single source:
//!
//!  * [`CardFlag`] — the process-global OPEN boolean every card wants, with the
//!    identical open/close/read surface. `about`/`lifetime`/`peek` each own one
//!    and expose it under their own verb (`about_open`/`set_open`, …), so the
//!    MECHANISM is shared while each card's public API — and its tests — are
//!    unchanged.
//!  * [`dismiss_summoned_card`] — the any-key / any-click DISMISS intercept for
//!    the two MODAL cards (About + Lifetime stats, which OWN the next key). Both
//!    `actions::apply_core`'s top-of-function arm and the live App's mouse-press
//!    handler dismiss through this ONE door instead of a per-card check+close.
//!
//! All three render through the SAME float-card pipeline
//! (`render/chrome/hud.rs::prepare_hud`, gated on their open flags). The hold-⌘
//! peek is deliberately NOT part of [`dismiss_summoned_card`]: it is not modal —
//! it closes when the hold breaks (`peek::PeekArm`), never on a key.

use std::sync::atomic::{AtomicBool, Ordering};

/// A summoned-card OPEN flag: the process-global drawn-boolean every card wants,
/// with the identical open/close/read surface. Held as a `static` per card and
/// wrapped by that card's own-verb accessors (`about_open`/`set_open`, …), so the
/// flag boilerplate lives here ONCE.
pub struct CardFlag(AtomicBool);

impl CardFlag {
    /// A CLOSED flag — the calm-room default (no card drawn until summoned).
    pub const fn new() -> Self {
        CardFlag(AtomicBool::new(false))
    }
    /// True while the card is summoned / drawn.
    pub fn is_open(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    /// Open or close the card explicitly.
    pub fn set_open(&self, open: bool) {
        self.0.store(open, Ordering::Relaxed);
    }
}

/// Dismiss whichever MODAL summoned card (About, Lifetime stats, or Writing
/// streaks) is open, returning `true` iff one WAS open (and is now closed). THE
/// one owner of the "a modal card OWNS the next key/click" intercept:
/// `actions::apply_core`'s top-of-function arm and the live App's mouse-press
/// handler both call this instead of duplicating a per-card check+close. They are
/// mutually exclusive (each opens only after the palette that summoned it closed,
/// and each dismisses on the first key), so closing "the open one" is the whole
/// contract. One carve-out lives UPSTREAM of this door: while the streaks card
/// is open, `apply_core` intercepts ←/→ to flip its heatmap⇄cumulative page
/// (`streaks::toggle_view`) before ever reaching here — every other key still
/// dismisses. The
/// hold-⌘ peek is deliberately absent — it is not modal (it closes when the hold
/// breaks, via `peek::PeekArm`).
pub fn dismiss_summoned_card() -> bool {
    if crate::about::about_open() {
        crate::about::set_open(false);
        return true;
    }
    if crate::lifetime::lifetime_open() {
        crate::lifetime::set_open(false);
        return true;
    }
    if crate::streaks::streaks_open() {
        crate::streaks::set_open(false);
        return true;
    }
    false
}
