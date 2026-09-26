//! The stitched tree: derived levels/indents, ancestry, block ownership (§3).

use crate::ident::Id;
use crate::parse::{Kind, Node, ParsedFile, TaskState};
use std::collections::HashMap;
use std::sync::OnceLock;

/// The logical tree formed by all parsed files stitched together.
pub struct Tree {
    pub files: Vec<ParsedFile>,
    /// (file, node) of the implicit Root of root.md.
    pub root: (usize, usize),
    /// (file, node) → id for every block.
    pub blocks: Vec<((usize, usize), Id)>,
    /// `embeds`, built at first use; `restitch` drops it.
    embed_index: OnceLock<HashMap<Id, NRef>>,
}

/// A global node reference.
pub type NRef = (usize, usize);

impl Tree {
    /// Stitch parsed files, `root.md` first, into one tree (§4.7).
    pub fn new(files: Vec<ParsedFile>) -> Tree {
        let mut t = Tree {
            files,
            root: (0, 0),
            blocks: Vec::new(),
            embed_index: OnceLock::new(),
        };
        t.restitch();
        t
    }

    /// Rebuild the block list, and what is derived from the embeds, after
    /// a file was replaced: every block is the first node of its own file
    /// (§4.9).
    pub fn restitch(&mut self) {
        let mut blocks: Vec<(NRef, Id)> = Vec::new();
        for (fi, f) in self.files.iter().enumerate() {
            if fi == 0 {
                continue;
            }
            if let Some(&first) = f.nodes[f.root_node].children.first() {
                if let Some(b) = &f.nodes[first].block {
                    if let Some(id) = &b.id {
                        blocks.push(((fi, first), id.clone()));
                    }
                }
            }
        }
        self.blocks = blocks;
        self.embed_index = OnceLock::new();
    }

    pub fn node(&self, r: NRef) -> &Node {
        &self.files[r.0].nodes[r.1]
    }
    pub fn file(&self, f: usize) -> &ParsedFile {
        &self.files[f]
    }
    /// Text of the file a node lives in.
    pub fn text_of(&self, r: NRef) -> &str {
        &self.files[r.0].text
    }

    /// level(n) = 1 + number of section ancestors (§3.1). When a section's
    /// written level is deeper than its ancestry implies (headings under
    /// bullets, skipped levels), the written level is honoured: the app never
    /// re-levels a node it didn't touch.
    pub fn level(&self, r: NRef) -> usize {
        let node = self.node(r);
        match node.kind {
            Kind::Section => {
                let written = node.level.unwrap_or(1);
                let mut derived = 1usize;
                let mut cur = node.parent;
                while let Some(p) = cur {
                    let pr = (r.0, p);
                    if self.node(pr).kind == Kind::Section {
                        derived += 1;
                    }
                    cur = self.node(pr).parent;
                }
                written.max(derived)
            }
            Kind::Item => {
                // An item displays at the level of its enclosing section.
                let mut cur = node.parent;
                while let Some(p) = cur {
                    let pr = (r.0, p);
                    if self.node(pr).kind == Kind::Section {
                        return self.level(pr);
                    }
                    cur = self.node(pr).parent;
                }
                1
            }
            Kind::Root => 0,
        }
    }

    /// The level a section has by position alone: 1 + its section ancestors
    /// (§3.1), whatever level is written.
    pub fn derived_level(&self, r: NRef) -> usize {
        let mut level = 1;
        let mut cur = self.node(r).parent;
        while let Some(p) = cur {
            if self.node((r.0, p)).kind == Kind::Section {
                level += 1;
            }
            cur = self.node((r.0, p)).parent;
        }
        level
    }

    /// indent(n) = 2 × number of item ancestors (§3.1).
    pub fn indent(&self, r: NRef) -> usize {
        let mut n = 0usize;
        let mut cur = self.node(r).parent;
        while let Some(p) = cur {
            let pr = (r.0, p);
            if self.node(pr).kind == Kind::Item {
                n += 2;
            }
            cur = self.node(pr).parent;
        }
        n
    }

    /// Ancestor chain from the root down to (but excluding) `r`.
    pub fn ancestors(&self, r: NRef) -> Vec<NRef> {
        let mut out = Vec::new();
        let mut cur = self.node(r).parent;
        while let Some(p) = cur {
            let pr = (r.0, p);
            if self.node(pr).kind != Kind::Root {
                out.push(pr);
            }
            cur = self.node(pr).parent;
        }
        out.reverse();
        out
    }

    /// Path segments (ancestor titles + own title) (§3.4).
    pub fn path(&self, r: NRef) -> Vec<String> {
        let mut segs: Vec<String> = self
            .ancestors(r)
            .iter()
            .map(|&a| self.node(a).title.clone())
            .collect();
        segs.push(self.node(r).title.clone());
        segs
    }

    /// Resolve an embed node to the block it references (one level). Only
    /// the block's own embed (`embed_of`) resolves: any other embed of its
    /// id is left as it is and reads as broken everywhere, as §6.2 says,
    /// so walk, render, the outline and every verb meet a block once.
    pub fn resolved_child(&self, r: NRef) -> NRef {
        let n = self.node(r);
        if let Some(id) = &n.embed {
            if let Some(target) = self.block_by_id(id) {
                if self.embed_of(id) == Some(r) {
                    return target;
                }
            }
        }
        r
    }

    /// The embed a block is stitched in at (§4.7), if any: see `embeds`.
    pub fn embed_of(&self, id: &Id) -> Option<NRef> {
        self.embeds().get(id).copied()
    }

    /// `embed_of` for every id at once: the embed each block is stitched in
    /// at, the first of its id met in tree order from the root, as walk
    /// and the outline meet them. Any other embed of the id is a duplicate
    /// and reads as broken (§6.2). An id embedded only in files the root
    /// does not reach (an orphan block, a cycle) keeps its first embed in
    /// file order.
    pub fn embeds(&self) -> &HashMap<Id, NRef> {
        self.embed_index.get_or_init(|| {
            let mut out = HashMap::new();
            if self.files.is_empty() {
                return out;
            }
            // pre-order, entering each block at its first embed only, so
            // the walk ends even through a cycle
            let mut stack = vec![self.root];
            while let Some(r) = stack.pop() {
                let mut at = r;
                if let Some(id) = &self.node(r).embed {
                    if !out.contains_key(id) {
                        out.insert(id.clone(), r);
                        at = self.block_by_id(id).unwrap_or(r);
                    }
                }
                stack.extend(self.node(at).children.iter().rev().map(|&c| (at.0, c)));
            }
            for (fi, f) in self.files.iter().enumerate() {
                for (ni, nd) in f.nodes.iter().enumerate() {
                    if let Some(id) = &nd.embed {
                        out.entry(id.clone()).or_insert((fi, ni));
                    }
                }
            }
            out
        })
    }

    pub fn block_by_id(&self, id: &Id) -> Option<NRef> {
        self.blocks
            .iter()
            .find(|(_, bid)| bid == id)
            .map(|(t, _)| *t)
    }

    /// Direct children of a node with embeds resolved (broken embeds, and
    /// second embeds of a block, kept as-is).
    pub fn resolved_children(&self, r: NRef) -> Vec<NRef> {
        self.node(r)
            .children
            .iter()
            .map(|&c| self.resolved_child((r.0, c)))
            .collect()
    }

    /// Direct children without resolving embeds.
    pub fn raw_children(&self, r: NRef) -> Vec<NRef> {
        self.node(r).children.iter().map(|&c| (r.0, c)).collect()
    }

    /// Derived open/total task counts for a subtree (§3.5).
    pub fn task_counts(&self, r: NRef) -> (usize, usize) {
        let mut open = 0;
        let mut total = 0;
        self.walk(r, &mut |t, n| {
            if let Some(st) = t.node(n).task {
                total += 1;
                if st == TaskState::Open {
                    open += 1;
                }
            }
        });
        (open, total)
    }

    /// Pre-order walk over the resolved tree (embeds followed, cycles guarded).
    pub fn walk(&self, r: NRef, f: &mut dyn FnMut(&Tree, NRef)) {
        let mut seen = Vec::new();
        self.walk_inner(r, f, &mut seen);
    }

    fn walk_inner(&self, r: NRef, f: &mut dyn FnMut(&Tree, NRef), seen: &mut Vec<NRef>) {
        if seen.contains(&r) {
            return; // cycle guard
        }
        seen.push(r);
        f(self, r);
        for cr in self.resolved_children(r) {
            self.walk_inner(cr, f, seen);
        }
    }
}

impl Node {
    pub fn is_block(&self) -> bool {
        self.block.is_some()
    }
    pub fn is_embed(&self) -> bool {
        self.embed.is_some()
    }
    /// Body lines (the leading text children) exactly as found in the file.
    pub fn body_lines<'a>(&self, file_text: &'a str) -> Vec<&'a str> {
        match self.body_span() {
            Some(sp) => content_lines(file_text, sp),
            None => Vec::new(),
        }
    }

    /// The lines of every text child, in order: the node's own prose,
    /// wherever it sits among its children (§3.3).
    pub fn text_lines<'a>(&self, file_text: &'a str) -> Vec<&'a str> {
        self.text_runs()
            .into_iter()
            .flat_map(|(_, sp)| content_lines(file_text, sp))
            .collect()
    }
}

/// The lines of a text run. A run of blank lines alone is a separator, not
/// content: the renderer reproduces it, but it has no lines to show.
fn content_lines(file_text: &str, sp: crate::parse::Span) -> Vec<&str> {
    let mut lines: Vec<&str> = file_text[sp.start..sp.end]
        .split_inclusive('\n')
        .map(|l| l.strip_suffix('\n').unwrap_or(l))
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if lines.iter().all(|l| l.trim().is_empty()) {
        lines.clear();
    }
    lines
}
