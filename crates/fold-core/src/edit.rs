//! The built-in editor's document model (§5.2, §10.6): a flat list of lines,
//! each tagged with the block that owns it. Edits apply to whichever block
//! the cursor is in; dirty blocks are spliced back one file each.
//!
//! Ownership is carried by the tags, never re-derived from text. Splice of
//! one block collects its owned lines in buffer order, puts nested blocks
//! back as embeds where their title lines sit, re-levels/re-indents, and
//! replaces the block's span atomically.

use crate::ident::Id;
use crate::parse::{parse_file, parse_frontmatter, Block, Content, Frontmatter, Kind, ParsedFile, Span};
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
    /// Nested blocks deleted by deleting their title line (§5.2): their
    /// lines are the enclosing block's now, and each file goes to trash
    /// once no file embeds it.
    dropped: Vec<Owner>,
    /// Nested blocks whose title line is in the editor's clipboard, cut
    /// and not put back: in transit, not deleted (`hold`).
    held: Vec<Owner>,
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
    /// Whether that node is a section (its title line a heading), not an
    /// item.
    pub section: bool,
    /// Level/indent of that node in its own file; splice shifts by
    /// `target − display` (§5.2.3).
    pub target_level: usize,
    pub target_indent: usize,
    /// The owner whose text holds this block's embed (None for the owner
    /// of the render root).
    pub parent: Option<Owner>,
    /// The file the block is written to.
    pub path: String,
    /// For a render root that is not a block: the bytes `start..end` of its
    /// file that the buffer owns — its span without the blank lines after
    /// it — and each splice moves `end` to the end of what it wrote. Once
    /// its own text changes (a line above its title, the title deleted) no
    /// node need start where it did; the base-hash check (§5.2.5) ensures
    /// nothing else wrote the file since.
    pub start: usize,
    pub end: usize,
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
                    section: n.kind == Kind::Section,
                    target_level,
                    target_indent,
                    parent,
                    path: tree.files[nref.0].path.clone(),
                    start: n.span.start,
                    end: owned_end(&tree.files[nref.0].text, n.span),
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
            dropped: Vec::new(),
            held: Vec::new(),
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

    /// The nested blocks whose title line the editor's clipboard holds
    /// (§5.2: cutting a block's title line and pasting it moves the block).
    /// While held, a block with nothing in the buffer (no line of its own or
    /// of a block nested in it) is in transit: the enclosing block is
    /// written without its embed, and the block and its file are left alone
    /// until the line is pasted back. A block released with no line in the
    /// buffer is dirty again, so the next save deletes it, as deleting its
    /// title line does.
    pub fn hold(&mut self, owners: Vec<Owner>) {
        for o in std::mem::replace(&mut self.held, owners) {
            if !self.held.contains(&o) && !self.lines.iter().any(|l| l.owner == o) {
                self.mark_dirty(o);
            }
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

    /// Where the block is now: its file, and for a block or a file's Root
    /// its node in the current tree (it may have been re-parsed since the
    /// buffer was built). A render root that is not a block is no node but
    /// the region `start..end` of its file.
    fn locate(&self, vault: &Vault, info: &OwnerInfo) -> Option<(usize, Option<usize>)> {
        if let Some(id) = &info.id {
            return vault.tree.block_by_id(id).map(|(f, n)| (f, Some(n)));
        }
        let file = vault.file_index(&info.path)?;
        Some((file, info.is_root.then_some(vault.tree.files[file].root_node)))
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

    /// The line where nested block `o`'s text starts (§5.2): its first line
    /// if that is a title line of its kind at its display indent — a heading
    /// for a section, a bullet for an item — else the first such line below
    /// it, a heading no deeper than the block's level (deeper ones are its
    /// children). None: its title line was deleted.
    fn title_line(&self, o: Owner) -> Option<usize> {
        let info = self.owners.get(&o)?;
        let mut fence = None;
        let mut first = true;
        for (i, l) in self.lines.iter().enumerate().filter(|(_, l)| l.owner == o) {
            let in_code = fence_transition(&l.text, &mut fence) || fence.is_some();
            let trimmed = l.text.trim_start_matches(' ');
            if !in_code && l.text.len() - trimmed.len() == info.indent {
                let title = if info.section {
                    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
                    let after = &trimmed[hashes..];
                    hashes > 0
                        && (after.is_empty() || after.starts_with(' '))
                        && (first || hashes <= info.level.max(1))
                } else {
                    ["- ", "* ", "+ "].iter().any(|m| trimmed.starts_with(m))
                        || ["-", "*", "+"].contains(&trimmed)
                };
                if title {
                    return Some(i);
                }
            }
            first = false;
        }
        None
    }

    /// §5.2's edge cases, applied from the tags before a splice. A nested
    /// block's text starts at its title line: lines it owns above that line
    /// sit in the enclosing block and become its text. Deleting the title
    /// line deletes the block: every line it still owned is re-tagged to
    /// the enclosing block, the blocks nested in it are nested in that
    /// block, and its file goes to trash once that block is written without
    /// its embed — unless it is in transit, its title line cut (`hold`).
    fn settle(&mut self) {
        // lines of a deleted block put back (by the editor's own undo) are
        // the enclosing block's text like the rest of them
        let dropped = self.dropped.clone();
        self.dirty.retain(|o| !dropped.contains(o));
        for i in 0..self.lines.len() {
            let s = self.surviving(self.lines[i].owner);
            if s != self.lines[i].owner {
                self.lines[i].owner = s;
                self.mark_dirty(s);
            }
        }
        loop {
            let mut changed = false;
            for o in self.dirty.clone() {
                let Some(parent) = self.owners.get(&o).and_then(|i| i.parent) else {
                    continue;
                };
                let title = self.title_line(o);
                // a title line emptied to be typed again (vim `cc`) is not
                // a deleted one; splice refuses to write the block meanwhile
                let first = self.lines.iter().find(|l| l.owner == o);
                if title.is_none() && first.is_some_and(|l| l.text.trim().is_empty()) {
                    continue;
                }
                // one whose title line the clipboard holds, with nothing of
                // it or of the blocks in it left here, is in transit (`hold`)
                let transit = first.is_none()
                    && self.held.contains(&o)
                    && !self.lines.iter().any(|l| self.nested_in(l.owner, o).is_some());
                let mut moved = title.is_none();
                let above = title.unwrap_or(self.lines.len());
                for l in self.lines[..above].iter_mut().filter(|l| l.owner == o) {
                    l.owner = parent;
                    moved = true;
                }
                if title.is_none() {
                    self.dirty.retain(|d| *d != o);
                }
                if title.is_none() && !transit {
                    for i in self.owners.values_mut() {
                        if i.parent == Some(o) {
                            i.parent = Some(parent);
                        }
                    }
                    self.dropped.push(o);
                }
                if moved {
                    self.mark_dirty(parent);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// The block that holds a deleted block's lines now.
    fn surviving(&self, mut o: Owner) -> Owner {
        while self.dropped.contains(&o) {
            match self.owners.get(&o).and_then(|i| i.parent) {
                Some(p) => o = p,
                None => break,
            }
        }
        o
    }

    /// Move deleted blocks' files to the trash (§11.5) once no file embeds
    /// them any more: once the block that held the embed has been written.
    fn trash_dropped(&self, vault: &mut Vault) -> std::io::Result<()> {
        loop {
            let file = self.dropped.iter().find_map(|o| {
                let id = self.owners.get(o)?.id.as_ref()?;
                match vault.tree.embed_of(id) {
                    Some(_) => None,
                    None => vault.tree.block_by_id(id).map(|r| r.0),
                }
            });
            match file {
                Some(f) => vault.trash_file(f)?,
                None => return Ok(()),
            }
        }
    }

    /// Splice one dirty block (§5.2) — or, if deleting its title line
    /// deleted it, the block its lines went to — then trash the files of
    /// deleted blocks no longer embedded.
    pub fn splice(&mut self, vault: &mut Vault, owner: Owner) -> std::io::Result<()> {
        self.settle();
        let owner = self.surviving(owner);
        self.write(vault, owner)?;
        self.trash_dropped(vault)
    }

    /// Write one block (§5.2): collect its lines, put nested blocks back as
    /// embeds, shift back, write the one file atomically. False: there was
    /// nothing to write, and the block is no longer dirty either.
    fn write(&mut self, vault: &mut Vault, owner: Owner) -> std::io::Result<bool> {
        let info = match self.owners.get(&owner) {
            Some(i) => i.clone(),
            None => {
                self.dirty.retain(|o| *o != owner);
                return Ok(false);
            }
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
            // every line of the edited node gone: the editor does not delete
            // it (a nested block with no lines left was deleted by settle)
            self.dirty.retain(|o| *o != owner);
            return Ok(false);
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
        self.rebase(vault, owner)?;
        let info = self.owners.get(&owner).cloned().unwrap_or(info);
        let (file, node) = self.locate(vault, &info).ok_or_else(|| not_found(&info))?;
        let path = vault.tree.files[file].path.clone();
        let region = Span {
            start: info.start,
            end: info.end,
        };
        let f = &vault.tree.files[file];
        // §4.9: a block file with text or an embed before its root is
        // read-only until fixed — the bytes before the root are not in the
        // buffer, and rewriting the file would drop them
        if malformed_block_file(f) {
            return Err(std::io::Error::other(format!(
                "{}: text or an embed before the block's root; read-only until fixed",
                path
            )));
        }
        if let Some(ni) = node {
            // §5.2 step 2: a block's text parses to exactly one root-level
            // node, so splice never writes a block file it would then treat
            // as read-only, or one with no root at all
            if let Some(b) = f.nodes[ni].block.as_ref().filter(|b| b.id.is_some()) {
                let b = Block {
                    frontmatter_span: None,
                    ..b.clone()
                };
                if malformed_block_file(&parse_file(&path, &out, file, Some(b))) {
                    return Err(std::io::Error::other(format!(
                        "{}: the block's text must start with its title line; not saved",
                        path
                    )));
                }
            }
            // the block IS the file: keep its frontmatter, replace the rest
            let fm_end = f.nodes[ni]
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
            // a region inside a file: the blank lines that separate it from
            // what follows stay
            vault.write_span(file, region, &out)?;
            if let Some(i) = self.owners.get_mut(&owner) {
                i.start = region.start;
                i.end = region.start + out.len();
            }
        }
        self.base_hashes
            .insert(path, hash(&vault.tree.files[file].text));
        self.dirty.retain(|o| *o != owner);
        Ok(true)
    }

    /// External-change check (§5.2.5) for one block: the block's source
    /// span on disk must be what was read (or last written) through this
    /// buffer. If the file changed underneath and, unless it was reloaded
    /// since, the vault still holds it as this buffer last read or wrote it,
    /// but the block's own text is unchanged, the change was elsewhere
    /// (another section, a property): the file as it is now is taken as
    /// what the buffer has read, the change kept. Otherwise the save is
    /// refused.
    fn rebase(&mut self, vault: &mut Vault, owner: Owner) -> std::io::Result<()> {
        let Some(info) = self.owners.get(&owner).cloned() else {
            return Ok(());
        };
        let (file, node) = self.locate(vault, &info).ok_or_else(|| not_found(&info))?;
        let path = vault.tree.files[file].path.clone();
        let on_disk = std::fs::read_to_string(vault.dir.join(&path)).unwrap_or_default();
        let Some(base) = self.base_hashes.get(&path).filter(|h| **h != hash(&on_disk)) else {
            return Ok(());
        };
        let region = Span {
            start: info.start,
            end: info.end,
        };
        let known = &vault.tree.files[file].text;
        let span = (*base == hash(known))
            .then(|| span_now(known, &on_disk, node.is_none().then_some(region)))
            .flatten();
        let Some(span) = span else {
            return Err(std::io::Error::other(format!(
                "{} changed on disk; not overwriting",
                path
            )));
        };
        // the vault writes over the file as it is now (its write guard
        // compares with it)
        vault.reparse(file, &on_disk)?;
        self.base_hashes.insert(path, hash(&on_disk));
        if node.is_none() {
            if let Some(i) = self.owners.get_mut(&owner) {
                (i.start, i.end) = (span.start, span.end);
            }
        }
        Ok(())
    }

    /// Take in what another program changed outside the dirty blocks'
    /// source spans (§5.2.5) before they are saved, so that the vault, and
    /// a snapshot taken of it for the op-log (§10.10), holds that change:
    /// undoing the save then does not revert it. A block whose own span
    /// changed is left for its save to refuse.
    pub fn rebase_dirty(&mut self, vault: &mut Vault) {
        self.settle();
        for owner in self.dirty.clone() {
            let _ = self.rebase(vault, owner);
        }
    }

    /// Splice every dirty block (§10.6: a commit may write several files,
    /// one splice per dirty block). Each block is written by its own splice
    /// (§5.2): one that is refused stays dirty, the others are still
    /// written, and the error names every refusal.
    pub fn save_all(&mut self, vault: &mut Vault) -> std::io::Result<usize> {
        self.settle();
        let dirty = self.dirty.clone();
        let mut n = 0;
        let mut errs = Vec::new();
        for owner in dirty {
            match self.write(vault, owner) {
                Ok(true) => n += 1,
                Ok(false) => {}
                Err(e) => errs.push(e),
            }
        }
        if let Err(e) = self.trash_dropped(vault) {
            errs.push(e);
        }
        match errs.len() {
            0 => Ok(n),
            1 => Err(errs.remove(0)),
            _ => Err(std::io::Error::other(
                errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "),
            )),
        }
    }
}

/// A line collected for splice: owned text, or a nested block's embed at
/// the display indent of its first line, with the heading level of that line
/// if it is a heading.
enum Line {
    Text(String),
    Embed(usize, Option<usize>, Id),
}

fn not_found(info: &OwnerInfo) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("{}: edited node no longer found", info.path),
    )
}

fn hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

/// Where a block's source span (§5.2 step 5) is in `now`, a newer text of
/// its file than `known`, when the block's own text there is as it was:
/// `None` if the change touched it, or if the frontmatter names another
/// block now. A block (or root.md's Root) is its file after the
/// frontmatter (§4.9); another render root is `region` of `known`, found
/// where its bytes occur in `now`, at a line start and only once.
fn span_now(known: &str, now: &str, region: Option<Span>) -> Option<Span> {
    let (k, n) = (parse_frontmatter(known), parse_frontmatter(now));
    let id = |f: &Option<Frontmatter>| f.as_ref().and_then(|f| f.props.get("id").cloned());
    if id(&k) != id(&n) {
        return None;
    }
    let Some(region) = region else {
        let body = |f: &Option<Frontmatter>| f.as_ref().map_or(0, |f| f.span.end);
        let (kb, nb) = (body(&k), body(&n));
        return (known[kb..] == now[nb..]).then_some(Span { start: nb, end: now.len() });
    };
    let own = &known[region.start.min(known.len())..region.end.min(known.len())];
    let mut found = now
        .match_indices(own)
        .map(|(i, _)| i)
        .filter(|&i| i == 0 || now.as_bytes()[i - 1] == b'\n');
    let at = found.next()?;
    found.next().is_none().then_some(Span { start: at, end: at + own.len() })
}

/// A block file with text or an embed before its root node, or with no
/// root at all (§4.9). `root.md` never is: text before its first node is
/// its Root's own.
fn malformed_block_file(f: &ParsedFile) -> bool {
    let root = &f.nodes[f.root_node];
    if root.block.is_some() {
        return false;
    }
    let text_before = root.content.iter().any(|c| match c {
        Content::Text(sp) => !sp.text(&f.text).trim().is_empty(),
        Content::Node(_) => false,
    });
    text_before || root.children.len() != 1 || f.nodes[root.children[0]].is_embed()
}

/// Where a node's own text ends: its span without the blank lines that
/// separate it from what follows.
fn owned_end(text: &str, span: Span) -> usize {
    let old = &text[span.start.min(text.len())..span.end.min(text.len())];
    let content_end = old.trim_end_matches([' ', '\t', '\r', '\n']).len();
    let own = old[content_end..]
        .find('\n')
        .map(|i| content_end + i + 1)
        .unwrap_or(old.len());
    span.start + own
}

fn fence_transition(raw: &str, open: &mut Option<(char, usize)>) -> bool {
    // fences as the parser reads them, so splice re-levels what it does
    crate::parse::fence_transition(raw, open)
}

/// What the editor needs to know when it opens: the buffer plus a render of
/// the same text for display (the lines ARE the display).
pub fn open_editor(vault: &Vault, root: NRef) -> EditBuffer {
    EditBuffer::build(vault, root)
}

