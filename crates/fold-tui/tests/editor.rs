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
fn vim_dgj_above_a_block_keeps_the_block() {
    // `dgj` from column 0 of "- one" deletes up to column 0 of the block's
    // title line, exclusive: "- one" goes whole, with its tag, and the title
    // line stays the block's
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjdgj:w⏎");
    let r = root(&d);
    assert!(r.starts_with("# A\n\n![[") && !r.contains("- "), "{}", r);
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

#[test]
fn helix_x_tilde_keeps_line_owners() {
    // §5.2: the tag travels with the line; a case change over "- one\n"
    // must not re-tag the block's title line below it
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjx~:w⏎");
    assert!(root(&d).starts_with("# A\n\n- ONE\n![["), "{}", root(&d));
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
}

#[test]
fn vim_visual_line_tilde_keeps_line_owners() {
    // the case of each line changes in place, in the block that owns it
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjVj~:w⏎");
    assert!(root(&d).starts_with("# A\n\n- ONE\n![["), "{}", root(&d));
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before.replace("- two", "- TWO"));
}

#[test]
fn helix_tilde_from_mid_line_across_a_block_title_keeps_line_owners() {
    // a range that starts inside "- one" and ends inside the block's title
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjllvjl~:w⏎");
    assert!(root(&d).starts_with("# A\n\n- ONE\n![["), "{}", root(&d));
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before.replace("- two", "- TWo"));
}

#[test]
fn vim_case_toggle_keeps_sharp_s() {
    // 'ß' has no one-letter capital: like Vim, it stays as it is rather than
    // turning into the first letter of "SS"
    let (d, mut app) = app_with("# A\n\nstraße ﬁx\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjv$~:w⏎");
    assert_eq!(root(&d), "# A\n\nSTRAßE ﬁX\n");
}

/// root.md "# A\n\n- one\n![[b]]\n- two\n", block b "- b" holding block c
/// "- c" as its child; the editor open on A with the cursor on "  - c".
/// Returns the embeds of b and c and b's file.
fn editing_nested_blocks() -> (tempfile::TempDir, App, String, String, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n- b\n  - c\n- two\n").unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let c = v.find_by_path(&["A".into(), "b".into(), "c".into()]).unwrap();
    let c = fold_core::ops::make_block(&mut v, c).unwrap();
    let b = v.find_by_path(&["A".into(), "b".into()]).unwrap();
    let b = fold_core::ops::make_block(&mut v, b).unwrap();
    let b_file = v.dir.join(&v.tree.files[v.tree.block_by_id(&b).unwrap().0].path);
    drop(v);
    let (b, c) = (format!("![[{}]]", b.as_str()), format!("![[{}]]", c.as_str()));
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", b));
    assert!(std::fs::read_to_string(&b_file).unwrap().ends_with(&format!("\n- b\n  {}\n", c)));
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    keys(&mut app, "e");
    // the editor shows "# A", "", "- one", "- b", "  - c", "- two"
    for _ in 0..4 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    (d, app, b, c, b_file)
}

#[test]
fn moving_a_block_above_the_block_it_is_in_moves_its_embed_out() {
    // §5.2: the embed goes where the title line now sits, in A under "- one";
    // b's file must not be left with c's embed ahead of its own title
    let (d, mut app, b, c, b_file) = editing_nested_blocks();
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n- one\n  {}\n{}\n- two\n", c, b));
    assert!(std::fs::read_to_string(&b_file).unwrap().ends_with("\n- b\n"));
}

#[test]
fn moving_a_block_below_the_block_it_is_in_moves_its_embed_out() {
    // under "- two" it is A's, not b's: saved as it is shown
    let (d, mut app, b, c, b_file) = editing_nested_blocks();
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n  {}\n", b, c));
    assert!(std::fs::read_to_string(&b_file).unwrap().ends_with("\n- b\n"));
}

#[test]
fn cutting_a_block_out_of_the_block_it_is_in_and_pasting_it_moves_its_embed() {
    let (d, mut app, b, c, b_file) = editing_nested_blocks();
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    // the cursor is on "- two"; paste above "- one"
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n  {}\n- one\n{}\n- two\n", c, b));
    assert!(std::fs::read_to_string(&b_file).unwrap().ends_with("\n- b\n"));
}

#[test]
fn undoing_a_move_out_of_a_block_puts_the_embed_back_in_it() {
    let (d, mut app, b, c, b_file) = editing_nested_blocks();
    let before = std::fs::read_to_string(&b_file).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(root(&d), format!("# A\n\n- one\n  {}\n{}\n- two\n", c, b));
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", b));
    assert_eq!(std::fs::read_to_string(&b_file).unwrap(), before);
}

#[test]
fn helix_x_r_above_a_block_keeps_the_block() {
    // `r` over a whole line and its end replaces that line; the block's
    // title line below it keeps its tag (§5.2)
    let (d, block) = vault_with_bullet_block();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjxr-:w⏎");
    assert!(root(&d).starts_with("# A\n\n-----\n![["), "{}", root(&d));
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
}

#[test]
fn vim_dot_after_a_counted_dot_repeats_the_change() {
    let (d, mut app) = app_with("# A\n\na\nb\nc\nd\ne\nf\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjdd2.:w⏎");
    assert_eq!(root(&d), "# A\n\nd\ne\nf\n");
    // `.` repeats `dd` with the count 2 of the last `2.`; it must not replay `2.` into itself
    keys(&mut app, ".:w⏎");
    assert_eq!(root(&d), "# A\n\nf\n");
}

#[test]
fn vim_counted_undo_is_not_repeated_by_dot() {
    let (d, mut app) = app_with("# A\n\na\nb\nc\nd\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjdddddd2u:w⏎");
    assert_eq!(root(&d), "# A\n\nb\nc\nd\n");
    // `.` repeats the last change (`dd`), never the undo
    keys(&mut app, "ggjj.:w⏎");
    assert_eq!(root(&d), "# A\n\nc\nd\n");
}

#[test]
fn vim_cw_at_the_end_of_a_word_changes_only_that_word() {
    // a one-letter word: `cw` changes `a`, not `a cat`
    let (d, mut app) = app_with("# A\n\na cat\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjcwthe⎋:w⏎");
    assert_eq!(root(&d), "# A\n\nthe cat\n");
    // the last letter of a word: `cw` on the `e` of `one` changes `e`, not `e two`
    let (d, mut app) = app_with("# A\n\none two\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjllcwX⎋:w⏎");
    assert_eq!(root(&d), "# A\n\nonX two\n");
    // with a count, the first word is that character, as in Vim: `2cw` changes `e two`
    let (d, mut app) = app_with("# A\n\none two three\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjll2cwX⎋:w⏎");
    assert_eq!(root(&d), "# A\n\nonX three\n");
}

#[test]
fn vim_visual_line_put_replaces_the_line() {
    // `yy` on "a", then `V` on "b" and `p`: "b" becomes "a", no blank line is left behind.
    let (d, mut app) = app_with("# A\n\na\nb\nc\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjyyjVp:w⏎");
    assert_eq!(root(&d), "# A\n\na\na\nc\n");
}

#[test]
fn vim_visual_line_put_saves_the_replaced_line_whole() {
    // `V` + `p` puts the replaced line in the register as a whole line (with its newline),
    // so a later `2p` puts two lines "b", not one line "bb".
    let (d, mut app) = app_with("# A\n\na\nb\nc\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjyljVp");
    keys(&mut app, "2p:w⏎");
    assert_eq!(root(&d), "# A\n\na\na\nb\nb\nc\n");
}

#[test]
fn helix_repeated_w_selects_the_next_word() {
    // the second `w` selects "two ", not " two t" from the previous selection's end
    let (d, mut app) = app_with("# A\n\none two three\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjwwd:wq⏎");
    assert_eq!(root(&d), "# A\n\none three\n");
    // `2w` is the same
    let (d, mut app) = app_with("# A\n\none two three\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejj2wd:wq⏎");
    assert_eq!(root(&d), "# A\n\none three\n");
    // the last word of a line is selected without the line end, and the next
    // `w` selects the first word of the next line: nothing joins the lines
    let (d, mut app) = app_with("# A\n\none two\nthree four\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjwwd:wq⏎");
    assert_eq!(root(&d), "# A\n\none \nthree four\n");
    let (d, mut app) = app_with("# A\n\none two\nthree four\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjwwwd:wq⏎");
    assert_eq!(root(&d), "# A\n\none two\nfour\n");
}

#[test]
fn helix_repeated_b_selects_the_previous_word() {
    // after `glb` selects "three", a second `b` selects "two ", not "two t"
    let (d, mut app) = app_with("# A\n\none two three\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjglbbd:wq⏎");
    assert_eq!(root(&d), "# A\n\none three\n");
}

#[test]
fn helix_join_two_selected_lines() {
    // `xx` selects lines a and b; `J` joins those two, as Helix does, not c too
    let (d, mut app) = app_with("# A\n\na\nb\nc\nd\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejjxxJ:wq⏎");
    assert_eq!(root(&d), "# A\n\na b\nc\nd\n");
}

/// Vim: `d` + `gj`/`gk`/`ge` is one complete command — whatever it does, the
/// operator is no longer pending, so a following `j` only moves the cursor.
#[test]
fn vim_operator_with_g_motion_does_not_stay_pending() {
    let text = "# A\n\none\ntwo\nthree\nfour\n";
    let run = |seq: &str| {
        let (d, mut app) = app_with(text);
        app.set_edit_keys(fold_tui::app::EditKeys::Vim);
        keys(&mut app, "e");
        draw(&mut app);
        // cursor on "two"; Esc drops anything still pending before `:w`
        keys(&mut app, "jjj");
        keys(&mut app, seq);
        keys(&mut app, "⎋:w⏎");
        root(&d)
    };
    for m in ["gj", "gk", "ge"] {
        let without = run(&format!("d{}", m));
        let with_j = run(&format!("d{}j", m));
        assert_eq!(with_j, without, "`j` after `d{}` ran as an operator motion", m);
    }
    // the reported case: `dgj` then `j` must not also delete "three"
    assert!(run("dgjj").contains("three"), "{}", run("dgjj"));
    // and the operators act over the motions, charwise as in Vim: `gj`/`gk`
    // exclusive, `ge` (back to the end of the previous word) inclusive
    assert_eq!(run("dgj"), "# A\n\none\nthree\nfour\n");
    assert_eq!(run("dgk"), "# A\n\ntwo\nthree\nfour\n");
    assert_eq!(run("dge"), "# A\n\nonwo\nthree\nfour\n");
}

#[test]
fn vim_dot_after_deleting_a_dragged_selection_leaves_no_operator_pending() {
    let (d, mut app) = app_with("# A\n\none two\nthree\nfour\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "e");
    draw(&mut app);
    let s = draw(&mut app);
    let (y, line) = s.lines().enumerate().find(|(_, l)| l.contains("one two")).unwrap();
    let x = line.find("one two").map(|b| line[..b].chars().count()).unwrap() as u16;
    let y = y as u16;
    // drag over "one": a Vim drag is a visual selection
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), (x, y)));
    app.handle_mouse(ev(MouseEventKind::Drag(MouseButton::Left), (x + 2, y)));
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), (x + 2, y)));
    keys(&mut app, "d:w⏎");
    assert_eq!(root(&d), "# A\n\n two\nthree\nfour\n");
    // repeat, then move down: `j` must only move
    keys(&mut app, ".j:w⏎");
    let text = root(&d);
    assert!(text.contains("three") && text.contains("four"), "{:?}", text);
    // as in Vim, `.` repeats the delete over as much text as was selected
    assert_eq!(text, "# A\n\no\nthree\nfour\n");
}

#[test]
fn vim_dot_after_changing_a_double_clicked_word_changes_as_much_again() {
    let (d, mut app) = app_with("# A\n\none two three\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "e");
    draw(&mut app);
    let s = draw(&mut app);
    let (y, line) = s.lines().enumerate().find(|(_, l)| l.contains("one two")).unwrap();
    let x = line.find("one two").map(|b| line[..b].chars().count()).unwrap() as u16;
    let y = y as u16;
    // double-click "one", change it to "1", then `w.` changes "two" the same way
    for _ in 0..2 {
        app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left), (x, y)));
        app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left), (x, y)));
    }
    keys(&mut app, "c1⎋w.:w⏎");
    assert_eq!(root(&d), "# A\n\n1 1 three\n");
}

#[test]
fn helix_shift_n_selects_the_previous_match() {
    // `/foo` from the start of the line selects the second "foo"; `N` must go
    // back to the first one, not reselect the match it is standing on
    let (d, mut app) = app_with("# A\n\nfoo bar foo\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejj/foo⏎Nd:wq⏎");
    assert_eq!(root(&d), "# A\n\n bar foo\n");
}

#[test]
fn helix_repeated_backward_search_selects_the_previous_match() {
    // `?foo` from the start of the line wraps to the second "foo"; a repeated
    // `?` must then select the first one
    let (d, mut app) = app_with("# A\n\nfoo bar foo\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Helix);
    keys(&mut app, "ejj?foo⏎?⏎d:wq⏎");
    assert_eq!(root(&d), "# A\n\n bar foo\n");
}

#[test]
fn vim_brace_in_last_paragraph_takes_the_last_line() {
    // No blank line after the paragraph (the usual end of an edited subtree):
    // Vim's `}` lands on the last character and is inclusive, so `d}` from the
    // first line deletes the whole paragraph, last line included.
    let (d, mut app) = app_with("# A\n\na\nb\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjd}:w⏎");
    assert_eq!(root(&d).trim_end(), "# A", "{:?}", root(&d));
    // with a blank line after it, `}` stops there (column 0, exclusive)
    let (d, mut app) = app_with("# A\n\na\nb\n\nc\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjd}:w⏎");
    assert_eq!(root(&d), "# A\n\n\nc\n");
}

#[test]
fn vim_count_before_operator_applies_to_find() {
    // `2df-` is `d2f-`: a count typed before the operator multiplies into f/t/F/T
    let (d, mut app) = app_with("# A\n\na-b-c-d\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejj2df-:w⏎");
    assert_eq!(root(&d), "# A\n\nc-d\n");
    // and into `gg`, whose count is a line: `3dgg` from the last line deletes lines 3 to 5
    let (d, mut app) = app_with("# A\n\na\nb\nc\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "eG3dgg:w⏎");
    assert_eq!(root(&d), "# A\n");
}

#[test]
fn vim_huge_count_put_does_not_panic() {
    let (d, mut app) = app_with("# A\n\nx\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    // usize::MAX copies of the yanked "x": today `str::repeat` panics with
    // "capacity overflow" instead of the count being clamped
    keys(&mut app, "ejjyl18446744073709551615p");
    draw(&mut app);
    keys(&mut app, "⎋:q!⏎");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# A\n\nx\n");
}

/// A count far past the end of the text acts like one that just reaches it:
/// the motion or command stops there, rather than looping on (or
/// overflowing) for billions of steps.
#[test]
fn huge_counts_stop_at_the_end_of_the_text() {
    let huge = "18446744073709551615";
    let text = "# A\n\none two\n    three four\n\nfive\n";
    let run = |keymap: fold_tui::app::EditKeys, seq: &str| {
        let (d, mut app) = app_with(text);
        app.set_edit_keys(keymap);
        keys(&mut app, "ejj");
        keys(&mut app, seq);
        keys(&mut app, "⎋:w⏎");
        root(&d)
    };
    // (`#` stands for the count; nothing here edits the title line)
    let vim = [
        "#wx", "#bjjx", "#ex", "#}x", "G#{jjx", "x#.", "xx#u", "#lx", "#$x", "2d#w", "#d#w", "#dd", "#J", "#~", "#rz",
        "#x", "#s-", "#S-", "#D", "#C-", "#Yp", "#>>", "#jx", "#Gx", "G#ggjjx",
    ];
    for s in vim {
        let small = run(fold_tui::app::EditKeys::Vim, &s.replace('#', "20"));
        assert_eq!(run(fold_tui::app::EditKeys::Vim, &s.replace('#', huge)), small, "vim {}", s);
    }
    let helix = ["#ed", "#wd", "#b;jjd", "#ld", "#xd", "xd#u", "xdu#U", "x>>#<"];
    for s in helix {
        let small = run(fold_tui::app::EditKeys::Helix, &s.replace('#', "20"));
        assert_eq!(run(fold_tui::app::EditKeys::Helix, &s.replace('#', huge)), small, "helix {}", s);
    }
    // indenting by a count is one step, however deep; too deep is refused
    assert_eq!(run(fold_tui::app::EditKeys::Helix, "3>u"), text);
    assert_eq!(run(fold_tui::app::EditKeys::Helix, &format!("{}>", huge)), text);
}

#[test]
fn typing_after_n_titles_the_new_node() {
    // §10.6: `n` / `N` put the cursor on the new node in the editor, so the
    // first thing typed is its title, after the marker
    let (d, mut app) = app_with("- a\n");
    keys(&mut app, "nb⎋");
    assert_eq!(root(&d), "- a\n- b\n");
    let (d, mut app) = app_with("# A\n");
    keys(&mut app, "nB⎋");
    assert_eq!(root(&d), "# A\n\n# B\n");
    let (d, mut app) = app_with("- a\n");
    keys(&mut app, "Nc⎋");
    let text = root(&d);
    assert!(text.starts_with("- a\n") && text.lines().any(|l| l == "  - c"), "{:?}", text);
    // Vim and Helix start typing too, as after `o`
    for k in [fold_tui::app::EditKeys::Vim, fold_tui::app::EditKeys::Helix] {
        let (d, mut app) = app_with("- a\n");
        app.set_edit_keys(k);
        keys(&mut app, "nb⎋:wq⏎");
        assert_eq!(root(&d), "- a\n- b\n");
        assert_eq!(app.mode_pub(), "normal");
    }
    // a new node left untitled stays as it was written
    let (d, mut app) = app_with("- a\n");
    keys(&mut app, "n⎋");
    assert_eq!(root(&d), "- a\n-\n");
}

#[test]
fn typing_after_an_external_change_to_the_edited_file_is_saved() {
    // §11.2: an external change while editing re-renders the buffer, so a
    // later save is not refused and the typed text is never thrown away
    let (d, mut app) = app_with("# A\n\nbody\n\n# B\n\nother\n");
    keys(&mut app, "e");
    std::fs::write(d.path().join("root.md"), "# A\n\nbody\n\n# B\n\nother, from Helix\n").unwrap();
    app.reload_external();
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    keys(&mut app, "X⎋");
    let t = root(&d);
    assert!(t.contains("# AX") && t.contains("other, from Helix"), "{}", t);
    assert_eq!(app.mode_pub(), "normal");
}

#[test]
fn a_refused_save_keeps_the_editor_open_with_its_text() {
    // a save that fails never drops the buffer: the edited node's text
    // changed on disk under typed text (§5.2 step 5), and leaving the
    // editor keeps it open with the text
    let (d, mut app) = app_with("# A\n\nbody\n\n# B\n\nother\n");
    keys(&mut app, "e");
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    keys(&mut app, "X");
    std::fs::write(d.path().join("root.md"), "# A\n\nbody, from Helix\n\n# B\n\nother\n").unwrap();
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "edit", "the editor stays open");
    assert!(app.editor_dirty(), "with the typed text still in it");
    assert!(draw(&mut app).contains("# AX"));
}

fn ctrl(app: &mut App, c: char) {
    app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

#[test]
fn undoing_a_saved_cut_of_a_block_title_puts_the_block_back() {
    // the cut title line is in the clipboard, so a save between the cut and
    // the undo leaves the block alone (§5.2); undo puts it back, embed and all
    let (d, mut app, embed, block) = editing_task_block_with_body();
    let before = std::fs::read_to_string(&block).unwrap();
    ctrl(&mut app, 'k');
    ctrl(&mut app, 's');
    assert_eq!(root(&d), "# A\n\n- one\n  body\n- two\n");
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
    ctrl(&mut app, 'z');
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", embed));
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
}

#[test]
fn a_cut_block_title_not_pasted_back_deletes_the_block() {
    // §5.2: deleting a nested block's title line deletes the block; a cut one
    // is deleted once it can no longer be pasted back, when the editor
    // closes or when something else is copied over it
    let (d, mut app, _, block) = editing_task_block_with_body();
    ctrl(&mut app, 'k');
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# A\n\n- one\n  body\n- two\n");
    assert!(!block.exists());
    let (d, mut app, _, block) = editing_task_block_with_body();
    ctrl(&mut app, 'k');
    // the cursor is on "  body": copy it
    ctrl(&mut app, 'c');
    ctrl(&mut app, 's');
    assert_eq!(root(&d), "# A\n\n- one\n  body\n- two\n");
    assert!(!block.exists());
    // the clipboard holds "  body" alone, pasted as a copy above the cursor
    ctrl(&mut app, 'v');
    keys(&mut app, "⎋");
    assert_eq!(root(&d), "# A\n\n- one\n  body\n  body\n- two\n");
}

#[test]
fn a_line_cut_before_a_reload_and_pasted_after_it_is_saved() {
    // another program changes the file while a nested block's title line is
    // cut: the editor is re-rendered over the files as they are (§11.2), and
    // the pasted line is text of the block it lands in, not a line of a
    // block the new buffer does not have, written nowhere
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n- task\n- two\n\n# B\n\nb\n").unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    fold_core::ops::make_block(&mut v, t).unwrap();
    drop(v);
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    keys(&mut app, "e");
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    ctrl(&mut app, 'k');
    ctrl(&mut app, 's');
    assert_eq!(root(&d), "# A\n\n- one\n- two\n\n# B\n\nb\n");
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n- two\n\n# B\n\nb, from Helix\n").unwrap();
    app.reload_external();
    ctrl(&mut app, 'v');
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# A\n\n- one\n- task\n- two\n\n# B\n\nb, from Helix\n");
}

#[test]
fn vim_t_next_to_its_target_stays_put() {
    let run = |keymap: fold_tui::app::EditKeys, text: &str, seq: &str| {
        let (d, mut app) = app_with(&format!("# A\n\n{}\n", text));
        app.set_edit_keys(keymap);
        keys(&mut app, "ejj");
        keys(&mut app, seq);
        keys(&mut app, "⎋:w⏎");
        root(&d).trim_start_matches("# A\n\n").trim_end().to_string()
    };
    let vim = |text: &str, seq: &str| run(fold_tui::app::EditKeys::Vim, text, seq);
    // Vim: `t)` with `)` right after the cursor lands on the cursor, so `dt)`
    // deletes the one character before `)`
    assert_eq!(vim("foo(a) bar(b)", "fadt)"), "foo() bar(b)");
    // `T(` with `(` right before it does not move: `dT(` deletes nothing
    assert_eq!(vim("(b(a)", "fadT("), "(b(a)");
    // only `;` and `,` repeating a `t`/`T` skip a target next to the cursor,
    // and only without a count (Vim's 'cpoptions' without `;`)
    assert_eq!(vim("a)b)c)", "t);x"), "a))c)");
    assert_eq!(vim("(a(b(c", "$T(;x"), "(a((c");
    assert_eq!(vim("a(b(c(d", "fct(,x"), "a((c(d");
    assert_eq!(vim("a)b)c)d", "t)2;x"), "a))c)d");
    // Helix's `t` always selects up to the next target it does not touch
    assert_eq!(run(fold_tui::app::EditKeys::Helix, "a)b)", "t)d"), ")");
}

#[test]
fn deleting_a_block_title_line_in_vim_or_helix_and_putting_it_back_moves_the_block() {
    // §5.2: moving a nested block's title line (cut and paste in the editor)
    // moves its embed, in every keymap: Vim's `dd` then `P`/`p`, Helix's
    // `xd` then `P`/`p`
    for (keymap, seq, moved_up) in [
        (fold_tui::app::EditKeys::Vim, "ddkP:w⏎", true),
        (fold_tui::app::EditKeys::Vim, "ddp:w⏎", false),
        (fold_tui::app::EditKeys::Helix, "xdkP:w⏎", true),
        (fold_tui::app::EditKeys::Helix, "xdp:w⏎", false),
    ] {
        let (d, mut app, embed, block) = editing_task_block_with_body();
        let before = std::fs::read_to_string(&block).unwrap();
        app.set_edit_keys(keymap);
        keys(&mut app, seq);
        let want = if moved_up {
            format!("# A\n\n{}\n- one\n  body\n- two\n", embed)
        } else {
            format!("# A\n\n- one\n  body\n{}\n- two\n", embed)
        };
        assert_eq!(root(&d), want, "{:?} {}", keymap, seq);
        // the body left behind is the parent's; the block keeps its title
        assert_eq!(std::fs::read_to_string(&block).unwrap(), before.replace("  body\n", ""));
    }
    // a line yanked and put is a copy, of the block above it: the block is
    // not duplicated
    for (keymap, seq) in [(fold_tui::app::EditKeys::Vim, "yyjp:w⏎"), (fold_tui::app::EditKeys::Helix, "xyjp:w⏎")] {
        let (d, mut app, embed, block) = editing_task_block_with_body();
        let before = std::fs::read_to_string(&block).unwrap();
        app.set_edit_keys(keymap);
        keys(&mut app, seq);
        assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", embed), "{:?} {}", keymap, seq);
        assert_eq!(std::fs::read_to_string(&block).unwrap(), before.replace("  body\n", "  body\n- task\n"));
    }
}

#[test]
fn replacing_characters_from_mid_line_into_a_block_title_keeps_line_owners() {
    // helix `r` and vim visual `r` over "one" and the "-" of the block's
    // title line below: each character is replaced where it is, and the
    // title line stays the block's (§5.2)
    for (keymap, seq) in [(fold_tui::app::EditKeys::Helix, "ejjllvjhhr-:w⏎"), (fold_tui::app::EditKeys::Vim, "ejjllvjhhr-:w⏎")] {
        let (d, block) = vault_with_bullet_block();
        let before = std::fs::read_to_string(&block).unwrap();
        let mut app = App::new(d.path()).unwrap();
        app.set_edit_keys(keymap);
        keys(&mut app, seq);
        assert!(root(&d).starts_with("# A\n\n- ---\n![["), "{:?}: {}", keymap, root(&d));
        assert_eq!(std::fs::read_to_string(&block).unwrap(), before, "{:?}", keymap);
    }
}

#[test]
fn a_cursor_key_in_insert_mode_starts_a_new_undo_step() {
    // Vim: what is typed before and after an arrow key, Home or End in
    // insert mode is undone separately; Helix's insert mode does the same
    for keymap in [fold_tui::app::EditKeys::Vim, fold_tui::app::EditKeys::Helix] {
        for k in [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down, KeyCode::Home, KeyCode::End] {
            let (d, mut app) = app_with("# A\n\nx\n");
            app.set_edit_keys(keymap);
            keys(&mut app, "ejjAone");
            app.handle_key(KeyEvent::new(k, KeyModifiers::NONE));
            keys(&mut app, "two⎋u:w⏎");
            assert_eq!(root(&d), "# A\n\nxone\n", "{:?} {:?}", keymap, k);
        }
    }
}

#[test]
fn undoing_a_save_that_took_in_another_programs_change_keeps_that_change() {
    // §5.2 step 5: the save keeps a change made outside the edited node;
    // undoing the save takes back only the typing (§10.10)
    let (d, mut app) = app_with("# A\n\nbody\n\n# B\n\nother\n");
    keys(&mut app, "e");
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    keys(&mut app, "X");
    std::fs::write(d.path().join("root.md"), "# A\n\nbody\n\n# B\n\nother, from Helix\n").unwrap();
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# AX\n\nbody\n\n# B\n\nother, from Helix\n");
    keys(&mut app, "u");
    assert_eq!(root(&d), "# A\n\nbody\n\n# B\n\nother, from Helix\n");
}

#[test]
fn a_cut_block_title_is_deleted_when_a_reload_re_renders_the_editor() {
    // the re-rendered editor cannot paste the line back as the block, so the
    // block is deleted then (§5.2), not left in a file embedded nowhere
    let (d, mut app, _, block) = editing_task_block_with_body();
    ctrl(&mut app, 'k');
    ctrl(&mut app, 's');
    assert!(block.exists(), "in transit");
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n  body\n- two\n\n# C\n").unwrap();
    app.reload_external();
    assert!(!block.exists());
    assert_eq!(root(&d), "# A\n\n- one\n  body\n- two\n\n# C\n");
}

#[test]
fn review_undoing_a_saved_block_title_deletion_keeps_the_block_nested_in_it() {
    // §5.2: re-spelling b's title line as plain text deletes b: the save
    // embeds c (nested in b) in A and trashes b's file. Undo puts "- b"
    // back as A's plain text; the next save must keep c embedded, not write
    // an embed of the trashed b in its place (leaving c embedded nowhere)
    let (d, mut app, b, c, b_file) = editing_nested_blocks();
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    ctrl(&mut app, 's');
    assert_eq!(root(&d), format!("# A\n\n- one\nb\n  {}\n- two\n", c));
    assert!(!b_file.exists());
    ctrl(&mut app, 'z');
    keys(&mut app, "⎋");
    let r = root(&d);
    assert!(!r.contains(&b) && r.contains(&c), "b = {} (trashed), c = {}; root.md:\n{}", b, c, r);
}

#[test]
fn review_undo_after_a_save_that_deleted_a_block_keeps_the_block_in_it() {
    // b's title line joined onto "- one" and saved: b is deleted, its file
    // trashed, and c's embed goes to A (§5.2). Undo in the editor puts the
    // text back; b's lines are A's text now, and A must embed c, not the
    // trashed b
    let (d, mut app, b, c, b_file) = editing_nested_blocks();
    for k in [
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
    ] {
        app.handle_key(k);
    }
    ctrl(&mut app, 's');
    assert_eq!(root(&d), format!("# A\n\n- one- b\n  {}\n- two\n", c));
    assert!(!b_file.exists(), "b's file is trashed");
    ctrl(&mut app, 'z');
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), format!("# A\n\n- one\n- b\n  {}\n- two\n", c), "b is {}, trashed", b);
}

#[test]
fn a_refused_close_keeps_a_cut_block_in_transit() {
    for reload in [false, true] {
        a_refused_save_keeps_a_cut_block_in_transit(reload);
    }
}

/// On Esc, or before a reload (which saves the editor first, §11.2).
fn a_refused_save_keeps_a_cut_block_in_transit(reload: bool) {
    // A embeds "cut" (racfer, with a due date) and "other" (dozzod)
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("root.md"),
        "# A\n\n- one\n![[racfer-hattes-mislup-nodrys]]\n![[dozzod-binwes-talsun-worbec]]\n- two\n",
    )
    .unwrap();
    let cut = d.path().join("racfer~cut.md");
    std::fs::write(&cut, "---\nid: racfer-hattes-mislup-nodrys\ndue: 2026-10-01\n---\n\n- cut\n").unwrap();
    let other = d.path().join("dozzod~other.md");
    std::fs::write(&other, "---\nid: dozzod-binwes-talsun-worbec\n---\n\n- other\n").unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    keys(&mut app, "e");
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    // cut "- cut": the block is in transit while the line is in the clipboard
    ctrl(&mut app, 'k');
    ctrl(&mut app, 's');
    assert!(cut.exists(), "in transit");
    // type into "other", which another program changes meanwhile
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    keys(&mut app, "X");
    std::fs::write(&other, "---\nid: dozzod-binwes-talsun-worbec\n---\n\n- other, from Helix\n").unwrap();
    // Esc: the save is refused, so the editor stays open with its text, and
    // the cut line can still be pasted back as the block
    if reload {
        app.reload_external();
    } else {
        keys(&mut app, "⎋");
    }
    assert_eq!(app.mode_pub(), "edit", "still editing");
    assert!(cut.exists(), "a block cut in an editor still open was trashed (reload: {})", reload);
}

#[test]
fn review_reverting_after_a_saved_cut_leaves_no_block_file_embedded_nowhere() {
    // the cut title line is in transit across the save (§5.2): A is written
    // without the embed and the block's file kept. Revert (`:q!`) leaves the
    // editor; the block must then be deleted, as on any other way out, or
    // still be embedded, never a file that nothing embeds
    let (d, mut app, embed, block) = editing_task_block_with_body();
    ctrl(&mut app, 'k');
    ctrl(&mut app, 's');
    assert_eq!(root(&d), "# A\n\n- one\n  body\n- two\n");
    ctrl(&mut app, 'e');
    keys(&mut app, "q!⏎");
    assert_eq!(app.mode_pub(), "normal");
    let r = root(&d);
    assert!(!block.exists() || r.contains(&embed), "{} exists, root.md is {:?}", block.display(), r);
}

#[test]
fn indenting_a_block_title_line_nests_the_block_where_the_line_sits() {
    // §5.2: Tab at the start of a nested block's title line, or Vim `>>`,
    // nests the block under "- one" — its embed goes where the line sits,
    // the block and its file stay — and Shift-Tab takes it back out
    let task_file = |d: &tempfile::TempDir| {
        std::fs::read_dir(d.path()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with("~task.md"))
    };
    let (d, mut app, embed) = editing_task_block();
    app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    ctrl(&mut app, 's');
    assert_eq!(root(&d), format!("# A\n\n- one\n  {}\n- two\n", embed));
    assert!(task_file(&d));
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    keys(&mut app, "⎋");
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", embed));
    assert!(task_file(&d));
    let (d, mut app, embed) = editing_task_block();
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, ">>:w⏎");
    assert_eq!(root(&d), format!("# A\n\n- one\n  {}\n- two\n", embed));
    assert!(task_file(&d));
}

/// root.md "# A\n\n- one\n![[task]]\n\na\nb\n"; returns the embed and the
/// block's file.
fn vault_with_block_then_text() -> (tempfile::TempDir, String, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n- task\n\na\nb\n").unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = fold_core::ops::make_block(&mut v, t).unwrap();
    let block = v.dir.join(&v.tree.files[v.tree.block_by_id(&id).unwrap().0].path);
    drop(v);
    let embed = format!("![[{}]]", id.as_str());
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n\na\nb\n", embed));
    (d, embed, block)
}

#[test]
fn vim_put_below_an_empty_line_after_a_block_stays_in_the_parent() {
    // §5.2: a line pasted somewhere takes the tag of the line above it; `p`
    // on the blank line after the block puts "a" and "b" below that blank
    // line, which is A's, so they are A's text, not the block's
    let (d, embed, block) = vault_with_block_then_text();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    // lines: "# A", "", "- one", "- task", "", "a", "b"
    keys(&mut app, "ejjjjjyjkp:w⏎");
    let r = root(&d);
    let b = std::fs::read_to_string(&block).unwrap();
    assert_eq!((r, b), (format!("# A\n\n- one\n{}\n\na\nb\na\nb\n", embed), before));
    // `o` on that blank line opens a line of A below it, as Enter there does
    // in the normal keymap
    let (d, embed, block) = vault_with_block_then_text();
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    keys(&mut app, "ejjjjonew⎋:w⏎");
    let r = root(&d);
    let b = std::fs::read_to_string(&block).unwrap();
    assert_eq!((r, b), (format!("# A\n\n- one\n{}\n\nnew\na\nb\n", embed), before));
}

#[test]
fn deleting_whole_lines_up_to_a_block_titles_end_deletes_the_block() {
    // normal keymap: "- one" and "- task" selected from column 0 to the end
    // of "- task" and deleted: the block's title line is deleted, so the
    // block is (§5.2), and leaving the editor saves
    let (d, mut app, _embed) = editing_task_block();
    for k in [
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::End, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
    ] {
        app.handle_key(k);
    }
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "normal", "{}", draw(&mut app));
    assert_eq!(root(&d), "# A\n\n\n- two\n");
}

#[test]
fn cutting_a_block_title_and_pasting_it_back_in_place_keeps_its_nested_block() {
    // Alt-Down then Alt-Up, or Ctrl-K then Ctrl-V (pasted straight back above
    // "  - c") on b's title line: the text and every line's place are as they
    // were, so c's embed must still be in b's file, not moved out to A (§5.2)
    let cut_paste = [KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL), KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL)];
    let down_up = [KeyEvent::new(KeyCode::Down, KeyModifiers::ALT), KeyEvent::new(KeyCode::Up, KeyModifiers::ALT)];
    for seq in [down_up, cut_paste] {
        let (d, mut app, b, c, b_file) = editing_nested_blocks();
        let before = std::fs::read_to_string(&b_file).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        for k in seq {
            app.handle_key(k);
        }
        keys(&mut app, "⎋");
        assert_eq!(app.mode_pub(), "normal");
        let r = root(&d);
        let bf = std::fs::read_to_string(&b_file).unwrap();
        assert_eq!((r, bf), (format!("# A\n\n- one\n{}\n- two\n", b), before), "{:?}; c is {}", seq[0], c);
    }
    // Vim's `ddP` and Helix's `xdP` on b's title line put it straight back too
    for (keymap, seq) in [(fold_tui::app::EditKeys::Vim, "kddP:wq⏎"), (fold_tui::app::EditKeys::Helix, "kxdP:wq⏎")] {
        let (d, mut app, b, _c, b_file) = editing_nested_blocks();
        let before = std::fs::read_to_string(&b_file).unwrap();
        app.set_edit_keys(keymap);
        keys(&mut app, seq);
        assert_eq!(app.mode_pub(), "normal");
        let bf = std::fs::read_to_string(&b_file).unwrap();
        assert_eq!((root(&d), bf), (format!("# A\n\n- one\n{}\n- two\n", b), before), "{:?}", keymap);
    }
}

#[test]
fn moving_a_block_under_another_blocks_title_moves_its_embed_into_that_block() {
    // §5.2: the embed goes to the block the title line now sits in; "  - c"
    // moved up from under "- e" to under "- b" is indented past b's title
    // line, so it is b's now, and its embed goes into b's file
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- one\n- b\n- e\n  - c\n- two\n").unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let mut block = |path: &[&str]| {
        let n = v.find_by_path(&path.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        let id = fold_core::ops::make_block(&mut v, n).unwrap();
        (format!("![[{}]]", id.as_str()), v.dir.join(&v.tree.files[v.tree.block_by_id(&id).unwrap().0].path))
    };
    let (c, _) = block(&["A", "e", "c"]);
    let (e, e_file) = block(&["A", "e"]);
    let (b, b_file) = block(&["A", "b"]);
    drop(v);
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n{}\n- two\n", b, e));
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    keys(&mut app, "e");
    // the editor shows "# A", "", "- one", "- b", "- e", "  - c", "- two"
    for _ in 0..5 {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n{}\n- two\n", b, e));
    assert!(std::fs::read_to_string(&b_file).unwrap().ends_with(&format!("\n- b\n  {}\n", c)));
    assert!(std::fs::read_to_string(&e_file).unwrap().ends_with("\n- e\n"));
}

#[test]
fn ctrl_x_of_a_selected_block_title_line_then_ctrl_v_moves_the_block() {
    // normal keymap: Shift-Down selects the whole "- task" line; Ctrl-X then
    // Ctrl-V below "  body" moves the line, as Ctrl-K then Ctrl-V does
    // (§5.2: cut and paste moves its embed)
    let (d, mut app, embed, block) = editing_task_block_with_body();
    for k in [
        KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
    ] {
        app.handle_key(k);
    }
    keys(&mut app, "⎋");
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), format!("# A\n\n- one\n  body\n{}\n- two\n", embed));
    assert!(std::fs::read_to_string(&block).unwrap().ends_with("\n- task\n"));
}
