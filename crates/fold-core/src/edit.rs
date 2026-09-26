//! The built-in editor's document model (§5.2, §10.6): a flat list of lines,
//! each tagged with the block that owns it. Edits apply to whichever block
//! the cursor is in; dirty blocks are spliced back one file each.
//!
//! Ownership is carried by the tags, never re-derived from text. Splice of
//! one block collects its owned lines in buffer order, puts nested blocks
//! back as embeds where their title lines sit, re-levels/re-indents, and
//! replaces the block's span atomically.

use crate::ident::Id;
use crate::parse::Kind;
use crate::render::render_lines;
use crate::tree::NRef;
use crate::vault::Vault;
use std::collections::{BTreeMap, HashMap};

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
    /// blake3 of each file (by path) as last read or written through this
    /// buffer (§5.2.5).
    base_hashes: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct OwnerInfo {
    pub title: String,
    pub id: Option<Id>,
    /// The block-root node in the tree at buffer-build time.
    pub nref: NRef,
    /// Display level/indent of the block's top node in the buffer.
    pub level: usize,
    pub indent: usize,
    /// Level/indent of that node in its own file; splice shifts by
    /// `target − display` (§5.2.3).
    pub target_level: usize,
    pub target_indent: usize,
    /// The owner whose text holds this block's embed (None for the owner
    /// of the render root).
    pub parent: Option<Owner>,
    /// The file the block is written to, and where its span starts (for a
    /// render root that is not a block: found again by position after
    /// reloads).
    pub path: String,
    pub start: usize,
    pub is_root: bool,
}

impl EditBuffer {
    /// Build the editing buffer for `render(node, 1, true)` (§5.2): every
    /// line tagged with its owning block.
    pub fn build(vault: &Vault, root: NRef) -> EditBuffer {
        let tree = &vault.tree;
        let mut lines = Vec::new();
        let mut owners: BTreeMap<Owner, OwnerInfo> = BTreeMap::new();
        let mut by_node: Vec<(NRef, Owner)> = Vec::new();
        let rlines = render_lines(tree, root, 1, true);
        let new_owner = |nref: NRef,
                             outer: NRef,
                             level: usize,
                             indent: usize,
                             owners: &mut BTreeMap<Owner, OwnerInfo>,
                             by_node: &mut Vec<(NRef, Owner)>|
         -> Owner {
            if let Some((_, o)) = by_node.iter().find(|(r, _)| *r == nref) {
                return *o;
            }
            let o = Owner {
                file: nref.0,
                block_ord: by_node.len(),
            };
            let n = tree.node(nref);
            let is_root = n.kind == Kind::Root;
            let (level, indent, target_level, target_indent) = if is_root {
                (0, 0, 0, 0)
            } else if n.is_block() {
                (level, indent, tree.level(nref), 0)
            } else {
                (level, indent, tree.level(nref), tree.indent(nref))
            };
            let parent = if outer == nref {
                None
            } else {
                by_node.iter().find(|(r, _)| *r == outer).map(|(_, o)| *o)
            };
            owners.insert(
                o,
                OwnerInfo {
                    title: if is_root { "root".into() } else { n.title.clone() },
                    id: n.block.as_ref().and_then(|b| b.id.clone()),
                    nref,
                    level,
                    indent,
                    target_level,
                    target_indent,
                    parent,
                    path: tree.files[nref.0].path.clone(),
                    start: n.span.start,
                    is_root,
                },
            );
            by_node.push((nref, o));
            o
        };
        for l in rlines {
            let owner = new_owner(l.owner, l.outer, l.level, l.indent, &mut owners, &mut by_node);
            lines.push(EditLine {
                text: l.text,
                owner,
            });
        }
        if lines.is_empty() {
            // an empty document still has one line to type into
            let owner = new_owner(root, root, 1, 0, &mut owners, &mut by_node);
            lines.push(EditLine {
                text: String::new(),
                owner,
            });
        }
        let base_hashes = tree
            .files
            .iter()
            .map(|f| (f.path.clone(), hash(&f.text)))
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

    /// The block's node in the current tree (it may have been re-parsed
    /// since the buffer was built).
    fn locate(&self, vault: &Vault, info: &OwnerInfo) -> Option<NRef> {
        if let Some(id) = &info.id {
            return vault.tree.block_by_id(id);
        }
        let file = vault.file_index(&info.path)?;
        let f = &vault.tree.files[file];
        if info.is_root {
            return Some((file, f.root_node));
        }
        f.nodes
            .iter()
            .position(|n| n.kind != Kind::Root && n.span.start == info.start)
            .map(|i| (file, i))
    }

    /// The owner directly nested in `owner` that `o` belongs to, if any.
    fn nested_in(&self, mut o: Owner, owner: Owner) -> Option<Owner> {
        while let Some(p) = self.owners.get(&o).and_then(|i| i.parent) {
            if p == owner {
                return Some(o);
            }
            o = p;
        }
        None
    }

    /// Splice one dirty block (§5.2): collect its lines, put nested blocks
    /// back as embeds, shift back, write the one file atomically.
    pub fn splice(&mut self, vault: &mut Vault, owner: Owner) -> std::io::Result<()> {
        let info = match self.owners.get(&owner) {
            Some(i) => i.clone(),
            None => return Ok(()),
        };
        // Collect the block's owned lines in buffer order; where a block
        // nested in it starts — whatever file it lives in — emit its embed
        // (§5.2.1).
        let mut text_lines: Vec<Line> = Vec::new();
        let mut emitted: Vec<Owner> = Vec::new();
        for l in &self.lines {
            if l.owner == owner {
                text_lines.push(Line::Text(l.text.clone()));
            } else if let Some(nested) = self.nested_in(l.owner, owner) {
                if !emitted.contains(&nested) {
                    emitted.push(nested);
                    if let Some(id) = self.owners.get(&nested).and_then(|i| i.id.clone()) {
                        // the embed takes the form of the block's title line:
                        // a heading embed for a heading (§4.7)
                        let trimmed = l.text.trim_start_matches(' ');
                        let indent = l.text.len() - trimmed.len();
                        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
                        let is_heading = hashes > 0 && trimmed[hashes..].starts_with(' ');
                        // an existing embed keeps its form, so an unchanged
                        // splice writes it back byte for byte
                        let heading = match vault.tree.embed_of(&id) {
                            Some(e) if vault.tree.node(e).kind == Kind::Section => {
                                Some(if is_heading { hashes } else { info.level + 1 })
                            }
                            Some(_) => None,
                            None => is_heading.then_some(hashes),
                        };
                        text_lines.push(Line::Embed(indent, heading, id));
                    }
                }
            }
        }
        if text_lines.is_empty() {
            return Ok(());
        }
        // Shift back from the display position to the block's position in
        // its file (§5.2.3).
        let level_delta = info.target_level as isize - info.level as isize;
        let indent_delta = info.target_indent as isize - info.indent as isize;
        let shift = |cols: usize| (cols as isize + indent_delta).max(0) as usize;
        let mut out = String::new();
        let mut fence: Option<(char, usize)> = None;
        for l in &text_lines {
            let l = match l {
                Line::Embed(indent, heading, id) => {
                    out.push_str(&" ".repeat(shift(*indent)));
                    if let Some(h) = heading {
                        out.push_str(&"#".repeat((*h as isize + level_delta).max(1) as usize));
                        out.push(' ');
                    }
                    out.push_str("![[");
                    out.push_str(id.as_str());
                    out.push_str("]]\n");
                    continue;
                }
                Line::Text(t) => t,
            };
            if l.trim().is_empty() {
                out.push('\n');
                continue;
            }
            let trimmed = l.trim_start_matches(' ');
            let new_indent = shift(l.len() - trimmed.len());
            out.push_str(&" ".repeat(new_indent));
            let in_code = fence_transition(l, &mut fence) || fence.is_some();
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            let after = &trimmed[hashes..];
            if !in_code && hashes > 0 && (after.is_empty() || after.starts_with(' ')) {
                let nl = (hashes as isize + level_delta).max(1) as usize;
                out.push_str(&"#".repeat(nl));
                out.push_str(after);
            } else {
                out.push_str(trimmed);
            }
            out.push('\n');
        }
        while out.ends_with("\n\n") {
            out.pop();
        }
        let nref = self.locate(vault, &info).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{}: edited node no longer found", info.path),
            )
        })?;
        let file = nref.0;
        let path = vault.tree.files[file].path.clone();
        // External-change check (§5.2.5): what is on disk now must be what
        // was read (or last written) through this buffer.
        let on_disk = std::fs::read_to_string(vault.dir.join(&path)).unwrap_or_default();
        if self.base_hashes.get(&path).map(|h| *h != hash(&on_disk)).unwrap_or(false) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{} changed on disk; not overwriting", path),
            ));
        }
        let f = &vault.tree.files[file];
        let n = &f.nodes[nref.1];
        if n.is_block() || n.kind == Kind::Root {
            // the block IS the file: keep its frontmatter, replace the rest
            let fm_end = n
                .block
                .as_ref()
                .and_then(|b| b.frontmatter_span)
                .map(|s| s.end)
                .unwrap_or(0)
                .min(f.text.len());
            let mut new_text = String::new();
            new_text.push_str(&f.text[..fm_end]);
            new_text.push_str(&out);
            vault.write_file_text(file, &new_text)?;
        } else {
            // a span inside a file: keep the blank lines that separate it
            // from what follows
            let old = n.span.text(&f.text);
            let content_end = old.trim_end_matches([' ', '\t', '\r', '\n']).len();
            let rest = &old[content_end..];
            let sep = rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or("");
            let replacement = format!("{}{}", out, sep);
            vault.write_span(file, n.span, &replacement)?;
        }
        self.base_hashes
            .insert(path, hash(&vault.tree.files[file].text));
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

/// A line collected for splice: owned text, or a nested block's embed at
/// the display indent of its first line, with the heading level of that line
/// if it is a heading.
enum Line {
    Text(String),
    Embed(usize, Option<usize>, Id),
}

fn hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
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

/// What the editor needs to know when it opens: the buffer plus a render of
/// the same text for display (the lines ARE the display).
pub fn open_editor(vault: &Vault, root: NRef) -> EditBuffer {
    EditBuffer::build(vault, root)
}

