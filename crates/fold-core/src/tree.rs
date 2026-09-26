//! The stitched tree: derived levels/indents, ancestry, block ownership (§3).

use crate::ident::Id;
use crate::parse::{Kind, Node, ParsedFile, TaskState};

/// The logical tree formed by all parsed files stitched together.
pub struct Tree {
    pub files: Vec<ParsedFile>,
    /// (file, node) of the implicit Root of root.md.
    pub root: (usize, usize),
    /// (file, node) → id for every block.
    pub blocks: Vec<((usize, usize), Id)>,
}

/// A global node reference.
pub type NRef = (usize, usize);

impl Tree {
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

    /// The block that owns a node: nearest ancestor-or-self that is one (§5.2).
    pub fn owning_block(&self, r: NRef) -> NRef {
        let mut cur = Some(r);
        while let Some(c) = cur {
            if self.node(c).is_block() {
                return c;
            }
            cur = self.node(c).parent.map(|p| (c.0, p));
        }
        self.root
    }

    /// Resolve an embed node to the block it references (one level).
    pub fn resolved_child(&self, r: NRef) -> NRef {
        let n = self.node(r);
        if let Some(id) = &n.embed {
            if let Some(target) = self.block_by_id(id) {
                return target;
            }
        }
        r
    }

    /// The embed node that references a block (§4.7), if any.
    pub fn embed_of(&self, id: &Id) -> Option<NRef> {
        self.files.iter().enumerate().find_map(|(fi, f)| {
            f.nodes
                .iter()
                .position(|nd| nd.embed.as_ref() == Some(id))
                .map(|ni| (fi, ni))
        })
    }

    pub fn block_by_id(&self, id: &Id) -> Option<NRef> {
        self.blocks
            .iter()
            .find(|(_, bid)| bid == id)
            .map(|(t, _)| *t)
    }

    /// Direct children of a node with embeds resolved (broken embeds kept as-is).
    pub fn resolved_children(&self, r: NRef) -> Vec<NRef> {
        self.node(r)
            .children
            .iter()
            .map(|&c| {
                let cr = (r.0, c);
                let rc = self.resolved_child(cr);
                if rc != cr {
                    rc
                } else {
                    cr
                }
            })
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
