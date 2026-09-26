//! Mouse-first (§10.1): every action is a click, drag or wheel away. Tests
//! draw a frame, find what the frame drew with `hit_pos`, and click it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use fold_tui::app::{Action, App, Hit};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

const W: u16 = 120;
const H: u16 = 32;

fn app_with(text: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), text).unwrap();
    let app = App::new(dir.path()).unwrap();
    (dir, app)
}

fn draw(app: &mut App) -> String {
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    let mut s = String::new();
    for y in 0..H {
        for x in 0..W {
            s.push_str(b[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

fn ev(kind: MouseEventKind, (x, y): (u16, u16)) -> MouseEvent {
    MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }
}

/// Draw, then click what the frame drew for `hit`.
fn click(app: &mut App, hit: Hit) {
    draw(app);
    let p = app.hit_pos(hit).unwrap_or_else(|| panic!("nothing drawn for {:?}", hit));
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), p));
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), p));
}

fn double_click(app: &mut App, hit: Hit) {
    draw(app);
    let p = app.hit_pos(hit).unwrap();
    for _ in 0..2 {
        app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), p));
        app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), p));
    }
}

fn right_click(app: &mut App, hit: Hit) {
    draw(app);
    let p = app.hit_pos(hit).unwrap();
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Right), p));
}

fn button(app: &mut App, a: Action) {
    draw(app);
    let p = app.button_pos(a).unwrap_or_else(|| panic!("no button {:?}", a));
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), p));
}

fn typing(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn root(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join("root.md")).unwrap()
}

fn title(app: &App) -> String {
    app.current().map(|r| app.title_of(r)).unwrap_or_default()
}

// ------------------------------------------------------------ outline

#[test]
fn click_selects_fold_marker_folds_checkbox_toggles() {
    let (d, mut app) = app_with("# A\n\n- [ ] t\n\n# B\n");
    click(&mut app, Hit::Row(2));
    assert_eq!(title(&app), "B");
    click(&mut app, Hit::Check(1));
    assert!(root(&d).contains("- [x] t"));
    click(&mut app, Hit::Fold(0));
    assert!(app.rows().len() == 2, "A folded");
    click(&mut app, Hit::Fold(0));
    assert_eq!(app.rows().len(), 3);
}

#[test]
fn double_click_zooms_and_breadcrumb_zooms_out() {
    let (_d, mut app) = app_with("# A\n\n## B\n\n- c\n");
    double_click(&mut app, Hit::Row(1));
    assert!(draw(&mut app).contains("fold › A › B"));
    click(&mut app, Hit::Crumb(None));
    assert!(!draw(&mut app).contains("fold › A"));
    assert_eq!(title(&app), "B", "cursor stays on the node zoomed out of");
}

#[test]
fn right_click_menu_runs_actions() {
    let (d, mut app) = app_with("# A\n\n- one\n- two\n");
    right_click(&mut app, Hit::Row(2));
    let s = draw(&mut app);
    assert!(s.contains("Move down") && s.contains("Delete"), "{}", s);
    let delete = fold_tui::app::node_menu_index(Action::Delete);
    click(&mut app, Hit::MenuItem(delete));
    assert_eq!(root(&d), "# A\n\n- one\n");
    // the row handle opens the same menu, and the backdrop closes it
    let c = app.cursor;
    click(&mut app, Hit::RowMenu(c));
    assert!(draw(&mut app).contains("Move down"));
    click(&mut app, Hit::Backdrop);
    assert!(!draw(&mut app).contains("Move down"));
}

#[test]
fn drag_onto_a_title_nests_and_left_of_it_places_before() {
    let (d, mut app) = app_with("# A\n\n- a1\n\n# B\n\n- b1\n");
    // rows: A, a1, B, b1 — drag b1 onto A's title: into A
    draw(&mut app);
    let from = app.hit_pos(Hit::Row(3)).unwrap();
    let onto = app.hit_pos(Hit::Row(0)).unwrap();
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), (onto.0, onto.1)));
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), onto));
    assert_eq!(root(&d), "# A\n\n- a1\n- b1\n\n# B\n");
    assert_eq!(title(&app), "b1", "the moved node stays selected");
    // drag a1 to the far left of b1's row: before b1
    draw(&mut app);
    let from = app.hit_pos(Hit::Row(1)).unwrap();
    let to_row = app.hit_pos(Hit::Row(2)).unwrap();
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), (2, to_row.1)));
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), (2, to_row.1)));
    assert_eq!(root(&d), "# A\n\n- b1\n- a1\n\n# B\n");
    // and one undo puts it back
    button(&mut app, Action::Undo);
    assert_eq!(root(&d), "# A\n\n- a1\n- b1\n\n# B\n");
}

#[test]
fn wheel_scrolls_without_moving_the_selection() {
    let text: String = (0..80).map(|i| format!("- item {}\n", i)).collect();
    let (_d, mut app) = app_with(&text);
    let s = draw(&mut app);
    assert!(s.contains("item 0 "));
    let p = app.hit_pos(Hit::Row(3)).unwrap();
    for _ in 0..5 {
        app.handle_mouse(ev(MouseEventKind::ScrollDown, p));
    }
    draw(&mut app);
    assert!(app.hit_pos(Hit::Row(0)).is_none() && app.hit_pos(Hit::Row(20)).is_some());
    assert_eq!(app.cursor, 0);
}

#[test]
fn dragging_the_divider_resizes_the_panes() {
    let (_d, mut app) = app_with("# A\n");
    draw(&mut app);
    let p = app.hit_pos(Hit::Divider).unwrap();
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), p));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), (60, p.1)));
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), (60, p.1)));
    draw(&mut app);
    // the border follows the pointer
    assert_eq!(app.hit_pos(Hit::Divider).map(|p| p.0), Some(60));
}

// ------------------------------------------------------------ reading pane

#[test]
fn reading_pane_checkbox_click_toggles_and_double_click_edits_at_the_line() {
    let (d, mut app) = app_with("# A\n\nfirst\nsecond\n\n- [ ] t\n");
    let s = draw(&mut app);
    assert!(s.contains("☐ t"), "{}", s);
    click(&mut app, Hit::DocCheck(5));
    assert!(root(&d).contains("- [x] t"));
    double_click(&mut app, Hit::DocLine(3));
    assert_eq!(app.mode_pub(), "edit");
    typing(&mut app, "X");
    click(&mut app, Hit::Button(Action::EditDone, None));
    assert_eq!(app.mode_pub(), "normal");
    assert!(root(&d).contains("Xsecond"), "{}", root(&d));
}

#[test]
fn pane_edit_button_opens_the_editor_on_the_shown_node() {
    let (_d, mut app) = app_with("# A\n\n## B\n\nbody\n");
    click(&mut app, Hit::Row(1));
    let b = app.current();
    click(&mut app, Hit::Button(Action::Edit, b));
    assert_eq!(app.mode_pub(), "edit");
    click(&mut app, Hit::Button(Action::EditRevert, None));
    assert_eq!(app.mode_pub(), "normal");
}

// ------------------------------------------------------------ popups

#[test]
fn move_to_picker_is_a_clickable_list() {
    let (d, mut app) = app_with("# Dest\n\n# Src\n\n- it\n");
    click(&mut app, Hit::Row(2));
    let it = app.current();
    right_click(&mut app, Hit::Row(2));
    click(&mut app, Hit::MenuItem(fold_tui::app::node_menu_index(Action::Refile)));
    let s = draw(&mut app);
    assert!(s.contains("Move to") && s.contains("Dest"), "{}", s);
    let _ = it;
    typing(&mut app, "dest");
    click(&mut app, Hit::PickRow(0));
    assert_eq!(root(&d), "# Dest\n\n- it\n\n# Src\n");
}

#[test]
fn filter_button_then_click_a_result() {
    let (_d, mut app) = app_with("# A\n\n## deep\n\n# B\n");
    click(&mut app, Hit::Row(0));
    click(&mut app, Hit::Fold(0));
    button(&mut app, Action::Filter);
    typing(&mut app, "deep");
    click(&mut app, Hit::FilterRow(0));
    assert_eq!(title(&app), "deep", "unfolded and selected");
}

#[test]
fn properties_form_edit_delete_add() {
    let (_d, mut app) = app_with("# A\n\n- t\n");
    click(&mut app, Hit::Row(1));
    typing(&mut app, "a");
    click(&mut app, Hit::Button(Action::PropAdd, None));
    typing(&mut app, "due");
    click(&mut app, Hit::Button(Action::PromptOk, None));
    typing(&mut app, "2026-10-01");
    click(&mut app, Hit::Button(Action::PromptOk, None));
    let s = draw(&mut app);
    assert!(s.contains("2026-10-01") && app.mode_pub() == "props", "{}", s);
    // clicking the value edits it, prefilled
    click(&mut app, Hit::PropValue(0));
    assert!(draw(&mut app).contains("› 2026-10-01"));
    click(&mut app, Hit::Button(Action::Close, None));
    click(&mut app, Hit::PropDelete(0));
    assert!(!draw(&mut app).contains("2026-10-01"));
}

#[test]
fn palette_rows_and_help_are_clickable() {
    let (_d, mut app) = app_with("# A\n\n- [x] done\n");
    button(&mut app, Action::Palette);
    typing(&mut app, "hide");
    click(&mut app, Hit::PaletteRow(0));
    assert!(draw(&mut app).contains("done hidden"));
    button(&mut app, Action::Help);
    assert_eq!(app.mode_pub(), "help");
    // anywhere outside the popup closes it
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), (0, H / 2)));
    assert_eq!(app.mode_pub(), "normal");
}

#[test]
fn capture_button_prompts_and_saves() {
    let (d, mut app) = app_with("# Inbox\n");
    button(&mut app, Action::Capture);
    typing(&mut app, "milk");
    click(&mut app, Hit::Button(Action::PromptOk, None));
    assert!(root(&d).contains("- milk"), "{}", root(&d));
    assert_eq!(title(&app), "milk");
}
