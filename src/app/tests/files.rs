use super::*;

// ── GOTO FILE-INDEX FRESHNESS (queue: "file picker freshness") ──────────
//
// The go-to overlay (`C-x f`) corpus comes from `App.file_index`, a CACHED
// field only ever rebuilt on specific triggers (root switch, a note's first
// save, a rename, a move) — never simply because the picker summoned. A file
// dropped into the root by another process, or a shell command, while awl
// sits open would never appear until one of those triggers happened to also
// fire. The fix: RE-SCAN ON EVERY SUMMON via `App::rescan_file_index` (called
// from `App::apply`'s `Action::OpenGoto` arm, over the `FileSystem` trait) —
// no watcher, no TTL, just re-walk right as the overlay opens.

#[test]
fn rescan_file_index_picks_up_a_file_created_after_the_last_scan() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new().with_file("/proj/a.txt", "a\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(None, "/proj", Config::empty());
    // The initial scan (at App::new) sees only the file that existed then.
    assert_eq!(app.file_index, vec!["a.txt".to_string()]);
    // SUMMON #1 (simulated: `rescan_file_index` is exactly what `C-x f`
    // triggers): still just the one file — nothing has changed yet.
    app.rescan_file_index();
    assert_eq!(app.file_index, vec!["a.txt".to_string()]);
    // A file appears on disk WITHOUT going through awl at all (another
    // process, a git checkout, a plain `touch`) — the picker is CLOSED at
    // this point, so nothing in awl has any reason to know yet.
    mem.write(std::path::Path::new("/proj/b.txt"), b"b\n")
        .unwrap();
    assert_eq!(
        app.file_index,
        vec!["a.txt".to_string()],
        "the cached index does not spontaneously update"
    );
    // SUMMON #2 (`C-x f` again): the fresh scan MUST find it.
    app.rescan_file_index();
    assert_eq!(
        app.file_index,
        vec!["a.txt".to_string(), "b.txt".to_string()],
        "re-summoning must re-scan and pick up the new file"
    );
    // Build the ACTUAL overlay the way `App::apply`'s Goto arm does, to prove
    // the fresh index really reaches the summoned picker's corpus (the same
    // `overlay::build` the live App and headless replay both call).
    let effective_keep = app.config.effective_linux_keep();
    let build_ctx = crate::overlay::BuildCtx {
        goto_corpus: app.file_index.clone(),
        goto_open: Vec::new(),
        goto_recent: Vec::new(),
        goto_times: Vec::new(),
        config_keys: &app.config.keys,
        config_linux_keep: &effective_keep,
        goto_headings: Vec::new(),
        spell_target: None,
        history_entries: Vec::new(),
        history_now: None,
        history_session_start: None,
        settings_values: Default::default(),
        assets: Vec::new(),
    };
    let ov = crate::overlay::build(crate::overlay::OverlayKind::Goto, &build_ctx)
        .expect("Goto always summons");
    assert!(
        ov.corpus.contains(&"b.txt".to_string()),
        "the new file is listed"
    );
}

// ── THE KEYMAP FLAVOR ROUND — the Settings "Keymap" toggle round-trip ────

/// Enter on the "Keymap" settings row (`App::toggle_keymap_flavor`, the
/// special-cased door `App::setting_toggle` routes "keymap" through):
/// flips native <-> emacs, PERSISTS the flip format-preservingly (the same
/// `persist_pref` owner every other sticky pref rides), and re-applies the
/// keymap LIVE from the updated in-memory config — proven here by feeding
/// the SAME `app.config.effective_linux_keep()` a fresh `KeymapState`
/// would consume (the exact composition `toggle_keymap_flavor` rebuilds
/// `self.keymap` from) into a `Convention::Linux`-pinned keymap and
/// confirming it now carries the full emacs preset.
#[test]
fn settings_keymap_toggle_flips_persists_and_live_reapplies() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let cfg = Config {
        path: PathBuf::from("/cfg/config.toml"),
        ..Config::empty()
    };
    let mut app = app_on(None, "/proj", cfg);
    assert_eq!(
        app.config.keymap_flavor(),
        crate::keymap::KeymapFlavor::Native,
        "starts native"
    );

    // Enter #1: native -> emacs.
    app.toggle_keymap_flavor();
    assert_eq!(
        app.config.keymap_flavor(),
        crate::keymap::KeymapFlavor::Emacs,
        "in-memory mirror flips"
    );
    let written = mem
        .read_to_string(std::path::Path::new("/cfg/config.toml"))
        .unwrap();
    assert!(
        written.contains("keymap = \"emacs\""),
        "persisted format-preservingly: {written:?}"
    );

    // LIVE RE-APPLY: the same composed keep-list the toggle rebuilt
    // `self.keymap` from now carries the WHOLE emacs preset — build a
    // fresh convention-pinned keymap from exactly that composition (the
    // private `KeymapState.linux_keep` field can't be introspected from
    // here, so this proves the INPUT the live rebuild consumed, which
    // `keymap::tests::keymap_flavor_emacs_preset_reverts_every_displaced_chord_to_emacs_meaning`
    // already proves is sufficient to flip dispatch).
    let effective = app.config.effective_linux_keep();
    let preset = crate::keymap::linux_emacs_preset_keep();
    // The insert-link-yields-to-kill-line round's built-in floor
    // (`keymap::linux_builtin_keep()`) rides ALONG with the preset — it is
    // NOT flavor-gated, so it's present under emacs too, just not part of
    // `preset` itself (see `linux_builtin_keep()`'s own doc).
    assert_eq!(
        effective.len(),
        preset.len() + crate::keymap::linux_builtin_keep().len(),
        "the live rebuild's keep-list is the whole preset plus the built-in floor"
    );
    for chord in &preset {
        assert!(
            effective.contains(chord),
            "{chord:?} missing from the live rebuild's keep-list"
        );
    }
    for chord in crate::keymap::linux_builtin_keep() {
        assert!(
            effective.iter().any(|c| c == chord),
            "{chord:?} missing from the live rebuild's keep-list"
        );
    }

    // Enter #2: emacs -> native (round-trips cleanly, doesn't accumulate).
    app.toggle_keymap_flavor();
    assert_eq!(
        app.config.keymap_flavor(),
        crate::keymap::KeymapFlavor::Native,
        "flips back"
    );
    let written2 = mem
        .read_to_string(std::path::Path::new("/cfg/config.toml"))
        .unwrap();
    assert!(
        written2.contains("keymap = \"native\""),
        "the second toggle persists too: {written2:?}"
    );
    // Native flavor: no preset widening, but the built-in floor is still
    // there (it's unconditional, not flavor-gated) — never truly empty.
    assert_eq!(
        app.config.effective_linux_keep().len(),
        crate::keymap::linux_builtin_keep().len(),
        "native flavor: no preset widening, just the built-in floor"
    );
}

/// LAW TEST (the "settings toggle rows dispatch live" round): EVERY row
/// the corpus marks `SettingKind::Toggle` — enumerated straight off
/// `settings::visible_rows()`, never hand-copied — round-trips through
/// the REAL live door, `App::setting_toggle(key)` (exactly what
/// `Effect::SettingToggle` resolves to at the `app/apply.rs` seam, see
/// `App::apply`'s `Effect::SettingToggle { key } => self.setting_toggle(&key)`
/// arm): the value readout VISIBLY CHANGES after one toggle, and
/// round-trips back to its exact starting value after a second — so a
/// toggle that silently no-ops (the Keymap-row bug: wired in
/// `settings::toggle_key` and in `settings_accept`, but never driven
/// through `App::setting_toggle` itself by any prior test — the prior
/// `settings_keymap_toggle_flips_persists_and_live_reapplies` test called
/// `app.toggle_keymap_flavor()` directly, skipping the string-keyed
/// dispatch a live Enter/click actually goes through) fails here instead
/// of shipping quietly. Companion:
/// `actions::tests::overlay_drive::every_settings_toggle_row_signals_its_own_setting_toggle_key`
/// (the pure `apply_core`-level half: Enter on the row signals the RIGHT
/// key in the first place). Each toggle is undone immediately after
/// asserting it, so every process-global this sweep touches (page /
/// typewriter / wysiwyg / inline images / ligatures / spellcheck /
/// writing nits / outline / menu bar / reduce motion) is back to its
/// pre-test value by the time the lock releases — no leak into a sibling
/// test, mirroring the `page::measure()` save/restore convention used
/// elsewhere in this file.
#[test]
fn every_settings_toggle_row_dispatches_live_and_flips_its_value() {
    use crate::fs::InMemoryFs;
    let _g2 = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
    let _g = crate::testlock::serial();

    let cfg = Config {
        path: PathBuf::from("/cfg/config.toml"),
        ..Config::empty()
    };
    let mut app = app_on(None, "/proj", cfg);

    let toggle_rows: Vec<crate::settings::SettingRow> = crate::settings::visible_rows()
        .into_iter()
        .filter(|r| r.kind == crate::settings::SettingKind::Toggle)
        .copied()
        .collect();
    assert_eq!(
        toggle_rows.len(),
        16,
        "the toggle roster changed size — update this sweep deliberately"
    );

    let gather = |app: &App| {
        crate::settings::SettingsValues::gather(
            &app.config,
            &app.root,
            app.zoom,
            crate::dateformat::CAPTURE_PLACEHOLDER_YMD,
        )
    };
    for row in &toggle_rows {
        let key = crate::settings::toggle_key(row.name).expect("a Toggle row always has a key");
        let values0 = gather(&app);
        let before = crate::settings::value_for(row, &values0);

        // DATE FORMAT is a genuine FIVE-way CYCLE (unlike every other Toggle
        // row here, which is a plain bool OR "Keymap"'s own 2-state cycle) —
        // snapshot the process-global directly so restoration below doesn't
        // assume "two toggles returns to start" (true for a 2-state row,
        // false for a 5-state one).
        let date_format_before =
            (row.name == "Date format").then(crate::dateformat::active_format);

        app.setting_toggle(key);
        let values1 = gather(&app);
        let after = crate::settings::value_for(row, &values1);
        assert_ne!(
            before, after,
            "row {:?} (key {:?}) did not visibly flip its value readout — the live dispatch is a silent no-op",
            row.name, key
        );

        if let Some(saved) = date_format_before {
            crate::dateformat::set_active_format(saved); // restore, no leak
            continue;
        }

        // Toggle back — restores the global/config AND proves the flip is
        // a clean round-trip, not a one-way ratchet.
        app.setting_toggle(key);
        let values2 = gather(&app);
        let restored = crate::settings::value_for(row, &values2);
        assert_eq!(
            restored, before,
            "row {:?} (key {:?}) did not round-trip back to its starting value",
            row.name, key
        );
    }
}

/// The corpus GREW to carry the row: "Keymap" is a real, visible settings
/// row (mirrors `settings::tests::settings_table_names_are_unique`'s own
/// count law, exercised here through the App's own config/root — a
/// belt-and-suspenders confirmation that the live overlay build would
/// actually list it).
#[test]
fn settings_corpus_includes_the_keymap_row() {
    assert!(crate::settings::visible_names().contains(&"Keymap".to_string()));
    assert_eq!(crate::settings::toggle_key("Keymap"), Some("keymap"));
}

#[test]
fn disk_changed_truth_table() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let p = std::path::Path::new("/d/f.md");
    // (None, None): the file never existed — our write CREATES it, no clobber.
    assert!(!App::disk_changed(p, None));
    mem.write(p, b"v1").unwrap();
    let t1 = App::disk_mtime_of(p);
    assert!(t1.is_some(), "the fake records mtimes");
    // (Some, Some) equal → unchanged.
    assert!(!App::disk_changed(p, t1));
    // (Some, None): the file APPEARED externally since we looked.
    assert!(App::disk_changed(p, None));
    // (Some, Some) differing → a real external change.
    std::thread::sleep(Duration::from_millis(2)); // ensure a distinct mtime
    mem.write(p, b"v2").unwrap();
    assert!(App::disk_changed(p, t1));
    // (Some, Some) with the SAME mtime but a DIFFERENT size → a same-tick
    // external edit (equal mtime, changed content) must still be caught by the
    // size guard, or we'd silently overwrite it.
    let cur = App::disk_mtime_of(p).expect("v2 exists");
    let same_tick_other_size = Some(crate::fs::Metadata {
        modified: cur.modified,
        len: cur.len.map(|n| n + 1),
    });
    assert!(App::disk_changed(p, same_tick_other_size));
    // (None, Some): the file was DELETED externally (renamed away here — the
    // trait has no remove op, and a rename models the same disappearance).
    let last = App::disk_mtime_of(p);
    mem.rename(p, std::path::Path::new("/d/elsewhere.md"))
        .unwrap();
    assert!(App::disk_changed(p, last));
}

#[test]
fn autosave_flush_writes_doc_and_snapshots_loose_file() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert!(
        app.autosave_last_ok.is_none(),
        "the debug panel's autosave clock is untouched before any write"
    );
    app.buffer.set_text("v2\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&p).unwrap(),
        "v2\n",
        "the edit hit the disk"
    );
    assert_eq!(
        app.doc_saved_version,
        Some(app.buffer.version()),
        "the flushed version is bookkept"
    );
    assert!(app.notice.is_none(), "a clean write raises no notice");
    assert!(
        app.autosave_last_ok.is_some(),
        "a real engine write stamps the debug panel's autosave clock"
    );
    // The debug panel's pure composer agrees: enabled + not held + a stamped
    // write => Saved (never Off/Held after a clean autosave).
    assert!(matches!(
        crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), Some(0)),
        crate::debug::AutosaveState::Saved(Some(0))
    ));
    // Every save records: the loose file grew a history snapshot.
    assert!(
        !crate::history::list(&p).is_empty(),
        "autosave records a local-history snapshot for a loose file"
    );
    // An unchanged buffer is not re-written (version bookkeeping short-circuits).
    let t = App::disk_mtime_of(&p);
    app.autosave_flush();
    assert_eq!(
        App::disk_mtime_of(&p),
        t,
        "no redundant write for a clean buffer"
    );
}

#[test]
fn autosave_flush_skips_and_notices_when_disk_changed_externally() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "disk v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    // Someone ELSE writes the file behind awl's back.
    std::thread::sleep(Duration::from_millis(2)); // distinct mtime
    mem.write(&p, b"external edit\n").unwrap();
    app.buffer.set_text("mine\n");
    app.autosave_flush();
    // The CLOBBER GUARD held the write: the external edit survives on disk.
    assert_eq!(
        mem.read_to_string(&p).unwrap(),
        "external edit\n",
        "autosave never overwrites external edits"
    );
    assert_eq!(
        app.notice.as_deref(),
        Some(CLOBBER_NOTICE),
        "a calm notice is raised"
    );
    assert!(
        app.autosave_last_ok.is_none(),
        "a HELD write must never stamp the debug panel's autosave clock — no write happened"
    );
    // The debug panel's pure composer agrees: held wins over "nothing written yet".
    assert_eq!(
        crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), None),
        crate::debug::AutosaveState::Held
    );
    // The version is marked handled so the idle timer doesn't spin; the NEXT
    // edit re-arms the engine (and the notice would recur calmly).
    assert_eq!(app.doc_saved_version, Some(app.buffer.version()));
}

#[test]
fn autosave_off_disables_flush() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let cfg = Config {
        autosave: Some(false),
        ..Config::empty()
    };
    let mut app = app_on(Some(p.clone()), "/notes", cfg);
    app.buffer.set_text("v2\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&p).unwrap(),
        "v1\n",
        "autosave = false leaves the disk untouched"
    );
    assert!(app.notice.is_none());
    assert!(
        app.autosave_last_ok.is_none(),
        "a disabled engine never stamps the debug panel's autosave clock"
    );
    // The debug panel's pure composer agrees: disabled wins over everything.
    assert_eq!(
        crate::debug::autosave_state(app.config.autosave_on(), app.notice.is_some(), None),
        crate::debug::AutosaveState::Off
    );
}

#[test]
fn load_path_flushes_the_leaving_buffer() {
    use crate::fs::{FileSystem, InMemoryFs};
    let a = PathBuf::from("/notes/a.md");
    let b = PathBuf::from("/notes/b.md");
    let mem = InMemoryFs::new().with_file(&a, "A\n").with_file(&b, "B\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
    app.buffer.set_text("A edited\n");
    app.load_path(b.clone());
    assert_eq!(
        mem.read_to_string(&a).unwrap(),
        "A edited\n",
        "switching files flushes the buffer being left"
    );
    assert_eq!(app.buffer.text(), "B\n", "the new file is open");
    assert_eq!(
        app.doc_saved_version,
        Some(app.buffer.version()),
        "the arriving buffer starts saved"
    );
}

// ── i18n WRITE-BACK-ONCE (App::new launch arg + App::load_path switch) ───

#[test]
fn launching_on_an_untagged_japanese_file_tags_it_once() {
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/nihongo.md");
    let original = "これは日本語の文章です。\n";
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert_eq!(
        app.buffer.text(),
        format!("---\nlang: ja\n---\n{original}"),
        "an untagged kana-bearing doc is tagged ja on first open"
    );
    // NEVER a silent disk write: the file on disk is untouched, and the
    // buffer reads as DIRTY (past doc_saved_version) so the ordinary
    // autosave engine picks the tag up on the next idle/blur/switch/quit.
    assert_eq!(
        mem.read_to_string(&p).unwrap(),
        original,
        "disk is untouched"
    );
    assert!(
        app.doc_saved_version.unwrap() < app.buffer.version(),
        "the stamped tag is a PENDING edit, not already-saved"
    );
}

#[test]
fn write_back_never_touches_a_pure_latin_document() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/english.md");
    let original = "Just some ordinary English prose.\n";
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert_eq!(
        app.buffer.text(),
        original,
        "a pure-Latin doc is never touched"
    );
    assert_eq!(
        app.doc_saved_version,
        Some(app.buffer.version()),
        "no edit landed -> still reads as saved"
    );
}

#[test]
fn write_back_never_fires_on_a_non_markdown_file() {
    use crate::fs::InMemoryFs;
    // A `.rs` file with a Japanese string literal: frontmatter is a
    // markdown/notes convention, and stamping `---`/`lang:` text into a
    // code file would corrupt it, so this must stay untouched.
    let p = PathBuf::from("/proj/main.rs");
    let original = "fn main() {\n    println!(\"こんにちは\");\n}\n";
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let app = app_on(Some(p.clone()), "/proj", Config::empty());
    assert_eq!(
        app.buffer.text(),
        original,
        "a non-markdown file is never tagged"
    );
}

#[test]
fn write_back_uses_the_configured_cjk_priority_for_ambiguous_han() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/hanzi.md");
    let original = "汉字漢字\n"; // Han only, no kana/hangul/bopomofo -> ambiguous
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let cfg = Config {
        cjk_priority: Some(vec![
            crate::frontmatter::Lang::ZhHans,
            crate::frontmatter::Lang::Ja,
        ]),
        ..Config::empty()
    };
    let app = app_on(Some(p.clone()), "/notes", cfg);
    assert_eq!(
        app.buffer.text(),
        format!("---\nlang: zh-Hans\n---\n{original}")
    );
}

#[test]
fn write_back_is_undoable_with_cmd_z() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/nihongo.md");
    let original = "こんにちは\n";
    let mem = InMemoryFs::new().with_file(&p, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert_ne!(app.buffer.text(), original, "the tag landed");
    app.buffer.undo();
    assert_eq!(
        app.buffer.text(),
        original,
        "Cmd-Z removes the stamped tag cleanly"
    );
}

#[test]
fn write_back_never_re_tags_a_document_already_carrying_frontmatter() {
    use crate::fs::InMemoryFs;
    let p = PathBuf::from("/notes/tagged.md");
    // Already tagged (as if a previous session's write-back had already
    // fired and been saved) — must never gain a SECOND block.
    let already = "---\nlang: ja\n---\nこんにちは\n";
    let mem = InMemoryFs::new().with_file(&p, already);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert_eq!(
        app.buffer.text(),
        already,
        "an already-tagged doc is untouched"
    );
    assert_eq!(
        app.doc_saved_version,
        Some(app.buffer.version()),
        "no edit landed -> still reads as saved"
    );
}

#[test]
fn write_back_never_fires_twice_across_a_reopen() {
    use crate::fs::{FileSystem, InMemoryFs};
    let a = PathBuf::from("/notes/a.md");
    let b = PathBuf::from("/notes/nihongo.md");
    let original = "こんにちは\n";
    let mem = InMemoryFs::new()
        .with_file(&a, "hello\n")
        .with_file(&b, original);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
    // First open of `b`: tags it (still only in-memory — disk untouched).
    app.load_path(b.clone());
    let tagged = app.buffer.text();
    assert_eq!(tagged, format!("---\nlang: ja\n---\n{original}"));
    // Simulate a save (autosave/Cmd-S would write exactly this).
    mem.write(&b, tagged.as_bytes()).unwrap();
    // Switch away, then back: `load_path`'s SWITCH branch (already open in
    // the registry) restores the live buffer untouched — no second call.
    app.load_path(a.clone());
    app.load_path(b.clone());
    assert_eq!(
        app.buffer.text(),
        tagged,
        "no second frontmatter block, live round trip"
    );
    // And a FRESH session reopening the now-tagged file also never re-tags
    // (the write-back gate is `frontmatter::detect`, not a one-shot flag).
    let app2 = app_on(Some(b.clone()), "/notes", Config::empty());
    assert_eq!(
        app2.buffer.text(),
        tagged,
        "a fresh session sees the tag and never re-fires"
    );
}

#[test]
fn load_path_preserves_a_clobber_notice_the_leaving_flush_just_raised() {
    // REGRESSION (code review nit): if the flush `load_path` runs on the
    // buffer being LEFT hits the autosave clobber guard (the file changed
    // on disk outside awl), the notice it raises must survive the switch
    // — the unconditional `self.notice = None` a few lines later used to
    // wipe it in the very same call, before a single frame ever rendered
    // it, so the user never learned their unsaved edit was held.
    use crate::fs::{FileSystem, InMemoryFs};
    let a = PathBuf::from("/notes/a.md");
    let b = PathBuf::from("/notes/b.md");
    let mem = InMemoryFs::new().with_file(&a, "A\n").with_file(&b, "B\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(a.clone()), "/notes", Config::empty());
    app.buffer.set_text("A edited\n");
    // Someone ELSE writes A behind awl's back before we switch away from it.
    std::thread::sleep(Duration::from_millis(2)); // distinct mtime
    mem.write(&a, b"external edit\n").unwrap();

    app.load_path(b.clone());

    assert_eq!(app.buffer.text(), "B\n", "the switch to B still happens");
    assert_eq!(
        mem.read_to_string(&a).unwrap(),
        "external edit\n",
        "the clobber guard held A's write — the external edit is intact"
    );
    assert_eq!(
        app.notice.as_deref(),
        Some(CLOBBER_NOTICE),
        "the notice raised while leaving A must survive into the switch, not vanish unseen"
    );
}

#[test]
fn scratch_stash_and_restore_round_trip() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let stash = crate::fs::scratch_stash_path();
    // A no-file launch, some typing, then a flush (idle/blur/quit all route here).
    let mut app = app_on(None, "/proj", Config::empty());
    app.buffer.set_text("brain dump\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&stash).unwrap(),
        "brain dump\n",
        "the scratch stashed"
    );
    assert!(
        !crate::history::list(&stash).is_empty(),
        "the persistent scratch grows its own timeline"
    );
    // A fresh no-argument launch RESTORES it: still path-less, still the
    // markdown-first scratch surface, not a note.
    let mut app2 = app_on(None, "/proj", Config::empty());
    assert_eq!(app2.buffer.text(), "brain dump\n", "the stash restores");
    assert!(
        app2.buffer.path().is_none(),
        "restored scratch stays path-less"
    );
    assert!(app2.buffer.is_markdown() && !app2.buffer.is_note());
    // The restore stamped the stash mtime, so a follow-up edit + flush is not
    // mistaken for a two-instance clobber.
    app2.buffer.set_text("brain dump\nmore\n");
    app2.autosave_flush();
    assert_eq!(mem.read_to_string(&stash).unwrap(), "brain dump\nmore\n");
    assert!(
        app2.notice.is_none(),
        "no false clobber notice after a restore"
    );
}

// ── SAVE-FEEDBACK round: scratch Save -> note, notice, dirty title marker ──

#[test]
fn convert_scratch_and_save_promotes_the_buffer_and_retires_the_stash() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let notes = PathBuf::from("/notes");
    // Stash an OLD scratch content first, exactly like a real prior session
    // would have — the very ghost-copy risk the round's own doc names.
    let stash = crate::fs::scratch_stash_path();
    mem.write(&stash, b"yesterday's dump\n").unwrap();

    let cfg = Config {
        notes_root: Some(notes.clone()),
        ..Config::empty()
    };
    let mut app = app_on(None, "/proj", cfg);
    assert_eq!(
        app.buffer.text(),
        "yesterday's dump\n",
        "restored from the stash first"
    );
    assert!(
        app.buffer.path().is_none() && !app.buffer.is_note(),
        "still a true scratch"
    );

    app.convert_scratch_and_save();

    assert!(
        app.buffer.is_note(),
        "Cmd-S promoted the scratch buffer into a note"
    );
    let p = app.buffer.path().unwrap().to_path_buf();
    assert!(
        p.starts_with(&notes),
        "the note landed under notes_root: {p:?}"
    );
    assert_eq!(mem.read_to_string(&p).unwrap(), "yesterday's dump\n");
    assert_eq!(
        app.file.as_deref(),
        Some(p.as_path()),
        "App.file tracks the new path"
    );
    assert_eq!(app.notice.as_deref(), Some("saved"));
    // THE STASH IS RETIRED: a later bare relaunch must never resurrect a
    // ghost copy of content that is now a real, named file.
    assert!(
        mem.read_to_string(&stash).is_err(),
        "the stash file was removed"
    );
}

#[test]
fn convert_scratch_and_save_second_save_is_a_plain_save() {
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let notes = PathBuf::from("/notes");
    let cfg = Config {
        notes_root: Some(notes),
        ..Config::empty()
    };
    let mut app = app_on(None, "/proj", cfg);
    app.buffer.set_text("first entry\n");
    app.convert_scratch_and_save();
    let named = app.buffer.path().unwrap().to_path_buf();

    // A SECOND explicit save (the buffer is now an ordinary note) must
    // NOT re-run the scratch-conversion machinery — same path, same file,
    // just the updated content. `Buffer::save()` here mirrors exactly
    // what `apply_core`'s `Action::Save` arm does before signalling
    // `Effect::SaveDone`; `finish_manual_save` is its post-save
    // bookkeeping half (see `app::apply`'s `Effect::SaveDone` arm).
    app.buffer.set_text("first entry\nmore\n");
    app.buffer.save().unwrap();
    app.finish_manual_save(true, "saved".to_string());
    assert_eq!(
        app.buffer.path().unwrap(),
        named,
        "no re-homing on the second save"
    );
    assert_eq!(mem.read_to_string(&named).unwrap(), "first entry\nmore\n");
}

#[test]
fn convert_scratch_and_save_unwritable_notes_root_raises_a_calm_notice_never_a_panic() {
    // A `notes_root` that can't be written to (a full disk, a permissions
    // error, …) must surface as the SAME calm notice a failed manual save
    // gets — never a terminal print, never a crash, and the scratch stash
    // is left untouched (nothing succeeded to retire it over).
    struct UnwritableFs;
    impl crate::fs::FileSystem for UnwritableFs {
        fn read_to_string(&self, _p: &std::path::Path) -> std::io::Result<String> {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "unwritable fake",
            ))
        }
        fn read(&self, _p: &std::path::Path) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "unwritable fake",
            ))
        }
        fn write(&self, _p: &std::path::Path, _d: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "notes_root unwritable",
            ))
        }
        fn create_dir_all(&self, _p: &std::path::Path) -> std::io::Result<()> {
            Ok(())
        }
        fn rename(&self, _f: &std::path::Path, _t: &std::path::Path) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "notes_root unwritable",
            ))
        }
        fn exists(&self, _p: &std::path::Path) -> bool {
            false
        }
        fn is_dir(&self, _p: &std::path::Path) -> bool {
            false
        }
        fn read_dir(&self, _p: &std::path::Path) -> std::io::Result<Vec<crate::fs::DirEntry>> {
            Ok(vec![])
        }
        fn metadata(&self, _p: &std::path::Path) -> std::io::Result<crate::fs::Metadata> {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "unwritable fake",
            ))
        }
        fn remove_file(&self, _p: &std::path::Path) -> std::io::Result<()> {
            Ok(())
        }
    }
    let _g = crate::fs::FsGuard::install(Arc::new(UnwritableFs));
    let notes = PathBuf::from("/notes");
    let cfg = Config {
        notes_root: Some(notes),
        ..Config::empty()
    };
    let mut app = app_on(None, "/proj", cfg);
    app.buffer.set_text("won't land\n");

    app.convert_scratch_and_save();

    assert!(
        app.notice
            .as_deref()
            .is_some_and(|n| n.starts_with("save failed:")),
        "a calm failure notice, not a panic: {:?}",
        app.notice
    );
}

// ── NOTES VERBS round: Rename note… / Duplicate note ──

#[test]
fn rename_current_file_happy_path_renames_disk_buffer_and_history() {
    use crate::fs::{FileSystem, InMemoryFs};
    let old = PathBuf::from("/notes/old.md");
    let mem = InMemoryFs::new().with_file(&old, "hi\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    // A prior snapshot exists under the OLD path — the ONE-OWNER rename must
    // carry it over so the timeline survives the rename.
    crate::history::record(&old, "hi\n", &Config::empty());
    assert!(
        !crate::history::list(&old).is_empty(),
        "arranged: a snapshot exists"
    );

    let mut app = app_on(Some(old.clone()), "/notes", Config::empty());
    assert_eq!(app.buffer.path(), Some(old.as_path()));

    app.rename_current_file("new.md");

    let new = PathBuf::from("/notes/new.md");
    assert_eq!(
        app.buffer.path(),
        Some(new.as_path()),
        "buffer follows the rename"
    );
    assert_eq!(
        app.file.as_deref(),
        Some(new.as_path()),
        "App.file follows the rename"
    );
    assert_eq!(mem.read_to_string(&new).unwrap(), "hi\n", "content moved");
    assert!(mem.read_to_string(&old).is_err(), "the old path is gone");
    assert_eq!(app.notice.as_deref(), Some("renamed to new.md"));
    // THE ONE-OWNER LAW: the history log followed too.
    assert!(
        !crate::history::list(&new).is_empty(),
        "history followed to the new path"
    );
    assert!(
        crate::history::list(&old).is_empty(),
        "nothing stranded under the old path"
    );
}

#[test]
fn rename_current_file_refuses_to_clobber_an_existing_name() {
    use crate::fs::{FileSystem, InMemoryFs};
    let old = PathBuf::from("/notes/old.md");
    let taken = PathBuf::from("/notes/taken.md");
    let mem = InMemoryFs::new()
        .with_file(&old, "old body\n")
        .with_file(&taken, "taken body\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(old.clone()), "/notes", Config::empty());

    app.rename_current_file("taken.md");

    assert_eq!(
        app.buffer.path(),
        Some(old.as_path()),
        "buffer stays put — refused, not clobbered"
    );
    assert_eq!(
        mem.read_to_string(&old).unwrap(),
        "old body\n",
        "old untouched"
    );
    assert_eq!(
        mem.read_to_string(&taken).unwrap(),
        "taken body\n",
        "never overwritten"
    );
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|n| n.contains("already a file named")),
        "a calm refusal notice: {:?}",
        app.notice
    );
}

#[test]
fn rename_current_file_refuses_a_git_managed_file() {
    use crate::fs::{FileSystem, InMemoryFs};
    let old = PathBuf::from("/proj/tracked.md");
    let mem = InMemoryFs::new()
        .with_file(&old, "body\n")
        .with_dir("/proj/.git");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(old.clone()), "/proj", Config::empty());

    app.rename_current_file("renamed.md");

    assert_eq!(
        app.buffer.path(),
        Some(old.as_path()),
        "a git-managed file never renames here"
    );
    assert!(mem.exists(&old), "old path untouched");
    assert!(
        !mem.exists(&PathBuf::from("/proj/renamed.md")),
        "no new file created"
    );
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|n| n.contains("git already tracks")),
        "a calm git-managed refusal notice: {:?}",
        app.notice
    );
}

#[test]
fn rename_current_file_unchanged_or_blank_name_is_a_quiet_no_op() {
    use crate::fs::InMemoryFs;
    let old = PathBuf::from("/notes/old.md");
    let mem = InMemoryFs::new().with_file(&old, "hi\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(old.clone()), "/notes", Config::empty());

    app.rename_current_file("old.md");
    assert_eq!(
        app.buffer.path(),
        Some(old.as_path()),
        "unchanged name: no-op"
    );
    assert!(app.notice.is_none(), "no notice for a no-op");

    app.rename_current_file("   ");
    assert_eq!(app.buffer.path(), Some(old.as_path()), "blank name: no-op");
    assert!(app.notice.is_none(), "no notice for a no-op");
}

#[test]
fn duplicate_current_file_dedups_the_name_and_starts_a_fresh_history_timeline() {
    use crate::fs::{FileSystem, InMemoryFs};
    let old = PathBuf::from("/notes/old.md");
    // A prior "old-2.md" already exists, so the dedup must land on "old-3.md".
    let taken2 = PathBuf::from("/notes/old-2.md");
    let mem = InMemoryFs::new()
        .with_file(&old, "on disk\n")
        .with_file(&taken2, "someone else's\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    // The old file has its own history timeline.
    crate::history::record(&old, "on disk\n", &Config::empty());
    assert!(
        !crate::history::list(&old).is_empty(),
        "arranged: old has history"
    );

    let mut app = app_on(Some(old.clone()), "/notes", Config::empty());
    // Simulate an UNSAVED edit: the duplicate must carry the LIVE buffer
    // content, not necessarily what's on disk.
    app.buffer.set_text("live edit, not yet flushed\n");

    app.duplicate_current_file();

    let dup = PathBuf::from("/notes/old-3.md");
    assert_eq!(
        app.buffer.path(),
        Some(dup.as_path()),
        "switched to the deduped sibling"
    );
    assert_eq!(app.file.as_deref(), Some(dup.as_path()));
    assert_eq!(
        mem.read_to_string(&dup).unwrap(),
        "live edit, not yet flushed\n",
        "the copy captures the buffer's LIVE content"
    );
    assert!(
        mem.exists(&old),
        "the original file is untouched, still present"
    );
    assert!(
        mem.exists(&taken2),
        "the pre-existing -2 sibling is never clobbered"
    );
    // FRESH HISTORY: the duplicate is a brand-new file, so its own timeline
    // starts empty, even though the SOURCE had history.
    assert!(
        crate::history::list(&dup).is_empty(),
        "the copy starts a fresh history timeline"
    );
    // The ORIGINAL buffer was PARKED (backgrounded), never discarded — its
    // pending edit is still flushed to disk (autosave_flush runs before the
    // dedup scan) and its live state survives in the registry.
    let key = crate::buffers::BufferKey::path(&old);
    assert!(
        app.buffer_registry.contains(&key),
        "the original was parked, not dropped"
    );
}

#[test]
fn duplicate_current_file_on_a_pathless_buffer_is_a_quiet_no_op() {
    // HERMETIC: install an InMemoryFs before `App::new` so this test never
    // touches the machine's real `session.toml`/stash (`app_on(None, ..)`
    // runs the full App startup). FsGuard also holds `testlock::serial()`
    // for the test's life, so the pass no longer rides ordering luck.
    use crate::fs::InMemoryFs;
    let mem = InMemoryFs::new().with_dir("/proj");
    let _g = crate::fs::FsGuard::install(Arc::new(mem));
    let mut app = app_on(None, "/proj", Config::empty());
    assert!(app.buffer.path().is_none());
    app.duplicate_current_file();
    assert!(app.buffer.path().is_none(), "nothing to duplicate yet");
    assert!(app.notice.is_none());
}

#[test]
fn finish_manual_save_ok_is_silent_failure_notices_the_error() {
    // SAVE-UX round: a SUCCESSFUL manual save raises NO bottom-center notice
    // (autosave is already silent; a lone non-fading "saved" is just noise).
    // A FAILURE still surfaces its error — errors must never go silent.
    use crate::fs::InMemoryFs;
    let _l = crate::testlock::serial();
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());

    app.finish_manual_save(true, "saved".to_string());
    assert_eq!(app.notice.as_deref(), Some("saved"));
    assert_eq!(app.notice_kind, NoticeKind::Toast);
    assert!(
        app.notice_expires_at.is_none(),
        "a headless test never arms a live timer"
    );

    app.finish_manual_save(false, "save failed: disk full".to_string());
    assert_eq!(app.notice.as_deref(), Some("save failed: disk full"));
}

#[test]
fn finish_manual_save_clears_a_notes_dirty_marker_immediately() {
    // BUG LOCK-DOWN: `is_document_dirty` reads `autosave_saved_version` for a
    // NOTE, but `finish_manual_save` used to stamp only `doc_saved_version`
    // — so ⌘S on a note left it reading dirty (the title `•` + native
    // titlebar dot lingering) until the note's ~400ms debounced autosave
    // redundantly rewrote and finally stamped the field.
    use crate::fs::InMemoryFs;
    let _l = crate::testlock::serial();
    let notes = PathBuf::from("/notes");
    let mem = InMemoryFs::new().with_dir(&notes);
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(None, "/notes", Config::empty());

    // Make the active buffer a NOTE with content, then write it to disk the
    // way `apply_core`'s `Action::Save` arm does before signalling SaveDone.
    app.buffer.start_note(notes.clone());
    app.buffer.set_text("note body\n");
    app.buffer.save().unwrap();
    assert!(
        app.buffer.is_note() && app.buffer.path().is_some(),
        "arranged: a saved note"
    );
    // Pre-bookkeeping the note reads DIRTY: `autosave_saved_version` is still
    // stale (None) against the edited version.
    assert!(
        app.is_document_dirty(),
        "arranged: the note reads dirty pre-bookkeeping"
    );

    app.finish_manual_save(true, "saved".to_string());

    assert!(
        !app.is_document_dirty(),
        "a note is clean IMMEDIATELY after ⌘S, not ~400ms later"
    );
    assert!(
        app.autosave_dirty_at.is_none(),
        "the redundant ~400ms note rewrite is suppressed"
    );
}

#[test]
fn finish_manual_save_clears_a_regular_files_dirty_marker_immediately() {
    // REGRESSION GUARD: a path-backed file reads `doc_saved_version` in
    // `is_document_dirty` — it was always fine, and must stay fine.
    use crate::fs::InMemoryFs;
    let _l = crate::testlock::serial();
    let p = PathBuf::from("/proj/doc.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/proj", Config::empty());

    app.buffer.set_text("edited body\n");
    app.buffer.save().unwrap();
    assert!(
        !app.buffer.is_note() && app.buffer.path().is_some(),
        "arranged: a saved file"
    );
    assert!(
        app.is_document_dirty(),
        "arranged: the file reads dirty pre-bookkeeping"
    );

    app.finish_manual_save(true, "saved".to_string());

    assert!(
        !app.is_document_dirty(),
        "a regular file is clean immediately after ⌘S"
    );
}

// ── SAVE-FEEDBACK round: the ambient dirty title marker ──

#[test]
fn sync_view_retitles_only_on_an_actual_dirty_flip() {
    // HERMETIC: install an InMemoryFs before `App::new` so this test never
    // touches the machine's real `session.toml`/stash (`app_on(None, ..)` runs
    // the full App startup, including the scratch-stash restore). FsGuard also
    // holds `testlock::serial()` for the test's life — without it this
    // uninstalled `app_on(None, ..)` read the PROCESS-GLOBAL active fs, so under
    // CI parallelism it raced INTO a concurrent test's installed InMemoryFs and,
    // finding that test's deliberately-corrupt scratch stash, preserved a second
    // `.corrupt-*` sibling into it — the deterministic CI-only failure of
    // `scratch_stash_invalid_utf8_preserves_a_corrupt_sibling_then_starts_a_blank_scratch`
    // (mirrors `duplicate_current_file_on_a_pathless_buffer_is_a_quiet_no_op`).
    use crate::fs::InMemoryFs;
    let mem = InMemoryFs::new().with_dir("/proj");
    let _g = crate::fs::FsGuard::install(Arc::new(mem));
    let mut app = app_on(None, "/proj", Config::empty());
    assert!(!app.title_dirty, "a fresh scratch buffer starts clean");
    // No gpu/window in a hermetic App: `sync_view` bails before the title
    // comparison (its own gpu-present gate) — this proves the flip-tracking
    // logic itself is reachable + correct via `is_document_dirty` directly,
    // mirroring `update_title_uses_the_same_pure_window_title`'s own
    // "no live window, still exercised" shape.
    assert!(!app.is_document_dirty(), "just-loaded content starts saved");
    app.buffer.set_text("edited\n");
    assert!(
        app.is_document_dirty(),
        "an edit past the saved version is dirty"
    );
}

#[test]
fn is_document_dirty_clears_on_autosave_not_just_manual_save() {
    // The definition this round settled on for the title's dirty marker:
    // "unsaved" by the SAME version-vs-saved-version bookkeeping the
    // autosave engine tracks — so an AUTOSAVED (not manually Cmd-S'd)
    // document reads as clean too, never stuck showing the edited marker
    // on content that's already safely on disk.
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/notes/draft.md");
    let mem = InMemoryFs::new().with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/notes", Config::empty());
    assert!(!app.is_document_dirty());
    app.buffer.set_text("v2\n");
    assert!(app.is_document_dirty(), "an unsaved edit reads dirty");
    app.autosave_flush(); // NOT a manual save — the background engine
    assert_eq!(mem.read_to_string(&p).unwrap(), "v2\n");
    assert!(
        !app.is_document_dirty(),
        "autosave clears the dirty marker too"
    );
}

#[test]
fn scratch_stash_invalid_utf8_preserves_a_corrupt_sibling_then_starts_a_blank_scratch() {
    // DATA-SAFETY HARDENING: the scratch stash IS a manuscript, so a
    // stash file that's PRESENT but fails to decode as UTF-8 text (real
    // disk corruption, never a bug write_atomic itself can produce) must
    // never be silently discarded — a `.corrupt-*` sibling preserves the
    // raw bytes BEFORE `App::new` falls back to a blank scratch buffer.
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let stash = crate::fs::scratch_stash_path();
    // Invalid UTF-8: a lone continuation byte can never decode.
    mem.write(&stash, &[0x2E, 0x62, 0xFF, 0xFE, 0x0A]).unwrap();

    let app = app_on(None, "/proj", Config::empty());
    assert_eq!(
        app.buffer.text(),
        "",
        "an undecodable stash falls back to a blank scratch"
    );
    assert!(app.buffer.path().is_none());

    let dir = stash.parent().unwrap();
    let names: Vec<String> = mem
        .read_dir(dir)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    let stash_name = stash.file_name().unwrap().to_string_lossy().into_owned();
    let backup_prefix = format!("{stash_name}.corrupt-");
    let backups: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with(&backup_prefix))
        .collect();
    assert_eq!(
        backups.len(),
        1,
        "exactly one corrupt sibling preserved: {names:?}"
    );
    let backup_bytes = mem.read(&dir.join(backups[0])).unwrap();
    assert_eq!(
        backup_bytes,
        vec![0x2E, 0x62, 0xFF, 0xFE, 0x0A],
        "the sibling holds the ORIGINAL undecodable bytes verbatim"
    );
}

#[test]
fn blur_flush_never_reloads_buffer_or_resets_cursor() {
    // WEB STRESS-TEST HYPOTHESIS (characterized, not reproduced): a Playwright
    // run typing "AAA" then, in a LATER dispatch batch, "BBB" observed BBB
    // landing at buffer position 0 instead of after "AAA", as if a blur/
    // visibility flap between the two batches made the web build RE-LOAD the
    // scratch from its localStorage stash mid-session (which would restore
    // the STASHED content and reset the cursor to 0 — restoring a buffer
    // always starts a fresh Buffer at cursor 0, see `App::new`).
    //
    // `WindowEvent::Focused(false)` is the one live door a blur reaches —
    // and it calls exactly `App::autosave_flush` (`app.rs`'s `Focused(false)`
    // arm), which fans out to `stash_scratch_now` for a no-path scratch. That
    // function is a pure WRITE: it reads `self.buffer.text()` and writes it
    // OUT to the stash path; it never calls `crate::fs::active().read_*` or
    // reconstructs `self.buffer`. The ONLY place a stash is ever read back
    // INTO a buffer is `App::new` (a true process/page (re)launch) — never a
    // blur, never any other live-App path. This test pins that down: typing
    // "AAA", flushing (the blur trigger) as many times as a stress test's
    // spurious focus flapping might, then typing "BBB" must land the cursor
    // right after "AAA", not at 0.
    use crate::fs::InMemoryFs;
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(None, "/proj", Config::empty());
    for c in "AAA".chars() {
        app.buffer.insert_char(c);
    }
    assert_eq!(
        app.buffer.cursor_char(),
        3,
        "cursor sits after the typed AAA"
    );
    // Simulate the exact call the live `Focused(false)` arm makes — as many
    // times as a flappy test harness might re-fire it between dispatches.
    app.autosave_flush();
    app.autosave_flush();
    app.autosave_flush();
    assert_eq!(
        app.buffer.text(),
        "AAA",
        "a blur-driven flush never reloads content"
    );
    assert_eq!(
        app.buffer.cursor_char(),
        3,
        "a blur-driven flush never resets the cursor — only App::new restores"
    );
    // A later "dispatch batch" continues typing from exactly where it left off.
    for c in "BBB".chars() {
        app.buffer.insert_char(c);
    }
    assert_eq!(
        app.buffer.text(),
        "AAABBB",
        "BBB lands after AAA, not at position 0"
    );
    assert_eq!(app.buffer.cursor_char(), 6);
}

#[test]
fn scratch_restore_skips_empty_stash() {
    use crate::fs::{FileSystem, InMemoryFs};
    // An EMPTY stash restores nothing (plain scratch)… (each half owns its
    // FsGuard — the guard holds the process-wide FS lock, so they must not
    // overlap on one thread.)
    {
        let mem = InMemoryFs::new();
        let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
        mem.write(&crate::fs::scratch_stash_path(), b"").unwrap();
        let app = app_on(None, "/proj", Config::empty());
        assert!(app.buffer.text().is_empty(), "empty stash → plain scratch");
    }
    // …and so does a MISSING one (fresh fake).
    {
        let _g = crate::fs::FsGuard::install(Arc::new(InMemoryFs::new()));
        let app = app_on(None, "/proj", Config::empty());
        assert!(
            app.buffer.text().is_empty(),
            "missing stash → plain scratch"
        );
    }
}

#[test]
fn autosave_writes_git_files_but_never_snapshots_them() {
    // LOCKED DECISION 4, both halves at the App seam: autosave still WRITES
    // a git-managed file (writing is not version-meddling), but records NO
    // awl snapshot for it — its timeline stays git log alone.
    use crate::fs::{FileSystem, InMemoryFs};
    let p = PathBuf::from("/repo/doc.md");
    let mem = InMemoryFs::new()
        .with_dir("/repo/.git")
        .with_file(&p, "v1\n");
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let mut app = app_on(Some(p.clone()), "/repo", Config::empty());
    app.buffer.set_text("v2\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&p).unwrap(),
        "v2\n",
        "autosave still WRITES a git-managed file"
    );
    assert!(app.notice.is_none(), "a clean write raises no notice");
    // The snapshot store never grew a log dir — the record gate held.
    let store = crate::fs::data_root().join("history");
    assert!(
        mem.read_dir(&store).map(|v| v.is_empty()).unwrap_or(true),
        "no awl snapshot log for a git-managed file"
    );
}

#[test]
fn scratch_stash_clobber_guard_holds_two_instance_writes() {
    // TWO-INSTANCE SAFETY: another awl (or anything) writes the stash after
    // this instance launched — the flush HOLDS (the external stash content
    // survives) and raises the same calm notice as the document guard.
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let stash = crate::fs::scratch_stash_path();
    let mut app = app_on(None, "/proj", Config::empty());
    mem.write(&stash, b"the other instance's dump\n").unwrap();
    app.buffer.set_text("mine\n");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&stash).unwrap(),
        "the other instance's dump\n",
        "the stash write is held — external content survives"
    );
    assert_eq!(
        app.notice.as_deref(),
        Some(CLOBBER_NOTICE),
        "the calm notice names the hold"
    );
}

#[test]
fn emptied_scratch_clears_the_stale_stash() {
    // The stash writes EVEN EMPTY text: emptying the restored scratch and
    // flushing must clear yesterday's dump, or a deliberately-emptied
    // scratch would resurrect on the next launch.
    use crate::fs::{FileSystem, InMemoryFs};
    let mem = InMemoryFs::new();
    let _g = crate::fs::FsGuard::install(Arc::new(mem.clone()));
    let stash = crate::fs::scratch_stash_path();
    mem.write(&stash, b"yesterday's dump\n").unwrap();
    let mut app = app_on(None, "/proj", Config::empty());
    assert_eq!(
        app.buffer.text(),
        "yesterday's dump\n",
        "the stash restored"
    );
    app.buffer.set_text("");
    app.autosave_flush();
    assert_eq!(
        mem.read_to_string(&stash).unwrap(),
        "",
        "an emptied scratch clears the stale stash"
    );
    assert!(
        app.notice.is_none(),
        "our own restore is not an external edit"
    );
}
