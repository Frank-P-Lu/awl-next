//! Tests for the `overlay` module (pickers, navigators, capture/value-edit
//! sub-states, faceting, elision) -- split verbatim out of the former
//! `overlay.rs` monolith's embedded `mod tests` (2026-07 code-organization
//! pass); every test's NAME and MODULE PATH are unchanged
//! (`overlay::tests::foo`) -- only which file its source lives in moved.

use super::*;

fn corpus() -> Vec<String> {
    vec![
        ".env".to_string(),
        "README.md".to_string(),
        "src/lib.rs".to_string(),
        "src/main.rs".to_string(),
    ]
}

#[test]
fn empty_query_shows_all() {
    let ov = OverlayState::new(OverlayKind::Goto, corpus(), vec![], vec![]);
    assert_eq!(ov.items.len(), 4);
}

fn orphan(rel: &str, size: u64) -> crate::assets::Orphan {
    let (name, parent) = match rel.rsplit_once('/') {
        Some((d, n)) => (n.to_string(), d.to_string()),
        None => (rel.to_string(), String::new()),
    };
    crate::assets::Orphan { rel: rel.to_string(), name, parent, size: Some(size) }
}

#[test]
fn assets_picker_shows_leaf_names_and_size_parent_secondary() {
    let ov = OverlayState::new_assets(vec![
        orphan("assets/photo.png", 12_600),
        orphan("notes/assets/old.png", 5),
    ]);
    // PRIMARY cell is the leaf file name, not the full path.
    assert_eq!(ov.item_strings(), vec!["photo.png", "old.png"]);
    // SECONDARY cell (bindings column) is "size · parent dir".
    assert_eq!(
        ov.item_bindings(),
        vec!["12.3 KB · assets", "5 B · notes/assets"]
    );
    // The ACCEPT value stays the full root-relative path (the trash key).
    assert_eq!(ov.selected_value(), Some("assets/photo.png"));
    // Fuzzy still matches over the full path, so typing a folder narrows.
    let mut ov2 = OverlayState::new_assets(vec![
        orphan("assets/photo.png", 1),
        orphan("notes/assets/old.png", 1),
    ]);
    ov2.push('n');
    ov2.push('o');
    ov2.push('t');
    assert_eq!(ov2.selected_value(), Some("notes/assets/old.png"));
}

#[test]
fn assets_remove_asset_row_shrinks_the_list_and_keeps_the_picker_open() {
    let mut ov = OverlayState::new_assets(vec![
        orphan("assets/a.png", 1),
        orphan("assets/b.png", 2),
        orphan("assets/c.png", 3),
    ]);
    assert_eq!(ov.items.len(), 3);
    // Remove the MIDDLE row by value: the other two remain, in order.
    assert!(ov.remove_asset_row("assets/b.png"));
    assert_eq!(ov.item_strings(), vec!["a.png", "c.png"]);
    // The secondary column stays index-aligned (b's row is gone, not misaligned).
    assert_eq!(ov.item_bindings(), vec!["1 B · assets", "3 B · assets"]);
    // Removing a value not present is a calm no-op.
    assert!(!ov.remove_asset_row("assets/zzz.png"));
    assert_eq!(ov.items.len(), 2);
}

#[test]
fn assets_emptying_the_list_shows_the_calm_empty_state() {
    let mut ov = OverlayState::new_assets(vec![orphan("assets/only.png", 1)]);
    assert!(ov.empty_notice().is_none(), "one row → no empty state");
    assert!(ov.remove_asset_row("assets/only.png"));
    // Now empty: the picker stays valid and shows the calm per-kind message.
    assert_eq!(ov.items.len(), 0);
    assert_eq!(ov.empty_notice().as_deref(), Some("no unused assets"));
    // Enter on the empty state is a no-op (nothing selected).
    assert_eq!(ov.selected_value(), None);
}

#[test]
fn assets_empty_corpus_always_summons_with_the_calm_message() {
    let ov = OverlayState::new_assets(vec![]);
    assert_eq!(ov.kind, OverlayKind::Assets);
    assert_eq!(ov.empty_notice().as_deref(), Some("no unused assets"));
}

#[test]
fn typing_filters() {
    let mut ov = OverlayState::new(OverlayKind::Goto, corpus(), vec![], vec![]);
    ov.push('e');
    ov.push('n');
    ov.push('v');
    // ".env" should be the top match.
    assert_eq!(ov.selected_value(), Some(".env"));
}

#[test]
fn goto_hides_dotfiles_until_revealed() {
    // A go-to corpus with a hidden dotfile, a hidden dir entry, an `.env` (the
    // earned exception), and ordinary files.
    let corpus = vec![
        ".gitignore".to_string(),
        ".env".to_string(),
        "src/.hidden/x.rs".to_string(),
        "README.md".to_string(),
        "src/main.rs".to_string(),
    ];
    let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
    // Default: dotfiles hidden, `.env` and ordinary files visible.
    let shown = ov.item_strings();
    assert!(!shown.iter().any(|s| s == ".gitignore"), "dotfile hidden: {shown:?}");
    assert!(!shown.iter().any(|s| s == "src/.hidden/x.rs"), "nested dot dir hidden: {shown:?}");
    assert!(shown.iter().any(|s| s == ".env"), ".env stays visible: {shown:?}");
    assert!(shown.iter().any(|s| s == "README.md"));
    assert!(shown.iter().any(|s| s == "src/main.rs"));
    assert!(!ov.show_hidden);
    // Toggle -> dotfiles now revealed alongside everything.
    assert!(ov.toggle_hidden());
    assert!(ov.show_hidden);
    let shown = ov.item_strings();
    assert!(shown.iter().any(|s| s == ".gitignore"), "dotfile revealed: {shown:?}");
    assert!(shown.iter().any(|s| s == "src/.hidden/x.rs"), "nested dot dir revealed: {shown:?}");
    assert!(shown.iter().any(|s| s == ".env"));
    // Toggle back -> hidden again.
    assert!(ov.toggle_hidden());
    assert!(!ov.show_hidden);
    assert!(!ov.item_strings().iter().any(|s| s == ".gitignore"));
}

#[test]
fn browse_hides_dot_leaves_until_revealed() {
    // Browse lists one directory LEVEL: bare leaf names.
    let corpus = vec![
        ".config".to_string(),
        "notes.md".to_string(),
        ".env".to_string(),
    ];
    let git = vec![false; 3];
    let is_dir = vec![true, false, false];
    let mut ov = OverlayState::new_marked(
        OverlayKind::Browse,
        corpus,
        git,
        is_dir,
        vec![],
        vec![],
        None,
    );
    let shown = ov.item_strings();
    assert!(!shown.iter().any(|s| s.starts_with(".config")), "dot dir hidden: {shown:?}");
    assert!(shown.iter().any(|s| s == "notes.md"));
    assert!(shown.iter().any(|s| s == ".env"), ".env visible in browse too");
    assert!(ov.toggle_hidden());
    assert!(ov.item_strings().iter().any(|s| s.starts_with(".config")), "dot dir revealed");
}

#[test]
fn non_file_picker_ignores_hidden_toggle() {
    // A theme/command picker never hides dotfiles and the toggle is a no-op.
    let mut ov = OverlayState::new_command(
        vec!["Save".into(), ".secret command".into()],
        vec!["C-x C-s".into(), String::new()],
        vec![false, false],
    );
    assert!(!ov.kind.hides_dotfiles());
    let before = ov.item_strings();
    assert!(!ov.toggle_hidden(), "toggle is a no-op for a non-file picker");
    assert!(!ov.show_hidden);
    assert_eq!(ov.item_strings(), before, "listing unchanged");
}

#[test]
fn move_clamps() {
    let mut ov = OverlayState::new(OverlayKind::Goto, corpus(), vec![], vec![]);
    ov.move_sel(-1);
    assert_eq!(ov.selected, 0);
    ov.move_sel(100);
    assert_eq!(ov.selected, ov.items.len() - 1);
}

#[test]
fn switch_marks_git_children() {
    // Project picker: repo-alpha/repo-beta are git, plain-notes is not.
    let corpus = vec![
        "plain-notes".to_string(),
        "repo-alpha".to_string(),
        "repo-beta".to_string(),
    ];
    let git = vec![false, true, true];
    let is_dir = vec![true, true, true];
    let ov = OverlayState::new_marked(
        OverlayKind::Project,
        corpus,
        git,
        is_dir,
        vec![],
        vec![],
        None,
    );
    let items = ov.item_strings();
    // The NAME column carries no git marker any more — a clean folder name (+ `/`).
    assert!(items.iter().all(|s| !s.contains('•')), "no bullet in names: {items:?}");
    // Git repos carry a `"git"` SECONDARY-column tag, parallel to `items`; a plain
    // folder's slot is empty.
    let tags = ov.item_git_tags();
    assert_eq!(tags.len(), items.len(), "git tags parallel to rows");
    let tag_of = |name: &str| {
        let pos = items.iter().position(|s| s.contains(name)).unwrap();
        tags[pos].as_str()
    };
    assert_eq!(tag_of("repo-alpha"), "git");
    assert_eq!(tag_of("repo-beta"), "git");
    assert_eq!(tag_of("plain-notes"), "", "plain folder carries no git tag");
    // plain-notes is a plain folder: trailing slash, no marker.
    let pn = items.iter().find(|s| s.contains("plain-notes")).unwrap();
    assert!(pn.ends_with('/'));
    // The accept value is always the RAW name (no marker).
    assert_eq!(ov.corpus[ov.selected_corpus_index().unwrap()], "plain-notes");
}

#[test]
fn new_project_pins_accept_row_and_marks_git() {
    // Folders for the explorer level: a plain folder + two git repos.
    let folders = vec![
        ("plain-notes".to_string(), false),
        ("repo-alpha".to_string(), true),
        ("repo-beta".to_string(), true),
    ];
    let ov = OverlayState::new_project("/ws".to_string(), folders, &[]);
    assert_eq!(ov.kind.as_str(), "switch");
    // The synthetic "." accept-this-folder row is pinned at the TOP.
    let items = ov.item_strings();
    assert_eq!(items[0], ".");
    // browse_dir carries the ABSOLUTE dir for path navigation.
    assert_eq!(ov.browse_dir.as_deref(), Some("/ws"));
    // Default selection skips "." and lands on the first REAL folder so
    // Enter descends into it immediately (the "." select row is Up).
    assert_eq!(ov.selected_value(), Some("plain-notes"));
    assert!(ov.selected_is_dir(), "first folder is a directory");
    // Git children carry the `"git"` SECONDARY tag (not a name bullet); "." is
    // neither git nor a dir, and no name carries a bullet.
    assert!(items.iter().all(|s| !s.contains('•')), "no name bullet: {items:?}");
    let tags = ov.item_git_tags();
    let alpha = items.iter().position(|s| s.contains("repo-alpha")).unwrap();
    assert_eq!(tags[alpha], "git");
    assert_eq!(tags[0], "", "the '.' accept row is never git-tagged");
    assert!(!items[0].ends_with('/'));
}

#[test]
fn project_hides_dotfolders_but_keeps_accept_row_and_env() {
    // A workspace level with dotfolders (.git/.claude), an .env, and plain folders.
    let folders = vec![
        (".git".to_string(), false),
        (".claude".to_string(), false),
        (".env".to_string(), false),
        ("src".to_string(), false),
        ("repo".to_string(), true),
    ];
    let mut ov = OverlayState::new_project("/ws".to_string(), folders, &[]);
    // Project now HIDES dotfolders by default (the Batch dotfile filter extended to
    // it), while the synthetic "." accept-this-folder row and `.env` (the earned
    // exception) stay visible.
    assert!(ov.kind.hides_dotfiles(), "Project hides dotfiles now");
    let shown = ov.item_strings();
    assert!(shown.iter().any(|s| s == "."), "the '.' accept row survives: {shown:?}");
    assert!(!shown.iter().any(|s| s.starts_with(".git")), ".git hidden: {shown:?}");
    assert!(!shown.iter().any(|s| s.starts_with(".claude")), ".claude hidden: {shown:?}");
    assert!(shown.iter().any(|s| s.starts_with(".env")), ".env stays visible: {shown:?}");
    assert!(shown.iter().any(|s| s.starts_with("src")));
    assert!(shown.iter().any(|s| s.starts_with("repo")));
    // The `.env` folder is not git, so its secondary tag is empty; the repo carries
    // the "git" tag — and no dotfolder-tag leaks (they are filtered out entirely).
    let tags = ov.item_git_tags();
    let repo_i = shown.iter().position(|s| s.starts_with("repo")).unwrap();
    assert_eq!(tags[repo_i], "git");
    // Cmd-Shift-. reveals the dotfolders for Project too.
    assert!(ov.toggle_hidden(), "the reveal toggle flips for Project");
    let revealed = ov.item_strings();
    assert!(revealed.iter().any(|s| s.starts_with(".git")), "revealed: {revealed:?}");
    assert!(revealed.iter().any(|s| s.starts_with(".claude")), "revealed: {revealed:?}");
    assert!(revealed.iter().any(|s| s == "."), "'.' still present after reveal");
}

#[test]
fn project_picker_has_an_all_recent_strip_and_lands_on_all() {
    // The switch-project navigator now FACETS: All (the flat workspace-folder
    // listing, the home) · Recent (the recent-projects MRU). It LANDS on All.
    let folders = vec![("proj-a".to_string(), true), ("proj-b".to_string(), false)];
    let ov = OverlayState::new_project("/ws".to_string(), folders, &[]);
    assert!(ov.is_faceting(), "Project facets now");
    let strip: Vec<String> = ov.lens_strip().into_iter().map(|(l, _)| l).collect();
    assert_eq!(strip, vec!["All".to_string(), "Recent".to_string()]);
    // HOME LAW: All is FIRST and the picker lands on it (the flat list).
    assert_eq!(ov.active_facet_id(), Some("all"));
    assert_eq!(ov.lens_strip().first().map(|(_, a)| *a), Some(true), "All is active on open");
    // The synthetic "." select-this-folder row survives under All (flat home).
    assert!(ov.item_strings().iter().any(|s| s == "."), "'.' survives under All");
}

#[test]
fn project_recent_lens_shows_only_mru_projects_in_mru_order() {
    // The workspace level lists three folders; the recent-PROJECTS MRU (absolute
    // paths, most-recent first) names two of them, out of listing order.
    let folders = vec![
        ("proj-a".to_string(), false), // corpus 1 — in the MRU (2nd most recent)
        ("proj-b".to_string(), false), // corpus 2 — NOT in the MRU
        ("proj-c".to_string(), false), // corpus 3 — in the MRU (most recent)
    ];
    // MRU: proj-c is most recent, then proj-a. A stale root elsewhere opts out.
    let recent = vec![
        "/ws/proj-c".to_string(),
        "/ws/proj-a".to_string(),
        "/elsewhere/gone".to_string(),
    ];
    let mut ov = OverlayState::new_project("/ws".to_string(), folders, &recent);
    // Switch to the Recent lens (strip index 1).
    ov.set_facet_lens(1);
    assert_eq!(ov.active_facet_id(), Some("recent"));
    // ONLY the two MRU folders show, in MRU order (proj-c before proj-a) — the
    // "." row and the non-MRU proj-b opt out.
    // (Folder rows render with a trailing "/" in the display strings.)
    assert_eq!(ov.item_strings(), vec!["proj-c/".to_string(), "proj-a/".to_string()]);
    // Every surviving row sits under the single "Recent" section header.
    assert!(ov.item_sections().iter().all(|s| s == "Recent"));
}

#[test]
fn project_recent_lens_is_empty_on_a_fresh_session() {
    // Nothing switched-to yet → empty MRU → the Recent lens lists NOTHING (shows
    // its "no recent projects yet" empty state), never the whole workspace.
    let folders = vec![("proj-a".to_string(), false), ("proj-b".to_string(), false)];
    let mut ov = OverlayState::new_project("/ws".to_string(), folders, &[]);
    ov.set_facet_lens(1);
    assert_eq!(ov.active_facet_id(), Some("recent"));
    assert!(
        ov.item_strings().is_empty(),
        "Recent is empty with no recent projects: {:?}",
        ov.item_strings()
    );
    // The warm empty-lens wording invites rather than reports.
    assert_eq!(
        OverlayKind::Project.empty_lens_message("recent"),
        Some("no recent projects yet")
    );
}

/// A minimal [`BuildCtx`] with every field empty/None — the tests that only
/// care about ONE input fill just that one.
fn empty_build_ctx<'a>(config_keys: &'a [(String, Vec<String>)]) -> BuildCtx<'a> {
    BuildCtx {
        goto_corpus: Vec::new(),
        goto_open: Vec::new(),
        goto_recent: Vec::new(),
        goto_times: Vec::new(),
        config_keys,
        config_linux_keep: &[],
        goto_headings: Vec::new(),
        spell_target: None,
        history_entries: Vec::new(),
        history_now: None,
        history_session_start: None,
        settings_values: Default::default(),
        assets: Vec::new(),
        has_waiter: false,
    }
}

/// FINISH BUFFER GATING (item 13): the palette row list excludes "Finish file"
/// with no daemon `--wait` client waiting, and re-includes it — dispatching
/// correctly — the instant one is. Built through the REAL `overlay::build` seam
/// (the same one the live App and the headless replay both call), so this is the
/// purest reachable seam short of a live daemon round-trip (flagged for human
/// confirmation in the report — the daemon itself is structurally live-only).
#[test]
fn command_palette_hides_finish_buffer_without_a_waiter_and_shows_it_with_one() {
    // NO waiter (the default `BuildCtx`, matching headless capture / a fresh
    // live App with nothing waiting): "Finish file" is absent from what's
    // rankable/selectable...
    let ctx_idle = BuildCtx { has_waiter: false, ..empty_build_ctx(&[]) };
    let ov_idle = crate::overlay::build(OverlayKind::Command, &ctx_idle)
        .expect("the Command palette always summons");
    assert!(
        !ov_idle.item_strings().contains(&"Finish file".to_string()),
        "Finish file must be hidden from the palette with no active daemon waiter"
    );
    // ...but the underlying corpus itself is UNTOUCHED (only what's shown/
    // selectable shrinks) — the row-index math `commands::visible_action_of`
    // relies on for every OTHER command stays valid.
    assert!(
        ov_idle.corpus.contains(&"Finish file".to_string()),
        "hiding a row must not shrink the underlying corpus (index-stability)"
    );
    assert_eq!(
        ov_idle.corpus.len(),
        crate::commands::visible_names().len() + crate::settings::palette_names().len(),
        "corpus stays exactly commands::visible() + the settings union, unshrunk"
    );

    // A waiter IS active: the row reappears...
    let ctx_waiting = BuildCtx { has_waiter: true, ..empty_build_ctx(&[]) };
    let mut ov_waiting = crate::overlay::build(OverlayKind::Command, &ctx_waiting)
        .expect("the Command palette always summons");
    assert!(
        ov_waiting.item_strings().contains(&"Finish file".to_string()),
        "Finish file must show while a daemon waiter is active"
    );
    // ...and selecting it resolves through the SAME `commands::visible_action_of`
    // seam the real palette Enter/accept uses (`actions::overlay_nav`) — proving
    // DISPATCH stays unchanged: a shown Finish file row still runs the real
    // `Action::FinishBuffer`.
    ov_waiting.query = "Finish file".to_string();
    ov_waiting.refilter();
    let idx = ov_waiting
        .selected_corpus_index()
        .expect("the exact name must fuzzy-match its own row");
    assert_eq!(crate::commands::visible_action_of(idx), crate::keymap::Action::FinishBuffer);
}

#[test]
fn goto_headings_lens_folds_in_the_docs_headings() {
    // THE FOLD: a Go-to overlay with the doc's headings attached lists them mixed
    // with the files under the flat `All` home (item 11's unified default), and
    // ONLY the headings under the dedicated Headings lens — which is where the
    // retired standalone Outline picker now lives as an explicit refinement.
    let corpus = vec!["README.md".to_string(), "src/main.rs".to_string()];
    let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
    ov.attach_headings(vec![
        ("Introduction".to_string(), 3),
        ("  Details".to_string(), 7),
    ]);
    // Strip carries the Headings lens, parked last — "By type" was CUT (item 11).
    let strip: Vec<String> = ov.lens_strip().into_iter().map(|(l, _)| l).collect();
    assert_eq!(strip, vec!["All", "Recent", "This folder", "Headings"]);
    // ALL home: files AND headings, mixed in the same fuzzy-ranked list. A heading
    // row carries the `❡ ` KIND-HINT marker (item 11's rowlayout PRIMARY-cell
    // disambiguator); a file row never does.
    const H: &str = OverlayKind::HEADING_MARKER_PREFIX;
    assert_eq!(ov.active_facet_id(), Some("all"));
    let all = ov.item_strings();
    assert!(all.iter().any(|s| s == "README.md") && all.iter().any(|s| s == "src/main.rs"));
    assert!(
        all.iter().any(|s| s == &format!("{H}Introduction")),
        "headings mixed into All, marked: {all:?}"
    );
    assert!(
        all.iter().any(|s| s == &format!("{H}  Details")),
        "headings mixed into All, marked: {all:?}"
    );
    assert_eq!(all.len(), 4);
    // Headings lens (strip index 3): ONLY the headings, and each row IS a heading
    // whose accept is its line number, not a file open.
    ov.focus_facet_id("headings");
    assert_eq!(ov.active_facet_id(), Some("headings"));
    assert_eq!(
        ov.item_strings(),
        vec![format!("{H}Introduction"), format!("{H}  Details")]
    );
    assert!(ov.selected_is_heading(), "the Headings lens rows are headings");
    assert_eq!(ov.selected_line(), Some(3), "the first heading jumps to line 3");
    // "This folder" (strip index 2): a file-only REFINEMENT — headings drop out.
    ov.set_facet_lens(2);
    let folder = ov.item_strings();
    assert!(!folder.iter().any(|s| s == "Introduction" || s == "  Details"), "{folder:?}");
}

#[test]
fn goto_headings_lens_is_empty_without_headings() {
    // A non-markdown buffer (or one with no headings) attaches nothing: the
    // Headings lens is still on the strip but reads empty ("no headings yet").
    let corpus = vec!["a.rs".to_string(), "b.rs".to_string()];
    let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
    ov.attach_headings(Vec::new()); // no-op
    ov.focus_facet_id("headings");
    assert_eq!(ov.active_facet_id(), Some("headings"));
    assert!(ov.item_strings().is_empty(), "no headings → empty lens");
    assert_eq!(ov.empty_message(), "no headings yet");
}

#[test]
fn theme_picker_is_flat_and_lists_every_world_with_active_selected() {
    // The theme picker's runtime lens strip was RETIRED (2026-07-15): it is now a
    // FLAT browsable list of every world in THEMES order, no faceting, no sections.
    let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let gum = names.iter().position(|n| n == "Gumtree").unwrap();
    let mut ov = OverlayState::new_theme(names.clone(), gum);
    assert_eq!(ov.kind.as_str(), "theme");
    assert_eq!(ov.original_theme, Some(gum), "the opening theme is remembered for revert");
    // FLAT: no facet scheme, no lens strip, no section labels.
    assert!(!ov.is_faceting(), "the theme picker does not facet");
    assert!(ov.active_facet_id().is_none(), "no active lens");
    assert!(ov.lens_strip().is_empty(), "no lens strip");
    assert!(ov.item_sections().iter().all(|s| s.is_empty()), "no section grouping");
    // Every world is listed, in THEMES declaration order, and the active world opens
    // selected (so it is highlighted + previewable with no move).
    assert_eq!(ov.item_strings(), names, "flat list = every world in THEMES order");
    assert_eq!(ov.selected_value(), Some("Gumtree"), "active world opens selected");
    // No git / dir markers on the theme rows.
    assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
    // cycle_lens is inert on a non-faceting picker (it grew no strip to cycle).
    ov.cycle_lens(1);
    assert_eq!(ov.facet_lens, 0);
    assert!(ov.active_facet_id().is_none());
    assert_eq!(ov.item_strings(), names, "cycle_lens did not regroup the flat list");
}

/// The CLICKABLE lens strip's pointing counterpart to a no-op LEFT/RIGHT at an
/// end: clicking the ALREADY-ACTIVE facet is a calm no-op (documented on
/// `set_facet_lens` itself) — `facet_lens`, the selected item, and the scroll
/// position all stay byte-identical, unlike a real switch (which regroups the
/// list and can move `selected`/`scroll`).
#[test]
fn clicking_the_current_facet_is_a_calm_no_op() {
    // Driven over a still-faceting picker (the Command palette) — the theme picker
    // retired its lens strip, so this generic law now rides a surviving faceter.
    let names = crate::commands::names();
    let hidden = vec![false; names.len()];
    let mut ov =
        OverlayState::new_command(names, crate::commands::effective_bindings(&[], &[]), hidden);
    ov.set_facet_lens(2); // switch to the Edit lens once, a real change
    assert_eq!(ov.active_facet_id(), Some("edit"));
    let (before_lens, before_selected, before_scroll, before_items) =
        (ov.facet_lens, ov.selected, ov.scroll, ov.item_strings());
    ov.set_facet_lens(2); // click the SAME facet again — a calm no-op
    assert_eq!(ov.facet_lens, before_lens);
    assert_eq!(ov.selected, before_selected);
    assert_eq!(ov.scroll, before_scroll);
    assert_eq!(ov.item_strings(), before_items);
}

#[test]
fn flat_pickers_have_no_lens_strip() {
    // A non-faceting picker never grows a lens strip or section labels, and has no
    // facet scheme (so `active_facet_id` is None). Both the Caret picker and — since
    // 2026-07-15 — the THEME picker are flat, non-faceting examples (Goto / Browse /
    // Project / Command / History / Settings still facet — see `facets::scheme`).
    let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let theme = OverlayState::new_theme(names, 0);
    for mut ov in [OverlayState::new(OverlayKind::Caret, corpus(), vec![], vec![]), theme] {
        assert!(!ov.is_faceting(), "{:?} must not facet", ov.kind);
        assert!(ov.lens_strip().is_empty(), "{:?} has no lens strip", ov.kind);
        assert!(ov.active_facet_id().is_none(), "{:?} has no active lens", ov.kind);
        assert!(ov.item_sections().iter().all(|s| s.is_empty()), "{:?} no sections", ov.kind);
        // cycle_lens on a non-faceting picker is inert (facet_lens stays 0).
        ov.cycle_lens(1);
        assert_eq!(ov.facet_lens, 0, "{:?} cycle_lens is inert", ov.kind);
    }
}

#[test]
fn caret_picker_lists_three_styles_navigates_and_maps_modes() {
    use crate::caret::CaretMode;
    // `new_caret` reads `crate::caret::is_auto()` at construction (for
    // `original_caret_was_auto`), so hold the caret global's lock and pin an
    // explicit override for the whole test — otherwise this races whatever
    // override another parallel test leaves behind.
    let _g = crate::testlock::serial();
    // SUMMON with Block active: the corpus is the three look labels in ALL order,
    // each row's "binding" column carrying its description.
    crate::caret::set_mode(CaretMode::Block);
    let ov = OverlayState::new_caret(CaretMode::Block);
    assert_eq!(ov.kind.as_str(), "caret");
    assert_eq!(ov.item_strings(), vec!["Block", "Morph", "I-beam"]);
    assert_eq!(
        ov.item_bindings(),
        vec![
            "rounded square + trailing underline",
            "takes the glyph silhouette",
            "an alive insertion bar",
        ]
    );
    // Opens highlighting the ACTIVE look, and `original_caret` remembers it.
    assert_eq!(ov.selected_value(), Some("Block"));
    assert_eq!(ov.selected_caret_mode(), Some(CaretMode::Block));
    assert_eq!(ov.original_caret, Some(CaretMode::Block));
    // An explicit override was active at open — not auto.
    assert!(!ov.original_caret_was_auto);
    // NAVIGATE down the list -> the selected look maps back via from_label.
    let mut ov = ov;
    ov.move_sel(1);
    assert_eq!(ov.selected_caret_mode(), Some(CaretMode::Morph));
    ov.move_sel(1);
    assert_eq!(ov.selected_caret_mode(), Some(CaretMode::Ibeam));
    // Opening with a non-Block look pre-selects THAT row.
    crate::caret::set_mode(CaretMode::Ibeam);
    let ov2 = OverlayState::new_caret(CaretMode::Ibeam);
    assert_eq!(ov2.selected_value(), Some("I-beam"));
    assert_eq!(ov2.original_caret, Some(CaretMode::Ibeam));
    assert!(!ov2.original_caret_was_auto);
    // The hint leads with the universal jump cluster (move + type-to-filter) then
    // names ↵'s action; flat picker (no descend).
    assert_eq!(
        OverlayKind::Caret.hint(),
        "type to filter   \u{21B5} apply"
    );
    // selected_caret_mode is None for a non-caret picker.
    let theme = OverlayState::new_theme(vec!["Tawny".into()], 0);
    assert_eq!(theme.selected_caret_mode(), None);

    // Restore.
    crate::caret::clear_override();
}

/// `original_caret_was_auto`: the field the Caret-style picker's auto-aware
/// Cancel relies on (see `actions::overlay_nav`'s Cancel arm). It reads the
/// LIVE `crate::caret::is_auto()` global at construction, independent of
/// whatever concrete `active` mode is passed in (the two real call sites keep
/// the two in step by always passing `crate::caret::mode()`).
#[test]
fn caret_picker_captures_whether_it_opened_while_auto() {
    use crate::caret::CaretMode;
    let _g = crate::testlock::serial();
    let _t = crate::testlock::serial();

    // AUTO: no override set — a mono world resolves Block.
    crate::caret::clear_override();
    crate::theme::set_active_by_name("Tawny").unwrap();
    assert_eq!(crate::caret::mode(), CaretMode::Block);
    let ov = OverlayState::new_caret(crate::caret::mode());
    assert_eq!(ov.original_caret, Some(CaretMode::Block), "records the RESOLVED look");
    assert!(ov.original_caret_was_auto, "but flags it as auto's resolution, not a pin");

    // EXPLICIT: an actual pin, even one that resolves to the exact same
    // concrete mode, is NOT auto.
    crate::caret::set_mode(CaretMode::Block);
    let ov2 = OverlayState::new_caret(crate::caret::mode());
    assert_eq!(ov2.original_caret, Some(CaretMode::Block));
    assert!(!ov2.original_caret_was_auto, "an explicit pin is never reported as auto");

    // Restore.
    crate::caret::clear_override();
    crate::theme::set_active(crate::theme::DEFAULT_THEME);
}

#[test]
fn command_palette_lists_names_with_parallel_bindings() {
    let names = vec![
        "Go to file".to_string(),
        "Switch theme".to_string(),
        "Save".to_string(),
    ];
    let binds = vec!["C-x C-f".to_string(), "C-x t".to_string(), "C-x C-s".to_string()];
    let mut ov = OverlayState::new_command(names.clone(), binds.clone(), vec![false; names.len()]);
    assert_eq!(ov.kind.as_str(), "command");
    // Empty query: rows are the names in order, bindings stay parallel.
    assert_eq!(ov.item_strings(), names);
    assert_eq!(ov.item_bindings(), binds);
    // Fuzzy filter narrows to "Switch theme" and keeps its binding aligned.
    ov.push('t');
    ov.push('h');
    ov.push('e');
    assert_eq!(ov.selected_value(), Some("Switch theme"));
    assert_eq!(ov.item_bindings().first().map(|s| s.as_str()), Some("C-x t"));
}

#[test]
fn goto_headings_lens_fuzzy_filters_and_jumps_by_line() {
    // The retired Outline picker's fuzzy-jump behavior, now under Go-to's Headings
    // lens: filter to a heading, its accept is the LINE (titles can repeat), not
    // the file-open the other lenses do.
    let corpus = vec!["notes.md".to_string()];
    let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
    ov.attach_headings(vec![
        ("Intro".to_string(), 0usize),
        ("  Setup".to_string(), 4usize),
        ("  Usage".to_string(), 9usize),
    ]);
    ov.focus_facet_id("headings");
    // Rows are the (indented) titles in order, marker-prefixed; lines stay parallel.
    const H: &str = OverlayKind::HEADING_MARKER_PREFIX;
    assert_eq!(
        ov.item_strings(),
        vec![format!("{H}Intro"), format!("{H}  Setup"), format!("{H}  Usage")]
    );
    assert_eq!(ov.selected_line(), Some(0));
    // Fuzzy filter to "Usage" -> selected row jumps to its line (9), not its text.
    // `selected_value` reads the RAW corpus (unprefixed) — the marker is display-only.
    ov.push('u');
    ov.push('s');
    ov.push('a');
    assert_eq!(ov.selected_value(), Some("  Usage"));
    assert!(ov.selected_is_heading());
    assert_eq!(ov.selected_line(), Some(9));
    // No git / dir markers on heading rows; the indentation + kind-hint survive.
    assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
    assert!(ov.item_strings().iter().all(|s| s.starts_with(H)));
}

#[test]
fn spell_picker_lists_suggestions_and_carries_target() {
    // Three corrections for a word flagged at line 2, cols 6..13.
    let sugg = vec!["receive".to_string(), "relieve".to_string(), "reprieve".to_string()];
    let ov = OverlayState::new_spell(sugg.clone(), (2, 6, 13), "recieve".to_string());
    assert_eq!(ov.kind.as_str(), "spell");
    // Rows are the suggestions in order (best first), then ONE appended "Add
    // '<word>' to dictionary" row; the top suggestion is selected.
    let rows = ov.item_strings();
    assert_eq!(&rows[..sugg.len()], &sugg[..], "the suggestions lead, in order");
    assert_eq!(
        rows.last().map(String::as_str),
        Some(super::state::add_to_dictionary_label("recieve").as_str()),
        "the LAST row is the Add-to-dictionary affordance"
    );
    assert_eq!(ov.selected_value(), Some("receive"));
    // The target span is carried so the accept can replace the word.
    assert_eq!(ov.spell_target, Some((2, 6, 13)));
    // No git / dir markers on the suggestion rows.
    assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
    // The add row is flagged (only it) and carries the word for the accept effect.
    assert!(!ov.selected_is_add_to_dictionary(), "a suggestion row is not the add row");
    assert_eq!(ov.add_word.as_deref(), Some("recieve"));
    let last = ov.items.len() - 1;
    assert!(ov.spell_add[ov.items[last]], "the last corpus row is the add row");
    // The hint names the ↵ action (replace) after the universal jump lead, flat
    // picker (no descend).
    assert_eq!(
        OverlayKind::Spell.hint(),
        "type to filter   \u{21B5} replace"
    );
}

/// Three history rows newest-first, exercising both WHICH shapes (a git
/// subject, an edited-heading description) and an empty which.
fn history_rows() -> Vec<crate::history::TimelineRow> {
    let row = |when: &str, which: &str, counts: &str, id: &str| crate::history::TimelineRow {
        when: when.to_string(),
        which: which.to_string(),
        counts: counts.to_string(),
        id: id.to_string(),
        timestamp: id.parse().unwrap_or(0),
        pinned: false,
        name: None,
    };
    vec![
        row("just now", "fix: the engine", "+0 −0", "300"),
        row("2 min ago", "edited \"Two flows\"", "+0 −1", "200"),
        row("1 hr ago", "", "+1 −2", "100"),
    ]
}

#[test]
fn history_picker_lists_versions_navigates_and_carries_ids() {
    let mut ov = OverlayState::new_history(history_rows(), None, None);
    assert_eq!(ov.kind.as_str(), "history");
    // The top (newest) row is selected; its restore id is the accept value.
    assert_eq!(ov.selected_history_id(), Some("300"));
    // NAVIGATE down -> the selected id tracks the highlighted version.
    ov.move_sel(1);
    assert_eq!(ov.selected_history_id(), Some("200"));
    ov.move_sel(1);
    assert_eq!(ov.selected_history_id(), Some("100"));
    // No git / dir markers on the version rows.
    assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
    // The hint teaches restore + diff + lens + close (informational, button-free) —
    // DIFF-AS-PREVIEW: Tab shifts focus into the diff panel, so the cell reads "diff".
    assert_eq!(
        OverlayKind::History.hint(),
        "type to filter   \u{21B5} restore   tab diff   \u{2190}/\u{2192} lens"
    );
    assert!(ov.foot_hint().contains("restore"));
}

#[test]
fn command_picker_lands_on_all_then_groups_by_menu_section_and_recent() {
    let names = crate::commands::names();
    let binds = crate::commands::effective_bindings(&[], &[]);
    let hidden = vec![false; names.len()];
    let mut ov = OverlayState::new_command(names, binds, hidden);
    // Lands on the flat All home; the strip is All-first.
    assert_eq!(ov.active_facet_id(), Some("all"), "opens on the flat All landing");
    assert_eq!(ov.lens_strip().first().map(|(l, _)| l.clone()), Some("All".to_string()));
    assert!(ov.item_sections().iter().all(|s| s.is_empty()), "All never groups");
    // → File lens: every shown row is a File-section command, headed "File".
    ov.cycle_lens(1);
    assert_eq!(ov.active_facet_id(), Some("file"));
    assert!(!ov.items.is_empty(), "File section is non-empty");
    for (row, &ci) in ov.items.iter().enumerate() {
        assert_eq!(ov.item_sections()[row], "File");
        assert_eq!(crate::commands::menu_section(&ov.corpus[ci]), Some("File"));
    }
    assert!(ov.item_strings().iter().any(|s| s == "Save"), "Save is a File command");
    // The Recent lens (strip index 4) reads the recency vec: seed one, see it group.
    let undo = ov.corpus.iter().position(|c| c == "Undo").unwrap();
    ov.recent = vec![undo];
    ov.set_facet_lens(4);
    assert_eq!(ov.active_facet_id(), Some("recent"));
    assert_eq!(ov.item_strings(), vec!["Undo".to_string()], "only the recent command");
    assert!(ov.item_sections().iter().all(|s| s == "Recent"));
}

#[test]
fn history_picker_groups_by_session_and_today_with_injected_now() {
    const DAY: u64 = 86_400_000;
    let now = 100 * DAY + 5_000;
    let session_start = 100 * DAY + 3_000; // this session began mid-day 100
    let row = |id: &str, ts: u64| crate::history::TimelineRow {
        when: "x".to_string(),
        which: String::new(),
        counts: "+0 −0".to_string(),
        id: id.to_string(),
        timestamp: ts,
        pinned: false,
        name: None,
    };
    let rows = vec![
        row("a", 100 * DAY + 4_000), // today AND in this session
        row("b", 100 * DAY + 1_000), // today, but before this session started
        row("c", 99 * DAY + 1_000),  // yesterday
    ];
    let mut ov = OverlayState::new_history(rows, Some(now), Some(session_start));
    // Lands on All (every version); strip is All-first.
    assert_eq!(ov.active_facet_id(), Some("all"));
    assert_eq!(ov.items.len(), 3);
    // → Session lens: only "a" (at/after session start).
    ov.cycle_lens(1);
    assert_eq!(ov.active_facet_id(), Some("session"));
    let session_ids: Vec<String> =
        ov.items.iter().map(|&ci| ov.history_ids[ci].clone()).collect();
    assert_eq!(session_ids, vec!["a".to_string()]);
    assert!(ov.item_sections().iter().all(|s| s == "Session"));
    // → Today lens: "a" and "b" (same calendar day), never yesterday's "c".
    ov.cycle_lens(1);
    assert_eq!(ov.active_facet_id(), Some("today"));
    let today_ids: Vec<String> =
        ov.items.iter().map(|&ci| ov.history_ids[ci].clone()).collect();
    assert_eq!(today_ids, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn history_time_lenses_are_inert_headless_no_clock() {
    // With no reference clock (the headless capture path), Session/Today group
    // NOTHING — the determinism gate — so those lenses show an empty list.
    let mut ov = OverlayState::new_history(history_rows(), None, None);
    ov.cycle_lens(1); // Session
    assert_eq!(ov.active_facet_id(), Some("session"));
    assert!(ov.items.is_empty(), "Session inert with no clock");
    ov.cycle_lens(1); // Today
    assert_eq!(ov.active_facet_id(), Some("today"));
    assert!(ov.items.is_empty(), "Today inert with no clock");
}

#[test]
fn history_rows_show_when_dot_which_and_counts_ride_the_faint_column() {
    // The MAIN column composes "when · which" (the bare when for an empty
    // which); the faint right column carries the "+N −M" changed-counts —
    // the existing binding-column pattern, zero new layout.
    let ov = OverlayState::new_history(history_rows(), None, None);
    assert_eq!(
        ov.item_strings(),
        vec![
            "just now · fix: the engine",
            "2 min ago · edited \"Two flows\"",
            "1 hr ago",
        ]
    );
    assert_eq!(ov.item_bindings(), vec!["+0 −0", "+0 −1", "+1 −2"]);
    // The composed corpus is what the fuzzy filter matches, so a SUBJECT
    // query finds its version (a free win of the composition).
    let mut ov = OverlayState::new_history(history_rows(), None, None);
    for c in "engine".chars() {
        ov.push(c);
    }
    assert_eq!(ov.item_strings().len(), 1);
    assert_eq!(ov.selected_history_id(), Some("300"));
}

#[test]
fn history_picker_marks_a_pinned_version_in_the_secondary_column() {
    // THE CONSCIOUS MARK: a KEPT (pinned) version wears the calm "pinned" tag
    // AHEAD of its changed-count in the faint secondary column (`item_bindings`
    // — the exact source the sidecar's `overlay.bindings` folds from), while an
    // un-pinned version stays bare. The count is never dropped for the tag.
    let mk = |id: &str, pinned: bool| crate::history::TimelineRow {
        when: "just now".to_string(),
        which: String::new(),
        counts: "+0 −1".to_string(),
        id: id.to_string(),
        timestamp: id.parse().unwrap_or(0),
        pinned,
        name: None,
    };
    let ov = OverlayState::new_history(vec![mk("2", true), mk("1", false)], None, None);
    let binds = ov.item_bindings();
    assert!(binds[0].contains(PIN_TAG), "the pinned row is marked: {:?}", binds[0]);
    assert!(binds[0].contains("+0 −1"), "and keeps its changed-count: {:?}", binds[0]);
    assert!(!binds[1].contains(PIN_TAG), "an un-pinned row stays bare: {:?}", binds[1]);
}

#[test]
fn history_picker_named_row_shows_name_primary_and_demotes_the_timestamp() {
    // NAMED SAVE POINTS: a named row's PRIMARY cell is the NAME itself (the
    // fuzzy corpus too — typing the name finds it), with the timestamp DEMOTED
    // beside the changed-count in the faint secondary column ("when · +N −M").
    // The redundant "pinned" tag is dropped for a named row (the name IS the
    // conscious mark); an unnamed sibling — pinned or not — keeps the exact
    // pre-name shape. Same corpus/bindings columns, no new layout path.
    let mk = |id: &str, pinned: bool, name: Option<&str>| crate::history::TimelineRow {
        when: "2 hr ago".to_string(),
        which: "edited \"Title\"".to_string(),
        counts: "+3 −1".to_string(),
        id: id.to_string(),
        timestamp: id.parse().unwrap_or(0),
        pinned,
        name: name.map(str::to_string),
    };
    let ov = OverlayState::new_history(
        vec![mk("3", true, Some("draft A")), mk("2", true, None), mk("1", false, None)],
        None,
        None,
    );
    // Primary cells: name for the named row, "when · which" for the rest.
    assert_eq!(ov.corpus[0], "draft A", "the name IS the primary cell");
    assert_eq!(ov.corpus[1], "2 hr ago · edited \"Title\"", "unnamed rows unchanged");
    // Secondary cells: timestamp demoted for the named row; pin tag only on the
    // unnamed pinned row.
    let binds = ov.item_bindings();
    assert_eq!(binds[0], "2 hr ago · +3 −1", "timestamp + count demoted to secondary");
    assert!(!binds[0].contains(PIN_TAG), "no redundant pin tag on a named row");
    assert_eq!(binds[1], format!("{PIN_TAG} · +3 −1"), "unnamed pinned row keeps its tag");
    assert_eq!(binds[2], "+3 −1", "plain row untouched");
    // The restore ids stay parallel — Enter/Tab on a named row reach id "3".
    assert_eq!(ov.history_ids, vec!["3", "2", "1"]);
    // Typing the NAME finds the named row (it rides the fuzzy corpus).
    let mut ov2 = ov.clone();
    for c in "draft".chars() {
        ov2.push(c);
    }
    assert_eq!(ov2.item_strings().len(), 1, "the name is fuzzy-findable");
    assert_eq!(ov2.selected_history_id(), Some("3"));
}

#[test]
fn history_picker_empty_state_shows_calm_row_and_no_op_accept() {
    // No versions -> an empty-corpus picker that summons but lists nothing; the
    // SHARED empty-state owner supplies the calm "no history yet" message row,
    // and every accept path already no-ops on an empty item list.
    let ov = OverlayState::new_history(Vec::new(), None, None);
    assert_eq!(ov.kind.as_str(), "history");
    assert!(ov.item_strings().is_empty(), "empty corpus lists no real rows");
    assert_eq!(
        ov.empty_notice().as_deref(),
        Some("no history yet"),
        "the shared empty-state supplies History's calm message"
    );
    assert_eq!(ov.selected_history_id(), None, "nothing to restore on empty");
}

#[test]
fn keybindings_capture_key_mode_finishes_instantly() {
    // SUMMON: the rebind menu lists the catalog with its effective chords.
    let names = crate::commands::visible_names();
    let binds = crate::commands::visible_effective_bindings(&[], &[]);
    let mut ov = OverlayState::new_keybindings(names.clone(), binds);
    assert_eq!(ov.kind.as_str(), "keybindings");
    assert_eq!(ov.item_strings(), names);
    assert!(ov.capture.is_none());
    // NAVIGATE: filter to "Undo" so the selected command is deterministic.
    for c in "undo".chars() {
        ov.push(c);
    }
    assert_eq!(ov.selected_value(), Some("Undo"));
    // ENTER → ChooseMode; default selection is KEY.
    ov.start_capture();
    let cap = ov.capture.as_ref().unwrap();
    assert_eq!(cap.stage, CaptureStage::ChooseMode);
    assert_eq!(cap.cmd_name, "Undo");
    assert!(!cap.chord_mode);
    // Choose KEY, begin recording, then ONE combo finishes instantly.
    ov.capture_move_mode(-1); // KEY row
    ov.capture_begin_recording();
    assert_eq!(ov.capture.as_ref().unwrap().stage, CaptureStage::Recording);
    let done = ov.capture_record("C-j".to_string());
    assert!(done, "KEY mode finishes on the first combo");
    assert_eq!(ov.capture_target(), Some(("undo".to_string(), "C-j".to_string())));
}

#[test]
fn keybindings_capture_chord_mode_collects_then_finishes() {
    let mut ov = OverlayState::new_keybindings(
        crate::commands::visible_names(),
        crate::commands::visible_effective_bindings(&[], &[]),
    );
    for c in "save".chars() {
        ov.push(c);
    }
    assert_eq!(ov.selected_value(), Some("Save"));
    ov.start_capture();
    ov.capture_move_mode(1); // CHORD row
    ov.capture_begin_recording();
    assert!(ov.capture.as_ref().unwrap().chord_mode);
    // First combo does NOT finish a chord; the 2-deep cap does.
    assert!(!ov.capture_record("C-x".to_string()));
    assert!(ov.capture_record("C-s".to_string()));
    // A THIRD combo is dropped (capped at 2).
    assert!(ov.capture_record("C-q".to_string()));
    assert_eq!(
        ov.capture_target(),
        Some(("save".to_string(), "C-x C-s".to_string()))
    );
}

#[test]
fn keybindings_confirm_and_reset_helpers() {
    let mut ov = OverlayState::new_keybindings(
        crate::commands::visible_names(),
        crate::commands::visible_effective_bindings(&[], &[]),
    );
    // RESET targets the highlighted command's slug.
    for c in "redo".chars() {
        ov.push(c);
    }
    assert_eq!(ov.selected_command_slug().as_deref(), Some("redo"));
    // CONFLICT: a finished capture can be pushed into the Confirm phase, which the
    // prompt reflects (naming the clashing command). Esc-equivalent aborts it.
    ov.start_capture();
    ov.capture_begin_recording();
    ov.capture_record("C-s".to_string());
    ov.capture_into_confirm("Search forward".to_string());
    let cap = ov.capture.as_ref().unwrap();
    assert_eq!(cap.stage, CaptureStage::Confirm);
    assert!(cap.prompt().contains("Search forward"));
    ov.capture_abort();
    assert!(ov.capture.is_none());
}

#[test]
fn hint_teaches_descend_only_for_navigable_kinds() {
    // The NON-faceting navigable explorers (Project / MoveDest) teach the
    // select-vs-descend asymmetry — but now with the UNIFIED glyph vocabulary:
    // ↵ selects/accepts FIRST (primary), then → descends, ← ascends (the old
    // ASCII `->/C-f` / `<-/C-b` word-chords are gone). Browse is a FACETED
    // explorer, so its ←/→ teach the LENS, not descend (descend rides Enter).
    for k in [OverlayKind::Project, OverlayKind::MoveDest] {
        let h = k.hint();
        // Unicode arrows, never the old ASCII/word-chord forms.
        assert!(h.contains('\u{2192}'), "{k:?} hint should teach → descend: {h}");
        assert!(h.contains('\u{2190}'), "{k:?} hint should teach ← ascend: {h}");
        assert!(!h.contains("C-f") && !h.contains("->"), "{k:?} no ASCII chord: {h}");
        // The universal type-to-filter cell LEADS the line, then the primary ↵ Return action.
        assert!(h.starts_with("type to filter"), "{k:?} hint leads with type to filter: {h}");
        assert!(h.contains("\u{21B5}"), "{k:?} hint names ↵ Return: {h}");
    }
    // Project ↵ SELECTS; MoveDest ↵ MOVES.
    assert!(OverlayKind::Project.hint().contains("\u{21B5} select"));
    assert!(OverlayKind::MoveDest.hint().contains("move here"));
    // The FACETED pickers (Goto / Browse / Command / History) teach ←/→ lens, not
    // ->/C-f descend, and each starts with the ↵ Return glyph. (The THEME picker
    // retired its lens strip 2026-07-15 — it is checked below as a FLAT picker.)
    for k in [
        OverlayKind::Goto,
        OverlayKind::Browse,
        OverlayKind::Command,
        OverlayKind::History,
    ] {
        let h = k.hint();
        assert!(!h.contains("C-f"), "{k:?} facets, no descend hint: {h}");
        assert!(h.contains("\u{2190}/\u{2192} lens"), "{k:?} hint should teach ←/→ lens: {h}");
        assert!(h.starts_with("type to filter"), "{k:?} hint leads with type to filter: {h}");
    }
    // The FLAT theme picker teaches ↵ keep + esc revert, and NO lens axis (its strip
    // was retired) — type to filter still leads.
    let th = OverlayKind::Theme.hint();
    assert!(th.starts_with("type to filter"), "theme hint leads with type to filter: {th}");
    assert!(th.contains("\u{21B5} keep"), "theme ↵ keeps: {th}");
    assert!(th.contains("esc") && th.contains("revert"), "theme esc reverts: {th}");
    assert!(!th.contains("lens"), "the flat theme picker teaches no lens: {th}");
    // Browse ↵ still OPENS (a folder descends / a file opens) and ⌫ ascends.
    assert!(OverlayKind::Browse.hint().contains("\u{21B5} open"));
    assert!(OverlayKind::Browse.hint().contains("\u{232B} up"));
}

/// The SHARED hint formatter produces ONE consistent shape for every picker:
/// `glyph SPACE label`, actions joined by the single `HINT_SEP`, the universal
/// `type to filter` FIRST, then the primary (↵), and cancel (esc) — where present —
/// LAST and lowercase. This is the pass-2 unification law: a sample of overlays
/// must all read identically formed.
#[test]
fn hint_formatter_is_consistent_across_pickers() {
    // The formatter itself: `glyph label`, HINT_SEP-joined, in order.
    let sample = [
        HintAction { glyph: "\u{21B5}", label: "keep" },
        HintAction { glyph: "\u{2190}/\u{2192}", label: "lens" },
        HintAction { glyph: "esc", label: "revert" },
    ];
    assert_eq!(
        format_hint(&sample),
        format!("\u{21B5} keep{HINT_SEP}\u{2190}/\u{2192} lens{HINT_SEP}esc revert")
    );
    assert_eq!(HINT_SEP, "   ", "the one canonical separator is a triple space");

    // Every kind's rendered hint obeys the shape: each action is `glyph SPACE
    // label` (exactly one space), the separator is HINT_SEP, the universal JUMP
    // lead (move → type-to-filter) comes first, the ↵ primary follows it, and any
    // cancel action is the lowercase `esc` (never `Esc`) LAST.
    for k in OverlayKind::ALL {
        let actions = k.hint_actions();
        assert!(actions.len() >= 2, "{k:?} must teach the filter lead + ↵ primary");
        // The universal jump-affordance lead: type to filter —
        // the discoverability fix for "you can only go one by one".
        assert_eq!(actions[0].glyph, "type", "{k:?} leads with type to filter");
        assert_eq!(actions[0].label, "to filter", "{k:?} lead cell reads type to filter");
        assert_eq!(actions[1].glyph, "\u{21B5}", "{k:?} ↵ primary follows the jump lead");
        // Cancel-last + lowercase esc: no action names capital `Esc`; if any
        // action is the esc cancel, it is the LAST one.
        for (i, a) in actions.iter().enumerate() {
            assert_ne!(a.glyph, "Esc", "{k:?} esc must be lowercase");
            if a.glyph == "esc" {
                assert_eq!(i, actions.len() - 1, "{k:?} esc cancel sits last");
            }
        }
        // The rendered line == the formatter over the same actions (one owner).
        let h = k.hint();
        assert_eq!(h, format_hint(&actions), "{k:?} hint routes through format_hint");
        // Separator discipline: the ONLY multi-space runs are the HINT_SEP joins,
        // so splitting on HINT_SEP yields exactly `actions.len()` `glyph label` cells.
        let cells: Vec<&str> = h.split(HINT_SEP).collect();
        assert_eq!(cells.len(), actions.len(), "{k:?} cells == actions: {h}");
        for cell in cells {
            assert!(!cell.contains("  "), "{k:?} no stray double space in {cell:?}");
        }
    }
}

/// The SHARED empty-state owner: a picker with NO matching rows reports a calm
/// message — the universal "no matches" when a QUERY filtered a non-empty corpus
/// out, the per-kind [`OverlayKind::empty_corpus_message`] when the CORPUS itself
/// is empty — and Enter on it is already a no-op (nothing selected). This is the
/// pass-3 unification: every picker shares one empty-state, not a blank card.
#[test]
fn empty_state_message_is_shared_and_accept_is_a_no_op() {
    // A non-empty corpus filtered to nothing by a query → the universal message.
    let mut ov = OverlayState::new(
        OverlayKind::Goto,
        vec!["alpha.md".into(), "beta.md".into()],
        vec![],
        vec![],
    );
    ov.push('z'); // matches neither → items empty
    assert!(ov.items.is_empty());
    assert_eq!(ov.empty_notice().as_deref(), Some("no matches"));
    // Enter accept is a no-op: nothing is selected on an empty list.
    assert_eq!(ov.selected_value(), None, "empty list selects nothing");

    // A non-empty list reports NO empty-state (it has rows to show).
    let ov2 = OverlayState::new(OverlayKind::Goto, vec!["alpha.md".into()], vec![], vec![]);
    assert_eq!(ov2.empty_notice(), None, "a picker with rows has no empty-state");

    // An EMPTY corpus reads the per-kind message (query still empty).
    let empty_goto = OverlayState::new(OverlayKind::Goto, vec![], vec![], vec![]);
    assert_eq!(empty_goto.empty_notice().as_deref(), Some("no files here"));
    let empty_hist = OverlayState::new_history(Vec::new(), None, None);
    assert_eq!(empty_hist.empty_notice().as_deref(), Some("no history yet"));
    // Every kind's empty-corpus message is a non-empty calm line (never blank).
    for k in OverlayKind::ALL {
        assert!(!k.empty_corpus_message().is_empty(), "{k:?} needs an empty line");
    }
}

/// BREADCRUMB LAW: every overlay kind DECLARES a pop-vs-close-all accept class
/// (the no-wildcard match in [`OverlayKind::accept_disposition`] is the real
/// compile-time guard — a future kind won't build until it declares one; this
/// sweep pins the specific classifications so a silent reclassification trips a
/// test). The rule the whole round turns on: Esc/cancel always POPS (uniform, not
/// per-kind); an ACCEPT is Navigate (close the whole stack — you land in the
/// result), ValuePick (pop back to the summoning overlay — you committed a
/// setting), or StayOpen (never closes).
#[test]
fn every_kind_declares_an_accept_disposition() {
    use AcceptDisposition::*;
    for k in OverlayKind::ALL {
        // Exhaustive by construction — this just witnesses each kind resolves.
        let _ = k.accept_disposition();
    }
    // The VALUE-PICKERS pop back to the parent (theme keep / caret apply /
    // dictionary apply commit a setting the summoning overlay was choosing).
    for k in [OverlayKind::Theme, OverlayKind::Caret, OverlayKind::Dictionary] {
        assert_eq!(k.accept_disposition(), ValuePick, "{k:?} is a value-picker → pop");
    }
    // The NAVIGATORS close the whole stack (open a file, jump, switch project,
    // move a note, restore a version, run a command — you land in the result).
    for k in [
        OverlayKind::Goto,
        OverlayKind::Browse,
        OverlayKind::Project,
        OverlayKind::MoveDest,
        OverlayKind::Spell,
        OverlayKind::History,
        OverlayKind::Command,
    ] {
        assert_eq!(k.accept_disposition(), Navigate, "{k:?} navigates → close-all");
    }
    // The STAY-OPEN kinds never close on accept (trash keeps listing, rebind
    // starts a capture, the settings menu toggles / swaps in place).
    for k in [OverlayKind::Assets, OverlayKind::Keybindings, OverlayKind::Settings] {
        assert_eq!(k.accept_disposition(), StayOpen, "{k:?} stays open on accept");
    }
}

/// THE OVERLAY-TITLES ROUND: every kind names itself with a nonempty, lowercase
/// title (`OverlayKind::title`) — the no-wildcard law a future kind must satisfy
/// before it compiles. Titles are also pairwise DISTINCT (so a sidecar `overlay.
/// title` read unambiguously identifies which picker is open).
#[test]
fn every_kind_names_itself_with_a_nonempty_distinct_title() {
    use std::collections::HashSet;
    let mut titles: HashSet<&'static str> = HashSet::new();
    for k in OverlayKind::ALL {
        let t = k.title();
        assert!(!t.is_empty(), "{k:?} has no title");
        assert_eq!(t, t.to_lowercase(), "{k:?}'s title {t:?} must be lowercase");
        assert!(titles.insert(t), "{k:?}'s title {t:?} collides with another kind's");
    }
    // Rename/InsertLink/KeepName are the RENDER exceptions (their own modal
    // prompt already orients) — every other kind draws its title prefix.
    for k in [OverlayKind::Rename, OverlayKind::InsertLink, OverlayKind::KeepName] {
        assert!(!k.draws_title_prefix(), "{k:?} should not draw the title prefix");
    }
    for k in OverlayKind::ALL {
        if !matches!(k, OverlayKind::Rename | OverlayKind::InsertLink | OverlayKind::KeepName) {
            assert!(k.draws_title_prefix(), "{k:?} should draw the title prefix");
        }
    }
}

/// MODE-STRING ROUND-TRIP LAW (born from the KeepName drift audit): every kind's
/// sidecar mode string resolves back to the kind via [`OverlayKind::from_mode`] —
/// the lookup the headless capture path uses to consult the REAL per-kind owners
/// (`draws_title_prefix`) instead of hand-listing mode strings (the aligned copy
/// in `capture/modes.rs` that silently kept drawing the title prefix on the
/// KeepName minibuffer until this round caught it in a capture PNG). An unknown
/// string resolves to None (fail-visible: the capture then keeps the title).
#[test]
fn every_mode_string_round_trips_through_from_mode() {
    for k in OverlayKind::ALL {
        assert_eq!(
            OverlayKind::from_mode(k.as_str()),
            Some(k),
            "{k:?}'s mode string must resolve back to itself"
        );
    }
    assert_eq!(OverlayKind::from_mode("not-a-mode"), None);
}

/// BREADCRUMB KINDS ARE VALUE-BASED, never positional. A `return_to` breadcrumb
/// stores an [`OverlayKind`] by VALUE and re-summons it by that value
/// ([`make_overlay`](crate::actions::ActionCtx) is keyed on the kind, not an
/// index), so its identity is its stable `as_str` NAME. This guards against the
/// exact class of bug the lens-fold round could have caused — retiring a sibling
/// variant SHIFTING enum positions and re-aiming a stored breadcrumb at a
/// different picker ("return to palette" decoding as "return to Goto/recents").
/// (a) `as_str` is a bijection over `ALL` — a name maps to exactly one kind, so a
/// stored kind can never be confused with another after a variant is removed.
/// (b) Only the SETTINGS surface re-summons a value-pick child on ACCEPT; every
/// other summoning surface (the Command palette, a direct summon) lands in the
/// buffer — the one gate the ship-blocker fix turns on.
#[test]
fn breadcrumb_kinds_are_value_based_never_positional() {
    use std::collections::HashSet;
    let mut names: HashSet<&'static str> = HashSet::new();
    for k in OverlayKind::ALL {
        assert!(
            names.insert(k.as_str()),
            "{k:?}: overlay names must be a bijection — {:?} is a duplicate",
            k.as_str()
        );
    }
    assert_eq!(names.len(), OverlayKind::ALL.len(), "every kind has a distinct name");
    // Exactly ONE parent retains a value-pick child on commit: Settings.
    for k in OverlayKind::ALL {
        assert_eq!(
            k.retains_value_pick_child(),
            k == OverlayKind::Settings,
            "{k:?}: only Settings re-summons a value-pick child on accept"
        );
    }
}

/// The calm, per-context empty-state COPY (the "nice text, ready" pass): each
/// context reads a warm, non-error line — the Go-to Recent lens especially
/// invites rather than reports, and a refinement lens with no members reads the
/// calm catch-all "nothing here".
#[test]
fn empty_state_copy_is_calm_and_context_aware() {
    // The refined per-kind corpus lines.
    assert_eq!(OverlayKind::Browse.empty_corpus_message(), "this folder is empty");
    assert_eq!(OverlayKind::History.empty_corpus_message(), "no history yet");
    assert_eq!(OverlayKind::Spell.empty_corpus_message(), "no suggestions");
    // Jump-to-heading + recent-projects are LENS empty-states now (the folds):
    assert_eq!(OverlayKind::Goto.empty_lens_message("headings"), Some("no headings yet"));
    assert_eq!(
        OverlayKind::Project.empty_lens_message("recent"),
        Some("no recent projects yet")
    );

    // The lens-scoped lines: Go-to Recent is the warm invitation; every other
    // refinement lens with no members reads the catch-all; `All` opts out (None).
    assert_eq!(
        OverlayKind::Goto.empty_lens_message("recent").as_deref(),
        Some("no recent files yet"),
    );
    assert_eq!(OverlayKind::Goto.empty_lens_message("folder").as_deref(), Some("nothing here"));
    assert_eq!(OverlayKind::Goto.empty_lens_message("all"), None);
}

/// A FRESH Go-to Recent lens (the recently-opened MRU is empty, nothing opened
/// yet) reads the calm "no recent files yet" line via `empty_message` — the
/// context that matters most this pass. A query still overrides with "no matches".
#[test]
fn goto_recent_empty_lens_reads_the_warm_invitation() {
    let mut ov = OverlayState::new(
        OverlayKind::Goto,
        vec!["alpha.md".into(), "beta.md".into()],
        vec![],
        vec![], // no recently-opened files → the Recent lens has no members
    );
    ov.set_facet_lens(1); // strip index 1 == Recent
    assert_eq!(ov.active_facet_id(), Some("recent"));
    assert!(ov.items.is_empty(), "a fresh Recent lens lists nothing");
    assert_eq!(
        ov.empty_notice().as_deref(),
        Some("no recent files yet"),
    );
    // A query on the empty Recent lens still reads the universal "no matches".
    ov.push('z');
    assert_eq!(ov.empty_notice().as_deref(), Some("no matches"));
}

// A Goto picker over N synthetic rows (row0..rowN-1), empty query so items are in
// corpus order 1:1.
fn deep(n: usize) -> OverlayState {
    let corpus: Vec<String> = (0..n).map(|i| format!("row{i}")).collect();
    OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![])
}

#[test]
fn hover_only_highlights_visible_rows_and_never_scrolls() {
    // 40 rows, window 12. Keyboard down to row 30 → the window scrolls so 30 is the
    // BOTTOM visible row (scroll = 30+1-12 = 19), showing items 19..=30.
    let mut ov = deep(40);
    ov.move_sel(30);
    assert_eq!(ov.selected, 30);
    assert_eq!(ov.scroll, 19);
    // Hovering a row INSIDE the visible band re-highlights it WITHOUT moving scroll.
    assert!(ov.hover_select(21));
    assert_eq!(ov.selected, 21);
    assert_eq!(ov.scroll, 19, "a hover must NOT move the scroll window");
    // Hovering the TOP visible row: still no scroll (the bug was this scrolling up).
    assert!(ov.hover_select(19));
    assert_eq!(ov.scroll, 19);
    // Hovering ABOVE the band (a row scrolled off the top) is REJECTED, no change.
    assert!(!ov.hover_select(5));
    assert_eq!(ov.selected, 19);
    assert_eq!(ov.scroll, 19);
    // Hovering BELOW the band (past the last visible row) is likewise rejected.
    assert!(!ov.hover_select(31));
    assert_eq!(ov.selected, 19);
    assert_eq!(ov.scroll, 19);
    // Re-hovering the SAME row is a no-op (returns false, nothing moved).
    assert!(!ov.hover_select(19));
}

#[test]
fn keyboard_move_keeps_selection_in_the_window() {
    let mut ov = deep(40);
    // Down a page-ish: selection tracks, window scrolls the minimum to keep it shown.
    ov.move_sel(15);
    assert_eq!(ov.selected, 15);
    assert_eq!(ov.scroll, 4); // 15+1-12
    assert!(ov.selected >= ov.scroll && ov.selected < ov.scroll + ov.window_rows());
    // Back up above the window → scroll follows up (never leaves selection off-screen).
    ov.move_sel(-14);
    assert_eq!(ov.selected, 1);
    assert_eq!(ov.scroll, 1);
    // A short list never scrolls.
    let mut small = deep(5);
    small.move_sel(100);
    assert_eq!(small.selected, 4);
    assert_eq!(small.scroll, 0);
}

#[test]
fn query_edit_resets_scroll_to_top() {
    let mut ov = deep(40);
    ov.move_sel(30);
    assert_eq!(ov.scroll, 19);
    ov.push('r'); // matches every "rowN" → list stays long, but selection resets top
    assert_eq!(ov.selected, 0);
    assert_eq!(ov.scroll, 0);
}

#[test]
fn elide_keeps_filename_and_extension_with_one_ellipsis() {
    // A deep path, narrow budget: the filename + ext survive, the DIR is elided.
    let out = elide_path("src/app/render/chrome.rs", 16);
    assert!(out.ends_with("chrome.rs"), "filename+ext must survive: {out}");
    assert_eq!(out.matches('…').count(), 1, "exactly one ellipsis: {out}");
    assert!(out.chars().count() <= 16, "fits the budget: {out}");
    // The split point is the last '/': dir prefix (muted) vs filename (content).
    let split = row_split(&out);
    assert!(out[..split].ends_with('/'));
    assert_eq!(&out[split..], "chrome.rs");
    // A row that already fits is returned WHOLE (no ellipsis, no change).
    assert_eq!(elide_path("src/main.rs", 40), "src/main.rs");
    assert_eq!(row_split("src/main.rs"), 4); // "src/"
}

#[test]
fn elide_middle_truncates_the_filename_when_it_alone_overflows() {
    // Filename longer than the whole budget → the filename ITSELF is middle-elided,
    // the directory dropped, extension end kept, still a single ellipsis.
    let out = elide_path("deep/dir/averyveryverylongfilename.rs", 12);
    assert_eq!(out.matches('…').count(), 1, "one ellipsis: {out}");
    assert!(out.chars().count() <= 12, "fits: {out}");
    assert!(out.ends_with(".rs"), "extension survives: {out}");
    assert!(!out.contains('/'), "dir dropped when the filename alone overflows: {out}");
    assert_eq!(row_split(&out), 0, "no '/', so all content ink");
    // A bare filename with no directory elides the same way.
    let bare = elide_path("supercalifragilistic.md", 10);
    assert!(bare.ends_with(".md") && bare.matches('…').count() == 1);
}

#[test]
fn browse_dir_flags_directories() {
    // One level: a folder (docs) and a file (README.md).
    let corpus = vec!["docs".to_string(), "README.md".to_string()];
    let git = vec![false, false];
    let is_dir = vec![true, false];
    let mut ov = OverlayState::new_marked(
        OverlayKind::Browse,
        corpus,
        git,
        is_dir,
        vec![],
        vec![],
        None,
    );
    // docs selected -> a directory; README.md selected -> a file.
    assert!(ov.selected_is_dir());
    ov.move_sel(1);
    assert!(!ov.selected_is_dir());
    assert_eq!(ov.selected_value(), Some("README.md"));
}

// ── WORD-OPS ROUND (b): ⌥⌫ word-delete in the minibuffer ────────────────────
// Every overlay input (the fuzzy query + the Rename / Link / Keep / Settings-
// value edits) deletes a WHOLE trailing word on ⌥⌫, routed through the ONE
// document-buffer boundary owner (`buffer::word_delete_backward_boundary`) via
// `nav::truncate_trailing_word` — so the palette can never disagree with the
// text about where a word ends. (Word MOTION ⌥←/⌥→ is intentionally NOT added:
// the query is an append/pop field with no in-query caret to move, and Left/
// Right already drive list navigation — see the round's report.)

#[test]
fn query_word_delete_removes_a_trailing_word_not_a_char() {
    let mut ov = OverlayState::new(OverlayKind::Goto, corpus(), vec![], vec![]);
    for c in "foo bar baz".chars() {
        ov.push(c);
    }
    assert_eq!(ov.query, "foo bar baz");
    ov.pop_word(); // ⌥⌫ removes the trailing word "baz"
    assert_eq!(ov.query, "foo bar ");
    ov.pop_word(); // and its whitespace + the next word
    assert_eq!(ov.query, "foo ");
    ov.pop_word();
    assert_eq!(ov.query, "");
    ov.pop_word(); // NO-OP on an empty query (never panics / underflows)
    assert_eq!(ov.query, "");
    // Plain ⌫ still removes ONE char — the split is real.
    ov.push('a');
    ov.push('b');
    ov.pop();
    assert_eq!(ov.query, "a");
}

#[test]
fn rename_minibuffer_word_delete() {
    let mut ov = OverlayState::new_rename("hello world".to_string());
    ov.rename_edit_pop_word();
    // The word-deleted value mirrors into corpus[0] (the visible editable row).
    assert_eq!(ov.corpus[0], "hello ");
    ov.rename_edit_pop_word();
    assert_eq!(ov.corpus[0], "");
}

#[test]
fn link_minibuffer_word_delete() {
    let mut ov =
        OverlayState::new_link_edit("http://a.com/path".to_string(), LinkEditMode::Empty { at: 0 });
    ov.link_edit_pop_word(); // drops the trailing "path" segment, keeps the "/"
    assert_eq!(ov.corpus[0], "http://a.com/");
}

#[test]
fn keep_minibuffer_word_delete() {
    let mut ov = OverlayState::new_keep_name();
    for c in "my great note".chars() {
        ov.keep_edit_push(c);
    }
    ov.keep_edit_pop_word();
    assert_eq!(ov.corpus[0], "my great ");
}
