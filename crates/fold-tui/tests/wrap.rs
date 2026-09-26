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
    let mut app = App::new(d.path()).unwrap();
    // these tests are about the reading pane, hidden by default
    app.show_reading = true;
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

#[test]
fn tabs_in_code_keep_their_indentation() {
    // a tab-indented line (Go, a Makefile recipe) is drawn indented, not
    // flush with the line above: raw text is never hidden (§10.9)
    let (_d, mut app) = app_with("# A\n\n```\nflush\n\tindented\n```\n");
    let s = draw(&mut app);
    let f = s.iter().find(|l| l.contains("flush")).unwrap();
    let i = s.iter().find(|l| l.contains("indented")).unwrap();
    // a tab reaches the next stop, every 4 columns
    assert_eq!(col(i, "indented"), col(f, "flush") + 4, "reading pane: {:#?}", s);
}

#[test]
fn tabs_in_code_keep_their_indentation_in_the_editor() {
    // the editor shows the tab as whitespace too, and its cursor agrees
    let (_d, mut app) = app_with("# A\n\n```\nflush\n\tindented\n```\n");
    app.show_reading = false;
    keys(&mut app, "e");
    let s = draw(&mut app);
    let f = s.iter().find(|l| l.contains("flush")).unwrap();
    let y = s.iter().position(|l| l.contains("indented")).unwrap();
    assert_eq!(col(&s[y], "indented"), col(f, "flush") + 4, "editor: {:#?}", s);
    // the cursor just past the tab (Home from the line's end goes to the
    // first non-blank) sits on the `i`
    for _ in 0..4 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Home);
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let p = t.get_cursor_position().unwrap();
    assert_eq!((p.x, p.y), (col(&s[y], "indented"), y as u16), "editor: {:#?}", s);
}

#[test]
fn a_very_long_line_does_not_overflow_the_styler() {
    // over 65,535 columns in all, with a stray `*` so the styler pushes it in
    // several pieces, and a link 65,541 columns in
    let (_d, mut app) = app_with(&format!("# A\n\n{} * {} [docs](x)\n", "a".repeat(40000), "b".repeat(25536)));
    let s = draw(&mut app);
    assert!(s.iter().any(|l| l.contains("aaaa")), "{:#?}", s);
    // zw: cut at the edge; the link is far off screen, not wrapped round
    // into a u16 over the start of the line
    keys(&mut app, "zw");
    let s = draw(&mut app);
    let y = s.iter().position(|l| l.contains("aaaa")).unwrap();
    for x in 0..W {
        assert_ne!(app.hit_at(x, y as u16), Some(Hit::Link(2)), "{:#?}", s);
    }
}

#[test]
fn a_link_after_wide_text_is_clickable_where_drawn() {
    // 28 wide characters fill the first row; the link is drawn on the second
    let line = format!("{} see [docs](https://x.org)", "漢".repeat(28));
    let (_d, mut app) = app_with(&format!("# A\n\n{}\n", line));
    let s = draw(&mut app);
    let y = s.iter().position(|l| l.contains("[docs]")).unwrap();
    let x = col(&s[y], "docs");
    assert_eq!(app.hit_at(x, y as u16), Some(Hit::Link(2)), "{:#?}", s);
}

#[test]
fn wide_text_on_an_earlier_row_does_not_shift_the_link_region() {
    // 13 wide characters fill the first row, the link sits early on the second
    let line = format!("{} {}see [docs](x) and plain words", "漢".repeat(13), "word ".repeat(6));
    let (_d, mut app) = app_with(&format!("# A\n\n{}\n", line));
    let s = draw(&mut app);
    let y = s.iter().position(|l| l.contains("[docs]")).unwrap();
    let docs = col(&s[y], "docs");
    let plain = col(&s[y], "plain");
    assert_eq!(app.hit_at(plain, y as u16), Some(Hit::DocLine(2)), "plain text is not the link: {:#?}", s);
    assert_eq!(app.hit_at(docs, y as u16), Some(Hit::Link(2)), "{:#?}", s);
}

#[test]
fn a_checkbox_and_link_after_a_tab_are_clickable_where_drawn() {
    // a tab before them is drawn as spaces to its tab stop
    let (_d, mut app) = app_with("# A\n\n- [ ] x\tsee [docs](x)\n");
    let s = draw(&mut app);
    // the reading pane's row with the item (the outline may show it too, on
    // the left)
    let y = s.iter().rposition(|l| l.contains("[docs]")).unwrap();
    let rcol = |n: &str| s[y][..s[y].rfind(n).unwrap()].chars().count() as u16;
    assert_eq!(app.hit_at(rcol("☐"), y as u16), Some(Hit::DocCheck(2)), "{:#?}", s);
    // exactly the link text is the link, not the brackets around it
    let docs = rcol("docs");
    for x in docs..docs + 4 {
        assert_eq!(app.hit_at(x, y as u16), Some(Hit::Link(2)), "{:#?}", s);
    }
    for x in [docs - 1, docs + 4] {
        assert_eq!(app.hit_at(x, y as u16), Some(Hit::DocLine(2)), "{:#?}", s);
    }
}

#[test]
fn a_link_wrapped_onto_the_next_row_is_clickable_on_both() {
    let line = format!("{} [alpha beta gamma delta epsilon zeta](u) end", "x".repeat(40));
    let (_d, mut app) = app_with(&format!("# A\n\n{}\n", line));
    let s = draw(&mut app);
    let y0 = s.iter().position(|l| l.contains("[alpha")).unwrap();
    let y1 = s.iter().position(|l| l.contains("gamma")).unwrap();
    assert_eq!(y1, y0 + 1, "{:#?}", s);
    assert_eq!(app.hit_at(col(&s[y0], "alpha"), y0 as u16), Some(Hit::Link(2)), "{:#?}", s);
    assert_eq!(app.hit_at(col(&s[y1], "gamma"), y1 as u16), Some(Hit::Link(2)), "{:#?}", s);
    assert_eq!(app.hit_at(col(&s[y1], "end"), y1 as u16), Some(Hit::DocLine(2)), "{:#?}", s);
}
