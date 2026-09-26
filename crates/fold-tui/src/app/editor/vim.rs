//! The Vim keymap: normal, insert and visual modes; counts; motions,
//! operators and text objects; `.` repeat; `:` and `/`.

use super::{class, order, Class, Editor, Group, Mode, Outcome, Pos};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Default)]
pub struct State {
    count: String,
    op: Option<char>,
    op_count: usize,
    prefix: Option<char>,
    last_find: Option<(char, char)>,
    /// Keys since the editor was last idle in normal mode, and the change
    /// counter then: when a command changes the text, they become `.`.
    record: Vec<KeyEvent>,
    record_changes: u64,
    last_change: Vec<KeyEvent>,
    replaying: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Excl,
    Incl,
    Line,
}

fn idle(e: &Editor) -> bool {
    e.mode == Mode::Normal && e.vim.op.is_none() && e.vim.prefix.is_none() && e.vim.count.is_empty() && e.cmdline.is_none()
}

pub fn handle(e: &mut Editor, key: KeyEvent) -> Outcome {
    if !e.vim.replaying {
        if e.vim.record.is_empty() {
            e.vim.record_changes = e.changes;
            e.vim.record = select_keys(e);
        }
        e.vim.record.push(key);
    }
    let out = match e.mode {
        Mode::Insert => insert(e, key),
        _ => normal(e, key),
    };
    if e.cmdline.is_some() {
        e.vim.record.clear();
    } else if idle(e) && !e.vim.replaying {
        let rec = std::mem::take(&mut e.vim.record);
        // undo, redo and `.` itself (after any count) are not changes to repeat
        let ctl = |k: &KeyEvent| k.modifiers.contains(KeyModifiers::CONTROL);
        let is_undo = rec.get(count_end(&rec, 0)).is_some_and(|k| match k.code {
            KeyCode::Char('u') | KeyCode::Char('.') => !ctl(k),
            KeyCode::Char('r') => ctl(k),
            _ => false,
        });
        if e.changes != e.vim.record_changes && !is_undo {
            e.vim.last_change = rec;
        }
    }
    out
}

/// A visual selection no recorded key made (a drag or a double-click, or one
/// left by a `:` command): keys that select as much from the cursor, for the
/// record to start with, so `.` acts on as much text as Vim's does.
fn select_keys(e: &Editor) -> Vec<KeyEvent> {
    let (Mode::Visual { line }, Some(a)) = (e.mode, e.anchor) else { return Vec::new() };
    let (s, en) = order(a, e.cursor);
    let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let counted = |n: usize, c: char| -> Vec<KeyEvent> {
        if n == 0 { Vec::new() } else { n.to_string().chars().chain([c]).map(key).collect() }
    };
    let mut keys = vec![key(if line { 'V' } else { 'v' })];
    if en.line > s.line {
        // as many lines down, then (charwise) the same end column
        keys.extend(counted(en.line - s.line, 'j'));
        if !line {
            keys.push(key('0'));
            keys.extend(counted(en.col, 'l'));
        }
    } else if !line {
        keys.extend(counted(en.col - s.col, 'l'));
    }
    keys
}

/// Where a count starting at `i` in recorded keys ends (`i` if there is none).
fn count_end(keys: &[KeyEvent], i: usize) -> usize {
    let mut j = i;
    while keys.get(j).is_some_and(|k| {
        !k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char(c) if c.is_ascii_digit() && (c != '0' || j > i))
    }) {
        j += 1;
    }
    j
}

/// A recorded change with its count replaced by `n`, as Vim's `.` does with
/// a count: the count before the command and the one after an operator both
/// go (`2d3w` becomes `{n}dw`).
fn with_count(keys: &[KeyEvent], n: usize) -> Vec<KeyEvent> {
    let mut out: Vec<KeyEvent> = n.to_string().chars().map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).collect();
    let rest = &keys[count_end(keys, 0)..];
    let op = rest.first().is_some_and(|k| {
        !k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('d' | 'c' | 'y' | '>' | '<'))
    });
    if op {
        out.push(rest[0]);
        out.extend_from_slice(&rest[count_end(rest, 1)..]);
    } else {
        out.extend_from_slice(rest);
    }
    out
}

fn to_insert(e: &mut Editor) {
    e.checkpoint();
    e.group = Group::Typing;
    e.mode = Mode::Insert;
    e.anchor = None;
}

fn insert(e: &mut Editor, key: KeyEvent) -> Outcome {
    let ctl = key.modifiers.contains(KeyModifiers::CONTROL);
    // a key that moves the cursor ends a run of typing: what is typed
    // after it is its own undo step, as in Vim
    if matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End) {
        e.group = Group::None;
    }
    match key.code {
        KeyCode::Esc => leave_insert(e),
        KeyCode::Char('c') | KeyCode::Char('[') if ctl => leave_insert(e),
        KeyCode::Char('w') if ctl => {
            let start = e.word_back(e.cursor, false);
            e.delete(start, e.cursor);
            e.cursor = start;
        }
        KeyCode::Char('u') if ctl => {
            let start = Pos::new(e.cursor.line, e.first_non_blank(e.cursor.line).min(e.cursor.col));
            e.delete(start, e.cursor);
            e.cursor = start;
        }
        KeyCode::Char(c) if !ctl => e.type_char(c),
        KeyCode::Enter => e.newline(),
        KeyCode::Backspace => e.backspace(),
        KeyCode::Delete => e.delete_forward(),
        KeyCode::Tab => {
            e.type_char(' ');
            e.type_char(' ');
        }
        KeyCode::Left => e.set_cursor(Pos::new(e.cursor.line, e.cursor.col.saturating_sub(1))),
        KeyCode::Right => e.set_cursor(Pos::new(e.cursor.line, (e.cursor.col + 1).min(e.len(e.cursor.line)))),
        KeyCode::Up => e.move_vert(-1),
        KeyCode::Down => e.move_vert(1),
        KeyCode::Home => e.set_cursor(Pos::new(e.cursor.line, 0)),
        KeyCode::End => e.set_cursor(Pos::new(e.cursor.line, e.len(e.cursor.line))),
        _ => {}
    }
    Outcome::default()
}

fn leave_insert(e: &mut Editor) {
    e.mode = Mode::Normal;
    e.group = Group::None;
    e.cursor.col = e.cursor.col.saturating_sub(1);
}

/// The largest count, as in Vim: more digits than that are this.
const MAX_COUNT: usize = 999_999_999;

/// The most text a count may make one command put in (Vim's "text too long").
const MAX_TEXT: usize = 4 << 20;

fn take_count(st: &mut State) -> Option<usize> {
    let s = std::mem::take(&mut st.count);
    (!s.is_empty()).then(|| s.parse().map_or(MAX_COUNT, |c: usize| c.min(MAX_COUNT)))
}

/// `f` applied `n` times from `x`, stopping once it no longer moves: a count
/// past the end of the text stops there.
fn repeat<T: PartialEq + Copy>(n: usize, x: T, f: impl Fn(T) -> T) -> T {
    let mut x = x;
    for _ in 0..n {
        let y = f(x);
        if y == x {
            break;
        }
        x = y;
    }
    x
}

fn reset(e: &mut Editor) {
    e.vim.count.clear();
    e.vim.op = None;
    e.vim.prefix = None;
}

/// A motion from the cursor: where it lands and how an operator treats it.
fn motion(e: &mut Editor, c: char, count: Option<usize>) -> Option<(Pos, Kind)> {
    let n = count.unwrap_or(1).max(1);
    let p = e.cursor;
    let last = e.lines() - 1;
    let rep = |e: &Editor, f: &dyn Fn(&Editor, Pos) -> Pos| repeat(n, p, |q| f(e, q));
    Some(match c {
        'h' => (Pos::new(p.line, p.col.saturating_sub(n)), Kind::Excl),
        'l' | ' ' => (Pos::new(p.line, p.col.saturating_add(n).min(e.len(p.line))), Kind::Excl),
        'j' => (Pos::new(p.line.saturating_add(n).min(last), p.col), Kind::Line),
        'k' => (Pos::new(p.line.saturating_sub(n), p.col), Kind::Line),
        'w' => (rep(e, &|e, q| e.word_fwd(q, false)), Kind::Excl),
        'W' => (rep(e, &|e, q| e.word_fwd(q, true)), Kind::Excl),
        'b' => (rep(e, &|e, q| e.word_back(q, false)), Kind::Excl),
        'B' => (rep(e, &|e, q| e.word_back(q, true)), Kind::Excl),
        'e' => (rep(e, &|e, q| e.word_end(q, false)), Kind::Incl),
        'E' => (rep(e, &|e, q| e.word_end(q, true)), Kind::Incl),
        '0' => (Pos::new(p.line, 0), Kind::Excl),
        '^' => (Pos::new(p.line, e.first_non_blank(p.line)), Kind::Excl),
        '$' => {
            let l = p.line.saturating_add(n - 1).min(last);
            (Pos::new(l, e.len(l)), Kind::Excl)
        }
        'G' => {
            let l = count.map(|c| c.saturating_sub(1)).unwrap_or(last).min(last);
            (Pos::new(l, e.first_non_blank(l)), Kind::Line)
        }
        '}' => {
            let l = repeat(n, p.line, |l| e.paragraph(l, true));
            if l == last && e.len(l) > 0 {
                // no blank line below: on the last character, inclusive (Vim's findpar)
                (Pos::new(l, e.len(l) - 1), Kind::Incl)
            } else {
                (Pos::new(l, 0), Kind::Excl)
            }
        }
        '{' => (Pos::new(repeat(n, p.line, |l| e.paragraph(l, false)), 0), Kind::Excl),
        '%' => (e.match_bracket(p)?, Kind::Incl),
        // `n` the way the last search went, `N` the other way
        'n' | 'N' => {
            let pat = e.search.clone()?;
            (e.find(&pat, p, (c == 'n') == e.search_fwd)?, Kind::Excl)
        }
        ';' | ',' => {
            let (kind, ch) = e.vim.last_find?;
            let kind = if c == ',' {
                match kind {
                    'f' => 'F',
                    'F' => 'f',
                    't' => 'T',
                    _ => 't',
                }
            } else {
                kind
            };
            // a repeat moves: it skips a `t`/`T` target next to the cursor,
            // unless given a count (Vim's 'cpoptions' without `;`)
            find(e, kind, ch, n, n == 1)?
        }
        _ => return None,
    })
}

fn find(e: &Editor, kind: char, ch: char, n: usize, skip: bool) -> Option<(Pos, Kind)> {
    let to = e.find_char(e.cursor, ch, kind, n, skip)?;
    Some((to, if kind == 'f' || kind == 't' { Kind::Incl } else { Kind::Excl }))
}

/// `ge`: back to the end of the `n`th word before `p`, as Vim's
/// bckend_word: off the word `p` is in, then back over blanks and line ends,
/// stopping at an empty line.
fn end_back(e: &Editor, p: Pos, n: usize) -> Pos {
    let class_at = |q: Pos| e.char_at(q).map(|ch| class(ch, false));
    let blank = |q: Pos| class_at(q) == Some(Class::Space) && !(q.col == 0 && e.len(q.line) == 0);
    let mut q = p;
    for _ in 0..n.max(1) {
        let k = class_at(q).filter(|k| *k != Class::Space);
        let Some(mut r) = e.prev(q) else { break };
        while k.is_some() && class_at(r) == k {
            let Some(x) = e.prev(r) else { return r };
            r = x;
        }
        while blank(r) {
            let Some(x) = e.prev(r) else { return r };
            r = x;
        }
        q = r;
    }
    q
}

fn normal(e: &mut Editor, key: KeyEvent) -> Outcome {
    let ctl = key.modifiers.contains(KeyModifiers::CONTROL);
    let mut out = Outcome::default();
    let visual = matches!(e.mode, Mode::Visual { .. });
    // keys that are not characters
    let c = match key.code {
        KeyCode::Esc => {
            reset(e);
            if visual {
                e.mode = Mode::Normal;
                e.anchor = None;
            }
            return out;
        }
        KeyCode::Char('c') | KeyCode::Char('[') if ctl => {
            reset(e);
            e.mode = Mode::Normal;
            e.anchor = None;
            return out;
        }
        KeyCode::Char('r') if ctl => {
            reset(e);
            e.redo();
            return out;
        }
        KeyCode::Char('d') | KeyCode::Char('u') if ctl => {
            reset(e);
            let half = (e.page / 2).max(1) as isize;
            e.move_vert(if key.code == KeyCode::Char('d') { half } else { -half });
            return out;
        }
        KeyCode::Char('f') | KeyCode::Char('b') if ctl => {
            reset(e);
            let page = e.page.max(2) as isize - 1;
            e.move_vert(if key.code == KeyCode::Char('f') { page } else { -page });
            return out;
        }
        KeyCode::Char('s') if ctl => {
            out.save = true;
            return out;
        }
        KeyCode::Char(_) if ctl => return out,
        KeyCode::Char(c) => c,
        KeyCode::Left | KeyCode::Backspace => 'h',
        KeyCode::Right => 'l',
        KeyCode::Up => 'k',
        KeyCode::Down => 'j',
        KeyCode::Home => '0',
        KeyCode::End => '$',
        KeyCode::Delete => 'x',
        KeyCode::Enter => {
            reset(e);
            e.move_vert(1);
            e.cursor.col = e.first_non_blank(e.cursor.line);
            return out;
        }
        _ => return out,
    };

    // a pending prefix takes this key as its argument
    if let Some(p) = e.vim.prefix.take() {
        return prefixed(e, p, c);
    }
    // counts
    if c.is_ascii_digit() && (c != '0' || !e.vim.count.is_empty()) {
        e.vim.count.push(c);
        return out;
    }
    let count = take_count(&mut e.vim);

    // an operator waiting for its motion
    if let Some(op) = e.vim.op {
        if c == op || (op == 'c' && c == 'c') {
            // dd, cc, yy, >>, <<: whole lines
            let n = count.unwrap_or(1).saturating_mul(e.vim.op_count.max(1)).min(MAX_COUNT);
            let l2 = e.cursor.line.saturating_add(n - 1).min(e.lines() - 1);
            let from = Pos::new(e.cursor.line, 0);
            e.vim.op = None;
            apply(e, op, from, Pos::new(l2, 0), Kind::Line);
            return out;
        }
        // the count before the operator multiplies the motion's (`2d3w` is `d6w`)
        let n = count.map(|c| c.saturating_mul(e.vim.op_count.max(1)).min(MAX_COUNT)).or(Some(e.vim.op_count).filter(|c| *c > 0));
        e.vim.op_count = 0;
        match c {
            'i' | 'a' | 'f' | 'F' | 't' | 'T' | 'g' => {
                e.vim.prefix = Some(c);
                e.vim.count = n.map(|c| c.to_string()).unwrap_or_default();
                return out;
            }
            _ => {}
        }
        e.vim.op = None;
        // cw on a word is ce; dw stops at the end of the line
        let cw = op == 'c' && (c == 'w' || c == 'W') && e.char_at(e.cursor).is_some_and(|ch| !ch.is_whitespace());
        let c = if cw {
            if c == 'w' { 'e' } else { 'E' }
        } else {
            c
        };
        // but from the last character of a word, that character is the
        // first word (Vim's end_word with stop): `cw` there changes just it
        let m = if cw && e.word_at(e.cursor, c == 'E').1.col == e.cursor.col + 1 {
            match n.unwrap_or(1) {
                1 => Some((e.cursor, Kind::Incl)),
                k => motion(e, c, Some(k - 1)),
            }
        } else {
            motion(e, c, n)
        };
        if let Some((mut to, kind)) = m {
            if (c == 'w' || c == 'W') && to.line > e.cursor.line {
                to = Pos::new(e.cursor.line, e.len(e.cursor.line));
            }
            let from = e.cursor;
            apply(e, op, from, to, kind);
        }
        return out;
    }

    // plain motions move the cursor (and extend a visual selection)
    if !matches!(c, 'x' | 'p' | 'P' | 'J' | 'r' | 'o' | 'O' | 'u' | 'U' | '~' | 'y' | 'd' | 'c' | 's' | 'S' | 'D' | 'C' | 'Y' | 'X' | 'i' | 'a' | 'I' | 'A' | 'v' | 'V' | 'g' | 'f' | 'F' | 't' | 'T' | 'Z' | '>' | '<' | '.' | ':' | '/' | '?' | '*' | '#' | 'q') {
        if c == 'j' || c == 'k' {
            let n = count.unwrap_or(1) as isize;
            e.move_vert(if c == 'j' { n } else { -n });
            return out;
        }
        if let Some((to, _)) = motion(e, c, count) {
            e.set_cursor(to);
            if c == 'G' {
                e.cursor.col = e.first_non_blank(e.cursor.line);
            }
        }
        return out;
    }

    let n = count.unwrap_or(1);
    if visual {
        return visual_cmd(e, c, n);
    }
    match c {
        'i' => to_insert(e),
        'a' => {
            to_insert(e);
            e.cursor.col = (e.cursor.col + 1).min(e.len(e.cursor.line));
        }
        'I' => {
            to_insert(e);
            e.cursor.col = e.first_non_blank(e.cursor.line);
        }
        'A' => {
            to_insert(e);
            e.cursor.col = e.len(e.cursor.line);
        }
        'o' | 'O' => {
            to_insert(e);
            let l = e.cursor.line;
            let indent: String = e.line(l).chars().take_while(|c| *c == ' ').collect();
            if c == 'o' {
                let end = Pos::new(l, e.len(l));
                e.cursor = e.insert(end, &format!("\n{}", indent));
            } else {
                e.insert(Pos::new(l, 0), &format!("{}\n", indent));
                e.cursor = Pos::new(l, indent.chars().count());
            }
        }
        'x' | 'X' => {
            let p = e.cursor;
            let (a, b) = if c == 'x' {
                (p, Pos::new(p.line, p.col.saturating_add(n).min(e.len(p.line))))
            } else {
                (Pos::new(p.line, p.col.saturating_sub(n)), p)
            };
            if a != b {
                e.checkpoint();
                let t = e.delete(a, b);
                e.copy(t, false);
                e.cursor = a;
            }
        }
        's' => {
            let p = e.cursor;
            let b = Pos::new(p.line, p.col.saturating_add(n).min(e.len(p.line)));
            to_insert(e);
            let t = e.delete(p, b);
            e.copy(t, false);
        }
        'S' => {
            let l = e.cursor.line;
            apply(e, 'c', Pos::new(l, 0), Pos::new(l.saturating_add(n - 1).min(e.lines() - 1), 0), Kind::Line);
        }
        'D' | 'C' | 'Y' => {
            let op = match c {
                'D' => 'd',
                'C' => 'c',
                _ => 'y',
            };
            if c == 'Y' {
                let l = e.cursor.line;
                apply(e, 'y', Pos::new(l, 0), Pos::new(l.saturating_add(n - 1).min(e.lines() - 1), 0), Kind::Line);
            } else {
                let l = e.cursor.line.saturating_add(n - 1).min(e.lines() - 1);
                let from = e.cursor;
                apply(e, op, from, Pos::new(l, e.len(l)), Kind::Excl);
            }
        }
        'd' | 'c' | 'y' | '>' | '<' => {
            e.vim.op = Some(c);
            e.vim.op_count = count.unwrap_or(0);
        }
        'p' | 'P' => put(e, c == 'p', n),
        'J' => {
            let l = e.cursor.line;
            e.join(l, n.saturating_sub(1).max(1));
        }
        '~' => {
            let p = e.cursor;
            let b = Pos::new(p.line, p.col.saturating_add(n).min(e.len(p.line)));
            e.checkpoint();
            e.change_case(p, b, '~');
            e.cursor = Pos::new(p.line, b.col);
        }
        'r' | 'g' | 'f' | 'F' | 't' | 'T' | 'Z' => {
            e.vim.prefix = Some(c);
            e.vim.count = count.map(|c| c.to_string()).unwrap_or_default();
        }
        'u' => {
            for _ in 0..n {
                if !e.undo() {
                    break;
                }
            }
        }
        // never inside a replay: `.` would replay itself
        '.' if !e.vim.replaying => {
            // a count replaces the change's own, and stays for the next `.`
            if let Some(n) = count.filter(|_| !e.vim.last_change.is_empty()) {
                e.vim.last_change = with_count(&e.vim.last_change, n);
            }
            let keys = e.vim.last_change.clone();
            e.vim.replaying = true;
            for k in keys {
                handle(e, k);
            }
            e.vim.replaying = false;
            // a replay cut short (say, its selection was not made by keys)
            // leaves no operator waiting for the next key
            if e.mode == Mode::Normal {
                reset(e);
            }
        }
        'v' | 'V' => {
            e.mode = Mode::Visual { line: c == 'V' };
            e.anchor = Some(e.cursor);
        }
        ':' => e.open_cmdline(':'),
        '/' | '?' => e.open_cmdline(c),
        '*' | '#' => {
            let (s, en) = e.word_at(e.cursor, false);
            let w = e.text(s, en);
            if !w.trim().is_empty() {
                e.search = Some(w);
                e.search_fwd = c == '*';
                e.set_cursor(s);
                e.search_next(c == '*');
            }
        }
        _ => {}
    }
    out
}

/// The key after a prefix: `f`/`t`/`F`/`T` + char, `r` + char, `g…`, `Z…`,
/// and `i`/`a` + object after an operator or in visual mode.
fn prefixed(e: &mut Editor, p: char, c: char) -> Outcome {
    let mut out = Outcome::default();
    let count = take_count(&mut e.vim);
    match p {
        'f' | 'F' | 't' | 'T' => {
            e.vim.last_find = Some((p, c));
            if let Some((to, kind)) = find(e, p, c, count.unwrap_or(1), false) {
                if let Some(op) = e.vim.op.take() {
                    let from = e.cursor;
                    apply(e, op, from, to, kind);
                } else {
                    e.set_cursor(to);
                }
            } else {
                e.vim.op = None;
            }
        }
        'r' => {
            if matches!(e.mode, Mode::Visual { .. }) {
                if let Some((s, en)) = e.selection() {
                    // each character where it is: line ends and tags stay (§5.2)
                    e.checkpoint();
                    e.map_chars(s, en, |_| c);
                    e.cursor = s;
                    e.anchor = None;
                    e.mode = Mode::Normal;
                }
            } else {
                let n = count.unwrap_or(1);
                let pos = e.cursor;
                if pos.col.checked_add(n).is_some_and(|end| end <= e.len(pos.line)) {
                    e.checkpoint();
                    e.delete(pos, Pos::new(pos.line, pos.col + n));
                    e.insert(pos, &c.to_string().repeat(n));
                    e.cursor = Pos::new(pos.line, pos.col + n - 1);
                }
            }
        }
        'g' => match c {
            'g' => {
                let l = count.map(|c| c.saturating_sub(1)).unwrap_or(0).min(e.lines() - 1);
                let to = Pos::new(l, e.first_non_blank(l));
                if let Some(op) = e.vim.op.take() {
                    let from = e.cursor;
                    apply(e, op, from, to, Kind::Line);
                } else {
                    e.set_cursor(to);
                }
            }
            // ge: back to the end of the previous word, inclusive
            'e' => {
                let to = end_back(e, e.cursor, count.unwrap_or(1));
                if let Some(op) = e.vim.op.take() {
                    let from = e.cursor;
                    apply(e, op, from, to, Kind::Incl);
                } else {
                    e.set_cursor(to);
                }
            }
            // gj / gk: by screen row through wrapped lines; with an operator
            // they are charwise and exclusive, not linewise like j / k
            'j' | 'k' => {
                let n = count.unwrap_or(1) as isize;
                let from = e.cursor;
                e.move_visual(if c == 'j' { n } else { -n });
                if let Some(op) = e.vim.op.take() {
                    let to = e.cursor;
                    e.cursor = from;
                    apply(e, op, from, to, Kind::Excl);
                }
            }
            _ => e.vim.op = None,
        },
        'Z' => match c {
            'Z' => out.close = true,
            'Q' => out.revert = true,
            _ => {}
        },
        'i' | 'a' => {
            if let Some((s, en)) = e.text_object(p == 'a', c) {
                let linewise = c == 'p';
                if let Some(op) = e.vim.op.take() {
                    if linewise {
                        let l2 = if en.col == 0 && en.line > s.line { en.line - 1 } else { en.line };
                        apply(e, op, s, Pos::new(l2, 0), Kind::Line);
                    } else {
                        apply(e, op, s, en, Kind::Excl);
                    }
                } else if matches!(e.mode, Mode::Visual { .. }) {
                    e.anchor = Some(s);
                    e.cursor = e.prev(en).filter(|q| *q >= s).unwrap_or(s);
                }
            } else {
                e.vim.op = None;
            }
        }
        _ => {}
    }
    out
}

/// Apply an operator to the text between `from` and `to`.
fn apply(e: &mut Editor, op: char, from: Pos, to: Pos, kind: Kind) {
    let (s, en) = order(from, to);
    if kind == Kind::Line {
        let (l1, l2) = (s.line, en.line);
        match op {
            'd' => {
                // cut, so a nested block's title line put back moves the block (§5.2)
                e.checkpoint();
                e.cut_lines(l1, l2);
                e.cursor = Pos::new(e.cursor.line, e.first_non_blank(e.cursor.line));
            }
            'y' => {
                let mut t = String::new();
                for l in l1..=l2 {
                    t.push_str(e.line(l));
                    t.push('\n');
                }
                e.copy(t, true);
                e.cursor = Pos::new(l1, e.cursor.col.min(e.len(l1)));
            }
            'c' => {
                to_insert(e);
                let indent: String = e.line(l1).chars().take_while(|c| *c == ' ').collect();
                let mut t = String::new();
                for l in l1..=l2 {
                    t.push_str(e.line(l));
                    t.push('\n');
                }
                e.copy(t, true);
                let end = Pos::new(l2, e.len(l2));
                e.delete(Pos::new(l1, 0), end);
                e.insert(Pos::new(l1, 0), &indent);
                e.cursor = Pos::new(l1, indent.chars().count());
            }
            '>' | '<' => {
                e.indent(l1, l2, if op == '>' { 1 } else { -1 });
                e.cursor = Pos::new(l1, e.first_non_blank(l1));
            }
            _ => {}
        }
        return;
    }
    let en = if kind == Kind::Incl { e.next(en).unwrap_or(Pos::new(en.line, e.len(en.line))) } else { en };
    match op {
        'd' => {
            e.checkpoint();
            let t = e.delete(s, en);
            e.copy(t, false);
            e.cursor = s;
        }
        'y' => {
            let t = e.text(s, en);
            e.copy(t, false);
            e.cursor = s;
        }
        'c' => {
            to_insert(e);
            let t = e.delete(s, en);
            e.copy(t, false);
            e.cursor = s;
        }
        '>' | '<' => {
            e.indent(s.line, en.line, if op == '>' { 1 } else { -1 });
        }
        _ => {}
    }
}

/// `p` / `P`: whole lines go below / above the line, text after / at the
/// cursor.
fn put(e: &mut Editor, after: bool, n: usize) {
    let clip = e.clip.clone();
    if clip.text.is_empty() {
        return;
    }
    if n > 1 && clip.text.len().saturating_mul(n) > MAX_TEXT {
        e.message = Some("text too long".into());
        return;
    }
    e.checkpoint();
    let text = clip.text.repeat(n.max(1));
    if clip.linewise {
        // one put of cut lines moves a nested block they hold (§5.2);
        // more are copies
        let l = e.cursor.line;
        let first = if n > 1 { e.put_lines(l, &text, after) } else { e.put_clip_lines(l, after) };
        e.cursor = Pos::new(first, e.first_non_blank(first));
    } else {
        let at = if after { Pos::new(e.cursor.line, (e.cursor.col + 1).min(e.len(e.cursor.line))) } else { e.cursor };
        let end = e.insert(at, &text);
        e.cursor = e.prev(end).unwrap_or(end);
    }
}

fn visual_cmd(e: &mut Editor, c: char, n: usize) -> Outcome {
    let line = matches!(e.mode, Mode::Visual { line: true });
    let Some(a) = e.anchor else { return Outcome::default() };
    let (s, en) = order(a, e.cursor);
    let kind = if line { Kind::Line } else { Kind::Incl };
    let exit = |e: &mut Editor| {
        if e.mode != Mode::Insert {
            e.mode = Mode::Normal;
        }
        e.anchor = None;
    };
    match c {
        'd' | 'x' | 'X' | 'D' => {
            apply(e, 'd', s, en, if c == 'X' || c == 'D' { Kind::Line } else { kind });
            exit(e);
        }
        'y' | 'Y' => {
            apply(e, 'y', s, en, if c == 'Y' { Kind::Line } else { kind });
            exit(e);
        }
        'c' | 's' | 'S' | 'C' => {
            apply(e, 'c', s, en, if c == 'S' || c == 'C' { Kind::Line } else { kind });
            exit(e);
        }
        '>' | '<' => {
            e.indent(s.line, en.line, if c == '>' { n as i32 } else { -1 });
            exit(e);
        }
        '~' | 'u' | 'U' => {
            let (a, b) = e.selection().unwrap_or((s, en));
            e.checkpoint();
            e.change_case(a, b, c);
            e.cursor = s;
            exit(e);
        }
        'J' => {
            e.join(s.line, (en.line - s.line).max(1));
            exit(e);
        }
        'p' | 'P' => {
            let clip = e.clip.clone();
            let (a, b) = e.selection().unwrap_or((s, en));
            e.checkpoint();
            // in V-LINE the lines' text goes and one line is left for the new
            // text, so whole lines go in without their last newline, and the
            // lines replaced are kept whole
            let mut gone = e.delete(a, b);
            if line {
                gone.push('\n');
            }
            e.cursor = a;
            let end = e.insert(a, clip.text.strip_suffix('\n').filter(|_| clip.linewise).unwrap_or(&clip.text));
            e.cursor = if line { Pos::new(a.line, e.first_non_blank(a.line)) } else { e.prev(end).unwrap_or(end) };
            e.copy(gone, line);
            exit(e);
        }
        'o' | 'O' => {
            let cur = e.cursor;
            e.cursor = a;
            e.anchor = Some(cur);
        }
        'v' | 'V' => {
            let want = Mode::Visual { line: c == 'V' };
            if e.mode == want {
                exit(e);
            } else {
                e.mode = want;
            }
        }
        'i' | 'a' | 'r' | 'g' | 'f' | 'F' | 't' | 'T' => e.vim.prefix = Some(c),
        ':' => e.open_cmdline(':'),
        _ => {}
    }
    Outcome::default()
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{body, editor, keys};
    use super::super::{Keys, Mode, Pos};

    #[test]
    fn insert_and_escape() {
        let (_d, mut e) = editor("hello\n", Keys::Vim);
        keys(&mut e, "A world<Esc>");
        assert_eq!(body(&e), "hello world");
        assert_eq!(e.mode, Mode::Normal);
        assert_eq!(e.cursor, Pos::new(2, 10));
        keys(&mut e, "0iSay: <Esc>");
        assert_eq!(body(&e), "Say: hello world");
        keys(&mut e, "u");
        assert_eq!(body(&e), "hello world", "an insert is one undo step");
    }

    #[test]
    fn operators_with_motions_counts_and_text_objects() {
        let (_d, mut e) = editor("one two three four\n", Keys::Vim);
        keys(&mut e, "dw");
        assert_eq!(body(&e), "two three four");
        keys(&mut e, "2dw");
        assert_eq!(body(&e), "four");
        keys(&mut e, "u");
        assert_eq!(body(&e), "two three four");
        keys(&mut e, "wcwTHREE<Esc>");
        assert_eq!(body(&e), "two THREE four");
        keys(&mut e, "ciwtri<Esc>");
        assert_eq!(body(&e), "two tri four");
        keys(&mut e, "0d$");
        assert_eq!(body(&e), "");
    }

    #[test]
    fn lines_yank_put_and_dot() {
        let (_d, mut e) = editor("a\nb\nc\n", Keys::Vim);
        keys(&mut e, "yyp");
        assert_eq!(body(&e), "a\na\nb\nc");
        keys(&mut e, "jdd");
        assert_eq!(body(&e), "a\na\nc");
        keys(&mut e, ".");
        assert_eq!(body(&e), "a\na");
        keys(&mut e, "3GA!<Esc>j.");
        assert_eq!(body(&e), "a!\na!");
        keys(&mut e, ">>");
        assert_eq!(body(&e), "a!\n  a!");
        keys(&mut e, "kJ");
        assert_eq!(body(&e), "a! a!");
    }

    #[test]
    fn find_quotes_brackets_and_search() {
        let (_d, mut e) = editor("call(\"a b\", [x])\n", Keys::Vim);
        keys(&mut e, "fbx");
        assert_eq!(body(&e), "call(\"a \", [x])");
        keys(&mut e, "di\"");
        assert_eq!(body(&e), "call(\"\", [x])");
        keys(&mut e, "fxda[");
        assert_eq!(body(&e), "call(\"\", )");
        keys(&mut e, "0f(ci(z<Esc>");
        assert_eq!(body(&e), "call(z)");
        keys(&mut e, "0/z<CR>");
        assert_eq!(e.cursor, Pos::new(2, 5));
    }

    #[test]
    fn visual_modes() {
        let (_d, mut e) = editor("abc def\nghi\njkl\n", Keys::Vim);
        keys(&mut e, "vey");
        assert_eq!(e.clip.text, "abc");
        keys(&mut e, "Vjd");
        assert_eq!(body(&e), "jkl");
        keys(&mut e, "P");
        assert_eq!(body(&e), "abc def\nghi\njkl");
        keys(&mut e, "wv$~");
        assert_eq!(body(&e), "abc DEF\nghi\njkl");
    }

    #[test]
    fn ex_commands() {
        let (_d, mut e) = editor("x\n", Keys::Vim);
        let out = keys(&mut e, ":w<CR>");
        assert!(out.last().unwrap().save);
        let out = keys(&mut e, ":q!<CR>");
        assert!(out.last().unwrap().revert);
        let out = keys(&mut e, "ZZ");
        assert!(out.last().unwrap().close);
    }
}
