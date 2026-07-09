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
#[allow(dead_code)] // kept for future BuildCtx-driven tests (was the recents-build helper).
fn empty_build_ctx<'a>(config_keys: &'a [(String, Vec<String>)]) -> BuildCtx<'a> {
    BuildCtx {
        goto_corpus: Vec::new(),
        goto_open: Vec::new(),
        goto_recent: Vec::new(),
        goto_times: Vec::new(),
        config_keys,
        goto_headings: Vec::new(),
        spell_target: None,
        history_entries: Vec::new(),
        history_now: None,
        history_session_start: None,
        settings_values: Default::default(),
        assets: Vec::new(),
    }
}

#[test]
fn goto_headings_lens_folds_in_the_docs_headings() {
    // THE FOLD: a Go-to overlay with the doc's headings attached lists ONLY files
    // under All / the file lenses, and ONLY the headings under the Headings lens —
    // which is where the retired standalone Outline picker now lives.
    let corpus = vec!["README.md".to_string(), "src/main.rs".to_string()];
    let mut ov = OverlayState::new(OverlayKind::Goto, corpus, vec![], vec![]);
    ov.attach_headings(vec![
        ("Introduction".to_string(), 3),
        ("  Details".to_string(), 7),
    ]);
    // Strip carries the Headings lens, parked last after the file lenses.
    let strip: Vec<String> = ov.lens_strip().into_iter().map(|(l, _)| l).collect();
    assert_eq!(strip, vec!["All", "Recent", "This folder", "By type", "Headings"]);
    // ALL home: files only — the appended heading rows are hidden here.
    assert_eq!(ov.active_facet_id(), Some("all"));
    let all = ov.item_strings();
    assert!(all.iter().any(|s| s == "README.md") && all.iter().any(|s| s == "src/main.rs"));
    assert!(!all.iter().any(|s| s == "Introduction"), "headings hidden under All: {all:?}");
    assert!(!ov.selected_is_heading(), "a file row is not a heading");
    // Headings lens (strip index 4): ONLY the headings, and each row IS a heading
    // whose accept is its line number, not a file open.
    ov.focus_facet_id("headings");
    assert_eq!(ov.active_facet_id(), Some("headings"));
    assert_eq!(ov.item_strings(), vec!["Introduction".to_string(), "  Details".to_string()]);
    assert!(ov.selected_is_heading(), "the Headings lens rows are headings");
    assert_eq!(ov.selected_line(), Some(3), "the first heading jumps to line 3");
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
fn theme_picker_groups_by_lens_and_selects_active() {
    use crate::theme::Lens;
    // The full world corpus (in THEMES order) + Gumtree active (its index).
    let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let gum = names.iter().position(|n| n == "Gumtree").unwrap();
    // Open with Gumtree active -> mode "theme", opens on the flat All lens.
    let mut ov = OverlayState::new_theme(names.clone(), gum);
    assert_eq!(ov.kind.as_str(), "theme");
    assert_eq!(ov.original_theme, Some(gum));
    assert_eq!(ov.active_facet_id(), Some("all"), "opens on the flat All landing");
    assert_eq!(ov.selected_value(), Some("Gumtree"));
    // Step into the Time lens (strip index 1) to exercise the grouping (Gumtree is
    // shown under Time).
    ov.set_facet_lens(1);
    assert_eq!(ov.active_facet_id(), Some("time"));
    // The active world is highlighted (and thus previewed) wherever its section is.
    assert_eq!(ov.selected_value(), Some("Gumtree"));
    // Grouped by Time: rows come out in section order (Dawn, Day, Dusk, Night),
    // and each row's parallel section label matches the world's Time tag.
    let sections = ov.item_sections();
    assert_eq!(sections.len(), ov.item_strings().len());
    for (row, name) in ov.item_strings().iter().enumerate() {
        // Every grouped row is a SHOWN world, so its Time tag is `Some`.
        assert_eq!(
            Some(sections[row].as_str()),
            crate::theme::tag_for(name, Lens::Time),
            "row {name} under wrong section"
        );
    }
    // Sections appear in the lens's declared order (no interleaving).
    let order: Vec<&str> = Lens::Time.sections().to_vec();
    let mut last = 0usize;
    for s in &sections {
        let pos = order.iter().position(|o| o == s).unwrap();
        assert!(pos >= last, "sections must be contiguous + ordered: {sections:?}");
        last = pos;
    }
    // No git / dir markers on the theme rows.
    assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
}

#[test]
fn theme_lens_cycles_with_all_parked_left_and_keeps_world() {
    let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    // Potoroo headlines ALL four faceted lenses, so it survives every regroup.
    let potoroo = names.iter().position(|n| n == "Potoroo").unwrap();
    let mut ov = OverlayState::new_theme(names, potoroo);
    assert_eq!(ov.active_facet_id(), Some("all"), "opens on the far-left All landing");
    assert_eq!(ov.selected_value(), Some("Potoroo"));
    // The All lens is the flat corpus list (no section headers).
    assert!(ov.item_sections().iter().all(|s| s.is_empty()));
    // LEFT at All is a clamped no-op (nothing before it).
    ov.cycle_lens(-1);
    assert_eq!(ov.active_facet_id(), Some("all"), "All is the far-left floor");
    // RIGHT steps along the strip; the highlighted world is KEPT across regroups.
    for expect in ["time", "register", "voice", "temperature"] {
        ov.cycle_lens(1);
        assert_eq!(ov.active_facet_id(), Some(expect));
        assert_eq!(ov.selected_value(), Some("Potoroo"));
    }
    // RIGHT at Temperature is a clamped no-op (it is now the far-right end).
    ov.cycle_lens(1);
    assert_eq!(ov.active_facet_id(), Some("temperature"), "Temperature parked at the far right");
    // The lens strip reflects the active lens (exactly one active, All FIRST).
    let strip = ov.lens_strip();
    assert_eq!(strip.len(), 5);
    assert_eq!(strip.first().unwrap().0, "All");
    assert_eq!(strip.iter().filter(|(_, a)| *a).count(), 1);
    assert!(strip[4].1, "Temperature is active");
}

/// The CLICKABLE lens strip's pointing counterpart to a no-op LEFT/RIGHT at an
/// end: clicking the ALREADY-ACTIVE facet is a calm no-op (documented on
/// `set_facet_lens` itself) — `facet_lens`, the selected item, and the scroll
/// position all stay byte-identical, unlike a real switch (which regroups the
/// list and can move `selected`/`scroll`).
#[test]
fn clicking_the_current_facet_is_a_calm_no_op() {
    let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let potoroo = names.iter().position(|n| n == "Potoroo").unwrap();
    let mut ov = OverlayState::new_theme(names, potoroo);
    ov.set_facet_lens(2); // switch to Register once, a real change
    assert_eq!(ov.active_facet_id(), Some("register"));
    let (before_lens, before_selected, before_scroll, before_items) =
        (ov.facet_lens, ov.selected, ov.scroll, ov.item_strings());
    ov.set_facet_lens(2); // click the SAME facet again — a calm no-op
    assert_eq!(ov.facet_lens, before_lens);
    assert_eq!(ov.selected, before_selected);
    assert_eq!(ov.scroll, before_scroll);
    assert_eq!(ov.item_strings(), before_items);
}

#[test]
fn opted_out_world_hidden_under_its_lens_but_present_under_all() {
    use crate::theme::Lens;
    let names: Vec<String> = crate::theme::THEMES.iter().map(|t| t.name.to_string()).collect();
    let gum = names.iter().position(|n| n == "Gumtree").unwrap();
    let mut ov = OverlayState::new_theme(names, gum);
    // Tawny opts OUT of Voice (voice: None), so it never appears in the Voice
    // grouping — every SHOWN row under Voice has a `Some` Voice tag.
    ov.set_facet_lens(3); // Voice
    assert!(
        !ov.item_strings().iter().any(|n| n == "Tawny"),
        "Tawny is hidden under the Voice lens"
    );
    for name in ov.item_strings() {
        assert!(
            crate::theme::tag_for(&name, Lens::Voice).is_some(),
            "{name} shown under Voice must carry a Some tag"
        );
    }
    // But the flat All lens still lists EVERY world, Tawny included (opt-out only
    // trims the faceted lenses; nothing is unreachable).
    ov.set_facet_lens(0); // All
    assert_eq!(ov.item_strings().len(), crate::theme::THEMES.len());
    assert!(ov.item_strings().iter().any(|n| n == "Tawny"));
}

#[test]
fn theme_lens_is_flat_all_and_no_strip_for_nontheme() {
    // A non-faceting picker never grows a lens strip or section labels, and has no
    // facet scheme (so `active_facet_id` is None). The Caret picker is a good flat,
    // non-faceting example (Goto/Browse/Command/History now facet — see
    // `facets::scheme`).
    let ov = OverlayState::new(OverlayKind::Caret, corpus(), vec![], vec![]);
    assert!(!ov.is_faceting());
    assert!(ov.lens_strip().is_empty());
    assert!(ov.active_facet_id().is_none());
    assert!(ov.item_sections().iter().all(|s| s.is_empty()));
    // cycle_lens on a non-faceting picker is inert (facet_lens stays 0).
    let mut ov = ov;
    ov.cycle_lens(1);
    assert_eq!(ov.facet_lens, 0);
}

#[test]
fn caret_picker_lists_three_styles_navigates_and_maps_modes() {
    use crate::caret::CaretMode;
    // SUMMON with Block active: the corpus is the three look labels in ALL order,
    // each row's "binding" column carrying its description.
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
    // NAVIGATE down the list -> the selected look maps back via from_label.
    let mut ov = ov;
    ov.move_sel(1);
    assert_eq!(ov.selected_caret_mode(), Some(CaretMode::Morph));
    ov.move_sel(1);
    assert_eq!(ov.selected_caret_mode(), Some(CaretMode::Ibeam));
    // Opening with a non-Block look pre-selects THAT row.
    let ov2 = OverlayState::new_caret(CaretMode::Ibeam);
    assert_eq!(ov2.selected_value(), Some("I-beam"));
    assert_eq!(ov2.original_caret, Some(CaretMode::Ibeam));
    // The hint names ↵'s action; flat picker (no descend).
    assert_eq!(OverlayKind::Caret.hint(), "\u{2191}/\u{2193} move   \u{21B5} apply");
    // selected_caret_mode is None for a non-caret picker.
    let theme = OverlayState::new_theme(vec!["Tawny".into()], 0);
    assert_eq!(theme.selected_caret_mode(), None);
}

#[test]
fn command_palette_lists_names_with_parallel_bindings() {
    let names = vec![
        "Go to file".to_string(),
        "Switch theme".to_string(),
        "Save".to_string(),
    ];
    let binds = vec!["C-x C-f".to_string(), "C-x t".to_string(), "C-x C-s".to_string()];
    let mut ov = OverlayState::new_command(names.clone(), binds.clone());
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
    // Rows are the (indented) titles in order; lines stay parallel.
    assert_eq!(ov.item_strings(), vec!["Intro", "  Setup", "  Usage"]);
    assert_eq!(ov.selected_line(), Some(0));
    // Fuzzy filter to "Usage" -> selected row jumps to its line (9), not its text.
    ov.push('u');
    ov.push('s');
    ov.push('a');
    assert_eq!(ov.selected_value(), Some("  Usage"));
    assert!(ov.selected_is_heading());
    assert_eq!(ov.selected_line(), Some(9));
    // No git / dir markers on heading rows; the indentation survives in display.
    assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
}

#[test]
fn spell_picker_lists_suggestions_and_carries_target() {
    // Three corrections for a word flagged at line 2, cols 6..13.
    let sugg = vec!["receive".to_string(), "relieve".to_string(), "reprieve".to_string()];
    let ov = OverlayState::new_spell(sugg.clone(), (2, 6, 13));
    assert_eq!(ov.kind.as_str(), "spell");
    // Rows are the suggestions in order (best first); the top is selected.
    assert_eq!(ov.item_strings(), sugg);
    assert_eq!(ov.selected_value(), Some("receive"));
    // The target span is carried so the accept can replace the word.
    assert_eq!(ov.spell_target, Some((2, 6, 13)));
    // No git / dir markers on the suggestion rows.
    assert!(ov.item_strings().iter().all(|s| !s.contains('•') && !s.ends_with('/')));
    // The hint names the ↵ action (replace), flat picker (no descend).
    assert_eq!(OverlayKind::Spell.hint(), "\u{2191}/\u{2193} move   \u{21B5} replace");
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
    // The hint teaches restore + lens + close (informational, button-free).
    assert_eq!(
        OverlayKind::History.hint(),
        "\u{2191}/\u{2193} move   ↵ restore   \u{2190}/\u{2192} lens   esc close"
    );
    assert!(ov.foot_hint().contains("restore"));
}

#[test]
fn command_picker_lands_on_all_then_groups_by_menu_section_and_recent() {
    let names = crate::commands::names();
    let binds = crate::commands::effective_bindings(&[]);
    let mut ov = OverlayState::new_command(names, binds);
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
    };
    let ov = OverlayState::new_history(vec![mk("2", true), mk("1", false)], None, None);
    let binds = ov.item_bindings();
    assert!(binds[0].contains(PIN_TAG), "the pinned row is marked: {:?}", binds[0]);
    assert!(binds[0].contains("+0 −1"), "and keeps its changed-count: {:?}", binds[0]);
    assert!(!binds[1].contains(PIN_TAG), "an un-pinned row stays bare: {:?}", binds[1]);
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
    let names = crate::commands::names();
    let binds = crate::commands::effective_bindings(&[]);
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
        crate::commands::names(),
        crate::commands::effective_bindings(&[]),
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
        crate::commands::names(),
        crate::commands::effective_bindings(&[]),
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
        // The universal ↑/↓ move LEADS the line, then the primary ↵ Return action.
        assert!(h.starts_with("\u{2191}/\u{2193} move"), "{k:?} hint leads with ↑/↓ move: {h}");
        assert!(h.contains("\u{21B5}"), "{k:?} hint names ↵ Return: {h}");
    }
    // Project ↵ SELECTS; MoveDest ↵ MOVES.
    assert!(OverlayKind::Project.hint().contains("\u{21B5} select"));
    assert!(OverlayKind::MoveDest.hint().contains("move here"));
    // The FACETED pickers (Goto / Browse / Theme / Command / History) teach ←/→
    // lens, not ->/C-f descend, and each starts with the ↵ Return glyph.
    for k in [
        OverlayKind::Goto,
        OverlayKind::Browse,
        OverlayKind::Theme,
        OverlayKind::Command,
        OverlayKind::History,
    ] {
        let h = k.hint();
        assert!(!h.contains("C-f"), "{k:?} facets, no descend hint: {h}");
        assert!(h.contains("\u{2190}/\u{2192} lens"), "{k:?} hint should teach ←/→ lens: {h}");
        assert!(h.starts_with("\u{2191}/\u{2193} move"), "{k:?} hint leads with ↑/↓ move: {h}");
    }
    // Browse ↵ still OPENS (a folder descends / a file opens) and ⌫ ascends.
    assert!(OverlayKind::Browse.hint().contains("\u{21B5} open"));
    assert!(OverlayKind::Browse.hint().contains("\u{232B} up"));
}

/// The SHARED hint formatter produces ONE consistent shape for every picker:
/// `glyph SPACE label`, actions joined by the single `HINT_SEP`, the universal
/// `↑/↓ move` FIRST, then the primary (↵), and cancel (esc) — where present —
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
    // label` (exactly one space), the separator is HINT_SEP, ↵ leads, and any
    // cancel action is the lowercase `esc` (never `Esc`) sitting LAST.
    for k in OverlayKind::ALL {
        let actions = k.hint_actions();
        assert!(actions.len() >= 2, "{k:?} must teach move + at least one action");
        // Move-first: the universal ↑/↓ move leads, then the ↵ Return primary.
        assert_eq!(actions[0].glyph, "\u{2191}/\u{2193}", "{k:?} leads with ↑/↓ move");
        assert_eq!(actions[0].label, "move", "{k:?} lead action is labelled move");
        assert_eq!(actions[1].glyph, "\u{21B5}", "{k:?} ↵ primary follows the move lead");
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
    assert_eq!(OverlayKind::Goto.empty_lens_message("type").as_deref(), Some("nothing here"));
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
