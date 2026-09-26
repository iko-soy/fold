//! Regressions for keymap, undo, cursor and small-terminal bugs.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fold_tui::app::App;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn press(app: &mut App, keys: &str) {
    for c in keys.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

fn app_with(text: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), text).unwrap();
    let app = App::new(dir.path()).unwrap();
    (dir, app)
}

fn draw(app: &mut App, w: u16, h: u16) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
}

fn root(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("root.md")).unwrap()
}

fn md_files(dir: &tempfile::TempDir) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md"))
        .collect();
    v.sort();
    v
}

fn current_title(app: &App) -> String {
    app.current().map(|r| app.title_of(r)).unwrap_or_default()
}

#[test]
fn ctrl_d_pages_instead_of_deleting() {
    let text = "# A\n\n# B\n\n# C\n";
    let (d, mut app) = app_with(text);
    draw(&mut app, 100, 24);
    app.handle_key(ctrl('d'));
    assert_eq!(root(&d), text);
    assert_eq!(current_title(&app), "C");
    app.handle_key(ctrl('u'));
    assert_eq!(current_title(&app), "A");
}

#[test]
fn ctrl_c_in_the_editor_never_quits() {
    // in the normal keymap Ctrl-c copies; the app quits only outside it
    let (_d, mut app) = app_with("# A\n\nbody\n");
    press(&mut app, "e");
    app.handle_key(ctrl('c'));
    assert!(!app.quit_requested());
    assert_eq!(app.mode_pub(), "edit");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode_pub(), "normal");
    app.handle_key(ctrl('c'));
    assert!(app.quit_requested());
}

#[test]
fn yank_keeps_redo() {
    let (d, mut app) = app_with("- [ ] a\n");
    press(&mut app, "xu");
    assert_eq!(root(&d), "- [ ] a\n");
    press(&mut app, "yU");
    assert_eq!(root(&d), "- [x] a\n");
}

#[test]
fn undo_make_block_removes_new_file() {
    let (d, mut app) = app_with("# A\n\n## B\n\nbody\n");
    let before = md_files(&d);
    press(&mut app, "js");
    assert!(md_files(&d).len() > before.len());
    press(&mut app, "u");
    assert_eq!(md_files(&d), before);
    assert_eq!(root(&d), "# A\n\n## B\n\nbody\n");
}

#[test]
fn noop_verbs_push_no_undo_entry() {
    let (d, mut app) = app_with("- [ ] a\n");
    // toggle, hide done (no rows left), then a verb with no target
    press(&mut app, "xzdx");
    press(&mut app, "u");
    assert_eq!(root(&d), "- [ ] a\n");
}

#[test]
fn new_sibling_lands_after_cursor() {
    let (_d, mut app) = app_with("# A\n\n# \n\n# C\n");
    press(&mut app, "G");
    assert_eq!(current_title(&app), "C");
    press(&mut app, "n");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.cursor, 3);
}

#[test]
fn new_child_of_folded_node_opens_the_child() {
    let (_d, mut app) = app_with("# A\n\n## B\n");
    press(&mut app, "hN");
    assert_eq!(app.mode_pub(), "edit");
    assert_eq!(current_title(&app), "");
}

#[test]
fn hide_done_clamps_cursor() {
    let (_d, mut app) = app_with("- a\n- [x] b\n- [x] c\n");
    press(&mut app, "G");
    press(&mut app, "zd");
    assert_eq!(app.cursor, 0);
}

#[test]
fn parent_and_sibling_from_block_row() {
    let (_d, mut app) = app_with("# A\n\n## B\n\n## C\n");
    press(&mut app, "js");
    assert_eq!(current_title(&app), "B");
    press(&mut app, "}");
    assert_eq!(current_title(&app), "C");
    press(&mut app, "{-");
    assert_eq!(current_title(&app), "A");
}

#[test]
fn gg_in_reading_pane_keeps_outline_cursor() {
    let (_d, mut app) = app_with("# A\n\nx\ny\nz\n\n# B\n");
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "jjgg");
    assert_eq!(app.read_cursor_pub(), 0);
    assert_eq!(app.cursor, 0);
}

#[test]
fn reading_cursor_resets_on_new_target() {
    let body: String = (0..30).map(|i| format!("line {}\n", i)).collect();
    let (_d, mut app) = app_with(&format!("# A\n\n{}\n# B\n\nb1\n", body));
    draw(&mut app, 100, 24);
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "G");
    assert!(app.read_cursor_pub() > 20);
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "j");
    draw(&mut app, 100, 24);
    assert_eq!(app.read_cursor_pub(), 0);
}

#[test]
fn tiny_terminal_popups_do_not_panic() {
    let (_d, mut app) = app_with("# A\n");
    press(&mut app, ":");
    draw(&mut app, 40, 3);
    app.handle_key(key(KeyCode::Esc));
    press(&mut app, "a");
    draw(&mut app, 40, 3);
}

#[test]
fn tiny_terminal_filter_does_not_panic() {
    let (_d, mut app) = app_with("# A\n");
    press(&mut app, "/A");
    for h in 0..5 {
        draw(&mut app, 40, h);
    }
}

#[test]
fn respelling_moves_the_node_and_keeps_the_cursor_on_it() {
    let (d, mut app) = app_with("# P\n\n- a\n- b\n");
    press(&mut app, "j~");
    assert_eq!(root(&d), "# P\n\n- b\n\n## a\n");
    assert_eq!(current_title(&app), "a");
    // an item cannot move below a section: nothing changes
    press(&mut app, "kJ");
    assert_eq!(root(&d), "# P\n\n- b\n\n## a\n");
}

#[test]
fn editor_message_in_a_short_terminal_does_not_panic() {
    let (_d, mut app) = app_with("# A\n\nbody\n");
    press(&mut app, "e");
    // the default keymap answers "nothing to undo", a message on the pane's
    // last line; the pane has no inner rows at these sizes
    app.handle_key(ctrl('z'));
    for (w, h) in [(60u16, 5u16), (60, 6), (100, 4), (100, 3)] {
        draw(&mut app, w, h);
    }
}

#[test]
fn alt_down_keeps_a_multi_line_selection() {
    let with = |code, m| KeyEvent::new(code, m);
    let (d, mut app) = app_with("# A\n\na\nb\nc\nd\n");
    press(&mut app, "e");
    draw(&mut app, 100, 24);
    // the cursor opens on the title; go to "a", select "a" and "b", move them down twice
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(with(KeyCode::Down, KeyModifiers::SHIFT));
    app.handle_key(with(KeyCode::End, KeyModifiers::SHIFT));
    app.handle_key(with(KeyCode::Down, KeyModifiers::ALT));
    app.handle_key(with(KeyCode::Down, KeyModifiers::ALT));
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode_pub(), "normal");
    // the first move must leave "a" and "b" selected, so the second moves both
    assert_eq!(root(&d), "# A\n\nc\nd\na\nb\n");
}

#[test]
fn editor_undo_stops_at_cursor_moves() {
    // normal keymap: typing somewhere else after a cursor move is a new undo
    // step, so one Ctrl-Z takes back only the last run of typing
    let (d, mut app) = app_with("# A\n\none\ntwo\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    press(&mut app, "e");
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    press(&mut app, "A");
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Home));
    press(&mut app, "B");
    app.handle_key(ctrl('s'));
    assert_eq!(root(&d), "# A\n\nAone\nBtwo\n");
    app.handle_key(ctrl('z'));
    app.handle_key(ctrl('s'));
    assert_eq!(root(&d), "# A\n\nAone\ntwo\n");
}
