//! The Helix keymap: selection first. Motions select, actions act on the
//! selection; `v` makes motions extend it. One selection (no multi-cursor).

use super::{class, order, Editor, Group, Mode, Outcome, Pos};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Default)]
pub struct State {
    count: String,
    prefix: Option<char>,
}

/// The largest count: more digits than that are this.
const MAX_COUNT: usize = 999_999_999;

/// The most text a count may make `>` add.
const MAX_TEXT: usize = 4 << 20;

/// The count typed before a key, 1 if none.
fn take_count(st: &mut State) -> usize {
    let s = std::mem::take(&mut st.count);
    if s.is_empty() { 1 } else { s.parse().map_or(MAX_COUNT, |n: usize| n.clamp(1, MAX_COUNT)) }
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

/// Helix's word boundary: a change of class, a line end being a class of
/// its own.
fn boundary(a: char, b: char, big: bool) -> bool {
    class(a, big) != class(b, big) || (a == '\n') != (b == '\n')
}

/// Select the `n`th word forward or back, as Helix's word motions do: each
/// walks from the cursor until `target` holds between the character behind
/// and the one ahead, and selects what it walked over. A target right at the
/// cursor (it ends a word, or the whitespace after one) and line ends right
/// ahead move the start past them, so a repeated `w` or `b` selects the next
/// word rather than keeping the last character of this one.
fn select_words(e: &mut Editor, n: usize, fwd: bool, target: &dyn Fn(char, char) -> bool) {
    let mut sel = None;
    for _ in 0..n {
        let cur = sel.map_or(e.cursor, |(_, c)| c);
        match word_step(e, cur, fwd, target) {
            Some(s) => sel = Some(s),
            None => break,
        }
    }
    if let Some((from, to)) = sel {
        select_to(e, from, to);
    }
}

/// One word from `cur`: the selection's anchor and cursor, or `None` at the
/// end (start) of the text. A place between two characters is named by the
/// position of the one after it.
fn word_step(e: &Editor, cur: Pos, fwd: bool, target: &dyn Fn(char, char) -> bool) -> Option<(Pos, Pos)> {
    let ahead = |g: Pos| if fwd { e.char_at(g) } else { e.prev(g).and_then(|q| e.char_at(q)) };
    let step = |g: Pos| if fwd { e.next(g) } else { e.prev(g) };
    // the walk starts on the cursor's far side, the selection's other end
    // is on its near side
    let (mut h, mut anchor) = if fwd { (e.next(cur)?, cur) } else { (cur, e.next(cur).unwrap_or(cur)) };
    ahead(h)?;
    let mut behind = e.char_at(cur).unwrap_or('\n');
    while ahead(h) == Some('\n') {
        behind = '\n';
        h = step(h)?;
    }
    if behind == '\n' {
        anchor = h;
    }
    let start = h;
    while let Some(ch) = ahead(h) {
        if target(behind, ch) {
            if h != start {
                break;
            }
            anchor = h;
        }
        behind = ch;
        h = step(h)?;
    }
    Some(if fwd {
        (anchor, e.prev(h).filter(|q| *q >= anchor).unwrap_or(anchor))
    } else {
        (e.prev(anchor).filter(|q| *q >= h).unwrap_or(h), h)
    })
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
    let n = take_count(&mut e.helix);
    let p = e.cursor;
    match (c, alt) {
        ('h', _) => move_to(e, Pos::new(p.line, p.col.saturating_sub(n))),
        ('l', _) => move_to(e, Pos::new(p.line, p.col.saturating_add(n).min(e.len(p.line)))),
        ('j' | 'k', _) => {
            if e.mode == Mode::Select {
                if e.anchor.is_none() {
                    e.anchor = Some(e.cursor);
                }
            } else {
                e.anchor = None;
            }
            // Helix moves by screen row when lines wrap
            e.move_visual(if c == 'j' { n as isize } else { -(n as isize) });
        }
        ('w' | 'W', false) => {
            // up to the start of the next word, or a line end
            let big = c == 'W';
            select_words(e, n, true, &|a, b| boundary(a, b, big) && (b == '\n' || !b.is_whitespace()));
        }
        ('e' | 'E', false) => {
            let big = c == 'E';
            let mut from = p;
            let mut to = p;
            for _ in 0..n {
                // a count past the end of the text stops there
                if e.word_end(to, big) == to {
                    break;
                }
                // from here if the word goes on, else from the next character
                let at_end = e.word_end(to, big) != to && e.next(to).map(|q| e.word_at(q, big).0 != e.word_at(to, big).0).unwrap_or(true);
                from = if at_end { e.next(to).unwrap_or(to) } else { to };
                to = e.word_end(to, big);
            }
            select_to(e, from.min(to), to);
        }
        ('b' | 'B', false) => {
            // back to the start of a word (`a` is the character after `b`)
            let big = c == 'B';
            select_words(e, n, false, &|a, b| boundary(a, b, big) && (a == '\n' || !a.is_whitespace()));
        }
        ('x', false) => {
            // select the line; again, extend by a line
            let whole = e.anchor.is_some_and(|a| a.col == 0 && a <= e.cursor) && e.cursor.col >= e.len(e.cursor.line);
            let (l1, l2) = if whole {
                (e.anchor.unwrap().line, e.cursor.line.saturating_add(n).min(e.lines() - 1))
            } else {
                (p.line, p.line.saturating_add(n - 1).min(e.lines() - 1))
            };
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
            if c == 'd' && !alt && s.col == 0 && en.col == 0 && en.line > s.line {
                // whole lines are cut, so a nested block's title line put
                // back moves the block (§5.2)
                e.cut_lines(s.line, en.line - 1);
            } else {
                let gone = e.delete(s, en);
                if !alt {
                    let lw = gone.ends_with('\n') && s.col == 0;
                    e.copy(gone, lw);
                }
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
            if clip.linewise {
                // whole lines below the selection's last line or above its
                // first, a nested block cut with its title line moving back
                // with them (§5.2); the lines put in are selected
                let first = if c == 'p' {
                    let l = if en.col == 0 && en.line > s.line { en.line - 1 } else { en.line };
                    e.put_clip_lines(l, true)
                } else {
                    e.put_clip_lines(s.line, false)
                };
                let last = first + clip.text.matches('\n').count().max(1) - 1;
                e.anchor = Some(Pos::new(first, 0));
                e.cursor = Pos::new(last, e.len(last));
                return out;
            }
            let at = if c == 'p' { en } else { s };
            let end = e.insert(at, &clip.text);
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
            // a count of levels (two spaces each) in one change, as Helix does
            let (s, en) = range(e);
            let l2 = if en.col == 0 && en.line > s.line { en.line - 1 } else { en.line };
            let width = n.saturating_mul(2);
            if c == '>' && n > 1 && width.saturating_mul(l2 - s.line + 1) > MAX_TEXT {
                e.message = Some("text too long".into());
                return out;
            }
            let a = e.anchor;
            e.checkpoint();
            for l in s.line..=l2 {
                if c == '>' {
                    if !e.line(l).is_empty() {
                        e.insert(Pos::new(l, 0), &" ".repeat(width));
                    }
                } else {
                    let k = e.line(l).chars().take(width).take_while(|ch| *ch == ' ').count();
                    e.delete(Pos::new(l, 0), Pos::new(l, k));
                }
            }
            e.cursor.col = if c == '>' { e.cursor.col + width } else { e.cursor.col.saturating_sub(width) };
            e.anchor = a;
        }
        ('J', false) => {
            // the selected lines (a line selection ends on the last one's
            // line end), or this one and the next
            let (s, en) = range(e);
            let l2 = if en.col == 0 && en.line > s.line { en.line - 1 } else { en.line };
            let lines = l2.saturating_sub(s.line).max(1);
            e.join(s.line, lines);
            e.anchor = None;
        }
        ('u', false) => {
            for _ in 0..n {
                if !e.undo() {
                    break;
                }
            }
        }
        ('U', false) => {
            for _ in 0..n {
                if !e.redo() {
                    break;
                }
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
    let n = take_count(&mut e.helix);
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
            // Helix's `t`/`T` look past a target next to the cursor
            if let Some(to) = e.find_char(cur, c, p, n, true) {
                select_to(e, cur, to);
            }
        }
        'r' => {
            // each character where it is: line ends stay, and so does every
            // line's tag (§5.2)
            let (s, en) = range(e);
            e.checkpoint();
            e.map_chars(s, en, |_| c);
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
