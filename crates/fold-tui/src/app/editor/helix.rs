//! The Helix keymap: selection first. Motions select, actions act on the
//! selection; `v` makes motions extend it. One selection (no multi-cursor).

use super::{order, Editor, Group, Mode, Outcome, Pos};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Default)]
pub struct State {
    count: String,
    prefix: Option<char>,
}

/// The selection as `[start, end)`: the anchor to the cursor, both included;
/// with no anchor, the character under the cursor.
fn range(e: &Editor) -> (Pos, Pos) {
    let (s, en) = match e.anchor {
        Some(a) => order(a, e.cursor),
        None => (e.cursor, e.cursor),
    };
    (s, e.next(en).unwrap_or(Pos::new(en.line, e.len(en.line))))
}

/// Select from the cursor (normal mode) or extend from the anchor (select
/// mode) to `to`.
fn select_to(e: &mut Editor, from: Pos, to: Pos) {
    if e.mode == Mode::Select {
        if e.anchor.is_none() {
            e.anchor = Some(e.cursor);
        }
    } else {
        e.anchor = Some(from);
    }
    e.set_cursor(to);
    if e.anchor == Some(e.cursor) && e.mode != Mode::Select {
        e.anchor = None;
    }
}

/// Move the cursor, collapsing the selection (or extending it in select mode).
fn move_to(e: &mut Editor, to: Pos) {
    if e.mode == Mode::Select {
        if e.anchor.is_none() {
            e.anchor = Some(e.cursor);
        }
    } else {
        e.anchor = None;
    }
    e.set_cursor(to);
}

pub fn handle(e: &mut Editor, key: KeyEvent) -> Outcome {
    match e.mode {
        Mode::Insert => insert(e, key),
        _ => normal(e, key),
    }
}

fn insert(e: &mut Editor, key: KeyEvent) -> Outcome {
    let ctl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            e.mode = Mode::Normal;
            e.group = Group::None;
        }
        KeyCode::Char('c') if ctl => {
            e.mode = Mode::Normal;
            e.group = Group::None;
        }
        KeyCode::Char('w') if ctl => e.delete_word_back(),
        KeyCode::Char(c) if !ctl => e.type_char(c),
        KeyCode::Enter => e.newline(),
        KeyCode::Backspace => e.backspace(),
        KeyCode::Delete => e.delete_forward(),
        KeyCode::Tab => {
            e.type_char(' ');
            e.type_char(' ');
        }
        KeyCode::Left => e.set_cursor(e.prev(e.cursor).unwrap_or(e.cursor)),
        KeyCode::Right => e.set_cursor(e.next(e.cursor).unwrap_or(e.cursor)),
        KeyCode::Up => e.move_vert(-1),
        KeyCode::Down => e.move_vert(1),
        KeyCode::Home => e.set_cursor(Pos::new(e.cursor.line, 0)),
        KeyCode::End => e.set_cursor(Pos::new(e.cursor.line, e.len(e.cursor.line))),
        _ => {}
    }
    Outcome::default()
}

fn to_insert(e: &mut Editor, at: Pos) {
    e.checkpoint();
    e.group = Group::Typing;
    e.mode = Mode::Insert;
    e.anchor = None;
    e.set_cursor(at);
}

fn normal(e: &mut Editor, key: KeyEvent) -> Outcome {
    let ctl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let out = Outcome::default();
    let c = match key.code {
        KeyCode::Esc => {
            e.helix = State::default();
            if e.mode == Mode::Select {
                e.mode = Mode::Normal;
            } else {
                e.anchor = None;
            }
            return out;
        }
        KeyCode::Char('d') | KeyCode::Char('u') if ctl => {
            let half = (e.page / 2).max(1) as isize;
            e.anchor = None;
            e.move_vert(if key.code == KeyCode::Char('d') { half } else { -half });
            return out;
        }
        KeyCode::Char('f') | KeyCode::Char('b') if ctl => {
            let page = e.page.max(2) as isize - 1;
            e.anchor = None;
            e.move_vert(if key.code == KeyCode::Char('f') { page } else { -page });
            return out;
        }
        KeyCode::Char(_) if ctl => return out,
        KeyCode::Char(c) => c,
        KeyCode::Left => 'h',
        KeyCode::Right => 'l',
        KeyCode::Up => 'k',
        KeyCode::Down => 'j',
        KeyCode::Home => {
            move_to(e, Pos::new(e.cursor.line, 0));
            return out;
        }
        KeyCode::End => {
            let l = e.cursor.line;
            move_to(e, Pos::new(l, e.len(l).saturating_sub(1)));
            return out;
        }
        KeyCode::Delete => 'd',
        _ => return out,
    };

    if let Some(p) = e.helix.prefix.take() {
        return prefixed(e, p, c);
    }
    if c.is_ascii_digit() && (c != '0' || !e.helix.count.is_empty()) && !alt {
        e.helix.count.push(c);
        return out;
    }
    let n: usize = std::mem::take(&mut e.helix.count).parse().unwrap_or(1).max(1);
    let p = e.cursor;
    match (c, alt) {
        ('h', _) => move_to(e, Pos::new(p.line, p.col.saturating_sub(n))),
        ('l', _) => move_to(e, Pos::new(p.line, (p.col + n).min(e.len(p.line)))),
        ('j' | 'k', _) => {
            if e.mode == Mode::Select {
                if e.anchor.is_none() {
                    e.anchor = Some(e.cursor);
                }
            } else {
                e.anchor = None;
            }
            e.move_vert(if c == 'j' { n as isize } else { -(n as isize) });
        }
        ('w' | 'W', false) => {
            let big = c == 'W';
            let mut from = p;
            let mut to = p;
            for _ in 0..n {
                from = to;
                let next = e.word_fwd(to, big);
                to = if next > to { e.prev(next).unwrap_or(next) } else { next };
                if to == from {
                    to = e.word_fwd(e.next(to).unwrap_or(to), big);
                }
            }
            select_to(e, from, to);
        }
        ('e' | 'E', false) => {
            let big = c == 'E';
            let mut from = p;
            let mut to = p;
            for _ in 0..n {
                // from here if the word goes on, else from the next character
                let at_end = e.word_end(to, big) != to && e.next(to).map(|q| e.word_at(q, big).0 != e.word_at(to, big).0).unwrap_or(true);
                from = if at_end { e.next(to).unwrap_or(to) } else { to };
                to = e.word_end(to, big);
            }
            select_to(e, from.min(to), to);
        }
        ('b' | 'B', false) => {
            let big = c == 'B';
            let mut from = p;
            let mut to = p;
            for _ in 0..n {
                from = to;
                to = e.word_back(to, big);
            }
            select_to(e, from, to);
        }
        ('x', false) => {
            // select the line; again, extend by a line
            let whole = e.anchor.is_some_and(|a| a.col == 0 && a <= e.cursor) && e.cursor.col >= e.len(e.cursor.line);
            let (l1, l2) = if whole { (e.anchor.unwrap().line, (e.cursor.line + n).min(e.lines() - 1)) } else { (p.line, (p.line + n - 1).min(e.lines() - 1)) };
            e.anchor = Some(Pos::new(l1, 0));
            e.cursor = Pos::new(l2, e.len(l2));
        }
        ('X', false) => {
            let (s, en) = range(e);
            let l2 = if en.col == 0 && en.line > s.line { en.line - 1 } else { en.line };
            e.anchor = Some(Pos::new(s.line, 0));
            e.cursor = Pos::new(l2, e.len(l2));
        }
        ('%', false) => {
            let l = e.lines() - 1;
            e.anchor = Some(Pos::new(0, 0));
            e.cursor = Pos::new(l, e.len(l).saturating_sub(1));
        }
        (';', false) => e.anchor = None,
        (';', true) => {
            if let Some(a) = e.anchor {
                e.anchor = Some(e.cursor);
                e.cursor = a;
            }
        }
        ('g' | 'f' | 't' | 'F' | 'T' | 'r' | 'm', false) => e.helix.prefix = Some(c),
        ('d', _) | ('c', _) => {
            let (s, en) = range(e);
            if s == en {
                return out;
            }
            if c == 'c' {
                to_insert(e, s);
            } else {
                e.checkpoint();
            }
            let gone = e.delete(s, en);
            if !alt {
                let lw = gone.ends_with('\n') && s.col == 0;
                e.copy(gone, lw);
            }
            e.anchor = None;
            e.set_cursor(s);
            if e.mode == Mode::Select {
                e.mode = Mode::Normal;
            }
        }
        ('y', false) => {
            let (s, en) = range(e);
            let t = e.text(s, en);
            let lw = t.ends_with('\n') && s.col == 0;
            e.copy(t, lw);
            e.message = Some("yanked".into());
        }
        ('p' | 'P', false) => {
            let clip = e.clip.clone();
            if clip.text.is_empty() {
                return out;
            }
            e.checkpoint();
            let (s, en) = range(e);
            let at = if clip.linewise {
                if c == 'p' {
                    if en.col == 0 && en.line > s.line { en } else { Pos::new((en.line + 1).min(e.lines()), 0) }
                } else {
                    Pos::new(s.line, 0)
                }
            } else if c == 'p' {
                en
            } else {
                s
            };
            let end = if clip.linewise && at.line >= e.lines() {
                let last = e.lines() - 1;
                e.put_lines(last, &clip.text, true);
                Pos::new(e.lines(), 0)
            } else {
                e.insert(at, &clip.text)
            };
            let at = if clip.linewise && at.line >= e.lines() { Pos::new(e.lines() - clip.text.matches('\n').count(), 0) } else { at };
            e.anchor = Some(at);
            e.cursor = e.prev(end).unwrap_or(end);
        }
        ('R', false) => {
            let clip = e.clip.clone();
            let (s, en) = range(e);
            e.checkpoint();
            e.delete(s, en);
            let end = e.insert(s, &clip.text);
            e.anchor = Some(s);
            e.cursor = e.prev(end).unwrap_or(end);
        }
        ('~' | '`', _) => {
            let (s, en) = range(e);
            e.checkpoint();
            let how = match (c, alt) {
                ('`', true) => 'U',
                ('`', false) => 'u',
                _ => '~',
            };
            e.change_case(s, en, how);
        }
        ('>' | '<', false) => {
            let (s, en) = range(e);
            let l2 = if en.col == 0 && en.line > s.line { en.line - 1 } else { en.line };
            let a = e.anchor;
            for _ in 0..n {
                e.indent(s.line, l2, if c == '>' { 1 } else { -1 });
            }
            e.anchor = a;
        }
        ('J', false) => {
            let (s, en) = range(e);
            let lines = en.line.saturating_sub(s.line).max(1);
            e.join(s.line, lines);
            e.anchor = None;
        }
        ('u', false) => {
            for _ in 0..n {
                e.undo();
            }
        }
        ('U', false) => {
            for _ in 0..n {
                e.redo();
            }
        }
        ('i', false) => {
            let (s, _) = range(e);
            to_insert(e, s);
        }
        ('a', false) => {
            let (_, en) = range(e);
            to_insert(e, en);
        }
        ('I', false) => {
            let l = e.cursor.line;
            to_insert(e, Pos::new(l, e.first_non_blank(l)));
        }
        ('A', false) => {
            let l = e.cursor.line;
            to_insert(e, Pos::new(l, e.len(l)));
        }
        ('o' | 'O', false) => {
            let l = e.cursor.line;
            let indent: String = e.line(l).chars().take_while(|ch| *ch == ' ').collect();
            to_insert(e, e.cursor);
            if c == 'o' {
                let end = Pos::new(l, e.len(l));
                e.cursor = e.insert(end, &format!("\n{}", indent));
            } else {
                e.insert(Pos::new(l, 0), &format!("{}\n", indent));
                e.cursor = Pos::new(l, indent.chars().count());
            }
        }
        ('v', false) => {
            e.mode = if e.mode == Mode::Select { Mode::Normal } else { Mode::Select };
        }
        ('/' | '?', false) => e.open_cmdline(c),
        ('n' | 'N', false) => {
            if let Some(q) = e.search_next(c == 'n') {
                let len = e.search.as_ref().map(|s| s.chars().count()).unwrap_or(1);
                e.anchor = Some(q);
                e.cursor = Pos::new(q.line, (q.col + len).saturating_sub(1));
            }
        }
        ('*', false) => {
            let (s, en) = range(e);
            let t = e.text(s, en);
            if !t.trim().is_empty() && !t.contains('\n') {
                e.message = Some(format!("search: {}", t));
                e.search = Some(t);
            }
        }
        (':', false) => e.open_cmdline(':'),
        _ => {}
    }
    out
}

/// The key after `g`, `f`/`t`/`F`/`T`, `r` or `m`.
fn prefixed(e: &mut Editor, p: char, c: char) -> Outcome {
    let n: usize = std::mem::take(&mut e.helix.count).parse().unwrap_or(1).max(1);
    let cur = e.cursor;
    match p {
        'g' => {
            let last = e.lines() - 1;
            let to = match c {
                'g' => Some(Pos::new(0, 0)),
                'e' => Some(Pos::new(last, 0)),
                'h' => Some(Pos::new(cur.line, 0)),
                'l' => Some(Pos::new(cur.line, e.len(cur.line).saturating_sub(1))),
                's' => Some(Pos::new(cur.line, e.first_non_blank(cur.line))),
                _ => None,
            };
            if let Some(to) = to {
                move_to(e, to);
            }
        }
        'f' | 't' | 'F' | 'T' => {
            if let Some(to) = e.find_char(cur, c, p, n) {
                select_to(e, cur, to);
            }
        }
        'r' => {
            let (s, en) = range(e);
            e.checkpoint();
            let t: String = e.text(s, en).chars().map(|ch| if ch == '\n' { ch } else { c }).collect();
            e.delete(s, en);
            e.insert(s, &t);
        }
        'm' => {
            // mi( / ma" …: select inside / around a pair
            e.helix.prefix = match c {
                'i' => Some('i'),
                'a' => Some('a'),
                'm' => {
                    if let Some(q) = e.match_bracket(cur) {
                        move_to(e, q);
                    }
                    None
                }
                _ => None,
            };
        }
        'i' | 'a' => {
            if let Some((s, en)) = e.text_object(p == 'a', c) {
                e.anchor = Some(s);
                e.cursor = e.prev(en).filter(|q| *q >= s).unwrap_or(s);
            }
        }
        _ => {}
    }
    Outcome::default()
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{body, editor, keys};
    use super::super::{Keys, Mode, Pos};

    #[test]
    fn select_then_act() {
        let (_d, mut e) = editor("one two three\n", Keys::Helix);
        keys(&mut e, "wd");
        assert_eq!(body(&e), "two three");
        keys(&mut e, "ec2<Esc>");
        assert_eq!(body(&e), "2 three");
        keys(&mut e, "u");
        assert_eq!(body(&e), "two three");
        keys(&mut e, "U");
        assert_eq!(body(&e), "2 three");
    }

    #[test]
    fn lines_yank_paste() {
        let (_d, mut e) = editor("a\nb\nc\n", Keys::Helix);
        keys(&mut e, "xyp");
        assert_eq!(body(&e), "a\na\nb\nc");
        // the pasted line stays selected; collapse, then x x takes two lines
        keys(&mut e, ";xxd");
        assert_eq!(body(&e), "a\nc");
        // the cursor is on "c" after the delete
        keys(&mut e, "x>");
        assert_eq!(body(&e), "a\n  c");
    }

    #[test]
    fn select_mode_find_and_surround_objects() {
        let (_d, mut e) = editor("f(x, y) + z\n", Keys::Helix);
        keys(&mut e, "t,");
        assert_eq!(e.selection(), Some((Pos::new(2, 0), Pos::new(2, 3))));
        keys(&mut e, ";mi(c");
        assert_eq!(e.mode, Mode::Insert);
        keys(&mut e, "a<Esc>");
        assert_eq!(body(&e), "f(a) + z");
        keys(&mut e, "ghvllld");
        assert_eq!(body(&e), " + z");
    }

    #[test]
    fn insert_append_open_and_commands() {
        let (_d, mut e) = editor("mid\n", Keys::Helix);
        keys(&mut e, "Iat <Esc>Aend<Esc>");
        assert_eq!(body(&e), "at midend");
        keys(&mut e, "onext<Esc>");
        assert_eq!(body(&e), "at midend\nnext");
        let out = keys(&mut e, ":w<CR>");
        assert!(out.last().unwrap().save);
        let out = keys(&mut e, ":q<CR>");
        assert!(out.last().unwrap().close);
    }
}
