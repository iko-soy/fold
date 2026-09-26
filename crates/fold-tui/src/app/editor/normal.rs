//! The normal keymap: a conventional editor in the manner of micro. Always
//! typing; Shift extends a selection; Ctrl does the rest.

use super::{ctrl, order, Editor, Group, Outcome, Pos};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle(e: &mut Editor, key: KeyEvent) -> Outcome {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let ctl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut out = Outcome::default();
    // any other key ends a run of typing: what is typed next is its own undo step
    let typing = !ctl && !alt && matches!(key.code, KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete | KeyCode::Tab);
    if !typing {
        e.group = Group::None;
    }

    // movement, extending the selection with Shift
    let moved = match key.code {
        KeyCode::Left if ctl => Some(e.word_back(e.cursor, false)),
        KeyCode::Right if ctl => Some(e.word_fwd(e.cursor, false)),
        KeyCode::Left => {
            if !shift && e.anchor.is_some() {
                Some(e.selection().map(|(s, _)| s).unwrap_or(e.cursor))
            } else {
                Some(e.prev(e.cursor).unwrap_or(e.cursor))
            }
        }
        KeyCode::Right => {
            if !shift && e.anchor.is_some() {
                Some(e.selection().map(|(_, en)| en).unwrap_or(e.cursor))
            } else {
                Some(e.next(e.cursor).unwrap_or(e.cursor))
            }
        }
        KeyCode::Home if ctl => Some(Pos::new(0, 0)),
        KeyCode::End if ctl => {
            let l = e.lines() - 1;
            Some(Pos::new(l, e.len(l)))
        }
        KeyCode::Home => {
            // smart home: first non-blank, then column 0
            let fnb = e.first_non_blank(e.cursor.line);
            Some(Pos::new(e.cursor.line, if e.cursor.col == fnb { 0 } else { fnb }))
        }
        KeyCode::End => Some(Pos::new(e.cursor.line, e.len(e.cursor.line))),
        _ => None,
    };
    if let Some(p) = moved {
        select_to(e, shift, |e| e.set_cursor(p));
        return out;
    }
    let vert = match key.code {
        KeyCode::Up if !alt => Some(-1),
        KeyCode::Down if !alt => Some(1),
        KeyCode::PageUp => Some(-(e.page.max(2) as isize - 1)),
        KeyCode::PageDown => Some(e.page.max(2) as isize - 1),
        _ => None,
    };
    if let Some(d) = vert {
        // by screen row: a wrapped line is several rows
        select_to(e, shift, |e| e.move_visual(d));
        return out;
    }

    match key.code {
        KeyCode::Esc => {
            if e.anchor.take().is_none() {
                out.close = true;
            }
        }
        _ if ctrl(&key, 'q') => out.close = true,
        _ if ctrl(&key, 's') => out.save = true,
        _ if ctrl(&key, 'a') => {
            let l = e.lines() - 1;
            e.anchor = Some(Pos::new(0, 0));
            e.cursor = Pos::new(l, e.len(l));
        }
        _ if ctrl(&key, 'c') => {
            let (t, lw) = selection_or_line(e);
            e.copy(t, lw);
            e.message = Some("copied".into());
        }
        _ if ctrl(&key, 'x') => cut(e),
        _ if ctrl(&key, 'k') => {
            e.anchor = None;
            cut(e);
        }
        _ if ctrl(&key, 'v') => paste(e),
        _ if ctrl(&key, 'z') && shift => {
            e.redo();
        }
        _ if ctrl(&key, 'z') => {
            e.undo();
        }
        _ if ctrl(&key, 'y') => {
            e.redo();
        }
        _ if ctrl(&key, 'd') => {
            // duplicate the line (or the selection)
            e.checkpoint();
            match e.selection() {
                Some((s, en)) => {
                    let t = e.text(s, en);
                    let end = e.insert(en, &t);
                    e.anchor = Some(en);
                    e.cursor = end;
                }
                None => {
                    let l = e.cursor.line;
                    let t = format!("{}\n", e.line(l));
                    e.put_lines(l, &t, true);
                    e.cursor.line += 1;
                }
            }
        }
        _ if ctrl(&key, 'f') => e.open_cmdline('/'),
        _ if ctrl(&key, 'e') => e.open_cmdline(':'),
        _ if ctrl(&key, 'l') || ctrl(&key, 'g') => e.open_cmdline(':'),
        _ if ctrl(&key, 'n') || key.code == KeyCode::F(3) && !shift => {
            e.anchor = None;
            e.search_next(true);
        }
        _ if ctrl(&key, 'p') || key.code == KeyCode::F(3) => {
            e.anchor = None;
            e.search_next(false);
        }
        _ if ctrl(&key, 'w') || (alt && key.code == KeyCode::Backspace) => {
            e.anchor = None;
            e.delete_word_back();
        }
        KeyCode::Up | KeyCode::Down if alt => move_lines(e, key.code == KeyCode::Down),
        KeyCode::Enter => e.newline(),
        KeyCode::Backspace => e.backspace(),
        KeyCode::Delete => e.delete_forward(),
        KeyCode::Tab | KeyCode::BackTab => {
            let multi = e.selection().map(|(s, en)| s.line != en.line).unwrap_or(false);
            if key.code == KeyCode::BackTab || multi {
                let (s, en) = e.selection().unwrap_or((e.cursor, e.cursor));
                let a = e.anchor;
                e.indent(s.line, en.line, if key.code == KeyCode::BackTab { -1 } else { 1 });
                if let Some(a) = a {
                    e.anchor = Some(Pos::new(a.line, 0));
                    let l = e.cursor.line;
                    e.cursor = Pos::new(l, e.len(l));
                }
            } else {
                e.type_char(' ');
                e.type_char(' ');
            }
        }
        KeyCode::Char(c) if !ctl && !alt => e.type_char(c),
        _ => {}
    }
    out
}

/// Move the cursor with `f`, extending the selection when Shift is held and
/// dropping it otherwise.
fn select_to(e: &mut Editor, shift: bool, f: impl FnOnce(&mut Editor)) {
    if shift {
        if e.anchor.is_none() {
            e.anchor = Some(e.cursor);
        }
    } else {
        e.anchor = None;
    }
    f(e);
    if e.anchor == Some(e.cursor) {
        e.anchor = None;
    }
}

/// The selection's text, or the whole current line (linewise).
fn selection_or_line(e: &Editor) -> (String, bool) {
    match e.selection() {
        Some((s, en)) => (e.text(s, en), false),
        None => (format!("{}\n", e.line(e.cursor.line)), true),
    }
}

fn cut(e: &mut Editor) {
    e.checkpoint();
    match e.selection() {
        Some((s, en)) => {
            let t = e.delete(s, en);
            e.copy(t, false);
            e.anchor = None;
            e.set_cursor(s);
        }
        None => {
            let l = e.cursor.line;
            e.cut_lines(l, l);
        }
    }
}

fn paste(e: &mut Editor) {
    if e.clip.text.is_empty() {
        return;
    }
    e.checkpoint();
    if let Some((s, en)) = e.selection() {
        e.delete(s, en);
        e.anchor = None;
        e.set_cursor(s);
    }
    let clip = e.clip.clone();
    if clip.linewise {
        let l = e.cursor.line;
        let first = e.put_clip_lines(l, false);
        e.set_cursor(Pos::new(first + clip.text.matches('\n').count(), e.cursor.col));
    } else {
        let end = e.insert(e.cursor, &clip.text);
        e.set_cursor(end);
    }
}

/// Alt-Up / Alt-Down: move the current line (or the selected lines), each
/// with its tag, so a nested block's title line takes its embed along (§5.2).
fn move_lines(e: &mut Editor, down: bool) {
    let (s, en) = e.selection().map(|(s, en)| order(s, en)).unwrap_or((e.cursor, e.cursor));
    let (l1, l2) = (s.line, en.line);
    if (down && l2 + 1 >= e.lines()) || (!down && l1 == 0) {
        return;
    }
    e.checkpoint();
    e.move_lines(l1, l2, down);
    let d: isize = if down { 1 } else { -1 };
    let shift = |p: Pos| Pos::new((p.line as isize + d) as usize, p.col);
    e.cursor = shift(if e.anchor.is_some() { e.cursor } else { Pos::new(l1, 0) });
    if let Some(a) = e.anchor {
        e.anchor = Some(shift(a));
    } else {
        e.cursor.col = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{body, editor, keys};
    use super::super::{Keys, Pos};

    #[test]
    fn typing_selecting_and_the_clipboard() {
        let (_d, mut e) = editor("hello world\n", Keys::Normal);
        keys(&mut e, "<End> there");
        assert_eq!(body(&e), "hello world there");
        keys(&mut e, "<Home><S-C-Right><C-x>");
        assert_eq!(body(&e), "world there");
        keys(&mut e, "<End><C-v>");
        assert_eq!(body(&e), "world therehello ");
        keys(&mut e, "<C-z><C-z>");
        assert_eq!(body(&e), "hello world there");
        keys(&mut e, "<C-y>");
        assert_eq!(body(&e), "world there");
    }

    #[test]
    fn lines_cut_duplicate_move_and_indent() {
        let (_d, mut e) = editor("a\nb\nc\n", Keys::Normal);
        keys(&mut e, "<C-d>");
        assert_eq!(body(&e), "a\na\nb\nc");
        // the cursor is on the copy; cutting it leaves the cursor on "b"
        keys(&mut e, "<C-k>");
        assert_eq!(body(&e), "a\nb\nc");
        keys(&mut e, "<A-Up>");
        assert_eq!(body(&e), "b\na\nc");
        keys(&mut e, "<Down><S-Down><Tab>");
        assert_eq!(body(&e), "b\n  a\n  c");
        keys(&mut e, "<BTab>");
        assert_eq!(body(&e), "b\na\nc");
    }

    #[test]
    fn enter_keeps_indentation_and_esc_closes() {
        let (_d, mut e) = editor("- item\n  - sub\n", Keys::Normal);
        e.cursor = Pos::new(3, 7);
        keys(&mut e, "<CR>- next");
        assert_eq!(body(&e), "- item\n  - sub\n  - next");
        let out = keys(&mut e, "<Esc>");
        assert!(out[0].close);
        let out = keys(&mut e, "<C-s>");
        assert!(out[0].save);
    }

    #[test]
    fn a_click_ends_a_run_of_typing() {
        let (_d, mut e) = editor("one\ntwo\n", Keys::Normal);
        keys(&mut e, "A");
        e.click(Pos::new(3, 0));
        keys(&mut e, "B<C-z>");
        assert_eq!(body(&e), "Aone\ntwo");
    }

    #[test]
    fn find_selects_the_match() {
        let (_d, mut e) = editor("one two three two\n", Keys::Normal);
        keys(&mut e, "<C-f>two<CR>");
        assert_eq!(e.selection(), Some((Pos::new(2, 4), Pos::new(2, 7))));
        keys(&mut e, "<C-n>");
        assert_eq!(e.cursor, Pos::new(2, 14));
    }
}
