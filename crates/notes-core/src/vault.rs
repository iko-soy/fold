//! The vault on disk: loading, indexing, atomic span-preserving writes,
//! trash (§4.1, §11).

use crate::ident::{filename, slug, split_filename, Id};
use crate::parse::{parse_file, parse_frontmatter, Block, Kind, ParsedFile, Span};
use crate::tree::{NRef, Tree};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Vault {
    pub dir: PathBuf,
    pub tree: Tree,
    /// blake3 hash of each file's text as last read or written by us (§5.2.5).
    pub hashes: Vec<String>,
}

/// A file found in the vault directory that the parser ignores (§4.1).
pub struct IgnoredFile {
    pub path: String,
    pub reason: String,
}

const FRESH_ROOT: &str = "# Inbox\n\nCaptures land here under a heading for the day. Press `s` on this section to give it its own small file for phone capture.\n";

impl Vault {
    /// Open a vault directory, creating `root.md` if it is empty (§4.1.1).
    pub fn open(dir: &Path) -> std::io::Result<Vault> {
        std::fs::create_dir_all(dir)?;
        let root_path = dir.join("root.md");
        let empty_dir = !root_path.exists()
            && std::fs::read_dir(dir)?
                .filter_map(|e| e.ok())
                .all(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.starts_with('.') || e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                });
        if empty_dir {
            atomic_write(&root_path, FRESH_ROOT)?;
        }
        let mut v = Vault {
            dir: dir.to_path_buf(),
            tree: Tree {
                files: Vec::new(),
                root: (0, 0),
                blocks: Vec::new(),
            },
            hashes: Vec::new(),
        };
        v.reload()?;
        Ok(v)
    }

    /// Rebuild the index from the vault (§11.3).
    pub fn reload(&mut self) -> std::io::Result<()> {
        let mut files: Vec<ParsedFile> = Vec::new();
        let mut hashes = Vec::new();
        let root_text = read_if_exists(&self.dir.join("root.md"))?.unwrap_or_default();
        let root_block = Block {
            id: None,
            path: "root.md".into(),
            props: parse_frontmatter(&root_text)
                .map(|f| f.props)
                .unwrap_or_default(),
            frontmatter_raw: parse_frontmatter(&root_text)
                .map(|f| f.raw)
                .unwrap_or_default(),
            frontmatter_span: parse_frontmatter(&root_text).map(|f| f.span),
            edge_span: None,
        };
        hashes.push(blake3::hash(root_text.as_bytes()).to_hex().to_string());
        files.push(parse_file("root.md", &root_text, 0, Some(root_block)));

        let mut entries: Vec<String> = std::fs::read_dir(&self.dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".md") && n != "root.md")
            .collect();
        entries.sort();
        for name in entries {
            if name.contains(".sync-conflict-") {
                continue; // merge engine only (§11.4)
            }
            let text = match read_if_exists(&self.dir.join(&name))? {
                Some(t) => t,
                None => continue,
            };
            let fm = parse_frontmatter(&text);
            let id = fm
                .as_ref()
                .and_then(|f| f.props.get("id"))
                .and_then(|v| Id::parse(v));
            let id = match id {
                Some(id) => id,
                None => continue, // ignored: never parsed (§4.1)
            };
            let idx = files.len();
            hashes.push(blake3::hash(text.as_bytes()).to_hex().to_string());
            let block = Block {
                id: Some(id),
                path: name.clone(),
                props: fm.as_ref().map(|f| f.props.clone()).unwrap_or_default(),
                frontmatter_raw: fm.as_ref().map(|f| f.raw.clone()).unwrap_or_default(),
                frontmatter_span: fm.as_ref().map(|f| f.span),
                edge_span: None,
            };
            files.push(parse_file(&name, &text, idx, Some(block)));
        }

        // Stitch: collect blocks, resolve embed edges (§4.7).
        let mut blocks: Vec<(NRef, Id)> = Vec::new();
        for (fi, f) in files.iter().enumerate() {
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
        // Set each block's edge_span by finding its embed in some file.
        for f in files.iter_mut() {
            for n in f.nodes.iter_mut() {
                n.block.as_mut().map(|b| b.edge_span = None);
            }
        }
        for ((fi, ni), id) in &blocks {
            let mut found: Option<Span> = None;
            let mut count = 0;
            for f in files.iter() {
                for n in &f.nodes {
                    if n.embed.as_ref() == Some(id) {
                        count += 1;
                        if found.is_none() {
                            found = Some(n.title_span);
                        }
                    }
                }
            }
            if count > 1 {
                // duplicate embed: diagnostic (§6.2); recorded in check
            }
            if let Some(span) = found {
                files[*fi].nodes[*ni].block.as_mut().unwrap().edge_span = Some(span);
            }
        }

        self.tree = Tree {
            files,
            root: (0, 0),
            blocks,
        };
        self.hashes = hashes;
        Ok(())
    }

    /// Files ignored by the parser, for `notes check` (§4.1).
    pub fn ignored_files(&self) -> std::io::Result<Vec<IgnoredFile>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.dir)?.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.contains(".sync-conflict-") {
                continue;
            }
            let is_md = name.ends_with(".md");
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue; // assets/ etc: silently ignored
            }
            if !is_md {
                continue;
            }
            if name == "root.md" {
                continue;
            }
            let known = self.tree.files.iter().any(|f| f.path == name);
            if !known {
                let text = read_if_exists(&e.path())?.unwrap_or_default();
                let reason = match parse_frontmatter(&text).and_then(|f| f.props.get("id").cloned())
                {
                    Some(v) if Id::parse(&v).is_none() => {
                        format!("id {:?} does not validate against the syllable tables", v)
                    }
                    Some(_) => "duplicate id".into(),
                    None => "no valid id in frontmatter".into(),
                };
                out.push(IgnoredFile { path: name, reason });
            }
        }
        Ok(out)
    }

    pub fn file_index(&self, path: &str) -> Option<usize> {
        self.tree.files.iter().position(|f| f.path == path)
    }

    /// The file a node lives in, and its recorded hash.
    pub fn hash_of(&self, file: usize) -> &str {
        &self.hashes[file]
    }

    /// Replace a byte span in a file atomically (§11.1), then re-parse.
    pub fn write_span(&mut self, file: usize, span: Span, replacement: &str) -> std::io::Result<()> {
        let f = &self.tree.files[file];
        let mut new_text = String::with_capacity(f.text.len() + replacement.len());
        new_text.push_str(&f.text[..span.start]);
        new_text.push_str(replacement);
        new_text.push_str(&f.text[span.end..]);
        self.write_file_text(file, &new_text)
    }

    /// Write a whole file atomically, then re-parse it.
    pub fn write_file_text(&mut self, file: usize, new_text: &str) -> std::io::Result<()> {
        let path = self.tree.files[file].path.clone();
        atomic_write(&self.dir.join(&path), new_text)?;
        self.reparse(file, new_text)
    }

    /// Re-parse one file from given text (after our own write) and re-stitch.
    pub fn reparse(&mut self, file: usize, text: &str) -> std::io::Result<()> {
        let path = self.tree.files[file].path.clone();
        let old_block = self.tree.files[file]
            .nodes
            .iter()
            .find_map(|n| n.block.clone());
        let block = if path == "root.md" {
            Some(Block {
                id: None,
                path: path.clone(),
                props: parse_frontmatter(text).map(|f| f.props).unwrap_or_default(),
                frontmatter_raw: parse_frontmatter(text).map(|f| f.raw).unwrap_or_default(),
                frontmatter_span: parse_frontmatter(text).map(|f| f.span),
                edge_span: None,
            })
        } else {
            old_block.map(|mut b| {
                let fm = parse_frontmatter(text);
                if let Some(f) = fm.as_ref() {
                    b.props = f.props.clone();
                    b.frontmatter_raw = f.raw.clone();
                    b.frontmatter_span = Some(f.span);
                } else {
                    b.props.clear();
                    b.frontmatter_raw.clear();
                    b.frontmatter_span = None;
                }
                b
            })
        };
        self.tree.files[file] = parse_file(&path, text, file, block);
        self.hashes[file] = blake3::hash(text.as_bytes()).to_hex().to_string();
        self.restitch();
        Ok(())
    }

    /// Rebuild the block list and edge spans after a re-parse.
    fn restitch(&mut self) {
        let mut blocks: Vec<(NRef, Id)> = Vec::new();
        for (fi, f) in self.tree.files.iter().enumerate() {
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
        for f in self.tree.files.iter_mut() {
            for n in f.nodes.iter_mut() {
                if let Some(b) = n.block.as_mut() {
                    b.edge_span = None;
                }
            }
        }
        for ((fi, ni), id) in &blocks {
            let mut edge: Option<Span> = None;
            for f in self.tree.files.iter() {
                for n in &f.nodes {
                    if n.embed.as_ref() == Some(id) {
                        edge = Some(n.title_span);
                    }
                }
            }
            if let Some(span) = edge {
                self.tree.files[*fi].nodes[*ni]
                    .block
                    .as_mut()
                    .unwrap()
                    .edge_span = Some(span);
            }
        }
        self.tree.blocks = blocks;
    }

    /// Create a new block file (§6.1): picks a unique prefix, writes
    /// `render(node, 1, false)` with frontmatter, returns the file index.
    pub fn create_block_file(
        &mut self,
        id: &Id,
        title: &str,
        body: &str,
    ) -> std::io::Result<usize> {
        let prefix = self.unique_prefix(id);
        let name = slug(title);
        let fname = filename(&prefix, &name);
        let text = format!("---\nid: {}\n---\n\n{}", id, body);
        atomic_write(&self.dir.join(&fname), &text)?;
        self.reload()?;
        Ok(self.file_index(&fname).unwrap())
    }

    /// Shortest prefix of `id` not already used by another file (§6.4).
    pub fn unique_prefix(&self, id: &Id) -> String {
        let taken: Vec<String> = self
            .tree
            .files
            .iter()
            .skip(1)
            .filter_map(|f| split_filename(&f.path).map(|(p, _)| p.to_string()))
            .collect();
        id.shortest_prefix(&|cand| taken.iter().any(|t| t == cand))
    }

    /// Rename a block's file after a title change (§6.4).
    pub fn rename_block_file(&mut self, file: usize, new_title: &str) -> std::io::Result<()> {
        let old_path = self.tree.files[file].path.clone();
        let node = &self.tree.files[file];
        let root_node = node.nodes[node.root_node].children[0];
        let id = match &node.nodes[root_node].block {
            Some(b) => b.id.clone().unwrap(),
            None => return Ok(()),
        };
        let prefix = split_filename(&old_path)
            .map(|(p, _)| p.to_string())
            .unwrap_or_else(|| self.unique_prefix(&id));
        let new_name = filename(&prefix, &slug(new_title));
        if new_name == old_path {
            return Ok(());
        }
        std::fs::rename(self.dir.join(&old_path), self.dir.join(&new_name))?;
        self.reload()?;
        Ok(())
    }

    /// Move a file to the trash (§11.5) and remove it from the vault.
    pub fn trash_file(&mut self, file: usize) -> std::io::Result<()> {
        let path = self.tree.files[file].path.clone();
        let trash = trash_dir();
        std::fs::create_dir_all(&trash)?;
        let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
        let target = trash.join(format!("{}-{}", stamp, path.to_lowercase()));
        std::fs::rename(self.dir.join(&path), &target)?;
        self.reload()?;
        Ok(())
    }

    /// Write `text` to the trash under `name` (§11.5).
    pub fn trash_text(&self, name: &str, text: &str) -> std::io::Result<PathBuf> {
        let trash = trash_dir();
        std::fs::create_dir_all(&trash)?;
        let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
        let target = trash.join(format!("{}-{}", stamp, name.to_lowercase()));
        atomic_write(&target, text)?;
        Ok(target)
    }

    pub fn conflict_files(&self) -> std::io::Result<Vec<String>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.dir)?.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.contains(".sync-conflict-") && name.ends_with(".md") {
                out.push(name);
            }
        }
        out.sort();
        Ok(out)
    }
}

pub fn trash_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "notes")
        .map(|p| p.state_dir().unwrap_or(p.data_dir()).join("trash"))
        .unwrap_or_else(|| PathBuf::from(".notes-trash"))
}

/// Write to `.<name>.notes-tmp`, fsync, rename (§11.1).
pub fn atomic_write(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().unwrap().to_string_lossy();
    let tmp = dir.join(format!(".{}.notes-tmp", name));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn read_if_exists(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// A node reference that survives reloads: block id if the node is a block,
/// else its path of titles (§11.2).
#[derive(Debug, Clone)]
pub enum NodeKey {
    Root,
    Id(Id),
    Path(Vec<String>),
}

impl Vault {
    pub fn key_of(&self, r: NRef) -> NodeKey {
        let n = self.tree.node(r);
        if n.kind == Kind::Root {
            return NodeKey::Root;
        }
        if let Some(b) = &n.block {
            if let Some(id) = &b.id {
                return NodeKey::Id(id.clone());
            }
        }
        NodeKey::Path(self.tree.path(r))
    }

    /// Re-attach a cursor after reload: by id, then path, then nearest
    /// surviving ancestor (§11.2).
    pub fn find_by_key(&self, key: &NodeKey) -> Option<NRef> {
        match key {
            NodeKey::Root => Some(self.tree.root),
            NodeKey::Id(id) => self.tree.block_by_id(id),
            NodeKey::Path(segs) => {
                for n in (1..=segs.len()).rev() {
                    if let Some(r) = self.find_by_path(&segs[..n]) {
                        return Some(r);
                    }
                }
                None
            }
        }
    }

    /// Find a node by case-insensitive title path from the root (§3.4).
    pub fn find_by_path(&self, segs: &[String]) -> Option<NRef> {
        let mut cur = self.tree.root;
        'seg: for seg in segs {
            for c in self.tree.resolved_children(cur) {
                if self.tree.node(c).title.eq_ignore_ascii_case(seg) {
                    cur = c;
                    continue 'seg;
                }
            }
            return None;
        }
        if cur == self.tree.root && !segs.is_empty() {
            return None;
        }
        Some(cur)
    }

    /// Resolve a command target: id (or unique leading run), then path, then
    /// unique title (§3.4).
    pub fn resolve_target(&self, text: &str) -> Result<NRef, String> {
        let text = text.trim();
        // 1. id
        let unwrapped = crate::ident::unwrap_embed_text(text);
        if crate::ident::looks_like_id_prefix(unwrapped) {
            let matches: Vec<NRef> = self
                .tree
                .blocks
                .iter()
                .filter(|(_, id)| id.as_str().starts_with(unwrapped))
                .map(|(r, _)| *r)
                .collect();
            match matches.len() {
                1 => return Ok(matches[0]),
                0 => return Err(format!("no block with id prefix {:?}", text)),
                _ => return Err(format!("id prefix {:?} is ambiguous", text)),
            }
        }
        // 2. path
        if text.contains('/') {
            let rel = text.strip_prefix("./");
            let segs: Vec<String> = text.split('/').map(|s| s.trim().to_string()).collect();
            let base = match rel {
                Some(_) => return Err("./ relative resolution needs a zoom root".into()),
                None => self.tree.root,
            };
            let _ = base;
            if let Some(r) = self.find_by_path(&segs) {
                return Ok(r);
            }
            return Err(format!("no node at path {:?}", text));
        }
        // 3. unique title
        let mut found: Vec<NRef> = Vec::new();
        self.tree.walk(self.tree.root, &mut |t, r| {
            if t.node(r).title.eq_ignore_ascii_case(text) {
                found.push(r);
            }
        });
        match found.len() {
            1 => Ok(found[0]),
            0 => Err(format!("no node titled {:?}", text)),
            _ => Err(format!("title {:?} is ambiguous", text)),
        }
    }

    /// Collect every node in the resolved tree (for the outline and filter).
    pub fn all_nodes(&self) -> Vec<NRef> {
        let mut out = Vec::new();
        self.tree.walk(self.tree.root, &mut |_, r| out.push(r));
        out
    }
}

/// Map from id string to NRef, for embed resolution in the TUI.
pub fn id_map(tree: &Tree) -> HashMap<String, NRef> {
    tree.blocks
        .iter()
        .map(|(r, id)| (id.as_str().to_string(), *r))
        .collect()
}
