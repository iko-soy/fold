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

    /// level(n) = 1 + number of section ancestors (§3.1). Items contribute
    /// nothing; a section under items keeps its own level.
    pub fn level(&self, r: NRef) -> usize {
        let node = self.node(r);
        match node.kind {
            Kind::Section => node.level.unwrap_or(1),
            Kind::Item => {
                // An item displays at the level of its enclosing section.
                let mut cur = node.parent;
                while let Some(p) = cur {
                    let pr = (r.0, p);
                    let pn = self.node(pr);
                    if pn.kind == Kind::Section {
                        return pn.level.unwrap_or(1);
                    }
                    cur = pn.parent;
                }
                1
            }
            Kind::Root => 0,
        }
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
    /// Body lines exactly as found in the file text.
    pub fn body_lines<'a>(&self, file_text: &'a str) -> Vec<&'a str> {
        if self.body_span.start >= self.body_span.end {
            return Vec::new();
        }
        let mut lines: Vec<&'a str> = file_text[self.body_span.start..self.body_span.end]
            .split_inclusive('\n')
            .map(|l| l.strip_suffix('\n').unwrap_or(l))
            .collect();
        // A blank line produced only by the separator before a child node is
        // not body content, but it is remembered: the renderer uses it to
        // reproduce the file's spacing.
        if lines.iter().all(|l| l.trim().is_empty()) {
            lines.clear();
        }
        lines
    }
}
