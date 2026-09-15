//! Structural operations on the tree (§5.2, §6, §7, §8). Every op mutates
//! files through the vault and returns an inverse for the undo log (§10.11).

use crate::ident::{slug, Id};
use crate::parse::{Kind, Span, TaskState};
use crate::render::render;
use crate::tree::NRef;
use crate::vault::Vault;

/// An inverse snapshot for undo (§10.11): file path → prior text. Files that
/// did not exist are recorded as None.
#[derive(Debug, Clone)]
pub struct Inverse {
    pub files: Vec<(String, Option<String>)>,
    pub description: String,
}

impl Inverse {
    pub fn apply(self, vault: &mut Vault) -> std::io::Result<()> {
        for (path, text) in self.files {
            match text {
                Some(t) => {
                    let full = vault.dir.join(&path);
                    // untouched files keep their bytes and mtime (§11.1)
                    if std::fs::read_to_string(&full).ok().as_deref() != Some(t.as_str()) {
                        crate::vault::atomic_write(&full, &t)?;
                    }
                }
                None => {
                    let _ = std::fs::remove_file(vault.dir.join(&path));
                }
            }
        }
        vault.reload()
    }
}

fn snapshot(vault: &Vault, description: &str) -> Inverse {
    Inverse {
        files: vault
            .tree
            .files
            .iter()
            .map(|f| (f.path.clone(), Some(f.text.clone())))
            .collect(),
        description: description.into(),
    }
}

fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

// ---------------------------------------------------------------- capture

/// Append an item under today's day section of `Inbox` (§7).
pub fn capture(vault: &mut Vault, text: &str, task: bool) -> std::io::Result<NRef> {
    let inv = snapshot(vault, "capture");
    let r = capture_inner(vault, text, task, None)?;
    let _ = inv;
    Ok(r)
}

pub fn capture_to(
    vault: &mut Vault,
    text: &str,
    task: bool,
    target: NRef,
) -> std::io::Result<NRef> {
    capture_inner(vault, text, task, Some(target))
}

fn capture_inner(
    vault: &mut Vault,
    text: &str,
    task: bool,
    target: Option<NRef>,
) -> std::io::Result<NRef> {
    let dest = match target {
        Some(t) => t,
        None => {
            let inbox = find_or_create_inbox(vault)?;
            find_or_create_day(vault, inbox)?
        }
    };
    // The first line is the title; further lines are the item's body, so a
    // captured document (`capture < file.md`) nests under the item instead
    // of injecting structure beside it (§4.1): its headings are pushed below
    // the destination's level and every line is indented under the item.
    let text = text.trim();
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let title = if task {
        format!("[ ] {}", first.trim())
    } else {
        first.trim().to_string()
    };
    let mut line = format!("- {}", title);
    if !rest.trim().is_empty() {
        let dn = vault.tree.node(dest);
        let item_indent = if dn.kind == Kind::Item {
            vault.tree.indent(dest) + 2
        } else {
            vault.tree.indent(dest)
        };
        let level = vault.tree.level(dest) as isize;
        let body = shift_lines(rest.trim_end(), level, item_indent as isize + 2);
        line.push('\n');
        line.push_str(body.trim_end_matches('\n'));
    }
    append_child_line(vault, dest, &line, false)
}

fn find_or_create_inbox(vault: &mut Vault) -> std::io::Result<NRef> {
    let root = vault.tree.root;
    for c in vault.tree.resolved_children(root) {
        if vault.tree.node(c).title.eq_ignore_ascii_case("inbox") {
            return Ok(c);
        }
    }
    // create `# Inbox` as the last top-level section of root.md (§7)
    append_top_section(vault, "Inbox")
}

fn find_or_create_day(vault: &mut Vault, inbox: NRef) -> std::io::Result<NRef> {
    let day = today();
    for c in vault.tree.resolved_children(inbox) {
        if vault.tree.node(c).title == day {
            return Ok(c);
        }
    }
    // `## <today>` as the last child of Inbox (§7)
    let level = vault.tree.level(inbox) + 1;
    let indent = vault.tree.indent(inbox);
    let heading = format!(
        "{}{} {}",
        " ".repeat(indent),
        "#".repeat(level),
        day
    );
    append_structural_line(vault, inbox, &heading)?;
    vault.reload()?;
    let inbox = vault
        .find_by_path(&vault_inbox_path(vault))
        .ok_or_else(|| io_err("inbox lost after reload"))?;
    for c in vault.tree.resolved_children(inbox) {
        if vault.tree.node(c).title == day {
            return Ok(c);
        }
    }
    Err(io_err("day section not created"))
}

fn vault_inbox_path(vault: &Vault) -> Vec<String> {
    let root = vault.tree.root;
    for c in vault.tree.resolved_children(root) {
        if vault.tree.node(c).title.eq_ignore_ascii_case("inbox") {
            return vault.tree.path(c);
        }
    }
    vec!["Inbox".into()]
}

fn append_top_section(vault: &mut Vault, title: &str) -> std::io::Result<NRef> {
    let root_file = 0;
    let old = vault.tree.files[root_file].text.clone();
    let mut new = old.clone();
    if !new.is_empty() && !new.ends_with("\n\n") {
        if !new.ends_with('\n') {
            new.push('\n');
        }
        new.push('\n');
    }
    new.push_str(&format!("# {}\n", title));
    vault.write_file_text(root_file, &new)?;
    let root = vault.tree.root;
    for c in vault.tree.resolved_children(root) {
        if vault.tree.node(c).title.eq_ignore_ascii_case(title) {
            return Ok(c);
        }
    }
    Err(io_err("section not created"))
}

/// Append a structural line (a heading) as the last child of a node.
fn append_structural_line(vault: &mut Vault, parent: NRef, line: &str) -> std::io::Result<()> {
    let node = vault.tree.node(parent);
    let file = parent.0;
    let text = vault.tree.files[file].text.clone();
    let pos = trimmed_end(&text, node.span);
    insert_at(vault, file, pos, "\n\n", line)
}

/// End of a node's span without its trailing newlines: where a new last
/// child is inserted.
fn trimmed_end(text: &str, span: Span) -> usize {
    let mut pos = span.end.min(text.len());
    while pos > span.start && text.as_bytes()[pos - 1] == b'\n' {
        pos -= 1;
    }
    pos
}

/// Insert `sep` + `body` at `pos`, reusing the newline already there (the
/// one `trimmed_end` stepped back over) so no blank line is doubled.
fn insert_at(
    vault: &mut Vault,
    file: usize,
    pos: usize,
    sep: &str,
    body: &str,
) -> std::io::Result<()> {
    let text = &vault.tree.files[file].text;
    let body = body.trim_end_matches('\n');
    let insertion = if text[pos..].starts_with('\n') {
        format!("{}{}", sep, body)
    } else {
        format!("{}{}\n", sep, body)
    };
    vault.write_span(file, Span { start: pos, end: pos }, &insertion)
}

/// Append an item line as the last child of a node; returns the new node.
///
/// An item cannot follow a section child: it would parse as that section's
/// child. With `section_ok` (the TUI's `N`) the new node is then spelled as a
/// section beside the last one; otherwise (capture) it stays an item and goes
/// after the parent's other items, before its first section.
fn append_child_line(
    vault: &mut Vault,
    parent: NRef,
    line: &str,
    section_ok: bool,
) -> std::io::Result<NRef> {
    let parent_path = vault.tree.path(parent);
    let parent_key = vault.key_of(parent);
    let node = vault.tree.node(parent);
    let file = parent.0;
    // Items under a section sit at the section's own indent; items under an
    // item nest one level deeper (§3.1, §4.10).
    let indent = if node.kind == Kind::Item {
        vault.tree.indent(parent) + 2
    } else {
        vault.tree.indent(parent)
    };
    let text = vault.tree.files[file].text.clone();
    let kids = vault.tree.raw_children(parent);
    let is_section = |r: NRef| vault.tree.node(r).kind == Kind::Section;
    let first_section = kids.iter().position(|&c| is_section(c));
    let index;
    match (first_section, section_ok) {
        (Some(_), true) => {
            // a section sibling of the last section child
            let last = *kids.last().unwrap();
            let last = if is_section(last) {
                last
            } else {
                *kids.iter().rev().find(|&&c| is_section(c)).unwrap()
            };
            let title = line.trim_start().strip_prefix("- ").unwrap_or(line.trim_start());
            let heading = format!(
                "{}{} {}",
                " ".repeat(vault.tree.indent(last)),
                "#".repeat(vault.tree.level(last)),
                title
            );
            let pos = trimmed_end(&text, node.span);
            insert_at(vault, file, pos, "\n\n", &heading)?;
            index = kids.len();
        }
        (Some(fs), false) => {
            // before the first section child, after the items and body
            let line_indented = format!("{}{}", " ".repeat(indent), line);
            let mut pos = vault.tree.node(kids[fs]).span.start;
            while pos >= 2 && &text.as_bytes()[pos - 2..pos] == b"\n\n" {
                pos -= 1;
            }
            let prev_is_item = fs > 0 && !is_section(kids[fs - 1]);
            let sep = if prev_is_item { "" } else { "\n" };
            vault.write_span(
                file,
                Span { start: pos, end: pos },
                &format!("{}{}\n", sep, line_indented),
            )?;
            index = fs;
        }
        (None, _) => {
            let line_indented = format!("{}{}", " ".repeat(indent), line);
            let pos = trimmed_end(&text, node.span);
            // blank line before unless the previous sibling is also an item
            // (tight list)
            let prev_is_item = kids.last().map(|&c| !is_section(c)).unwrap_or(false);
            let sep = if prev_is_item { "\n" } else { "\n\n" };
            insert_at(vault, file, pos, sep, &line_indented)?;
            index = kids.len();
        }
    }
    vault.reload()?;
    let parent = vault
        .find_by_key(&parent_key)
        .or_else(|| vault.find_by_path(&parent_path))
        .ok_or_else(|| io_err("parent lost after reload"))?;
    vault
        .tree
        .resolved_children(parent)
        .get(index)
        .copied()
        .ok_or_else(|| io_err("child not created"))
}

// ---------------------------------------------------------------- tasks

/// Toggle task open/done (§8.1, §8.2).
pub fn toggle_task(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let n = vault.tree.node(r);
    if n.is_block() {
        let new_state = match n.task {
            Some(TaskState::Open) | None => TaskState::Done,
            Some(TaskState::Done) => TaskState::Open,
        };
        set_block_todo(vault, r, Some(new_state))
    } else if n.is_embed() {
        let target = vault.tree.resolved_child(r);
        if target != r {
            return toggle_task(vault, target);
        }
        Ok(())
    } else {
        // checkbox on the title line
        let file = r.0;
        let ts = n.title_span;
        let line = ts.text(&vault.tree.files[file].text).to_string();
        let to = match n.task {
            Some(TaskState::Open) => "[x]",
            Some(TaskState::Done) => "[ ]", // also rewrites `[X]` / `[-]`
            None => return Ok(()), // not a task; `t` makes it one
        };
        let Some((start, _)) = checkbox_span(&line) else {
            return Ok(());
        };
        let mut new_line = line.clone();
        new_line.replace_range(start..start + 3, to);
        vault.write_span(file, ts, &new_line)
    }
}

/// Toggle task-ness itself (§10.3 `t`).
pub fn toggle_taskness(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let n = vault.tree.node(r);
    if n.is_block() {
        if n.task.is_some() {
            set_block_todo(vault, r, None)
        } else {
            set_block_todo(vault, r, Some(TaskState::Open))
        }
    } else if n.is_embed() {
        let target = vault.tree.resolved_child(r);
        if target != r {
            return toggle_taskness(vault, target);
        }
        Ok(())
    } else {
        let file = r.0;
        let ts = n.title_span;
        let line = ts.text(&vault.tree.files[file].text).to_string();
        let new_line = match n.task {
            Some(_) => match checkbox_span(&line) {
                Some((start, end)) => format!("{}{}", &line[..start], &line[end..]),
                None => line.clone(),
            },
            // insert `[ ] ` right after the marker, whatever the indent
            None => match marker_end(&line) {
                Some(idx) => format!("{}[ ] {}", &line[..idx], &line[idx..]),
                None => line.clone(),
            },
        };
        vault.write_span(file, ts, &new_line)
    }
}

/// Byte offset just past a title line's marker: `#…` plus its spaces, or
/// `- ` / `* ` / `+ `, after any indent.
fn marker_end(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    if rest.starts_with('#') {
        let h = rest.bytes().take_while(|&b| b == b'#').count();
        let after = &rest[h..];
        let sp = after.len() - after.trim_start_matches(' ').len();
        if sp == 0 {
            return None;
        }
        Some(indent + h + sp)
    } else if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        Some(indent + 2)
    } else {
        None
    }
}

/// Byte range of the checkbox right after the marker (`[ ]`, `[x]`, `[X]`,
/// `[-]`) plus the space after it (§4.2). Checkbox-like text later in the
/// title is title text.
fn checkbox_span(line: &str) -> Option<(usize, usize)> {
    let me = marker_end(line)?;
    let rest = &line[me..];
    if ["[ ]", "[x]", "[X]", "[-]"].iter().any(|cb| rest.starts_with(cb)) {
        let mut end = me + 3;
        if line[end..].starts_with(' ') {
            end += 1;
        }
        Some((me, end))
    } else {
        None
    }
}

/// Write `todo:`/`done:` for a task block (§8.2).
pub fn set_block_todo(
    vault: &mut Vault,
    r: NRef,
    state: Option<TaskState>,
) -> std::io::Result<()> {
    let file = r.0;
    match state {
        Some(TaskState::Done) => {
            set_frontmatter_key(vault, file, "todo", Some("done"))?;
            set_frontmatter_key(vault, file, "done", Some(&today()))
        }
        Some(TaskState::Open) => {
            set_frontmatter_key(vault, file, "todo", Some("open"))?;
            set_frontmatter_key(vault, file, "done", None)
        }
        None => {
            set_frontmatter_key(vault, file, "todo", None)?;
            set_frontmatter_key(vault, file, "done", None)
        }
    }
}

// ------------------------------------------------------- frontmatter edits

/// Set or remove a top-level frontmatter key, preserving every other line
/// byte for byte (§4.4). If the file has no frontmatter, one is created.
pub fn set_frontmatter_key(
    vault: &mut Vault,
    file: usize,
    key: &str,
    value: Option<&str>,
) -> std::io::Result<()> {
    let f = vault.tree.files[file].clone();
    let root_node = f.nodes[f.root_node].children.get(0).copied();
    let block = root_node.and_then(|rn| f.nodes[rn].block.clone());
    let (mut raw, fm_span) = match &block {
        Some(b) => (b.frontmatter_raw.clone(), b.frontmatter_span),
        None => (String::new(), None),
    };
    // rewrite the line for `key` inside raw
    let mut lines: Vec<String> = raw.split_inclusive('\n').map(|s| s.to_string()).collect();
    if !lines.is_empty() && !lines.last().unwrap().ends_with('\n') {
        // keep as is; we append a newline when needed
    }
    let key_prefix = format!("{}:", key);
    let mut found = false;
    for l in lines.iter_mut() {
        let trimmed = l.trim_end_matches('\n');
        if !trimmed.starts_with(char::is_whitespace) && trimmed.starts_with(&key_prefix) {
            let after = &trimmed[key_prefix.len()..];
            if after.is_empty() || after.starts_with(' ') {
                found = true;
                match value {
                    Some(v) => *l = format!("{}: {}\n", key, v),
                    None => *l = String::new(), // remove the line
                }
            }
        }
    }
    if !found {
        if let Some(v) = value {
            lines.push(format!("{}: {}\n", key, v));
        }
    }
    raw = lines.concat();
    // reassemble frontmatter text
    let fm_text = if raw.is_empty() && block.as_ref().map(|b| b.frontmatter_span.is_none()).unwrap_or(true) {
        String::new()
    } else {
        format!("---\n{}---\n\n", raw)
    };
    let new_text = match fm_span {
        Some(span) => {
            let mut t = String::with_capacity(f.text.len());
            t.push_str(&f.text[..span.start]);
            t.push_str(&fm_text);
            t.push_str(&f.text[span.end..]);
            t
        }
        None => {
            if fm_text.is_empty() {
                f.text.clone()
            } else {
                format!("{}{}", fm_text, f.text)
            }
        }
    };
    vault.write_file_text(file, &new_text)
}

/// All top-level keys of a block's frontmatter, for the property editor.
pub fn frontmatter_lines(vault: &Vault, file: usize) -> Vec<(String, String, bool)> {
    // (key, value, editable) — unknown-structure lines are read-only (§10.6)
    let f = &vault.tree.files[file];
    let Some(&rn) = f.nodes[f.root_node].children.first() else {
        return Vec::new();
    };
    let Some(b) = &f.nodes[rn].block else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for l in b.frontmatter_raw.lines() {
        if l.starts_with(char::is_whitespace) || l.is_empty() || !l.contains(':') {
            out.push((l.to_string(), String::new(), false));
            continue;
        }
        let (k, v) = l.split_once(':').unwrap();
        if crate::parse::is_valid_key(k.trim_end()) {
            out.push((k.trim_end().to_string(), v.trim().to_string(), true));
        } else {
            out.push((l.to_string(), String::new(), false));
        }
    }
    out
}

// --------------------------------------------------------------- structure

/// Delete a subtree: its span leaves its file; block files go to trash (§11.5),
/// including blocks nested anywhere under it, so none is left orphaned.
pub fn delete_subtree(vault: &mut Vault, r: NRef) -> std::io::Result<String> {
    let n = vault.tree.node(r);
    if n.kind == Kind::Root {
        return Err(io_err("cannot delete the root"));
    }
    if n.is_block() || n.is_embed() {
        let target = if n.is_embed() { vault.tree.resolved_child(r) } else { r };
        if target == r && n.is_embed() {
            // broken embed: just the line
            let span = n.span;
            remove_span_with_separator(vault, r.0, span)?;
            return Ok("broken embed removed".into());
        }
        let ids = nested_block_ids(vault, target);
        let path = vault.tree.node(target).block.as_ref().unwrap().path.clone();
        for id in ids {
            trash_block(vault, &id)?;
        }
        return Ok(format!("block {:?} trashed", path));
    }
    let ids = plain_remove(vault, r)?;
    for id in ids {
        trash_block(vault, &id)?;
    }
    Ok("subtree trashed".into())
}

/// Ids of every block in the resolved subtree of `r` (pre-order, `r` itself
/// included if it is one).
fn nested_block_ids(vault: &Vault, r: NRef) -> Vec<Id> {
    let mut ids = Vec::new();
    vault.tree.walk(r, &mut |t, n| {
        if let Some(id) = t.node(n).block.as_ref().and_then(|b| b.id.clone()) {
            ids.push(id);
        }
    });
    ids
}

/// Remove a plain node's span after writing a trash copy of its text;
/// returns the ids of blocks embedded under it, still to be trashed.
fn plain_remove(vault: &mut Vault, r: NRef) -> std::io::Result<Vec<Id>> {
    let ids = nested_block_ids(vault, r);
    let text = render(&vault.tree, r, 1, true);
    let name = format!("{}.md", slug(&vault.tree.node(r).title));
    vault.trash_text(&name, &text)?;
    let span = vault.tree.node(r).span;
    remove_span_with_separator(vault, r.0, span)?;
    Ok(ids)
}

/// Remove a block's embed line (if it is still anywhere) and move its file to
/// the trash.
fn trash_block(vault: &mut Vault, id: &Id) -> std::io::Result<()> {
    let embed = vault.tree.files.iter().enumerate().find_map(|(fi, f)| {
        f.nodes
            .iter()
            .position(|nd| nd.embed.as_ref() == Some(id))
            .map(|ni| (fi, ni))
    });
    if let Some(e) = embed {
        let span = vault.tree.node(e).span;
        remove_span_with_separator(vault, e.0, span)?;
    }
    if let Some(b) = vault.tree.block_by_id(id) {
        vault.trash_file(b.0)?;
    }
    Ok(())
}

/// The byte range to cut when removing `span`: the node plus whatever blank
/// separator would otherwise be doubled or left dangling. A blank line that
/// separated the node from what follows stays when nothing blank precedes
/// it, so `- a\n- b\n\n# S` minus `b` keeps its blank before `# S`.
fn removal_range(text: &str, span: Span) -> (usize, usize) {
    let b = text.as_bytes();
    let end = span.end.min(text.len());
    let mut start = span.start.min(end);
    let mut end = end;
    let blank_before = start >= 2 && &b[start - 2..start] == b"\n\n";
    let ends_blank = end >= start + 2 && &b[end - 2..end] == b"\n\n";
    if end >= text.len() {
        if blank_before {
            start -= 1; // no trailing blank line at EOF
        }
    } else if start > 0 && ends_blank && !blank_before {
        end -= 1;
    }
    (start, end)
}

/// Shift a byte position for the removal of `start..end`.
fn after_removal(pos: usize, start: usize, end: usize) -> usize {
    if pos >= end {
        pos - (end - start)
    } else if pos > start {
        start
    } else {
        pos
    }
}

/// Remove a span plus one adjacent blank separator line, then re-parse just
/// that file (no full reload, so other files' node refs stay valid).
fn remove_span_no_reload(vault: &mut Vault, file: usize, span: Span) -> std::io::Result<()> {
    let (start, end) = removal_range(&vault.tree.files[file].text, span);
    vault.write_span(file, Span { start, end }, "")
}

/// Remove a span plus one adjacent blank separator line.
fn remove_span_with_separator(vault: &mut Vault, file: usize, span: Span) -> std::io::Result<()> {
    remove_span_no_reload(vault, file, span)
}

/// Yank: render the subtree resolved (the register's text) (§10.3).
pub fn yank(vault: &Vault, r: NRef) -> String {
    render(&vault.tree, r, 1, true)
}

/// Paste rendered text after/before a node as siblings (§10.3 `p`/`P`).
pub fn paste(vault: &mut Vault, at: NRef, text: &str, after: bool) -> std::io::Result<()> {
    let n = vault.tree.node(at);
    if n.kind == Kind::Root {
        return Err(io_err("cannot paste beside the root"));
    }
    let file = at.0;
    let indent = vault.tree.indent(at);
    let level_base = vault.tree.level(at);
    // re-indent/level the pasted text: parse it standalone and re-emit at
    // the target's position.
    let shifted = shift_document(text, level_base, indent);
    if after {
        let pos = trimmed_end(&vault.tree.files[file].text, n.span);
        insert_at(vault, file, pos, "\n\n", &shifted)
    } else {
        let pos = n.span.start;
        vault.write_span(file, Span { start: pos, end: pos }, &format!("{}\n", shifted))
    }
}

/// Parse a standalone document and re-emit it at the given level/indent.
/// `level` is the display level the document's *root* should take; `indent`
/// its indent. Everything inside keeps its position relative to the root.
pub fn shift_document(text: &str, level: usize, indent: usize) -> String {
    let block = crate::parse::Block {
        id: None,
        path: "clip.md".into(),
        props: Default::default(),
        frontmatter_raw: String::new(),
        frontmatter_span: None,
        edge_span: None,
    };
    let pf = crate::parse::parse_file("clip.md", text, 0, Some(block));
    let tree = crate::tree::Tree {
        files: vec![pf],
        root: (0, 0),
        blocks: vec![],
    };
    let mut out = String::new();
    let kids = tree.resolved_children(tree.root);
    for (i, k) in kids.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let r = render(&tree, *k, 1, true);
        let first = r.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let t = first.trim_start();
        let cur_level = if t.starts_with('#') {
            t.chars().take_while(|&c| c == '#').count()
        } else {
            1
        };
        let cur_indent = first.len() - t.len();
        let ld = level as isize - cur_level as isize;
        let id = indent as isize - cur_indent as isize;
        out.push_str(&shift_lines(&r, ld, id));
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Shift every line of a rendered document by (level_delta, indent_delta).
fn shift_lines(raw: &str, level_delta: isize, indent_delta: isize) -> String {
    let mut out = String::new();
    let mut fence: Option<(char, usize)> = None;
    for line in raw.split_inclusive('\n') {
        let l = line.strip_suffix('\n').unwrap_or(line);
        let nl = if line.ends_with('\n') { "\n" } else { "" };
        if fence_transition(l, &mut fence) || fence.is_some() {
            out.push_str(l);
            out.push_str(nl);
            continue;
        }
        let trimmed = l.trim_start();
        if trimmed.is_empty() {
            out.push_str(nl);
            continue;
        }
        let cur_indent = l.len() - trimmed.len();
        let new_indent = (cur_indent as isize + indent_delta).max(0) as usize;
        if trimmed.starts_with('#') {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            let new_level = (hashes as isize + level_delta).max(1) as usize;
            out.push_str(&" ".repeat(new_indent));
            out.push_str(&"#".repeat(new_level));
            out.push_str(&trimmed[hashes..]);
            out.push_str(nl);
        } else {
            out.push_str(&" ".repeat(new_indent));
            out.push_str(trimmed);
            out.push_str(nl);
        }
    }
    out
}

fn fence_transition(raw: &str, open: &mut Option<(char, usize)>) -> bool {
    let t = raw.trim_start();
    let first = match t.chars().next() {
        Some(c) if c == '`' || c == '~' => c,
        _ => return false,
    };
    let count = t.chars().take_while(|&c| c == first).count();
    if count < 3 {
        return false;
    }
    match open {
        None => {
            *open = Some((first, count));
            true
        }
        Some((c, n)) => {
            if *c == first && count >= *n {
                *open = None;
                true
            } else {
                false
            }
        }
    }
}

/// Refile: move a subtree under a new parent as its last child (§6.5).
pub fn refile(vault: &mut Vault, r: NRef, dest: NRef) -> std::io::Result<()> {
    let n = vault.tree.node(r);
    // guard: cannot refile into own subtree — ancestry followed through
    // embeds, so a destination inside a nested block counts too
    let embed_of_r = n.block.as_ref().and_then(|b| b.id.clone());
    let mut anc = Some(dest);
    let mut guard = 0;
    while let Some(a) = anc {
        let an = vault.tree.node(a);
        if a == r || (embed_of_r.is_some() && an.embed == embed_of_r) {
            return Err(io_err("cannot refile a node into itself"));
        }
        guard += 1;
        if guard > 10_000 {
            break; // embed cycle: a diagnostic elsewhere
        }
        anc = resolved_parent(&vault.tree, a);
    }
    if n.is_block() || n.is_embed() {
        // move only the embed line (§6.5)
        let embed_ref = if n.is_embed() {
            r
        } else {
            // find the embed node for this block
            let id = n.block.as_ref().unwrap().id.clone().unwrap();
            let mut embed_ref = None;
            for (fi, f) in vault.tree.files.iter().enumerate() {
                for (ni, nd) in f.nodes.iter().enumerate() {
                    if nd.embed.as_ref() == Some(&id) {
                        embed_ref = Some((fi, ni));
                    }
                }
            }
            match embed_ref {
                Some(e) => e,
                None => return Err(io_err("block has no embed")),
            }
        };
        // just the `![[id]]` line, without its old indent: insert_child_text
        // indents it for the destination
        let embed_text = {
            let en = vault.tree.node(embed_ref);
            en.title_span.text(&vault.tree.files[embed_ref.0].text).trim().to_string()
        };
        let dest_key = vault.key_of(dest);
        let dest_path = vault.tree.path(dest);
        // remove embed from source (no reload)
        let src_file = embed_ref.0;
        let span = vault.tree.node(embed_ref).span;
        let src_text = vault.tree.files[src_file].text.clone();
        remove_span_no_reload(vault, src_file, span)?;
        let Some(dest) = vault
            .find_by_key(&dest_key)
            .or_else(|| vault.find_by_path(&dest_path))
        else {
            // put the source back rather than lose the embed
            vault.write_file_text(src_file, &src_text)?;
            return Err(io_err("destination lost"));
        };
        insert_child_text(vault, dest, &embed_text, true)
    } else {
        // plain node: same-file span surgery when possible (no reload, so
        // sibling structure is preserved), cross-file otherwise.
        let src_file = r.0;
        // on-disk spelling: nested blocks travel as their embed lines
        let rendered = render(&vault.tree, r, 1, false);
        let span = vault.tree.node(r).span;
        if dest.0 == src_file && !vault.tree.node(dest).is_embed() {
            let text = vault.tree.files[src_file].text.clone();
            let dest_span = vault.tree.node(dest).span;
            let level = vault.tree.level(dest) + 1;
            let indent = if vault.tree.node(dest).kind == Kind::Item {
                vault.tree.indent(dest) + 2
            } else {
                vault.tree.indent(dest)
            };
            let shifted = shift_document(&rendered, level, indent);
            let (start, end) = removal_range(&text, span);
            let mut new_text = String::with_capacity(text.len());
            new_text.push_str(&text[..start]);
            new_text.push_str(&text[end..]);
            // adjust insertion point for the removal
            let mut ins = after_removal(dest_span.end.min(text.len()), start, end);
            while ins > 0 && new_text.as_bytes()[ins - 1] == b'\n' {
                ins -= 1;
            }
            let kids = vault.tree.resolved_children(dest);
            let all_items = !kids.is_empty()
                && kids.iter().all(|&k| vault.tree.node(k).kind == Kind::Item);
            let first_is_item = shifted.trim_start().starts_with("- ");
            let sep = if all_items && first_is_item { "\n" } else { "\n\n" };
            new_text.insert_str(ins, &format!("{}{}", sep, shifted));
            vault.write_file_text(src_file, &new_text)?;
            vault.reload()
        } else {
            let dest_key = vault.key_of(dest);
            let dest_path = vault.tree.path(dest);
            let src_text = vault.tree.files[src_file].text.clone();
            remove_span_with_separator(vault, src_file, span)?;
            let Some(dest) = vault
                .find_by_key(&dest_key)
                .or_else(|| vault.find_by_path(&dest_path))
            else {
                // never lose the subtree: put the source back
                vault.write_file_text(src_file, &src_text)?;
                return Err(io_err("destination lost"));
            };
            insert_child_text(vault, dest, &rendered, false)
        }
    }
}

/// Insert a rendered document as the last child of `parent`.
fn insert_child_text(
    vault: &mut Vault,
    parent: NRef,
    text: &str,
    is_embed: bool,
) -> std::io::Result<()> {
    let node = vault.tree.node(parent);
    let file = parent.0;
    // Items nest under items; everything under a section stays at the
    // section's own indent (§3.1, §4.10).
    let indent = if node.kind == Kind::Item {
        vault.tree.indent(parent) + 2
    } else {
        vault.tree.indent(parent)
    };
    let level = vault.tree.level(parent) + 1;
    let shifted = if is_embed {
        let mut s = String::new();
        for line in text.trim_end_matches('\n').split('\n') {
            s.push_str(&" ".repeat(indent));
            s.push_str(line);
            s.push('\n');
        }
        s
    } else {
        shift_document(text, level, indent)
    };
    // Separator: item children of an item-only list stay tight; sections and
    // first children of a body-less node get a blank line, except in a block
    // file whose spacing is the file's own (canonical: one blank).
    let kids = vault.tree.resolved_children(parent);
    let all_items = !kids.is_empty()
        && kids.iter().all(|&k| vault.tree.node(k).kind == Kind::Item);
    let first_line_is_item = shifted.trim_start().starts_with("- ");
    let sep = if all_items && first_line_is_item { "\n" } else { "\n\n" };
    let t = vault.tree.files[file].text.clone();
    let pos = trimmed_end(&t, node.span);
    insert_at(vault, file, pos, sep, &shifted)?;
    vault.reload()
}

/// Archive: refile under the top-level `Archive` section (§6.5).
pub fn archive(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let root = vault.tree.root;
    let mut dest = None;
    for c in vault.tree.resolved_children(root) {
        if vault.tree.node(c).title.eq_ignore_ascii_case("archive") {
            dest = Some(c);
            break;
        }
    }
    let dest = match dest {
        Some(d) => d,
        None => append_top_section(vault, "Archive")?,
    };
    refile(vault, r, dest)
}

/// Clear done: trash every done item with no open descendants under `target`
/// (§8.5). Returns the number trashed.
///
/// Nodes are removed by span, last-in-file first, so no node is ever looked
/// up again by its title path (two done items may share a title).
pub fn clear_done(vault: &mut Vault, target: NRef) -> std::io::Result<usize> {
    let mut to_clear: Vec<NRef> = Vec::new();
    vault.tree.walk(target, &mut |t, r| {
        let n = t.node(r);
        if n.task == Some(TaskState::Done) {
            let (open, _total) = t.task_counts(r);
            if open == 0 {
                to_clear.push(r);
            }
        }
    });
    // skip nodes under another node being cleared
    let tops: Vec<NRef> = to_clear
        .iter()
        .copied()
        .filter(|&r| {
            let mut a = resolved_parent(&vault.tree, r);
            let mut guard = 0;
            while let Some(p) = a {
                if to_clear.contains(&p) || guard > 10_000 {
                    return false;
                }
                guard += 1;
                a = resolved_parent(&vault.tree, p);
            }
            true
        })
        .collect();
    let mut blocks: Vec<Id> = Vec::new();
    let mut plain: Vec<NRef> = Vec::new();
    for &r in &tops {
        let n = vault.tree.node(r);
        match n.block.as_ref().and_then(|b| b.id.clone()) {
            Some(id) => blocks.push(id),
            None => plain.push(r),
        }
    }
    // plain spans: per file, from the end backwards, so earlier spans stay valid
    plain.sort_by_key(|&(f, n)| (f, std::cmp::Reverse(vault.tree.files[f].nodes[n].span.start)));
    let mut done = 0;
    for r in plain {
        let ids = plain_remove(vault, r)?;
        blocks.extend(ids);
        done += 1;
    }
    let n_block_tops = tops.len() - done;
    for id in blocks {
        trash_block(vault, &id)?;
    }
    Ok(done + n_block_tops)
}

/// A node's parent in the resolved tree: within its file, or — for a block
/// root — the embed that stitches it in (§4.7).
fn resolved_parent(tree: &crate::tree::Tree, r: NRef) -> Option<NRef> {
    let p = tree.node(r).parent?;
    let pr = (r.0, p);
    if tree.node(pr).kind == Kind::Root && r.0 != tree.root.0 {
        let id = tree.node(r).block.as_ref()?.id.clone()?;
        tree.files.iter().enumerate().find_map(|(fi, f)| {
            f.nodes
                .iter()
                .position(|nd| nd.embed.as_ref() == Some(&id))
                .map(|ni| (fi, ni))
        })
    } else if tree.node(pr).kind == Kind::Root {
        None
    } else {
        Some(pr)
    }
}

// --------------------------------------------------------------- make block

/// Give a node an id and its own file, leaving an embed (§6.1).
pub fn make_block(vault: &mut Vault, r: NRef) -> std::io::Result<Id> {
    let n = vault.tree.node(r);
    if n.is_block() || n.is_embed() {
        return Err(io_err("already a block"));
    }
    let id = Id::generate();
    // the file: render(node, 1, false) with frontmatter (§6.1.2)
    let body = render(&vault.tree, r, 1, false);
    let mut file_text = format!("---\nid: {}\n", id);
    // checkbox becomes todo: (§6.1.3)
    if let Some(state) = n.task {
        file_text.push_str(&format!(
            "todo: {}\n",
            match state {
                TaskState::Open => "open",
                TaskState::Done => "done",
            }
        ));
    }
    file_text.push_str("---\n\n");
    // strip the checkbox from the root title line (§4.5)
    let body_lines: Vec<&str> = body.lines().collect();
    let mut stripped = String::new();
    for (i, l) in body_lines.iter().enumerate() {
        if i == 0 && n.task.is_some() {
            match checkbox_span(l) {
                Some((start, end)) => {
                    stripped.push_str(&l[..start]);
                    stripped.push_str(&l[end..]);
                }
                None => stripped.push_str(l),
            }
        } else {
            stripped.push_str(l);
        }
        stripped.push('\n');
    }
    file_text.push_str(&stripped);
    let prefix = vault.unique_prefix(&id);
    let fname = crate::ident::filename(&prefix, &slug(&n.title));
    crate::vault::atomic_write(&vault.dir.join(&fname), &file_text)?;
    // replace the node's span with an embed (§6.1.3)
    let file = r.0;
    let span = n.span;
    let indent = vault.tree.indent(r);
    let embed = format!("{}![[{}]]\n", " ".repeat(indent), id);
    vault.write_span(file, span, &embed)?;
    vault.reload()?;
    Ok(id)
}

/// Set the first property on a plain node: makes it a block, then sets the
/// key (§3.2, §6.1).
pub fn set_property(vault: &mut Vault, r: NRef, key: &str, value: &str) -> std::io::Result<()> {
    let target = if vault.tree.node(r).is_block() {
        r
    } else if vault.tree.node(r).is_embed() {
        vault.tree.resolved_child(r)
    } else {
        // make_block reloads, so `r` is stale: find the new block by its id
        let id = make_block(vault, r)?;
        vault
            .tree
            .block_by_id(&id)
            .ok_or_else(|| io_err("block not found after make_block"))?
    };
    set_frontmatter_key(vault, target.0, key, Some(value))
}

// --------------------------------------------------------------- move/spelling

/// Move a node among its siblings (§10.3 `J`/`K`). A block moves by its
/// embed line, so blocks and plain nodes swap freely in the parent file.
pub fn move_sibling(vault: &mut Vault, r: NRef, down: bool) -> std::io::Result<()> {
    // a block root stands in its parent file as its embed
    let r = match vault.tree.node(r).block.as_ref().and_then(|b| b.id.clone()) {
        Some(_) if r.0 != vault.tree.root.0 => match resolved_parent(&vault.tree, r) {
            Some(e) if vault.tree.node(e).is_embed() => e,
            _ => return Ok(()),
        },
        _ => r,
    };
    let parent = match vault.tree.node(r).parent {
        Some(p) => (r.0, p),
        None => return Ok(()),
    };
    let siblings = vault.tree.raw_children(parent);
    let pos = match siblings.iter().position(|&s| s == r) {
        Some(p) => p,
        None => return Ok(()),
    };
    let swap_with = if down {
        pos + 1
    } else {
        match pos.checked_sub(1) {
            Some(p) => p,
            None => return Ok(()),
        }
    };
    if swap_with >= siblings.len() {
        return Ok(());
    }
    let other = siblings[swap_with];
    let file = r.0;
    let text = vault.tree.files[file].text.clone();
    // swap the nodes' own lines; trailing blank lines stay where they are,
    // so the list's spacing is unchanged
    let core = |sp: Span| {
        let end = sp.end.min(text.len());
        let mut e = end;
        while e >= sp.start + 2 && &text.as_bytes()[e - 2..e] == b"\n\n" {
            e -= 1;
        }
        Span { start: sp.start, end: e }
    };
    let a = core(vault.tree.node(r).span);
    let b = core(vault.tree.node(other).span);
    let (first, second) = if a.start < b.start { (a, b) } else { (b, a) };
    let with_nl = |t: &str| {
        if t.ends_with('\n') {
            t.to_string()
        } else {
            format!("{}\n", t)
        }
    };
    let mut new_text = format!(
        "{}{}{}{}",
        &text[..first.start],
        with_nl(second.text(&text)),
        &text[first.end..second.start],
        with_nl(first.text(&text)),
    );
    let rest = &text[second.end..];
    new_text.push_str(rest);
    vault.write_file_text(file, &new_text)
}

/// Toggle spelling section ↔ item (§10.3 `~`). The checkbox, if any, is kept.
pub fn toggle_spelling(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let n = vault.tree.node(r);
    if n.is_embed() || n.kind == Kind::Root {
        return Ok(());
    }
    let file = r.0;
    let ts = n.title_span;
    let line = ts.text(&vault.tree.files[file].text).to_string();
    let indent_str = &line[..line.len() - line.trim_start().len()];
    let trimmed = line.trim_start();
    // split off marker and checkbox
    let (after_marker, written_level) = if trimmed.starts_with('#') {
        let h = trimmed.chars().take_while(|&c| c == '#').count();
        (trimmed[h..].trim_start(), h)
    } else {
        (trimmed.trim_start_matches("- "), 0)
    };
    let (checkbox, title) = if let Some(rest) = after_marker.strip_prefix("[ ] ") {
        ("[ ] ", rest)
    } else if let Some(rest) = after_marker
        .strip_prefix("[x] ")
        .or_else(|| after_marker.strip_prefix("[X] "))
        .or_else(|| after_marker.strip_prefix("[-] "))
    {
        ("[x] ", rest)
    } else {
        ("", after_marker)
    };
    let new_line = match n.kind {
        Kind::Section => format!("{}- {}{}", indent_str, checkbox, title),
        Kind::Item => {
            // item → section: deeper than the nearest section ancestor so
            // the parser keeps it a child (§3.1)
            let lvl = (written_level + 1).max(vault.tree.level(r) + 1).max(1);
            format!("{}{} {}{}", indent_str, "#".repeat(lvl), checkbox, title)
        }
        Kind::Root => unreachable!(),
    };
    vault.write_span(file, ts, &new_line)
}

/// Demote: become the last child of the previous sibling (§10.3 `>`).
/// Same-file span surgery, no reload.
pub fn demote(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let parent = match vault.tree.node(r).parent {
        Some(p) => (r.0, p),
        None => return Ok(()),
    };
    let siblings = vault.tree.resolved_children(parent);
    let pos = match siblings.iter().position(|&s| s == r) {
        Some(p) => p,
        None => return Ok(()),
    };
    if pos == 0 {
        return Ok(());
    }
    let prev = siblings[pos - 1];
    if prev.0 != r.0 || vault.tree.node(r).is_embed() || vault.tree.node(prev).is_embed() {
        // cross-file or embed demotion falls back to refile
        return refile(vault, r, prev);
    }
    // Sections and items nest differently: delegate to the same logic refile
    // uses so the shifted text is right for the spelling.
    return refile(vault, r, prev);
}

/// Promote: become the next sibling of the parent (§10.3 `<`). Same-file
/// span surgery; out of a block's root, the node moves to the parent file
/// right after the block's embed.
pub fn promote(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let n = vault.tree.node(r);
    let parent = match n.parent {
        Some(p) => (r.0, p),
        None => return Ok(()),
    };
    if vault.tree.node(parent).kind == Kind::Root {
        return Ok(());
    }
    let file = r.0;
    // on-disk spelling: nested blocks travel as their embed lines
    let rendered = render(&vault.tree, r, 1, false);
    let span = n.span;
    let text = vault.tree.files[file].text.clone();
    let (start, end) = removal_range(&text, span);
    let mut new_text = String::with_capacity(text.len());
    new_text.push_str(&text[..start]);
    new_text.push_str(&text[end..]);
    if vault.tree.node(parent).is_block() && file != vault.tree.root.0 {
        // the parent is a block root: its sibling position is beside the embed
        let Some(embed) = resolved_parent(&vault.tree, parent) else {
            return Err(io_err("block has no embed"));
        };
        // a section beside the embed sits one below the embed's enclosing
        // section (the embed line itself is spelled as an item)
        let in_section = vault
            .tree
            .ancestors(embed)
            .iter()
            .any(|&a| vault.tree.node(a).kind == Kind::Section);
        let level = if vault.tree.node(r).kind == Kind::Section && in_section {
            vault.tree.level(embed) + 1
        } else {
            vault.tree.level(embed).max(1)
        };
        let indent = vault.tree.indent(embed);
        let shifted = shift_document(&rendered, level, indent);
        let efile = embed.0;
        let espan = vault.tree.node(embed).title_span;
        let etext = vault.tree.files[efile].text.clone();
        let ins = (espan.end + 1).min(etext.len());
        let tight = vault.tree.node(r).kind == Kind::Item
            && vault.tree.node(parent).kind == Kind::Item;
        let mut out = String::with_capacity(etext.len() + shifted.len() + 2);
        out.push_str(&etext[..ins]);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        if !tight {
            out.push('\n');
        }
        out.push_str(&shifted);
        let after = &etext[ins..];
        if !tight && !after.is_empty() && !after.starts_with('\n') {
            out.push('\n');
        }
        out.push_str(after);
        vault.write_file_text(file, &new_text)?;
        vault.write_file_text(efile, &out)?;
        return vault.reload();
    }
    let parent_span = vault.tree.node(parent).span;
    let level = vault.tree.level(parent);
    let indent = vault.tree.indent(parent);
    let shifted = shift_document(&rendered, level, indent);
    // the parent contains r, so its span end moves back by the removed bytes
    let mut ins = after_removal(parent_span.end.min(text.len()), start, end);
    while ins > 0 && new_text.as_bytes()[ins - 1] == b'\n' {
        ins -= 1;
    }
    new_text.insert_str(ins, &format!("\n\n{}", shifted));
    vault.write_file_text(file, &new_text)?;
    vault.reload()
}

/// Rename a node's title (used by the editor when it detects a title change,
/// and by tests).
pub fn rename_title(vault: &mut Vault, r: NRef, new_title: &str) -> std::io::Result<()> {
    let n = vault.tree.node(r);
    let file = r.0;
    let ts = n.title_span;
    let line = ts.text(&vault.tree.files[file].text).to_string();
    // replace only the title text after the marker/checkbox
    let marker_len = line.len() - line.trim_start().len();
    let after_indent = &line[marker_len..];
    let prefix_len = if after_indent.starts_with('#') {
        let h = after_indent.chars().take_while(|&c| c == '#').count();
        marker_len + h + 1
    } else if after_indent.starts_with("- ") {
        marker_len + 2
    } else {
        marker_len
    };
    let mut prefix = line[..prefix_len].to_string();
    // keep checkbox
    let rest = &line[prefix_len..];
    let checkbox = if rest.starts_with("[ ] ") {
        "[ ] "
    } else if rest.starts_with("[x] ") || rest.starts_with("[X] ") || rest.starts_with("[-] ") {
        "[x] "
    } else {
        ""
    };
    if !checkbox.is_empty() {
        prefix.push_str(checkbox);
    }
    let new_line = format!("{}{}", prefix, new_title);
    let is_block_root = n.is_block();
    vault.write_span(file, ts, &new_line)?;
    if is_block_root {
        vault.rename_block_file(file, new_title)?;
    }
    Ok(())
}

/// Append a plain item child titled `title` under `parent`; returns the new
/// node. Used by `N` in the TUI (§10.3).
pub fn append_child_public(
    vault: &mut Vault,
    parent: NRef,
    title: &str,
) -> std::io::Result<NRef> {
    append_child_line(vault, parent, &format!("- {}", title), true)
}

fn io_err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, msg.to_string())
}
