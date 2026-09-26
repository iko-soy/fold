//! The built-in editor's document model (§5.2, §10.6): a flat list of lines,
//! each tagged with the block that owns it. Edits apply to whichever block
//! the cursor is in; dirty blocks are spliced back one file each.
//!
//! Ownership is carried by the tags, never re-derived from text. Splice of
//! one block collects its owned lines in buffer order, puts nested blocks
//! back as embeds where their title lines sit, re-levels/re-indents, and
//! replaces the block's span atomically.

use crate::ident::Id;
use crate::parse::{parse_file, parse_frontmatter, Block, Content, Frontmatter, Kind, Node, ParsedFile, Span};
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
    /// Where each nested block's title line sat when its embed was last
    /// written (`title_form`), for those written since the buffer was
    /// built: one re-indented or re-spelled since moves the embed (§5.2).
    placed: BTreeMap<Owner, (usize, Option<usize>)>,
}

#[derive(Debug, Clone)]
pub struct OwnerInfo {
    pub title: String,
    pub id: Option<Id>,
    /// The block-root node in the tree at buffer-build time.
    pub nref: NRef,
    /// Display level/indent of the block's top node in the buffer, as
    /// built: a nested block's title line may be re-indented or re-spelled
    /// since, and splice shifts from where it sits then.
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
            placed: BTreeMap::new(),
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

    /// Each nested block's embed goes to the block its title line (at
    /// `titles`) now sits in (§5.2), out of the block it was in or into one
    /// nested there, however the line got there: moved, re-indented or
    /// re-spelled. Of the block holding the line above the title line (the
    /// blank lines between nest nothing) and the blocks that one is nested
    /// in, the innermost whose title line holds it (`holds`). Blocks go in
    /// buffer order, so where a block sits is settled before the blocks
    /// after it.
    pub fn reparent(&mut self, titles: &BTreeMap<Owner, usize>) {
        let mut order: Vec<(usize, Owner)> = titles.iter().map(|(&o, &t)| (t, o)).collect();
        order.sort();
        for (t, o) in order {
            let Some(p) = self.owners.get(&o).and_then(|i| i.parent) else { continue };
            let lines = &self.lines[..t.min(self.lines.len())];
            let Some(above) = lines.iter().rev().find(|l| !l.text.trim().is_empty()) else { continue };
            let mut q = Some(above.owner);
            while let Some(c) = q {
                if c != o && self.nested_in(c, o).is_none() && self.holds(titles, c, o) {
                    break;
                }
                q = self.owners.get(&c).and_then(|i| i.parent);
            }
            let Some(q) = q.filter(|&q| q != p) else { continue };
            if let Some(i) = self.owners.get_mut(&o) {
                i.parent = Some(q);
            }
            self.mark_dirty(p);
            self.mark_dirty(q);
        }
    }

    /// Whether block `q`'s title line holds block `o`'s, written below its
    /// lines, as the parser nests title lines (§3.1): the edited node's
    /// block holds every line, an item what is indented past it, a section
    /// what is indented past it (under an item of its own) and, at its
    /// indent, items and deeper sections. Each title line as it is written
    /// now (`title_form`), not as the buffer was built: one re-indented or
    /// re-spelled nests where it sits (else where it was last written).
    fn holds(&self, titles: &BTreeMap<Owner, usize>, q: Owner, o: Owner) -> bool {
        if self.owners.get(&q).is_some_and(|i| i.parent.is_none()) {
            return true;
        }
        let form = |o: Owner| {
            titles
                .get(&o)
                .and_then(|&t| self.lines.get(t))
                .and_then(|l| title_form(&l.text))
                .or_else(|| self.placed(o))
        };
        let (Some((qi, qh)), Some((oi, oh))) = (form(q), form(o)) else { return false };
        oi > qi || (oi == qi && qh.is_some_and(|q| oh.is_none_or(|o| o > q)))
    }

    /// The line where nested block `o`'s text starts (§5.2): the first title
    /// line of either kind at any indent named as the block is in its file
    /// (`file_title`), whatever lines of the block come before it, typed or
    /// split off in front of it; else its first line if that is a title line
    /// of its kind at its display indent — a heading for a section, a
    /// bullet for an item — or, re-indented or re-spelled, a title line of
    /// either kind at any indent that is not one of the block's children
    /// (`left_first`); else the first title line of its kind at its display
    /// indent below it, a heading no deeper than the block's level (deeper
    /// ones are its children). None: its title line was deleted.
    fn title_line(&self, vault: &Vault, o: Owner) -> Option<usize> {
        let info = self.owners.get(&o)?;
        let named = self.file_title(vault, o);
        let mut fence = None;
        let mut first = true;
        let mut found = None;
        for (i, l) in self.lines.iter().enumerate().filter(|(_, l)| l.owner == o) {
            let in_code = fence_transition(&l.text, &mut fence) || fence.is_some();
            if let Some((indent, heading)) = title_form(&l.text).filter(|_| !in_code) {
                if named == Some(after_marker(&l.text)) {
                    return Some(i);
                }
                let own = indent == info.indent && heading.is_some() == info.section;
                let title = (first && (own || !self.left_first(vault, o, &l.text)))
                    || (own && heading.is_none_or(|h| h <= info.level.max(1)));
                if title && found.is_none() {
                    found = Some(i);
                }
            }
            first = false;
        }
        found
    }

    /// Nested block `o`'s title in its file as last read or written, after
    /// its marker (`after_marker`).
    fn file_title<'v>(&self, vault: &'v Vault, o: Owner) -> Option<&'v str> {
        let (file, root) = self.owners.get(&o).and_then(|i| self.locate(vault, i))?;
        let f = &vault.tree.files[file];
        let n = f.nodes.get(root?)?;
        Some(after_marker(&f.text[n.title_span.start..n.title_span.end.min(f.text.len())]))
    }

    /// Whether `line`, first of block `o`'s lines, is the title line of one
    /// of its children, not its own: a node below its title in its file as
    /// last read or written, not named as the block is. So deleting a
    /// block's title line, which may leave a child's first, deletes it
    /// (§5.2), while its title line re-indented stays its title.
    fn left_first(&self, vault: &Vault, o: Owner, line: &str) -> bool {
        let Some((file, Some(root))) = self.owners.get(&o).and_then(|i| self.locate(vault, i)) else {
            return false;
        };
        let f = &vault.tree.files[file];
        let title = |n: &Node| after_marker(&f.text[n.title_span.start..n.title_span.end.min(f.text.len())]);
        let t = after_marker(line);
        t != title(&f.nodes[root])
            && f.nodes.iter().enumerate().any(|(i, n)| {
                i != root && n.kind != Kind::Root && !n.is_embed() && title(n) == t
            })
    }

    /// Where nested block `o`'s embed was last written: its title line's
    /// indent and heading level (`title_form`), as built or since saved.
    fn placed(&self, o: Owner) -> Option<(usize, Option<usize>)> {
        self.placed.get(&o).copied().or_else(|| {
            let i = self.owners.get(&o)?;
            Some((i.indent, i.section.then_some(i.level)))
        })
    }

    /// §5.2's edge cases, applied from the tags before a splice. A nested
    /// block's text starts at its title line: lines it owns above that line
    /// sit in the enclosing block and become its text. Deleting the title
    /// line deletes the block: every line it still owned is re-tagged to
    /// the enclosing block, the blocks nested in it are nested in that
    /// block, and its file goes to trash once that block is written without
    /// its embed — unless it is in transit, its title line cut (`hold`).
    /// Re-indenting or re-spelling the title line moves the block: the
    /// enclosing block is dirty, its embed written where the line now sits,
    /// in the block that line sits in (`reparent`).
    fn settle(&mut self, vault: &Vault) {
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
        // and so are the blocks nested in it, whatever parents the editor's
        // undo put back: no embed is written into a deleted block
        let parents: Vec<(Owner, Owner)> = self
            .owners
            .iter()
            .filter_map(|(&o, i)| Some((o, i.parent?)))
            .filter(|&(o, p)| !dropped.contains(&o) && self.surviving(p) != p)
            .collect();
        for (o, p) in parents {
            let s = self.surviving(p);
            if let Some(i) = self.owners.get_mut(&o) {
                i.parent = Some(s);
            }
            self.mark_dirty(s);
        }
        loop {
            let mut changed = false;
            for o in self.dirty.clone() {
                let Some(parent) = self.owners.get(&o).and_then(|i| i.parent) else {
                    continue;
                };
                let title = self.title_line(vault, o);
                let form = title.and_then(|t| title_form(&self.lines[t].text));
                if form.is_some() && form != self.placed(o) && !self.dirty.contains(&parent) {
                    self.mark_dirty(parent);
                    changed = true;
                }
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
        // and each goes to the block its title line now sits in, so no
        // embed is written nested under another's line (§4.7)
        let titles: BTreeMap<Owner, usize> = self
            .owners
            .iter()
            .filter(|(_, i)| i.parent.is_some())
            .filter_map(|(&o, _)| Some((o, self.title_line(vault, o)?)))
            .collect();
        self.reparent(&titles);
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
    /// That is the block its lines went to, and it must be saved (not
    /// dirty, its save not refused) with its file on disk as written: a
    /// file another program changed may embed the block again.
    fn trash_dropped(&self, vault: &mut Vault) -> std::io::Result<()> {
        loop {
            let file = self.dropped.iter().find_map(|o| {
                let id = self.owners.get(o)?.id.as_ref()?;
                let s = self.surviving(*o);
                if self.dirty.contains(&s) || vault.tree.embed_of(id).is_some() {
                    return None;
                }
                let (holder, _) = self.locate(vault, self.owners.get(&s)?)?;
                vault.check_unchanged(holder).ok()?;
                vault.tree.block_by_id(id).map(|r| r.0)
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
        self.settle(vault);
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
        let mut forms = Vec::new();
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
                        let form = title_form(&l.text);
                        let hashes = form.and_then(|f| f.1);
                        let respelled = form.is_some_and(|f| {
                            Some(f.1.is_some()) != self.placed(nested).map(|p| p.1.is_some())
                        });
                        // an existing embed keeps its form, so an unchanged
                        // splice writes it back byte for byte; a title line
                        // re-spelled since it was written changes it
                        let heading = match vault.tree.embed_of(&id) {
                            _ if respelled => hashes,
                            Some(e) if vault.tree.node(e).kind == Kind::Section => {
                                Some(hashes.unwrap_or(info.level + 1))
                            }
                            Some(_) => None,
                            None => hashes,
                        };
                        text_lines.push(Line::Embed(indent, heading, id));
                        if let Some(f) = form {
                            forms.push((nested, f));
                        }
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
        // its file (§5.2.3). A nested block's display position is where its
        // title line sits now, re-indented or re-spelled or not: its root
        // goes to column 0, a heading to its file's level (§4.9)
        let title = info.parent.and_then(|_| self.title_line(vault, owner));
        let (level, indent, target_level) = match title.and_then(|t| title_form(&self.lines[t].text)) {
            Some((indent, Some(h))) => (h, indent, if info.section { info.target_level } else { 1 }),
            // an item shows at its section's level, one above a section's
            Some((indent, None)) if info.section => (info.level.saturating_sub(1).max(1), indent, 1),
            Some((indent, None)) => (info.level, indent, info.target_level),
            None => (info.level, info.indent, info.target_level),
        };
        let level_delta = target_level as isize - level as isize;
        let indent_delta = info.target_indent as isize - indent as isize;
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
        // §4.7: an embed line has no lines nested under it, where the tree
        // does not see them. A line the parser would nest there (the block's
        // own line indented under a nested block's title line, or after one
        // re-spelled as a heading) is refused, not hidden, unless the file
        // had it so already
        let crowded = crowded_embeds(f);
        if crowded_embeds(&parse_file(&path, &out, file, None)).iter().any(|id| !crowded.contains(id)) {
            return Err(std::io::Error::other(format!(
                "{}: a line would be nested under a block's embed, out of the outline; not saved",
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
        self.placed.extend(forms);
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
        self.settle(vault);
        for owner in self.dirty.clone() {
            let _ = self.rebase(vault, owner);
        }
    }

    /// Leaving the editor without saving (*Revert*, §10.6): the unsaved
    /// text is dropped, not written, but a block a save took out of the
    /// block it was in — its title line cut and the parent written in
    /// transit, whether the clipboard still holds it or has let it go since
    /// (it has no line left here), or its title line deleted and its file
    /// not trashed yet — can no longer be pasted back as itself. It is
    /// deleted as on leaving the editor any other way (§5.2): its file goes
    /// to trash once no file embeds it, never left in the vault embedded
    /// nowhere. `vault` must hold the files as they are on disk.
    pub fn discard(&self, vault: &mut Vault) -> std::io::Result<()> {
        let lineless: Vec<Owner> = self
            .owners
            .iter()
            .filter(|&(o, i)| i.parent.is_some() && !self.lines.iter().any(|l| l.owner == *o))
            .map(|(&o, _)| o)
            .collect();
        loop {
            let file = self.held.iter().chain(&self.dropped).chain(&lineless).find_map(|o| {
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

    /// Splice every dirty block (§10.6: a commit may write several files,
    /// one splice per dirty block). Each block is written by its own splice
    /// (§5.2): one that is refused stays dirty, the others are still
    /// written, and the error names every refusal.
    pub fn save_all(&mut self, vault: &mut Vault) -> std::io::Result<usize> {
        self.settle(vault);
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

/// A title line's form (§4.3): its indent, and its level if it is a
/// heading rather than a bullet. None: not a title line.
fn title_form(line: &str) -> Option<(usize, Option<usize>)> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    let after = &trimmed[hashes..];
    if hashes > 0 && (after.is_empty() || after.starts_with(' ')) {
        return Some((indent, Some(hashes)));
    }
    let bullet = ["- ", "* ", "+ "].iter().any(|m| trimmed.starts_with(m))
        || ["-", "*", "+"].contains(&trimmed);
    bullet.then_some((indent, None))
}

/// A title line's text after its indent and its heading or bullet marker,
/// whichever form and level it is written in.
fn after_marker(line: &str) -> &str {
    let t = line.trim_start_matches([' ', '\t']);
    let t = match t.strip_prefix(['-', '*', '+']) {
        Some(rest) => rest,
        None => t.trim_start_matches('#'),
    };
    t.trim()
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
/// frontmatter (§4.9); another render root is `region` of `known`, whole
/// lines, followed through a line diff of `known` against `now`: its lines
/// must all be lines the diff keeps, in one unchanged run. Never where its
/// bytes merely occur in `now`: an identical node elsewhere is another
/// node.
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
    let diff = similar::TextDiff::from_lines(known, now);
    // the byte offset where each line starts, and the text's end
    let starts = |lines: &[&str]| {
        let mut at = vec![0];
        for l in lines {
            at.push(at[at.len() - 1] + l.len());
        }
        at
    };
    let (old, new) = (starts(diff.old_slices()), starts(diff.new_slices()));
    let line = |b: usize| old.binary_search(&b.min(known.len())).ok();
    let (first, end) = (line(region.start)?, line(region.end)?);
    diff.ops().iter().find_map(|op| match *op {
        similar::DiffOp::Equal { old_index, new_index, len } if old_index <= first && end <= old_index + len => {
            Some(Span {
                start: new[new_index + first - old_index],
                end: new[new_index + end - old_index],
            })
        }
        _ => None,
    })
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

/// The blocks whose embed in `f` has lines nested under it: nodes, or text
/// other than blank lines (§4.7: a diagnostic).
fn crowded_embeds(f: &ParsedFile) -> Vec<&Id> {
    f.nodes
        .iter()
        .filter(|n| {
            n.content.iter().any(|c| match c {
                Content::Text(sp) => !sp.text(&f.text).trim().is_empty(),
                Content::Node(_) => true,
            })
        })
        .filter_map(|n| n.embed.as_ref())
        .collect()
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

