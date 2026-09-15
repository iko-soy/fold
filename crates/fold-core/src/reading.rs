//! The reading pane's document: the resolved render with a map from line
//! numbers to nodes, so keys can act on "the node under the cursor" (§10.4).

use crate::render::{render_lines, LineKind};
use crate::tree::NRef;
use crate::vault::Vault;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRef {
    Title(NRef),
    Body(NRef),
    Embed(NRef),
    Blank,
}

pub struct ReadingDoc {
    pub lines: Vec<String>,
    pub refs: Vec<LineRef>,
}

/// Build the reading document for a node: `render(node, 1, true)` with a
/// per-line node map.
pub fn build(vault: &Vault, root: NRef) -> ReadingDoc {
    let mut lines = Vec::new();
    let mut refs = Vec::new();
    for l in render_lines(&vault.tree, root, 1, true) {
        refs.push(match l.kind {
            LineKind::Title => LineRef::Title(l.node),
            LineKind::Body => LineRef::Body(l.node),
            LineKind::Embed => LineRef::Embed(l.node),
            LineKind::Blank => LineRef::Blank,
        });
        lines.push(l.text);
    }
    ReadingDoc { lines, refs }
}

/// The nearest node at-or-above a line (blank lines belong to whatever is
/// above them).
pub fn node_at(doc: &ReadingDoc, line: usize) -> Option<LineRef> {
    let mut i = line.min(doc.refs.len().saturating_sub(1));
    loop {
        match doc.refs.get(i) {
            Some(LineRef::Blank) => {
                if i == 0 {
                    return None;
                }
                i -= 1;
            }
            Some(r) => return Some(*r),
            None => return None,
        }
    }
}
