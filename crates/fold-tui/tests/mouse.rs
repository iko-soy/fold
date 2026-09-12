use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use fold_tui::app::App;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

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

fn wheel(x: u16, y: u16, up: bool) -> MouseEvent {
    MouseEvent {
        kind: if up { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown },
        column: x,
        row: y,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

#[test]
fn click_selects_outline_row() {
    let (_d, mut app) = app_with("# A\n\n## B\n\n## C\n\n## D\n");
    app.enable_mouse();
    draw_once(&mut app, 100, 24);
    // outline pane is the left ~34%; rows start at y=1 inside the border.
    // row 0 = A (y=1), row 1 = B (y=2), row 2 = C (y=3), row 3 = D (y=4)
    app.handle_mouse(click(10, 3));
    assert_eq!(app.cursor, 2);
}

#[test]
fn click_on_fold_marker_folds() {
    let (_d, mut app) = app_with("# A\n\n## B\n\n## C\n");
    app.enable_mouse();
    draw_once(&mut app, 100, 24);
    // fold marker for row 0 is at x=1 (border) + depth 0 → columns 1..=2
    let rows_before = app.rows().len();
    app.handle_mouse(click(2, 1));
    let rows_after = app.rows().len();
    assert!(rows_after < rows_before, "{} -> {}", rows_before, rows_after);
}

#[test]
fn wheel_moves_cursor() {
    let (_d, mut app) = app_with("# A\n\n## B\n\n## C\n\n## D\n\n## E\n");
    app.enable_mouse();
    draw_once(&mut app, 100, 24);
    app.handle_mouse(wheel(10, 5, false)); // +3
    assert_eq!(app.cursor, 3);
    app.handle_mouse(wheel(10, 5, false)); // +3 more, clamped to the last row
    assert_eq!(app.cursor, 4);
    app.handle_mouse(wheel(10, 5, true)); // -3
    assert_eq!(app.cursor, 1);
}

#[test]
fn double_click_zooms() {
    let (_d, mut app) = app_with("# A\n\n## B\n\nbody of b\n");
    app.enable_mouse();
    draw_once(&mut app, 100, 24);
    app.handle_mouse(click(10, 2)); // row 1 = B
    app.handle_mouse(click(10, 2)); // same spot quickly = double click
    // zoomed into B: the outline now shows B's subtree (empty) and the
    // reading pane shows B's document
    let doc = app.reading_doc_pub();
    assert!(doc.lines[0].contains("# B"), "{:?}", doc.lines);
}

#[test]
fn click_in_reading_pane_moves_cursor() {
    let (_d, mut app) = app_with("# A\n\nline one\n\nline two\n\nline three\n");
    app.enable_mouse();
    app.key_normal(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    )); // zoom into A, focus reading
    draw_once(&mut app, 100, 24);
    // reading pane is the right ~66% starting at x≈34; doc line 0 "# A" at y=1
    let x = 50;
    app.handle_mouse(click(x, 4)); // doc line 3 = "line two"
    assert_eq!(app.read_cursor_pub(), 3);
}
