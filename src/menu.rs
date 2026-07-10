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
//! `event_loop.exit()` — the identical path Cmd-P → Quit / `Cmd-Q` take
//! today — so autosave/session/daemon teardown all still run.
//!
//! **ABOUT is ALSO ROUTED now (v1 shipped it as muda's predefined About; this
//! round replaced it) — for TWO independent reasons, only the first of which
//! was ever really about About specifically:**
//! 1. **The real bug this round found + fixed lived one layer BELOW About: a
//!    Rust-side use-after-free in [`install`]**, not anything About-specific.
//!    `crate::menu::install` used to return `()` and just let its built `Menu`
//!    fall out of scope — but every native `NSMenuItem` muda builds stashes a
//!    RAW (non-retaining) pointer back into that `Menu`'s owned
//!    `Rc<RefCell<MenuChild>>` chain, so once the Rust side dropped it, EVERY
//!    item (About, Quit, every routed item, Window's still-predefined Minimize/
//!    Maximize) pointed at freed memory — clicking literally any of them was a
//!    use-after-free, confirmed empirically to manifest two different ways
//!    (an `Icon`-decode panic reading corrupted `AboutMetadata` bytes in one
//!    repro; a clean `SIGSEGV` null-deref in another) purely depending on what
//!    reused that freed allocation by click time. `install` now returns the
//!    `Menu` and `App` keeps it alive for the app's whole lifetime — see both
//!    docs. This alone made every predefined AND routed item safe again.
//! 2. **Separately, About is now an in-app card** (`about.rs` +
//!    `render/chrome.rs`, reusing the HUD's float-card pipeline) rather than
//!    AppKit's stock About dialog, so it reads as awl chrome (one warm accent,
//!    `base_300` card, the active world's own name + end-mark ornament) instead
//!    of a generic system panel, and so it works identically on Linux (no
//!    native menu bar there) and is `--keys`/sidecar-drivable like the rest of
//!    the app. This is a taste upgrade, not a correctness fix — the About
//!    dialog itself never touched an icon unless `AboutMetadata.icon` was
//!    `Some` (it wasn't), so it was never the actual crash source; see (1).
//!
//! **LIVE-ONLY (needs human confirmation):** the bar actually appearing, an
//! item firing under a real click, the About card's actual pixel look + Quit's
//! teardown, and macOS text-services behavior in the Edit menu (see
//! `app/menu.rs`'s module doc for why Edit uses routed items, not muda's
//! predefined Cut/Copy/Paste/Undo/Redo). The harness proves the roster/routing
//! DATA and the resolve direction; it cannot drive an NSMenu click.
//!
//! **CROSS-PLATFORM SPLIT (the web/Linux menu-bar round):** the PURE ROSTER —
//! [`SECTIONS`] / [`roster`] / [`resolve`] / [`print_roster`] and their data types —
//! is compiled on EVERY target now, because the awl-RENDERED menu bar
//! ([`crate::menubar`], shown on web + Linux where the OS gives no chrome) reads the
//! SAME roster the macOS NSMenu bar does: ONE roster, three consumers (the native
//! bar, the awl renderer, the law tests). Only the muda CONSTRUCTION
//! ([`build_menu`] / [`install`] / [`to_menu_item`] / [`to_predefined`]) and the
//! icon set stay `#[cfg(target_os = "macos")]` — muda is a macOS-only dependency.

use crate::commands;
use crate::keymap::Action;
#[cfg(target_os = "macos")]
use crate::menu_icons;
#[cfg(target_os = "macos")]
use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

/// One ROUTED menu item: the muda [`muda::MenuId`] string assigned to a plain
/// [`MenuItem`], the exact `commands::COMMANDS` display NAME it fires on
/// activation, its menu-facing DISPLAY LABEL, and whether it carries an
/// ICON (see `menu_icons.rs`; `false` for the great majority — Apple's own
/// apps stay text-mostly, a logged taste call). `label` equals `command` for
/// every item except the two macOS App-menu conventions ("Quit Awl" / "About
/// Awl" append the app name, per every stock macOS app) — see the module doc
/// and the law test below, which enumerates that exact exception rather than
/// silently allowing labels to drift from the catalog everywhere.
struct Routed {
    id: &'static str,
    command: &'static str,
    label: &'static str,
    icon: bool,
}

/// Build a [`Routed`] whose menu-facing label is IDENTICAL to its catalog
/// command name and carries NO icon (the common case — everywhere except the
/// two macOS App-menu conventions in [`APP_ITEMS`], which spell their labels
/// out explicitly, and the small `ri`-built icon set below).
const fn r(id: &'static str, command: &'static str) -> Routed {
    Routed { id, command, label: command, icon: false }
}

/// Like [`r`], but flagged to carry an icon (`menu_icons::icon_for(id)`) — see
/// that module's doc for the small, deliberately minimal set this is used for.
const fn ri(id: &'static str, command: &'static str) -> Routed {
    Routed { id, command, label: command, icon: true }
}

/// App menu's THREE routed items — About (an in-app card, see `about.rs`),
/// Settings (P1 of the keybinding-idiom audit — Cmd-, is the preferences
/// chord since Mac OS X 10.1), and Quit (see the module doc for why all three
/// are routed rather than muda's predefined items). About's and Quit's labels
/// append "Awl" per the stock macOS App-menu convention (every system app's
/// About/Quit items name the app); Settings keeps its bare catalog name
/// ("Settings…" is already unambiguous). All three CATALOG names ("About" /
/// "Settings…" / "Quit") stay what the Cmd-P palette shows.
const APP_ITEMS: &[Routed] = &[
    Routed { id: "awl.about", command: "About", label: "About Awl", icon: false },
    Routed { id: "awl.settings", command: "Settings…", label: "Settings…", icon: false },
    Routed { id: "awl.quit", command: "Quit", label: "Quit Awl", icon: false },
];

const FILE_ITEMS: &[Routed] = &[
    ri("awl.new_note", "New note"),
    // "Open…" is the Finder-style "choose a file" affordance — the closest
    // catalog match is "Browse files…" (a file-tree picker), not the fuzzy
    // "Go to file…" quick-open. The label below stays "Browse files…" (menus
    // teach the SAME words Cmd-P does), documented here rather than silently.
    ri("awl.open", "Browse files…"),
    ri("awl.switch_project", "Switch project…"),
    // "Recent projects" is a SINGLE File item that opens the SWITCH-PROJECT
    // navigator pre-lensed onto its Recent lens (`Action::OpenRecentProjects` — the
    // fold that retired the standalone RecentProjects picker), not a dynamic
    // Open-Recent SUBMENU of the roots themselves — a deliberate scope choice: this
    // menu bar is PURE STATIC DATA routed by an id → catalog-command-NAME table
    // ([`SECTIONS`]), and each recent root is runtime state, not a catalog command,
    // so it has no place in that table. The navigator (fuzzy-filterable,
    // keyboard-drivable, shared with the palette command) is the one door; a live
    // submenu is a possible future round. No icon (kept minimal, like most items).
    r("awl.recent_projects", "Recent projects…"),
    ri("awl.save", "Save"),
    ri("awl.finish_buffer", "Finish file"),
];

const EDIT_ITEMS: &[Routed] = &[
    r("awl.undo", "Undo"),
    r("awl.redo", "Redo"),
    r("awl.cut", "Cut"),
    r("awl.copy", "Copy"),
    r("awl.paste", "Paste"),
    r("awl.select_all", "Select all"),
];

const VIEW_ITEMS: &[Routed] = &[
    r("awl.toggle_page_mode", "Toggle page mode"),
    ri("awl.switch_theme", "Switch theme…"),
    r("awl.zoom_in", "Zoom in"),
    r("awl.zoom_out", "Zoom out"),
    r("awl.reset_zoom", "Reset zoom"),
    r("awl.toggle_debug", "Toggle debug"),
];

/// Every routed section, in build order — the ONE thing [`resolve`] and the
/// law test below walk, so a new section added to [`roster`] is automatically
/// covered by both.
const SECTIONS: &[&[Routed]] = &[APP_ITEMS, FILE_ITEMS, EDIT_ITEMS, VIEW_ITEMS];

/// A muda PREDEFINED item this menu bar uses — no `Action`, no catalog entry:
/// genuinely OS chrome (a window-manager command), never app behavior (see
/// the module doc's Quit/About-vs-predefined decisions for why that boundary
/// is drawn here and not wider — both are now routed instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredefinedKind {
    Minimize,
    Maximize,
    /// App-menu Hide (⌘H, macOS gives the accelerator for free). P3 of the
    /// keybinding-idiom audit.
    Hide,
    /// App-menu Hide Others (⌥⌘H).
    HideOthers,
    /// App-menu Show All (no default accelerator).
    ShowAll,
}

/// One item in a menu's PURE structure (see [`roster`]) — either a routed
/// command (resolved via [`resolve`]), a predefined item, or a separator.
#[derive(Debug, PartialEq)]
pub enum RosterItem {
    Routed { id: &'static str, label: &'static str, icon: bool },
    Predefined(PredefinedKind),
    Separator,
}

/// One top-level menu in the PURE roster: a title + its items, in order.
#[derive(Debug, PartialEq)]
pub struct RosterMenu {
    pub title: &'static str,
    pub items: Vec<RosterItem>,
}

fn routed(item: &Routed) -> RosterItem {
    RosterItem::Routed { id: item.id, label: item.label, icon: item.icon }
}

/// The menu bar structure for THIS COMPILED PLATFORM (`commands::Platform::current()`)
/// — pure data, ZERO muda calls, so it is buildable and assertable from any thread
/// (see the module doc for why [`build_menu`], unlike this, is live-only).
/// [`build_menu`] translates this EXACT data into real muda types, so the built menu
/// can never diverge from what this function (and its tests) describe. On native this
/// is BYTE-IDENTICAL to the full roster [`roster_all`] describes (nothing is hidden);
/// on web it is [`roster_all`] filtered through [`roster_for`] — see that function's
/// doc for what drops and why.
pub fn roster() -> Vec<RosterMenu> {
    roster_for(commands::Platform::current())
}

/// [`roster`], parameterized by an EXPLICIT platform — the seam that lets a native-run
/// test assert the WEB-filtered roster (`roster_for(Platform::Web)`) without any `cfg!`
/// gymnastics or an actual wasm build. PLATFORM-SCOPED COMMANDS: a ROUTED item whose
/// catalog action is unavailable on `platform` (`commands::action_available`) is
/// dropped; a PREDEFINED item (genuine OS window-manager chrome — Minimize/Zoom) is
/// dropped outright on `Platform::Web` (there is no OS window to minimize/zoom in a
/// browser tab) and kept on `Platform::Native` (every native platform, including
/// Linux, where the awl-rendered bar still shows them as the existing inert dead
/// rows — unchanged v1 behavior, only web newly prunes them). Any separator left
/// dangling by a drop (leading, trailing, or doubled-up) is trimmed so the visible
/// list never opens or closes on a rule. A menu left with ZERO items after filtering
/// is dropped entirely — this is what removes the whole Window menu on web (both its
/// items are predefined) and the "Quit Awl" + the predefined Hide block (both
/// dropped on web — Quit is `native_only`, Hide/Hide Others/Show All are OS window
/// chrome) plus their now-dangling separators from the App menu, leaving "About Awl"
/// and "Settings…" (neither `native_only`) with exactly one separator between them.
pub fn roster_for(platform: commands::Platform) -> Vec<RosterMenu> {
    roster_all()
        .into_iter()
        .map(|m| RosterMenu { title: m.title, items: filter_items_for_platform(m.items, platform) })
        .filter(|m| !m.items.is_empty())
        .collect()
}

/// Filter one menu's ITEMS for `platform` (the per-item availability rules described
/// in [`roster_for`]'s doc), then trim any now-dangling separator.
fn filter_items_for_platform(items: Vec<RosterItem>, platform: commands::Platform) -> Vec<RosterItem> {
    let kept: Vec<RosterItem> = items
        .into_iter()
        .filter(|item| match item {
            RosterItem::Routed { id, .. } => {
                resolve(id).map(|a| commands::action_available(&a, platform)).unwrap_or(true)
            }
            RosterItem::Predefined(_) => platform == commands::Platform::Native,
            RosterItem::Separator => true, // dangling ones trimmed below
        })
        .collect();
    trim_separators(kept)
}

/// Drop a LEADING separator, collapse consecutive separators to one, and drop a
/// TRAILING separator — so a menu whose surrounding items got filtered away never
/// opens or closes on a bare rule.
fn trim_separators(items: Vec<RosterItem>) -> Vec<RosterItem> {
    let mut out: Vec<RosterItem> = Vec::new();
    for item in items {
        if matches!(item, RosterItem::Separator)
            && (out.is_empty() || matches!(out.last(), Some(RosterItem::Separator)))
        {
            continue;
        }
        out.push(item);
    }
    if matches!(out.last(), Some(RosterItem::Separator)) {
        out.pop();
    }
    out
}

/// The FULL, UNFILTERED menu bar structure, in build order — every platform's items,
/// before [`roster_for`]'s per-platform filter runs. `pub(crate)`/private helper for
/// [`roster_for`]/[`roster`]; the public door is always one of those two.
fn roster_all() -> Vec<RosterMenu> {
    vec![
        RosterMenu {
            title: "awl",
            items: vec![
                routed(&APP_ITEMS[0]), // About Awl
                RosterItem::Separator,
                routed(&APP_ITEMS[1]), // Settings…
                RosterItem::Separator,
                // The standard macOS App-menu Hide block (P3) — genuine OS
                // window-manager commands with no app state, the same
                // predefined class as Window's Minimize/Maximize below.
                RosterItem::Predefined(PredefinedKind::Hide),
                RosterItem::Predefined(PredefinedKind::HideOthers),
                RosterItem::Predefined(PredefinedKind::ShowAll),
                RosterItem::Separator,
                routed(&APP_ITEMS[2]), // Quit Awl
            ],
        },
        RosterMenu {
            title: "File",
            items: vec![
                routed(&FILE_ITEMS[0]), // New note
                routed(&FILE_ITEMS[1]), // Browse files… ("Open…")
                routed(&FILE_ITEMS[2]), // Switch project…
                routed(&FILE_ITEMS[3]), // Recent projects… (opens the picker)
                RosterItem::Separator,
                routed(&FILE_ITEMS[4]), // Save
                routed(&FILE_ITEMS[5]), // Finish file
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
                routed(&VIEW_ITEMS[1]), // Switch theme…
                RosterItem::Separator,
                routed(&VIEW_ITEMS[2]), // Zoom in
                routed(&VIEW_ITEMS[3]), // Zoom out
                routed(&VIEW_ITEMS[4]), // Reset zoom
                RosterItem::Separator,
                routed(&VIEW_ITEMS[5]), // Toggle debug
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

/// The NATIVE chord for a routed command NAME, CONVENTION-RESOLVED
/// (`commands::resolved_native_label`, e.g. `"Cmd-O"` -> `"⌘O"` on Mac / `"Ctrl+O"`
/// on Linux) for the awl-rendered menu bar's secondary column, or `""` for a
/// palette-only command with no native chord. Cross-platform (the awl bar shows on
/// web/Linux — this is the ONE label door that surface reads). Reads the SAME
/// catalog [`commands::COMMANDS`] the palette does, so a menu item's chord can
/// never drift from the command it fires.
pub fn item_chord(command: &str) -> String {
    commands::COMMANDS
        .iter()
        .find(|c| c.name == command)
        .map(|c| commands::resolved_native_label(c, crate::convention::Convention::current()))
        .unwrap_or_default()
}

/// The native chord GLYPHS for a routed menu item by its muda `id` (the id -> command
/// -> [`item_chord`] hop), or `""` for an id this table doesn't own / a command with
/// no native chord. The awl-rendered dropdown uses THIS (it carries the item's `id`,
/// not its command name — the two differ for the App-menu "About Awl"/"Quit Awl"
/// items, which have no chord anyway).
pub fn item_chord_for_id(id: &str) -> String {
    SECTIONS
        .iter()
        .flat_map(|s| s.iter())
        .find(|r| r.id == id)
        .map(|r| item_chord(r.command))
        .unwrap_or_default()
}

/// One routed [`RosterItem`] translated into a real, id-carrying, ACCELERATOR-
/// LESS menu item (see the module doc's accelerator decision) — an
/// [`muda::IconMenuItem`] when `icon` is set AND `menu_icons::icon_for`
/// actually resolves one for this id (see that module's safety-guarded
/// construction), else a plain [`MenuItem`] (also the fallback if the icon
/// somehow fails to resolve — never a missing/dead menu item over a missing
/// icon).
#[cfg(target_os = "macos")]
fn to_menu_item(id: &'static str, label: &'static str, icon: bool) -> Box<dyn muda::IsMenuItem> {
    if icon {
        if let Some(icon) = menu_icons::icon_for(id) {
            return Box::new(muda::IconMenuItem::with_id(id, label, true, Some(icon), None));
        }
    }
    Box::new(MenuItem::with_id(id, label, true, None))
}

/// Translate one [`PredefinedKind`] into muda's real predefined item.
#[cfg(target_os = "macos")]
fn to_predefined(kind: PredefinedKind) -> PredefinedMenuItem {
    match kind {
        PredefinedKind::Minimize => PredefinedMenuItem::minimize(None),
        PredefinedKind::Maximize => PredefinedMenuItem::maximize(None),
        PredefinedKind::Hide => PredefinedMenuItem::hide(None),
        PredefinedKind::HideOthers => PredefinedMenuItem::hide_others(None),
        PredefinedKind::ShowAll => PredefinedMenuItem::show_all(None),
    }
}

/// The ACTUAL AppKit-displayed label for a predefined item — muda's own
/// `PredefinedMenuItemType::text()` on macOS (`&Minimize` -> "Minimize" once
/// its mnemonic `&` is stripped, `Maximize` -> "Zoom", the real macOS
/// convention muda itself special-cases per-platform). Kept as a small,
/// hand-verified pair here rather than depending on muda's private `text()`,
/// so [`print_roster`] (and therefore `scripts/smoke-menus.sh`, which drives
/// real menu clicks by exactly this displayed text) can never silently name
/// an item AppKit doesn't actually show.
pub fn predefined_label(kind: PredefinedKind) -> &'static str {
    match kind {
        PredefinedKind::Minimize => "Minimize",
        PredefinedKind::Maximize => "Zoom",
        PredefinedKind::Hide => "Hide",
        PredefinedKind::HideOthers => "Hide Others",
        PredefinedKind::ShowAll => "Show All",
    }
}

/// Print the WHOLE menu bar roster as plain, greppable lines — one per
/// CLICKABLE item (separators dropped), `<top-level menu title>\t<item
/// label>` — to stdout, then return. This is the ONE door the live-smoke
/// harness (`scripts/smoke-menus.sh`) uses to enumerate exactly what to
/// click: it shells out to `awl --print-menu-roster` and reads this output,
/// so the roster it drives can never silently drift from [`roster`] itself
/// (the same data `build_menu` translates into the real menu bar). Reachable
/// from ANY thread (pure data, like `roster` itself) — `main.rs` calls this
/// before ever touching a window, so it works even with no display attached.
#[cfg(target_os = "macos")]
pub fn print_roster() {
    for menu in roster() {
        for item in menu.items {
            let label = match item {
                RosterItem::Routed { label, .. } => label,
                RosterItem::Predefined(kind) => predefined_label(kind),
                RosterItem::Separator => continue,
            };
            println!("{}\t{}", menu.title, label);
        }
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
#[cfg(target_os = "macos")]
pub fn build_menu() -> Menu {
    let submenus: Vec<Submenu> = roster()
        .into_iter()
        .map(|m| {
            let items: Vec<Box<dyn muda::IsMenuItem>> = m
                .items
                .iter()
                .map(|item| -> Box<dyn muda::IsMenuItem> {
                    match item {
                        RosterItem::Routed { id, label, icon } => to_menu_item(id, label, *icon),
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
/// **Returns the built [`Menu`] — the CALLER MUST KEEP IT ALIVE for as long as
/// the app runs.** This is not cosmetic: every native `NSMenuItem` muda builds
/// stashes a RAW pointer (`ivars().set(&*self)`, no retain) back to its Rust-side
/// `MenuChild`, whose actual allocation is owned by an `Rc<RefCell<MenuChild>>`
/// chain rooted in this `Menu` value. `Menu::init_for_nsapp` hands the NATIVE
/// `NSMenu`/`NSMenuItem` objects to AppKit (which retains those fine), but does
/// nothing to keep the RUST-side `Rc` chain alive — if this return value is
/// simply dropped (the v1 bug: it used to be a local that fell out of scope at
/// the end of this very function), every `MenuChild` is freed while AppKit's
/// native items still point at that freed memory, and clicking ANY item later
/// (About, Quit, a routed item, even a menu built with no icons at all) is a
/// clean use-after-free — confirmed empirically: it manifested as an
/// `Icon`-decoding panic in one repro and a bare `SIGSEGV` null-deref in
/// another, purely depending on what reused that freed memory by click time.
/// `App` stores this in a field for its whole lifetime; see its doc.
///
/// Call exactly ONCE, from `resumed()`, after the window (and therefore
/// NSApp) exists.
#[cfg(target_os = "macos")]
#[must_use = "the returned Menu must be kept alive for the app's lifetime — see this fn's doc"]
pub fn install<E: Send + 'static>(
    proxy: winit::event_loop::EventLoopProxy<E>,
    wrap: impl Fn(String) -> E + Send + Sync + 'static,
) -> Menu {
    let menu = build_menu();
    menu.init_for_nsapp();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = proxy.send_event(wrap(event.id().0.clone()));
    }));
    menu
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

    /// DRIFT GUARD (single-owner): `commands::menu_section` is the CROSS-PLATFORM
    /// owner of "which menu section a command sits under" (this file is macOS-only,
    /// so the palette's File/Edit/View lenses can't reference `SECTIONS` directly —
    /// see `commands.rs`'s module note). This test pins the two representations in
    /// lockstep: every File/Edit/View menu item's command reports the MATCHING
    /// section, and the App-menu items (About/Quit) report `None`. A rename in either
    /// place fails here instead of silently splitting the menu from the palette.
    #[test]
    fn routed_sections_match_command_section() {
        for (items, expect) in [
            (APP_ITEMS, None),
            (FILE_ITEMS, Some("File")),
            (EDIT_ITEMS, Some("Edit")),
            (VIEW_ITEMS, Some("View")),
        ] {
            for r in items {
                assert_eq!(
                    commands::menu_section(r.command),
                    expect,
                    "menu item {:?} ({:?}) must agree with commands::menu_section",
                    r.id,
                    r.command
                );
            }
        }
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

    /// LAW: the awl-RENDERED menu bar (`crate::menubar` + `render/chrome/menubar.rs`)
    /// reads THIS roster on web/Linux exactly as the macOS NSMenu bar does. This pins
    /// what the renderer needs from EVERY roster item so a future roster change can't
    /// silently leave the rendered bar with a dead row: a `Routed` item's `id` must
    /// `resolve` to a real Action (the fire path) AND its `item_chord_for_id` must not
    /// panic; a `Predefined` item must have a non-empty display label. (The renderer's
    /// own `match` over `RosterItem` — Routed / Predefined / Separator — is the
    /// no-wildcard compile-time guard; this pins the DATA each arm consumes is present.)
    #[test]
    fn renderer_consumes_every_roster_item() {
        for menu in roster() {
            for item in &menu.items {
                match item {
                    RosterItem::Routed { id, .. } => {
                        assert!(
                            resolve(id).is_some(),
                            "rendered bar item {id:?} resolves to no Action (dead row)"
                        );
                        // The secondary-column chord lookup must never panic (empty is
                        // fine for a palette-only command like About/Quit).
                        let _ = item_chord_for_id(id);
                    }
                    RosterItem::Predefined(kind) => {
                        assert!(!predefined_label(*kind).is_empty(), "predefined {kind:?} has no label");
                    }
                    RosterItem::Separator => {}
                }
            }
        }
    }

    /// An unknown id resolves to nothing (never panics) — a predefined item's
    /// muda event (Minimize/Maximize/separator — none of which route through
    /// this table) or any stray event must be a harmless no-op.
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
    fn roster_app_menu_is_about_settings_hide_block_then_quit() {
        // The standard macOS App-menu shape: About · —sep— · Settings… · —sep—
        // · Hide / Hide Others / Show All (predefined) · —sep— · Quit.
        let menus = roster();
        let app = &menus[0];
        assert_eq!(
            app.items,
            vec![
                RosterItem::Routed { id: "awl.about", label: "About Awl", icon: false },
                RosterItem::Separator,
                RosterItem::Routed { id: "awl.settings", label: "Settings…", icon: false },
                RosterItem::Separator,
                RosterItem::Predefined(PredefinedKind::Hide),
                RosterItem::Predefined(PredefinedKind::HideOthers),
                RosterItem::Predefined(PredefinedKind::ShowAll),
                RosterItem::Separator,
                RosterItem::Routed { id: "awl.quit", label: "Quit Awl", icon: false },
            ]
        );
    }

    /// The File menu's exact clustered sequence: New note · Open… · Switch
    /// project… · Recent projects… · —sep— · Save · Finish file, with the iconed
    /// items flagged and "Recent projects…" (a plain, un-iconed picker door)
    /// placed just after Switch project… — pinned so the cluster can't silently
    /// reorder or lose/gain a flag.
    #[test]
    fn roster_file_menu_is_the_iconed_open_switch_save_cluster() {
        let menus = roster();
        let file = menus.iter().find(|m| m.title == "File").unwrap();
        assert_eq!(
            file.items,
            vec![
                RosterItem::Routed { id: "awl.new_note", label: "New note", icon: true },
                RosterItem::Routed { id: "awl.open", label: "Browse files…", icon: true },
                RosterItem::Routed { id: "awl.switch_project", label: "Switch project…", icon: true },
                RosterItem::Routed { id: "awl.recent_projects", label: "Recent projects…", icon: false },
                RosterItem::Separator,
                RosterItem::Routed { id: "awl.save", label: "Save", icon: true },
                RosterItem::Routed { id: "awl.finish_buffer", label: "Finish file", icon: true },
            ]
        );
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
    /// exactly (menus teach the same words Cmd-P does) — EXCEPT the two
    /// enumerated macOS App-menu conventions (`awl.about` / `awl.quit`), whose
    /// labels append "Awl" per every stock system app's About/Quit items. This
    /// is a real law for File/Edit/View (a typo there would silently diverge
    /// the menu from the palette), narrowed by name rather than left open.
    #[test]
    fn roster_routed_labels_match_the_command_catalog_display_names() {
        const APP_NAME_SUFFIXED: &[&str] = &["awl.about", "awl.quit"];
        for menu in roster() {
            for item in menu.items {
                if let RosterItem::Routed { id, label, .. } = item {
                    let r = SECTIONS.iter().flat_map(|s| s.iter()).find(|r| r.id == id).unwrap();
                    if APP_NAME_SUFFIXED.contains(&id) {
                        assert_ne!(label, r.command, "{id:?} is expected to differ from its bare catalog name");
                    } else {
                        assert_eq!(label, r.command);
                    }
                }
            }
        }
    }

    /// ICON FLAGS: a routed item's `icon: true` in the roster must ALWAYS
    /// resolve a real icon via `menu_icons::icon_for`, and — the converse,
    /// equally important half — an item that does NOT carry the flag must
    /// have NO icon registered for its id either. Either direction drifting
    /// (a flagged id with no drawn glyph, or a drawn glyph nobody flags) would
    /// silently diverge `roster()`'s pure data from what `build_menu` actually
    /// constructs, since `to_menu_item` only ever consults `menu_icons` when
    /// the flag is set. (macOS-only: `menu_icons` — like muda — is macOS-gated; the
    /// awl-rendered bar draws no icons, so the roster's `icon` flag is inert there.)
    #[cfg(target_os = "macos")]
    #[test]
    fn icon_flagged_routed_items_agree_with_menu_icons_exactly() {
        for menu in roster() {
            for item in menu.items {
                if let RosterItem::Routed { id, icon, .. } = item {
                    assert_eq!(
                        menu_icons::icon_for(id).is_some(),
                        icon,
                        "{id:?}: roster icon flag ({icon}) must match menu_icons::icon_for's presence"
                    );
                }
            }
        }
    }

    // ── PLATFORM-SCOPED COMMANDS: web filtering (all run on the native test binary,
    // asserting `roster_for(Platform::Web)` directly — see `commands::Platform`'s doc
    // for why a native-run test can assert the web view without an actual wasm build).

    /// `roster()` (this compiled platform, native under `cargo test`) is BYTE-IDENTICAL
    /// to `roster_for(Platform::Native)` — the compiled-platform door is exactly the
    /// explicit-platform door with `Platform::current()` filled in, never a second copy.
    #[test]
    fn roster_native_matches_roster_for_native_explicitly() {
        assert_eq!(roster(), roster_for(commands::Platform::Native));
    }

    /// The App menu on web drops "Quit Awl" (native_only) and the predefined
    /// Hide/Hide Others/Show All block (OS window chrome, pruned on web outright),
    /// along with every separator left dangling by those drops — keeping "About Awl"
    /// and "Settings…" (neither is `native_only`) with exactly one separator between
    /// them.
    #[test]
    fn web_roster_app_menu_keeps_about_and_settings_drops_quit_and_hide_block() {
        let menus = roster_for(commands::Platform::Web);
        let app = menus.iter().find(|m| m.title == "awl").unwrap();
        assert_eq!(
            app.items,
            vec![
                RosterItem::Routed { id: "awl.about", label: "About Awl", icon: false },
                RosterItem::Separator,
                RosterItem::Routed { id: "awl.settings", label: "Settings…", icon: false },
            ]
        );
    }

    /// The File menu on web drops "Recent projects…" and "Finish file" (both
    /// `native_only`), keeping the rest — including the separator that still has real
    /// items on both sides of it.
    #[test]
    fn web_roster_file_menu_drops_recent_projects_and_finish_file() {
        let menus = roster_for(commands::Platform::Web);
        let file = menus.iter().find(|m| m.title == "File").unwrap();
        assert_eq!(
            file.items,
            vec![
                RosterItem::Routed { id: "awl.new_note", label: "New note", icon: true },
                RosterItem::Routed { id: "awl.open", label: "Browse files…", icon: true },
                RosterItem::Routed { id: "awl.switch_project", label: "Switch project…", icon: true },
                RosterItem::Separator,
                RosterItem::Routed { id: "awl.save", label: "Save", icon: true },
            ]
        );
    }

    /// Edit and View are untouched on web (none of their routed commands are
    /// `native_only`) — byte-identical to the native roster's own Edit/View menus.
    #[test]
    fn web_roster_edit_and_view_are_untouched() {
        let native = roster_for(commands::Platform::Native);
        let web = roster_for(commands::Platform::Web);
        for title in ["Edit", "View"] {
            assert_eq!(
                native.iter().find(|m| m.title == title).unwrap().items,
                web.iter().find(|m| m.title == title).unwrap().items,
                "{title} menu must be untouched on web"
            );
        }
    }

    /// The Window menu (Minimize/Zoom, both predefined OS chrome) is entirely
    /// PREDEFINED, so it vanishes on web — no OS window to minimize/zoom in a browser
    /// tab, and a menu left with zero items after filtering is dropped, not shown empty.
    #[test]
    fn web_roster_drops_the_whole_window_menu() {
        let menus = roster_for(commands::Platform::Web);
        assert!(menus.iter().all(|m| m.title != "Window"), "Window must vanish on web");
        // Exactly four menus survive: awl · File · Edit · View.
        let titles: Vec<&str> = menus.iter().map(|m| m.title).collect();
        assert_eq!(titles, vec!["awl", "File", "Edit", "View"]);
    }

    /// No separator in the web roster is ever LEADING, TRAILING, or DOUBLED — the
    /// dangling-separator trim's own law, checked over every surviving menu.
    #[test]
    fn web_roster_never_leaves_a_dangling_separator() {
        for menu in roster_for(commands::Platform::Web) {
            assert!(!matches!(menu.items.first(), Some(RosterItem::Separator)), "{}: leading separator", menu.title);
            assert!(!matches!(menu.items.last(), Some(RosterItem::Separator)), "{}: trailing separator", menu.title);
            assert!(
                !menu.items.windows(2).any(|w| matches!(w, [RosterItem::Separator, RosterItem::Separator])),
                "{}: doubled separator",
                menu.title
            );
        }
    }

    /// Every ROUTED item that survives web filtering still resolves to a real Action
    /// (the renderer-consumption law, narrowed to the filtered view) — filtering can
    /// drop a row, but never leave a dead one behind.
    #[test]
    fn web_roster_every_surviving_routed_item_resolves() {
        for menu in roster_for(commands::Platform::Web) {
            for item in &menu.items {
                if let RosterItem::Routed { id, .. } = item {
                    assert!(resolve(id).is_some(), "web roster item {id:?} resolves to no Action");
                }
            }
        }
    }
}
