//! Editing the buffer's text (§5.2, §10.6): positions in characters, and
//! every change made by the rules that keep each line with its block, so
//! each block is written back with its own lines however the text was
//! typed, cut or pasted. The TUI's editor and the app's text field both
//! edit through these; a cursor, a keymap and a clipboard stay theirs.

use super::{EditBuffer, EditLine, Owner};
use std::collections::{BTreeMap, BTreeSet};

/// A place in the text: a line, and a column in characters.
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

/// The text as it was, with the block each block's embed sat in (moving
/// lines can change it): a step of the editor's own undo (§10.6).
#[derive(Clone, Debug)]
pub struct Snapshot {
    lines: Vec<EditLine>,
    parents: BTreeMap<Owner, Option<Owner>>,
}

impl EditBuffer {
    /// How many lines the text has; an empty text has one.
    pub fn line_count(&self) -> usize {
        self.lines.len().max(1)
    }

    pub fn line(&self, l: usize) -> &str {
        self.lines.get(l).map(|x| x.text.as_str()).unwrap_or("")
    }

    /// The length of line `l` in characters.
    pub fn line_len(&self, l: usize) -> usize {
        self.line(l).chars().count()
    }

    /// The text of `[a, b)`.
    pub fn text_between(&self, a: Pos, b: Pos) -> String {
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
        if self.lines.is_empty() {
            self.lines.push(EditLine { text: String::new(), owner: Owner { file: 0, block_ord: 0 } });
        }
        let line = self.line(at.line).to_string();
        let b = byte(&line, at.col);
        let (pre, post) = line.split_at(b);
        let parts: Vec<&str> = s.split('\n').collect();
        if parts.len() == 1 {
            self.set_line(at.line, format!("{}{}{}", pre, s, post));
            return Pos::new(at.line, at.col + s.chars().count());
        }
        let owner = self.lines[at.line].owner;
        let block_title = self.nested(owner) && !self.lines[..at.line].iter().any(|l| l.owner == owner);
        if at.col == 0 && at.line > 0 && block_title && !(parts[0].is_empty() && post.is_empty()) {
            // in front of a nested block's title line, whole lines go in
            // after the line above, taking its tag: they are not the block's
            // text; the title line keeps its own tag and only gains the last
            // part
            let mut l = at.line - 1;
            for p in &parts[..parts.len() - 1] {
                self.insert_line(l, p.to_string());
                l += 1;
            }
            let last = parts[parts.len() - 1];
            self.set_line(l + 1, format!("{}{}", last, post));
            return Pos::new(l + 1, last.chars().count());
        }
        self.set_line(at.line, format!("{}{}", pre, parts[0]));
        let mut l = at.line;
        for p in &parts[1..parts.len() - 1] {
            self.insert_line(l, p.to_string());
            l += 1;
        }
        let last = parts[parts.len() - 1];
        self.insert_line(l, format!("{}{}", last, post));
        Pos::new(l + 1, last.chars().count())
    }

    /// Delete `[a, b)`; returns what was deleted. A line deleted whole takes
    /// its tag with it (§5.2): what is left of a joined line keeps the tag of
    /// line `a`, unless the range starts at column 0 and leaves some of line
    /// `b`, or ends at its start (short of the end of the text): then line
    /// `b` is what is left. A range from column 0 through the end of line
    /// `b` deletes lines `a` to `b` whole, and the empty line left keeps the
    /// tag of a block whose title line was not among them (`kept`), so a
    /// nested block's title line deleted that way deletes the block,
    /// whichever end of the range it is at.
    pub fn delete(&mut self, a: Pos, b: Pos) -> String {
        let (a, mut b) = order(a, b);
        if b.line >= self.line_count() {
            b = Pos::new(self.line_count() - 1, self.line_len(self.line_count() - 1));
        }
        let gone = self.text_between(a, b);
        if gone.is_empty() {
            return gone;
        }
        let first = self.line(a.line).to_string();
        let last = self.line(b.line).to_string();
        let rest = &last[byte(&last, b.col)..];
        if a.col == 0 && a.line < b.line && (!rest.is_empty() || (b.col == 0 && b.line + 1 < self.line_count())) {
            for l in (a.line..b.line).rev() {
                self.delete_line(l);
            }
            self.set_line(a.line, rest.to_string());
            return gone;
        }
        let whole = (a.col == 0 && a.line < b.line).then(|| self.kept(a.line, b.line));
        let joined = format!("{}{}", &first[..byte(&first, a.col)], rest);
        for l in (a.line + 1..=b.line).rev() {
            self.delete_line(l);
        }
        self.set_line(a.line, joined);
        if let Some(o) = whole {
            let was = std::mem::replace(&mut self.lines[a.line].owner, o);
            self.mark_dirty(was);
            self.mark_dirty(o);
        }
        gone
    }

    /// The block the empty line left where lines `l1..=l2` are deleted whole
    /// belongs to: one whose title line (§5.2) is not among them — line
    /// `l1`'s, else line `l2`'s, else the block they sit in.
    fn kept(&self, l1: usize, l2: usize) -> Owner {
        let titles = self.titles();
        let gone = |o: Owner| self.nested(o) && titles.get(&o).is_some_and(|t| (l1..=l2).contains(t));
        let (mut o, last) = (self.lines[l1].owner, self.lines[l2].owner);
        if gone(o) && !gone(last) {
            return last;
        }
        while gone(o) {
            match self.owners.get(&o).and_then(|i| i.parent) {
                Some(p) => o = p,
                None => break,
            }
        }
        o
    }

    /// Delete whole lines `l1..=l2`; returns them, each ending in `\n`.
    fn delete_lines(&mut self, l1: usize, l2: usize) -> String {
        let l2 = l2.min(self.line_count() - 1);
        let mut gone = String::new();
        for l in l1..=l2 {
            gone.push_str(self.line(l));
            gone.push('\n');
        }
        if l1 == 0 && l2 + 1 >= self.line_count() {
            // the text keeps one (empty) line
            for l in (1..self.line_count()).rev() {
                self.delete_line(l);
            }
            self.set_line(0, String::new());
        } else {
            for l in (l1..=l2).rev() {
                self.delete_line(l);
            }
        }
        gone
    }

    /// Put whole lines (text ending in `\n`) below or above line `l`;
    /// returns the first new line.
    pub fn put_lines(&mut self, l: usize, text: &str, below: bool) -> usize {
        let body = text.strip_suffix('\n').unwrap_or(text);
        if below {
            let end = Pos::new(l, self.line_len(l));
            self.insert(end, &format!("\n{}", body));
            l + 1
        } else {
            self.insert(Pos::new(l, 0), &format!("{}\n", body));
            l
        }
    }

    /// Move whole lines `l1..=l2` one line down or up, past their neighbour;
    /// false where there is none. Each line keeps its tag (§5.2), so moving
    /// a nested block's title line moves its embed.
    pub fn move_lines(&mut self, l1: usize, l2: usize, down: bool) -> bool {
        let (lo, hi) = if down { (l1, l2 + 1) } else { (l1.wrapping_sub(1), l2) };
        if (!down && l1 == 0) || hi >= self.lines.len() {
            return false;
        }
        let titles = self.titles();
        if down {
            self.lines[lo..=hi].rotate_right(1);
        } else {
            self.lines[lo..=hi].rotate_left(1);
        }
        for i in lo..=hi {
            let o = self.lines[i].owner;
            self.mark_dirty(o);
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
        self.regroup(&moved);
        true
    }

    /// Cut whole lines `l1..=l2`: their text, each line ending in `\n`, and
    /// for each line the nested block whose title line went with it, if
    /// any, which a clipboard keeps so that pasting the lines moves the
    /// block (§5.2; `hold` it meanwhile). Its lines left behind go to the
    /// block they now sit in.
    pub fn cut_lines(&mut self, l1: usize, l2: usize) -> (String, Vec<Option<Owner>>) {
        let l2 = l2.min(self.line_count() - 1);
        let titles = self.titles();
        let tags = (l1..=l2)
            .map(|l| {
                let o = self.lines.get(l)?.owner;
                (self.nested(o) && titles.get(&o).is_some_and(|&t| t >= l1)).then_some(o)
            })
            .collect();
        let text = self.delete_lines(l1, l2);
        let n = l2 + 1 - l1;
        let left = titles
            .into_iter()
            .map(|(o, t)| (o, if t < l1 { Some(t) } else if t > l2 { Some(t - n) } else { None }))
            .collect();
        self.regroup(&left);
        (text, tags)
    }

    /// Put cut lines (`cut_lines`' text and tags) above or below line `l`;
    /// returns the first new line. They take the tag of the line above
    /// (§5.2), except the lines of a nested block cut with its title line and
    /// no longer in the text: those go back with their own tag, so the block
    /// moves. A tag the buffer does not know (it was re-rendered since the
    /// cut, §11.2) names no block to write the line to, and is not put back.
    pub fn put_clip_lines(&mut self, l: usize, text: &str, tags: &[Option<Owner>], below: bool) -> usize {
        let titles = self.titles();
        let first = self.put_lines(l, text, below);
        let n = tags.len();
        if n == 0 || text.matches('\n').count() != n || first + n > self.lines.len() {
            return first;
        }
        let present: BTreeSet<Owner> =
            self.lines.iter().enumerate().filter(|(i, _)| *i < first || *i >= first + n).map(|(_, x)| x.owner).collect();
        let mut back: BTreeMap<Owner, Option<usize>> = BTreeMap::new();
        for (i, tag) in tags.iter().enumerate() {
            let Some(o) = tag.filter(|o| !present.contains(o) && self.owners.contains_key(o)) else { continue };
            let was = std::mem::replace(&mut self.lines[first + i].owner, o);
            self.mark_dirty(was);
            self.touch(o);
            back.entry(o).or_insert(Some(first + i));
        }
        if !back.is_empty() {
            for (o, t) in titles {
                back.insert(o, Some(if t >= first { t + n } else { t }));
            }
            self.regroup(&back);
        }
        first
    }

    /// Whether lines `l1..=l2` hold a nested block's title line (§5.2).
    pub fn holds_title(&self, l1: usize, l2: usize) -> bool {
        let titles = self.titles();
        (l1..=l2.min(self.line_count() - 1)).any(|l| {
            let o = self.lines[l].owner;
            self.nested(o) && titles.get(&o) == Some(&l)
        })
    }

    /// The text as it is now, for the editor's own undo.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot { lines: self.lines.clone(), parents: self.parents() }
    }

    /// Put a snapshot back, returning the text as it was before: every
    /// block whose text differs is dirty again, and so is every block a
    /// nested block moved in.
    pub fn restore(&mut self, s: Snapshot) -> Snapshot {
        let cur = Snapshot { lines: std::mem::take(&mut self.lines), parents: self.parents() };
        let was = splice_views(&cur.lines, &cur.parents);
        let now = splice_views(&s.lines, &s.parents);
        for o in was.keys().chain(now.keys()) {
            if was.get(o) != now.get(o) {
                self.mark_dirty(*o);
            }
        }
        self.lines = s.lines;
        for (o, p) in s.parents {
            if let Some(i) = self.owners.get_mut(&o) {
                i.parent = p;
            }
        }
        cur
    }

    /// The first line of each block: its title line (§5.2).
    fn titles(&self) -> BTreeMap<Owner, usize> {
        let mut t = BTreeMap::new();
        for (i, l) in self.lines.iter().enumerate() {
            t.entry(l.owner).or_insert(i);
        }
        t
    }

    /// Whether block `o` is nested in another, its embed in that block's text.
    fn nested(&self, o: Owner) -> bool {
        self.owners.get(&o).is_some_and(|i| i.parent.is_some())
    }

    /// Whether block `o` is `outer` or nested in it.
    fn within(&self, mut o: Owner, outer: Owner) -> bool {
        while o != outer {
            match self.owners.get(&o).and_then(|i| i.parent) {
                Some(p) => o = p,
                None => return false,
            }
        }
        true
    }

    /// A block changed and moved: it is dirty, and so is the block its embed
    /// sits in (§5.2).
    fn touch(&mut self, o: Owner) {
        self.mark_dirty(o);
        if let Some(p) = self.owners.get(&o).and_then(|i| i.parent) {
            self.mark_dirty(p);
        }
    }

    /// After lines moved with their tags, each goes with the block it now
    /// sits in (§5.2): `retag_strays`, then each nested block's embed goes
    /// to the block its title line sits in, by the rule a save follows too
    /// (`reparent`).
    fn regroup(&mut self, titles: &BTreeMap<Owner, Option<usize>>) {
        self.retag_strays(titles);
        let now = self.titles();
        self.reparent(&now);
    }

    /// A line of a nested block that is no longer contiguous with the
    /// block's title line (at `titles`, or gone) is re-tagged to the block it
    /// now sits in, that of the line above (§5.2).
    fn retag_strays(&mut self, titles: &BTreeMap<Owner, Option<usize>>) {
        let n = self.lines.len();
        for (&o, &title) in titles {
            if !self.nested(o) {
                continue;
            }
            // the title line, then the lines of the block and the blocks in it
            let (t, mut end) = match title {
                Some(t) => (t, t + 1),
                None => (n, n),
            };
            while end < n && self.within(self.lines[end].owner, o) {
                end += 1;
            }
            for i in 1..n {
                if (i < t || i >= end) && self.lines[i].owner == o {
                    let above = self.lines[i - 1].owner;
                    self.lines[i].owner = above;
                    self.mark_dirty(above);
                    self.mark_dirty(o);
                }
            }
        }
    }

    /// The block each block's embed sits in.
    fn parents(&self) -> BTreeMap<Owner, Option<Owner>> {
        self.owners.iter().map(|(&o, i)| (o, i.parent)).collect()
    }
}

/// What splice writes for each block (§5.2): its own lines, and the first
/// line of each block nested in it, where that block's embed goes.
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

/// Two places in text order.
pub fn order(a: Pos, b: Pos) -> (Pos, Pos) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// The byte offset of a character column.
fn byte(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map(|(i, _)| i).unwrap_or(s.len())
}
