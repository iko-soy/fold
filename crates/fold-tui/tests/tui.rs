use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fold_tui::app::App;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn app_with(text: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), text).unwrap();
    let mut app = App::new(dir.path()).unwrap();
    // these tests are about the reading pane, hidden by default
    app.show_reading = true;
    (dir, app)
}

fn screen(app: &mut App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn outline_shows_tree_and_counts() {
    let (_d, mut app) = app_with("# Homelab\n\n## NAS\n\n- [ ] task one\n- [x] done one\n\n# Inbox\n");
    let s = screen(&mut app, 100, 24);
    assert!(s.contains("Homelab"), "{}", s);
    assert!(s.contains("NAS"), "{}", s);
    assert!(s.contains("task one"), "{}", s);
    assert!(s.contains("Inbox"), "{}", s);
    assert!(s.contains("☐"), "{}", s);
}

#[test]
fn cursor_moves_and_zooms() {
    let (_d, mut app) = app_with("# A\n\n## B\n\n# C\n");
    let rows = app.rows();
    app.key_normal(key(KeyCode::Char('j')));
    assert_eq!(app.cursor, 1);
    app.key_normal(key(KeyCode::Enter)); // zoom into B
    let s = screen(&mut app, 100, 24);
    assert!(s.contains("B"), "{}", s);
    let _ = rows;
}

#[test]
fn toggle_task_via_key() {
    let (_d, mut app) = app_with("# A\n\n- [ ] do it\n");
    app.key_normal(key(KeyCode::Char('j'))); // to the task
    app.key_normal(key(KeyCode::Char('x')));
    let text = std::fs::read_to_string(
        app_vault_path(&app).join("root.md"),
    )
    .unwrap();
    assert!(text.contains("- [x] do it"), "{}", text);
}

#[test]
fn editor_edits_and_esc_saves() {
    let (_d, mut app) = app_with("# A\n\nold body\n");
    app.key_normal(key(KeyCode::Char('e')));
    // move to the body line and change it
    app.key_normal(key(KeyCode::Down)); // blank
    app.key_normal(key(KeyCode::Down)); // body
    // select whole line: delete chars then type
    for _ in 0..8 {
        app.key_edit(key(KeyCode::Backspace));
    }
    for c in "fresh text".chars() {
        app.key_edit(key(KeyCode::Char(c)));
    }
    app.key_edit(key(KeyCode::Esc));
    let text = std::fs::read_to_string(app_vault_path(&app).join("root.md")).unwrap();
    assert!(text.contains("fresh text"), "{}", text);
}

#[test]
fn filter_finds_titles() {
    let (_d, mut app) = app_with("# Alpha\n\n# Beta\n\n# Gamma\n");
    app.key_normal(key(KeyCode::Char('/')));
    for c in "beta".chars() {
        app.key_filter(key(KeyCode::Char(c)));
    }
    app.key_filter(key(KeyCode::Enter));
    let s = screen(&mut app, 100, 24);
    assert!(s.contains("Beta"), "{}", s);
}

#[test]
fn palette_runs_actions() {
    let (_d, mut app) = app_with("# A\n\n- [x] finished\n");
    app.key_normal(key(KeyCode::Char(':')));
    for c in "clear".chars() {
        app.key_palette(key(KeyCode::Char(c)));
    }
    app.key_palette(key(KeyCode::Enter));
    let text = std::fs::read_to_string(app_vault_path(&app).join("root.md")).unwrap();
    assert!(!text.contains("finished"), "{}", text);
}

#[test]
fn props_editor_sets_due_and_makes_block() {
    let (_d, mut app) = app_with("# A\n\n- plain task\n");
    app.key_normal(key(KeyCode::Char('j')));
    app.key_normal(key(KeyCode::Char('a'))); // props form
    app.key_props(key(KeyCode::Char('n'))); // new key prompt
    for c in "due".chars() {
        app.key_prompt(key(KeyCode::Char(c)));
    }
    app.key_prompt(key(KeyCode::Enter)); // opens value prompt
    for c in "2026-09-20".chars() {
        app.key_prompt(key(KeyCode::Char(c)));
    }
    app.key_prompt(key(KeyCode::Enter));
    // a block file appeared with the due date
    let dir = app_vault_path(&app);
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".md") && n != "root.md")
        .collect();
    assert_eq!(files.len(), 1, "{:?}", files);
    let text = std::fs::read_to_string(dir.join(&files[0])).unwrap();
    assert!(text.contains("due: 2026-09-20"), "{}", text);
}

#[test]
fn undo_restores_delete() {
    let (_d, mut app) = app_with("# A\n\n- keep me\n");
    app.key_normal(key(KeyCode::Char('j')));
    app.key_normal(key(KeyCode::Char('d')));
    let text = std::fs::read_to_string(app_vault_path(&app).join("root.md")).unwrap();
    assert!(!text.contains("keep me"));
    app.key_normal(key(KeyCode::Char('u')));
    let text = std::fs::read_to_string(app_vault_path(&app).join("root.md")).unwrap();
    assert!(text.contains("keep me"), "{}", text);
}

fn app_vault_path(app: &App) -> std::path::PathBuf {
    app.vault_dir()
}

#[test]
fn question_mark_opens_help() {
    let (_d, mut app) = app_with("# A\n");
    app.key_normal(key(KeyCode::Char('?')));
    assert_eq!(app.mode_pub(), "help");
    let s = screen(&mut app, 100, 40);
    assert!(s.contains("Help") && s.contains("MOUSE"), "{}", s);
    assert!(s.contains("right-click"), "{}", s);
    assert!(s.contains("make block"), "{}", s);
    // Esc closes
    app.key_normal(key(KeyCode::Esc));
    // Esc in help mode is handled by the dispatcher, not key_normal; simulate:
    // (the real loop routes Help mode keys) — here we just re-open via palette
    app.key_normal(key(KeyCode::Char(':')));
    for c in "help".chars() {
        app.key_palette(key(KeyCode::Char(c)));
    }
    app.key_palette(key(KeyCode::Enter));
    assert_eq!(app.mode_pub(), "help");
}

#[test]
fn code_blocks_are_highlighted_by_their_language() {
    let (_d, mut app) = app_with("# A\n\n```rust\nfn main() {}\n```\n\n~~~unknown\nfn main() {}\n~~~\n");
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    // find each `fn` in the reading pane (right of the outline)
    let mut fns = Vec::new();
    for y in 0..20 {
        for x in 34..97 {
            if buf[(x, y)].symbol() == "f" && buf[(x + 1, y)].symbol() == "n" && buf[(x + 2, y)].symbol() == " " {
                fns.push(buf[(x, y)].fg);
            }
        }
    }
    assert_eq!(fns.len(), 2, "both blocks drawn");
    assert_eq!(fns[0], ratatui::style::Color::Magenta, "rust keyword highlighted");
    assert_eq!(fns[1], ratatui::style::Color::Yellow, "unknown language: plain code colour");
}

#[test]
fn fences_are_drawn_where_the_parser_reads_them() {
    // code breaks hard, marked with ↪, in the reading pane and the editor
    // alike; prose wraps at spaces (§10.1)
    let long = "Mirrored pairs, no raidz. Snapshots hourly via sanoid and pruned daily, replicated offsite every night.";
    let code_in = |text: &str| -> (bool, bool) {
        let (_d, mut app) = app_with(text);
        app.set_edit_keys(fold_tui::app::EditKeys::Normal);
        let reading = screen(&mut app, 100, 24).contains('↪');
        app.handle_key(key(KeyCode::Char('e')));
        assert_eq!(app.mode_pub(), "edit");
        (reading, screen(&mut app, 100, 24).contains('↪'))
    };
    // §3.3: a backtick fence's info string holds no backtick, so this is a
    // paragraph with inline code, and what follows is prose
    assert_eq!(code_in(&format!("# A\n\n```npm i``` first\n{}\n", long)), (false, false));
    // a closing fence has nothing after it: "``` x" is code, not a closer
    assert_eq!(code_in(&format!("# A\n\n```\n``` x\n{}\n```\n", long)), (true, true));
    // nor is a shorter run of the same character
    assert_eq!(code_in(&format!("# A\n\n~~~~\n~~~\n{}\n~~~~\n", long)), (true, true));
    // the line with inline code is styled as text, not as a fence (whose
    // info string is in the accent colour)
    let (_d, mut app) = app_with("# A\n\n```npm i``` first\n");
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let (x, y) = (0..20u16)
        .flat_map(|y| (34..90u16).map(move |x| (x, y)))
        .find(|&(x, y)| (0..5).map(|i| buf[(x + i, y)].symbol()).collect::<String>() == "npm i")
        .expect("drawn");
    assert_ne!(buf[(x, y)].fg, ratatui::style::Color::Cyan, "styled as a fence's info string");
}

#[test]
fn the_conflict_view_draws_fenced_code_as_code() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "- [ ] task\n  ```sh\n  # a comment\n  ```\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "- [x] task\n  ```sh\n  # a comment\n  ```\n",
    )
    .unwrap();
    let mut v = fold_core::vault::Vault::open(dir.path()).unwrap();
    fold_core::merge::merge_sync_conflicts(&mut v, false).unwrap();
    drop(v);
    let mut app = App::new(dir.path()).unwrap();
    app.enter_conflict_view();
    assert_eq!(app.mode_pub(), "conflict");
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let at: Vec<(u16, u16)> = (0..20u16)
        .flat_map(|y| (0..92u16).map(move |x| (x, y)))
        .filter(|&(x, y)| (0..9).map(|i| buf[(x + i, y)].symbol()).collect::<String>() == "a comment")
        .collect();
    assert_eq!(at.len(), 2, "both sides drawn");
    for (x, y) in at {
        // code, not a heading
        assert_eq!(buf[(x, y)].fg, ratatui::style::Color::Yellow);
        assert!(!buf[(x, y)].modifier.contains(ratatui::style::Modifier::BOLD));
    }
}
