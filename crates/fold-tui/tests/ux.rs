//! What the app does over time, as `fold` runs it: the watcher, the
//! autosave and the status line (§10.6, §11.2), and where the selection
//! goes when rows come and go (§8.5).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use fold_tui::app::{Action, App, EditKeys};
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
