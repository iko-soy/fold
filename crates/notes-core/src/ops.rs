//! Structural operations on the tree (§5.2, §6, §7, §8). Every op mutates
//! files through the vault and returns an inverse for the undo log (§10.11).

use crate::ident::{slug, Id};
use crate::parse::{Kind, Span, TaskState};
use crate::render::render;
use crate::tree::NRef;
use crate::vault::{NodeKey, Vault};

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
                Some(t) => crate::vault::atomic_write(&vault.dir.join(&path), &t)?,
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
    let title = if task {
        format!("[ ] {}", text.trim())
    } else {
        text.trim().to_string()
    };
    append_child_line(vault, dest, &format!("- {}", title), task)
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
    // insert at the end of the parent's span
    let mut pos = node.span.end;
    let text = &vault.tree.files[file].text;
    // skip trailing newlines of the span to insert before them
    while pos > node.span.start && text.as_bytes()[pos - 1] == b'\n' {
        pos -= 1;
    }
    let insertion = format!("\n\n{}\n", line);
    vault.write_span(file, Span { start: pos, end: pos }, &insertion)
}

/// Append an item line as the last child of a node; returns the new node.
fn append_child_line(
    vault: &mut Vault,
    parent: NRef,
    line: &str,
    _task: bool,
) -> std::io::Result<NRef> {
    let parent_path = vault.tree.path(parent);
    let parent_key = vault.key_of(parent);
    let node = vault.tree.node(parent);
    let file = parent.0;
    let indent = if node.kind == Kind::Root {
        0
    } else {
        vault.tree.indent(parent) + 2
    };
    let line_indented = format!("{}{}", " ".repeat(indent), line);
    let text = vault.tree.files[file].text.clone();
    let mut pos = node.span.end;
    while pos > node.span.start && text.as_bytes()[pos - 1] == b'\n' {
        pos -= 1;
    }
    // blank line before unless the previous sibling is also an item
    let prev_is_item = vault
        .tree
        .resolved_children(parent)
        .last()
        .map(|&c| vault.tree.node(c).kind == Kind::Item)
        .unwrap_or(false);
    let insertion = if prev_is_item || node.kind == Kind::Item {
        format!("\n{}\n", line_indented)
    } else {
        format!("\n\n{}\n", line_indented)
    };
    vault.write_span(file, Span { start: pos, end: pos }, &insertion)?;
    vault.reload()?;
    let parent = vault
        .find_by_key(&parent_key)
        .or_else(|| vault.find_by_path(&parent_path))
        .ok_or_else(|| io_err("parent lost after reload"))?;
    let kids = vault.tree.resolved_children(parent);
    kids.last()
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
        let (from, to) = match n.task {
            Some(TaskState::Open) => ("[ ]", "[x]"),
            Some(TaskState::Done) => ("[x]", "[ ]"),
            None => return Ok(()), // not a task; `t` makes it one
        };
        let new_line = line.replacen(from, to, 1);
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
            Some(TaskState::Open) => line.replacen("[ ] ", "", 1),
            Some(TaskState::Done) => line
                .replacen("[x] ", "", 1)
                .replacen("[X] ", "", 1)
                .replacen("[-] ", "", 1),
            None => {
                // insert `[ ] ` after the marker
                if line.trim_start().starts_with('#') {
                    let idx = line.find(' ').map(|i| i + 1).unwrap_or(line.len());
                    format!("{}[ ] {}", &line[..idx], &line[idx..])
                } else if let Some(pos) = line.find("- ") {
                    let mut s = line.clone();
                    s.replace_range(pos..pos + 2, "- [ ] ");
                    s
                } else {
                    line.clone()
                }
            }
        };
        vault.write_span(file, ts, &new_line)
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

/// Delete a subtree: its span leaves its file; block files go to trash (§11.5).
pub fn delete_subtree(vault: &mut Vault, r: NRef) -> std::io::Result<String> {
    let n = vault.tree.node(r);
    if n.is_block() {
        // delete the embed from the parent file and trash the block file
        let b = n.block.clone().unwrap();
        if let Some(edge) = b.edge_span {
            // find which file holds the edge
            for (fi, f) in vault.tree.files.iter().enumerate() {
                if f.nodes.iter().any(|nd| nd.embed.as_ref() == b.id.as_ref()) {
                    let embed_node = f
                        .nodes
                        .iter()
                        .position(|nd| nd.embed.as_ref() == b.id.as_ref())
                        .unwrap();
                    let span = vault.tree.files[fi].nodes[embed_node].span;
                    remove_span_with_separator(vault, fi, span)?;
                    break;
                }
            }
            let _ = edge;
        }
        let file = r.0;
        vault.trash_file(file)?;
        return Ok(format!("block {:?} trashed", b.path));
    }
    if n.is_embed() {
        // delete the embed line and trash the referenced block file
        let id = n.embed.clone().unwrap();
        let target = vault.tree.block_by_id(&id);
        let file = r.0;
        let span = vault.tree.node(r).span;
        remove_span_with_separator(vault, file, span)?;
        if let Some(t) = target {
            let tf = t.0;
            vault.trash_file(tf)?;
        }
        return Ok("embed and block trashed".into());
    }
    // plain node: remove its span; trash a copy of the rendered text (§11.5)
    let text = render(&vault.tree, r, 1, true);
    let name = format!("{}.md", slug(&vault.tree.node(r).title));
    vault.trash_text(&name, &text)?;
    let file = r.0;
    let span = vault.tree.node(r).span;
    remove_span_with_separator(vault, file, span)?;
    Ok("subtree trashed".into())
}

/// Remove a span plus one adjacent blank separator line, then re-parse just
/// that file (no full reload, so other files' node refs stay valid).
fn remove_span_no_reload(vault: &mut Vault, file: usize, span: Span) -> std::io::Result<()> {
    let text = vault.tree.files[file].text.clone();
    let mut start = span.start;
    let mut end = span.end;
    if start >= 2 && &text[start - 2..start] == "\n\n" {
        start -= 1;
    } else if end < text.len() && text.as_bytes()[end] == b'\n' {
        end += 1;
    }
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
    let (pos, prefix) = if after {
        let mut pos = n.span.end;
        let t = &vault.tree.files[file].text;
        while pos > n.span.start && t.as_bytes()[pos - 1] == b'\n' {
            pos -= 1;
        }
        (pos, "\n\n")
    } else {
        (n.span.start, "")
    };
    let insertion = if after {
        format!("{}{}", prefix, shifted)
    } else {
        format!("{}\n", shifted)
    };
    vault.write_span(file, Span { start: pos, end: pos }, &insertion)
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
    // guard: cannot refile into own subtree
    let mut anc = Some(dest);
    while let Some(a) = anc {
        if a == r {
            return Err(io_err("cannot refile a node into itself"));
        }
        anc = vault.tree.node(a).parent.map(|p| (a.0, p));
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
        let embed_text = {
            let en = vault.tree.node(embed_ref);
            en.span.text(&vault.tree.files[embed_ref.0].text).to_string()
        };
        let dest_key = vault.key_of(dest);
        let dest_path = vault.tree.path(dest);
        // remove embed from source (no reload)
        let src_file = embed_ref.0;
        let span = vault.tree.node(embed_ref).span;
        remove_span_no_reload(vault, src_file, span)?;
        let dest = vault
            .find_by_key(&dest_key)
            .or_else(|| vault.find_by_path(&dest_path))
            .ok_or_else(|| io_err("destination lost"))?;
        insert_child_text(vault, dest, &embed_text, true)
    } else {
        // plain node: same-file span surgery when possible (no reload, so
        // sibling structure is preserved), cross-file otherwise.
        let src_file = r.0;
        let rendered = render(&vault.tree, r, 1, true);
        let span = vault.tree.node(r).span;
        if dest.0 == src_file && !vault.tree.node(dest).is_embed() {
            let text = vault.tree.files[src_file].text.clone();
            let dest_span = vault.tree.node(dest).span;
            let level = vault.tree.level(dest) + 1;
            let indent = if vault.tree.node(dest).kind == Kind::Root {
                0
            } else {
                vault.tree.indent(dest) + 2
            };
            let shifted = shift_document(&rendered, level, indent);
            // remove r's span plus a preceding blank separator
            let mut start = span.start;
            if start >= 2 && &text[start - 2..start] == "\n\n" {
                start -= 1;
            }
            let mut new_text = String::with_capacity(text.len());
            new_text.push_str(&text[..start]);
            new_text.push_str(&text[span.end..]);
            // adjust insertion point for the removal
            let removed = span.end - start;
            let mut ins = if dest_span.end > start {
                dest_span.end - removed
            } else {
                dest_span.end
            };
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
            remove_span_with_separator(vault, src_file, span)?;
            let dest = vault
                .find_by_key(&dest_key)
                .or_else(|| vault.find_by_path(&dest_path))
                .ok_or_else(|| io_err("destination lost"))?;
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
    let indent = if node.kind == Kind::Root {
        0
    } else {
        vault.tree.indent(parent) + 2
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
    let mut pos = node.span.end;
    while pos > node.span.start && t.as_bytes()[pos - 1] == b'\n' {
        pos -= 1;
    }
    let insertion = format!("{}{}", sep, shifted);
    vault.write_span(file, Span { start: pos, end: pos }, &insertion)?;
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
    // Record reload-stable keys and ancestor relationships before mutating.
    let keys: Vec<NodeKey> = to_clear.iter().map(|&r| vault.key_of(r)).collect();
    let paths: Vec<Vec<String>> = to_clear.iter().map(|&r| vault.tree.path(r)).collect();
    let mut done = 0;
    // Deepest first; skip nodes whose ancestor is also being cleared.
    let mut order: Vec<usize> = (0..to_clear.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(paths[i].len()));
    for i in order {
        let is_descendant_of_cleared = to_clear
            .iter()
            .enumerate()
            .any(|(j, _)| j != i && paths[j].len() < paths[i].len()
                && paths[i][..paths[j].len()] == paths[j][..]);
        if is_descendant_of_cleared {
            continue;
        }
        if let Some(r) = vault.find_by_key(&keys[i]) {
            delete_subtree(vault, r)?;
            done += 1;
        }
    }
    Ok(done)
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
            let l2 = l
                .replacen("[ ] ", "", 1)
                .replacen("[x] ", "", 1)
                .replacen("[X] ", "", 1)
                .replacen("[-] ", "", 1);
            stripped.push_str(&l2);
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
        make_block(vault, r)?;
        // find the new block by re-resolving: the embed replaced the node
        let path = vault.tree.path(r);
        let mut found = None;
        for (fi, f) in vault.tree.files.iter().enumerate() {
            for (ni, nd) in f.nodes.iter().enumerate() {
                if nd.embed.is_some() {
                    let rr = (fi, ni);
                    if vault.tree.path(rr) == path {
                        found = Some(vault.tree.resolved_child(rr));
                    }
                }
            }
        }
        found.ok_or_else(|| io_err("block not found after make_block"))?
    };
    set_frontmatter_key(vault, target.0, key, Some(value))
}

// --------------------------------------------------------------- move/spelling

/// Move a node among its siblings (§10.3 `J`/`K`).
pub fn move_sibling(vault: &mut Vault, r: NRef, down: bool) -> std::io::Result<()> {
    let parent = match vault.tree.node(r).parent {
        Some(p) => (r.0, p),
        None => return Ok(()),
    };
    let siblings = vault.tree.resolved_children(parent);
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
    if other.0 != file {
        return Ok(()); // different files: skip (v1 simplicity)
    }
    let a = vault.tree.node(r).span;
    let b = vault.tree.node(other).span;
    let text = vault.tree.files[file].text.clone();
    let (first, second) = if a.start < b.start { (a, b) } else { (b, a) };
    let between = &text[first.end..second.start];
    let new_text = format!(
        "{}{}{}{}{}",
        &text[..first.start],
        second.text(&text),
        between,
        first.text(&text),
        &text[second.end..]
    );
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

/// Promote: become the next sibling of the parent (§10.3 `<`).
/// Promote: become the next sibling of the parent (§10.3 `<`). Same-file
/// span surgery, no reload.
pub fn promote(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let n = vault.tree.node(r);
    let parent = match n.parent {
        Some(p) => (r.0, p),
        None => return Ok(()),
    };
    if vault.tree.node(parent).kind == Kind::Root {
        return Ok(());
    }
    // move out: delete span, insert after parent's span
    let file = r.0;
    let rendered = render(&vault.tree, r, 1, true);
    let span = n.span;
    let parent_span = vault.tree.node(parent).span;
    let level = vault.tree.level(parent);
    let indent = vault.tree.indent(parent);
    let shifted = shift_document(&rendered, level, indent);
    let text = vault.tree.files[file].text.clone();
    // remove r's span plus a preceding blank separator
    let mut start = span.start;
    if start >= 2 && &text[start - 2..start] == "\n\n" {
        start -= 1;
    }
    let removed = span.end - start;
    let mut new_text = String::with_capacity(text.len());
    new_text.push_str(&text[..start]);
    new_text.push_str(&text[span.end..]);
    // the parent contains r, so its span end moves back by the removed bytes
    let mut ins = parent_span.end - removed;
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

fn io_err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, msg.to_string())
}
