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
    let mut app = App::new(dir.path()).unwrap();
    // these tests are about the reading pane, hidden by default
    app.show_reading = true;
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
    // drag b1 to the far left of a1's row: before a1 (a1 dropped before b1,
    // its next sibling, would stay where it is)
    draw(&mut app);
    let from = app.hit_pos(Hit::Row(2)).unwrap();
    let to_row = app.hit_pos(Hit::Row(1)).unwrap();
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

#[test]
fn the_reading_pane_is_hidden_until_asked_for_or_editing() {
    let (_d, mut app) = app_with("# A\n\ntext\n");
    app.show_reading = false;
    let s = draw(&mut app);
    assert!(app.hit_pos(Hit::ReadingPane).is_none() && app.hit_pos(Hit::Divider).is_none(), "{}", s);
    assert!(!s.contains("# A"), "{}", s);
    // editing opens the pane for the editor, and closing it hides it again
    typing(&mut app, "e");
    assert_eq!(app.mode_pub(), "edit");
    assert!(draw(&mut app).contains("# A"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode_pub(), "normal");
    assert!(!draw(&mut app).contains("# A"));
    assert!(app.hit_pos(Hit::ReadingPane).is_none());
    // the top bar's button shows it, and `zp` hides it
    button(&mut app, Action::ReadingPane);
    assert!(draw(&mut app).contains("# A"));
    assert!(app.hit_pos(Hit::ReadingPane).is_some());
    typing(&mut app, "zp");
    draw(&mut app);
    assert!(app.hit_pos(Hit::ReadingPane).is_none());
}

#[test]
fn without_the_pane_rows_show_counts_and_their_text() {
    let (_d, mut app) = app_with("# Homelab\n\nTwo boxes in the closet.\n\n- [ ] a\n- [x] b\n");
    app.show_reading = false;
    let s = draw(&mut app);
    let line = s.lines().find(|l| l.contains("Homelab")).unwrap();
    assert!(line.contains("Homelab  1/2  Two boxes in the closet."), "{}", line);
    // with the pane, the text is there instead, and counts sit at the right
    app.show_reading = true;
    let s = draw(&mut app);
    let line = s.lines().find(|l| l.contains("▾ Homelab")).unwrap();
    assert!(!line.contains("Homelab  1/2") && line.contains("1/2"), "{}", line);
}

#[test]
fn outline_keys_work_from_the_reading_pane_and_editing_returns_focus() {
    let (_d, mut app) = app_with("# A\n\ntext\n");
    draw(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    typing(&mut app, ":");
    assert_eq!(app.mode_pub(), "picker");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    typing(&mut app, "?");
    assert_eq!(app.mode_pub(), "help");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    // `z` sequences too
    typing(&mut app, "zp");
    assert!(!app.show_reading);
    // the editor gives focus back to the pane it came from: the outline
    typing(&mut app, "e");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    typing(&mut app, ":");
    assert_eq!(app.mode_pub(), "picker");
}

#[test]
fn toggles_say_whether_they_are_on() {
    let (_d, mut app) = app_with("# A\n");
    assert_eq!(app.action_on(Action::ReadingPane), Some(true));
    button(&mut app, Action::Palette);
    typing(&mut app, "reading pane");
    let s = draw(&mut app);
    assert!(s.contains("Reading pane · on"), "{}", s);
}

#[test]
fn the_view_is_remembered() {
    let (d, mut app) = app_with("# A\n\n## B\n\n- c\n\n# D\n");
    app.show_reading = false;
    typing(&mut app, "zdzw");
    typing(&mut app, "h"); // fold A
    typing(&mut app, "jj");
    let v = app.view();
    assert!(!v.show_reading && v.hide_done && !v.wrap && v.folded.len() == 1);
    let mut again = App::new(d.path()).unwrap();
    again.apply_view(fold_tui::app::View::parse(&v.to_text()).unwrap(), false);
    assert_eq!(again.view(), v);
}

#[test]
fn a_short_hint_where_the_long_one_does_not_fit() {
    let (_d, mut app) = app_with("# A\n");
    let mut t = Terminal::new(TestBackend::new(70, 10)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    let last: String = (0..70).map(|x| b[(x, 9)].symbol()).collect();
    assert!(last.contains("right-click for actions · ? help"), "{}", last);
}

#[test]
fn move_to_from_a_menu_moves_the_menus_node() {
    // B is folded away under A, so it has no outline row: its menu comes
    // from its line in the reading pane, and Move to… must move B, not A
    let (d, mut app) = app_with("# A\n\n## B\n\n# C\n");
    typing(&mut app, "h");
    assert_eq!(app.rows().len(), 2, "A folded");
    let s = draw(&mut app);
    assert!(s.contains("## B"), "{}", s);
    right_click(&mut app, Hit::DocLine(2));
    click(&mut app, Hit::MenuItem(fold_tui::app::node_menu_index(Action::Refile)));
    typing(&mut app, "C");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(root(&d), "# A\n\n# C\n\n## B\n");
}

#[test]
fn a_keymap_chosen_by_flag_is_not_remembered() {
    // §10.6: the keymap is remembered with the view unless `--keys` or
    // `$FOLD_KEYS` chose it for this run
    use fold_tui::app::{EditKeys, View};
    let (_d, mut app) = app_with("# A\n");
    app.set_edit_keys(EditKeys::Vim); // `--keys vim`, as run() does
    app.apply_view(View::parse("keys normal\n").unwrap(), true);
    assert_eq!(app.view().keys, Some(EditKeys::Normal));
    // nor is a switch made during such a run
    app.set_edit_keys(EditKeys::Helix);
    assert_eq!(app.view().keys, Some(EditKeys::Normal));
    // without --keys, the keymap in use is what the view remembers
    let (_d, mut app) = app_with("# A\n");
    app.apply_view(View::parse("keys normal\n").unwrap(), false);
    app.set_edit_keys(EditKeys::Helix);
    assert_eq!(app.view().keys, Some(EditKeys::Helix));
}

#[test]
fn clicking_a_checkbox_while_editing_keeps_the_typed_text() {
    // §10.6: any outline verb saves the editor first; the outline's ☐ stays
    // drawn and clickable beside the editor
    let (d, mut app) = app_with("# A\n\nbody\n\n- [ ] t\n");
    typing(&mut app, "e");
    assert_eq!(app.mode_pub(), "edit");
    for k in [KeyCode::Down, KeyCode::Down, KeyCode::End] {
        app.handle_key(KeyEvent::new(k, KeyModifiers::NONE));
    }
    typing(&mut app, "!");
    click(&mut app, Hit::Check(1));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# A\n\nbody!\n\n- [x] t\n");
}

#[test]
fn a_menu_verb_while_editing_saves_the_editor_and_keeps_editing() {
    // the node menu's Delete on another node, then more typing: both saved
    let (d, mut app) = app_with("# A\n\nbody\n\n# B\n\n# C\n");
    typing(&mut app, "e");
    for k in [KeyCode::Down, KeyCode::Down, KeyCode::End] {
        app.handle_key(KeyEvent::new(k, KeyModifiers::NONE));
    }
    typing(&mut app, "!");
    right_click(&mut app, Hit::Row(1));
    click(&mut app, Hit::MenuItem(fold_tui::app::node_menu_index(Action::Delete)));
    assert_eq!(app.mode_pub(), "edit");
    assert_eq!(root(&d), "# A\n\nbody!\n\n# C\n");
    typing(&mut app, "?");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# A\n\nbody!?\n\n# C\n");
}

#[test]
fn a_drop_while_editing_saves_the_editor_first() {
    // dragging a row is a verb too (§10.1, §10.6)
    let (d, mut app) = app_with("# A\n\nbody\n\n# B\n\n- b1\n");
    typing(&mut app, "e");
    for k in [KeyCode::Down, KeyCode::Down, KeyCode::End] {
        app.handle_key(KeyEvent::new(k, KeyModifiers::NONE));
    }
    typing(&mut app, "!");
    // rows: A, B, b1 — drag b1 onto A's title
    draw(&mut app);
    let from = app.hit_pos(Hit::Row(2)).unwrap();
    let onto = app.hit_pos(Hit::Row(0)).unwrap();
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), onto));
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), onto));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(root(&d), "# A\n\nbody!\n\n- b1\n\n# B\n");
}

#[test]
fn a_checkbox_click_toggles_the_row_clicked_when_the_save_first_adds_rows() {
    // unsaved text adds a task above `t`; saving it first shifts the rows,
    // and the click still toggles `t`
    let (d, mut app) = app_with("# A\n\n- [ ] t\n");
    typing(&mut app, "e");
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    typing(&mut app, "- [ ] n");
    click(&mut app, Hit::Check(1));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(root(&d), "# A\n- [ ] n\n\n- [x] t\n");
}

#[test]
fn palette_from_the_editor_keeps_its_text() {
    let (d, mut app) = app_with("# A\n\nbody\n");
    typing(&mut app, "e");
    for k in [KeyCode::Down, KeyCode::Down, KeyCode::End] {
        app.handle_key(KeyEvent::new(k, KeyModifiers::NONE));
    }
    typing(&mut app, "!");
    assert!(app.editor_dirty());
    // ☰ Commands in the top bar, before the 750 ms autosave, then Esc: the
    // popup closes back to the editor it opened over (§10.2)
    button(&mut app, Action::Palette);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode_pub(), "edit");
    typing(&mut app, "?");
    // and a pasted line goes into the editor only while it is on screen
    button(&mut app, Action::Help);
    app.handle_paste("pasted");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode_pub(), "edit");
    button(&mut app, Action::Filter);
    click(&mut app, Hit::Backdrop);
    assert_eq!(app.mode_pub(), "edit");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# A\n\nbody!?\n");
}

#[test]
fn editor_keys_from_the_palette_keeps_the_editor_and_its_text() {
    let (d, mut app) = app_with("# A\n\nbody\n");
    typing(&mut app, "e");
    for k in [KeyCode::Down, KeyCode::Down, KeyCode::End] {
        app.handle_key(KeyEvent::new(k, KeyModifiers::NONE));
    }
    typing(&mut app, "!");
    // ☰ → Editor keys, the documented way to change keymaps while editing
    button(&mut app, Action::Palette);
    typing(&mut app, "editor keys");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.mode_pub(), "edit", "switching keymaps leaves the editor open");
    click(&mut app, Hit::Button(Action::EditDone, None));
    assert_eq!(root(&d), "# A\n\nbody!\n");
}

#[test]
fn resolving_conflicts_from_the_editor_saves_and_leaves_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\nbody\n\n- [ ] task\n").unwrap();
    std::fs::write(dir.path().join("root.sync-conflict-20260912-100000-phone.md"), "# A\n\nbody\n\n- [x] task\n").unwrap();
    let mut v = fold_core::vault::Vault::open(dir.path()).unwrap();
    fold_core::merge::merge_sync_conflicts(&mut v, false).unwrap();
    drop(v);
    let mut app = App::new(dir.path()).unwrap();
    app.show_reading = true;
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    typing(&mut app, "ejjA!");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.cursor_block() && app.editor_dirty(), "Vim normal mode, unsaved");
    // the status bar's ⚠ opens the conflict view, which replaces the editor:
    // no editor is left behind it, holding the terminal's block cursor
    button(&mut app, Action::ResolveConflicts);
    assert_eq!(app.mode_pub(), "conflict");
    assert!(!app.cursor_block());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode_pub(), "normal");
    assert!(root(&dir).contains("body!"), "{}", root(&dir));
    // nothing is left behind the outline: `e` opens a fresh editor
    typing(&mut app, "e");
    assert_eq!(app.mode_pub(), "edit");
    assert!(!app.editor_dirty());
}
