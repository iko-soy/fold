//! Long lines wrap (§10.1): in the reading pane and the editor, with code
//! broken hard and `zw` to cut lines at the edge instead.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use fold_tui::app::{App, EditKeys, Hit};
use ratatui::{backend::TestBackend, Terminal};

const W: u16 = 90;
const H: u16 = 20;

fn app_with(text: &str) -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), text).unwrap();
    let app = App::new(d.path()).unwrap();
    (d, app)
}

fn draw(app: &mut App) -> Vec<String> {
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let b = t.backend().buffer().clone();
    (0..H).map(|y| (0..W).map(|x| b[(x, y)].symbol()).collect::<String>()).collect()
}

/// Character column of a substring in a screen row.
fn col(line: &str, needle: &str) -> u16 {
    line[..line.find(needle).unwrap()].chars().count() as u16
}

fn key(app: &mut App, c: KeyCode) {
    app.handle_key(KeyEvent::new(c, KeyModifiers::NONE));
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        key(app, KeyCode::Char(c));
    }
}

const LONG: &str = "Mirrored pairs, no raidz. Snapshots hourly via sanoid and pruned daily, replicated to the offsite box every night.";

#[test]
fn the_reading_pane_wraps_and_every_row_clicks_through() {
    let (_d, mut app) = app_with(&format!("# A\n\n{}\n\nlast\n", LONG));
    let s = draw(&mut app);
    let first = s.iter().position(|l| l.contains("Mirrored pairs")).unwrap();
    assert!(s.iter().any(|l| l.contains("night.")), "{:#?}", s);
    // the second row of the paragraph is still the paragraph's line
    let second = &s[first + 1];
    let x = second.chars().position(|c| c.is_alphabetic()).unwrap_or(40) as u16;
    assert_eq!(app.hit_at(x, first as u16 + 1), Some(Hit::DocLine(2)), "{:#?}", s);
    // zw: cut at the edge instead
    keys(&mut app, "zw");
    let s = draw(&mut app);
    assert!(!s.iter().any(|l| l.contains("night.")));
}

#[test]
fn list_items_hang_under_their_text() {
    let (_d, mut app) = app_with(&format!("# A\n\n- [ ] {}\n", LONG));
    let s = draw(&mut app);
    // the reading pane: the row with "- ☐ Mirrored" and the one after it
    let first = s.iter().position(|l| l.contains("- ☐ Mirrored")).unwrap();
    let text_col = col(&s[first], "Mirrored");
    let next: Vec<char> = s[first + 1].chars().collect();
    let pane = col(&s[first], "- ☐") as usize;
    let cont = (pane..next.len()).find(|&i| !next[i].is_whitespace()).unwrap() as u16;
    assert_eq!(cont, text_col, "{:#?}", &s[first..first + 2]);
}

#[test]
fn code_breaks_hard_with_a_marker() {
    let code = "let replicated = snapshots.iter().filter(|s| s.age() < max_age).map(|s| s.send(offsite)).count();";
    let (_d, mut app) = app_with(&format!("# A\n\n```rust\n{}\n```\n", code));
    let s = draw(&mut app);
    assert!(s.iter().any(|l| l.contains('↪')), "{:#?}", s);
}

#[test]
fn editor_arrows_move_by_screen_row_and_vim_j_by_line() {
    let (_d, mut app) = app_with(&format!("# A\n\n{}\nnext\n", LONG));
    keys(&mut app, "e");
    draw(&mut app);
    // normal keymap: Down from the title goes through blank, then the rows
    // of the long line, before reaching "next"
    for _ in 0..3 {
        key(&mut app, KeyCode::Down);
    }
    draw(&mut app);
    keys(&mut app, "Z");
    key(&mut app, KeyCode::Esc);
    let text = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(!text.contains("next\nZ") && !text.starts_with("Z"), "Z landed inside the long line: {}", text);
    assert!(text.contains("Z"), "{}", text);
    let line = text.lines().find(|l| l.contains('Z')).unwrap();
    assert!(line.starts_with("Mirrored") && !line.starts_with("Z"), "{}", line);

    // vim: j goes to the next line, gj to the next screen row
    let (_d, mut app) = app_with(&format!("# A\n\n{}\nnext\n", LONG));
    app.set_edit_keys(EditKeys::Vim);
    keys(&mut app, "e");
    draw(&mut app);
    keys(&mut app, "jjjiY");
    key(&mut app, KeyCode::Esc);
    keys(&mut app, "kgjiG");
    key(&mut app, KeyCode::Esc);
    keys(&mut app, ":wq");
    key(&mut app, KeyCode::Enter);
    let text = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(text.contains("\nYnext\n"), "{}", text);
    let long = text.lines().find(|l| l.starts_with("Mirrored")).unwrap();
    assert!(long.contains('G') && !long.starts_with('G'), "gj stays inside the long line: {}", long);
}

#[test]
fn clicking_a_wrapped_row_in_the_editor_places_the_cursor_there() {
    let (_d, mut app) = app_with(&format!("# A\n\n{}\n", LONG));
    keys(&mut app, "e");
    let s = draw(&mut app);
    let y = s.iter().position(|l| l.contains("offsite")).unwrap() as u16;
    let x = col(&s[y as usize], "offsite");
    let ev = |kind| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    app.handle_mouse(ev(MouseEventKind::Down(MouseButton::Left)));
    app.handle_mouse(ev(MouseEventKind::Up(MouseButton::Left)));
    keys(&mut app, "#");
    key(&mut app, KeyCode::Esc);
    let text = std::fs::read_to_string(app.vault_dir().join("root.md")).unwrap();
    assert!(text.contains("the #offsite box"), "{}", text);
}
