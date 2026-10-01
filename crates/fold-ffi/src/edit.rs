//! The editor behind a platform text field (§10.6).
//!
//! A text field hands over its whole text after every change. `apply`
//! finds the one span that changed and replays it on the tagged buffer
//! (`fold_core::edit::EditBuffer`) through the same rules the TUI's editor
//! follows (`fold-tui/src/app/editor/mod.rs`): every line keeps or inherits
//! the tag of the block that owns it (§5.2), whole lines cut with a nested
//! block's title line hold the block in transit, and pasting those lines
//! back puts the block where they land.

use fold_core::edit::{EditBuffer, EditLine, Owner};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

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

/// Whole lines cut in this editor, and the block of each line whose title
/// line was cut with it (§5.2: cut and paste moves its embed).
#[derive(Clone, Debug)]
struct Clip {
    text: String,
    tags: Vec<Option<Owner>>,
}

/// The text before a change, for the editor's own undo.
struct Snap {
    lines: Vec<EditLine>,
    parents: BTreeMap<Owner, Option<Owner>>,
}

/// Small changes this close together are one undo step (typing).
const TYPING: Duration = Duration::from_secs(2);

pub struct TextEditor {
    pub buf: EditBuffer,
    clip: Option<Clip>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// When the last small change (a character typed or erased) was made:
    /// the next one within `TYPING` joins its undo step.
    typing: Option<Instant>,
}

impl TextEditor {
    pub fn new(buf: EditBuffer) -> TextEditor {
        TextEditor {
            buf,
            clip: None,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: None,
        }
    }

    /// The buffer's text, lines joined by `\n`.
    pub fn text(&self) -> String {
        self.buf.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    pub fn dirty(&self) -> bool {
        !self.buf.dirty.is_empty()
    }

    /// Take in the text field's new text. `cursor`, the caret after the
    /// change in characters, decides where a change that could have been
    /// made in more than one place was made: Enter at the end of a line or
    /// at the start of the next leaves the same text, but the new line
    /// belongs to a different block (§5.2). False when nothing changed.
    pub fn apply(&mut self, new: &str, cursor: Option<usize>) -> bool {
        let old: Vec<char> = self.text().chars().collect();
        let new: Vec<char> = new.chars().collect();
        if old == new {
            return false;
        }
        let mut p = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - p;
        let s = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
        let (del, ins) = (old.len() - p - s, new.len() - p - s);
        // where the caret says the change was, if the text agrees
        if let Some(c) = cursor {
            let start = if del == 0 { c.checked_sub(ins) } else if ins == 0 { Some(c) } else { None };
            if let Some(start) = start.filter(|&st| st != p) {
                let fits = start + del <= old.len()
                    && start + ins <= new.len()
                    && old[..start] == new[..start]
                    && old[start + del..] == new[start + ins..];
                if fits {
                    p = start;
                }
            }
        }
        let small = del <= 1 && ins <= 1;
        let joins = small && self.typing.is_some_and(|t| t.elapsed() < TYPING);
        if !joins {
            self.checkpoint();
        }
        self.typing = small.then(Instant::now);
        let at = pos_of(&old, p);
        if del > 0 {
            let end = pos_of(&old, p + del);
            self.remove(at, end);
        }
        if ins > 0 {
            let s: String = new[p..p + ins].iter().collect();
            self.put(at, &s);
        }
        true
    }

    /// Delete `[a, b)`: whole lines, from the start of one to the start of
    /// another, that hold a nested block's title line are cut as the TUI's
    /// linewise cut is, so the block is held in transit until they are
    /// pasted back (§5.2); anything else is deleted.
    fn remove(&mut self, a: Pos, b: Pos) {
        if a.col == 0 && b.col == 0 && b.line > a.line && self.has_nested_title(a.line, b.line - 1) {
            self.cut_lines(a.line, b.line - 1);
        } else {
            self.delete(a, b);
        }
    }

    /// Insert `s` at `at`: the lines the last cut took, put back in front of
    /// a line, go back with their tags (§5.2); anything else is typed.
    fn put(&mut self, at: Pos, s: &str) {
        let clip = self.clip.clone().filter(|c| c.tags.iter().any(|t| t.is_some()));
        let body = clip.as_ref().map(|c| c.text.strip_suffix('\n').unwrap_or(&c.text).to_string());
        match (clip, body) {
            // in front of a line: the lines go above it
            (Some(clip), _) if at.col == 0 && clip.text == s => self.put_clip_lines(at.line, &clip, false),
            // a line break and the lines at the end of a line: below it
            (Some(clip), Some(body)) if at.col == self.len(at.line) && s.strip_prefix('\n') == Some(body.as_str()) => {
                self.put_clip_lines(at.line, &clip, true)
            }
            _ => {
                self.insert(at, s);
            }
        }
    }

    fn has_nested_title(&self, l1: usize, l2: usize) -> bool {
        let titles = self.titles();
        (l1..=l2.min(self.lines() - 1)).any(|l| {
            let o = self.buf.lines[l].owner;
            self.nested(o) && titles.get(&o) == Some(&l)
        })
    }

    fn nested(&self, o: Owner) -> bool {
        self.buf.owners.get(&o).is_some_and(|i| i.parent.is_some())
    }

    // ------------------------------------------------------------ text

    fn lines(&self) -> usize {
        self.buf.lines.len().max(1)
    }

    fn line(&self, l: usize) -> &str {
        self.buf.lines.get(l).map(|x| x.text.as_str()).unwrap_or("")
    }

    fn len(&self, l: usize) -> usize {
        self.line(l).chars().count()
    }

    /// The text of `[a, b)`.
    fn text_between(&self, a: Pos, b: Pos) -> String {
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
    /// a nested block's title line is not written into that block.
    fn insert(&mut self, at: Pos, s: &str) -> Pos {
        if s.is_empty() {
            return at;
        }
        if self.buf.lines.is_empty() {
            self.buf.lines.push(EditLine { text: String::new(), owner: Owner { file: 0, block_ord: 0 } });
        }
        let line = self.line(at.line).to_string();
        let b = byte(&line, at.col);
        let (pre, post) = line.split_at(b);
        let parts: Vec<&str> = s.split('\n').collect();
        if parts.len() == 1 {
            self.buf.set_line(at.line, format!("{}{}{}", pre, s, post));
            return Pos::new(at.line, at.col + s.chars().count());
        }
        let owner = self.buf.lines[at.line].owner;
        let block_title = self.nested(owner) && !self.buf.lines[..at.line].iter().any(|l| l.owner == owner);
        if at.col == 0 && at.line > 0 && block_title && !(parts[0].is_empty() && post.is_empty()) {
            // in front of a nested block's title line, whole lines go in
            // after the line above, taking its tag: they are not the block's
            // text; the title line keeps its own tag and only gains the last
            // part
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
    /// line `a`, unless the range starts at column 0 and leaves some of line
    /// `b`, or ends at its start (short of the end of the text): then line
    /// `b` is what is left. A range from column 0 through the end of line
    /// `b` deletes lines `a` to `b` whole, and the empty line left keeps the
    /// tag of a block whose title line was not among them (`kept`).
    fn delete(&mut self, a: Pos, b: Pos) -> String {
        let (a, mut b) = order(a, b);
        if b.line >= self.lines() {
            b = Pos::new(self.lines() - 1, self.len(self.lines() - 1));
        }
        let gone = self.text_between(a, b);
        if gone.is_empty() {
            return gone;
        }
        let first = self.line(a.line).to_string();
        let last = self.line(b.line).to_string();
        let rest = &last[byte(&last, b.col)..];
        if a.col == 0 && a.line < b.line && (!rest.is_empty() || (b.col == 0 && b.line + 1 < self.lines())) {
            for l in (a.line..b.line).rev() {
                self.buf.delete_line(l);
            }
            self.buf.set_line(a.line, rest.to_string());
            return gone;
        }
        let whole = (a.col == 0 && a.line < b.line).then(|| self.kept(a.line, b.line));
        let joined = format!("{}{}", &first[..byte(&first, a.col)], rest);
        for l in (a.line + 1..=b.line).rev() {
            self.buf.delete_line(l);
        }
        self.buf.set_line(a.line, joined);
        if let Some(o) = whole {
            let was = std::mem::replace(&mut self.buf.lines[a.line].owner, o);
            self.buf.mark_dirty(was);
            self.buf.mark_dirty(o);
        }
        gone
    }

    /// The block the empty line left where lines `l1..=l2` are deleted whole
    /// belongs to: one whose title line (§5.2) is not among them — line
    /// `l1`'s, else line `l2`'s, else the block they sit in.
    fn kept(&self, l1: usize, l2: usize) -> Owner {
        let titles = self.titles();
        let gone = |o: Owner| self.nested(o) && titles.get(&o).is_some_and(|t| (l1..=l2).contains(t));
        let (mut o, last) = (self.buf.lines[l1].owner, self.buf.lines[l2].owner);
        if gone(o) && !gone(last) {
            return last;
        }
        while gone(o) {
            match self.buf.owners.get(&o).and_then(|i| i.parent) {
                Some(p) => o = p,
                None => break,
            }
        }
        o
    }

    /// Delete whole lines `l1..=l2`; returns them, each ending in `\n`.
    fn delete_lines(&mut self, l1: usize, l2: usize) -> String {
        let l2 = l2.min(self.lines() - 1);
        let mut gone = String::new();
        for l in l1..=l2 {
            gone.push_str(self.line(l));
            gone.push('\n');
        }
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
        gone
    }

    /// Cut whole lines `l1..=l2`. A nested block whose title line goes
    /// keeps its tag in the clip, and its lines left behind go to the block
    /// they now sit in, so pasting the lines moves the block (§5.2).
    fn cut_lines(&mut self, l1: usize, l2: usize) {
        let l2 = l2.min(self.lines() - 1);
        let titles = self.titles();
        let tags: Vec<Option<Owner>> = (l1..=l2)
            .map(|l| {
                let o = self.buf.lines.get(l)?.owner;
                (self.nested(o) && titles.get(&o).is_some_and(|&t| t >= l1)).then_some(o)
            })
            .collect();
        let text = self.delete_lines(l1, l2);
        let n = l2 + 1 - l1;
        let left = titles
            .into_iter()
            .map(|(o, t)| (o, if t < l1 { Some(t) } else if t > l2 { Some(t - n) } else { None }))
            .collect();
        self.settle(&left);
        self.set_clip(text, tags);
    }

    /// Put the clip's whole lines above or below line `l`. They take the tag
    /// of the line above (§5.2), except the lines of a nested block cut with
    /// its title line and no longer in the buffer: those go back with their
    /// own tag, so the block moves.
    fn put_clip_lines(&mut self, l: usize, clip: &Clip, below: bool) {
        let titles = self.titles();
        let first = if below {
            let body = clip.text.strip_suffix('\n').unwrap_or(&clip.text);
            self.insert(Pos::new(l, self.len(l)), &format!("\n{}", body));
            l + 1
        } else {
            self.insert(Pos::new(l, 0), &clip.text);
            l
        };
        let n = clip.tags.len();
        if n == 0 || clip.text.matches('\n').count() != n || first + n > self.buf.lines.len() {
            return;
        }
        let present: BTreeSet<Owner> = self
            .buf
            .lines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i < first || *i >= first + n)
            .map(|(_, x)| x.owner)
            .collect();
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
    /// sits in: `retag_strays`, then each nested block's embed goes to the
    /// block its title line sits in (`EditBuffer::reparent`).
    fn settle(&mut self, titles: &BTreeMap<Owner, Option<usize>>) {
        self.retag_strays(titles);
        let now = self.titles();
        self.buf.reparent(&now);
    }

    /// A line of a nested block that is no longer contiguous with the
    /// block's title line (at `titles`, or gone) is re-tagged to the block it
    /// now sits in, that of the line above (§5.2).
    fn retag_strays(&mut self, titles: &BTreeMap<Owner, Option<usize>>) {
        let n = self.buf.lines.len();
        for (&o, &title) in titles {
            if !self.nested(o) {
                continue;
            }
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

    /// The block each block's embed sits in.
    fn parents(&self) -> BTreeMap<Owner, Option<Owner>> {
        self.buf.owners.iter().map(|(&o, i)| (o, i.parent)).collect()
    }

    /// What splice writes for each block (§5.2): its own lines, and the
    /// first line of each block nested in it, where that block's embed goes.
    fn splice_views(
        lines: &[EditLine],
        parents: &BTreeMap<Owner, Option<Owner>>,
    ) -> BTreeMap<Owner, Vec<(Owner, String)>> {
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

    // ------------------------------------------------------------ clip

    /// The blocks whose title lines the clip holds are in transit until
    /// they are pasted back or the clip is replaced: the buffer holds them,
    /// so a save meanwhile does not delete them (§5.2).
    fn set_clip(&mut self, text: String, tags: Vec<Option<Owner>>) {
        self.buf.hold(tags.iter().flatten().copied().collect());
        self.clip = Some(Clip { text, tags });
    }

    /// Leaving the editor: a block whose title line was cut and not pasted
    /// back is deleted now (§5.2).
    pub fn release_clip(&mut self) {
        self.clip = None;
        self.buf.hold(Vec::new());
    }

    // ------------------------------------------------------------ undo

    fn checkpoint(&mut self) {
        self.redo.clear();
        self.undo.push(Snap { lines: self.buf.lines.clone(), parents: self.parents() });
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        let cur = self.restore(s);
        self.redo.push(cur);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        let cur = self.restore(s);
        self.undo.push(cur);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Put a snapshot back; every block whose text differs is dirty again,
    /// and so is every block a nested block moved in.
    fn restore(&mut self, s: Snap) -> Snap {
        self.typing = None;
        let cur = Snap { lines: std::mem::take(&mut self.buf.lines), parents: self.parents() };
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
        cur
    }
}

/// The position of character `i` of `text`.
fn pos_of(text: &[char], i: usize) -> Pos {
    let before = &text[..i.min(text.len())];
    let line = before.iter().filter(|&&c| c == '\n').count();
    let col = before.iter().rev().take_while(|&&c| c != '\n').count();
    Pos::new(line, col)
}

fn order(a: Pos, b: Pos) -> (Pos, Pos) {
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
