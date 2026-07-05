//! The native macOS MENU BAR: a THIRD DOOR to actions that already live in
//! [`crate::commands::COMMANDS`] — never new behavior, never a menu-only code
//! path. `cfg(target_os = "macos")` only; Linux/wasm never see a menu bar (a
//! documented v1 scope trim, not a bug — muda supports gtk on Linux, but
//! wiring it is left for a future round; wasm has no native chrome at all).
//!
//! **The design law:** every routed menu item fires an `Action` the
//! `commands.rs` catalog already dispatches, through the SAME apply seam a
//! keypress uses (`App::apply`, via `App::handle_menu_event` in
//! `app/menu.rs`). This file owns exactly ONE table ([`SECTIONS`]) mapping a
//! muda item id to a catalog command NAME — it both feeds [`roster`] (what
//! gets BUILT) and [`resolve`] (what a fired id RESOLVES to), so a typo'd or
//! renamed command name fails the law test below instead of silently building
//! a dead menu item.
//!
//! **Two doors, one owner:**
//! - [`roster`] is a PURE data description of the whole menu bar (titles +
//!   items, no muda calls) — buildable and assertable from ANY thread,
//!   including a `cargo test` worker. This is what the tests below check.
//! - [`build_menu`] translates that SAME roster into real `muda::Menu`/
//!   `Submenu`/`MenuItem` types. It is LIVE-ONLY: muda's macOS backend
//!   requires the process's TRUE main thread (`MainThreadMarker::new()`
//!   panics otherwise — confirmed empirically: a root `muda::Menu` built off
//!   the main thread panics with "can only be created on the main thread",
//!   even under `cfg(test)`), so it is exercised only by the live app via
//!   [`install`], never by a unit test.
//!
//! **ACCELERATOR DECISION (researched, not guessed):** every routed command
//! already has a keymap-owned chord (native and/or emacs slot in
//! `commands::COMMANDS`). On macOS, an `NSMenuItem` key equivalent ALWAYS
//! intercepts that key combination in `NSApplication::sendEvent:` — via
//! `keyWindow performKeyEquivalent:` then a `mainMenu performKeyEquivalent:`
//! fallback — BEFORE the event ever reaches `keyDown:`/winit's key path
//! (there is no "display-only, non-intercepting" key equivalent in AppKit;
//! muda's `Accelerator` sets the same real key-equivalent slot either way).
//! So v1 registers `None` for every routed item's accelerator uniformly: the
//! chord keeps firing through the keymap exactly as it does today (recoil
//! juice, input stamping, debug `key→px` timing all intact), and the menu is
//! a second, accelerator-less door to the same `Action` — "menu shows the
//! item, the chord keeps working through the keymap as today" is the
//! documented lesser evil versus double-dispatch semantics or a stolen chord.
//! (One item, Quit, has NO native macOS chord in the catalog at all — a
//! `Cmd-Q` accelerator there would collide with nothing — but is left
//! unaccelerated too, for uniformity with this decision and because adding
//! Cmd-Q as a *keymap* chord, if ever wanted, belongs in `commands.rs`, not
//! bolted on only in the menu.)
//!
//! **QUIT is ROUTED, not muda's `PredefinedMenuItem::quit()` (a deliberate,
//! evidence-based deviation from "predefined items where possible"):** muda's
//! predefined Quit sends the `terminate:` selector straight to `NSApplication`
//! (confirmed in muda 0.19.3's macOS backend,
//! `PredefinedMenuItemType::Quit => Some(sel!(terminate:))`), which does NOT
//! run through winit's event loop at all — `App::exiting()` (the hook that
//! flushes autosave, session-restore, and the daemon-socket teardown; see
//! CLAUDE.md's Autosave/Daemon sections) is only ever called by
//! `ActiveEventLoop::exit()`'s own clean-shutdown path, which `terminate:`
//! never touches. A routed Quit item instead fires the EXISTING
//! `Action::Quit`, which already signals `Effect::Quit` → `App::apply` calls
//! `event_loop.exit()` — the identical path Cmd-P → Quit / `C-x C-c` take
//! today — so autosave/session/daemon teardown all still run. `About` stays
//! muda's predefined item: it is genuinely OS chrome (a system dialog) with
//! no app state to flush, so there is no correctness reason to route it.
//!
//! **LIVE-ONLY (needs human confirmation):** the bar actually appearing, an
//! item firing under a real click, About's panel + Quit's teardown, and
//! macOS text-services behavior in the Edit menu (see `app/menu.rs`'s module
//! doc for why Edit uses routed items, not muda's predefined Cut/Copy/Paste/
//! Undo/Redo). The harness proves the roster/routing DATA and the resolve
//! direction; it cannot drive an NSMenu click.
#![cfg(target_os = "macos")]

use crate::commands;
use crate::keymap::Action;
use muda::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

/// One ROUTED menu item: the muda [`muda::MenuId`] string assigned to a plain
/// [`MenuItem`], and the exact `commands::COMMANDS` display NAME it fires on
/// activation. See the module doc: [`roster`] and [`resolve`] both walk this
/// data, so there is exactly one place naming which command a menu item runs.
struct Routed {
    id: &'static str,
    command: &'static str,
}

/// App menu's one routed item — Quit (see the module doc for why it is
/// routed rather than muda's predefined Quit).
const APP_ITEMS: &[Routed] = &[Routed { id: "awl.quit", command: "Quit" }];

const FILE_ITEMS: &[Routed] = &[
    Routed { id: "awl.new_note", command: "New note" },
    // "Open…" is the Finder-style "choose a file" affordance — the closest
    // catalog match is "Browse files" (a file-tree picker), not the fuzzy
    // "Go to file" quick-open. The label below stays "Browse files" (menus
    // teach the SAME words Cmd-P does), documented here rather than silently.
    Routed { id: "awl.open", command: "Browse files" },
    Routed { id: "awl.save", command: "Save" },
    Routed { id: "awl.finish_buffer", command: "Finish Buffer" },
];

const EDIT_ITEMS: &[Routed] = &[
    Routed { id: "awl.undo", command: "Undo" },
    Routed { id: "awl.redo", command: "Redo" },
    Routed { id: "awl.cut", command: "Cut" },
    Routed { id: "awl.copy", command: "Copy" },
    Routed { id: "awl.paste", command: "Paste" },
    Routed { id: "awl.select_all", command: "Select all" },
];

const VIEW_ITEMS: &[Routed] = &[
    Routed { id: "awl.toggle_page_mode", command: "Toggle page mode" },
    Routed { id: "awl.switch_theme", command: "Switch theme" },
    Routed { id: "awl.focus_mode", command: "Focus mode" },
    Routed { id: "awl.zoom_in", command: "Zoom in" },
    Routed { id: "awl.zoom_out", command: "Zoom out" },
    Routed { id: "awl.reset_zoom", command: "Reset zoom" },
    Routed { id: "awl.toggle_debug", command: "Toggle Debug" },
];

/// Every routed section, in build order — the ONE thing [`resolve`] and the
/// law test below walk, so a new section added to [`roster`] is automatically
/// covered by both.
const SECTIONS: &[&[Routed]] = &[APP_ITEMS, FILE_ITEMS, EDIT_ITEMS, VIEW_ITEMS];

/// A muda PREDEFINED item this menu bar uses — no `Action`, no catalog entry:
/// genuinely OS chrome (a system dialog / window-manager command), never
/// app behavior (see the module doc's Quit-vs-predefined decision for why
/// that boundary is drawn here and not wider).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredefinedKind {
    About,
    Minimize,
    Maximize,
}

/// One item in a menu's PURE structure (see [`roster`]) — either a routed
/// command (resolved via [`resolve`]), a predefined item, or a separator.
#[derive(Debug, PartialEq)]
pub enum RosterItem {
    Routed { id: &'static str, label: &'static str },
    Predefined(PredefinedKind),
    Separator,
}

/// One top-level menu in the PURE roster: a title + its items, in order.
#[derive(Debug, PartialEq)]
pub struct RosterMenu {
    pub title: &'static str,
    pub items: Vec<RosterItem>,
}

fn routed(r: &Routed) -> RosterItem {
    RosterItem::Routed { id: r.id, label: r.command }
}

/// The FULL menu bar structure, in build order — pure data, ZERO muda calls,
/// so it is buildable and assertable from any thread (see the module doc for
/// why [`build_menu`], unlike this, is live-only). [`build_menu`] translates
/// this EXACT data into real muda types, so the built menu can never diverge
/// from what this function (and its tests) describe.
pub fn roster() -> Vec<RosterMenu> {
    vec![
        RosterMenu {
            title: "awl",
            items: vec![
                RosterItem::Predefined(PredefinedKind::About),
                RosterItem::Separator,
                routed(&APP_ITEMS[0]), // Quit
            ],
        },
        RosterMenu {
            title: "File",
            items: vec![
                routed(&FILE_ITEMS[0]), // New note
                routed(&FILE_ITEMS[1]), // Browse files ("Open…")
                RosterItem::Separator,
                routed(&FILE_ITEMS[2]), // Save
                routed(&FILE_ITEMS[3]), // Finish Buffer
            ],
        },
        RosterMenu {
            title: "Edit",
            items: vec![
                routed(&EDIT_ITEMS[0]), // Undo
                routed(&EDIT_ITEMS[1]), // Redo
                RosterItem::Separator,
                routed(&EDIT_ITEMS[2]), // Cut
                routed(&EDIT_ITEMS[3]), // Copy
                routed(&EDIT_ITEMS[4]), // Paste
                RosterItem::Separator,
                routed(&EDIT_ITEMS[5]), // Select all
            ],
        },
        RosterMenu {
            title: "View",
            items: vec![
                routed(&VIEW_ITEMS[0]), // Toggle page mode
                routed(&VIEW_ITEMS[1]), // Switch theme
                routed(&VIEW_ITEMS[2]), // Focus mode
                RosterItem::Separator,
                routed(&VIEW_ITEMS[3]), // Zoom in
                routed(&VIEW_ITEMS[4]), // Zoom out
                routed(&VIEW_ITEMS[5]), // Reset zoom
                RosterItem::Separator,
                routed(&VIEW_ITEMS[6]), // Toggle Debug
            ],
        },
        RosterMenu {
            title: "Window",
            items: vec![
                RosterItem::Predefined(PredefinedKind::Minimize),
                RosterItem::Predefined(PredefinedKind::Maximize),
            ],
        },
    ]
}

/// Resolve a fired muda item id (its raw [`muda::MenuId`] string) back to the
/// `Action` it routes to, via `commands::action_for_name` — the SAME catalog
/// lookup the config `[keys]` rebinder uses, so a routed item can never name
/// an action the catalog doesn't recognize. `None` for an id this table
/// doesn't own (a predefined item, or a stray/foreign event) — a silent,
/// harmless no-op at the `App::handle_menu_event` seam, never a panic.
pub fn resolve(id: &str) -> Option<Action> {
    SECTIONS.iter().flat_map(|s| s.iter()).find(|r| r.id == id).and_then(|r| commands::action_for_name(r.command))
}

/// One routed [`RosterItem`] translated into a real, id-carrying, ACCELERATOR-
/// LESS [`MenuItem`] (see the module doc's accelerator decision).
fn to_menu_item(id: &'static str, label: &'static str) -> MenuItem {
    MenuItem::with_id(id, label, true, None)
}

/// Translate one [`PredefinedKind`] into muda's real predefined item.
fn to_predefined(kind: PredefinedKind) -> PredefinedMenuItem {
    match kind {
        PredefinedKind::About => PredefinedMenuItem::about(
            Some("About awl"),
            Some(AboutMetadata { name: Some("awl".into()), ..Default::default() }),
        ),
        PredefinedKind::Minimize => PredefinedMenuItem::minimize(None),
        PredefinedKind::Maximize => PredefinedMenuItem::maximize(None),
    }
}

/// Build the whole menu bar as real muda types, from [`roster`] verbatim.
///
/// **LIVE-ONLY / main-thread-only:** muda's macOS backend calls
/// `MainThreadMarker::new().expect(..)` when constructing a root [`Menu`],
/// with NO `cfg(test)` exemption (unlike its `Submenu` constructor, which
/// does special-case tests) — confirmed empirically: calling this off the
/// real process main thread panics. It is therefore called exactly once, live,
/// from `crate::menu::install` (via `resumed()`), never from a unit test —
/// see [`roster`]'s tests for the structure this function is not re-tested
/// against directly.
pub fn build_menu() -> Menu {
    let submenus: Vec<Submenu> = roster()
        .into_iter()
        .map(|m| {
            let items: Vec<Box<dyn muda::IsMenuItem>> = m
                .items
                .iter()
                .map(|item| -> Box<dyn muda::IsMenuItem> {
                    match item {
                        RosterItem::Routed { id, label } => Box::new(to_menu_item(id, label)),
                        RosterItem::Separator => Box::new(PredefinedMenuItem::separator()),
                        RosterItem::Predefined(kind) => Box::new(to_predefined(*kind)),
                    }
                })
                .collect();
            let refs: Vec<&dyn muda::IsMenuItem> = items.iter().map(|b| b.as_ref()).collect();
            Submenu::with_items(m.title, true, &refs).expect("submenu build")
        })
        .collect();
    let refs: Vec<&dyn muda::IsMenuItem> = submenus.iter().map(|s| s as &dyn muda::IsMenuItem).collect();
    Menu::with_items(&refs).expect("root menu build")
}

/// Build + install the menu bar for the running NSApp (`Menu::init_for_nsapp`,
/// itself main-thread-only), and register muda's global event handler to
/// forward every fired item's id into the live winit event loop via `proxy` —
/// mirroring `crate::daemon::spawn_accept_thread`'s own "hand the live App an
/// `EventLoopProxy`, forward posted events" shape (the SAME seam the daemon
/// built; see `crate::app::AwlEvent`). `wrap` lets the caller name its own
/// event-enum variant (`AwlEvent::Menu`) without this module depending on
/// `crate::app`'s types — the same decoupling `spawn_accept_thread` uses.
///
/// Call exactly ONCE, from `resumed()`, after the window (and therefore
/// NSApp) exists.
pub fn install<E: Send + 'static>(
    proxy: winit::event_loop::EventLoopProxy<E>,
    wrap: impl Fn(String) -> E + Send + Sync + 'static,
) {
    let menu = build_menu();
    menu.init_for_nsapp();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = proxy.send_event(wrap(event.id().0.clone()));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LAW TEST: every routed table entry's `command` name must resolve to a
    /// real catalog `Action` — a walk of every section, so a typo'd or
    /// renamed command name in this file fails a test instead of silently
    /// building a dead menu item.
    #[test]
    fn every_routed_command_exists_in_the_catalog() {
        for section in SECTIONS {
            for r in *section {
                assert!(
                    commands::action_for_name(r.command).is_some(),
                    "menu id {:?} names {:?}, which is not a commands::COMMANDS entry",
                    r.id,
                    r.command
                );
            }
        }
    }

    /// Every routed id is UNIQUE — muda dispatches purely by id string, so a
    /// collision would silently alias two different menu items to whichever
    /// table entry `resolve` happens to find first.
    #[test]
    fn every_routed_id_is_unique() {
        let mut ids: Vec<&str> = SECTIONS.iter().flat_map(|s| s.iter()).map(|r| r.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate menu id in the routed table");
    }

    /// `resolve` round-trips every table entry back to its exact catalog
    /// Action — the id → Action direction `App::handle_menu_event` depends on.
    #[test]
    fn resolve_round_trips_every_routed_entry() {
        for section in SECTIONS {
            for r in *section {
                let want = commands::action_for_name(r.command);
                assert_eq!(resolve(r.id), want, "resolve({:?}) must match the catalog", r.id);
            }
        }
    }

    /// An unknown id resolves to nothing (never panics) — a predefined item's
    /// muda event (About/Minimize/Maximize/separator — none of which route
    /// through this table) or any stray event must be a harmless no-op.
    #[test]
    fn unknown_id_resolves_to_none() {
        assert_eq!(resolve("awl.nonexistent"), None);
        assert_eq!(resolve(""), None);
    }

    /// The ROSTER'S structure: five top-level menus, in the documented order,
    /// each carrying the exact routed/predefined/separator sequence spelled
    /// out in `roster()` above. Pure data — no muda calls, so this runs on
    /// any test thread (unlike `build_menu`, which is main-thread-only; see
    /// its own doc).
    #[test]
    fn roster_has_the_five_top_level_menus_in_order() {
        let menus = roster();
        let titles: Vec<&str> = menus.iter().map(|m| m.title).collect();
        assert_eq!(titles, vec!["awl", "File", "Edit", "View", "Window"]);
    }

    #[test]
    fn roster_app_menu_is_about_then_separator_then_routed_quit() {
        let menus = roster();
        let app = &menus[0];
        assert_eq!(app.items.len(), 3);
        assert_eq!(app.items[0], RosterItem::Predefined(PredefinedKind::About));
        assert_eq!(app.items[1], RosterItem::Separator);
        assert_eq!(app.items[2], RosterItem::Routed { id: "awl.quit", label: "Quit" });
    }

    #[test]
    fn roster_window_menu_is_minimize_then_maximize_predefined_only() {
        let menus = roster();
        let window = menus.iter().find(|m| m.title == "Window").unwrap();
        assert_eq!(
            window.items,
            vec![
                RosterItem::Predefined(PredefinedKind::Minimize),
                RosterItem::Predefined(PredefinedKind::Maximize),
            ]
        );
    }

    /// Every routed table entry (APP/FILE/EDIT/VIEW) appears EXACTLY once
    /// somewhere in the roster, so `roster()` can never silently drop or
    /// duplicate a catalog-backed item relative to the routing table.
    #[test]
    fn roster_contains_every_routed_table_entry_exactly_once() {
        let menus = roster();
        let roster_ids: Vec<&str> = menus
            .iter()
            .flat_map(|m| m.items.iter())
            .filter_map(|i| match i {
                RosterItem::Routed { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let mut table_ids: Vec<&str> = SECTIONS.iter().flat_map(|s| s.iter()).map(|r| r.id).collect();
        let mut sorted_roster = roster_ids.clone();
        sorted_roster.sort_unstable();
        table_ids.sort_unstable();
        assert_eq!(sorted_roster, table_ids, "roster() must place every routed table entry exactly once");
    }

    /// Every routed item's LABEL matches its `commands::COMMANDS` display name
    /// exactly (menus teach the same words Cmd-P does).
    #[test]
    fn roster_routed_labels_match_the_command_catalog_display_names() {
        for menu in roster() {
            for item in menu.items {
                if let RosterItem::Routed { id, label } = item {
                    let r = SECTIONS.iter().flat_map(|s| s.iter()).find(|r| r.id == id).unwrap();
                    assert_eq!(label, r.command);
                }
            }
        }
    }
}
