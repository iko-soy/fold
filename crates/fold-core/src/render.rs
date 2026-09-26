//! `render(node, base, resolve_blocks) → String` (§5.1).
//!
//! One walker produces the lines; `render`, the reading pane (§10.4) and the
//! editing buffer (§5.2) all consume it, so what the user reads, what they
//! edit and what splice writes back are the same text.

use crate::parse::{Content, Kind, Span, TaskState};
use crate::tree::{NRef, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Title,
    Body,
    /// An embed kept as `![[id]]` (unresolved render, or a broken embed).
    Embed,
    Blank,
}

/// One rendered line and where it came from.
#[derive(Debug, Clone)]
pub struct RLine {
    pub text: String,
    pub kind: LineKind,
    /// The node whose title, body or embed this line is.
    pub node: NRef,
    /// The block that owns the line: the nearest block at or above it in the
    /// resolved walk, or the render root when there is none (§5.2).
    pub owner: NRef,
    /// The owner in effect where `owner` was entered (itself for the
    /// render root's owner).
    pub outer: NRef,
    /// Display level and indent of `node`.
    pub level: usize,
    pub indent: usize,
}

/// Render the node and its subtree as a standalone Markdown document.
///
/// With `resolve_blocks = false` this is the on-disk spelling (embeds kept);
/// with `true` every nested block is inlined and no frontmatter, id or file
/// boundary appears.
pub fn render(tree: &Tree, node: NRef, base: usize, resolve_blocks: bool) -> String {
    let mut out = String::new();
    // Frontmatter: only for a block rendered at base 1 unresolved (§5.1.1).
    if !resolve_blocks && base == 1 {
        if let Some(b) = &tree.node(node).block {
            if let Some(fs) = b.frontmatter_span {
                out.push_str(fs.text(tree.text_of(node)));
            } else if !b.frontmatter_raw.is_empty() {
                out.push_str("---\n");
                out.push_str(&b.frontmatter_raw);
                out.push_str("---\n\n");
            }
        }
    }
    for l in render_lines(tree, node, base, resolve_blocks) {
        out.push_str(&l.text);
        out.push('\n');
    }
    // canonical: exactly one trailing newline
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// The lines of `render(node, base, resolve_blocks)` (without frontmatter
/// and trailing blank lines), each tagged with its node and owning block.
///
/// Bodies are verbatim, blank lines included, so the file's own spacing is
/// reproduced. Sections are re-levelled by `base − level(node)` and
/// everything re-indented by `−indent(node)` (§5.1.4); the implicit Root
/// renders its top-level nodes at `base`.
pub fn render_lines(tree: &Tree, node: NRef, base: usize, resolve_blocks: bool) -> Vec<RLine> {
    let dlevel = if tree.node(node).kind == Kind::Root {
        base.saturating_sub(1)
    } else {
        base
    };
    let mut out = Vec::new();
    let mut w = Walk {
        tree,
        resolve: resolve_blocks,
        out: &mut out,
        seen: Vec::new(),
    };
    w.node(node, dlevel, 0, node, node);
    while out.last().map(|l| l.kind == LineKind::Blank) == Some(true) {
        out.pop();
    }
    out
}

struct Walk<'a> {
    tree: &'a Tree,
    resolve: bool,
    out: &'a mut Vec<RLine>,
    seen: Vec<NRef>,
}

impl Walk<'_> {
    /// Emit node `r` at display position (`dlevel`, `dindent`): title line,
    /// body, children in document order with any tail text between them.
    fn node(&mut self, r: NRef, dlevel: usize, dindent: usize, owner: NRef, outer: NRef) {
        if self.seen.contains(&r) {
            return; // cycle guard
        }
        self.seen.push(r);
        let tree = self.tree;
        let n = tree.node(r);
        let (owner, outer) = if n.is_block() && r != owner {
            (r, owner)
        } else {
            (owner, outer)
        };
        let title = match n.kind {
            Kind::Root => None,
            _ if n.is_embed() => Some((embed_line(n, dlevel, dindent), LineKind::Embed)),
            Kind::Section => {
                let mut s = " ".repeat(dindent);
                s.push_str(&"#".repeat(dlevel.max(1)));
                s.push(' ');
                // A block's task state is frontmatter-only (§4.9): no
                // checkbox on its own title line.
                if !n.is_block() {
                    push_checkbox(n.task, &mut s);
                }
                s.push_str(&n.title);
                Some((s.trim_end().to_string(), LineKind::Title))
            }
            Kind::Item => {
                let mut s = " ".repeat(dindent);
                s.push_str("- ");
                if !n.is_block() {
                    push_checkbox(n.task, &mut s);
                }
                s.push_str(&n.title);
                Some((s.trim_end().to_string(), LineKind::Title))
            }
        };
        if let Some((text, kind)) = title {
            self.push(text, kind, r, owner, outer, dlevel, dindent);
        }
        for c in &n.content {
            match *c {
                Content::Text(sp) => self.span_lines(r, sp, dlevel, dindent, owner, outer),
                Content::Node(c) => self.child(r, (r.0, c), dlevel, dindent, owner, outer),
            }
        }
        self.seen.pop();
    }

    fn child(&mut self, r: NRef, c: NRef, dlevel: usize, dindent: usize, owner: NRef, outer: NRef) {
        let tree = self.tree;
        let cn = tree.node(c);
        // Display position relative to the parent, from positions in the
        // parent's file (an embed's target sits at the embed's position).
        let cindent = dindent + tree.indent(c).saturating_sub(tree.indent(r));
        if self.resolve && cn.is_embed() {
            let t = tree.resolved_child(c);
            if t != c {
                // A section at this position is one below the parent; an
                // item shares its enclosing section's level.
                let tlevel = if tree.node(t).kind == Kind::Section
                    || tree.node(r).kind == Kind::Root
                {
                    dlevel + 1
                } else {
                    dlevel
                };
                self.node(t, tlevel, cindent, owner, outer);
                // the embed line's own blank separators (and any lines
                // wrongly placed under it) stay in the parent
                for cc in &cn.content {
                    match *cc {
                        Content::Text(sp) => {
                            self.span_lines(c, sp, dlevel, cindent, owner, outer)
                        }
                        Content::Node(k) => self.child(c, (c.0, k), dlevel, cindent, owner, outer),
                    }
                }
                return;
            }
        }
        let clevel = (dlevel as isize + tree.level(c) as isize - tree.level(r) as isize).max(0);
        self.node(c, clevel as usize, cindent, owner, outer);
    }

    /// Body lines of a span, verbatim: dedented by the node's written indent
    /// (§5.1.3) and re-indented to its display indent. Blank lines stay empty.
    fn span_lines(
        &mut self,
        r: NRef,
        span: Span,
        dlevel: usize,
        dindent: usize,
        owner: NRef,
        outer: NRef,
    ) {
        if span.start >= span.end {
            return;
        }
        let tree = self.tree;
        let cols = tree.node(r).indent;
        for l in tree.text_of(r)[span.start..span.end].split_inclusive('\n') {
            let l = l.strip_suffix('\n').unwrap_or(l);
            let l = l.strip_suffix('\r').unwrap_or(l);
            if l.trim().is_empty() {
                self.push(String::new(), LineKind::Blank, r, owner, outer, dlevel, dindent);
            } else {
                let text = format!("{}{}", " ".repeat(dindent), dedent(l, cols));
                self.push(text, LineKind::Body, r, owner, outer, dlevel, dindent);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        text: String,
        kind: LineKind,
        node: NRef,
        owner: NRef,
        outer: NRef,
        level: usize,
        indent: usize,
    ) {
        self.out.push(RLine {
            text,
            kind,
            node,
            owner,
            outer,
            level,
            indent,
        });
    }
}

/// An embed line at a display position: a heading embed (`## ![[id]]`) for
/// a section-position embed, a bare `![[id]]` for an item position (§4.7).
pub fn embed_line(n: &crate::parse::Node, dlevel: usize, dindent: usize) -> String {
    let id = n.embed.as_ref().expect("embed node");
    match n.kind {
        Kind::Section => format!("{}{} ![[{}]]", " ".repeat(dindent), "#".repeat(dlevel.max(1)), id),
        _ => format!("{}![[{}]]", " ".repeat(dindent), id),
    }
}

fn push_checkbox(task: Option<TaskState>, out: &mut String) {
    match task {
        Some(TaskState::Open) => out.push_str("[ ] "),
        Some(TaskState::Done) => out.push_str("[x] "),
        None => {}
    }
}

/// Remove up to `cols` columns of leading indentation (a tab counts four).
pub fn dedent(line: &str, cols: usize) -> &str {
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
