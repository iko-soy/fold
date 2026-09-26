//! The editor inside the app (§10.6): keymaps, saving, the mouse, paste.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use fold_tui::app::{Action, App, Hit};
use ratatui::{backend::TestBackend, Terminal};

fn app_with(text: &str) -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), text).unwrap();
    let app = App::new(d.path()).unwrap();
    (d, app)
}

fn draw(app: &mut App) -> String {
    let mut t = Terminal::new(TestBackend::new(110, 24)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    (0..24).map(|y| (0..110).map(|x| b[(x, y)].symbol()).collect::<String>() + "\n").collect()
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        let k = match c {
            '⎋' => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            '⏎' => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            c => KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        };
        app.handle_key(k);
    }
}

fn root(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join("root.md")).unwrap()
}

fn ev(kind: MouseEventKind, (x, y): (u16, u16)) -> MouseEvent {
    MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }
}

#[test]
fn vim_keys_edit_and_write() {
    let (d, mut app) = app_with("# A\n\nfirst line\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "e");
    let s = draw(&mut app);
    assert!(s.contains("NORMAL") && s.contains("vim"), "{}", s);
    keys(&mut app, "jjcwsecond⎋:w⏎");
    assert_eq!(root(&d), "# A\n\nsecond line\n");
    keys(&mut app, "ddu:q⏎");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# A\n\nsecond line\n");
}

#[test]
fn helix_keys_select_then_delete() {
    let (d, mut app) = app_with("# A\n\none two\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjwd:wq⏎");
    assert_eq!(root(&d), "# A\n\ntwo\n");
    assert_eq!(app.mode_pub(), "normal");
}

#[test]
fn keymap_label_cycles_the_keys() {
    let (_d, mut app) = app_with("# A\n");
    keys(&mut app, "e");
    draw(&mut app);
    let p = app.hit_pos(Hit::Button(Action::EditorKeys, None)).unwrap();
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), p));
    assert!(draw(&mut app).contains("⌨ vim"));
}

#[test]
fn drag_selects_and_paste_replaces() {
    let (d, mut app) = app_with("# A\n\nhello world\n");
    keys(&mut app, "e");
    draw(&mut app);
    let s = draw(&mut app);
    let (y, line) = s.lines().enumerate().find(|(_, l)| l.contains("hello world")).unwrap();
    let x = line.chars().position(|c| c == 'h').unwrap() as u16;
    let y = y as u16;
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), (x, y)));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), (x + 5, y)));
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), (x + 5, y)));
    app.handle_paste("goodbye");
    keys(&mut app, "⎋⎋");
    assert_eq!(root(&d), "# A\n\ngoodbye world\n");
}
