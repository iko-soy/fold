//! The editor behind a platform text field (§10.6).
//!
//! A text field hands over its whole text after every change. `apply`
//! finds the one span that changed and replays it on the tagged buffer
//! through the text operations the TUI's editor uses too
//! (`fold_core::edit`): every line keeps or inherits the tag of the block
//! that owns it (§5.2), whole lines cut with a nested block's title line
//! hold the block in transit, and pasting those lines back puts the block
//! where they land.

use fold_core::edit::{EditBuffer, Owner, Pos, Snapshot};
use std::time::{Duration, Instant};

/// Whole lines cut in this editor, and the block of each line whose title
/// line was cut with it (§5.2: cut and paste moves its embed).
#[derive(Clone, Debug)]
struct Clip {
    text: String,
    tags: Vec<Option<Owner>>,
}

/// Small changes this close together are one undo step (typing).
const TYPING: Duration = Duration::from_secs(2);

pub struct TextEditor {
    pub buf: EditBuffer,
    clip: Option<Clip>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
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
        if a.col == 0 && b.col == 0 && b.line > a.line && self.buf.holds_title(a.line, b.line - 1) {
            let (text, tags) = self.buf.cut_lines(a.line, b.line - 1);
            self.set_clip(text, tags);
        } else {
            self.buf.delete(a, b);
        }
    }

    /// Insert `s` at `at`: the lines the last cut took, put back in front of
    /// a line or after a line break at the end of one, go back with their
    /// tags (§5.2); anything else is typed.
    fn put(&mut self, at: Pos, s: &str) {
        if let Some(clip) = self.clip.as_ref().filter(|c| c.tags.iter().any(Option::is_some)) {
            let body = clip.text.strip_suffix('\n').unwrap_or(&clip.text);
            let above = at.col == 0 && clip.text == s;
            let below = at.col == self.buf.line_len(at.line) && s.strip_prefix('\n') == Some(body);
            if above || below {
                self.buf.put_clip_lines(at.line, &clip.text, &clip.tags, !above);
                return;
            }
        }
        self.buf.insert(at, s);
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
        self.undo.push(self.buf.snapshot());
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

    /// Put a snapshot back (`EditBuffer::restore`): the next change starts
    /// an undo step of its own.
    fn restore(&mut self, s: Snapshot) -> Snapshot {
        self.typing = None;
        self.buf.restore(s)
    }
}

/// The position of character `i` of `text`.
fn pos_of(text: &[char], i: usize) -> Pos {
    let before = &text[..i.min(text.len())];
    let line = before.iter().filter(|&&c| c == '\n').count();
    let col = before.iter().rev().take_while(|&&c| c != '\n').count();
    Pos::new(line, col)
}
