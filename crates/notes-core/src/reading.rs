//! The reading pane's document: the resolved render with a map from line
//! numbers to nodes, so keys can act on "the node under the cursor" (§10.4).

use crate::parse::{Kind, TaskState};
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
    build_node(vault, root, 1, 0, &mut lines, &mut refs, &mut Vec::new());
    ReadingDoc { lines, refs }
}

#[allow(clippy::too_many_arguments)]
fn build_node(
    vault: &Vault,
    r: NRef,
    dlevel: usize,
    dindent: usize,
    lines: &mut Vec<String>,
    refs: &mut Vec<LineRef>,
    seen: &mut Vec<NRef>,
) {
    if seen.contains(&r) {
        return;
    }
    seen.push(r);
    let n = vault.tree.node(r);
    match n.kind {
        Kind::Root => {}
        Kind::Section => {
            let mut s = " ".repeat(dindent);
            s.push_str(&"#".repeat(dlevel.max(1)));
            s.push(' ');
            if !n.is_block() {
                checkbox(n.task, &mut s);
            }
            s.push_str(n.title.trim_end());
            lines.push(s);
            refs.push(LineRef::Title(r));
        }
        Kind::Item => {
            let mut s = " ".repeat(dindent);
            s.push_str("- ");
            if !n.is_block() {
                checkbox(n.task, &mut s);
            }
            s.push_str(n.title.trim_end());
            lines.push(s);
            refs.push(LineRef::Title(r));
        }
    }
    // body
    let body = n.body_lines(vault.tree.text_of(r));
    let mut body: Vec<&str> = body;
    while body.first().map(|l| l.trim().is_empty()) == Some(true) {
        body.remove(0);
    }
    while body.last().map(|l| l.trim().is_empty()) == Some(true) {
        body.pop();
    }
    if !body.is_empty() && n.kind != Kind::Root {
        lines.push(String::new());
        refs.push(LineRef::Blank);
    }
    let dedent_by = vault.tree.indent(r);
    for l in &body {
        lines.push(format!("{}{}", " ".repeat(dindent), dedent(l, dedent_by)));
        refs.push(LineRef::Body(r));
    }
    // children
    let children = vault.tree.resolved_children(r);
    let mut first = true;
    for c in children {
        let cn = vault.tree.node(c);
        if cn.kind == Kind::Root {
            continue;
        }
        if first {
            if !body.is_empty() || n.kind != Kind::Root {
                lines.push(String::new());
                refs.push(LineRef::Blank);
            }
            first = false;
        } else if cn.kind == Kind::Section {
            lines.push(String::new());
            refs.push(LineRef::Blank);
        }
        let cindent = dindent + vault.tree.indent(c).saturating_sub(vault.tree.indent(r));
        let clevel = match cn.kind {
            Kind::Section => dlevel + 1,
            _ => dlevel,
        };
        let target = vault.tree.resolved_child(c);
        if target != c {
            build_node(vault, target, clevel, cindent, lines, refs, seen);
        } else if cn.is_embed() {
            lines.push(format!(
                "{}![[{}]]",
                " ".repeat(cindent),
                cn.embed.as_ref().unwrap()
            ));
            refs.push(LineRef::Embed(c));
        } else {
            build_node(vault, c, clevel, cindent, lines, refs, seen);
        }
    }
    seen.pop();
}

fn checkbox(task: Option<TaskState>, out: &mut String) {
    match task {
        Some(TaskState::Open) => out.push_str("[ ] "),
        Some(TaskState::Done) => out.push_str("[x] "),
        None => {}
    }
}

fn dedent(line: &str, cols: usize) -> &str {
    if cols == 0 {
        return line;
    }
    let mut removed = 0;
    for (i, ch) in line.char_indices() {
        if removed >= cols {
            return &line[i..];
        }
        match ch {
            ' ' => removed += 1,
            '\t' => removed += 4,
            _ => return &line[i..],
        }
    }
    ""
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
