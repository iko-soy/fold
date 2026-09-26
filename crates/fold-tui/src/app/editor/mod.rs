//! The built-in editor (§10.6): one text-editing core over the tagged
//! buffer, and three keymaps on top of it — a conventional one in the manner
//! of micro, Vim, and Helix.
//!
//! Every change goes through `insert` and `delete`, which use the buffer's
//! own line operations, so each line keeps (or inherits) its owning block and
//! the blocks it touches become dirty (§5.2); whole lines moved, or cut and
//! put back, carry their tags (`move_lines`, `cut_lines`, `put_clip_lines`).

mod helix;
mod normal;
mod vim;

use super::wrap::{self, Row};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fold_core::edit::{EditBuffer, EditLine, Owner};
use std::collections::{BTreeMap, BTreeSet};

/// A position: line, and column in characters.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Pos {
        Pos { line, col }
    }
}

/// Which keymap the editor speaks.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Keys {
    #[default]
    Normal,
    Vim,
    Helix,
}

impl Keys {
    pub fn parse(s: &str) -> Option<Keys> {
        match s.trim().to_ascii_lowercase().as_str() {
            "normal" | "default" | "micro" | "standard" => Some(Keys::Normal),
            "vim" | "vi" | "nvim" | "neovim" => Some(Keys::Vim),
            "helix" | "hx" | "kakoune" | "kak" => Some(Keys::Helix),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Keys::Normal => "normal",
            Keys::Vim => "vim",
            Keys::Helix => "helix",
        }
    }

    pub fn next(self) -> Keys {
        match self {
            Keys::Normal => Keys::Vim,
            Keys::Vim => Keys::Helix,
            Keys::Helix => Keys::Normal,
        }
    }
}

/// The editor's own mode (Vim's and Helix's); the normal keymap is always
/// `Insert`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Insert,
    Normal,
    /// Vim visual, charwise or linewise.
    Visual { line: bool },
    /// Helix select mode: motions extend the selection.
    Select,
}

/// What the app has to do after a key.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub save: bool,
    pub close: bool,
    pub revert: bool,
}

/// The clipboard, and whether it holds whole lines.
#[derive(Clone, Default, Debug)]
pub struct Clip {
    pub text: String,
    pub linewise: bool,
    /// Whole lines cut in this editor: each line's block, where that block's
    /// title line was cut with it (§5.2: cut and paste moves its embed).
    pub tags: Vec<Option<Owner>>,
}

/// An input line at the bottom of the editor: `:` commands or `/` search.
#[derive(Clone, Debug)]
pub struct CmdLine {
    pub kind: char,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    None,
    Typing,
}

struct Snap {
    lines: Vec<EditLine>,
    cursor: Pos,
    /// The block each block's embed sits in (moving lines can change it).
    parents: BTreeMap<Owner, Option<Owner>>,
}

pub struct Editor {
    pub buf: EditBuffer,
    pub cursor: Pos,
    /// The other end of the selection, if there is one.
    pub anchor: Option<Pos>,
    pub keys: Keys,
    pub mode: Mode,
    pub clip: Clip,
    pub cmdline: Option<CmdLine>,
    pub search: Option<String>,
    /// Whether the last search went forward (`/`, Vim's `*`) or back (`?`,
    /// `#`): Vim's `n` goes on the same way.
    search_fwd: bool,
    /// A one-line message for the editor's status (a failed search, …).
    pub message: Option<String>,
    /// Lines on screen, for paging (set by the renderer).
    pub page: usize,
    /// Text copied since the last frame, for the system clipboard (OSC 52).
    pub copied: Option<String>,
    /// Where a mouse drag started.
    drag_origin: Option<Pos>,
    /// Columns to wrap at (set by the renderer); `None` means no wrapping.
    pub wrap_cols: Option<usize>,
    /// Screen rows of every line, for the text and width they were made for.
    layout: Option<(u64, Option<usize>, Vec<Vec<Row>>, Vec<bool>)>,
    /// The screen column vertical moves by screen row aim for.
    want_x: Option<usize>,
    want_col: Option<usize>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    group: Group,
    /// Bumped by every change; keymaps compare it to see whether a command
    /// changed the text.
    pub changes: u64,
    vim: vim::State,
    helix: helix::State,
}

impl Editor {
    pub fn new(buf: EditBuffer, keys: Keys, mut clip: Clip) -> Editor {
        // tags name blocks of the buffer they were cut from
        clip.tags.clear();
        let mode = match keys {
            Keys::Normal => Mode::Insert,
            Keys::Vim | Keys::Helix => Mode::Normal,
        };
        Editor {
            buf,
            cursor: Pos::default(),
            anchor: None,
            keys,
            mode,
            clip,
            cmdline: None,
            search: None,
            search_fwd: true,
            message: None,
            page: 20,
            copied: None,
            drag_origin: None,
            wrap_cols: None,
            layout: None,
            want_x: None,
            want_col: None,
            undo: Vec::new(),
            redo: Vec::new(),
            group: Group::None,
            changes: 0,
            vim: vim::State::default(),
            helix: helix::State::default(),
        }
    }

    /// Switch keymaps mid-edit: the text, cursor and history stay.
    pub fn set_keys(&mut self, keys: Keys) {
        self.keys = keys;
        self.anchor = None;
        self.cmdline = None;
        self.vim = vim::State::default();
        self.helix = helix::State::default();
        self.mode = match keys {
            Keys::Normal => Mode::Insert,
            _ => Mode::Normal,
        };
        self.clamp_cursor();
    }

    /// Handle a key in the current keymap.
    pub fn handle(&mut self, key: KeyEvent) -> Outcome {
        self.message = None;
        if self.cmdline.is_some() {
            return self.cmdline_key(key);
        }
        let out = match self.keys {
            Keys::Normal => normal::handle(self, key),
            Keys::Vim => vim::handle(self, key),
            Keys::Helix => helix::handle(self, key),
        };
        self.clamp_cursor();
        out
    }

    /// Text from the terminal's paste (bracketed paste): typed in as is.
    pub fn paste_text(&mut self, text: &str) {
        if let Some(cl) = self.cmdline.as_mut() {
            // an open `:` or `/` line is where typing goes: its first line
            cl.text.push_str(text.lines().next().unwrap_or(""));
            return;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.forget_goal();
        if self.mode != Mode::Insert && self.keys != Keys::Normal {
            // pasting in a normal mode puts the text after the cursor
            self.checkpoint();
            let at = self.after_cursor();
            self.cursor = self.insert(at, &text);
            return;
        }
        self.checkpoint();
        self.delete_selection();
        self.cursor = self.insert(self.cursor, &text);
        self.group = Group::None;
    }

    // -------------------------------------------------------------- pointer

    /// A click: the cursor goes there and any selection is dropped. Typing
    /// after it is a new undo step.
    pub fn click(&mut self, p: Pos) {
        self.anchor = None;
        self.group = Group::None;
        if matches!(self.mode, Mode::Visual { .. } | Mode::Select) {
            self.mode = Mode::Normal;
        }
        self.set_cursor(self.clamp_pos(p));
        self.clamp_cursor();
        self.drag_origin = Some(self.cursor);
    }

    /// Dragging selects from where the button went down (a Vim drag is a
    /// visual selection).
    pub fn drag_to(&mut self, p: Pos) {
        let Some(o) = self.drag_origin else { return };
        let p = self.clamp_pos(p);
        if p == o && self.anchor.is_none() {
            return;
        }
        self.anchor = Some(o);
        if self.keys == Keys::Vim && self.mode == Mode::Normal {
            self.mode = Mode::Visual { line: false };
        }
        self.set_cursor(p);
        self.clamp_cursor();
    }

    pub fn end_drag(&mut self) {
        self.drag_origin = None;
    }

    /// A double-click selects the word under the pointer.
    pub fn select_word(&mut self, p: Pos) {
        let (s, e) = self.word_at(self.clamp_pos(p), false);
        if s == e {
            return;
        }
        self.group = Group::None;
        self.anchor = Some(s);
        self.cursor = if self.keys == Keys::Normal { e } else { self.prev(e).unwrap_or(e) };
        if self.keys == Keys::Vim && self.mode == Mode::Normal {
            self.mode = Mode::Visual { line: false };
        }
    }

    fn clamp_pos(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines() - 1);
        Pos::new(line, p.col.min(self.len(line)))
    }

    /// The selected range as `[start, end)` in document order, for drawing.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        let inclusive = match self.keys {
            Keys::Normal => false,
            _ => true,
        };
        if let Mode::Visual { line: true } = self.mode {
            let (s, e) = order(a, self.cursor);
            return Some((Pos::new(s.line, 0), Pos::new(e.line, self.len(e.line))));
        }
        let (s, e) = order(a, self.cursor);
        Some((s, if inclusive { self.next(e).unwrap_or(Pos::new(e.line, self.len(e.line))) } else { e }))
    }

    /// A block cursor (Vim and Helix outside insert mode) or a bar.
    pub fn block_cursor(&self) -> bool {
        self.keys != Keys::Normal && self.mode != Mode::Insert
    }

    /// A short mode name for the border.
    pub fn mode_name(&self) -> &'static str {
        match (self.keys, self.mode) {
            (Keys::Normal, _) => "",
            (_, Mode::Insert) => "INSERT",
            (_, Mode::Normal) => "NORMAL",
            (_, Mode::Visual { line: false }) => "VISUAL",
            (_, Mode::Visual { line: true }) => "V-LINE",
            (_, Mode::Select) => "SELECT",
        }
    }

    // -------------------------------------------------------------- text

    pub fn lines(&self) -> usize {
        self.buf.lines.len().max(1)
    }

    pub fn line(&self, l: usize) -> &str {
        self.buf.lines.get(l).map(|x| x.text.as_str()).unwrap_or("")
    }

    pub fn len(&self, l: usize) -> usize {
        self.line(l).chars().count()
    }

    /// The character at a position; a line end reads as `\n` (except the last).
    pub fn char_at(&self, p: Pos) -> Option<char> {
        let line = self.line(p.line);
        match line.chars().nth(p.col) {
            Some(c) => Some(c),
            None if p.line + 1 < self.lines() => Some('\n'),
            None => None,
        }
    }

    pub fn next(&self, p: Pos) -> Option<Pos> {
        if p.col < self.len(p.line) {
            Some(Pos::new(p.line, p.col + 1))
        } else if p.line + 1 < self.lines() {
            Some(Pos::new(p.line + 1, 0))
        } else {
            None
        }
    }

    pub fn prev(&self, p: Pos) -> Option<Pos> {
        if p.col > 0 {
            Some(Pos::new(p.line, p.col - 1))
        } else if p.line > 0 {
            Some(Pos::new(p.line - 1, self.len(p.line - 1)))
        } else {
            None
        }
    }

    /// The text of `[a, b)`.
    pub fn text(&self, a: Pos, b: Pos) -> String {
        let (a, b) = order(a, b);
        if a.line == b.line {
            let l = self.line(a.line);
            return l.chars().skip(a.col).take(b.col.saturating_sub(a.col)).collect();
        }
        let mut s: String = self.line(a.line).chars().skip(a.col).collect();
        for l in a.line + 1..b.line {
            s.push('\n');
            s.push_str(self.line(l));
        }
        s.push('\n');
        s.extend(self.line(b.line).chars().take(b.col));
        s
    }

    /// Insert text at a position; returns the position after it. New lines
    /// belong to the block of the line they follow (§5.2): the line they
    /// split, or at column 0 the line above, so a line opened or pasted above
    /// a nested block's title line is not written into that block. Text
    /// starting with a line break at an empty line goes in below it (`o`,
    /// `p`, Enter, Ctrl-D there), so those lines are that line's block's.
    pub fn insert(&mut self, at: Pos, s: &str) -> Pos {
        if s.is_empty() {
            return at;
        }
        if self.buf.lines.is_empty() {
            self.buf.lines.push(EditLine { text: String::new(), owner: Owner { file: 0, block_ord: 0 } });
        }
        self.changes += 1;
        let line = self.line(at.line).to_string();
        let b = byte(&line, at.col);
        let (pre, post) = line.split_at(b);
        let parts: Vec<&str> = s.split('\n').collect();
        if parts.len() == 1 {
            self.buf.set_line(at.line, format!("{}{}{}", pre, s, post));
            return Pos::new(at.line, at.col + s.chars().count());
        }
        if at.col == 0 && at.line > 0 && !(parts[0].is_empty() && post.is_empty()) {
            // whole lines go in after the line above, taking its tag; the
            // line at `at` keeps its own and only gains the last part
            let mut l = at.line - 1;
            for p in &parts[..parts.len() - 1] {
                self.buf.insert_line(l, p.to_string());
                l += 1;
            }
            let last = parts[parts.len() - 1];
            self.buf.set_line(l + 1, format!("{}{}", last, post));
            return Pos::new(l + 1, last.chars().count());
        }
        self.buf.set_line(at.line, format!("{}{}", pre, parts[0]));
        let mut l = at.line;
        for p in &parts[1..parts.len() - 1] {
            self.buf.insert_line(l, p.to_string());
            l += 1;
        }
        let last = parts[parts.len() - 1];
        self.buf.insert_line(l, format!("{}{}", last, post));
        Pos::new(l + 1, last.chars().count())
    }

    /// Delete `[a, b)`; returns what was deleted. A line deleted whole takes
    /// its tag with it (§5.2): what is left of a joined line keeps the tag of
    /// line `a`, unless the range starts at column 0, when line `b` is what
    /// is left (unless nothing of it is, at the end of the text).
    pub fn delete(&mut self, a: Pos, b: Pos) -> String {
        let (a, mut b) = order(a, b);
        if b.line >= self.lines() {
            b = Pos::new(self.lines() - 1, self.len(self.lines() - 1));
        }
        let gone = self.text(a, b);
        if gone.is_empty() {
            return gone;
        }
        self.changes += 1;
        let first = self.line(a.line).to_string();
        let last = self.line(b.line).to_string();
        let rest = &last[byte(&last, b.col)..];
        if a.col == 0 && a.line < b.line && (!rest.is_empty() || b.line + 1 < self.lines()) {
            for l in (a.line..b.line).rev() {
                self.buf.delete_line(l);
            }
            self.buf.set_line(a.line, rest.to_string());
            return gone;
        }
        let joined = format!("{}{}", &first[..byte(&first, a.col)], rest);
        for l in (a.line + 1..=b.line).rev() {
            self.buf.delete_line(l);
        }
        self.buf.set_line(a.line, joined);
        gone
    }

    /// Delete whole lines `l1..=l2`; returns them, each ending in `\n`.
    pub fn delete_lines(&mut self, l1: usize, l2: usize) -> String {
        let l2 = l2.min(self.lines() - 1);
        let mut gone = String::new();
        for l in l1..=l2 {
            gone.push_str(self.line(l));
            gone.push('\n');
        }
        self.changes += 1;
        if l1 == 0 && l2 + 1 >= self.lines() {
            // the buffer keeps one (empty) line
            for l in (1..self.lines()).rev() {
                self.buf.delete_line(l);
            }
            self.buf.set_line(0, String::new());
        } else {
            for l in (l1..=l2).rev() {
                self.buf.delete_line(l);
            }
        }
        self.cursor = Pos::new(l1.min(self.lines() - 1), 0);
        gone
    }

    /// Put whole lines (text ending in `\n`) below or above line `l`;
    /// returns the first new line.
    pub fn put_lines(&mut self, l: usize, text: &str, below: bool) -> usize {
        let body = text.strip_suffix('\n').unwrap_or(text);
        if below {
            let end = Pos::new(l, self.len(l));
            self.insert(end, &format!("\n{}", body));
            l + 1
        } else {
            self.insert(Pos::new(l, 0), &format!("{}\n", body));
            l
        }
    }

    fn delete_selection(&mut self) -> Option<String> {
        let (s, e) = self.selection()?;
        self.anchor = None;
        let gone = self.delete(s, e);
        self.cursor = s;
        Some(gone)
    }

    // -------------------------------------------------------------- moving lines

    /// Move whole lines `l1..=l2` one line down or up, past their neighbour.
    /// Each line keeps its tag (§5.2), so moving a nested block's title line
    /// moves its embed.
    pub fn move_lines(&mut self, l1: usize, l2: usize, down: bool) {
        let (lo, hi) = if down { (l1, l2 + 1) } else { (l1.wrapping_sub(1), l2) };
        if (!down && l1 == 0) || hi >= self.buf.lines.len() {
            return;
        }
        let titles = self.titles();
        if down {
            self.buf.lines[lo..=hi].rotate_right(1);
        } else {
            self.buf.lines[lo..=hi].rotate_left(1);
        }
        self.changes += 1;
        for i in lo..=hi {
            let o = self.buf.lines[i].owner;
            self.buf.mark_dirty(o);
        }
        // where the line at `i` went: the neighbour to the far end, the rest by one
        let to = |i: usize| {
            if i < lo || i > hi {
                i
            } else if down {
                if i == hi { lo } else { i + 1 }
            } else if i == lo {
                hi
            } else {
                i - 1
            }
        };
        let mut moved = BTreeMap::new();
        for (o, t) in titles {
            if (lo..=hi).contains(&t) {
                self.touch(o);
            }
            moved.insert(o, Some(to(t)));
        }
        self.settle(&moved);
    }

    /// Cut whole lines `l1..=l2` to the clipboard. A nested block whose title
    /// line goes keeps its tag there, and its lines left behind go to the
    /// block they now sit in, so pasting the lines moves the block (§5.2).
    pub fn cut_lines(&mut self, l1: usize, l2: usize) {
        let l2 = l2.min(self.lines() - 1);
        let titles = self.titles();
        let nested = |e: &Editor, o: Owner| e.buf.owners.get(&o).is_some_and(|i| i.parent.is_some());
        let tags = (l1..=l2)
            .map(|l| {
                let o = self.buf.lines.get(l)?.owner;
                (nested(self, o) && titles.get(&o).is_some_and(|&t| t >= l1)).then_some(o)
            })
            .collect();
        let text = self.delete_lines(l1, l2);
        let n = l2 + 1 - l1;
        let left = titles
            .into_iter()
            .map(|(o, t)| (o, if t < l1 { Some(t) } else if t > l2 { Some(t - n) } else { None }))
            .collect();
        self.settle(&left);
        self.set_clip(text, true, tags);
    }

    /// Put the clipboard's whole lines above or below line `l`; returns the
    /// first new line. They take the tag of the line above (§5.2), except the
    /// lines of a nested block cut with its title line and no longer in the
    /// buffer: those go back with their own tag, so the block moves. A tag
    /// the buffer does not know (it was re-rendered since the cut, §11.2)
    /// names no block to write the line to, and is not put back.
    pub fn put_clip_lines(&mut self, l: usize, below: bool) -> usize {
        let clip = self.clip.clone();
        let titles = self.titles();
        let first = self.put_lines(l, &clip.text, below);
        let n = clip.tags.len();
        if n == 0 || clip.text.matches('\n').count() != n || first + n > self.buf.lines.len() {
            return first;
        }
        let present: BTreeSet<Owner> =
            self.buf.lines.iter().enumerate().filter(|(i, _)| *i < first || *i >= first + n).map(|(_, x)| x.owner).collect();
        let mut back: BTreeMap<Owner, Option<usize>> = BTreeMap::new();
        for (i, tag) in clip.tags.iter().enumerate() {
            let Some(o) = tag.filter(|o| !present.contains(o) && self.buf.owners.contains_key(o)) else { continue };
            let was = std::mem::replace(&mut self.buf.lines[first + i].owner, o);
            self.buf.mark_dirty(was);
            self.touch(o);
            back.entry(o).or_insert(Some(first + i));
        }
        if !back.is_empty() {
            for (o, t) in titles {
                back.insert(o, Some(if t >= first { t + n } else { t }));
            }
            self.settle(&back);
        }
        first
    }

    /// The first line of each block: its title line (§5.2).
    fn titles(&self) -> BTreeMap<Owner, usize> {
        let mut t = BTreeMap::new();
        for (i, l) in self.buf.lines.iter().enumerate() {
            t.entry(l.owner).or_insert(i);
        }
        t
    }

    /// Whether block `o` is `outer` or nested in it.
    fn within(&self, mut o: Owner, outer: Owner) -> bool {
        while o != outer {
            match self.buf.owners.get(&o).and_then(|i| i.parent) {
                Some(p) => o = p,
                None => return false,
            }
        }
        true
    }

    /// A block changed and moved: it is dirty, and so is the block its embed
    /// sits in (§5.2).
    fn touch(&mut self, o: Owner) {
        self.buf.mark_dirty(o);
        if let Some(p) = self.buf.owners.get(&o).and_then(|i| i.parent) {
            self.buf.mark_dirty(p);
        }
    }

    /// After lines moved with their tags, each goes with the block it now
    /// sits in (§5.2): `retag_strays`, then `reparent`.
    fn settle(&mut self, titles: &BTreeMap<Owner, Option<usize>>) {
        self.retag_strays(titles);
        self.reparent();
    }

    /// A line of a nested block that is no longer contiguous with the
    /// block's title line (at `titles`, or gone) is re-tagged to the block it
    /// now sits in, that of the line above (§5.2).
    fn retag_strays(&mut self, titles: &BTreeMap<Owner, Option<usize>>) {
        let n = self.buf.lines.len();
        for (&o, &title) in titles {
            if self.buf.owners.get(&o).and_then(|i| i.parent).is_none() {
                continue;
            }
            // the title line, then the lines of the block and the blocks in it
            let (t, mut end) = match title {
                Some(t) => (t, t + 1),
                None => (n, n),
            };
            while end < n && self.within(self.buf.lines[end].owner, o) {
                end += 1;
            }
            for i in 1..n {
                if (i < t || i >= end) && self.buf.lines[i].owner == o {
                    let above = self.buf.lines[i - 1].owner;
                    self.buf.lines[i].owner = above;
                    self.buf.mark_dirty(above);
                    self.buf.mark_dirty(o);
                }
            }
        }
    }

    /// A nested block whose title line no longer sits among the lines of
    /// the block its embed is in moves its embed to the block it now sits in
    /// (§5.2): the innermost one holding both the line above the title line
    /// and the block it was in. Blocks go in buffer order, so where a block
    /// sits is settled before the blocks after it.
    fn reparent(&mut self) {
        let titles = self.titles();
        let mut order: Vec<(usize, Owner)> = titles.iter().map(|(&o, &t)| (t, o)).collect();
        order.sort();
        for (t, o) in order {
            let Some(p) = self.buf.owners.get(&o).and_then(|i| i.parent) else { continue };
            let inside = titles.get(&p).is_some_and(|&tp| tp < t && (tp + 1..t).all(|i| self.within(self.buf.lines[i].owner, p)));
            if inside || t == 0 {
                continue;
            }
            let mut q = self.buf.lines[t - 1].owner;
            while !self.within(p, q) {
                match self.buf.owners.get(&q).and_then(|i| i.parent) {
                    Some(up) => q = up,
                    None => break,
                }
            }
            if q != p && q != o && self.within(p, q) {
                if let Some(i) = self.buf.owners.get_mut(&o) {
                    i.parent = Some(q);
                }
                self.buf.mark_dirty(p);
                self.buf.mark_dirty(q);
            }
        }
    }

    /// The block each block's embed sits in.
    fn parents(&self) -> BTreeMap<Owner, Option<Owner>> {
        self.buf.owners.iter().map(|(&o, i)| (o, i.parent)).collect()
    }

    /// What splice writes for each block (§5.2): its own lines, and the
    /// first line of each block nested in it, where that block's embed goes.
    fn splice_views(lines: &[EditLine], parents: &BTreeMap<Owner, Option<Owner>>) -> BTreeMap<Owner, Vec<(Owner, String)>> {
        let mut views: BTreeMap<Owner, Vec<(Owner, String)>> = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for l in lines {
            views.entry(l.owner).or_default().push((l.owner, l.text.clone()));
            let mut o = l.owner;
            while let Some(p) = parents.get(&o).copied().flatten() {
                if seen.insert(o) {
                    views.entry(p).or_default().push((o, l.text.clone()));
                }
                o = p;
            }
        }
        views
    }

    // -------------------------------------------------------------- clipboard

    pub fn copy(&mut self, text: String, linewise: bool) {
        self.set_clip(text, linewise, Vec::new());
    }

    /// Fill the clipboard. The blocks whose title lines it holds are in
    /// transit until they are pasted back or the clipboard is replaced: the
    /// buffer holds them, so a save meanwhile does not delete them (§5.2).
    fn set_clip(&mut self, text: String, linewise: bool, tags: Vec<Option<Owner>>) {
        self.copied = Some(text.clone());
        self.buf.hold(tags.iter().flatten().copied().collect());
        self.clip = Clip { text, linewise, tags };
    }

    /// Leaving the editor: the clipboard keeps its text, but a block whose
    /// title line was cut and not pasted back is deleted now (§5.2).
    pub fn release_clip(&mut self) {
        self.clip.tags.clear();
        self.buf.hold(Vec::new());
    }

    fn after_cursor(&self) -> Pos {
        Pos::new(self.cursor.line, (self.cursor.col + 1).min(self.len(self.cursor.line)))
    }

    // -------------------------------------------------------------- undo

    /// Remember the text before a change. Consecutive typing is one step.
    pub fn checkpoint(&mut self) {
        self.redo.clear();
        self.undo.push(Snap { lines: self.buf.lines.clone(), cursor: self.cursor, parents: self.parents() });
        self.group = Group::None;
    }

    fn checkpoint_typing(&mut self) {
        if self.group != Group::Typing {
            self.checkpoint();
            self.group = Group::Typing;
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else {
            self.message = Some("nothing to undo".into());
            return false;
        };
        let cur = self.restore(s);
        self.redo.push(cur);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else {
            self.message = Some("nothing to redo".into());
            return false;
        };
        let cur = self.restore(s);
        self.undo.push(cur);
        true
    }

    /// Put a snapshot back; every block whose text differs is dirty again,
    /// and so is every block a nested block moved in.
    fn restore(&mut self, s: Snap) -> Snap {
        let cur = Snap { lines: std::mem::take(&mut self.buf.lines), cursor: self.cursor, parents: self.parents() };
        let was = Self::splice_views(&cur.lines, &cur.parents);
        let now = Self::splice_views(&s.lines, &s.parents);
        for o in was.keys().chain(now.keys()) {
            if was.get(o) != now.get(o) {
                self.buf.mark_dirty(*o);
            }
        }
        self.buf.lines = s.lines;
        for (o, p) in s.parents {
            if let Some(i) = self.buf.owners.get_mut(&o) {
                i.parent = p;
            }
        }
        self.cursor = s.cursor;
        self.forget_goal();
        self.anchor = None;
        self.changes += 1;
        self.group = Group::None;
        self.clamp_cursor();
        cur
    }

    // -------------------------------------------------------------- motion

    pub fn clamp_cursor(&mut self) {
        let last = self.lines() - 1;
        self.cursor.line = self.cursor.line.min(last);
        let len = self.len(self.cursor.line);
        // outside insert mode the cursor sits on a character — except the end
        // of a Helix selection, which may take in the line's newline
        let helix_sel = self.keys == Keys::Helix && self.anchor.is_some();
        let max = if self.block_cursor() && len > 0 && !helix_sel { len - 1 } else { len };
        self.cursor.col = self.cursor.col.min(max);
    }

    pub fn move_vert(&mut self, delta: isize) {
        let want = self.want_col.unwrap_or(self.cursor.col);
        let line = (self.cursor.line as isize + delta).clamp(0, self.lines() as isize - 1) as usize;
        self.cursor = Pos::new(line, want.min(self.len(line)));
        self.want_col = Some(want);
    }

    pub fn set_cursor(&mut self, p: Pos) {
        self.cursor = p;
        self.forget_goal();
    }

    /// Up and down aim from where the cursor is now, not for the column an
    /// earlier vertical move aimed for (after typing, undo, …).
    fn forget_goal(&mut self) {
        self.want_col = None;
        self.want_x = None;
    }

    // -------------------------------------------------------------- layout

    /// Every line's screen rows (§10.1): prose wraps at spaces, lines inside
    /// fenced code break hard. Cached until the text or width changes.
    pub fn layout(&mut self) -> &Vec<Vec<Row>> {
        let fresh = matches!(&self.layout, Some((c, w, _, _)) if *c == self.changes && *w == self.wrap_cols);
        if !fresh {
            let mut rows = Vec::with_capacity(self.lines());
            let mut codes = Vec::with_capacity(self.lines());
            let mut fence: Option<(char, usize)> = None;
            for l in 0..self.lines() {
                let text = self.line(l);
                // fences as the parser reads them (§3.3); the fences
                // themselves are not code
                let code = !fold_core::parse::fence_transition(text, &mut fence) && fence.is_some();
                codes.push(code);
                rows.push(match self.wrap_cols {
                    Some(cols) => wrap::wrap(text, cols, code),
                    None => vec![Row { start: 0, end: text.chars().count(), indent: 0 }],
                });
            }
            self.layout = Some((self.changes, self.wrap_cols, rows, codes));
        }
        &self.layout.as_ref().unwrap().2
    }

    /// Which lines sit inside fenced code (they break hard, with `↪`).
    pub fn code_lines(&mut self) -> Vec<bool> {
        self.layout();
        self.layout.as_ref().unwrap().3.clone()
    }

    /// Screen row (counted over the whole buffer) and column of a position.
    pub fn pos_to_screen(&mut self, p: Pos) -> (usize, usize) {
        let before: usize = self.layout().iter().take(p.line).map(|r| r.len()).sum();
        let l = p.line.min(self.lines() - 1);
        let rows = self.layout()[l].clone();
        let (r, x) = wrap::locate(&rows, self.line(p.line), p.col);
        (before + r, x)
    }

    /// The position under screen row `row` (over the whole buffer), column `x`.
    pub fn screen_to_pos(&mut self, row: usize, x: usize) -> Pos {
        let mut left = row;
        let n = self.lines();
        for l in 0..n {
            let rows = self.layout()[l].clone();
            if left < rows.len() || l + 1 == n {
                let r = left.min(rows.len() - 1);
                return Pos::new(l, wrap::column_at(&rows, self.line(l), r, x));
            }
            left -= rows.len();
        }
        Pos::new(0, 0)
    }

    /// Up or down by screen rows: a wrapped line is several rows (the
    /// normal keymap's arrows, Helix's `j`/`k`, Vim's `gj`/`gk`).
    pub fn move_visual(&mut self, delta: isize) {
        if self.wrap_cols.is_none() {
            return self.move_vert(delta);
        }
        let (row, x) = self.pos_to_screen(self.cursor);
        let want = self.want_x.unwrap_or(x);
        let total: usize = self.layout().iter().map(|r| r.len()).sum();
        let target = (row as isize + delta).clamp(0, total as isize - 1) as usize;
        let p = self.screen_to_pos(target, want);
        self.cursor = p;
        self.want_col = None;
        self.want_x = Some(want);
    }

    pub fn first_non_blank(&self, l: usize) -> usize {
        self.line(l).chars().take_while(|c| c.is_whitespace()).count()
    }

    /// Start of the next word (`w`), or of the next WORD (`W`).
    pub fn word_fwd(&self, p: Pos, big: bool) -> Pos {
        let mut q = p;
        let start = self.char_at(q).map(|c| class(c, big));
        if let Some(k) = start.filter(|k| *k != Class::Space) {
            while let Some(c) = self.char_at(q) {
                if class(c, big) != k {
                    break;
                }
                match self.next(q) {
                    Some(n) => q = n,
                    None => return Pos::new(q.line, self.len(q.line)),
                }
            }
        }
        // skip whitespace; an empty line is a word
        while let Some(c) = self.char_at(q) {
            if class(c, big) != Class::Space {
                break;
            }
            let Some(n) = self.next(q) else { break };
            if c == '\n' && self.len(n.line) == 0 {
                return n;
            }
            q = n;
        }
        q
    }

    /// End of the word (`e` / `E`).
    pub fn word_end(&self, p: Pos, big: bool) -> Pos {
        let mut q = match self.next(p) {
            Some(n) => n,
            None => return p,
        };
        while let Some(c) = self.char_at(q) {
            if class(c, big) != Class::Space {
                break;
            }
            match self.next(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        let k = self.char_at(q).map(|c| class(c, big));
        while let Some(n) = self.next(q) {
            if self.char_at(n).map(|c| class(c, big)) != k || self.char_at(n) == Some('\n') {
                break;
            }
            q = n;
        }
        q
    }

    /// Start of the previous word (`b` / `B`).
    pub fn word_back(&self, p: Pos, big: bool) -> Pos {
        let mut q = match self.prev(p) {
            Some(n) => n,
            None => return p,
        };
        while let Some(c) = self.char_at(q) {
            if class(c, big) != Class::Space || (c == '\n' && self.len(q.line) == 0) {
                break;
            }
            match self.prev(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        let k = self.char_at(q).map(|c| class(c, big));
        while let Some(n) = self.prev(q) {
            if self.char_at(n).map(|c| class(c, big)) != k || self.char_at(n) == Some('\n') {
                break;
            }
            q = n;
        }
        q
    }

    /// The word under a position: `[start, end)`.
    pub fn word_at(&self, p: Pos, big: bool) -> (Pos, Pos) {
        let line: Vec<char> = self.line(p.line).chars().collect();
        if line.is_empty() {
            return (p, p);
        }
        let i = p.col.min(line.len() - 1);
        let k = class(line[i], big);
        let mut s = i;
        while s > 0 && class(line[s - 1], big) == k {
            s -= 1;
        }
        let mut e = i + 1;
        while e < line.len() && class(line[e], big) == k {
            e += 1;
        }
        (Pos::new(p.line, s), Pos::new(p.line, e))
    }

    /// The next (or previous) blank line: `}` / `{`.
    pub fn paragraph(&self, from: usize, fwd: bool) -> usize {
        let blank = |l: usize| self.line(l).trim().is_empty();
        let mut l = from;
        if fwd {
            while l + 1 < self.lines() && blank(l) {
                l += 1;
            }
            while l + 1 < self.lines() && !blank(l) {
                l += 1;
            }
        } else {
            while l > 0 && blank(l) {
                l -= 1;
            }
            while l > 0 && !blank(l) {
                l -= 1;
            }
        }
        l
    }

    /// Find a character on the cursor's line: `f` (forward, on it), `t`
    /// (forward, before it), `F`, `T`. A `t`/`T` target right next to the
    /// cursor is found where the cursor is, as in Vim, unless `skip`: then
    /// the search starts past it, as Helix's `t` and Vim's `;` do.
    pub fn find_char(&self, from: Pos, c: char, kind: char, count: usize, skip: bool) -> Option<Pos> {
        let line: Vec<char> = self.line(from.line).chars().collect();
        let mut col = from.col;
        for k in 0..count.max(1) {
            // after the first, each step starts past the target it found
            let past = usize::from(skip || k > 0);
            col = match kind {
                'f' | 't' => {
                    let start = if kind == 't' { col + 1 + past } else { col + 1 };
                    let i = (start.min(line.len())..line.len()).find(|&i| line[i] == c)?;
                    if kind == 't' { i - 1 } else { i }
                }
                _ => {
                    let end = if kind == 'T' { col.saturating_sub(past) } else { col };
                    let i = (0..end).rev().find(|&i| line[i] == c)?;
                    if kind == 'T' { i + 1 } else { i }
                }
            };
        }
        Some(Pos::new(from.line, col))
    }

    /// The bracket matching the one at (or after) the cursor: `%`.
    pub fn match_bracket(&self, p: Pos) -> Option<Pos> {
        let line: Vec<char> = self.line(p.line).chars().collect();
        let i = (p.col..line.len()).find(|&i| "()[]{}".contains(line[i]))?;
        let (open, close, fwd) = match line[i] {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            _ => ('{', '}', false),
        };
        let mut depth = 0i32;
        let mut q = Pos::new(p.line, i);
        loop {
            match self.char_at(q) {
                Some(c) if c == open => depth += if fwd { 1 } else { -1 },
                Some(c) if c == close => depth += if fwd { -1 } else { 1 },
                _ => {}
            }
            if depth == 0 {
                return Some(q);
            }
            q = if fwd { self.next(q)? } else { self.prev(q)? };
        }
    }

    /// The span of a text object around the cursor: `iw`, `a(`, `i"`, `ip`, …
    pub fn text_object(&self, around: bool, obj: char) -> Option<(Pos, Pos)> {
        let p = self.cursor;
        match obj {
            'w' | 'W' => {
                let (s, mut e) = self.word_at(p, obj == 'W');
                if around {
                    let line: Vec<char> = self.line(p.line).chars().collect();
                    while e.col < line.len() && line[e.col].is_whitespace() {
                        e.col += 1;
                    }
                }
                Some((s, e))
            }
            '"' | '\'' | '`' => {
                // quotes pair up from the start of the line: an odd number
                // before the cursor means it is inside (or on the closer)
                let line: Vec<char> = self.line(p.line).chars().collect();
                let col = p.col.min(line.len());
                let before = line[..col].iter().filter(|&&c| c == obj).count();
                let (open, close) = if before % 2 == 1 {
                    let o = (0..col).rev().find(|&i| line[i] == obj)?;
                    (o, (col..line.len()).find(|&i| line[i] == obj)?)
                } else {
                    let o = (col..line.len()).find(|&i| line[i] == obj)?;
                    (o, (o + 1..line.len()).find(|&i| line[i] == obj)?)
                };
                Some(if around {
                    (Pos::new(p.line, open), Pos::new(p.line, close + 1))
                } else {
                    (Pos::new(p.line, open + 1), Pos::new(p.line, close))
                })
            }
            '(' | ')' | 'b' | '[' | ']' | '{' | '}' | 'B' | '<' | '>' => {
                let (o, c) = match obj {
                    '(' | ')' | 'b' => ('(', ')'),
                    '[' | ']' => ('[', ']'),
                    '<' | '>' => ('<', '>'),
                    _ => ('{', '}'),
                };
                // walk back to the unmatched opener
                let mut depth = 0;
                let mut q = p;
                let open = loop {
                    match self.char_at(q) {
                        Some(ch) if ch == c && q != p => depth += 1,
                        Some(ch) if ch == o => {
                            if depth == 0 {
                                break q;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                    q = self.prev(q)?;
                };
                let mut depth = 0;
                let mut q = self.next(open)?;
                let close = loop {
                    match self.char_at(q) {
                        Some(ch) if ch == o => depth += 1,
                        Some(ch) if ch == c => {
                            if depth == 0 {
                                break q;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                    q = self.next(q)?;
                };
                Some(if around { (open, self.next(close)?) } else { (self.next(open)?, close) })
            }
            'p' => {
                let blank = |l: usize| self.line(l).trim().is_empty();
                let mut s = p.line;
                while s > 0 && !blank(s - 1) {
                    s -= 1;
                }
                let mut e = p.line;
                while e + 1 < self.lines() && !blank(e + 1) {
                    e += 1;
                }
                if around {
                    while e + 1 < self.lines() && blank(e + 1) {
                        e += 1;
                    }
                }
                let end = if e + 1 < self.lines() { Pos::new(e + 1, 0) } else { Pos::new(e, self.len(e)) };
                Some((Pos::new(s, 0), end))
            }
            _ => None,
        }
    }

    // -------------------------------------------------------------- edits

    /// Typing: replaces a selection, one undo step per run of typing.
    pub fn type_char(&mut self, c: char) {
        self.checkpoint_typing();
        self.forget_goal();
        if self.anchor.is_some() && self.keys == Keys::Normal {
            self.delete_selection();
        }
        self.cursor = self.insert(self.cursor, &c.to_string());
    }

    /// Enter: a new line, keeping the current line's indentation.
    pub fn newline(&mut self) {
        self.checkpoint_typing();
        self.forget_goal();
        if self.anchor.is_some() && self.keys == Keys::Normal {
            self.delete_selection();
        }
        let indent: String = self.line(self.cursor.line).chars().take_while(|c| *c == ' ').collect();
        let indent: String = indent.chars().take(self.cursor.col).collect();
        self.cursor = self.insert(self.cursor, &format!("\n{}", indent));
    }

    pub fn backspace(&mut self) {
        self.checkpoint_typing();
        self.forget_goal();
        if self.anchor.is_some() && self.keys == Keys::Normal {
            self.delete_selection();
            return;
        }
        if let Some(p) = self.prev(self.cursor) {
            // a run of indentation goes back one level at a time
            let line = self.line(self.cursor.line);
            let col = self.cursor.col;
            let n = if col >= 2 && col % 2 == 0 && line.chars().take(col).all(|c| c == ' ') { 2 } else { 1 };
            let start = if n == 2 { Pos::new(self.cursor.line, col - 2) } else { p };
            self.delete(start, self.cursor);
            self.cursor = start;
        }
    }

    pub fn delete_forward(&mut self) {
        self.checkpoint_typing();
        self.forget_goal();
        if self.anchor.is_some() && self.keys == Keys::Normal {
            self.delete_selection();
            return;
        }
        if let Some(n) = self.next(self.cursor) {
            self.delete(self.cursor, n);
        }
    }

    /// Delete back to the start of the word (Ctrl-W, Alt-Backspace).
    pub fn delete_word_back(&mut self) {
        self.checkpoint();
        self.forget_goal();
        let start = self.word_back(self.cursor, false);
        self.delete(start, self.cursor);
        self.cursor = start;
    }

    /// Indent (`dir > 0`) or dedent lines `l1..=l2` by two spaces.
    pub fn indent(&mut self, l1: usize, l2: usize, dir: i32) {
        self.checkpoint();
        for l in l1..=l2.min(self.lines() - 1) {
            if dir > 0 {
                if !self.line(l).is_empty() {
                    self.insert(Pos::new(l, 0), "  ");
                }
            } else {
                let n = self.line(l).chars().take(2).take_while(|c| *c == ' ').count();
                self.delete(Pos::new(l, 0), Pos::new(l, n));
            }
        }
        let shift = if dir > 0 { 2 } else { 0 };
        self.cursor.col = if dir > 0 { self.cursor.col + shift } else { self.cursor.col.saturating_sub(2) };
    }

    /// Join `count` lines below onto line `l` with single spaces (`J`).
    pub fn join(&mut self, l: usize, count: usize) {
        self.checkpoint();
        for _ in 0..count.max(1) {
            if l + 1 >= self.lines() {
                break;
            }
            let end = Pos::new(l, self.len(l));
            let next_indent = self.first_non_blank(l + 1);
            let sep = if self.line(l).is_empty() || self.line(l + 1).trim().is_empty() { "" } else { " " };
            self.delete(end, Pos::new(l + 1, next_indent));
            self.insert(end, sep);
            self.cursor = end;
        }
    }

    /// Replace each character of `[a, b)` by `f` of it, one line at a time:
    /// line ends stay where they are and every line keeps its tag (§5.2).
    pub fn map_chars(&mut self, a: Pos, b: Pos, f: impl Fn(char) -> char) {
        let (a, mut b) = order(a, b);
        if b.line >= self.lines() {
            b = Pos::new(self.lines() - 1, self.len(self.lines() - 1));
        }
        for l in a.line..=b.line {
            let s = if l == a.line { a.col } else { 0 };
            let e = if l == b.line { b.col } else { usize::MAX };
            let new: String = self.line(l).chars().enumerate().map(|(i, c)| if i >= s && i < e { f(c) } else { c }).collect();
            if new != self.line(l) {
                self.changes += 1;
                self.buf.set_line(l, new);
            }
        }
    }

    /// Change the case of `[a, b)`: `u` lower, `U` upper, `~` swap. A
    /// character whose other case is not one character ('ß' is "SS") stays
    /// as it is, as in Vim.
    pub fn change_case(&mut self, a: Pos, b: Pos, how: char) {
        fn one(c: char, mut other: impl Iterator<Item = char>) -> char {
            match (other.next(), other.next()) {
                (Some(x), None) => x,
                _ => c,
            }
        }
        self.map_chars(a, b, |c| match how {
            'u' => one(c, c.to_lowercase()),
            'U' => one(c, c.to_uppercase()),
            _ if c.is_uppercase() => one(c, c.to_lowercase()),
            _ => one(c, c.to_uppercase()),
        });
    }

    // -------------------------------------------------------------- search

    /// The next match of the search after (or before) a position, wrapping.
    pub fn find(&self, pat: &str, from: Pos, fwd: bool) -> Option<Pos> {
        if pat.is_empty() {
            return None;
        }
        let pat = pat.to_lowercase();
        let n = self.lines();
        let hits = |l: usize| -> Vec<usize> {
            // a character can lowercase to several ('İ' to "i̇"): count
            // columns in the line itself, not in its lowercase
            let mut line = String::new();
            let mut col_of = Vec::new();
            for (col, c) in self.line(l).chars().enumerate() {
                for lc in c.to_lowercase() {
                    line.push(lc);
                    col_of.push(col);
                }
            }
            let mut out = Vec::new();
            let mut start = 0;
            while let Some(i) = line[start..].find(&pat) {
                out.push(col_of[line[..start + i].chars().count()]);
                start += i + pat.len().max(1);
            }
            out
        };
        for k in 0..=n {
            let l = if fwd { (from.line + k) % n } else { (from.line + n - k % n) % n };
            let hs = hits(l);
            let found = if fwd {
                hs.into_iter().find(|&c| k > 0 || c > from.col)
            } else {
                hs.into_iter().rev().find(|&c| k > 0 || c < from.col)
            };
            if let Some(c) = found {
                return Some(Pos::new(l, c));
            }
        }
        None
    }

    /// Jump to the next match of the last search.
    pub fn search_next(&mut self, fwd: bool) -> Option<Pos> {
        let pat = self.search.clone()?;
        // Helix searches on from the selection's end, or back from its start,
        // so the match it has selected is not found again
        let from = match self.anchor {
            Some(a) if self.keys == Keys::Helix => {
                if fwd { a.max(self.cursor) } else { a.min(self.cursor) }
            }
            _ => self.cursor,
        };
        match self.find(&pat, from, fwd) {
            Some(p) => {
                self.set_cursor(p);
                Some(p)
            }
            None => {
                self.message = Some(format!("not found: {}", pat));
                None
            }
        }
    }

    // -------------------------------------------------------------- command line

    pub fn open_cmdline(&mut self, kind: char) {
        self.cmdline = Some(CmdLine { kind, text: String::new() });
    }

    fn cmdline_key(&mut self, key: KeyEvent) -> Outcome {
        let Some(mut cl) = self.cmdline.take() else { return Outcome::default() };
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            KeyCode::Enter => return self.run_cmdline(cl),
            KeyCode::Backspace => {
                if cl.text.pop().is_some() {
                    self.cmdline = Some(cl);
                }
            }
            KeyCode::Char(c) => {
                cl.text.push(c);
                self.cmdline = Some(cl);
            }
            _ => self.cmdline = Some(cl),
        }
        Outcome::default()
    }

    fn run_cmdline(&mut self, cl: CmdLine) -> Outcome {
        let t = cl.text.trim();
        match cl.kind {
            '/' | '?' => {
                if !t.is_empty() {
                    self.search = Some(t.to_string());
                }
                let fwd = cl.kind == '/';
                self.search_fwd = fwd;
                if let Some(p) = self.search_next(fwd) {
                    if self.keys == Keys::Helix || self.keys == Keys::Normal {
                        // select the match
                        let len = self.search.as_ref().map(|s| s.chars().count()).unwrap_or(0);
                        let end = Pos::new(p.line, p.col + len);
                        match self.keys {
                            Keys::Helix => {
                                self.anchor = Some(p);
                                self.cursor = Pos::new(p.line, (p.col + len).saturating_sub(1));
                            }
                            _ => {
                                self.anchor = Some(p);
                                self.cursor = end;
                            }
                        }
                    }
                }
                Outcome::default()
            }
            _ => self.ex(t),
        }
    }

    /// `:` commands, shared by all keymaps.
    pub fn ex(&mut self, cmd: &str) -> Outcome {
        let mut out = Outcome::default();
        match cmd {
            "w" | "write" | "save" => out.save = true,
            "q" | "quit" | "close" | "wq" | "x" | "write-quit" | "exit" => out.close = true,
            "q!" | "quit!" | "e!" | "reload" | "revert" | "cq" => out.revert = true,
            "noh" | "nohlsearch" => self.search = None,
            "" => {}
            n if n.chars().all(|c| c.is_ascii_digit()) => {
                let l = n.parse::<usize>().unwrap_or(1).saturating_sub(1);
                self.set_cursor(Pos::new(l.min(self.lines() - 1), 0));
                self.anchor = None;
            }
            other => self.message = Some(format!("unknown command: {}", other)),
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char, big: bool) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if big || c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

pub fn order(a: Pos, b: Pos) -> (Pos, Pos) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Byte offset of a character column.
fn byte(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map(|(i, _)| i).unwrap_or(s.len())
}

fn ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// An editor over a one-file vault holding `text` (under a `# T` title).
    pub fn editor(text: &str, keys: Keys) -> (tempfile::TempDir, Editor) {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("root.md"), format!("# T\n\n{}", text)).unwrap();
        let v = fold_core::vault::Vault::open(d.path()).unwrap();
        let t = v.tree.resolved_children(v.tree.root)[0];
        let buf = fold_core::edit::open_editor(&v, t);
        let mut e = Editor::new(buf, keys, Clip::default());
        e.cursor = Pos::new(2, 0);
        (d, e)
    }

    /// Feed a string of keys: `<Esc>`, `<CR>`, `<BS>`, `<C-x>`, `<A-x>`,
    /// `<S-Left>` and friends in angle brackets, everything else literal.
    pub fn keys(e: &mut Editor, s: &str) -> Vec<Outcome> {
        let mut outs = Vec::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            let key = if c == '<' && chars.peek().is_some_and(|n| n.is_ascii_alphabetic()) {
                let name: String = chars.by_ref().take_while(|&c| c != '>').collect();
                parse_key(&name)
            } else {
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
            };
            outs.push(e.handle(key));
        }
        outs
    }

    fn parse_key(name: &str) -> KeyEvent {
        let mut mods = KeyModifiers::NONE;
        let mut base = name;
        while base.len() > 2 && base.as_bytes()[1] == b'-' && "CAS".contains(&base[..1]) {
            mods |= match &base[..1] {
                "C" => KeyModifiers::CONTROL,
                "A" => KeyModifiers::ALT,
                _ => KeyModifiers::SHIFT,
            };
            base = &base[2..];
        }
        let code = match base {
            "Esc" => KeyCode::Esc,
            "CR" | "Enter" => KeyCode::Enter,
            "BS" => KeyCode::Backspace,
            "Del" => KeyCode::Delete,
            "Tab" => KeyCode::Tab,
            "BTab" => KeyCode::BackTab,
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            "PgUp" => KeyCode::PageUp,
            "PgDn" => KeyCode::PageDown,
            "lt" => KeyCode::Char('<'),
            one if one.chars().count() == 1 => KeyCode::Char(one.chars().next().unwrap()),
            other => panic!("unknown key {}", other),
        };
        KeyEvent::new(code, mods)
    }

    /// The text after the `# T` title and its blank line.
    pub fn body(e: &Editor) -> String {
        (2..e.lines()).map(|l| e.line(l).to_string()).collect::<Vec<_>>().join("\n")
    }
}
