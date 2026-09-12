//! `render(node, base, resolve_blocks) → String` (§5.1).

use crate::parse::{Kind, TaskState};
use crate::tree::{NRef, Tree};

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
    let base_level = tree.level(node);
    let base_indent = tree.indent(node);
    render_at(
        tree,
        node,
        base,
        0,
        base_level,
        base_indent,
        resolve_blocks,
        &mut out,
        &mut Vec::new(),
    );
    // canonical: exactly one trailing newline
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Render node `r` at display position (`dlevel`, `dindent`), then its body
/// and children. `base_level`/`base_indent` are the tree position of the
/// render root, used to compute relative positions of descendants.
#[allow(clippy::too_many_arguments)]
fn render_at(
    tree: &Tree,
    r: NRef,
    dlevel: usize,
    dindent: usize,
    base_level: usize,
    base_indent: usize,
    resolve: bool,
    out: &mut String,
    seen: &mut Vec<NRef>,
) {
    if seen.contains(&r) {
        return; // cycle guard
    }
    seen.push(r);
    let n = tree.node(r);
    let is_block_root = n.is_block();
    match n.kind {
        Kind::Root => {}
        Kind::Section => {
            push_indent(out, dindent);
            for _ in 0..dlevel.max(1) {
                out.push('#');
            }
            out.push(' ');
            // A block's task state is frontmatter-only (§4.9): no checkbox
            // on its own title line.
            if !is_block_root {
                push_checkbox(n.task, out);
            }
            out.push_str(n.title.trim_end());
            out.push('\n');
        }
        Kind::Item => {
            push_indent(out, dindent);
            out.push_str("- ");
            if !is_block_root {
                push_checkbox(n.task, out);
            }
            out.push_str(n.title.trim_end());
            out.push('\n');
        }
    }
    // Body, verbatim, dedented by the node's original indent (§5.1.3) and
    // re-indented to the display position. For a block root rendered
    // unresolved the body's leading blank line is kept (it is the file's
    // separator after the frontmatter/title); otherwise bodies are separated
    // from the title by one canonical blank line.
    let body = trimmed_body_keep(tree, r, is_block_root && !resolve);
    let has_trailing_sep = body_sep_blank(tree, r);
    if !body.is_empty() && n.kind != Kind::Root && !is_block_root {
        out.push('\n');
    }
    let body_dedent = tree.indent(r);
    for l in &body {
        push_indent(out, dindent);
        out.push_str(dedent(l, body_dedent));
        out.push('\n');
    }
    // Children in document order.
    let children = tree.resolved_children(r);
    let all_item_children = !children.is_empty()
        && children
            .iter()
            .all(|&k| tree.node(k).kind == Kind::Item);
    let mut prev: Option<NRef> = None;
    for c in children {
        let cn = tree.node(c);
        if cn.kind == Kind::Root {
            continue;
        }
        let is_first = prev.is_none();
        let prev_was_section = prev
            .map(|p| tree.node(p).kind == Kind::Section)
            .unwrap_or(false);
        if is_first {
            if !body.is_empty() || has_trailing_sep || (n.kind != Kind::Root && !all_item_children)
            {
                out.push('\n');
            }
        } else if cn.kind == Kind::Section || prev_was_section {
            out.push('\n');
        }
        // Display position of the child: relative to this node.
        let cindent = dindent + tree.indent(c).saturating_sub(tree.indent(r));
        let clevel = match cn.kind {
            Kind::Section => dlevel + section_steps(tree, r, c),
            _ => dlevel,
        };
        if cn.is_embed() {
            let id = cn.embed.clone().unwrap();
            let target = if resolve { tree.resolved_child(c) } else { c };
            if target == c {
                push_indent(out, cindent);
                out.push_str("![[");
                out.push_str(id.as_str());
                out.push_str("]]\n");
            } else {
                render_at(
                    tree, target, clevel, cindent, base_level, base_indent, resolve, out, seen,
                );
            }
            prev = Some(c);
            continue;
        }
        render_at(
            tree, c, clevel, cindent, base_level, base_indent, resolve, out, seen,
        );
        prev = Some(c);
    }
    seen.pop();
}

/// Section steps between a node and a descendant (the number of section
/// boundaries crossed, counting the descendant itself if it is one).
fn section_steps(tree: &Tree, anc: NRef, desc: NRef) -> usize {
    let mut n = 0usize;
    let mut cur = Some(desc);
    while let Some(c) = cur {
        if c == anc {
            break;
        }
        if tree.node(c).kind == Kind::Section {
            n += 1;
        }
        cur = tree.node(c).parent.map(|p| (c.0, p));
    }
    n.max(1)
}

/// True if the node's body region ends with (or consists of) a blank line —
/// i.e. the file had a blank separating the body (or title) from the first
/// child.
fn body_sep_blank(tree: &Tree, r: NRef) -> bool {
    let n = tree.node(r);
    if n.body_span.end <= n.body_span.start {
        return false;
    }
    let region = &tree.text_of(r)[n.body_span.start..n.body_span.end];
    region
        .split_inclusive('\n')
        .map(|l| l.strip_suffix('\n').unwrap_or(l))
        .last()
        .map(|l| l.trim().is_empty())
        .unwrap_or(false)
}

fn trimmed_body_keep<'a>(tree: &'a Tree, r: NRef, keep_leading: bool) -> Vec<&'a str> {    let mut lines = tree.node(r).body_lines(tree.text_of(r));
    if !keep_leading {
        while lines.first().map(|l| l.trim().is_empty()) == Some(true) {
            lines.remove(0);
        }
    } else {
        // keep at most one leading blank (the file's separator)
        while lines.len() > 1 && lines[0].trim().is_empty() && lines[1].trim().is_empty() {
            lines.remove(0);
        }
    }
    while lines.last().map(|l| l.trim().is_empty()) == Some(true) {
        lines.pop();
    }
    lines
}

fn push_checkbox(task: Option<TaskState>, out: &mut String) {
    match task {
        Some(TaskState::Open) => out.push_str("[ ] "),
        Some(TaskState::Done) => out.push_str("[x] "),
        None => {}
    }
}

fn push_indent(out: &mut String, cols: usize) {
    for _ in 0..cols {
        out.push(' ');
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
