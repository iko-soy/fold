//! The built-in editor's document model (§5.2, §10.6): a flat list of lines,
//! each tagged with the block that owns it. Edits apply to whichever block
//! the cursor is in; dirty blocks are spliced back one file each.
//!
//! Ownership is carried by the tags, never re-derived from text. Splice of
//! one block collects its owned lines in buffer order, puts nested blocks
//! back as embeds where their title lines sit, re-levels/re-indents, and
//! replaces the block's span atomically.

use crate::ident::Id;
use crate::parse::{Kind, TaskState};
use crate::tree::NRef;
use crate::vault::Vault;
use std::collections::BTreeMap;

/// One buffer line and its owner.
#[derive(Debug, Clone)]
pub struct EditLine {
    pub text: String,
    /// The owning block: file index + the block-root node title (for the
    /// status line) + id if it has one. `file` identifies where the line
    /// will be written.
    pub owner: Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Owner {
    pub file: usize,
    /// The block-root node's title-line offset order within the file
    /// (0 = the file's own block/root). Used only for grouping.
    pub block_ord: usize,
}

/// A block's ownership extent inside the buffer, computed on demand.
pub struct EditBuffer {
    pub lines: Vec<EditLine>,
    /// Owner → block identity (title for the status line, id if any).
    pub owners: BTreeMap<Owner, OwnerInfo>,
    /// Owners whose lines changed since last save.
    pub dirty: Vec<Owner>,
    /// blake3 of each file as read when the buffer was built (§5.2.5).
    base_hashes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OwnerInfo {
    pub title: String,
    pub id: Option<Id>,
    /// The block-root node in the tree at buffer-build time.
    pub nref: NRef,
    /// The level/indent the block's text was shifted by when inlined into
    /// the buffer (inverse applied on splice, §5.2.3).
    pub level: usize,
    pub indent: usize,
}

impl EditBuffer {
    /// Build the editing buffer for `render(node, 1, true)` (§5.2): every
    /// line tagged with its owning block.
    pub fn build(vault: &Vault, root: NRef) -> EditBuffer {
        let mut lines = Vec::new();
        let mut owners: BTreeMap<Owner, OwnerInfo> = BTreeMap::new();
        let mut block_ord = 0usize;
        build_node(
            vault,
            root,
            1,
            0,
            &mut block_ord,
            &mut lines,
            &mut owners,
            &mut Vec::new(),
        );
        let base_hashes = vault
            .tree
            .files
            .iter()
            .enumerate()
            .map(|(i, _)| vault.hash_of(i).to_string())
            .collect();
        EditBuffer {
            lines,
            owners,
            dirty: Vec::new(),
            base_hashes,
        }
    }

    pub fn owner_at(&self, line: usize) -> Owner {
        self.lines
            .get(line.min(self.lines.len().saturating_sub(1)))
            .map(|l| l.owner)
            .unwrap_or(Owner { file: 0, block_ord: 0 })
    }

    pub fn mark_dirty(&mut self, owner: Owner) {
        if !self.dirty.contains(&owner) {
            self.dirty.push(owner);
        }
    }

    /// Insert a line after `idx`; it inherits that line's tag (§5.2).
    pub fn insert_line(&mut self, idx: usize, text: String) {
        let owner = if self.lines.is_empty() {
            Owner { file: 0, block_ord: 0 }
        } else {
            self.lines[idx.min(self.lines.len() - 1)].owner
        };
        self.lines.insert(idx + 1, EditLine { text, owner });
        self.mark_dirty(owner);
    }

    pub fn set_line(&mut self, idx: usize, text: String) {
        if let Some(l) = self.lines.get_mut(idx) {
            if l.text != text {
                l.text = text;
                let owner = l.owner;
                self.mark_dirty(owner);
            }
        }
    }

    pub fn delete_line(&mut self, idx: usize) {
        if idx < self.lines.len() {
            let owner = self.lines[idx].owner;
            self.lines.remove(idx);
            self.mark_dirty(owner);
        }
    }

    /// Splice one dirty block (§5.2): collect its lines, put nested blocks
    /// back as embeds, shift back, write the one file atomically.
    pub fn splice(&mut self, vault: &mut Vault, owner: Owner) -> std::io::Result<()> {
        let info = match self.owners.get(&owner) {
            Some(i) => i.clone(),
            None => return Ok(()),
        };
        // Collect the block's owned lines in buffer order; where a nested
        // block's title line sits, emit an embed instead (§5.2.1).
        let mut text_lines: Vec<String> = Vec::new();
        let mut skip_blocks: Vec<usize> = Vec::new();
        for l in &self.lines {
            if l.owner == owner {
                text_lines.push(l.text.clone());
            } else if l.owner.file == owner.file && l.owner.block_ord != owner.block_ord {
                // a line owned by a nested block in the same file: at the
                // position of that nested block's *title* line, emit an embed
                let nested_ord = l.owner.block_ord;
                if !skip_blocks.contains(&nested_ord) {
                    skip_blocks.push(nested_ord);
                    let nested_owner = Owner { file: owner.file, block_ord: nested_ord };
                    if let Some(ninfo) = self.owners.get(&nested_owner) {
                        if let Some(id) = &ninfo.id {
                            text_lines.push(format!("EMBED:{}", id));
                        }
                    }
                }
            }
        }
        // The first line is the block's title line.
        if text_lines.is_empty() {
            return Ok(());
        }
        // Shift back: re-level sections by +(level(block) − 1), re-indent by
        // +indent(block) (§5.2.3).
        let level_delta = info.level as isize - 1;
        let indent_delta = info.indent as isize;
        let mut out = String::new();
        let mut fence: Option<(char, usize)> = None;
        for l in &text_lines {
            if let Some(id) = l.strip_prefix("EMBED:") {
                out.push_str(&" ".repeat(indent_delta.max(0) as usize));
                out.push_str("![[");
                out.push_str(id);
                out.push_str("]]\n");
                continue;
            }
            let trimmed = l.trim_start();
            if fence_transition(l, &mut fence) || fence.is_some() {
                out.push_str(l);
                out.push('\n');
                continue;
            }
            let cur_indent = l.len() - trimmed.len();
            let new_indent = (cur_indent as isize + indent_delta).max(0) as usize;
            if trimmed.starts_with('#') {
                let hashes = trimmed.chars().take_while(|&c| c == '#').count();
                let nl = (hashes as isize + level_delta).max(1) as usize;
                out.push_str(&" ".repeat(new_indent));
                out.push_str(&"#".repeat(nl));
                out.push_str(&trimmed[hashes..]);
                out.push('\n');
            } else {
                out.push_str(&" ".repeat(new_indent));
                out.push_str(trimmed);
                out.push('\n');
            }
        }
        // External-change check (§5.2.5).
        let file = owner.file;
        let current_hash = blake3::hash(vault.tree.files[file].text.as_bytes())
            .to_hex()
            .to_string();
        if current_hash != self.base_hashes[file]
            && current_hash != vault.hash_of(file)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{} changed on disk; not overwriting", vault.tree.files[file].path),
            ));
        }
        // Replace the block's span (or the whole file for its own block).
        let nref = info.nref;
        let n = vault.tree.node(nref);
        if n.is_block() || n.kind == Kind::Root {
            // the block IS the file (minus frontmatter): replace everything
            // after the frontmatter
            let f = &vault.tree.files[file];
            let fm_end = f.nodes[f.root_node]
                .children
                .first()
                .and_then(|&rn| f.nodes[rn].block.as_ref())
                .and_then(|b| b.frontmatter_span)
                .map(|s| s.end)
                .unwrap_or(0);
            let mut new_text = String::new();
            new_text.push_str(&f.text[..fm_end]);
            new_text.push_str(&out);
            vault.write_file_text(file, &new_text)?;
        } else {
            vault.write_span(file, n.span, &out)?;
        }
        self.base_hashes[file] = blake3::hash(vault.tree.files[file].text.as_bytes())
            .to_hex()
            .to_string();
        self.dirty.retain(|o| *o != owner);
        Ok(())
    }

    /// Splice every dirty block (§10.6: a commit may write several files,
    /// one splice per dirty block).
    pub fn save_all(&mut self, vault: &mut Vault) -> std::io::Result<usize> {
        let dirty = self.dirty.clone();
        let mut n = 0;
        for owner in dirty {
            self.splice(vault, owner)?;
            n += 1;
        }
        Ok(n)
    }
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

/// Recursive buffer construction: emit a node's title/body at the display
/// position, tag lines with the owning block, descend (§5.2).
#[allow(clippy::too_many_arguments)]
fn build_node(
    vault: &Vault,
    r: NRef,
    dlevel: usize,
    dindent: usize,
    block_ord: &mut usize,
    lines: &mut Vec<EditLine>,
    owners: &mut BTreeMap<Owner, OwnerInfo>,
    seen: &mut Vec<NRef>,
) {
    if seen.contains(&r) {
        return;
    }
    seen.push(r);
    let n = vault.tree.node(r);
    // The owning block: nearest ancestor-or-self that is one. The render
    // root owns until a nested block starts.
    let is_block_start = n.is_block() && n.kind != Kind::Root;
    let owner = if is_block_start {
        *block_ord += 1;
        let o = Owner { file: r.0, block_ord: *block_ord };
        owners.insert(
            o,
            OwnerInfo {
                title: n.title.clone(),
                id: n.block.as_ref().and_then(|b| b.id.clone()),
                nref: r,
                level: dlevel,
                indent: dindent,
            },
        );
        o
    } else if n.kind == Kind::Root || owners.is_empty() {
        let o = Owner { file: r.0, block_ord: 0 };
        owners.entry(o).or_insert_with(|| OwnerInfo {
            title: if n.kind == Kind::Root {
                "root".into()
            } else {
                n.title.clone()
            },
            id: n.block.as_ref().and_then(|b| b.id.clone()),
            nref: r,
            level: dlevel,
            indent: dindent,
        });
        o
    } else {
        // owned by the enclosing block: the last opened owner
        *owners.keys().last().unwrap()
    };
    // Title line.
    match n.kind {
        Kind::Root => {}
        Kind::Section => {
            let mut s = " ".repeat(dindent);
            s.push_str(&"#".repeat(dlevel.max(1)));
            s.push(' ');
            if !n.is_block() {
                push_checkbox(n.task, &mut s);
            }
            s.push_str(n.title.trim_end());
            lines.push(EditLine { text: s, owner });
        }
        Kind::Item => {
            let mut s = " ".repeat(dindent);
            s.push_str("- ");
            if !n.is_block() {
                push_checkbox(n.task, &mut s);
            }
            s.push_str(n.title.trim_end());
            lines.push(EditLine { text: s, owner });
        }
    }
    // Body.
    let body = n.body_lines(vault.tree.text_of(r));
    let mut body: Vec<&str> = body;
    while body.first().map(|l| l.trim().is_empty()) == Some(true) {
        body.remove(0);
    }
    while body.last().map(|l| l.trim().is_empty()) == Some(true) {
        body.pop();
    }
    if !body.is_empty() && n.kind != Kind::Root {
        lines.push(EditLine { text: String::new(), owner });
    }
    let dedent_by = vault.tree.indent(r);
    for l in &body {
        lines.push(EditLine {
            text: format!("{}{}", " ".repeat(dindent), dedent(l, dedent_by)),
            owner,
        });
    }
    // Children.
    let children = vault.tree.resolved_children(r);
    let mut first = true;
    for c in children {
        let cn = vault.tree.node(c);
        if cn.kind == Kind::Root {
            continue;
        }
        if first {
            if !body.is_empty() || n.kind != Kind::Root {
                lines.push(EditLine { text: String::new(), owner });
            }
            first = false;
        } else if cn.kind == Kind::Section {
            lines.push(EditLine { text: String::new(), owner });
        }
        let cindent = dindent + vault.tree.indent(c).saturating_sub(vault.tree.indent(r));
        let clevel = match cn.kind {
            Kind::Section => dlevel + 1,
            _ => dlevel,
        };
        // embeds resolve in the editing buffer (§10.6): the nested block's
        // lines carry its own tag.
        let target = vault.tree.resolved_child(c);
        if target != c {
            build_node(vault, target, clevel, cindent, block_ord, lines, owners, seen);
        } else if cn.is_embed() {
            // broken embed: shown as text, owned here
            lines.push(EditLine {
                text: format!("{}![[{}]]", " ".repeat(cindent), cn.embed.as_ref().unwrap()),
                owner,
            });
        } else {
            build_node(vault, c, clevel, cindent, block_ord, lines, owners, seen);
        }
    }
    seen.pop();
}

fn push_checkbox(task: Option<TaskState>, out: &mut String) {
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

/// What the editor needs to know when it opens: the buffer plus a render of
/// the same text for display (the lines ARE the display).
pub fn open_editor(vault: &Vault, root: NRef) -> EditBuffer {
    EditBuffer::build(vault, root)
}

/// Renames detected by the splice: if a block's title line changed, its file
/// is renamed (§5.2 edge case). Returns the new title if the first owned
/// line's title differs from the stored one.
pub fn title_of_first_line(line: &str) -> Option<String> {
    let t = line.trim_start();
    if t.starts_with('#') {
        let h = t.chars().take_while(|&c| c == '#').count();
        Some(t[h..].trim().to_string())
    } else if let Some(rest) = t.strip_prefix("- ") {
        Some(rest.trim().to_string())
    } else {
        None
    }
}
