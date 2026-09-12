use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use fold_tui::app::App;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn toolbar_labels(app: &mut App) -> Vec<String> {
    draw_once(app, 160, 30);
    app.toolbar_labels()
}

fn app_with(text: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), text).unwrap();
    let app = App::new(dir.path()).unwrap();
    (dir, app)
}

fn draw_once(app: &mut App, w: u16, h: u16) {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
}

fn click(x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

fn toolbar_click(app: &mut App, label: &str) {
    draw_once(app, 160, 30);
    let (x, y) = app.toolbar_pos(label).unwrap_or_else(|| panic!("no button {}", label));
    app.handle_mouse(click(x, y));
}

#[test]
fn toolbar_done_toggles_task() {
    let (_d, mut app) = app_with("# A\n\n- [ ] do it\n");
    app.enable_mouse();
    // select the task row first (row 1)
    draw_once(&mut app, 160, 30);
    app.handle_mouse(click(10, 2));
    toolbar_click(&mut app, "Done");
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(root.contains("- [x] do it"), "{}", root);
}

#[test]
fn toolbar_edit_and_keyboard_types() {
    let (_d, mut app) = app_with("# A\n\nold\n");
    app.enable_mouse();
    toolbar_click(&mut app, "Edit");
    assert_eq!(app.mode_pub(), "edit");
    // click on the on-screen 'w' key, then Enter, then Close
    draw_once(&mut app, 160, 30);
    let (wx, wy) = app.kb_pos('w').expect("w key");
    app.handle_mouse(click(wx, wy));
    let root_before = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(!root_before.contains("w"));
    toolbar_click(&mut app, "Save");
    toolbar_click(&mut app, "Close");
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(root.contains('w'), "{}", root);
}

#[test]
fn toolbar_capture_prompt_via_keyboard() {
    let (_d, mut app) = app_with("# Inbox\n");
    app.enable_mouse();
    toolbar_click(&mut app, "Capture");
    // prompt is open; type "hi" with the on-screen keyboard
    draw_once(&mut app, 160, 30);
    for c in ['h', 'i'] {
        let (x, y) = app.kb_pos(c).unwrap();
        app.handle_mouse(click(x, y));
    }
    // accept: click Enter key
    draw_once(&mut app, 160, 30);
    let (x, y) = app.kb_special_pos("Enter").unwrap();
    app.handle_mouse(click(x, y));
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(root.contains("- hi"), "{}", root);
}

#[test]
fn toolbar_help_and_close() {
    let (_d, mut app) = app_with("# A\n");
    app.enable_mouse();
    draw_once(&mut app, 160, 30); // first draw populates the toolbar
    toolbar_click(&mut app, "Help");
    assert_eq!(app.mode_pub(), "help");
    // click anywhere outside the toolbar closes it
    draw_once(&mut app, 160, 30);
    app.handle_mouse(click(80, 10));
    assert_eq!(app.mode_pub(), "normal");
}

#[test]
fn palette_row_click_runs_action() {
    let (_d, mut app) = app_with("# A\n\n- [x] finished\n");
    app.enable_mouse();
    draw_once(&mut app, 160, 30); // first draw populates the toolbar
    toolbar_click(&mut app, "Menu");
    assert_eq!(app.mode_pub(), "picker");
    // click the "clear done" row
    draw_once(&mut app, 160, 30);
    let (x, y) = app.palette_row_pos("clear done").expect("row");
    app.handle_mouse(click(x, y));
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(!root.contains("finished"), "{}", root);
}

#[test]
fn toolbar_undo_restores() {
    let (_d, mut app) = app_with("# A\n\n- keep me\n");
    app.enable_mouse();
    draw_once(&mut app, 160, 30);
    app.handle_mouse(click(10, 2)); // select the item
    toolbar_click(&mut app, "Trash");
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(!root.contains("keep me"));
    toolbar_click(&mut app, "Undo");
    let root = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(root.contains("keep me"), "{}", root);
}

#[test]
fn filter_result_click_zooms() {
    let (_d, mut app) = app_with("# Alpha\n\n# Beta\n\n# Gamma\n");
    app.enable_mouse();
    toolbar_click(&mut app, "Filter");
    draw_once(&mut app, 160, 30);
    for c in ['g', 'a', 'm', 'm', 'a'] {
        let (x, y) = app.kb_pos(c).unwrap();
        app.handle_mouse(click(x, y));
    }
    draw_once(&mut app, 160, 30);
    let (x, y) = app.filter_row_pos(0).expect("one hit");
    app.handle_mouse(click(x, y));
    assert_eq!(app.mode_pub(), "normal");
    // cursor on Gamma
    let cur = app.cursor;
    let rows = app.rows();
    assert_eq!(app.title_of(rows[cur].nref), "Gamma");
}

#[test]
fn props_row_click_selects() {
    let (_d, mut app) = app_with("# A\n\n- task\n");
    app.enable_mouse();
    draw_once(&mut app, 160, 30);
    app.handle_mouse(click(10, 2));
    // give it a property first via core
    let r = app.rows()[1].nref;
    fold_core::ops::set_property(app.vault_mut(), r, "due", "2026-09-20").unwrap();
    toolbar_click(&mut app, "Props");
    assert_eq!(app.mode_pub(), "props");
    draw_once(&mut app, 160, 30);
    let (x, y) = app.props_row_pos(0).expect("prop row");
    app.handle_mouse(click(x, y));
    // now Del via toolbar deletes it
    toolbar_click(&mut app, "Del");
    let files: Vec<_> = std::fs::read_dir(app.vault_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".md") && n != "root.md")
        .collect();
    let text = std::fs::read_to_string(app.vault_dir().join(&files[0])).unwrap();
    assert!(!text.contains("due:"), "{}", text);
}
