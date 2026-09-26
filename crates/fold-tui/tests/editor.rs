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

/// root.md "# A\n\n- one\n![[id]]\n", the embedded block's own file holding
/// "- two"; returns the block's file.
fn vault_with_bullet_block() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n- two\n").unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let two = v.find_by_path(&["A".into(), "two".into()]).unwrap();
    fold_core::ops::make_block(&mut v, two).unwrap();
    drop(v);
    let block = std::fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "md") && !p.ends_with("root.md"))
        .unwrap();
    assert!(root(&d).starts_with("# A\n\n- one\n![["), "{}", root(&d));
    (d, block)
}

#[test]
fn helix_xyp_above_a_block_leaves_the_block_file_alone() {
    // §5.2: a line pasted somewhere takes the tag of the line above it, so
    // "- one" duplicated below itself belongs to A, not to the block whose
    // title line happens to follow
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjxyp:w⏎");
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
    assert!(root(&d).contains("- one\n- one\n![["), "{}", root(&d));
}

#[test]
fn vim_o_above_a_block_leaves_the_block_file_alone() {
    // §5.2: a line typed after a tagged line inherits its tag; `O` on the
    // block's title line opens a line of A, above the block
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjjO- new⎋:w⏎");
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
    assert!(root(&d).contains("- one\n- new\n![["), "{}", root(&d));
}

/// root.md "# A\n\n- one\n![[id]]\n- two\n", the block being "- task", and
/// the editor open on A with the cursor on the block's title line; returns
/// the embed.
fn editing_task_block() -> (tempfile::TempDir, App, String) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n- task\n- two\n").unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = fold_core::ops::make_block(&mut v, t).unwrap();
    drop(v);
    let embed = format!("![[{}]]", id.as_str());
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", embed));
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    keys(&mut app, "e");
    // the editor shows "# A", "", "- one", "- task", "- two"
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    (d, app, embed)
}

#[test]
fn alt_down_moves_a_nested_block_with_its_embed() {
    // §5.2: the tag travels with the line; moving a nested block's title
    // line moves its embed rather than turning it into parent text
    let (d, mut app, embed) = editing_task_block();
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), format!("# A\n\n- one\n- two\n{}\n", embed));
}

#[test]
fn undoing_a_block_move_puts_its_embed_back() {
    // the parent's own lines are the same before and after the move; only
    // where the block sits in it differs, and undo must write that too
    let (d, mut app, embed) = editing_task_block();
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(root(&d), format!("# A\n\n- one\n- two\n{}\n", embed));
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", embed));
}

#[test]
fn ctrl_k_then_ctrl_v_moves_a_nested_block_with_its_embed() {
    // §5.2: cutting a nested block's title line and pasting it elsewhere
    // moves its embed
    let (d, mut app, embed) = editing_task_block();
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    // the cursor is on "- two"; paste above "- one"
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n{}\n- one\n- two\n", embed));
}

/// Like `editing_task_block`, with a body line under "- task"; returns the
/// block's file too.
fn editing_task_block_with_body() -> (tempfile::TempDir, App, String, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n- task\n  body\n- two\n").unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = fold_core::ops::make_block(&mut v, t).unwrap();
    let block = v.dir.join(&v.tree.files[v.tree.block_by_id(&id).unwrap().0].path);
    drop(v);
    let embed = format!("![[{}]]", id.as_str());
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", embed));
    assert!(std::fs::read_to_string(&block).unwrap().ends_with("\n- task\n  body\n"));
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    keys(&mut app, "e");
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    (d, app, embed, block)
}

#[test]
fn a_block_title_moved_past_its_body_leaves_the_body_where_it_sits() {
    // §5.2: owned lines no longer contiguous with the moved title line go to
    // the block they now sit in, here A under "- one"; the block's file must
    // never start with its body
    let (d, mut app, embed, block) = editing_task_block_with_body();
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n- one\n  body\n{}\n- two\n", embed));
    assert!(std::fs::read_to_string(&block).unwrap().ends_with("\n- task\n"));
}

#[test]
fn cutting_a_block_title_and_pasting_it_moves_the_block() {
    // the body left behind goes to the block above; the pasted title line
    // brings the block back with its tag
    let (d, mut app, embed, block) = editing_task_block_with_body();
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    // the cursor is on "  body"; paste above "- two"
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n- one\n  body\n{}\n- two\n", embed));
    assert!(std::fs::read_to_string(&block).unwrap().ends_with("\n- task\n"));
}

#[test]
fn paste_goes_into_the_open_find_line() {
    // §10.6: pasted text is "inserted as typed"; with Ctrl-F's find line open,
    // typing goes into the find line, so a paste must too
    let (d, mut app) = app_with("# A\n\nhello world\n");
    keys(&mut app, "e");
    draw(&mut app);
    for k in [
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::End, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
    ] {
        app.handle_key(k);
    }
    app.handle_paste("world");
    let s = draw(&mut app);
    keys(&mut app, "⎋⎋⎋");
    assert_eq!(root(&d), "# A\n\nhello world\n");
    assert!(s.contains("/world"), "{}", s);
}

#[test]
fn paste_goes_into_the_open_vim_command_line() {
    let (d, mut app) = app_with("# A\n\nhello world\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejj:");
    app.handle_paste("w\n");
    keys(&mut app, "⏎");
    // `:w` ran and the text is as it was
    assert_eq!(app.mode_pub(), "edit");
    keys(&mut app, ":q⏎");
    assert_eq!(root(&d), "# A\n\nhello world\n");
}

#[test]
fn helix_search_after_dotted_capital_i_selects_the_match() {
    // 'İ' lowercases to two chars ("i̇"); the match column must still be
    // counted in the real line, so `/ab` selects the "ab" at col 1
    let (d, mut app) = app_with("# A\n\nİab ab\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejj/ab⏎d:wq⏎");
    assert_eq!(root(&d), "# A\n\nİ ab\n");
}

#[test]
fn vim_search_after_dotted_capital_i_lands_on_the_match() {
    let (d, mut app) = app_with("# A\n\nİİ xab\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejj/ab⏎x:wq⏎");
    assert_eq!(root(&d), "# A\n\nİİ xb\n");
}

#[test]
fn helix_xd_above_a_block_keeps_the_block() {
    // §5.2: a deleted line takes its tag with it; "- two" keeps its block's tag
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjxd:w⏎");
    let r = root(&d);
    assert!(r.contains("![[") && !r.contains("- two"), "{}", r);
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
}

#[test]
fn vim_d_paragraph_above_a_block_keeps_the_block() {
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjd}:w⏎");
    let r = root(&d);
    assert!(r.contains("![[") && !r.contains("- two"), "{}", r);
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
}

#[test]
fn deleting_a_selected_line_above_a_block_keeps_the_block() {
    // normal keymap: Shift-Down selects "- one" and its line end; Delete
    // takes that line away and leaves the block's title line as it was
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    keys(&mut app, "e");
    for k in [
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
    ] {
        app.handle_key(k);
    }
    keys(&mut app, "⎋");
    let r = root(&d);
    assert!(r.starts_with("# A\n\n![[") && !r.contains("- "), "{}", r);
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
}
