//! Structural operations on the tree (§5.2, §6, §7, §8). Every op mutates
//! files through the vault; the undo log records what each one changed as
//! an `Inverse` (§10.10).

use crate::ident::{slug, Id};
use crate::parse::{Kind, Span, TaskState};
use crate::render::render;
use crate::tree::NRef;
use crate::vault::Vault;

/// Every vault file's text at one moment, taken before an operation.
#[derive(Debug, Clone)]
pub struct Snapshot {
    files: Vec<(String, String)>,
    description: String,
}

impl Snapshot {
    pub fn take(vault: &Vault, description: &str) -> Snapshot {
        Snapshot {
            files: vault
                .tree
                .files
                .iter()
                .map(|f| (f.path.clone(), f.text.clone()))
                .collect(),
            description: description.into(),
        }
    }
}

/// One file an operation touched: its text before and after (`None`:
/// absent — created or deleted by the operation).
#[derive(Debug, Clone)]
pub struct Change {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// One entry of the session op log (§10.10): exactly the files an operation
/// changed, created or deleted, before and after.
#[derive(Debug, Clone)]
pub struct Inverse {
    pub changes: Vec<Change>,
    pub description: String,
}

impl Inverse {
    /// What changed since `snap` was taken; `None` if nothing did, so an
    /// operation that did nothing leaves no entry.
    pub fn since(snap: Snapshot, vault: &Vault) -> Option<Inverse> {
        let mut changes = Vec::new();
        for (path, before) in &snap.files {
            let after = vault.tree.files.iter().find(|f| f.path == *path).map(|f| &f.text);
            if after != Some(before) {
                changes.push(Change {
                    path: path.clone(),
                    before: Some(before.clone()),
                    after: after.cloned(),
                });
            }
        }
        for f in &vault.tree.files {
            if !snap.files.iter().any(|(p, _)| *p == f.path) {
                changes.push(Change {
                    path: f.path.clone(),
                    before: None,
                    after: Some(f.text.clone()),
                });
            }
        }
        (!changes.is_empty()).then(|| Inverse {
            changes,
            description: snap.description,
        })
    }

    /// Undo: put every touched file back as it was before. Refuses, writing
    /// nothing, if any of them is no longer as the operation left it — an
    /// external change since then (§10.10).
    pub fn undo(&self, vault: &mut Vault) -> std::io::Result<()> {
        self.swap(vault, false)
    }

    /// Redo: the reverse, with the same check against the `before` side.
    pub fn redo(&self, vault: &mut Vault) -> std::io::Result<()> {
        self.swap(vault, true)
    }

    fn swap(&self, vault: &mut Vault, forward: bool) -> std::io::Result<()> {
        for c in &self.changes {
            let expect = if forward { &c.before } else { &c.after };
            let now = std::fs::read_to_string(vault.dir.join(&c.path)).ok();
            if now != *expect {
                return Err(io_err(&format!(
                    "{} changed since {}; not overwriting",
                    c.path, self.description
                )));
            }
        }
        for c in &self.changes {
            let full = vault.dir.join(&c.path);
            match if forward { &c.after } else { &c.before } {
                Some(t) => crate::vault::atomic_write(&full, t)?,
                None => std::fs::remove_file(&full)?,
            }
        }
        vault.reload()
    }
}

/// The session op log (§10.10): what undo would take back and redo would
/// do again, and the blocks an editor save left in transit (§5.2), each
/// by its file, with the length of `undo` once the entry of the save that
/// wrote its embed out was in. What the TUI and the app hold between
/// verbs.
#[derive(Debug, Default)]
pub struct OpLog {
    undo: Vec<Inverse>,
    redo: Vec<Inverse>,
    transit: Vec<(String, usize)>,
}

impl OpLog {
    /// The entry undo would take back.
    pub fn last_undo(&self) -> Option<&Inverse> {
        self.undo.last()
    }

    /// The entry redo would do again.
    pub fn last_redo(&self) -> Option<&Inverse> {
        self.redo.last()
    }

    /// How many entries undo can take back.
    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    /// An operation's entry. `None`, an operation that changed nothing,
    /// leaves the log as it is, the redo stack too. True when recorded.
    pub fn record(&mut self, inv: Option<Inverse>) -> bool {
        let Some(inv) = inv else { return false };
        self.undo.push(inv);
        self.redo.clear();
        true
    }

    /// `record` for a save of the editor, its *Revert*, or a reload that
    /// re-renders it (§11.2): a block an earlier save left in transit
    /// (§5.2) and this one deletes is deleted in the entry of that save,
    /// which wrote its embed out, so one undo puts back its file and its
    /// embed together, never the file embedded nowhere. One this save
    /// pastes back, writing its embed again, makes that save, this one and
    /// those between one entry, so no undo stops where its file is embedded
    /// nowhere either. `in_transit` are the files of the blocks the editor
    /// holds in transit now, remembered with this save's entry until their
    /// file is deleted or embedded again; `editing`, whether it is still
    /// open. True when there was an entry to record.
    pub fn record_edit(&mut self, inv: Option<Inverse>, vault: &Vault, editing: bool, in_transit: Vec<String>) -> bool {
        // the length of `undo` once this save's entry is in, if it has one
        let mut at = None;
        let recorded = inv.is_some();
        if let Some(mut inv) = inv {
            let (transit, undo) = (&self.transit, &mut self.undo);
            inv.changes.retain(|c| {
                let entry = transit.iter().find(|(p, _)| *p == c.path && c.after.is_none());
                // that entry, and none since, left the file as it was
                let Some(&(_, n)) = entry.filter(|&&(_, n)| n > 0 && n <= undo.len()) else { return true };
                if undo[n - 1..].iter().any(|e| e.changes.iter().any(|x| x.path == c.path)) {
                    return true;
                }
                undo[n - 1].changes.push(c.clone());
                false
            });
            if !inv.changes.is_empty() {
                self.undo.push(inv);
                at = Some(self.undo.len());
            }
            self.redo.clear();
        }
        // pasted back: joined from the earliest save that wrote out the
        // embed of a block embedded again now
        let mut back: Vec<usize> =
            self.transit.iter().filter(|(p, _)| embedded(vault, p) == Some(true)).map(|&(_, n)| n).collect();
        back.sort_unstable();
        if let Some(n) = back.into_iter().find(|&n| n > 0 && self.join(n - 1)) {
            // the entries from `n - 1` on are the one at `n - 1` now, if any
            let joined = self.undo.len() == n;
            at = at.and(joined.then_some(n));
            self.transit.retain_mut(|(_, m)| {
                *m = (*m).min(n);
                *m < n || joined
            });
        }
        // still in transit, or pasted back with its embed not written yet
        self.transit.retain(|(p, _)| editing && embedded(vault, p) == Some(false));
        if let Some(n) = at {
            for p in in_transit {
                if !self.transit.iter().any(|(q, _)| *q == p) {
                    self.transit.push((p, n));
                }
            }
        }
        recorded
    }

    /// Make the entries from `from` on one: for each file, its text before
    /// the first and after the last; one that ends as it began is left
    /// out, and an entry left with none is dropped. False, with the entries
    /// as they were, where a file changed from outside between two of
    /// them: undoing them as one would drop that change.
    fn join(&mut self, from: usize) -> bool {
        let Some(first) = self.undo.get(from) else { return false };
        let description = first.description.clone();
        let mut changes: Vec<Change> = Vec::new();
        for c in self.undo[from..].iter().flat_map(|e| &e.changes) {
            match changes.iter_mut().find(|x| x.path == c.path) {
                Some(x) if x.after == c.before => x.after = c.after.clone(),
                Some(_) => return false,
                None => changes.push(c.clone()),
            }
        }
        changes.retain(|c| c.before != c.after);
        self.undo.truncate(from);
        if !changes.is_empty() {
            self.undo.push(Inverse { changes, description });
        }
        true
    }

    /// Undo the last entry, or with `undo` false redo the last one undone
    /// (§10.10), and say which by its description. `None` when there is
    /// none. Refused — a file it touched changed since — it stays where
    /// it was. Either way the blocks in transit are let go: the entries
    /// they were remembered with may go.
    pub fn step(&mut self, vault: &mut Vault, undo: bool) -> Option<std::io::Result<String>> {
        self.transit.clear();
        let (from, to) = if undo { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let inv = from.pop()?;
        let res = if undo { inv.undo(vault) } else { inv.redo(vault) };
        Some(match res {
            Ok(()) => {
                let done = inv.description.clone();
                to.push(inv);
                Ok(done)
            }
            Err(e) => {
                from.push(inv);
                Err(e)
            }
        })
    }
}

/// Whether the block whose file is `path` is embedded now; `None` where
/// the file is gone.
fn embedded(vault: &Vault, path: &str) -> Option<bool> {
    let tree = &vault.tree;
    let f = vault.file_index(path)?;
    let (_, id) = tree.blocks.iter().find(|(r, _)| r.0 == f)?;
    Some(tree.embed_of(id).is_some())
}

fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

// ---------------------------------------------------------------- capture

/// Append an item under today's day section of `Inbox` (§7).
pub fn capture(vault: &mut Vault, text: &str, task: bool) -> std::io::Result<NRef> {
    capture_inner(vault, text, task, None)
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
    // refused before the inbox or its day is created
    let mut line = item_line(&title)?;
    let dest = match target {
        Some(t) => write_target(&vault.tree, t)?,
        None => {
            let inbox = find_or_create_inbox(vault)?;
            find_or_create_day(vault, inbox)?
        }
    };
    if !rest.trim().is_empty() {
        let item_indent = child_indent(&vault.tree, dest);
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
    // `## <today>` as the last child of Inbox (§7), at its child indent: an
    // Inbox respelled as an item is still the inbox (§3.1); no deeper than
    // a day before it as written, which would take it as its child
    let last = vault.tree.raw_children(inbox).last().copied();
    let level = vault.tree.level(inbox) + 1;
    let level = written_level(&vault.tree, last).map_or(level, |w| level.min(w));
    let indent = child_indent(&vault.tree, inbox);
    let heading = format!(
        "{}{} {}",
        " ".repeat(indent),
        "#".repeat(level),
        day
    );
    let inbox_key = vault.key_of(inbox);
    append_structural_line(vault, inbox, &heading)?;
    let inbox = vault
        .find_by_key(&inbox_key)
        .ok_or_else(|| io_err("inbox lost after the write"))?;
    for c in vault.tree.resolved_children(inbox) {
        if vault.tree.node(c).title == day {
            return Ok(c);
        }
    }
    Err(io_err("day section not created"))
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
    // under the node an embed stands for: an embed line has no children
    // in its file (§6.2)
    let parent = write_target(&vault.tree, parent)?;
    let parent_key = vault.key_of(parent);
    let node = vault.tree.node(parent);
    let file = parent.0;
    // Items under a section sit at the section's own indent; items under an
    // item nest one level deeper (§3.1, §4.10).
    let indent = child_indent(&vault.tree, parent);
    let text = vault.tree.files[file].text.clone();
    let kids = vault.tree.raw_children(parent);
    let is_section = |r: NRef| vault.tree.node(r).kind == Kind::Section;
    let first_section = kids.iter().position(|&c| is_section(c));
    // §4.2 puts one blank line between a node's body and its first child:
    // an item with no body has its first child right under its title
    let bare_item = node.kind == Kind::Item && node.body_lines(&text).is_empty();
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
            // at the last section's written indent, which its parent
            // reaches, and written level, which keeps it beside it
            let heading = format!(
                "{}{} {}",
                " ".repeat(vault.tree.node(last).indent),
                "#".repeat(written_level(&vault.tree, Some(last)).unwrap_or(1)),
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
            let sep = if prev_is_item || (fs == 0 && bare_item) { "" } else { "\n" };
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
            // (tight list), or there is none under a bare item
            let prev_is_item = kids.last().map(|&c| !is_section(c)).unwrap_or(bare_item);
            let sep = if prev_is_item { "\n" } else { "\n\n" };
            insert_at(vault, file, pos, sep, &line_indented)?;
            index = kids.len();
        }
    }
    let parent = vault
        .find_by_key(&parent_key)
        .ok_or_else(|| io_err("parent lost after the write"))?;
    vault
        .tree
        .resolved_children(parent)
        .get(index)
        .copied()
        .ok_or_else(|| io_err("child not created"))
}

// ---------------------------------------------------------------- tasks

/// Toggle task open/done (§8.1, §8.2). The state is the checkbox on the
/// title line for every node; a block also gets `done:` stamped or removed.
pub fn toggle_task(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let r = vault.tree.resolved_child(r);
    let new_state = match vault.tree.node(r).task {
        Some(TaskState::Open) => TaskState::Done,
        Some(TaskState::Done) => TaskState::Open,
        None => return Ok(()), // not a task; `t` makes it one
    };
    set_task(vault, r, Some(new_state))
}

/// Toggle task-ness itself (§10.3 `t`).
pub fn toggle_taskness(vault: &mut Vault, r: NRef) -> std::io::Result<()> {
    let r = write_target(&vault.tree, r)?;
    let state = match vault.tree.node(r).task {
        Some(_) => None,
        None => Some(TaskState::Open),
    };
    set_task(vault, r, state)
}

/// Write a node's task state (§4.5, §8.1): the checkbox on its title line
/// (`None` removes it). For a block, `done:` is stamped when it is checked
/// and removed otherwise.
pub fn set_task(vault: &mut Vault, r: NRef, state: Option<TaskState>) -> std::io::Result<()> {
    let n = vault.tree.node(r);
    let is_block = n.is_block();
    let file = r.0;
    // a setext title has no marker to put the checkbox after: it is
    // rewritten as an ATX heading, underline and all (§4.2)
    let (ts, line) = setext_as_atx(&vault.tree, r).unwrap_or_else(|| {
        (n.title_span, n.title_span.text(&vault.tree.files[file].text).to_string())
    });
    let mark = |st: TaskState| match st {
        TaskState::Open => "[ ]",
        TaskState::Done => "[x]", // also rewrites `[X]` / `[-]`
    };
    let new_line = match (checkbox_span(&line), state) {
        (Some((start, _)), Some(st)) => {
            let mut l = line.clone();
            l.replace_range(start..start + 3, mark(st));
            l
        }
        (Some((start, end)), None) => format!("{}{}", &line[..start], &line[end..]),
        // insert right after the marker, whatever the indent
        (None, Some(st)) => match marker_end(&line) {
            Some(idx) => format!("{}{} {}", &line[..idx], mark(st), &line[idx..]),
            None => format!("{} {}", line.trim_end(), mark(st)),
        },
        (None, None) => line.clone(),
    };
    // `- [ ] ---` unchecked is `- ---`, a thematic break (§4.4)
    not_a_break(&new_line)?;
    if new_line != ts.text(&vault.tree.files[file].text) {
        vault.write_span(file, ts, &new_line)?;
    }
    if is_block {
        let done = match state {
            Some(TaskState::Done) => Some(today()),
            _ => None,
        };
        set_frontmatter_key(vault, file, "done", done.as_deref())?;
    }
    Ok(())
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

/// A setext section's title (§4.2: read, converted on write) as an ATX
/// heading at its written level and indent, with the span it replaces: the
/// title line and its underline, the line after it. `None` for any other
/// node. The written level keeps what nests under the heading unchanged.
fn setext_as_atx(tree: &crate::tree::Tree, r: NRef) -> Option<(Span, String)> {
    let n = tree.node(r);
    let text = tree.text_of(r);
    let line = n.title_span.text(text);
    let t = line.trim_start();
    // every other section's title line is an ATX heading (or a heading embed)
    if n.kind != Kind::Section || atx_hashes(t).is_some() {
        return None;
    }
    let under = text[n.title_span.end..].find('\n').map_or(text.len(), |i| n.title_span.end + i + 1);
    let end = text[under..].find('\n').map_or(text.len(), |i| under + i);
    let indent = &line[..line.len() - t.len()];
    let atx = format!("{}{} {}", indent, "#".repeat(n.level.unwrap_or(1)), n.title);
    Some((Span { start: n.title_span.start, end }, atx))
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
    let block = file_block(&f).cloned();
    let (mut raw, fm_span) = match &block {
        Some(b) => (b.frontmatter_raw.clone(), b.frontmatter_span),
        None => (String::new(), None),
    };
    // rewrite the line for `key` inside raw
    let mut lines: Vec<String> = raw.split_inclusive('\n').map(|s| s.to_string()).collect();
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

/// The block whose frontmatter a file holds: a block file's root, its
/// first node (§4.9); root.md's, the implicit Root, whose frontmatter holds
/// vault-level properties.
fn file_block(f: &crate::parse::ParsedFile) -> Option<&crate::parse::Block> {
    let root = &f.nodes[f.root_node];
    root.block.as_ref().or_else(|| f.nodes[*root.children.first()?].block.as_ref())
}

/// All top-level keys of a block's frontmatter, for the property editor.
pub fn frontmatter_lines(vault: &Vault, file: usize) -> Vec<(String, String, bool)> {
    // (key, value, editable) — unknown-structure lines are read-only (§10.6)
    let Some(b) = file_block(&vault.tree.files[file]) else {
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
/// Returns how many nodes went, the node itself included, for the TUI to
/// say (§10.1).
pub fn delete_subtree(vault: &mut Vault, r: NRef) -> std::io::Result<usize> {
    // a node's conflict copies go with it, or the first would pair with
    // the node before it (§12.5); a copy goes alone, as keeping ours
    // sends it
    let copies: Vec<Id> = conflict_copies(&vault.tree, r)
        .into_iter()
        .filter_map(|c| vault.tree.node(c).block.as_ref()?.id.clone())
        .collect();
    let mut count = delete_one(vault, r)?;
    for id in copies {
        if let Some(c) = vault.tree.block_by_id(&id) {
            count += delete_one(vault, c)?;
        }
    }
    Ok(count)
}

/// `delete_subtree` for `r` alone.
fn delete_one(vault: &mut Vault, r: NRef) -> std::io::Result<usize> {
    let n = vault.tree.node(r);
    if n.kind == Kind::Root {
        return Err(io_err("cannot delete the root"));
    }
    let mut count = 0;
    vault.tree.walk(vault.tree.resolved_child(r), &mut |_, _| count += 1);
    if n.is_block() || n.is_embed() {
        let target = if n.is_embed() { vault.tree.resolved_child(r) } else { r };
        if target == r && n.is_embed() {
            // broken embed: just the line. So for a second embed of a
            // block, which renders as broken (§6.2): the block, and the
            // embed it is stitched in at, are not this line's
            let span = embed_line_span(&vault.tree, r);
            remove_span(vault, r.0, span)?;
            return Ok(1);
        }
        // the block itself first: it leaves by its own embed
        let ids = nested_block_ids(vault, target);
        if let Some((own, nested)) = ids.split_first() {
            trash_block(vault, own)?;
            trash_nested(vault, nested)?;
        }
        return Ok(count);
    }
    let ids = plain_remove(vault, r)?;
    trash_nested(vault, &ids)?;
    Ok(count)
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
    remove_span(vault, r.0, span)?;
    Ok(ids)
}

/// Remove the embed line a block is stitched in at (if it is still
/// anywhere) and move its file to the trash.
fn trash_block(vault: &mut Vault, id: &Id) -> std::io::Result<()> {
    if let Some(e) = vault.tree.embed_of(id) {
        let span = embed_line_span(&vault.tree, e);
        remove_span(vault, e.0, span)?;
    }
    if let Some(b) = vault.tree.block_by_id(id) {
        vault.trash_file(b.0)?;
    }
    Ok(())
}

/// Move the files of blocks nested in a removed subtree to the trash. Their
/// embeds went with the lines and files that held them: an embed of one
/// still elsewhere is a second embed, a line of its own (§6.2), and stays.
fn trash_nested(vault: &mut Vault, ids: &[Id]) -> std::io::Result<()> {
    for id in ids {
        if let Some(b) = vault.tree.block_by_id(id) {
            vault.trash_file(b.0)?;
        }
    }
    Ok(())
}

/// The part of an embed's span that is the embed itself: its line and the
/// blank lines after it. Lines nested under an embed are a diagnostic
/// (§4.7) that the reading pane shows in the parent, so they stay there
/// when the embed goes: nothing else holds a copy of them (§11.5).
fn embed_line_span(tree: &crate::tree::Tree, e: NRef) -> Span {
    let n = tree.node(e);
    let text = tree.text_of(e);
    let end = n.span.end.min(text.len());
    let mut at = text[n.title_span.end..end].find('\n').map_or(end, |i| n.title_span.end + i + 1);
    for l in text[at..end].split_inclusive('\n') {
        if !l.trim().is_empty() {
            break;
        }
        at += l.len();
    }
    Span { start: n.span.start, end: at }
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
/// that file, so other files' node refs stay valid.
fn remove_span(vault: &mut Vault, file: usize, span: Span) -> std::io::Result<()> {
    let (start, end) = removal_range(&vault.tree.files[file].text, span);
    vault.write_span(file, Span { start, end }, "")
}

/// Yank: render the subtree resolved (the register's text) (§10.3).
pub fn yank(vault: &Vault, r: NRef) -> String {
    render(&vault.tree, r, 1, true)
}

/// Paste rendered text after/before a node as siblings (§10.3 `p`/`P`),
/// clamped by the ordering rule (§3.1).
pub fn paste(vault: &mut Vault, at: NRef, text: &str, after: bool) -> std::io::Result<Placed> {
    if vault.tree.node(at).kind == Kind::Root {
        return Err(io_err("cannot paste beside the root"));
    }
    // a block root stands in its parent file as its embed
    let at = stand_in(&vault.tree, at);
    let Some(parent) = vault.tree.node(at).parent.map(|p| (at.0, p)) else {
        return Err(io_err("cannot paste beside the root"));
    };
    let pos = vault.tree.raw_children(parent).iter().position(|&k| k == at).unwrap_or(0);
    let shifted = shift_document(text, vault.tree.level(parent), child_indent(&vault.tree, parent));
    let (nodes, clamped) = place(vault, &[], parent, pos + after as usize, &shifted)?;
    Placed::of(&nodes, 0, clamped)
}

/// Parse a standalone document and re-emit it as children of a node whose
/// level is `parent_level` (the root's is 0), at `indent`: a top-level
/// section becomes a level `parent_level + 1` heading, a top-level item sits
/// in a section of level `parent_level`. Everything inside keeps its position
/// relative to its top-level node.
pub fn shift_document(text: &str, parent_level: usize, indent: usize) -> String {
    let block = crate::parse::Block::new("clip.md", None, None);
    let pf = crate::parse::parse_file("clip.md", text, Some(block));
    let tree = crate::tree::Tree::new(vec![pf]);
    let mut out = String::new();
    let kids = tree.resolved_children(tree.root);
    for (i, k) in kids.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let r = render(&tree, *k, 1, true);
        let first = r.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let t = first.trim_start();
        // render(_, 1) writes a section root at level 1 and an item root in
        // a section of level 1 (its nested sections at 2)
        let ld = match atx_hashes(t) {
            Some(cur_level) => parent_level as isize + 1 - cur_level as isize,
            None => parent_level as isize - 1,
        };
        let cur_indent = first.len() - t.len();
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
/// Fenced code is shifted too, so it stays in its node's region (§3.3), but
/// never re-levelled.
fn shift_lines(raw: &str, level_delta: isize, indent_delta: isize) -> String {
    let mut out = String::new();
    let mut fence: Option<(char, usize)> = None;
    for line in raw.split_inclusive('\n') {
        let l = line.strip_suffix('\n').unwrap_or(line);
        let nl = if line.ends_with('\n') { "\n" } else { "" };
        let in_code = crate::parse::fence_transition(l, &mut fence) || fence.is_some();
        let trimmed = l.trim_start();
        if trimmed.is_empty() {
            out.push_str(nl);
            continue;
        }
        if in_code {
            // only the leading spaces change: a tab, or any indentation
            // inside the code, stays as written
            let code = l.trim_start_matches(' ');
            let cur_indent = l.len() - code.len();
            out.push_str(&" ".repeat((cur_indent as isize + indent_delta).max(0) as usize));
            out.push_str(code);
            out.push_str(nl);
            continue;
        }
        let cur_indent = l.len() - trimmed.len();
        let new_indent = (cur_indent as isize + indent_delta).max(0) as usize;
        if let Some(hashes) = atx_hashes(trimmed) {
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

/// The level of an ATX heading line, given without its indent: `#`+ then a
/// space or the end of the line (§4.2). `#tag` is text, as the parser reads it.
fn atx_hashes(trimmed: &str) -> Option<usize> {
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    let after = &trimmed[hashes..];
    (hashes > 0 && (after.is_empty() || after.starts_with(' '))).then_some(hashes)
}

/// Refile: move a subtree under a new parent as its last child (§6.5).
pub fn refile(vault: &mut Vault, r: NRef, dest: NRef) -> std::io::Result<Placed> {
    let dest = write_target(&vault.tree, dest)?;
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
            break; // embed cycle: `check` reports it (§6.2)
        }
        anc = resolved_parent(&vault.tree, a);
    }
    // a block moves by its embed line (§6.5); a plain node by its span, in
    // its on-disk spelling, so nested blocks travel as their embed lines
    let moving = stand_in(&vault.tree, r);
    if vault.tree.node(moving).is_block() {
        return Err(io_err("block has no embed"));
    }
    // with its conflict copies, or with the node a copy is of (§12.5)
    let run = pair_run(&vault.tree, moving);
    if run.iter().any(|&m| m != moving && within(&vault.tree, dest, m)) {
        return Err(io_err(PAIR_INTO_ITSELF));
    }
    let doc = run_doc(&vault.tree, &run, dest);
    let (nodes, clamped) = place(vault, &run, dest, usize::MAX, &doc)?;
    Placed::of(&nodes, run.iter().position(|&m| m == moving).unwrap_or(0), clamped)
}

/// A node and its conflict copy move as one (§12.5): neither into the
/// other.
const PAIR_INTO_ITSELF: &str = "cannot move a node and its conflict copy into either of them";

/// `run` (a node and its conflict copies, §12.5, as they stand in their
/// parent's file) as a document of children of `parent`: each in its
/// on-disk spelling, shifted there, a blank line before a heading or
/// where there was one.
fn run_doc(tree: &crate::tree::Tree, run: &[NRef], parent: NRef) -> String {
    let (level, indent) = (tree.level(parent), child_indent(tree, parent));
    let mut out = String::new();
    for (i, &m) in run.iter().enumerate() {
        if i > 0 {
            let (sp, text) = (tree.node(run[i - 1]).span, tree.text_of(run[i - 1]));
            if tree.node(m).kind == Kind::Section || text[sp.start..sp.end.min(text.len())].ends_with("\n\n") {
                out.push('\n');
            }
        }
        out.push_str(&shift_document(&render(tree, m, 1, false), level, indent));
    }
    out
}

/// Where a verb put the node it moved or made, by position — never found
/// again by its title (§3.4): the node as the outline shows it (a block,
/// not its embed), and whether the ordering rule (§3.1), not the verb,
/// chose the place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub node: NRef,
    pub clamped: bool,
}

impl Placed {
    /// A verb that left `r` where it was.
    fn stayed(tree: &crate::tree::Tree, r: NRef) -> Placed {
        Placed { node: tree.resolved_child(r), clamped: false }
    }

    /// The `i`-th of the nodes `place` put (a run's, in order: the one a
    /// verb moved among its conflict copies).
    fn of(nodes: &[NRef], i: usize, clamped: bool) -> std::io::Result<Placed> {
        let node = *nodes.get(i).ok_or_else(|| io_err("the placed node is not where it was written"))?;
        Ok(Placed { node, clamped })
    }
}

/// Where a dragged node is dropped (§10.3): before a node, as its sibling,
/// or into it, as its last child. Both are clamped by the ordering rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop {
    Before,
    Into,
}

/// Move `r` relative to `target` (§10.3 drag and drop). Refuses a move into
/// `r`'s own subtree.
pub fn move_node(vault: &mut Vault, r: NRef, target: NRef, drop: Drop) -> std::io::Result<Placed> {
    let target = vault.tree.resolved_child(target);
    if target == vault.tree.resolved_child(r) {
        return Ok(Placed::stayed(&vault.tree, r));
    }
    match drop {
        Drop::Into => refile(vault, r, target),
        Drop::Before => {
            let t = stand_in(&vault.tree, target);
            let Some(parent) = vault.tree.node(t).parent.map(|p| (t.0, p)) else {
                return Err(io_err("cannot move beside the root"));
            };
            // the destination parent must not sit inside the moving subtree
            if within(&vault.tree, parent, r) {
                return Err(io_err("cannot move a node into itself"));
            }
            let moving = stand_in(&vault.tree, r);
            // with its conflict copies, or with the node a copy is of
            // (§12.5); before either of them, it stays where it is
            let run = pair_run(&vault.tree, moving);
            if run.contains(&t) {
                return Ok(Placed::stayed(&vault.tree, r));
            }
            if run.iter().any(|&m| m != moving && within(&vault.tree, parent, m)) {
                return Err(io_err(PAIR_INTO_ITSELF));
            }
            // the target's index among the children `place` sees: without
            // the moving nodes, which may come before it
            let idx = vault
                .tree
                .raw_children(parent)
                .into_iter()
                .filter(|k| !run.contains(k))
                .position(|k| k == t)
                .unwrap_or(0);
            let doc = run_doc(&vault.tree, &run, parent);
            let (nodes, clamped) = place(vault, &run, parent, idx, &doc)?;
            Placed::of(&nodes, run.iter().position(|&m| m == moving).unwrap_or(0), clamped)
        }
    }
}

/// Whether `a` is `r` or inside its subtree, following embeds up.
fn within(tree: &crate::tree::Tree, a: NRef, r: NRef) -> bool {
    let r = tree.resolved_child(r);
    let mut cur = Some(tree.resolved_child(a));
    let mut guard = 0;
    while let Some(c) = cur {
        if c == r {
            return true;
        }
        guard += 1;
        if guard > 10_000 {
            return false;
        }
        cur = resolved_parent(tree, c);
    }
    false
}

// ---------------------------------------------------------------- placement

/// The node that stands for `r` among its parent's children in a file: a
/// block root's embed (§4.7), else `r` itself.
fn stand_in(tree: &crate::tree::Tree, r: NRef) -> NRef {
    if r.0 != tree.root.0 && tree.node(r).is_block() {
        if let Some(e) = resolved_parent(tree, r) {
            if tree.node(e).is_embed() {
                return e;
            }
        }
    }
    r
}

/// The node a verb writes into at `r`: a block's own embed is the block
/// (§4.7). An embed that resolves to nothing, broken or a second embed of
/// a block (§6.2), is only a line of the file that holds it: it has no
/// checkbox, properties or children of its own, so the verb is refused
/// rather than writing them into that file.
fn write_target(tree: &crate::tree::Tree, r: NRef) -> std::io::Result<NRef> {
    let t = tree.resolved_child(r);
    if tree.node(t).is_embed() {
        return Err(io_err("a broken or duplicate embed has no node to write to"));
    }
    Ok(t)
}

/// Indent of a child of `parent` (§3.1): items nest under items, everything
/// under a section stays at the section's own indent. Measured from the
/// parent's written indent, not its derived one: nesting written with tabs
/// or 4 spaces is read (§4.2), and a line at the derived child indent could
/// fall short of such a parent and parse as its sibling.
fn child_indent(tree: &crate::tree::Tree, parent: NRef) -> usize {
    let n = tree.node(parent);
    match n.kind {
        Kind::Item => n.indent + 2,
        Kind::Section => n.indent,
        Kind::Root => 0,
    }
}

/// Spellings of a document's top-level nodes, in order. A heading embed is
/// a section, a bare embed an item (§4.7).
fn top_kinds(doc: &str) -> Vec<Kind> {
    let pf = crate::parse::parse_file("clip.md", doc, None);
    pf.nodes[pf.root_node]
        .children
        .iter()
        .map(|&c| pf.nodes[c].kind)
        .collect()
}

/// The indent a document's first top-level node is written at.
fn top_indent(doc: &str) -> Option<usize> {
    let pf = crate::parse::parse_file("clip.md", doc, None);
    pf.nodes[pf.root_node].children.first().map(|&c| pf.nodes[c].indent)
}

/// Clamp a wanted child index by the ordering rule `(text | item)*
/// section*` (§3.1): items no later than the first section child, sections
/// no earlier. `kids` are the parent's child nodes without the one moving.
fn clamp_index(tree: &crate::tree::Tree, kids: &[NRef], want: usize, kinds: &[Kind]) -> usize {
    let first_section = kids
        .iter()
        .position(|&k| tree.node(k).kind == Kind::Section)
        .unwrap_or(kids.len());
    let lo = if kinds.contains(&Kind::Section) { first_section } else { 0 };
    let hi = if kinds.contains(&Kind::Item) { first_section } else { kids.len() };
    want.min(kids.len()).clamp(lo, hi)
}

/// The level of `k` as written, if it is a section. A written level is
/// honoured (§4.7): a section written after it deeper than that is its
/// child, and one written before it shallower than that takes it (and
/// every section after it) as a child (§3.1). The level `k`'s position
/// gives can be deeper: under an item, sections may be written shallower.
fn written_level(tree: &crate::tree::Tree, k: Option<NRef>) -> Option<usize> {
    k.map(|k| tree.node(k)).filter(|n| n.kind == Kind::Section).map(|n| n.level.unwrap_or(1))
}

/// `doc` (shifted to a child position) with its top-level sections, and
/// everything under them, re-levelled to stay siblings of the sections
/// around them (`written_level`): no deeper than `prev`, the sibling
/// section they are written after, and no shallower than `next`, the one
/// they are written before.
fn level_among(tree: &crate::tree::Tree, prev: Option<NRef>, next: Option<NRef>, doc: &str) -> String {
    let (most, least) = (written_level(tree, prev), written_level(tree, next));
    if most.is_none() && least.is_none() {
        return doc.to_string();
    }
    let pf = crate::parse::parse_file("clip.md", doc, None);
    let tops = &pf.nodes[pf.root_node].children;
    // a document's sections come after its items (§3.1)
    let Some(first) = tops.iter().map(|&c| &pf.nodes[c]).find(|n| n.kind == Kind::Section) else {
        return doc.to_string();
    };
    // indentation closes a section before its level does (§4.2): a sibling
    // written deeper than the sections, before them, or shallower, after
    // them, is no parent or child of theirs, whatever its level
    let most = most.filter(|_| prev.is_some_and(|p| tree.node(p).indent <= first.indent));
    let least = least.filter(|_| next.is_some_and(|k| tree.node(k).indent >= first.indent));
    let have = tops.last().and_then(|&c| pf.nodes[c].level).unwrap_or(1);
    let want = most.map_or(have, |m| have.min(m)).max(least.unwrap_or(1));
    if want == have {
        return doc.to_string();
    }
    let at = first.span.start;
    format!("{}{}", &doc[..at], shift_lines(&doc[at..], want as isize - have as isize, 0))
}

/// Write `doc` (already shifted to a child position of `parent`) as the
/// `want`-th child node of `parent`, clamped by the ordering rule (§3.1).
/// The `moving` nodes (siblings, in order: a node and its conflict copies,
/// §12.5), have their own lines removed in the same edit: text children
/// around them stay where they are. The insertion is written before any
/// removal in another file, so a failure never loses text. Returns the
/// document's top-level nodes where they are now, found by the position
/// they were written at, and whether the ordering rule moved them from
/// the wanted index.
fn place(
    vault: &mut Vault,
    moving: &[NRef],
    parent: NRef,
    want: usize,
    doc: &str,
) -> std::io::Result<(Vec<NRef>, bool)> {
    let tree = &vault.tree;
    let pnode = tree.node(parent);
    let kids: Vec<NRef> = tree
        .raw_children(parent)
        .into_iter()
        .filter(|k| !moving.contains(k))
        .collect();
    let kinds = top_kinds(doc);
    if kinds.is_empty() {
        return Err(io_err("nothing to place"));
    }
    let clamped = clamp_index(tree, &kids, want, &kinds);
    // nothing goes between a conflict copy and the node before it, which
    // it would pair with (§12.5): past the copies, unless the node they
    // follow is the one moving, back in its place
    let mut idx = clamped;
    let orig = tree.raw_children(parent);
    let back = |k: NRef| {
        let before = orig.iter().position(|&o| o == k).and_then(|i| i.checked_sub(1)).map(|i| orig[i]);
        before.is_some_and(|b| moving.contains(&b))
    };
    if !kids.get(idx).is_some_and(|&k| back(k)) {
        let copy_at = |i: usize| kids.get(i).is_some_and(|&k| is_copy(tree, k));
        while copy_at(idx) && clamp_index(tree, &kids, idx + 1, &kinds) == idx + 1 {
            idx += 1;
        }
    }
    let first = kinds[0];
    let last = *kinds.last().unwrap();
    let file = parent.0;
    let mut text = tree.files[file].text.clone();
    // the parent's content as it will be, without the moving nodes
    let content: Vec<crate::parse::Content> = pnode
        .content
        .iter()
        .copied()
        .filter(|c| !matches!(c, crate::parse::Content::Node(n) if moving.contains(&(file, *n))))
        .collect();
    // positions in the original text, then adjusted for a same-file removal
    let next = kids.get(idx).copied();
    let mut pos = match next {
        Some(k) => tree.node(k).span.start,
        None => pnode.span.end.min(text.len()),
    };
    // appending after the last child: is the list there tight? The last
    // item's own trailing blank may just separate the parent from what
    // follows it, so looseness is read between the items before it.
    let item_kids: Vec<&crate::parse::Node> = kids
        .iter()
        .map(|&k| tree.node(k))
        .filter(|n| n.kind == Kind::Item)
        .collect();
    let tight_after = match content.last() {
        Some(crate::parse::Content::Node(k)) => {
            let kn = &tree.files[file].nodes[*k];
            kn.kind == Kind::Item
                && match item_kids.len() {
                    0 | 1 => true,
                    n => !item_kids[n - 2].span.text(&text).ends_with("\n\n"),
                }
        }
        Some(crate::parse::Content::Text(_)) => false,
        None => pnode.kind == Kind::Item,
    };
    // inserting before an item: after another item, the list is as tight as
    // the text there says (read below); starting the list, it is read in
    // the list as found, the moving nodes still in it: between `next` and
    // the item after it, or else between the last moving node and `next`
    // right after it. A list of one item is tight.
    let is_item = |c: Option<&crate::parse::Content>| {
        matches!(c, Some(crate::parse::Content::Node(k)) if tree.files[file].nodes[*k].kind == Kind::Item)
    };
    let next_at = next.and_then(|k| content.iter().position(|c| *c == crate::parse::Content::Node(k.1)));
    let starts_run = next_at.is_some_and(|i| i == 0 || !is_item(content.get(i - 1)));
    let found = &pnode.content;
    let found_at = next.and_then(|k| found.iter().position(|c| *c == crate::parse::Content::Node(k.1)));
    let no_blank_after = |k: NRef| {
        let sp = tree.node(k).span;
        !text[sp.start..sp.end.min(text.len())].ends_with("\n\n")
    };
    let run_tight = match (next, found_at) {
        (Some(k), Some(i)) if is_item(found.get(i + 1)) => no_blank_after(k),
        (Some(_), Some(i)) => match moving.last() {
            Some(&m) if i > 0 && m.0 == file && found[i - 1] == crate::parse::Content::Node(m.1) => {
                no_blank_after(m)
            }
            _ => true,
        },
        _ => true,
    };
    let next_kind = next.map(|k| tree.node(k).kind);
    // the last first, so the spans before it stay where they are
    for &m in moving.iter().rev().filter(|m| m.0 == file) {
        let (start, end) = removal_range(&text, tree.node(m).span);
        text = format!("{}{}", &text[..start], &text[end..]);
        pos = after_removal(pos, start, end);
    }
    // before a sibling written deeper than the node (tab or 4-space
    // nesting is read, §4.2), the node goes at that sibling's indent: any
    // shallower, the sibling would parse as its child
    let doc = match (next.map(|k| tree.node(k).indent), top_indent(doc)) {
        (Some(want), Some(have)) if want > have => shift_lines(doc, 0, (want - have) as isize),
        _ => doc.to_string(),
    };
    let prev = idx.checked_sub(1).map(|i| kids[i]);
    let doc = level_among(tree, prev, next, &doc);
    let body = doc.trim_end_matches('\n');
    let tops = top_starts(body);
    // the insertion, and where `body` starts in it
    let (insertion, at) = match next_kind {
        Some(nk) => {
            // before the next child node: keep a blank line before a
            // heading and after anything that is not a tight item run
            let lead = if first == Kind::Section && pos > 0 && !text[..pos].ends_with("\n\n") {
                "\n"
            } else {
                ""
            };
            let tight = last == Kind::Item
                && nk == Kind::Item
                && if starts_run { run_tight } else { !text[..pos].ends_with("\n\n") };
            (format!("{}{}\n{}", lead, body, if tight { "" } else { "\n" }), lead.len())
        }
        None => {
            while pos > 0 && text.as_bytes()[pos - 1] == b'\n' {
                pos -= 1;
            }
            let sep = if pos == 0 {
                ""
            } else if first == Kind::Item && tight_after {
                "\n"
            } else {
                "\n\n"
            };
            let nl = if text[pos..].starts_with('\n') { "" } else { "\n" };
            (format!("{}{}{}", sep, body, nl), sep.len())
        }
    };
    let start = pos + at;
    text.insert_str(pos, &insertion);
    vault.write_file_text(file, &text)?;
    // the other file's node refs are untouched by the write above; each
    // removal re-parses that file, so the spans are read first
    let away: Vec<(usize, Span)> =
        moving.iter().filter(|m| m.0 != file).map(|&m| (m.0, vault.tree.node(m).span)).collect();
    for (f, span) in away.into_iter().rev() {
        remove_span(vault, f, span)?;
    }
    // `file` is as written: each node starts where its line went in
    let f = &vault.tree.files[file];
    let placed = tops
        .iter()
        .filter_map(|&t| f.nodes.iter().position(|n| n.kind != Kind::Root && n.title_span.start == start + t))
        .map(|i| vault.tree.resolved_child((file, i)))
        .collect();
    Ok((placed, clamped != want.min(kids.len())))
}

/// Where each of a document's top-level nodes starts in it.
fn top_starts(doc: &str) -> Vec<usize> {
    let pf = crate::parse::parse_file("clip.md", doc, None);
    pf.nodes[pf.root_node].children.iter().map(|&c| pf.nodes[c].title_span.start).collect()
}

/// Archive: refile under the top-level `Archive` section (§6.5).
pub fn archive(vault: &mut Vault, r: NRef) -> std::io::Result<Placed> {
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
            // any open task below keeps it, one in a conflict copy too,
            // which task counts leave out (§3.5)
            let mut open = false;
            t.walk(r, &mut |t, c| open |= t.node(c).task == Some(TaskState::Open));
            if !open {
                to_clear.push(r);
            }
        }
    });
    // a node's conflict copies go with it (§12.5): one whose copies are
    // not all cleared too stays, or they would pair with the node before
    let to_clear: Vec<NRef> = to_clear
        .iter()
        .copied()
        .filter(|&r| conflict_copies(&vault.tree, r).iter().all(|c| to_clear.contains(c)))
        .collect();
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
    let mut nested: Vec<Id> = Vec::new();
    for &r in &tops {
        let n = vault.tree.node(r);
        match n.block.as_ref().and_then(|b| b.id.clone()) {
            Some(id) => {
                // the block leaves by its own embed; blocks embedded in its
                // file go to the trash with it, as delete_subtree sends them
                let ids = nested_block_ids(vault, r);
                nested.extend(ids.into_iter().filter(|i| *i != id));
                blocks.push(id);
            }
            None => plain.push(r),
        }
    }
    // plain spans: per file, from the end backwards, so earlier spans stay valid
    plain.sort_by_key(|&(f, n)| (f, std::cmp::Reverse(vault.tree.files[f].nodes[n].span.start)));
    let mut done = 0;
    for r in plain {
        let ids = plain_remove(vault, r)?;
        nested.extend(ids);
        done += 1;
    }
    let n_block_tops = tops.len() - done;
    for id in blocks {
        trash_block(vault, &id)?;
    }
    trash_nested(vault, &nested)?;
    Ok(done + n_block_tops)
}

/// A node's parent in the resolved tree: within its file, or — for a block
/// root — the embed that stitches it in (§4.7).
fn resolved_parent(tree: &crate::tree::Tree, r: NRef) -> Option<NRef> {
    let p = tree.node(r).parent?;
    let pr = (r.0, p);
    if tree.node(pr).kind == Kind::Root && r.0 != tree.root.0 {
        tree.embed_of(tree.node(r).block.as_ref()?.id.as_ref()?)
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
    // the checkbox stays on the title line: it is the state (§4.5)
    let mut file_text = format!("---\nid: {}\n---\n\n", id);
    file_text.push_str(&body);
    let prefix = vault.unique_prefix(&id);
    let fname = crate::ident::filename(&prefix, &slug(&n.title));
    // the embed cannot be written into a file changed on disk (§11.2), and
    // a block file without it would be embedded nowhere: check first
    let file = r.0;
    vault.check_unchanged(file)?;
    let full = vault.dir.join(&fname);
    crate::vault::atomic_write(&full, &file_text)?;
    // replace the node's span with an embed in the node's form (§6.1.3,
    // §4.7), keeping the blank lines that separated it from what follows
    let span = n.span;
    let indent = " ".repeat(n.indent);
    // at its level, but kept a sibling of the sections around it as they
    // are written: no deeper than the one before it, which would take the
    // embed as its child, nor shallower than the one after it
    let kids = n.parent.map(|p| vault.tree.raw_children((file, p))).unwrap_or_default();
    let at = kids.iter().position(|&c| c == r);
    let prev = at.and_then(|i| i.checked_sub(1)).map(|i| kids[i]);
    let next = at.and_then(|i| kids.get(i + 1).copied());
    let mut embed = match n.kind {
        Kind::Section => {
            let line = format!("{}{} ![[{}]]\n", indent, "#".repeat(vault.tree.level(r)), id);
            level_among(&vault.tree, prev, next, &line)
        }
        _ => format!("{}![[{}]]\n", indent, id),
    };
    let old = span.text(&vault.tree.files[file].text);
    let trailing = old.len() - old.trim_end_matches('\n').len();
    for _ in 1..trailing {
        embed.push('\n');
    }
    if let Err(e) = vault.write_span(file, span, &embed) {
        // changed in between after all: nothing refers to the new file yet,
        // and the node is still in its parent
        let _ = std::fs::remove_file(&full);
        return Err(e);
    }
    vault.add_file(&fname, &file_text);
    Ok(id)
}

/// Set a property; on a plain node the first one makes it a block (§3.2,
/// §6.1). Returns the block that holds the property.
pub fn set_property(vault: &mut Vault, r: NRef, key: &str, value: &str) -> std::io::Result<NRef> {
    let target = if vault.tree.node(r).is_block() {
        r
    } else if vault.tree.node(r).is_embed() {
        write_target(&vault.tree, r)?
    } else {
        // make_block reloads, so `r` is stale: find the new block by its id
        let id = make_block(vault, r)?;
        vault
            .tree
            .block_by_id(&id)
            .ok_or_else(|| io_err("block not found after make_block"))?
    };
    let id = vault.tree.node(target).block.as_ref().and_then(|b| b.id.clone());
    set_frontmatter_key(vault, target.0, key, Some(value))?;
    Ok(id.and_then(|id| vault.tree.block_by_id(&id)).unwrap_or(target))
}

// --------------------------------------------------------------- move/spelling

/// Move a node among its siblings (§10.3 `J`/`K`). A block moves by its
/// embed line, so blocks and plain nodes swap freely in the parent file.
/// A node and the conflict copies after it are one (§12.5): they move
/// together, and a node moves past them together, so no copy is left
/// after another node, which it would pair with.
pub fn move_sibling(vault: &mut Vault, r: NRef, down: bool) -> std::io::Result<()> {
    // a block root stands in its parent file as its embed
    let r = stand_in(&vault.tree, r);
    if vault.tree.node(r).is_block() {
        return Ok(());
    }
    let parent = match vault.tree.node(r).parent {
        Some(p) => (r.0, p),
        None => return Ok(()),
    };
    let siblings = vault.tree.raw_children(parent);
    let pos = match siblings.iter().position(|&s| s == r) {
        Some(p) => p,
        None => return Ok(()),
    };
    let run = |i: usize| paired_run(&vault.tree, &siblings, i);
    let (a, b) = run(pos);
    let other = if down {
        if b + 1 >= siblings.len() {
            return Ok(());
        }
        run(b + 1)
    } else {
        match a.checked_sub(1) {
            Some(p) => run(p),
            None => return Ok(()),
        }
    };
    // the ordering rule (§3.1): an item never goes below a section sibling
    let (rk, ok) = (vault.tree.node(siblings[a]).kind, vault.tree.node(siblings[other.0]).kind);
    if rk != ok {
        return Err(io_err(if rk == Kind::Item {
            "an item cannot move below a section"
        } else {
            "a section cannot move above an item"
        }));
    }
    let file = r.0;
    let text = vault.tree.files[file].text.clone();
    // swap the runs' own lines; trailing blank lines stay where they are,
    // so the list's spacing is unchanged
    let core = |sp: Span| {
        let end = sp.end.min(text.len());
        let mut e = end;
        while e >= sp.start + 2 && &text.as_bytes()[e - 2..e] == b"\n\n" {
            e -= 1;
        }
        Span { start: sp.start, end: e }
    };
    let (fr, sr) = if down { ((a, b), other) } else { (other, (a, b)) };
    let lines = |(s, e): (usize, usize)| Span {
        start: vault.tree.node(siblings[s]).span.start,
        end: core(vault.tree.node(siblings[e]).span).end,
    };
    let (first, second) = (lines(fr), lines(sr));
    let with_nl = |t: &str| {
        if t.ends_with('\n') {
            t.to_string()
        } else {
            format!("{}\n", t)
        }
    };
    // The written level and indent decide the parent: a sibling written
    // deeper than the one after it (a skipped heading level, §4.7, or a
    // wider indent) would nest under that one once below it, and J/K keep
    // every other node's parent (§15.6). So the nodes moving down are
    // written at the level and indent of the one moving up, their subtrees
    // re-levelled with them, as a moved node is (§4.2).
    let (fnode, snode) = (vault.tree.node(siblings[fr.0]), vault.tree.node(siblings[sr.0]));
    let lower = if (fnode.level, fnode.indent) == (snode.level, snode.indent) {
        with_nl(first.text(&text))
    } else {
        let base = snode.level.unwrap_or_else(|| vault.tree.level(siblings[fr.0]));
        let pad = " ".repeat(snode.indent);
        let mut out = String::new();
        for i in fr.0..=fr.1 {
            let k = siblings[i];
            out.extend(
                render(&vault.tree, k, base, false)
                    .lines()
                    .map(|l| if l.is_empty() { "\n".to_string() } else { format!("{}{}\n", pad, l) }),
            );
            // the blank lines between a node and its copy stay
            if i < fr.1 {
                let next = vault.tree.node(siblings[i + 1]).span.start;
                out.push_str(&text[core(vault.tree.node(k).span).end..next]);
            }
        }
        out
    };
    let mut new_text = format!(
        "{}{}{}{}",
        &text[..first.start],
        with_nl(second.text(&text)),
        &text[first.end..second.start],
        lower,
    );
    let rest = &text[second.end..];
    new_text.push_str(rest);
    vault.write_file_text(file, &new_text)
}

/// Whether the child `k` is a conflict copy, which pairs with the node
/// before it (§12.4).
fn is_copy(tree: &crate::tree::Tree, k: NRef) -> bool {
    tree.node(tree.resolved_child(k)).conflict().is_some()
}

/// The run of `kids` that the `i`-th is in: a node and the conflict copies
/// right after it, which pair with it and each other (§12.4), as indices.
fn paired_run(tree: &crate::tree::Tree, kids: &[NRef], i: usize) -> (usize, usize) {
    let mut a = i;
    while a > 0 && is_copy(tree, kids[a]) {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < kids.len() && is_copy(tree, kids[b + 1]) {
        b += 1;
    }
    (a, b)
}

/// The run `m` (a node as it stands in its parent's file, §4.7) is in
/// there: a node and its conflict copies, which a verb that moves or
/// deletes the one takes with it, or a copy left behind would pair with
/// the node before it (§12.5). `m` alone when it is in no pair.
fn pair_run(tree: &crate::tree::Tree, m: NRef) -> Vec<NRef> {
    let Some(p) = tree.node(m).parent else { return vec![m] };
    let kids = tree.raw_children((m.0, p));
    let Some(i) = kids.iter().position(|&k| k == m) else { return vec![m] };
    let (a, b) = paired_run(tree, &kids, i);
    kids[a..=b].to_vec()
}

/// The conflict pair `r` is in (§12.4): the node, then its copies, as the
/// outline shows them; `r` alone when it is in none.
pub fn conflict_pair(tree: &crate::tree::Tree, r: NRef) -> Vec<NRef> {
    let m = stand_in(tree, tree.resolved_child(r));
    pair_run(tree, m).into_iter().map(|k| tree.resolved_child(k)).collect()
}

/// The conflict copies of `r` (§12.4), as the outline shows them: none
/// when it is a copy itself.
pub fn conflict_copies(tree: &crate::tree::Tree, r: NRef) -> Vec<NRef> {
    let r = tree.resolved_child(r);
    match conflict_pair(tree, r).split_first() {
        Some((&o, copies)) if o == r && tree.node(r).conflict().is_none() => copies.to_vec(),
        _ => Vec::new(),
    }
}

/// Toggle spelling section ↔ item (§10.3 `~`). The checkbox, if any, is kept,
/// and the subtree is re-indented and re-levelled for the new spelling. The
/// node moves to its parent's boundary if it has to (§3.1), so no sibling
/// changes parent; a block's embed changes form and moves with it (§4.7).
pub fn toggle_spelling(vault: &mut Vault, r: NRef) -> std::io::Result<bool> {
    let r = vault.tree.resolved_child(r);
    let n = vault.tree.node(r);
    if n.kind == Kind::Root {
        return Ok(false);
    }
    let to_section = n.kind == Kind::Item;
    // a node in a conflict pair keeps its spelling: moved to its boundary
    // it would leave its copy, or a copy its node, to pair with the node
    // before (§12.5), and keeping theirs would write the other one back
    if conflict_pair(&vault.tree, r).len() > 1 {
        return Err(io_err(PAIR_SPELLING));
    }
    // an embed still here after resolving is broken: there is no block to
    // respell, so only the embed's form changes and its id stays (§4.7)
    if n.is_embed() {
        return respell_embed(vault, r, to_section);
    }
    // `## ---` spelled as an item is `- ---`, a thematic break (§4.4)
    if !to_section && n.task.is_none() {
        item_line(&n.title)?;
    }
    let stand = stand_in(&vault.tree, r);
    let file = r.0;
    let text = vault.tree.files[file].text.clone();
    let span = Span { start: n.span.start, end: n.span.end.min(text.len()) };
    // respelled from the subtree as a move writes it (§4.2): every heading
    // in it ATX, a setext one converted, at the level its position gives,
    // so re-levelling keeps each under the node it was under; at the node's
    // indent, with the blank lines that end its span
    let blanks = span.text(&text).lines().rev().take_while(|l| l.trim().is_empty()).count();
    let rendered: String = crate::render::render_lines(&vault.tree, r, vault.tree.level(r), false)
        .into_iter()
        .map(|l| l.text + "\n")
        .collect();
    let src = shift_lines(&rendered, 0, n.indent as isize) + &"\n".repeat(blanks);
    if stand != r {
        // a block: respell its file's root at level 1, then its embed. The
        // embed cannot be written into a file changed on disk (§11.2), and
        // the block would be left under an embed of the other form (§4.7):
        // check first
        vault.check_unchanged(stand.0)?;
        let respelled = respell(&src, to_section, 1);
        let id = n.block.as_ref().and_then(|b| b.id.clone());
        vault.write_span(file, span, &respelled)?;
        let Some(e) = id.and_then(|id| vault.tree.embed_of(&id)) else {
            return Ok(false);
        };
        return respell_embed(vault, e, to_section);
    }
    let Some(parent) = n.parent.map(|p| (file, p)) else { return Ok(false) };
    let respelled = respell(&src, to_section, vault.tree.level(parent) + 1);
    reposition(vault, r, parent, respelled, to_section)
}

/// Why `~` is refused on either side of a conflict pair (§12.5).
pub const PAIR_SPELLING: &str = "resolve its conflict";

/// Rewrite an embed in the other form (§4.7) — heading for a section-spelled
/// block, bare for an item — moving it to its parent's boundary if needed.
pub fn respell_embed(vault: &mut Vault, e: NRef, to_section: bool) -> std::io::Result<bool> {
    let en = vault.tree.node(e);
    let Some(id) = en.embed.clone() else { return Ok(false) };
    let Some(parent) = en.parent.map(|p| (e.0, p)) else { return Ok(false) };
    let text = &vault.tree.files[e.0].text;
    let old = Span { start: en.span.start, end: en.span.end.min(text.len()) }.text(text);
    let rest = old.find('\n').map(|i| &old[i..]).unwrap_or("\n");
    let indent = " ".repeat(en.indent);
    let line = if to_section {
        format!("{}{} ![[{}]]", indent, "#".repeat(vault.tree.level(parent) + 1), id)
    } else {
        format!("{}![[{}]]", indent, id)
    };
    let new = format!("{}{}", line, rest);
    reposition(vault, e, parent, new, to_section)
}

/// Replace `r`'s span with `new` (the same node, respelled), in place when
/// the ordering rule allows it there, else at the parent's boundary.
fn reposition(
    vault: &mut Vault,
    r: NRef,
    parent: NRef,
    new: String,
    to_section: bool,
) -> std::io::Result<bool> {
    let tree = &vault.tree;
    let kids = tree.raw_children(parent);
    let pos = kids.iter().position(|&k| k == r).unwrap_or(0);
    let others: Vec<NRef> = kids.iter().copied().filter(|&k| k != r).collect();
    let kind = if to_section { Kind::Section } else { Kind::Item };
    if clamp_index(tree, &others, pos, &[kind]) == pos {
        let span = tree.node(r).span;
        let prev = pos.checked_sub(1).map(|i| others[i]);
        let new = level_among(tree, prev, others.get(pos).copied(), &new);
        vault.write_span(r.0, span, &new)?;
        return Ok(false);
    }
    place(vault, &[r], parent, pos, &new).map(|_| true)
}

/// A node's text respelled: its title line as a section at `level` or as an
/// item, and everything under it shifted to match — an item's content sits
/// two columns in, a section's at its own indent, and nested headings stay
/// one below their new parent.
fn respell(span_text: &str, to_section: bool, level: usize) -> String {
    let (first, rest) = match span_text.find('\n') {
        Some(i) => (&span_text[..i], &span_text[i..]),
        None => (span_text, ""),
    };
    let indent_str = &first[..first.len() - first.trim_start().len()];
    let (checkbox, title) = match checkbox_span(first) {
        Some((s, e)) => {
            let state = if &first[s..s + 3] == "[ ]" { "[ ] " } else { "[x] " };
            (state, &first[e..])
        }
        // an empty title (`-`, `#`) has no marker end
        None => ("", marker_end(first).map(|m| &first[m..]).unwrap_or("")),
    };
    let line = if to_section {
        format!("{}{} {}{}", indent_str, "#".repeat(level.max(1)), checkbox, title)
    } else {
        format!("{}- {}{}", indent_str, checkbox, title)
    };
    let (dl, di) = if to_section { (1, -2) } else { (-1, 2) };
    // the rest starts with the newline that ends the title line
    let shifted = shift_lines(&rest[1.min(rest.len())..], dl, di);
    if rest.is_empty() {
        line
    } else {
        format!("{}\n{}", line.trim_end(), shifted)
    }
}

/// Demote: become the last child of the previous sibling node (§10.3 `>`),
/// clamped by the ordering rule (§3.1).
pub fn demote(vault: &mut Vault, r: NRef) -> std::io::Result<Placed> {
    let stayed = Placed::stayed(&vault.tree, r);
    let m = stand_in(&vault.tree, r);
    let Some(parent) = vault.tree.node(m).parent.map(|p| (m.0, p)) else { return Ok(stayed) };
    let kids = vault.tree.raw_children(parent);
    let Some(pos) = kids.iter().position(|&k| k == m) else { return Ok(stayed) };
    // under the node before its conflict pair, which moves as one (§12.5)
    let (a, _) = paired_run(&vault.tree, &kids, pos);
    if a == 0 {
        return Ok(stayed);
    }
    let prev = vault.tree.resolved_child(kids[a - 1]);
    refile(vault, r, prev)
}

/// Promote: become the next sibling of the parent (§10.3 `<`), clamped by the
/// ordering rule (§3.1): an item leaving a section lands before the first
/// section among the parent's siblings. Out of a block's root, the node moves
/// to the parent file beside the block's embed.
pub fn promote(vault: &mut Vault, r: NRef) -> std::io::Result<Placed> {
    let stayed = Placed::stayed(&vault.tree, r);
    let m = stand_in(&vault.tree, r);
    let Some(parent) = vault.tree.node(m).parent.map(|p| (m.0, p)) else { return Ok(stayed) };
    if vault.tree.node(parent).kind == Kind::Root {
        return Ok(stayed);
    }
    // the parent's own position: its embed if it is a block root
    let pstand = stand_in(&vault.tree, parent);
    let Some(grand) = vault.tree.node(pstand).parent.map(|p| (pstand.0, p)) else {
        return Ok(stayed);
    };
    if vault.tree.node(grand).kind == Kind::Root && pstand.0 != vault.tree.root.0 {
        return Err(io_err("block has no embed"));
    }
    let pos = vault.tree.raw_children(grand).iter().position(|&k| k == pstand).unwrap_or(0);
    // with its conflict pair, which moves as one (§12.5)
    let run = pair_run(&vault.tree, m);
    let doc = run_doc(&vault.tree, &run, grand);
    let (nodes, clamped) = place(vault, &run, grand, pos + 1, &doc)?;
    Placed::of(&nodes, run.iter().position(|&k| k == m).unwrap_or(0), clamped)
}

/// Append a plain item child titled `title` under `parent`; returns the new
/// node. Used by `N` in the TUI (§10.3).
pub fn append_child_public(
    vault: &mut Vault,
    parent: NRef,
    title: &str,
) -> std::io::Result<NRef> {
    append_child_line(vault, parent, &item_line(title)?, true)
}

/// The item line titled `title`, refused when it would not read back as
/// one: a title of dashes (`---`, `- -`) makes it a thematic break, which
/// CommonMark reads before a bullet, and body text (§4.4). Checked before
/// anything is written, as the TUI won't create an invalid title (§4.3).
fn item_line(title: &str) -> std::io::Result<String> {
    let line = format!("- {}", title);
    not_a_break(&line)?;
    Ok(line)
}

/// Refuse an item's title line that reads as a thematic break (§4.4).
fn not_a_break(line: &str) -> std::io::Result<()> {
    if crate::parse::is_thematic_break(line.trim_start_matches([' ', '\t'])) {
        return Err(io_err("a title of dashes reads as a thematic break, not an item"));
    }
    Ok(())
}

fn io_err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, msg.to_string())
}
