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

#[test]
fn an_embed_cycle_does_not_overflow_the_stack() {
    // §6.2: a cycle is a diagnostic, so a vault may contain one. The outline
    // shows the block once and skips the repeat visit, as walk/render do.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "![[racfer-hattes-mislup-nodrys]]\n").unwrap();
    std::fs::write(
        dir.path().join("racfer~loop.md"),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- Loop\n  ![[racfer-hattes-mislup-nodrys]]\n",
    )
    .unwrap();
    let mut app = App::new(dir.path()).unwrap();
    draw(&mut app, 100, 24);
    let rows = app.rows();
    assert_eq!(current_title(&app), "Loop");
    let loops = rows.iter().filter(|r| app.title_of(r.nref) == "Loop").count();
    assert_eq!(loops, 1);
    app.show_reading = true;
    press(&mut app, "lj");
    draw(&mut app, 100, 24);
}

#[test]
fn deleting_the_zoomed_node_zooms_out() {
    let (d, mut app) = app_with("# A\n\n# B\n\n- b1\n");
    press(&mut app, "j");
    app.handle_key(key(KeyCode::Enter)); // zoom into B; the cursor is on B
    assert_eq!(current_title(&app), "B");
    press(&mut app, "d"); // B is gone: the zoom must not dangle
    assert_eq!(root(&d), "# A\n");
    assert_eq!(current_title(&app), "A");
    draw(&mut app, 100, 24);
    // a zoomed node inside another zooms out to it
    let (d, mut app) = app_with("# A\n\n## B\n\n# C\n");
    press(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    press(&mut app, "d");
    assert_eq!(root(&d), "# A\n\n# C\n");
    let titles: Vec<String> = app.rows().iter().map(|r| app.title_of(r.nref)).collect();
    assert_eq!(titles, ["A"]);
}

#[test]
fn undoing_under_a_zoom_keeps_the_zoom() {
    let (_d, mut app) = app_with("- a\n- b\n");
    press(&mut app, "ypG"); // paste a copy of a; the cursor is on b
    app.handle_key(key(KeyCode::Enter)); // zoom into b
    assert_eq!(current_title(&app), "b");
    press(&mut app, "u"); // the file shrinks back to a, b
    assert_eq!(current_title(&app), "b");
    press(&mut app, "U");
    assert_eq!(current_title(&app), "b");
    draw(&mut app, 100, 24);
}

#[test]
fn moving_the_zoomed_node_keeps_the_zoom_on_it() {
    let (d, mut app) = app_with("- A\n- B\n  - B1\n");
    press(&mut app, "j");
    app.handle_key(key(KeyCode::Enter)); // zoom into B
    press(&mut app, "K");
    assert_eq!(root(&d), "- B\n  - B1\n- A\n");
    assert_eq!(current_title(&app), "B");
    let titles: Vec<String> = app.rows().iter().map(|r| app.title_of(r.nref)).collect();
    assert_eq!(titles, ["B", "B1"]);
}

#[test]
fn a_dismissed_reading_pane_menu_does_not_retarget_outline_verbs() {
    // B is inside the folded A, so it has no outline row
    let (d, mut app) = app_with("# A\n\n## B\n\n# C\n");
    press(&mut app, "h");
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "jjm"); // the node menu of B, from its line in the pane
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "j");
    assert_eq!(current_title(&app), "C");
    press(&mut app, "d");
    assert_eq!(root(&d), "# A\n\n## B\n");
}

#[test]
fn new_sibling_of_the_zoomed_node_opens_the_new_node() {
    let (d, mut app) = app_with("# A\n\n# B\n\n- b1\n");
    press(&mut app, "j");
    app.handle_key(key(KeyCode::Enter)); // zoom into B, the cursor on B (row 0)
    assert_eq!(current_title(&app), "B");
    press(&mut app, "n");
    // the empty sibling is written after B's subtree...
    assert_eq!(root(&d), "# A\n\n# B\n\n- b1\n\n#\n");
    // ...and it, not B, is what the editor opens on (§10.3 `n`), with the
    // zoom widened so that it is on screen
    assert_eq!(app.mode_pub(), "edit");
    assert_eq!(current_title(&app), "");
    press(&mut app, "C");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), "# A\n\n# B\n\n- b1\n\n# C\n");
    assert_eq!(current_title(&app), "C");
    // a sibling of an embedded block lands after its embed
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), "- Homelab\n  ![[racfer-hattes-mislup-nodrys]]\n").unwrap();
    std::fs::write(
        d.path().join("racfer~zfs.md"),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- ZFS\n",
    )
    .unwrap();
    let mut app = App::new(d.path()).unwrap();
    press(&mut app, "j");
    assert_eq!(current_title(&app), "ZFS");
    press(&mut app, "nX");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), "- Homelab\n  ![[racfer-hattes-mislup-nodrys]]\n  - X\n");
    assert_eq!(current_title(&app), "X");
}

#[test]
fn filter_pick_inside_a_block_unfolds_the_embedding_ancestors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "- Homelab\n  ![[racfer-hattes-mislup-nodrys]]\n").unwrap();
    std::fs::write(
        dir.path().join("racfer~zfs.md"),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- ZFS\n  - snapshots\n",
    )
    .unwrap();
    let mut app = App::new(dir.path()).unwrap();
    assert_eq!(current_title(&app), "Homelab");
    press(&mut app, "h"); // fold Homelab
    press(&mut app, "/snapshots");
    app.handle_key(key(KeyCode::Enter));
    // §10.5: the pick unfolds its ancestors (through the embed) and selects it
    assert_eq!(current_title(&app), "snapshots");
}

#[test]
fn backspace_from_a_zoomed_block_goes_to_its_outline_parent() {
    // §10.3 `Backspace`: zoom out to parent. A block root's parent in the
    // outline is the node that embeds it (as the breadcrumbs show), not
    // its own file's root.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("root.md"),
        "- Homelab\n  - NAS\n    ![[racfer-hattes-mislup-nodrys]]\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("racfer~zfs.md"),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- ZFS\n  - snapshots\n",
    )
    .unwrap();
    let mut app = App::new(dir.path()).unwrap();
    let titles = |app: &App| -> Vec<String> { app.rows().iter().map(|r| app.title_of(r.nref)).collect() };
    press(&mut app, "jj");
    assert_eq!(current_title(&app), "ZFS");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(titles(&app), ["ZFS", "snapshots"]);
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(titles(&app), ["NAS", "ZFS", "snapshots"]);
    assert_eq!(current_title(&app), "ZFS");
    // the same from the reading pane
    app.show_reading = true;
    app.handle_key(key(KeyCode::Enter)); // zoom into ZFS; the pane takes focus
    assert_eq!(titles(&app), ["ZFS", "snapshots"]);
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(titles(&app), ["NAS", "ZFS", "snapshots"]);
}

#[test]
fn enter_on_a_task_heading_in_the_reading_pane_zooms() {
    // SPEC §10.4: Enter on a heading zooms into it; `x` toggles a task heading
    let (d, mut app) = app_with("# A\n\n## [ ] T\n\nbody\n\n- [ ] i\n");
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "jj");
    assert!(app.reading_doc_pub().lines[app.read_cursor_pub()].contains("[ ] T"));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(root(&d), "# A\n\n## [ ] T\n\nbody\n\n- [ ] i\n", "Enter must not toggle the task");
    let doc = app.reading_doc_pub();
    assert!(doc.lines[0].contains("T"), "zoomed into T: {:?}", doc.lines);
    // on a task item, Enter still toggles
    let i = doc.lines.iter().position(|l| l.contains("[ ] i")).unwrap();
    for _ in 0..i {
        press(&mut app, "j");
    }
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(root(&d), "# A\n\n## [ ] T\n\nbody\n\n- [x] i\n");
}

/// Z zoomed, in the file after block B's: trashing B's file renumbers Z's.
fn zoomed_on_z_over_b() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "- Top\n  ![[racfer-hattes-mislup-nodrys]]\n").unwrap();
    std::fs::write(
        dir.path().join("racfer~z.md"),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- Z\n  - c\n    ![[dozzod-binwes-talsun-worbec]]\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("dozzod~b.md"),
        "---\nid: dozzod-binwes-talsun-worbec\n---\n\n- B\n  - b1\n",
    )
    .unwrap();
    let mut app = App::new(dir.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    press(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    (dir, app)
}

fn titles(app: &App) -> Vec<String> {
    app.rows().iter().map(|r| app.title_of(r.nref)).collect()
}

#[test]
fn an_editor_save_keeps_the_zoom_on_its_node() {
    let shift = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
    // deleting a nested block's title line trashes its file (§5.2); here
    // the cursor lands on a line of the enclosing block, which splices B
    let (d, mut app) = zoomed_on_z_over_b();
    assert_eq!(titles(&app), ["Z", "c", "B", "b1"]);
    press(&mut app, "je"); // edit c: "- c", "  - B", "    - b1"
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Home));
    app.handle_key(shift(KeyCode::Down));
    app.handle_key(key(KeyCode::Delete));
    app.handle_key(key(KeyCode::Up));
    assert_eq!(md_files(&d), ["racfer~z.md", "root.md"]);
    assert_eq!(titles(&app), ["Z", "c", "b1"]);
    draw(&mut app, 100, 24);
    // the same deleted from c's line, and saved by Esc
    let (d, mut app) = zoomed_on_z_over_b();
    press(&mut app, "je");
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::End));
    app.handle_key(shift(KeyCode::Up));
    app.handle_key(shift(KeyCode::End));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(md_files(&d).len(), 3);
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(md_files(&d), ["racfer~z.md", "root.md"]);
    assert_eq!(titles(&app), ["Z", "c", "b1"]);
    draw(&mut app, 100, 24);
    // the zoomed node's own title edited: the zoom follows it
    let (d, mut app) = app_with("# A\n\n# B\n\n- b1\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    press(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    press(&mut app, "e");
    app.handle_key(key(KeyCode::End));
    press(&mut app, "ee");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), "# A\n\n# Bee\n\n- b1\n");
    assert_eq!(titles(&app), ["Bee", "b1"]);
    // a child's title edited: the zoom stays on the zoomed node
    let (d, mut app) = app_with("# A\n\n- a1\n\n# B\n\n- b1\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    press(&mut app, "jj");
    app.handle_key(key(KeyCode::Enter)); // zoom into B
    assert_eq!(titles(&app), ["B", "b1"]);
    press(&mut app, "e");
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::End));
    press(&mut app, "x");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), "# A\n\n- a1\n\n# B\n\n- b1x\n");
    assert_eq!(titles(&app), ["B", "b1x"]);
    // Revert re-reads the files, and one changed on disk meanwhile
    let (d, mut app) = app_with("- a\n- Z\n  - z1\n");
    app.set_edit_keys(fold_tui::app::EditKeys::Normal);
    press(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    press(&mut app, "ex");
    std::fs::write(d.path().join("root.md"), "- a\n- a2\n- Z\n  - z1\n").unwrap();
    app.run_action(fold_tui::app::Action::EditRevert);
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(titles(&app), ["Z", "z1"]);
}
