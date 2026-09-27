//! What the app does over time, as `fold` runs it: the watcher, the
//! autosave and the status line (§10.6, §11.2), where the selection goes
//! when rows come and go (§8.5), keys on a selection the wheel left out
//! of view (§10.1), and editor text no save can take when fold ends or
//! reverts (§10.6).

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
    app.vault_conflict_files().unwrap();
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
    assert!(s.contains("↻ changed outside fold: Inbox (+1 item) · saved 1 block(s) (external change)"), "{}", s);
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
    assert!(s.contains("“task number 2” is selected, press x again to mark it done"), "{}", s);
    press(&mut app, "x");
    assert!(has_line(&d, "- [x] task number 2"));
    // d likewise: nothing trashed until it is in view
    wheel(&mut app, Hit::OutlinePane, 10);
    press(&mut app, "d");
    assert!(has_line(&d, "- [x] task number 2"), "deleted out of view");
    let s = status(&mut app);
    assert!(s.contains("“task number 2” is selected, press d again to delete it"), "{}", s);
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
    assert!(s.contains("“task number 40” is selected, press za again to archive it"), "{}", s);
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
    assert!(s.contains("“buried task” is selected, press x again to mark it done"), "{}", s);
    press(&mut app, "x");
    assert!(has_line(&d, "- [x] buried task"));
    // Enter on a task toggles it: the same
    wheel(&mut app, Hit::ReadingPane, -10);
    app.handle_key(key(KeyCode::Enter));
    assert!(has_line(&d, "- [x] buried task"), "reopened out of view");
    let s = status(&mut app);
    assert!(s.contains("“buried task” is selected, press Enter again to reopen it"), "{}", s);
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
    assert!(s.contains("“task number 2” is selected, press J again to move it down"), "{}", s);
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
