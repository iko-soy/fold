//! What the app does over time, as `fold` runs it: the watcher, the
//! autosave and the status line (§10.6, §11.2), where the selection goes
//! when rows come and go (§8.5), keys on a selection the wheel left out
//! of view (§10.1), editor text no save can take when fold ends or
//! reverts (§10.6), sync conflicts that come in while you work (§10.7),
//! the conflict copies they leave in the outline (§10.1, §12.5), what
//! the status line says a verb did (§10.1), and the next step it shows
//! once that is old, or a key is half typed or does nothing (§10.1).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use fold_tui::app::{node_menu_index, Action, App, EditKeys, Hit};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::time::{Duration, Instant};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// A vault in a directory named as a real one would be: tempfile's own
/// names start with a dot, which the watcher skips.
fn vault(root: &str) -> tempfile::TempDir {
    let d = tempfile::Builder::new().prefix("vault").tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), root).unwrap();
    d
}

/// The app as `fold` starts it: the watcher, then the startup scan for
/// sync-conflict copies, which reads the vault directory.
fn start(d: &tempfile::TempDir) -> App {
    let mut app = App::new(d.path()).unwrap();
    app.start_watcher();
    app.merge_on_startup();
    app
}

/// The main loop for `ms` milliseconds with no key pressed; the number of
/// reloads it ran.
fn idle(app: &mut App, ms: u64) -> usize {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut reloads = 0;
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
        reloads += app.tick() as usize;
    }
    reloads
}

fn root(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join("root.md")).unwrap()
}

fn status_line(app: &mut App) -> String {
    let (w, h) = (160, 20);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| b[(x, y)].symbol()).collect::<String>()).find(|l| l.contains("saved")).unwrap_or_default()
}

/// Watch the vault as strace would: every file the app opens there.
fn spy(d: &tempfile::TempDir) -> (impl Sized, impl Fn() -> usize) {
    use notify::event::{AccessKind, EventKind};
    use notify::Watcher;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut w = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .unwrap();
    w.watch(d.path(), notify::RecursiveMode::Recursive).unwrap();
    let opens = move || {
        rx.try_iter()
            .filter(|e: &notify::Result<notify::Event>| {
                e.as_ref().is_ok_and(|e| matches!(e.kind, EventKind::Access(AccessKind::Open(_))))
            })
            .count()
    };
    (w, opens)
}

#[test]
fn an_idle_app_does_not_reload_on_its_own_reads_or_writes() {
    // a reload reads every file; reading one is no change to take in
    let d = vault("# Inbox\n\n- [ ] a\n");
    let mut app = start(&d);
    let (_w, opens) = spy(&d);
    assert_eq!(idle(&mut app, 2500), 0, "reloads while idle");
    assert_eq!(opens(), 0, "files read while idle");
    // nor is what fold wrote itself, seen once its self-write window is over
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(root(&d), "# Inbox\n\n- [x] a\n");
    assert_eq!(idle(&mut app, 1500), 0, "reloads after checking a task");
}

#[test]
fn typing_in_the_editor_is_one_save_not_split_by_reloads() {
    let text = "# Inbox\n\nnotes\n";
    let d = vault(text);
    let mut app = start(&d);
    app.set_edit_keys(EditKeys::Normal);
    app.handle_key(key(KeyCode::Char('e')));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::End));
    // ten words over some 2.5 s, never pausing long enough to autosave
    let typed = " on the quick brown fox and the lazy dog";
    for c in typed.chars() {
        app.handle_key(key(KeyCode::Char(c)));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!app.tick(), "reloaded while typing");
    }
    assert_eq!(root(&d), text, "saved while typing");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), format!("# Inbox\n\nnotes{}\n", typed));
    // one undo takes back all of it
    app.handle_key(key(KeyCode::Char('u')));
    assert_eq!(root(&d), text);
}

#[test]
fn a_nested_block_cut_in_the_editor_survives_until_it_is_pasted() {
    // §5.2: cutting a nested block's title line and pasting it elsewhere
    // moves the block, however long the pause between the two
    let d = vault("# A\n\n- one\n- task\n- two\n");
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = fold_core::ops::make_block(&mut v, t).unwrap();
    let block = v.dir.join(&v.tree.files[v.tree.block_by_id(&id).unwrap().0].path);
    drop(v);
    let embed = format!("![[{}]]", id.as_str());
    assert_eq!(root(&d), format!("# A\n\n- one\n{}\n- two\n", embed));
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = start(&d);
    app.set_edit_keys(EditKeys::Normal);
    app.handle_key(key(KeyCode::Char('e')));
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Down));
    }
    app.handle_key(ctrl('k'));
    // the pause autosaves A without the line; the block waits in transit
    let reloads = idle(&mut app, 2000);
    assert!(block.exists(), "the cut block was deleted before it was pasted");
    assert_eq!(root(&d), "# A\n\n- one\n- two\n");
    assert_eq!(reloads, 0, "reloads during the pause");
    // paste it above "- one"
    app.handle_key(key(KeyCode::Up));
    app.handle_key(ctrl('v'));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), format!("# A\n\n{}\n- one\n- two\n", embed));
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
}

#[test]
fn help_and_the_view_toggles_from_the_editor_keep_a_cut_block_to_paste() {
    // §5.2: help, the palette and the view toggles write nothing, so a
    // block cut in the editor is still moved once pasted after them
    let d = vault("# A\n\n- one\n- task\n- two\n");
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = fold_core::ops::make_block(&mut v, t).unwrap();
    let block = v.dir.join(&v.tree.files[v.tree.block_by_id(&id).unwrap().0].path);
    drop(v);
    let embed = format!("![[{}]]", id.as_str());
    let before = std::fs::read_to_string(&block).unwrap();
    let mut app = start(&d);
    app.set_edit_keys(EditKeys::Normal);
    app.handle_key(key(KeyCode::Char('e')));
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Down));
    }
    app.handle_key(ctrl('k'));
    idle(&mut app, 1000);
    assert_eq!(root(&d), "# A\n\n- one\n- two\n");
    // F1, then Esc back to the editor
    app.handle_key(key(KeyCode::F(1)));
    assert_eq!(app.mode_pub(), "help");
    app.handle_key(key(KeyCode::Esc));
    assert!(block.exists(), "F1 deleted the cut block");
    // the top bar's ?, closed with its ✕
    click_button(&mut app, Action::Help);
    assert_eq!(app.mode_pub(), "help");
    click_button(&mut app, Action::Close);
    assert!(block.exists(), "the ? button deleted the cut block");
    // wrap lines from ☰ Commands
    click_button(&mut app, Action::Palette);
    press(&mut app, "wrap");
    app.handle_key(key(KeyCode::Enter));
    assert!(block.exists(), "a view toggle deleted the cut block");
    assert_eq!(app.mode_pub(), "edit");
    // paste it above "- one"
    app.handle_key(key(KeyCode::Up));
    app.handle_key(ctrl('v'));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), format!("# A\n\n{}\n- one\n- two\n", embed));
    assert_eq!(std::fs::read_to_string(&block).unwrap(), before);
}

#[test]
fn a_change_from_outside_is_taken_in_and_announced() {
    let d = vault("# NAS\n\n- disks\n\n# Inbox\n\n- [ ] a\n");
    let mut app = start(&d);
    idle(&mut app, 300);
    // another program appends a task, as `echo >> root.md` would
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(d.path().join("root.md")).unwrap();
        f.write_all(b"- [ ] phone-added task\n").unwrap();
    }
    let end = Instant::now() + Duration::from_secs(5);
    let mut reloads = 0;
    while reloads == 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
        reloads += app.tick() as usize;
    }
    assert_eq!(reloads, 1, "the change was not taken in");
    let titles: Vec<String> = app.rows().iter().map(|r| app.title_of(r.nref)).collect();
    assert!(titles.contains(&"phone-added task".to_string()), "{:?}", titles);
    let s = status_line(&mut app);
    assert!(s.contains("↻ changed outside fold: Inbox (+1 item)"), "{}", s);
    assert!(!s.contains("NAS"), "{}", s);
    // and taken in once
    assert_eq!(idle(&mut app, 1000), 0);
}

#[test]
fn a_change_from_outside_while_editing_saves_the_editor_first_and_says_both() {
    let d = vault("# NAS\n\n- disks\n\n# Inbox\n\n- [ ] a\n");
    let mut app = start(&d);
    app.set_edit_keys(EditKeys::Normal);
    app.handle_key(key(KeyCode::Char('e')));
    app.handle_key(key(KeyCode::End));
    app.handle_key(key(KeyCode::Char('!')));
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(d.path().join("root.md")).unwrap();
        f.write_all(b"- [ ] phone-added task\n").unwrap();
    }
    // the reload comes before the autosave would
    let end = Instant::now() + Duration::from_millis(700);
    let mut reloads = 0;
    while reloads == 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
        reloads += app.tick() as usize;
    }
    assert_eq!(reloads, 1, "the change was not taken in");
    assert_eq!(root(&d), "# NAS!\n\n- disks\n\n# Inbox\n\n- [ ] a\n- [ ] phone-added task\n");
    // the typing is not what came from outside
    let s = status_line(&mut app);
    assert!(s.contains("↻ changed outside fold: Inbox (+1 item) · your typing was saved first"), "{}", s);
}

/// The main loop until it has run a reload, for up to 5 s.
fn until_reload(app: &mut App) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
        if app.tick() {
            return;
        }
    }
    panic!("the change was not taken in");
}

#[test]
fn a_change_from_outside_beside_a_copy_the_merge_left_alone_is_announced() {
    // notes.md has no id: the copy of an ignored file, which the merge
    // leaves alone (§12.2), is in the vault for good
    let d = vault("# NAS\n\n- disks\n\n# Inbox\n\n- [ ] a\n");
    std::fs::write(d.path().join("notes.md"), "mine\n").unwrap();
    std::fs::write(d.path().join("notes.sync-conflict-20260926-150000-PHONE.md"), "theirs\n").unwrap();
    let mut app = start(&d);
    // nothing came in: the greeting stays
    assert!(status_line(&mut app).contains("? help"), "{}", status_line(&mut app));
    idle(&mut app, 300);
    for task in ["from phone", "from laptop"] {
        let text = root(&d);
        std::fs::write(d.path().join("root.md"), format!("{}- [ ] {}\n", text, task)).unwrap();
        until_reload(&mut app);
        assert!(app.rows().iter().any(|r| app.title_of(r.nref) == task));
        let s = status_line(&mut app);
        assert!(s.contains("↻ changed outside fold: Inbox (+1 item)"), "{}", s);
        assert!(!s.contains("notes") && !s.contains("merged"), "{}", s);
    }
}

#[test]
fn a_copy_merged_without_a_pair_is_announced_as_what_came_in() {
    let d = vault("# NAS\n\n- disks\n\n# Inbox\n\n- [ ] a\n");
    let mut app = start(&d);
    idle(&mut app, 300);
    std::fs::write(
        d.path().join("root.sync-conflict-20260927-100000-PHONE.md"),
        "# NAS\n\n- disks\n\n# Inbox\n\n- [ ] a\n- [ ] from phone\n",
    )
    .unwrap();
    until_reload(&mut app);
    assert!(app.vault_conflict_files().unwrap().is_empty(), "the copy was not merged");
    assert!(root(&d).contains("- [ ] from phone\n"), "{}", root(&d));
    assert_eq!(pairs(&mut app), 0);
    assert_eq!(app.mode_pub(), "normal");
    let s = status_line(&mut app);
    assert!(s.contains("↻ changed outside fold: Inbox (+1 item)"), "{}", s);
    assert!(!s.contains("sync-conflict") && !s.contains("pair"), "{}", s);
}

#[test]
fn a_copy_merged_at_startup_without_a_pair_is_announced_as_what_came_in() {
    let d = vault("# NAS\n\n- disks\n\n# Inbox\n\n- [ ] a\n");
    std::fs::write(
        d.path().join("root.sync-conflict-20260927-100000-PHONE.md"),
        "# NAS\n\n- disks\n\n# Inbox\n\n- [ ] a\n- [ ] from phone\n",
    )
    .unwrap();
    let mut app = start(&d);
    assert!(app.vault_conflict_files().unwrap().is_empty(), "the copy was not merged");
    assert!(app.rows().iter().any(|r| app.title_of(r.nref) == "from phone"));
    let s = status_line(&mut app);
    assert!(s.contains("↻ changed outside fold: Inbox (+1 item)"), "{}", s);
    assert!(!s.contains("sync-conflict") && !s.contains("pair"), "{}", s);
    assert_eq!(idle(&mut app, 1000), 0);
}

// ------------------------------------------------------------ hiding done

/// Two lists with done tasks above the cursor's rows, as in the sample
/// vault.
const TASKS: &str = "# NAS\n\n- [x] Replace fan\n- [ ] Snapshot policy\n\n# Networking\n\n- [ ] Label the cables\n  - [x] patch panel\n  - [ ] rack\n\n# Atlas\n\nQuarterly planning notes.\nDeadline is end of month.\n\n- [ ] Draft the RFC\n- [ ] Review PR\n- [x] Kickoff meeting\n\n# Reading list\n\n- The Rust Book\n";

fn press(app: &mut App, keys: &str) {
    for c in keys.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

/// The selected node's title.
fn selected(app: &App) -> String {
    app.current().map(|r| app.title_of(r)).unwrap_or_default()
}

/// Put the cursor on the row titled `title`.
fn select(app: &mut App, title: &str) {
    app.cursor = app.rows().iter().position(|r| app.title_of(r.nref) == title).unwrap();
}

fn draw(app: &mut App) {
    let mut t = Terminal::new(TestBackend::new(120, 32)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
}

/// Draw, then click the button the frame drew for `a`.
fn click_button(app: &mut App, a: Action) {
    draw(app);
    let (x, y) = app.button_pos(a).unwrap_or_else(|| panic!("no button {:?}", a));
    app.handle_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE });
}

#[test]
fn hiding_and_showing_done_keeps_the_selected_node() {
    let d = vault(TASKS);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "Draft the RFC");
    // two done tasks above it go: the row number changes, the node does not
    press(&mut app, "zd");
    assert_eq!(selected(&app), "Draft the RFC");
    press(&mut app, "zd");
    assert_eq!(selected(&app), "Draft the RFC");
    // the next verb acts on it
    press(&mut app, "x");
    assert!(root(&d).contains("- [x] Draft the RFC"), "{}", root(&d));
}

#[test]
fn hiding_the_selected_done_task_selects_its_next_shown_sibling() {
    let d = vault(TASKS);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "patch panel");
    press(&mut app, "zd");
    assert_eq!(selected(&app), "rack");
    press(&mut app, "zd");
    select(&mut app, "Replace fan");
    press(&mut app, "zd");
    assert_eq!(selected(&app), "Snapshot policy");
    // the last one of its list: the row above
    press(&mut app, "zd");
    select(&mut app, "Kickoff meeting");
    press(&mut app, "zd");
    assert_eq!(selected(&app), "Review PR");
}

#[test]
fn hiding_done_skips_done_siblings_and_leaves_a_done_task_s_children() {
    let d = vault("# A\n\n- [x] Order parts\n  - [ ] check fan model\n- [x] Call shop\n- [ ] Fit it\n- [ ] Test it\n");
    let mut app = App::new(d.path()).unwrap();
    // under a done task, it goes with it: the task's next shown sibling
    select(&mut app, "check fan model");
    press(&mut app, "zd");
    assert_eq!(selected(&app), "Fit it");
    press(&mut app, "zd");
    select(&mut app, "Order parts");
    press(&mut app, "zd");
    assert_eq!(selected(&app), "Fit it");
}

#[test]
fn hiding_done_from_the_palette_or_the_status_bar_keeps_the_selected_node() {
    let d = vault(TASKS);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "Draft the RFC");
    press(&mut app, ":hide");
    app.handle_key(key(KeyCode::Enter));
    assert!(app.action_on(Action::HideDone).unwrap());
    assert_eq!(selected(&app), "Draft the RFC");
    // *done hidden* in the status bar shows them again
    click_button(&mut app, Action::HideDone);
    assert!(!app.action_on(Action::HideDone).unwrap());
    assert_eq!(selected(&app), "Draft the RFC");
}

#[test]
fn hiding_done_keeps_the_reading_pane_on_its_node() {
    let d = vault(TASKS);
    let mut app = App::new(d.path()).unwrap();
    app.show_reading = true;
    select(&mut app, "Atlas");
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "jj");
    draw(&mut app);
    let at = app.read_cursor_pub();
    assert!(at > 0);
    press(&mut app, "zd");
    draw(&mut app);
    assert_eq!(selected(&app), "Atlas");
    assert_eq!(app.read_cursor_pub(), at, "the reading cursor went back to the top");
}

// ------------------------------------------------------------ out of view

/// A list longer than the screen, as in the report: `# Tasks` and 80
/// tasks under it.
fn long_list() -> String {
    let tasks: String = (1..=80).map(|i| format!("- [ ] task number {}\n", i)).collect();
    format!("# Tasks\n\n{}", tasks)
}

fn has_line(d: &tempfile::TempDir, line: &str) -> bool {
    root(d).lines().any(|l| l == line)
}

/// Draw a frame, as the app does after every event: the screen's text.
fn screen(app: &mut App) -> String {
    let (w, h) = (120, 32);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| b[(x, y)].symbol()).collect::<String>() + "\n").collect()
}

/// The status bar, as the next frame draws it.
fn status(app: &mut App) -> String {
    screen(app).lines().last().unwrap_or_default().to_string()
}

/// Turn the wheel over what the last frame drew for `hit`, `notches`
/// times down (up if negative), a frame after each.
fn wheel(app: &mut App, hit: Hit, notches: i32) {
    let kind = if notches > 0 { MouseEventKind::ScrollDown } else { MouseEventKind::ScrollUp };
    for _ in 0..notches.abs() {
        screen(app);
        let (x, y) = app.hit_pos(hit).unwrap_or_else(|| panic!("nothing drawn for {:?}", hit));
        app.handle_mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
    }
    screen(app);
}

/// Whether the last frame drew outline row `i`.
fn shown(app: &App, i: usize) -> bool {
    app.hit_pos(Hit::Row(i)).is_some()
}

#[test]
fn a_key_that_changes_a_selection_out_of_view_shows_it_and_waits_for_a_second_press() {
    let d = vault(&long_list());
    let mut app = App::new(d.path()).unwrap();
    press(&mut app, "jj");
    assert_eq!(selected(&app), "task number 2");
    wheel(&mut app, Hit::OutlinePane, 10);
    assert!(!shown(&app, 2));
    // the first x shows it and says what a second would do
    press(&mut app, "x");
    assert!(has_line(&d, "- [ ] task number 2"), "checked out of view");
    let s = status(&mut app);
    assert!(shown(&app, 2), "not brought into view");
    assert!(s.contains("press x again to mark “task number 2” done"), "{}", s);
    press(&mut app, "x");
    assert!(has_line(&d, "- [x] task number 2"));
    // d likewise: nothing trashed until it is in view
    wheel(&mut app, Hit::OutlinePane, 10);
    press(&mut app, "d");
    assert!(has_line(&d, "- [x] task number 2"), "deleted out of view");
    let s = status(&mut app);
    assert!(s.contains("press d again to delete “task number 2”"), "{}", s);
    press(&mut app, "d");
    assert!(!root(&d).contains("task number 2\n"), "{}", root(&d));
}

#[test]
fn a_selection_brought_into_view_sits_a_third_of_the_way_down_until_the_wheel_moves_it_again() {
    let d = vault(&long_list());
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "task number 40");
    wheel(&mut app, Hit::OutlinePane, -10);
    assert!(!shown(&app, 40));
    press(&mut app, "t");
    assert!(has_line(&d, "- [ ] task number 40"));
    let s = screen(&mut app);
    let top = s.lines().position(|l| l.contains("Outline")).unwrap();
    let at = s.lines().position(|l| l.contains("task number 40 ")).expect("not brought into view");
    let (view, down) = (32 - 4, at - top - 1);
    assert!(down >= view / 4 && down <= view * 2 / 5, "{} rows down of {}", down, view);
    // the wheel takes it out of view again: the next t is a first press
    wheel(&mut app, Hit::OutlinePane, -10);
    press(&mut app, "t");
    assert!(has_line(&d, "- [ ] task number 40"), "changed out of view");
    press(&mut app, "t");
    assert!(has_line(&d, "- task number 40"), "{}", root(&d));
    // za, a two-key verb, too
    wheel(&mut app, Hit::OutlinePane, -10);
    press(&mut app, "za");
    assert!(has_line(&d, "- task number 40"), "archived out of view");
    let s = status(&mut app);
    assert!(s.contains("press za again to archive “task number 40”"), "{}", s);
    press(&mut app, "za");
    assert!(root(&d).contains("# Archive"), "{}", root(&d));
}

#[test]
fn a_key_that_changes_nothing_shows_a_selection_out_of_view_and_acts_at_once() {
    let d = vault(&long_list());
    let mut app = App::new(d.path()).unwrap();
    press(&mut app, "jj");
    wheel(&mut app, Hit::OutlinePane, 10);
    press(&mut app, "m");
    screen(&mut app);
    assert!(app.hit_pos(Hit::MenuItem(0)).is_some(), "no node menu");
    assert!(shown(&app, 2));
    app.handle_key(key(KeyCode::Esc));
    wheel(&mut app, Hit::OutlinePane, 10);
    press(&mut app, "e");
    assert_eq!(app.mode_pub(), "edit");
    screen(&mut app);
    assert!(shown(&app, 2));
}

#[test]
fn the_reading_pane_shows_its_cursor_line_before_changing_it() {
    let prose: String = (1..=60).map(|i| format!("line {} of the notes\n", i)).collect();
    let d = vault(&format!("# Notes\n\n{}\n- [ ] buried task\n", prose));
    let mut app = App::new(d.path()).unwrap();
    app.show_reading = true;
    screen(&mut app);
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "G");
    let task = app.read_cursor_pub();
    assert!(app.reading_doc_pub().lines[task].contains("buried task"));
    wheel(&mut app, Hit::ReadingPane, -10);
    assert!(app.hit_pos(Hit::DocLine(task)).is_none());
    press(&mut app, "x");
    assert!(has_line(&d, "- [ ] buried task"), "checked out of view");
    let s = status(&mut app);
    assert!(app.hit_pos(Hit::DocLine(task)).is_some(), "not brought into view");
    assert!(s.contains("press x again to mark “buried task” done"), "{}", s);
    press(&mut app, "x");
    assert!(has_line(&d, "- [x] buried task"));
    // Enter on a task toggles it: the same
    wheel(&mut app, Hit::ReadingPane, -10);
    app.handle_key(key(KeyCode::Enter));
    assert!(has_line(&d, "- [x] buried task"), "reopened out of view");
    let s = status(&mut app);
    assert!(s.contains("press Enter again to reopen “buried task”"), "{}", s);
    app.handle_key(key(KeyCode::Enter));
    assert!(has_line(&d, "- [ ] buried task"));
    assert!(!status(&mut app).contains("again"), "the words outlived the verb");
}

#[test]
fn a_held_verb_s_words_go_once_it_acts_or_the_selection_moves() {
    let d = vault(&long_list());
    let mut app = App::new(d.path()).unwrap();
    press(&mut app, "jj");
    wheel(&mut app, Hit::OutlinePane, 10);
    // J says nothing when it moves a node: the words must not stay
    press(&mut app, "J");
    let s = status(&mut app);
    assert!(s.contains("press J again to move “task number 2” down"), "{}", s);
    press(&mut app, "J");
    assert!(root(&d).contains("- [ ] task number 3\n- [ ] task number 2\n"), "{}", root(&d));
    let s = status(&mut app);
    assert!(!s.contains("again"), "{}", s);
    // a click selects another node: the words named the one before
    wheel(&mut app, Hit::OutlinePane, 10);
    press(&mut app, "d");
    assert!(status(&mut app).contains("press d again"));
    let (x, y) = app.hit_pos(Hit::Row(10)).unwrap();
    for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
        app.handle_mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
    }
    let s = status(&mut app);
    assert!(!s.contains("again"), "{}", s);
}

#[test]
fn the_pointer_acts_on_a_selection_out_of_view_at_once() {
    let d = vault(&long_list());
    let mut app = App::new(d.path()).unwrap();
    app.show_reading = true;
    press(&mut app, "jj");
    wheel(&mut app, Hit::OutlinePane, 10);
    assert!(!shown(&app, 2));
    // the reading pane's ⋯ is the selected node's menu
    click_button(&mut app, Action::NodeMenu);
    screen(&mut app);
    let (x, y) = app.hit_pos(Hit::MenuItem(node_menu_index(Action::Delete))).unwrap();
    app.handle_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE });
    assert!(!root(&d).contains("task number 2\n"), "{}", root(&d));
}

/// The status bar at 80×24, as the next frame draws it.
fn status80(app: &mut App) -> String {
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    (0..80).map(|x| b[(x, 23)].symbol()).collect()
}

/// Turn the wheel down over the outline at 80×24, `notches` times.
fn wheel80(app: &mut App, notches: usize) {
    for _ in 0..notches {
        status80(app);
        let (x, y) = app.hit_pos(Hit::OutlinePane).unwrap();
        app.handle_mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: x, row: y, modifiers: KeyModifiers::NONE });
    }
    status80(app);
}

#[test]
fn a_held_verb_s_words_keep_the_key_and_what_it_does_at_80_columns() {
    // a title too long for the bar gives way, not the words after it
    let title = "Replace the flaky switch in the closet before Friday";
    let d = vault(&long_list().replace("task number 2\n", &format!("{}\n", title)));
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, title);
    wheel80(&mut app, 10);
    press(&mut app, "x");
    let s = status80(&mut app);
    assert!(s.starts_with(" press x again to mark “Replace the flaky"), "{}", s);
    assert!(s.contains("…” done "), "{}", s);
    press(&mut app, "x");
    assert!(has_line(&d, &format!("- [x] {}", title)), "{}", root(&d));
    // with the ⚠ count and done hidden on the right, a short title still fits
    let text = format!("# Networking\n\n- [ ] Label the cables\n  - [x] patch panel\n  - [ ] rack\n\n{}", long_list());
    let d = vault(&text);
    std::fs::write(d.path().join("root.sync-conflict-20260927-100000-PHONE.md"), text.replace("[ ] task number 80", "[x] task number 80")).unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    fold_core::merge::merge_sync_conflicts(&mut v, false).unwrap();
    drop(v);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "rack");
    press(&mut app, "zd");
    wheel80(&mut app, 10);
    press(&mut app, "d");
    let s = status80(&mut app);
    assert!(s.contains(" ⚠ 1 conflict  done hidden "), "{}", s);
    assert!(s.starts_with(" press d again to delete “rack” "), "{}", s);
    press(&mut app, "d");
    assert!(!root(&d).contains("rack"), "{}", root(&d));
}

// ------------------------------------------------------------ unsaved text

/// The trash entries for editor text on `title`'s node that fold could
/// not save (§11.5), `<timestamp>-unsaved-<title>.md`, as the returned
/// closure finds them: those made after this call, since the trash
/// outlives a test run.
fn kept(title: &str) -> impl Fn() -> Vec<std::path::PathBuf> {
    let suffix = format!("-unsaved-{}.md", title);
    let entries = move || {
        let mut out: Vec<_> = std::fs::read_dir(fold_core::vault::trash_dir())
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        out.retain(|p: &std::path::PathBuf| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&suffix)));
        out
    };
    let before = entries();
    move || entries().into_iter().filter(|p| !before.contains(p)).collect()
}

/// The editor open on the vault's first node, `typed` typed at the end of
/// its title line.
fn typing(d: &tempfile::TempDir, typed: &str) -> App {
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(EditKeys::Normal);
    press(&mut app, "e");
    app.handle_key(key(KeyCode::End));
    press(&mut app, typed);
    app
}

#[test]
fn reverting_text_a_save_was_refused_for_keeps_a_copy_in_the_trash() {
    let d = vault("# Rotation schedule\n\nkeep 24\n");
    let kept = kept("rotation-schedule");
    let mut app = typing(&d, " hourly");
    // another program changes the node: the save is refused, and the
    // editor stays open with the text
    std::fs::write(d.path().join("root.md"), "# Rotation schedule\n\nkeep 48\n").unwrap();
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode_pub(), "edit");
    // :q! leaves it, and the text goes to the trash first
    app.handle_key(ctrl('e'));
    press(&mut app, "q!");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# Rotation schedule\n\nkeep 48\n");
    let copies = kept();
    assert_eq!(copies.len(), 1, "{:?}", copies);
    assert_eq!(std::fs::read_to_string(&copies[0]).unwrap(), "# Rotation schedule hourly\n\nkeep 24\n");
    // the status line says where
    let name = copies[0].file_name().unwrap().to_string_lossy().to_string();
    let s = status(&mut app);
    assert!(s.contains(&format!("changes discarded; a copy is in the trash: {}", name)), "{}", s);
}

#[test]
fn reverting_text_no_save_refused_keeps_no_copy() {
    let d = vault("# Retention window\n\nkeep 24\n");
    let kept = kept("retention-window");
    let mut app = typing(&d, " days");
    app.run_action(Action::EditRevert);
    assert_eq!(app.mode_pub(), "normal");
    assert_eq!(root(&d), "# Retention window\n\nkeep 24\n");
    assert!(kept().is_empty());
    let s = status(&mut app);
    assert!(s.contains("changes discarded") && !s.contains("trash"), "{}", s);
}

#[test]
fn ending_fold_while_editing_saves_the_editor() {
    // a signal, an error or a crash: the editor is saved as on a quit
    let d = vault("# Offsite copy\n\nnightly\n");
    let kept = kept("offsite-copy");
    let mut app = typing(&d, " to B2");
    assert_eq!(app.keep_unsaved(), None);
    assert_eq!(root(&d), "# Offsite copy to B2\n\nnightly\n");
    assert!(kept().is_empty());
    // nothing unsaved, nothing to keep
    assert_eq!(app.keep_unsaved(), None);
}

#[test]
fn ending_fold_while_editing_deletes_a_cut_block_the_autosave_left_in_transit() {
    // §5.2: a block cut and not pasted back is deleted on any end of the
    // app, even once the autosave has left nothing unsaved
    let d = vault("# A\n\n- one\n- task\n- two\n");
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = fold_core::ops::make_block(&mut v, t).unwrap();
    let block = v.dir.join(&v.tree.files[v.tree.block_by_id(&id).unwrap().0].path);
    drop(v);
    let kept = kept("a");
    let mut app = start(&d);
    app.set_edit_keys(EditKeys::Normal);
    app.handle_key(key(KeyCode::Char('e')));
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Down));
    }
    app.handle_key(ctrl('k'));
    idle(&mut app, 1000);
    assert_eq!(root(&d), "# A\n\n- one\n- two\n");
    assert!(block.exists());
    assert_eq!(app.keep_unsaved(), None);
    assert!(!block.exists(), "the cut block outlived the app");
    assert_eq!(root(&d), "# A\n\n- one\n- two\n");
    assert!(kept().is_empty());
}

#[test]
fn ending_fold_with_text_a_save_was_refused_for_keeps_it_in_the_trash() {
    let d = vault("# Scrub cadence\n\n- monthly\n- [ ] scrub now\n");
    let kept = kept("scrub-cadence");
    let mut app = typing(&d, " weekly");
    std::fs::write(d.path().join("root.md"), "# Scrub cadence\n\n- monthly, from the phone\n- [ ] scrub now\n").unwrap();
    let words = app.keep_unsaved().expect("nothing kept");
    let copies = kept();
    assert_eq!(copies.len(), 1, "{:?}", copies);
    assert_eq!(words, format!("unsaved text kept in {}", copies[0].display()));
    // the editor's whole text, as it was
    assert_eq!(std::fs::read_to_string(&copies[0]).unwrap(), "# Scrub cadence weekly\n\n- monthly\n- [ ] scrub now\n");
    // and the other program's change stays
    assert_eq!(root(&d), "# Scrub cadence\n\n- monthly, from the phone\n- [ ] scrub now\n");
}

/// Run the test named `test` again in a process of its own, with a trash
/// nothing can be written to: a plain file where its directory would be
/// (§11.5). The trash is the process's, where `$XDG_STATE_HOME` says, so
/// the other tests keep theirs. True in that process, where the test goes
/// on; false in this one, once it has passed there.
fn with_trash_unwritable(test: &str) -> bool {
    if std::env::var_os("FOLD_TEST_TRASH_UNWRITABLE").is_some() {
        return true;
    }
    let state = tempfile::tempdir().unwrap();
    std::fs::create_dir(state.path().join("fold")).unwrap();
    std::fs::write(state.path().join("fold").join("trash"), "").unwrap();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture"])
        .env("XDG_STATE_HOME", state.path())
        .env("FOLD_TEST_TRASH_UNWRITABLE", "1")
        .output()
        .unwrap();
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && said.contains("1 passed"), "{}", said);
    false
}

#[test]
fn reverting_text_the_trash_cannot_take_drops_it_when_asked_again() {
    if !with_trash_unwritable("reverting_text_the_trash_cannot_take_drops_it_when_asked_again") {
        return;
    }
    let d = vault("# Rotation schedule\n\nkeep 24\n");
    let mut app = typing(&d, " hourly");
    std::fs::write(d.path().join("root.md"), "# Rotation schedule\n\nkeep 48\n").unwrap();
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode_pub(), "edit");
    // :q! finds no trash to copy the text to: it drops nothing, and says
    // what does
    let revert = |app: &mut App| {
        app.handle_key(ctrl('e'));
        press(app, "q!");
        app.handle_key(key(KeyCode::Enter));
    };
    revert(&mut app);
    assert_eq!(app.mode_pub(), "edit");
    let s = status_line(&mut app);
    assert!(s.contains("can't copy to the trash; Revert (:q!) again drops the changes — File exists (os error 17)"), "{}", s);
    // text typed since is new to it: that is asked about again
    press(&mut app, " and");
    app.run_action(Action::EditRevert);
    assert_eq!(app.mode_pub(), "edit");
    assert!(status_line(&mut app).contains("can't copy to the trash"));
    // asked again, it drops them
    revert(&mut app);
    assert_eq!(app.mode_pub(), "normal");
    assert!(status_line(&mut app).contains("changes discarded; no copy kept"));
    assert_eq!(root(&d), "# Rotation schedule\n\nkeep 48\n");
    // and a quit ends fold
    app.run_action(Action::Quit);
    assert!(app.quit_requested() && app.mode_pub() == "normal");
}

// ------------------------------------------------------------ sync conflicts

/// Conflict block ids that list first and last: the view lists pairs in
/// the order of their blocks' files (§12.5), and a merge names its blocks
/// by fresh ids.
const FIRST: &str = "bacbec-bacbec-bacbec-bacbec";
const LAST: &str = "worzod-worzod-worzod-worzod";

/// A small homelab vault. With `old`, a pair left from an earlier sync
/// conflict (§12.5) on the flaky switch, its conflict block under that id.
fn homelab(old: Option<&str>) -> tempfile::TempDir {
    let embed = old.map(|id| format!("![[{}]]\n", id)).unwrap_or_default();
    let d = vault(&format!(
        "# Homelab\n\n## NAS\n\nMirrored pairs.\n\n## Networking\n\n- [ ] Replace the flaky switch\n{}- [ ] Label the cables\n",
        embed
    ));
    if let Some(id) = old {
        let prefix = id.split('-').next().unwrap();
        std::fs::write(
            d.path().join(format!("{}~replace-the-flaky-switch.md", prefix)),
            format!("---\nid: {}\nconflict: \"PHONE 20260926-090000\"\n---\n\n- [x] Replace the flaky switch\n", id),
        )
        .unwrap();
    }
    d
}

/// Syncthing's copy of root.md from a phone that changed NAS's text and
/// checked the cables: two pairs once merged.
fn phone_copy(d: &tempfile::TempDir) {
    std::fs::write(
        d.path().join("root.sync-conflict-20260927-100000-PHONE.md"),
        "# Homelab\n\n## NAS\n\nMirrored pairs, no raidz.\n\n## Networking\n\n- [ ] Replace the flaky switch\n- [x] Label the cables\n",
    )
    .unwrap();
}

fn pairs(app: &mut App) -> usize {
    fold_core::merge::conflict_pairs(app.vault_mut()).len()
}

/// Whether the status bar's ⚠ count is lit, as it is while pairs that came
/// in as you worked wait to be seen.
fn lit(app: &mut App) -> bool {
    let (w, h) = (120, 32);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer();
    // the count is on the right; the message on the left may name it too
    let badge = (0..w).rev().map(|x| &b[(x, h - 1)]).find(|c| c.symbol() == "⚠").expect("no ⚠ in the status bar");
    badge.bg != b[(0, h - 1)].bg
}

#[test]
fn a_sync_conflict_while_typing_leaves_the_editor_open_and_the_keys_in_it() {
    let d = homelab(None);
    let mut app = start(&d);
    app.set_edit_keys(EditKeys::Normal);
    select(&mut app, "NAS");
    press(&mut app, "e");
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::End));
    // a key every 120 ms, the main loop running between them; the phone's
    // copy lands mid-word
    let type_slowly = |app: &mut App, text: &str| {
        for c in text.chars() {
            app.handle_key(key(KeyCode::Char(c)));
            std::thread::sleep(Duration::from_millis(120));
            app.tick();
        }
    };
    type_slowly(&mut app, " bot");
    phone_copy(&d);
    type_slowly(&mut app, "tom note, both boot targets");
    assert!(app.vault_conflict_files().unwrap().is_empty(), "the copy was not taken in");
    // o, t, b and e went on typing: no pair was picked unseen
    assert_eq!(app.mode_pub(), "edit");
    assert_eq!(pairs(&mut app), 2);
    let s = status_line(&mut app);
    assert!(s.contains("sync conflicts in “NAS” and 1 more: click ⚠ 2 conflicts to resolve"), "{}", s);
    assert!(lit(&mut app));
    app.handle_key(key(KeyCode::Esc));
    assert!(has_line(&d, "Mirrored pairs. bottom note, both boot targets"), "{}", root(&d));
    // the ⚠ opens the view on the first of them, and it takes o at once
    assert!(lit(&mut app));
    click_button(&mut app, Action::ResolveConflicts);
    assert_eq!(app.mode_pub(), "conflict");
    assert!(!lit(&mut app));
    assert!(screen(&mut app).contains("Mirrored pairs, no raidz."));
    press(&mut app, "o");
    assert_eq!(pairs(&mut app), 1);
}

#[test]
fn pairs_that_came_in_while_busy_open_once_the_outline_is_idle() {
    let d = homelab(Some(FIRST));
    let mut app = App::new(d.path()).unwrap();
    assert_eq!(pairs(&mut app), 1);
    press(&mut app, "j");
    phone_copy(&d);
    app.reload_external();
    // a key a moment ago: the outline stays
    assert_eq!(app.mode_pub(), "normal");
    let s = status(&mut app);
    assert!(s.contains("sync conflicts in “NAS” and 1 more: click ⚠ 3 conflicts to resolve"), "{}", s);
    assert!(lit(&mut app));
    assert_eq!(idle(&mut app, 1500), 0);
    assert_eq!(app.mode_pub(), "normal");
    // two seconds without a key: the view opens on NAS, not on the pair
    // left from before, which is listed first
    idle(&mut app, 1000);
    assert_eq!(app.mode_pub(), "conflict");
    let s = screen(&mut app);
    assert!(s.contains("Mirrored pairs, no raidz.") && !s.contains("flaky"), "{}", s);
    assert!(!lit(&mut app));
    // closed, it stays closed
    app.handle_key(key(KeyCode::Esc));
    std::thread::sleep(Duration::from_millis(2100));
    app.tick();
    assert_eq!(app.mode_pub(), "normal");
}

#[test]
fn a_view_that_opens_on_its_own_takes_no_side_for_half_a_second() {
    let d = homelab(None);
    let mut app = App::new(d.path()).unwrap();
    phone_copy(&d);
    // idle in the outline: the view opens at once
    app.reload_external();
    assert_eq!(app.mode_pub(), "conflict");
    let s = status(&mut app);
    assert!(s.contains("sync conflicts in “NAS” and 1 more"), "{}", s);
    // keys meant for the outline pick nothing, nor edit
    press(&mut app, "otbe");
    assert_eq!(app.mode_pub(), "conflict");
    assert_eq!(pairs(&mut app), 2);
    std::thread::sleep(Duration::from_millis(550));
    press(&mut app, "t");
    assert_eq!(pairs(&mut app), 1);
    assert!(has_line(&d, "Mirrored pairs, no raidz."), "{}", root(&d));
}

#[test]
fn resolving_a_pair_names_its_node_and_u_in_the_view_undoes_it() {
    let d = homelab(None);
    let mut app = App::new(d.path()).unwrap();
    phone_copy(&d);
    fold_core::merge::merge_sync_conflicts(app.vault_mut(), false).unwrap();
    app.enter_conflict_view();
    let shown = |app: &mut App| if screen(app).contains("Mirrored pairs, no raidz.") { "NAS" } else { "Label the cables" };
    let first = shown(&mut app);
    press(&mut app, "t");
    assert!(status(&mut app).contains(&format!("kept theirs for “{}” · u undoes", first)), "{}", status(&mut app));
    // u, in the view, puts the pair back and shows it
    press(&mut app, "u");
    assert_eq!(pairs(&mut app), 2);
    assert_eq!(app.mode_pub(), "conflict");
    assert_eq!(shown(&mut app), first);
    assert!(status(&mut app).contains(&format!("undone: keep theirs for “{}”", first)), "{}", status(&mut app));
    // the buttons say the same
    click_button(&mut app, Action::ConflictOurs);
    assert!(status(&mut app).contains(&format!("kept ours for “{}” · u undoes", first)), "{}", status(&mut app));
    let second = shown(&mut app);
    assert_ne!(second, first);
    press(&mut app, "b");
    assert!(status(&mut app).contains(&format!("kept both for “{}” · u undoes", second)), "{}", status(&mut app));
    assert_eq!(pairs(&mut app), 0);
}

#[test]
fn pairs_that_come_in_while_the_view_is_open_leave_it_on_the_pair_it_shows() {
    let d = homelab(Some(LAST));
    let mut app = App::new(d.path()).unwrap();
    app.enter_conflict_view();
    assert!(screen(&mut app).contains("flaky"));
    phone_copy(&d);
    app.reload_external();
    // listed after the new ones, the pair on screen stays on screen
    assert_eq!(app.mode_pub(), "conflict");
    assert_eq!(pairs(&mut app), 3);
    let s = screen(&mut app);
    assert!(s.contains("Conflict 3 of 3") && s.contains("flaky"), "{}", s);
    press(&mut app, "o");
    assert!(!root(&d).contains(LAST), "{}", root(&d));
    assert_eq!(pairs(&mut app), 2);
}

/// A task of its own in the homelab's Networking list, as a block.
fn milk_block(d: &tempfile::TempDir, file: &str, task: &str) {
    std::fs::write(d.path().join(file), format!("---\nid: {}\n---\n\n- {} Buy milk\n", LAST, task)).unwrap();
}

#[test]
fn a_block_that_comes_in_with_its_copy_is_merged_with_it() {
    let d = homelab(None);
    let mut app = start(&d);
    idle(&mut app, 300);
    // one sync brings a block from the laptop, its embed, and the phone's
    // copy of it
    milk_block(&d, "worzod~buy-milk.md", "[ ]");
    milk_block(&d, "worzod~buy-milk.sync-conflict-20260927-100000-PHONE.md", "[x]");
    std::fs::write(d.path().join("root.md"), format!("{}![[{}]]\n", root(&d), LAST)).unwrap();
    until_reload(&mut app);
    assert!(app.vault_conflict_files().unwrap().is_empty(), "the copy was left alone");
    assert_eq!(pairs(&mut app), 1);
    assert_eq!(app.mode_pub(), "conflict");
}

#[test]
fn copies_a_merge_failed_on_are_merged_with_the_next_change() {
    let d = homelab(None);
    std::fs::write(d.path().join("root.md"), format!("{}![[{}]]\n", root(&d), LAST)).unwrap();
    milk_block(&d, "worzod~buy-milk.md", "[ ]");
    let mut app = start(&d);
    idle(&mut app, 300);
    // root.md cannot be written for now, as when the trash cannot be: the
    // merge stops at its copy, before the one after it
    let tmp = d.path().join(".root.md.fold-tmp");
    std::fs::create_dir(&tmp).unwrap();
    phone_copy(&d);
    milk_block(&d, "worzod~buy-milk.sync-conflict-20260927-100000-PHONE.md", "[x]");
    until_reload(&mut app);
    assert!(status(&mut app).contains("merge error"), "{}", status(&mut app));
    assert_eq!(app.vault_conflict_files().unwrap().len(), 2);
    // it can be again: the next change from outside takes both in
    std::fs::remove_dir(&tmp).unwrap();
    std::fs::write(d.path().join("root.md"), format!("{}- [ ] from laptop\n", root(&d))).unwrap();
    until_reload(&mut app);
    assert!(app.vault_conflict_files().unwrap().is_empty(), "the copies were not merged");
    assert!(has_line(&d, "- [ ] from laptop"), "{}", root(&d));
    assert_eq!(pairs(&mut app), 3);
}

// ------------------------------------------------------------ conflict copies

/// A homelab vault merged with a phone's copy of it that changed NAS's
/// text and checked the cables: a copy of NAS, with the tasks under it,
/// and a copy of the cables task, each right after its own (§12.4).
fn lab() -> (tempfile::TempDir, App) {
    let text = "# Homelab\n\n## NAS\n\nMirrored pairs.\n\n- [ ] Snapshot policy\n- [x] Replace fan\n\n## Networking\n\n- [ ] Label the cables\n";
    let d = vault(text);
    std::fs::write(
        d.path().join("root.sync-conflict-20260927-100000-PHONE.md"),
        text.replace("pairs.", "pairs, no raidz.").replace("[ ] Label", "[x] Label"),
    )
    .unwrap();
    let mut v = fold_core::vault::Vault::open(d.path()).unwrap();
    fold_core::merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert_eq!(fold_core::merge::conflict_pairs(&v).len(), 2);
    drop(v);
    let app = App::new(d.path()).unwrap();
    (d, app)
}

/// Draw a frame: the screen as cells, to read colours and find things.
fn frame(app: &mut App) -> ratatui::buffer::Buffer {
    let mut t = Terminal::new(TestBackend::new(120, 32)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    t.backend().buffer().clone()
}

fn line(b: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()
}

fn text(b: &ratatui::buffer::Buffer) -> String {
    (0..b.area.height).map(|y| line(b, y) + "\n").collect()
}

/// Where `s` starts on screen, the topmost first.
fn find(b: &ratatui::buffer::Buffer, s: &str) -> Option<(u16, u16)> {
    (0..b.area.height).find_map(|y| {
        let l = line(b, y);
        l.find(s).map(|i| (l[..i].chars().count() as u16, y))
    })
}

/// Where the ⚠ after `s` is on screen.
fn warning_after(b: &ratatui::buffer::Buffer, s: &str) -> (u16, u16) {
    let at = format!("{} ⚠", s);
    let (x, y) = find(b, &at).unwrap_or_else(|| panic!("no “{}” on screen:\n{}", at, text(b)));
    (x + at.chars().count() as u16 - 1, y)
}

fn click_at(app: &mut App, x: u16, y: u16, button: MouseButton) {
    app.handle_mouse(MouseEvent { kind: MouseEventKind::Down(button), column: x, row: y, modifiers: KeyModifiers::NONE });
    app.handle_mouse(MouseEvent { kind: MouseEventKind::Up(button), column: x, row: y, modifiers: KeyModifiers::NONE });
}

/// Which pair the conflict view shows, by what is on screen.
fn pair_shown(app: &mut App) -> &'static str {
    assert_eq!(app.mode_pub(), "conflict");
    let s = screen(app);
    match (s.contains("no raidz"), s.contains("Label the cables")) {
        (true, false) => "NAS",
        (false, true) => "Label the cables",
        _ => panic!("{}", s),
    }
}

#[test]
fn a_conflict_copy_s_row_shows_a_warning_and_whose_copy_it_is() {
    let (_d, mut app) = lab();
    let b = frame(&mut app);
    // ⚠ in the warning colour where a block's ▤ goes, and where its text
    // would go, the device and time it came from
    let (x, y) = warning_after(&b, "NAS");
    assert_eq!(b[(x, y)].fg, ratatui::style::Color::Yellow);
    let row = line(&b, y);
    assert!(row.contains("NAS ⚠  1/2  other device · PHONE 09-27 10:00"), "{}", row);
    assert!(!row.contains("raidz"), "{}", row);
    warning_after(&b, "☑ Label the cables");
    assert!(!text(&b).contains('▤'), "{}", text(&b));
    // their tasks are not counted twice above them
    assert!(text(&b).contains("Homelab  2/3"), "{}", text(&b));
    assert!(text(&b).contains("Networking  1/1"), "{}", text(&b));
}

#[test]
fn a_conflict_copy_starts_folded_and_that_fold_is_not_remembered() {
    let (_d, mut app) = lab();
    let s = screen(&mut app);
    assert!(s.contains("▸ NAS ⚠"), "{}", s);
    assert_eq!(s.matches("Snapshot policy").count(), 1, "{}", s);
    // a click unfolds it as any row, for this run alone
    let b = frame(&mut app);
    let (x, y) = find(&b, "▸ NAS ⚠").unwrap();
    click_at(&mut app, x, y, MouseButton::Left);
    let s = screen(&mut app);
    assert_eq!(s.matches("Snapshot policy").count(), 2, "{}", s);
    assert!(app.view().folded.is_empty());
    press(&mut app, "j");
    let b = frame(&mut app);
    let (x, y) = find(&b, "▾ NAS ⚠").unwrap();
    click_at(&mut app, x, y, MouseButton::Left);
    assert_eq!(screen(&mut app).matches("Snapshot policy").count(), 1);
    assert!(app.view().folded.is_empty());
    // zoomed into, it shows what is in it
    app.cursor = app.rows().iter().rposition(|r| app.title_of(r.nref) == "NAS").unwrap();
    app.handle_key(key(KeyCode::Enter));
    let s = screen(&mut app);
    assert!(s.contains("▾ NAS ⚠") && s.contains("Snapshot policy"), "{}", s);
}

#[test]
fn a_click_on_a_copy_s_warning_opens_the_conflict_view_at_its_pair() {
    // the view lists pairs by their copies' files, so each is tried
    let (_d, mut app) = lab();
    for copy in ["NAS", "Label the cables"] {
        let b = frame(&mut app);
        let (x, y) = warning_after(&b, copy);
        click_at(&mut app, x, y, MouseButton::Left);
        assert_eq!(pair_shown(&mut app), copy);
        app.handle_key(key(KeyCode::Esc));
    }
}

#[test]
fn resolve_conflict_in_the_node_menu_of_either_side_opens_its_pair() {
    let (_d, mut app) = lab();
    // right-click ours, then a copy: the menu ends in Resolve conflict…
    for side in ["Label the cables", "NAS ⚠"] {
        let b = frame(&mut app);
        let (x, y) = find(&b, side).unwrap();
        click_at(&mut app, x, y, MouseButton::Right);
        let b = frame(&mut app);
        let (x, y) = find(&b, "Resolve conflict…").unwrap_or_else(|| panic!("{}", text(&b)));
        click_at(&mut app, x, y, MouseButton::Left);
        assert_eq!(pair_shown(&mut app), side.trim_end_matches(" ⚠"));
        app.handle_key(key(KeyCode::Esc));
    }
    // a node in no pair has none
    select(&mut app, "Networking");
    press(&mut app, "m");
    let s = screen(&mut app);
    assert!(s.contains("Move to…") && !s.contains("Resolve conflict"), "{}", s);
}

/// Draw a frame on a `w`×`h` terminal: the screen as cells.
fn frame_at(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    t.backend().buffer().clone()
}

#[test]
fn the_node_menu_of_a_pair_fits_a_short_terminal_resolve_conflict_and_all() {
    let (_d, mut app) = lab();
    // 80×24: the separators give way, every item shows and a click runs it
    select(&mut app, "NAS");
    press(&mut app, "m");
    let b = frame_at(&mut app, 80, 24);
    for item in ["│ Edit ", "│ Delete ", "│ Resolve conflict…"] {
        assert!(find(&b, item).is_some(), "no {}:\n{}", item, text(&b));
    }
    let (x, y) = find(&b, "Resolve conflict…").unwrap();
    click_at(&mut app, x, y, MouseButton::Left);
    assert_eq!(pair_shown(&mut app), "NAS");
    app.handle_key(key(KeyCode::Esc));
    // shorter still, the list scrolls with the selection, down to the last
    // item and back up to the first
    press(&mut app, "m");
    let b = frame_at(&mut app, 80, 14);
    assert!(find(&b, "│ Edit ").is_some() && find(&b, "Resolve conflict…").is_none(), "{}", text(&b));
    for _ in 0..30 {
        app.handle_key(key(KeyCode::Down));
        frame_at(&mut app, 80, 14);
    }
    let b = frame_at(&mut app, 80, 14);
    assert!(find(&b, "Resolve conflict…").is_some() && find(&b, "│ Edit ").is_none(), "{}", text(&b));
    for _ in 0..30 {
        app.handle_key(key(KeyCode::Up));
        frame_at(&mut app, 80, 14);
    }
    assert!(find(&frame_at(&mut app, 80, 14), "│ Edit ").is_some());
    for _ in 0..30 {
        app.handle_key(key(KeyCode::Down));
    }
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(pair_shown(&mut app), "NAS");
}

#[test]
fn the_reading_pane_marks_a_conflict_copy_s_title_line() {
    let (_d, mut app) = lab();
    app.show_reading = true;
    select(&mut app, "Homelab");
    let b = frame(&mut app);
    let (x, y) = warning_after(&b, "## NAS");
    assert_eq!(b[(x, y)].fg, ratatui::style::Color::Yellow);
    assert!(line(&b, y).contains("## NAS ⚠  other device · PHONE 09-27 10:00"), "{}", line(&b, y));
    assert_eq!(text(&b).matches("## NAS").count(), 2, "{}", text(&b));
    warning_after(&b, "☑ Label the cables");
    // its ⚠ opens its pair, and the pane is on Homelab after
    click_at(&mut app, x, y, MouseButton::Left);
    assert_eq!(pair_shown(&mut app), "NAS");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(selected(&app), "Homelab");
    // the copy's own pane says so in its border
    app.cursor = app.rows().iter().rposition(|r| app.title_of(r.nref) == "NAS").unwrap();
    let b = frame(&mut app);
    assert!(find(&b, "╭ NAS ⚠ ─").is_some(), "{}", text(&b));
}

#[test]
fn enter_on_a_copy_s_heading_in_the_reading_pane_zooms_into_it_unfolded() {
    let (_d, mut app) = lab();
    app.show_reading = true;
    select(&mut app, "Homelab");
    app.handle_key(key(KeyCode::Tab));
    // down to the copy's heading, the second ## NAS
    let at = app.reading_doc_pub().lines.iter().rposition(|l| l.starts_with("## NAS")).unwrap();
    for _ in 0..at {
        press(&mut app, "j");
        draw(&mut app);
    }
    assert_eq!(app.read_cursor_pub(), at);
    app.handle_key(key(KeyCode::Enter));
    // as a double-click or the outline's Enter: it shows what is in it
    let s = screen(&mut app);
    assert!(s.contains("▾ NAS ⚠") && s.contains("Snapshot policy"), "{}", s);
    assert_eq!(app.rows().len(), 3, "{}", s);
}

#[test]
fn move_to_leaves_conflict_copies_out_and_other_pickers_mark_them() {
    let (_d, mut app) = lab();
    select(&mut app, "Replace fan");
    press(&mut app, "r");
    press(&mut app, "Snapshot");
    draw(&mut app);
    assert!(app.hit_pos(Hit::PickRow(0)).is_some());
    assert!(app.hit_pos(Hit::PickRow(1)).is_none(), "{}", screen(&mut app));
    app.handle_key(key(KeyCode::Esc));
    // Go to… and the filter list both, the one in the copy with its ⚠
    app.run_action(Action::GoTo);
    press(&mut app, "Snapshot");
    let b = frame(&mut app);
    assert!(app.hit_pos(Hit::PickRow(1)).is_some(), "{}", text(&b));
    assert!(find(&b, "Snapshot policy ⚠  Homelab › NAS").is_some(), "{}", text(&b));
    app.handle_key(key(KeyCode::Esc));
    press(&mut app, "/Snapshot");
    let b = frame(&mut app);
    assert!(app.hit_pos(Hit::FilterRow(1)).is_some(), "{}", text(&b));
    assert_eq!(text(&b).matches("Snapshot policy ⚠  Homelab › NAS").count(), 1, "{}", text(&b));
}

#[test]
fn indent_puts_nothing_into_a_conflict_copy() {
    // the node above Networking is the copy of NAS, folded, which keeping
    // ours trashes (§12.5)
    let (d, mut app) = lab();
    select(&mut app, "Networking");
    press(&mut app, ">");
    assert_eq!(said(&mut app), "can't indent “Networking”: the node above it is a conflict copy");
    assert_eq!(selected(&app), "Networking");
    assert!(has_line(&d, "## Networking") && has_line(&d, "- [ ] Label the cables"), "{}", root(&d));
    // inside a copy, a node still goes under the one above it
    app.cursor = app.rows().iter().rposition(|r| app.title_of(r.nref) == "NAS").unwrap();
    app.handle_key(key(KeyCode::Enter));
    select(&mut app, "Replace fan");
    press(&mut app, ">");
    assert_eq!(said(&mut app), "indented “Replace fan”");
}

#[test]
fn the_editor_s_border_says_when_the_cursor_is_in_a_conflict_copy() {
    let (_d, mut app) = lab();
    app.set_edit_keys(EditKeys::Normal);
    select(&mut app, "Homelab");
    press(&mut app, "e");
    let b = frame(&mut app);
    assert!(find(&b, "Editing Homelab").is_some() && find(&b, "conflict copy").is_none(), "{}", text(&b));
    // down through ours, into the copy of NAS
    let mut seen = None;
    for _ in 0..20 {
        app.handle_key(key(KeyCode::Down));
        let b = frame(&mut app);
        if let Some((_, y)) = find(&b, "conflict copy") {
            seen = Some(line(&b, y));
            break;
        }
    }
    let border = seen.expect("never said so");
    assert!(border.contains("Editing NAS ⚠ conflict copy from PHONE"), "{}", border);
}

// ------------------------------------------------------------ messages

/// The sample vault, as `fold` ships it, cut down.
const SAMPLE: &str = "# Homelab\n\nTwo boxes in the closet.\n\n## NAS\n\nMirrored pairs, no raidz.\n\n### [ ] Snapshot policy\n\n- hourly, keep 24\n- daily, keep 30\n\n### [x] Replace fan\n\n## Networking\n\n- [ ] Replace the flaky switch\n- [ ] Label the cables\n  - [x] patch panel\n  - [ ] rack\n- VLANs: 10 home, 20 iot, 30 guest\n\n# Work\n\n## Project Atlas\n\n- [ ] Draft the RFC\n- [ ] Review PR\n- [x] Kickoff meeting\n\n## Reading list\n\n- The Rust Book\n\n# Inbox\n\n- call the plumber\n- [ ] renew passport\n";

/// What the status bar says on its left, past a mode's badge, as the
/// next frame draws it.
fn said(app: &mut App) -> String {
    let s = status(app);
    let mut parts = s.split("  ").map(str::trim).filter(|p| !p.is_empty());
    let first = parts.next().unwrap_or_default();
    let badge = first.chars().all(|c| c.is_ascii_uppercase());
    if badge { parts.next().unwrap_or_default() } else { first }.to_string()
}

#[test]
fn copy_and_paste_name_the_node_in_the_menu_s_words() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "NAS");
    press(&mut app, "p");
    assert_eq!(said(&mut app), "nothing copied yet · y copies a node");
    press(&mut app, "y");
    assert_eq!(said(&mut app), "copied “NAS” · p pastes");
    select(&mut app, "Reading list");
    press(&mut app, "p");
    assert_eq!(said(&mut app), "pasted “NAS”");
    // the Copy button of the node menu says the same
    select(&mut app, "Work");
    app.run_action(Action::Copy);
    assert_eq!(said(&mut app), "copied “Work” · p pastes");
}

#[test]
fn checking_a_task_names_it_and_a_node_that_is_no_task_says_so() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "NAS");
    press(&mut app, "x");
    assert_eq!(said(&mut app), "“NAS” isn't a task · t makes it one");
    assert_eq!(root(&d), SAMPLE);
    press(&mut app, "u");
    assert_eq!(said(&mut app), "nothing to undo");
    select(&mut app, "Draft the RFC");
    press(&mut app, "x");
    assert_eq!(said(&mut app), "done: “Draft the RFC”");
    press(&mut app, "x");
    assert_eq!(said(&mut app), "reopened: “Draft the RFC”");
    press(&mut app, "t");
    assert_eq!(said(&mut app), "removed the checkbox from “Draft the RFC”");
    press(&mut app, "t");
    assert_eq!(said(&mut app), "made “Draft the RFC” a task");
    // x in the reading pane, on the line of a task, says the same
    app.show_reading = true;
    select(&mut app, "Project Atlas");
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "jj");
    press(&mut app, "x");
    assert!(root(&d).contains("- [x] Draft the RFC"), "{}", root(&d));
    assert_eq!(said(&mut app), "done: “Draft the RFC”");
}

#[test]
fn delete_and_make_block_name_the_node_and_show_no_file_or_id() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "Homelab");
    press(&mut app, "d");
    assert_eq!(said(&mut app), "deleted “Homelab” (12 nodes) · u undoes");
    press(&mut app, "u");
    select(&mut app, "rack");
    press(&mut app, "s");
    assert_eq!(said(&mut app), "gave “rack” its own file");
    // a block is deleted like any node: by name, its file unsaid
    press(&mut app, "d");
    assert_eq!(said(&mut app), "deleted “rack” · u undoes");
}

#[test]
fn moves_name_the_node_and_say_in_words_where_the_ordering_rule_put_it() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "rack");
    press(&mut app, ">");
    assert_eq!(said(&mut app), "indented “rack”");
    press(&mut app, "<");
    assert_eq!(said(&mut app), "outdented “rack”");
    // nowhere to go: nothing changed, and the words do not say it did
    let before = root(&d);
    select(&mut app, "patch panel");
    press(&mut app, ">");
    assert_eq!(said(&mut app), "can't indent “patch panel”: nothing above it to go under");
    select(&mut app, "Homelab");
    press(&mut app, "<");
    assert_eq!(said(&mut app), "can't outdent “Homelab”: it is already at the top level");
    assert_eq!(root(&d), before);
    // out of a heading among headings, an item goes before them
    select(&mut app, "hourly, keep 24");
    press(&mut app, "<");
    assert_eq!(said(&mut app), "outdented “hourly, keep 24” — placed before the sections");
    // a bullet made a heading goes after the bullets
    select(&mut app, "Replace the flaky switch");
    press(&mut app, "~");
    assert_eq!(said(&mut app), "made “Replace the flaky switch” a heading — placed after the items");
    // a heading made a bullet, before the headings
    select(&mut app, "Replace fan");
    press(&mut app, "~");
    assert_eq!(said(&mut app), "made “Replace fan” a bullet — placed before the sections");
    // Move to… names where to
    select(&mut app, "call the plumber");
    press(&mut app, "r");
    press(&mut app, "Project Atlas");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(said(&mut app), "moved “call the plumber” to “Project Atlas”");
    select(&mut app, "renew passport");
    press(&mut app, "r");
    press(&mut app, "Homelab");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(said(&mut app), "moved “renew passport” to “Homelab” — placed before the sections");
    select(&mut app, "Kickoff meeting");
    press(&mut app, "za");
    assert_eq!(said(&mut app), "archived “Kickoff meeting”");
    assert!(!status(&mut app).contains('§'));
}

#[test]
fn a_drop_the_ordering_rule_placed_says_where_in_words() {
    let d = vault("# A\n\n- a1\n\n## S\n\n# B\n\n- b1\n");
    let mut app = App::new(d.path()).unwrap();
    // rows: A, a1, S, B, b1; b1 onto A's title goes into A, before S
    draw(&mut app);
    let (from, onto) = (app.hit_pos(Hit::Row(4)).unwrap(), app.hit_pos(Hit::Row(0)).unwrap());
    let at = |kind, (column, row)| MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    app.handle_mouse(at(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(at(MouseEventKind::Drag(MouseButton::Left), onto));
    draw(&mut app);
    app.handle_mouse(at(MouseEventKind::Up(MouseButton::Left), onto));
    let r = root(&d);
    assert!(r.find("- b1").is_some_and(|i| i < r.find("## S").unwrap()), "{}", r);
    assert_eq!(said(&mut app), "moved “b1” — placed before the sections");
}

#[test]
fn clear_done_counts_the_done_tasks_it_cleared() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    // under the zoom, only there
    select(&mut app, "Networking");
    app.handle_key(key(KeyCode::Enter));
    app.run_action(Action::ClearDone);
    assert_eq!(said(&mut app), "cleared 1 done task under “Networking”");
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Backspace));
    app.run_action(Action::ClearDone);
    assert_eq!(said(&mut app), "cleared 2 done tasks");
    app.run_action(Action::ClearDone);
    assert_eq!(said(&mut app), "no done tasks to clear");
}

#[test]
fn the_editor_s_own_saves_say_nothing_and_ctrl_s_says_saved() {
    let d = vault("# Inbox\n\nnotes\n");
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(EditKeys::Normal);
    press(&mut app, "e");
    app.handle_key(key(KeyCode::End));
    app.handle_key(key(KeyCode::Char('!')));
    // the pause saves it; the status bar's right side says so, the left
    // side nothing
    std::thread::sleep(Duration::from_millis(800));
    app.tick();
    assert_eq!(root(&d), "# Inbox!\n\nnotes\n");
    let s = said(&mut app);
    assert!(!s.contains("saved") && !s.contains("block"), "{}", s);
    // leaving saves too, as quietly
    app.handle_key(key(KeyCode::Char('?')));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), "# Inbox!?\n\nnotes\n");
    let s = said(&mut app);
    assert!(!s.contains("saved") && !s.contains("block"), "{}", s);
    // Ctrl-S is asked for, and answered
    press(&mut app, "e");
    app.handle_key(key(KeyCode::End));
    app.handle_key(key(KeyCode::Char('.')));
    app.handle_key(ctrl('s'));
    assert_eq!(root(&d), "# Inbox!?.\n\nnotes\n");
    assert_eq!(said(&mut app), "saved");
}

// ------------------------------------------------------------ the next step

/// What the status bar says once its message has gone: the keys of what
/// is on screen (§10.1).
const OUTLINE_KEYS: &str = "n new · e edit · x done · m menu · / find · ? help";
const READING_KEYS: &str = "e edit · Enter zoom/follow · Tab outline";
const EDITOR_KEYS: &str = "Esc done · Ctrl-S save · Ctrl-Z undo";
const VIM_KEYS: &str = "i insert · :wq done · :q! revert";
const VIM_INSERT_KEYS: &str = "Esc normal mode · :wq done · :q! revert";
const PROPS_KEYS: &str = "n add · Enter change · d delete · Esc close";

#[test]
fn a_message_gives_way_to_the_keys_once_it_is_old_and_something_was_done_since() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "NAS");
    press(&mut app, "y");
    assert_eq!(said(&mut app), "copied “NAS” · p pastes");
    // a key since, but the message is new: it stays
    press(&mut app, "j");
    assert_eq!(said(&mut app), "copied “NAS” · p pastes");
    // five seconds on, with no key since the key: its time is over
    std::thread::sleep(Duration::from_millis(5100));
    assert_eq!(said(&mut app), OUTLINE_KEYS);
}

#[test]
fn a_message_no_key_came_after_stays_until_one_does() {
    // for someone who looked away: the message waits for them
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "Draft the RFC");
    press(&mut app, "x");
    std::thread::sleep(Duration::from_millis(5100));
    assert_eq!(said(&mut app), "done: “Draft the RFC”");
    press(&mut app, "k");
    assert_eq!(said(&mut app), OUTLINE_KEYS);
    // the greeting is a message like any other
    let mut app = App::new(d.path()).unwrap();
    assert!(said(&mut app).contains("right-click for actions"), "{}", said(&mut app));
}

#[test]
fn an_error_or_refusal_stays_until_the_next_key_after_it_was_read() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "Homelab");
    press(&mut app, "<");
    let refusal = "can't outdent “Homelab”: it is already at the top level";
    assert_eq!(said(&mut app), refusal);
    // a key right after, while it was being read: it stays, however long
    press(&mut app, "j");
    std::thread::sleep(Duration::from_millis(5100));
    assert_eq!(said(&mut app), refusal);
    // the next key gets it out of the way
    press(&mut app, "k");
    assert_eq!(said(&mut app), OUTLINE_KEYS);
}

#[test]
fn each_mode_shows_its_own_keys() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    // the outline's greeting does not follow into the editor, where ?
    // types a ?
    select(&mut app, "NAS");
    press(&mut app, "e");
    let s = said(&mut app);
    assert!(s == EDITOR_KEYS || s == VIM_KEYS, "{}", s);
    app.handle_key(key(KeyCode::Esc));
    // once a message has gone, the keys of what is on screen
    app.set_edit_keys(EditKeys::Normal);
    app.say("");
    assert_eq!(said(&mut app), OUTLINE_KEYS);
    press(&mut app, "e");
    assert_eq!(said(&mut app), EDITOR_KEYS);
    app.handle_key(key(KeyCode::Esc));
    // in Vim and Helix, Esc never leaves the editor
    for keys in [EditKeys::Vim, EditKeys::Helix] {
        app.set_edit_keys(keys);
        press(&mut app, "e");
        app.say("");
        assert_eq!(said(&mut app), VIM_KEYS);
        press(&mut app, "i");
        assert_eq!(said(&mut app), VIM_INSERT_KEYS);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(said(&mut app), VIM_KEYS);
        press(&mut app, ":wq");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.mode_pub(), "normal");
        assert_eq!(said(&mut app), OUTLINE_KEYS);
    }
    press(&mut app, "a");
    assert_eq!(said(&mut app), PROPS_KEYS);
    app.handle_key(key(KeyCode::Esc));
    app.show_reading = true;
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(said(&mut app), READING_KEYS);
    assert_eq!(root(&d), SAMPLE);
}

#[test]
fn the_conflict_view_shows_its_keys_while_it_has_a_pair_to_resolve() {
    let (_d, mut app) = lab();
    app.run_action(Action::ResolveConflicts);
    app.say("");
    assert_eq!(said(&mut app), "o ours · t theirs · b both · n next · Esc close");
    press(&mut app, "tt");
    assert_eq!(pairs(&mut app), 0);
    app.say("");
    assert_eq!(said(&mut app), "Esc close");
}

#[test]
fn z_and_g_show_what_can_follow_and_a_wrong_second_key_says_so() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    press(&mut app, "z");
    assert_eq!(said(&mut app), "z… p pane · w wrap · d hide done · r raw · a archive");
    // zq quits nothing, and says so
    press(&mut app, "q");
    assert!(!app.quit_requested());
    assert_eq!(said(&mut app), "zq does nothing · after z press p, w, d, r or a");
    press(&mut app, "g");
    assert_eq!(said(&mut app), "g… g top");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(said(&mut app), "g Enter does nothing · after g press g");
    // Esc lets the first key go, quietly
    press(&mut app, "z");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(said(&mut app), "g Enter does nothing · after g press g");
    // zd's words say what it would do now, and lead while done is hidden
    press(&mut app, "zd");
    press(&mut app, "z");
    assert_eq!(said(&mut app), "z… d show done · p pane · w wrap · r raw · a archive");
    app.handle_key(key(KeyCode::Esc));
    // [[ and ]] in the reading pane
    app.show_reading = true;
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "]");
    assert_eq!(said(&mut app), "]… ] next heading");
    assert_eq!(root(&d), SAMPLE);
}

#[test]
fn keys_fold_does_not_use_say_what_to_press_instead() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "NAS");
    press(&mut app, "i");
    assert_eq!(said(&mut app), "i does nothing here: e edits");
    press(&mut app, "o");
    assert_eq!(said(&mut app), "o does nothing here: n adds a node below");
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(said(&mut app), "Delete does nothing here: d deletes");
    app.handle_key(ctrl('z'));
    assert_eq!(said(&mut app), "Ctrl-Z does nothing here: u undoes");
    app.handle_key(ctrl('f'));
    assert_eq!(said(&mut app), "Ctrl-F does nothing here: / finds");
    assert_eq!(root(&d), SAMPLE);
    assert_eq!(app.mode_pub(), "normal");
    // F1 is help, from the outline, the reading pane and the editor, where
    // ? types a ?
    app.handle_key(key(KeyCode::F(1)));
    assert_eq!(app.mode_pub(), "help");
    app.handle_key(key(KeyCode::Esc));
    app.show_reading = true;
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "i");
    assert_eq!(said(&mut app), "i does nothing here: e edits");
    app.handle_key(ctrl('f'));
    assert_eq!(said(&mut app), "Ctrl-F does nothing here: / searches");
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(said(&mut app), "Delete does nothing here: Tab, then d deletes");
    app.handle_key(key(KeyCode::F(1)));
    assert_eq!(app.mode_pub(), "help");
    app.handle_key(key(KeyCode::Esc));
    app.set_edit_keys(EditKeys::Normal);
    press(&mut app, "e");
    app.handle_key(key(KeyCode::F(1)));
    assert_eq!(app.mode_pub(), "help");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode_pub(), "edit");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(root(&d), SAMPLE);
}

#[test]
fn undo_and_redo_say_in_words_what_they_undid() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    select(&mut app, "Draft the RFC");
    press(&mut app, "xu");
    assert_eq!(said(&mut app), "undone: mark “Draft the RFC” done");
    press(&mut app, "U");
    assert_eq!(said(&mut app), "redone: mark “Draft the RFC” done");
    press(&mut app, "xu");
    assert_eq!(said(&mut app), "undone: reopen “Draft the RFC”");
    press(&mut app, "U");
    assert_eq!(said(&mut app), "redone: reopen “Draft the RFC”");
    select(&mut app, "Homelab");
    press(&mut app, "du");
    assert_eq!(said(&mut app), "undone: delete “Homelab”");
    select(&mut app, "rack");
    press(&mut app, ">u");
    assert_eq!(said(&mut app), "undone: indent “rack”");
    select(&mut app, "rack");
    press(&mut app, "tu");
    assert_eq!(said(&mut app), "undone: remove the checkbox from “rack”");
    select(&mut app, "call the plumber");
    press(&mut app, "r");
    press(&mut app, "Project Atlas");
    app.handle_key(key(KeyCode::Enter));
    press(&mut app, "u");
    assert_eq!(said(&mut app), "undone: move “call the plumber” to “Project Atlas”");
    // the editor's saves, by the node written
    app.set_edit_keys(EditKeys::Normal);
    select(&mut app, "NAS");
    press(&mut app, "e");
    app.handle_key(key(KeyCode::End));
    press(&mut app, " box");
    app.handle_key(key(KeyCode::Esc));
    press(&mut app, "u");
    assert_eq!(said(&mut app), "undone: edit “NAS”");
    // the reading pane's x, as the outline's
    app.show_reading = true;
    select(&mut app, "Project Atlas");
    app.handle_key(key(KeyCode::Tab));
    press(&mut app, "jjxu");
    assert_eq!(said(&mut app), "undone: mark “Draft the RFC” done");
    assert_eq!(root(&d), SAMPLE);
    // a refusal names the file, and the step in words (§10.10)
    app.handle_key(key(KeyCode::Tab));
    select(&mut app, "rack");
    press(&mut app, "x");
    std::fs::write(d.path().join("root.md"), root(&d).replace("VLANs", "VLAN")).unwrap();
    press(&mut app, "u");
    assert_eq!(said(&mut app), "undo refused: root.md changed since mark “rack” done; not overwriting");
}

#[test]
fn a_hint_too_long_for_the_bar_loses_whole_parts_and_keeps_the_way_to_help() {
    let d = vault(SAMPLE);
    let mut app = App::new(d.path()).unwrap();
    press(&mut app, "zd");
    app.say("");
    // 80 columns, with done hidden, the file and the save state on the right
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    let bar: String = (0..80).map(|x| b[(x, 23)].symbol()).collect();
    assert!(bar.contains("n new · e edit · x done · m menu · ? help  "), "{}", bar);
    assert!(bar.contains("done hidden") && !bar.contains('…'), "{}", bar);
}

#[test]
fn a_short_bar_keeps_the_key_that_shows_hidden_done_tasks_in_the_z_hint() {
    // 80 columns, with the conflicts and done hidden on the right
    let (_d, mut app) = lab();
    press(&mut app, "zd");
    app.say("");
    press(&mut app, "z");
    let bar = status80(&mut app);
    assert!(bar.contains("⚠ 2 conflicts") && bar.contains("done hidden"), "{}", bar);
    assert!(bar.contains("d show done") && bar.contains("p pane"), "{}", bar);
    assert_eq!(bar.matches('…').count(), 1, "{}", bar);
    // Esc lets z go; zd shows them again
    app.handle_key(key(KeyCode::Esc));
    press(&mut app, "zd");
    assert!(!status80(&mut app).contains("done hidden"));
}
