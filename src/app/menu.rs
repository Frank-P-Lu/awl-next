//! The live App's half of the native macOS MENU BAR (`crate::menu` owns the
//! pure roster/routing table + the muda construction; this file is the
//! App-specific wiring — routing a fired menu item's id into the SAME
//! `App::apply` seam every keypress uses). Native macOS only
//! (`cfg(target_os = "macos")`); see `crate::menu`'s module doc for the full
//! design-law + accelerator/Quit decisions.
//!
//! **Edit menu correctness note (why routed items, not muda's predefined
//! Cut/Copy/Paste/Undo/Redo):** muda's `PredefinedMenuItem::cut/copy/paste/
//! select_all/undo/redo` work by sending AppKit selectors (`cut:`, `copy:`,
//! …) up the RESPONDER CHAIN to the key window's `firstResponder` — the
//! mechanism a standard `NSTextView` implements for free. awl's document view
//! is a raw wgpu-rendered `NSView` (via winit) that implements none of those
//! selectors, so a predefined item would validate/fire against nothing and
//! silently no-op. Routing Edit's items through the SAME id → `Action` table
//! every other menu uses instead (`Action::Undo`/`CopyRegion`/`KillRegion`/
//! `Yank`/`SelectAll`, all already fired via clipboard mirroring in
//! `App::apply` — see `actions.rs`'s module doc) is both the ONLY choice that
//! actually works against this app's view and the one consistent with the
//! module's "every item fires an existing catalog Action" law. The "free
//! correctness win" the mac-citizen brief names is satisfied a different way
//! than muda's out-of-the-box predefined items: simply having a populated
//! Edit menu (regardless of how its items dispatch) is what lets macOS offer
//! its Edit-menu-anchored text services (the Character Viewer / Emoji &
//! Symbols item, Services menu entries) at all — a structural presence
//! requirement, not a responder-chain one.
#![cfg(target_os = "macos")]

use super::*;

impl App {
    /// A menu item fired (posted via `EventLoopProxy::send_event`, so this
    /// always runs on the normal winit thread — the same cross-thread-safety
    /// shape as `handle_daemon_event`). Resolves `id` through `crate::menu`'s
    /// ONE routing table and fires it through the SAME `App::apply` seam a
    /// keypress uses (`shift: false` — a menu click carries no modifier-hold
    /// concept); an id the table doesn't own (a predefined item muda itself
    /// handled, or a stray event) is a silent no-op, never a panic.
    pub(super) fn handle_menu_event(&mut self, id: String, event_loop: &ActiveEventLoop) {
        if let Some(action) = crate::menu::resolve(&id) {
            self.apply(action, false, event_loop);
        }
    }
}
