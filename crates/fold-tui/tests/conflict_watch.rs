use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fold_tui::app::App;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn app_with(text: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), text).unwrap();
    let app = App::new(dir.path()).unwrap();
    (dir, app)
}

fn conflict_app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- [ ] task\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n- [x] task\n",
    )
    .unwrap();
    let mut v = fold_core::vault::Vault::open(dir.path()).unwrap();
    fold_core::merge::merge_sync_conflicts(&mut v, false).unwrap();
    drop(v);
    let app = App::new(dir.path()).unwrap();
    (dir, app)
}

#[test]
fn conflict_view_resolves_keep_ours() {
    let (_d, mut app) = conflict_app();
    app.enter_conflict_view();
    app.key_conflict_pub(key(KeyCode::Char('o')));
    // conflict resolved, ours kept
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(root.contains("- [ ] task"), "{}", root);
    let files: Vec<_> = std::fs::read_dir(app.vault_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".md") && n != "root.md")
        .collect();
    assert!(files.is_empty(), "{:?}", files);
}

#[test]
fn conflict_view_resolves_keep_theirs() {
    let (_d, mut app) = conflict_app();
    app.enter_conflict_view();
    app.key_conflict_pub(key(KeyCode::Char('t')));
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(root.contains("- [x] task"), "{}", root);
}

#[test]
fn conflict_view_resolves_keep_both() {
    let (_d, mut app) = conflict_app();
    app.enter_conflict_view();
    app.key_conflict_pub(key(KeyCode::Char('b')));
    // both stay; no conflict: key anywhere
    let files: Vec<_> = std::fs::read_dir(app.vault_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".md"))
        .collect();
    assert_eq!(files.len(), 2, "{:?}", files);
    for f in &files {
        let text = std::fs::read_to_string(app.vault_dir().join(f)).unwrap();
        assert!(!text.contains("conflict:"), "{}: {}", f, text);
    }
}

#[test]
fn reading_pane_toggle_task_at_cursor() {
    let (_d, mut app) = app_with("# A\n\n- [ ] do it\n");
    // zoom into A (reading focus)
    app.key_normal(key(KeyCode::Enter));
    // reading cursor to the task line (line 0 is "# A", 1 blank, 2 task)
    app.key_reading_pub(key(KeyCode::Char('j')));
    app.key_reading_pub(key(KeyCode::Char('j')));
    app.key_reading_pub(key(KeyCode::Char('x')));
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(root.contains("- [x] do it"), "{}", root);
}

#[test]
fn reading_search_jumps_to_match() {
    let (_d, mut app) = app_with("# A\n\nalpha\n\nbeta\n\ngamma\n");
    app.key_normal(key(KeyCode::Enter));
    app.key_reading_pub(key(KeyCode::Char('/')));
    for c in "gamma".chars() {
        app.key_prompt(key(KeyCode::Char(c)));
    }
    app.key_prompt(key(KeyCode::Enter));
    assert_eq!(app.read_cursor_pub(), 6, "cursor on the gamma line");
    app.key_reading_pub(key(KeyCode::Char('N')));
    // only one match; stays
    assert_eq!(app.read_cursor_pub(), 6);
}

#[test]
fn reading_heading_jump() {
    let (_d, mut app) = app_with("# A\n\ntext\n\n## B\n\nmore\n\n## C\n");
    app.show_reading = true;
    app.key_normal(key(KeyCode::Enter));
    // a single `]` does nothing (§10.4: `]]`)
    app.handle_key(key(KeyCode::Char(']')));
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.read_cursor_pub(), 0);
    let doc = app.reading_doc_pub();
    let b_line = doc.lines.iter().position(|l| l.contains("## B")).unwrap();
    let c_line = doc.lines.iter().position(|l| l.contains("## C")).unwrap();
    app.handle_key(key(KeyCode::Char(']')));
    app.handle_key(key(KeyCode::Char(']')));
    assert_eq!(app.read_cursor_pub(), b_line);
    app.handle_key(key(KeyCode::Char(']')));
    app.handle_key(key(KeyCode::Char(']')));
    assert_eq!(app.read_cursor_pub(), c_line);
    app.handle_key(key(KeyCode::Char('[')));
    app.handle_key(key(KeyCode::Char('[')));
    assert_eq!(app.read_cursor_pub(), b_line);
}

#[test]
fn reading_enter_zooms_section() {
    let (_d, mut app) = app_with("# A\n\n## B\n\nbody of b\n");
    app.key_normal(key(KeyCode::Enter)); // zoom A, focus reading
    app.key_reading_pub(key(KeyCode::Char('j')));
    app.key_reading_pub(key(KeyCode::Char('j'))); // onto "## B"
    app.key_reading_pub(key(KeyCode::Enter)); // zoom into B
    // now the reading doc is just B
    let doc = app.reading_doc_pub();
    assert!(doc.lines[0].contains("# B"), "{:?}", doc.lines);
}

#[test]
fn watcher_detects_external_change() {
    let (_d, mut app) = app_with("# A\n\n- one\n");
    app.start_watcher();
    std::thread::sleep(std::time::Duration::from_millis(500));
    // external edit
    std::fs::write(app.vault_dir().join("root.md"), "# A\n\n- one\n- two\n").unwrap();
    // poll until the debounced event surfaces
    let mut seen = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(300));
        if app.poll_watcher() {
            seen = true;
            break;
        }
    }
    assert!(seen, "watcher event received");
    app.reload_external();
    let rows = app.rows();
    let titles: Vec<String> = rows.iter().map(|r| app.title_of(r.nref)).collect();
    assert!(titles.contains(&"two".to_string()), "{:?}", titles);
}

#[test]
fn startup_merge_enters_conflict_view() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- [ ] task\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n- [x] task\n",
    )
    .unwrap();
    let mut app = App::new(dir.path()).unwrap();
    // simulate the run() startup path
    if let Ok(files) = app.vault_conflict_files() {
        if !files.is_empty() {
            fold_core::merge::merge_sync_conflicts(app.vault_mut(), false).unwrap();
            app.enter_conflict_view();
        }
    }
    assert_eq!(app.mode_pub(), "conflict");
}

#[test]
fn conflict_keys_act_on_the_shown_pair_after_resolving_the_last() {
    let pairs = |app: &mut App| fold_core::merge::conflict_pairs(app.vault_mut()).len();
    for resolve in ['o', 't', 'b'] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("root.md"), "# A\n\n- [ ] t1\n- [ ] t2\n").unwrap();
        std::fs::write(
            dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
            "# A\n\n- [x] t1\n- [x] t2\n",
        )
        .unwrap();
        let mut v = fold_core::vault::Vault::open(dir.path()).unwrap();
        fold_core::merge::merge_sync_conflicts(&mut v, false).unwrap();
        drop(v);
        let mut app = App::new(dir.path()).unwrap();
        assert_eq!(pairs(&mut app), 2);
        app.enter_conflict_view();
        // n: pair 2 of 2; resolving it leaves "Conflict 1 of 1" on screen
        app.key_conflict_pub(key(KeyCode::Char('n')));
        app.key_conflict_pub(key(KeyCode::Char(resolve)));
        assert_eq!(pairs(&mut app), 1, "after {}", resolve);
        // §10.7: o acts on the pair that is shown
        app.key_conflict_pub(key(KeyCode::Char('o')));
        assert_eq!(pairs(&mut app), 0, "o after {} did nothing", resolve);
    }
}

#[test]
fn editing_a_conflict_pair_before_the_zoom_keeps_the_zoom() {
    let titles = |app: &App| -> Vec<String> { app.rows().iter().map(|r| app.title_of(r.nref)).collect() };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "- [ ] task\n- Z\n  - z1\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "- [x] task\n- Z\n  - z1\n",
    )
    .unwrap();
    let mut v = fold_core::vault::Vault::open(dir.path()).unwrap();
    fold_core::merge::merge_sync_conflicts(&mut v, false).unwrap();
    drop(v);
    let mut app = App::new(dir.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    let z = titles(&app).iter().position(|t| t == "Z").unwrap();
    for _ in 0..z {
        app.handle_key(key(KeyCode::Char('j')));
    }
    app.handle_key(key(KeyCode::Enter)); // zoom into Z
    assert_eq!(titles(&app), ["Z", "z1"]);
    // §10.7 `e` edits ours, above Z in the same file: a new item after it
    app.enter_conflict_view();
    app.handle_key(key(KeyCode::Char('e')));
    assert_eq!(app.mode_pub(), "edit");
    app.handle_key(key(KeyCode::End));
    app.handle_key(key(KeyCode::Enter));
    for c in "- new".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Esc));
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.starts_with("- [ ] task\n- new\n"), "{}", root);
    assert_eq!(titles(&app), ["Z", "z1"]);
}

#[test]
fn a_conflict_copy_the_merge_leaves_alone_does_not_reopen_the_conflict_view() {
    // notes.md has no id: an ignored file, whose copy merge leaves alone
    let (d, mut app) = app_with("# A\n\nbody\n");
    std::fs::write(d.path().join("notes.md"), "mine\n").unwrap();
    std::fs::write(d.path().join("notes.sync-conflict-20260912-100000-phone.md"), "theirs\n").unwrap();
    app.handle_key(key(KeyCode::Char('e')));
    assert_eq!(app.mode_pub(), "edit");
    app.reload_external();
    assert_eq!(app.mode_pub(), "edit");
    app.handle_key(key(KeyCode::Esc));
    app.reload_external();
    assert_eq!(app.mode_pub(), "normal");
    // nor do pairs left unresolved (§12.5): only pairs the merge raises
    let (d, mut app) = conflict_app();
    assert_eq!(fold_core::merge::conflict_pairs(app.vault_mut()).len(), 1);
    std::fs::write(d.path().join("notes.md"), "mine\n").unwrap();
    std::fs::write(d.path().join("notes.sync-conflict-20260912-100000-phone.md"), "theirs\n").unwrap();
    app.reload_external();
    assert_eq!(app.mode_pub(), "normal");
    // a copy that raises a pair opens it
    let text = std::fs::read_to_string(d.path().join("root.md")).unwrap();
    std::fs::write(d.path().join("root.md"), format!("{}\n# B\n\nmine\n", text)).unwrap();
    std::fs::write(
        d.path().join("root.sync-conflict-20260913-100000-phone.md"),
        format!("{}\n# B\n\ntheirs\n", text),
    )
    .unwrap();
    app.reload_external();
    assert_eq!(fold_core::merge::conflict_pairs(app.vault_mut()).len(), 2);
    assert_eq!(app.mode_pub(), "conflict");
}
