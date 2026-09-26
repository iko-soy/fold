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

#[test]
fn an_open_menu_survives_an_external_rewrite_that_shrinks_the_file() {
    let (d, mut app) = app_with("# A\n\n- one\n- two\n- three\n");
    right_click(&mut app, Hit::Row(3));
    assert!(draw(&mut app).contains("Move down"), "menu open on three");
    // another program rewrites the file while the menu is open (§11.2)
    std::fs::write(d.path().join("root.md"), "# A\n").unwrap();
    app.reload_external();
    // the next frame must not index a node that no longer exists
    draw(&mut app);
}

#[test]
fn an_open_menu_never_acts_on_a_different_node_after_a_reload() {
    let (d, mut app) = app_with("# A\n\n- one\n- two\n- three\n");
    right_click(&mut app, Hit::Row(3));
    assert!(draw(&mut app).contains("Move down"), "menu open on three");
    // a line inserted above renumbers every node below it
    std::fs::write(d.path().join("root.md"), "# A\n\n- zero\n- one\n- two\n- three\n").unwrap();
    app.reload_external();
    // whether the menu closed or still targets "three", Delete never hits "two"
    draw(&mut app);
    let delete = fold_tui::app::node_menu_index(Action::Delete);
    if let Some(p) = app.hit_pos(Hit::MenuItem(delete)) {
        app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), p));
        app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), p));
    }
    let text = root(&d);
    assert!(text.contains("- two\n"), "Delete removed a node the menu was not opened on: {:?}", text);
    assert_eq!(text, "# A\n\n- zero\n- one\n- two\n", "the menu stayed on three");
}

#[test]
fn an_open_property_form_or_move_to_list_survives_an_external_rewrite() {
    let (d, mut app) = app_with("# A\n\n- one\n- two\n- three\n");
    click(&mut app, Hit::Row(3));
    typing(&mut app, "a");
    assert_eq!(app.mode_pub(), "props");
    std::fs::write(d.path().join("root.md"), "# A\n").unwrap();
    app.reload_external();
    draw(&mut app);
    assert_eq!(app.mode_pub(), "normal", "the form closes with its node gone");
    // Move to…'s list of nodes is made again from the files as they are
    let (d, mut app) = app_with("# A\n\n- one\n- two\n- three\n");
    right_click(&mut app, Hit::Row(3));
    click(&mut app, Hit::MenuItem(fold_tui::app::node_menu_index(Action::Refile)));
    std::fs::write(d.path().join("root.md"), "# A\n").unwrap();
    app.reload_external();
    assert!(draw(&mut app).contains("Move to"));
}

#[test]
fn folding_above_the_selection_keeps_it() {
    let (_d, mut app) = app_with("# A\n\n- a1\n- a2\n\n# B\n\n# C\n");
    click(&mut app, Hit::Row(3));
    assert_eq!(title(&app), "B");
    click(&mut app, Hit::Fold(0));
    assert_eq!(app.rows().len(), 3, "A folded");
    assert_eq!(title(&app), "B", "a fold click above the selection must not move it");
    click(&mut app, Hit::Fold(0));
    assert_eq!(title(&app), "B", "nor an unfold click");
    // folding the node the selection is inside selects the folded node
    click(&mut app, Hit::Row(2));
    assert_eq!(title(&app), "a2");
    click(&mut app, Hit::Fold(0));
    assert_eq!(title(&app), "A");
}

#[test]
fn dropping_outside_the_outline_does_nothing() {
    let (d, mut app) = app_with("# A\n\n- a1\n\n# B\n\n- b1\n");
    // rows: A, a1, B, b1 — drag b1 out into the reading pane, level with A
    draw(&mut app);
    let from = app.hit_pos(Hit::Row(3)).unwrap();
    let a = app.hit_pos(Hit::Row(0)).unwrap();
    let (px, _) = app.hit_pos(Hit::ReadingPane).unwrap();
    assert!(
        matches!(app.hit_at(px, a.1), Some(Hit::ReadingPane | Hit::DocLine(_))),
        "the drop point is in the reading pane, not on an outline row"
    );
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), (px, a.1)));
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), (px, a.1)));
    assert_eq!(root(&d), "# A\n\n- a1\n\n# B\n\n- b1\n");
}

#[test]
fn a_menu_verb_after_cutting_a_block_title_acts_on_the_menus_node() {
    // root.md embeds block "cut me" (dozzod, file 1) and block "task"
    // (racfer, file 2); the editor is open on A with "cut me"'s title cut
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("root.md"),
        "# A\n\n- one\n![[dozzod-binwes-talsun-worbec]]\n- two\n\n# B\n\n![[racfer-hattes-mislup-nodrys]]\n",
    )
    .unwrap();
    std::fs::write(d.path().join("dozzod~cut.md"), "---\nid: dozzod-binwes-talsun-worbec\n---\n\n- cut me\n").unwrap();
    let task = d.path().join("racfer~task.md");
    std::fs::write(&task, "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- [ ] task\n").unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    typing(&mut app, "e");
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    // right-click "task" in the outline, then its menu's Toggle done: the
    // verb deletes the cut block first, which renumbers the files after it
    draw(&mut app);
    let row = (0..10).find(|&i| app.rows().get(i).is_some_and(|r| app.title_of(r.nref) == "task")).unwrap();
    right_click(&mut app, Hit::Row(row));
    click(&mut app, Hit::MenuItem(fold_tui::app::node_menu_index(Action::ToggleDone)));
    assert!(std::fs::read_to_string(&task).unwrap().contains("- [x] task"));
}

#[test]
fn an_open_filter_survives_an_external_rewrite_that_shrinks_the_file() {
    let (d, mut app) = app_with("# A\n\n- one\n- two\n- three\n");
    typing(&mut app, "/thr");
    assert!(draw(&mut app).contains("three"), "the filter lists three");
    // another program rewrites the file while the filter is open (§11.2)
    std::fs::write(d.path().join("root.md"), "# A\n").unwrap();
    app.reload_external();
    // the next frame must not index a node that no longer exists
    draw(&mut app);
    // a line inserted above renumbers the nodes: the hits are found again,
    // and the selection stays on its node
    let (d, mut app) = app_with("# A\n\n- one\n- two\n- three\n");
    typing(&mut app, "/t");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    std::fs::write(d.path().join("root.md"), "# A\n\n- tea\n- one\n- two\n- three\n").unwrap();
    app.reload_external();
    assert!(draw(&mut app).contains("tea"), "the filter lists tea");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(title(&app), "three");
}

#[test]
fn a_reload_after_the_property_form_closed_does_not_crash() {
    // blocks B (file 1) and T (file 2); T's properties are looked at and
    // closed, then deleting B trashes its file, renumbering T's
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n![[dozzod-binwes-talsun-worbec]]\n![[racfer-hattes-mislup-nodrys]]\n").unwrap();
    std::fs::write(d.path().join("dozzod~b.md"), "---\nid: dozzod-binwes-talsun-worbec\n---\n\n- B\n").unwrap();
    std::fs::write(d.path().join("racfer~t.md"), "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- T\n").unwrap();
    let mut app = App::new(d.path()).unwrap();
    typing(&mut app, "jja");
    assert_eq!(app.mode_pub(), "props");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    typing(&mut app, "kd");
    assert!(!d.path().join("dozzod~b.md").exists());
    // then another program changes a file (§11.2)
    std::fs::write(d.path().join("root.md"), "# A\n\n![[racfer-hattes-mislup-nodrys]]\n- new\n").unwrap();
    app.reload_external();
    assert!(draw(&mut app).contains("new"));
}

#[test]
fn a_new_node_whose_editor_cannot_open_leaves_the_open_editor_alone() {
    // the editor is open on A with block b's body typed in, and another
    // program changed b's file: its save is refused, and it stays open
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n![[racfer-hattes-mislup-nodrys]]\n- two\n").unwrap();
    let b = d.path().join("racfer~b.md");
    std::fs::write(&b, "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- b\n  body\n").unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    typing(&mut app, "e");
    for _ in 0..4 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    typing(&mut app, "X");
    std::fs::write(&b, "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- b\n  body, from Helix\n").unwrap();
    // New child (palette, menu): the node is made in root.md, but the
    // editor on A cannot be switched to it
    app.run_action(Action::NewChild);
    assert_eq!(app.mode_pub(), "edit");
    typing(&mut app, "new");
    let s = draw(&mut app);
    assert!(!s.contains("# A new"), "typing for the new node went into A's title:\n{}", s);
    assert!(s.contains("bodyX"), "{}", s);
}

#[test]
fn making_the_edited_node_a_block_keeps_the_editor_on_it() {
    // editing b, *Make block* from the palette (or the node menu) on b: the
    // editor is re-rendered over what the verb wrote, still on b
    let (_d, mut app) = app_with("# A\n\n- a\n- b\n  body\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    typing(&mut app, "jje");
    assert!(draw(&mut app).contains("Editing b"));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.run_action(Action::MakeBlock);
    assert_eq!(app.mode_pub(), "edit");
    typing(&mut app, "!");
    let s = draw(&mut app);
    assert!(s.contains("Editing b") && !s.contains("# A"), "the editor moved off b:\n{}", s);
}

#[test]
fn a_verb_that_moves_the_edited_node_keeps_the_editor_on_it() {
    let editing_b = |text: &str| {
        let (d, mut app) = app_with(text);
        app.set_edit_keys(fold_tui::app::EditKeys::Normal);
        let b = app.rows().iter().position(|r| app.title_of(r.nref) == "b").unwrap();
        typing(&mut app, &"j".repeat(b));
        typing(&mut app, "e");
        assert!(draw(&mut app).contains("Editing b"));
        (d, app)
    };
    let on_b = |app: &mut App| {
        assert_eq!(app.mode_pub(), "edit");
        let s = draw(app);
        assert!(s.contains("Editing b") && !s.contains("- a"), "the editor moved off b:\n{}", s);
    };
    // Indent and Outdent (palette, node menu)
    let (d, mut app) = editing_b("# A\n\n- a\n- b\n  body\n");
    app.run_action(Action::Indent);
    assert_eq!(root(&d), "# A\n\n- a\n  - b\n    body\n");
    on_b(&mut app);
    app.run_action(Action::Outdent);
    assert_eq!(root(&d), "# A\n\n- a\n- b\n  body\n");
    on_b(&mut app);
    // Move to…
    let (d, mut app) = editing_b("# A\n\n- a\n- b\n  body\n\n# C\n");
    app.run_action(Action::Refile);
    typing(&mut app, "C");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(root(&d).ends_with("# C\n\n- b\n  body\n"), "{}", root(&d));
    on_b(&mut app);
    // a first property, set in the form opened over the editor, makes it
    // a block (§6.1)
    let (d, mut app) = editing_b("# A\n\n- a\n- b\n  body\n");
    app.run_action(Action::Props);
    typing(&mut app, "ntag");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    typing(&mut app, "x");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!root(&d).contains("- b"), "b is a block: {}", root(&d));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    on_b(&mut app);
    // dragged onto a's title (§10.1)
    let (d, mut app) = editing_b("# A\n\n- a\n- b\n  body\n");
    draw(&mut app);
    let (from, onto) = (app.hit_pos(Hit::Row(2)).unwrap(), app.hit_pos(Hit::Row(1)).unwrap());
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), onto));
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), onto));
    assert_eq!(root(&d), "# A\n\n- a\n  - b\n    body\n");
    on_b(&mut app);
    // deleted, it leaves nothing to edit: the editor closes instead of
    // showing the node above it
    let (d, mut app) = editing_b("# A\n\n- a\n- b\n  body\n");
    app.run_action(Action::Delete);
    assert_eq!(root(&d), "# A\n\n- a\n");
    assert_eq!(app.mode_pub(), "normal");
}

#[test]
fn edit_from_a_menu_after_cutting_a_block_title_opens_on_the_menus_node() {
    // as above, with the menu's Edit: opening an editor saves the open one
    // first, which deletes the cut block and renumbers the files after it
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("root.md"),
        "# A\n\n- one\n![[dozzod-binwes-talsun-worbec]]\n- two\n\n# B\n\n![[racfer-hattes-mislup-nodrys]]\n",
    )
    .unwrap();
    std::fs::write(d.path().join("dozzod~cut.md"), "---\nid: dozzod-binwes-talsun-worbec\n---\n\n- cut me\n").unwrap();
    std::fs::write(
        d.path().join("racfer~task.md"),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- task\n  - sub\n",
    )
    .unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.show_reading = true;
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    typing(&mut app, "e");
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    draw(&mut app);
    let row = (0..10).find(|&i| app.rows().get(i).is_some_and(|r| app.title_of(r.nref) == "task")).unwrap();
    right_click(&mut app, Hit::Row(row));
    click(&mut app, Hit::MenuItem(fold_tui::app::node_menu_index(Action::Edit)));
    assert_eq!(app.mode_pub(), "edit");
    assert!(!d.path().join("dozzod~cut.md").exists(), "the cut block went with the old editor");
    let s = draw(&mut app);
    assert!(s.contains("- task") && s.contains("- sub"), "{}", s);
}

#[test]
fn dragging_the_edited_node_before_a_sibling_keeps_the_editor_on_it_among_namesakes() {
    // editing the first "b" (body "mine"), dragged to the left of t's row:
    // before t (§10.1). A later sibling is titled "b" too; the editor must
    // stay on the node it was open on, not follow the namesake
    let (d, mut app) = app_with("# A\n\n- t\n- b\n  mine\n- b\n  theirs\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    // rows: A, t, b (mine), b (theirs)
    typing(&mut app, "jje");
    assert!(draw(&mut app).contains("mine"));
    let (from, to) = (app.hit_pos(Hit::Row(2)).unwrap(), app.hit_pos(Hit::Row(1)).unwrap());
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), (2, to.1)));
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), (2, to.1)));
    assert_eq!(root(&d), "# A\n\n- b\n  mine\n- t\n- b\n  theirs\n");
    assert_eq!(app.mode_pub(), "edit");
    // typed at the end of the body line the editor shows
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    typing(&mut app, "!");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(root(&d), "# A\n\n- b\n  mine!\n- t\n- b\n  theirs\n");
    // dragged before the namesake itself: the key that named the target
    // before the drop names the moved node after it
    let (d, mut app) = app_with("# A\n\n- t\n- b\n  theirs\n- b\n  mine\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    typing(&mut app, "jjje");
    assert!(draw(&mut app).contains("mine"));
    let (from, to) = (app.hit_pos(Hit::Row(3)).unwrap(), app.hit_pos(Hit::Row(2)).unwrap());
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), (2, to.1)));
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), (2, to.1)));
    assert_eq!(root(&d), "# A\n\n- t\n- b\n  mine\n- b\n  theirs\n");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    typing(&mut app, "!");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(root(&d), "# A\n\n- t\n- b\n  mine!\n- b\n  theirs\n");
}

#[test]
fn dragging_the_edited_item_into_a_node_keeps_the_editor_on_it_beside_a_section_namesake() {
    // editing item b, dropped onto T's title: into T, before T's section
    // "b" (§3.1). The editor stays on the item, not on the section
    let (d, mut app) = app_with("# A\n\n- b\n  mine\n\n## T\n\n### b\n\ntheirs\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    // rows: A, b (mine), T, b (theirs)
    typing(&mut app, "je");
    assert!(draw(&mut app).contains("mine"));
    let (from, onto) = (app.hit_pos(Hit::Row(1)).unwrap(), app.hit_pos(Hit::Row(2)).unwrap());
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), onto));
    draw(&mut app);
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), onto));
    let r = root(&d);
    assert!(r.find("- b\n  mine\n").is_some_and(|i| i > r.find("## T").unwrap()), "{}", r);
    assert_eq!(app.mode_pub(), "edit");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    typing(&mut app, "!");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(root(&d), r.replace("mine", "mine!"));
}

#[test]
fn outdenting_the_edited_node_past_a_namesake_parent_keeps_the_editor_on_it() {
    // editing the inner "z", Outdent: it lands after its old parent, also
    // titled "z" (§10.3). The editor stays on the node it was open on
    let (d, mut app) = app_with("# A\n\n- z\n  theirs\n  - z\n    mine\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    // rows: A, z (theirs), z (mine)
    typing(&mut app, "jje");
    assert!(draw(&mut app).contains("mine"));
    app.run_action(Action::Outdent);
    assert_eq!(root(&d), "# A\n\n- z\n  theirs\n- z\n  mine\n");
    assert_eq!(app.mode_pub(), "edit");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    typing(&mut app, "!");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(root(&d), "# A\n\n- z\n  theirs\n- z\n  mine!\n");
}
