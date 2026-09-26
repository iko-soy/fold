//! The vault on disk: loading, indexing, atomic span-preserving writes,
//! trash (§4.1, §11).

use crate::ident::{split_filename, Id};
use crate::parse::{parse_file, parse_frontmatter, Block, Kind, ParsedFile, Span};
use crate::tree::{NRef, Tree};
use std::path::{Path, PathBuf};

pub struct Vault {
    pub dir: PathBuf,
    pub tree: Tree,
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
        };
        v.reload()?;
        Ok(v)
    }

    /// Rebuild the index from the vault (§11.3).
    pub fn reload(&mut self) -> std::io::Result<()> {
        let mut files: Vec<ParsedFile> = Vec::new();
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
        };
        files.push(parse_file("root.md", &root_text, 0, Some(root_block)));

        let mut entries: Vec<String> = std::fs::read_dir(&self.dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".md") && n != "root.md" && !n.starts_with('.'))
            .collect();
        entries.sort();
        for name in entries {
            if name.contains(".sync-conflict-") {
                continue; // merge engine only (§11.4)
            }
            // anything unreadable — a directory named `x.md`, non-UTF-8
            // text — is simply ignored (§4.1), never fatal
            let text = match read_md_file(&self.dir.join(&name)) {
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
            let block = Block {
                id: Some(id),
                path: name.clone(),
                props: fm.as_ref().map(|f| f.props.clone()).unwrap_or_default(),
                frontmatter_raw: fm.as_ref().map(|f| f.raw.clone()).unwrap_or_default(),
                frontmatter_span: fm.as_ref().map(|f| f.span),
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

        self.tree = Tree {
            files,
            root: (0, 0),
            blocks,
        };
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
                let Some(text) = read_md_file(&e.path()) else {
                    out.push(IgnoredFile {
                        path: name,
                        reason: "not a readable UTF-8 file".into(),
                    });
                    continue;
                };
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

    /// Replace a byte span in a file atomically (§11.1), then re-parse.
    pub fn write_span(&mut self, file: usize, span: Span, replacement: &str) -> std::io::Result<()> {
        let f = &self.tree.files[file];
        // a node's span may run one byte past a file without a final newline
        let end = span.end.min(f.text.len());
        let start = span.start.min(end);
        let mut new_text = String::with_capacity(f.text.len() + replacement.len());
        new_text.push_str(&f.text[..start]);
        new_text.push_str(replacement);
        new_text.push_str(&f.text[end..]);
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
        self.tree.blocks = blocks;
    }

    /// Shortest prefix of `id` not already used by another file (§6.4).
    pub fn unique_prefix(&self, id: &Id) -> String {
        self.unique_prefix_except(id, None)
    }

    /// Like `unique_prefix`, but ignoring the file at `own` — a block's own
    /// file never counts against it.
    pub fn unique_prefix_except(&self, id: &Id, own: Option<&str>) -> String {
        let taken = self.taken_prefixes(own);
        id.shortest_prefix(&|cand| taken.iter().any(|t| t == cand))
    }

    /// Prefixes used by block files other than `own` (§6.4).
    pub fn taken_prefixes(&self, own: Option<&str>) -> Vec<String> {
        self.tree
            .files
            .iter()
            .skip(1)
            .filter(|f| Some(f.path.as_str()) != own)
            .filter_map(|f| split_filename(&f.path).map(|(p, _)| p.to_string()))
            .collect()
    }

    /// Move a file to the trash (§11.5) and remove it from the vault.
    pub fn trash_file(&mut self, file: usize) -> std::io::Result<()> {
        let path = self.tree.files[file].path.clone();
        let trash = trash_dir();
        std::fs::create_dir_all(&trash)?;
        let target = trash_target(&trash, &path);
        move_file(&self.dir.join(&path), &target)?;
        self.reload()?;
        Ok(())
    }

    /// Write `text` to the trash under `name` (§11.5).
    pub fn trash_text(&self, name: &str, text: &str) -> std::io::Result<PathBuf> {
        let trash = trash_dir();
        std::fs::create_dir_all(&trash)?;
        let target = trash_target(&trash, name);
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

/// A fresh trash path `<timestamp>-<name>`; a second entry in the same
/// second gets a numeric suffix instead of overwriting the first (§11.5).
fn trash_target(trash: &Path, name: &str) -> PathBuf {
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let name = name.to_lowercase();
    let first = trash.join(format!("{}-{}", stamp, name));
    if !first.exists() {
        return first;
    }
    let stem = name.strip_suffix(".md").unwrap_or(&name);
    (2..)
        .map(|i| trash.join(format!("{}-{}-{}.md", stamp, stem, i)))
        .find(|p| !p.exists())
        .unwrap()
}

pub fn trash_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "fold")
        .map(|p| p.state_dir().unwrap_or(p.data_dir()).join("trash"))
        .unwrap_or_else(|| PathBuf::from(".notes-trash"))
}

/// Write to `.<name>.fold-tmp`, fsync, rename (§11.1).
pub fn atomic_write(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().unwrap().to_string_lossy();
    let tmp = dir.join(format!(".{}.fold-tmp", name));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Rename, falling back to copy-and-remove when the trash and the vault
/// are on different filesystems.
pub fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}

/// Read a vault `.md` file as text; `None` for anything that is not a
/// readable UTF-8 regular file.
fn read_md_file(path: &Path) -> Option<String> {
    if !std::fs::metadata(path).map(|m| m.is_file()).unwrap_or(false) {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn read_if_exists(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// A node reference that survives reloads (§3.4, §11.2): a block by its id;
/// any other node by its steps from the root of the file it lives in, each
/// step a title and the node's ordinal among same-titled siblings, so two
/// siblings with one title (or none) are never confused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKey {
    Root,
    Id(Id),
    Path {
        /// The block whose file holds the node; `None` for root.md.
        block: Option<Id>,
        steps: Vec<(String, usize)>,
    },
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
        let f = &self.tree.files[r.0];
        let mut steps = Vec::new();
        let mut cur = r.1;
        let mut block = None;
        while let Some(p) = f.nodes[cur].parent {
            let title = &f.nodes[cur].title;
            let ordinal = f.nodes[p]
                .children
                .iter()
                .take_while(|&&c| c != cur)
                .filter(|&&c| f.nodes[c].title == *title)
                .count();
            steps.push((title.clone(), ordinal));
            if let Some(id) = f.nodes[p].block.as_ref().and_then(|b| b.id.clone()) {
                block = Some(id);
                break;
            }
            cur = p;
        }
        steps.reverse();
        NodeKey::Path { block, steps }
    }

    /// Re-attach a node after a reload: by id, else by its steps, falling
    /// back to the deepest step that still exists (§11.2).
    pub fn find_by_key(&self, key: &NodeKey) -> Option<NRef> {
        match key {
            NodeKey::Root => Some(self.tree.root),
            NodeKey::Id(id) => self.tree.block_by_id(id),
            NodeKey::Path { block, steps } => {
                let mut cur = match block {
                    Some(id) => self.tree.block_by_id(id)?,
                    None => self.tree.root,
                };
                let mut found = None;
                for (title, ordinal) in steps {
                    let f = &self.tree.files[cur.0];
                    let next = f.nodes[cur.1]
                        .children
                        .iter()
                        .filter(|&&c| f.nodes[c].title == *title)
                        .nth(*ordinal);
                    match next {
                        Some(&c) => {
                            cur = (cur.0, c);
                            found = Some(cur);
                        }
                        None => break,
                    }
                }
                found.or(match block {
                    Some(id) => self.tree.block_by_id(id),
                    None => None,
                })
            }
        }
    }

    /// Find a node by case-insensitive title path from the root (§3.4).
    pub fn find_by_path(&self, segs: &[String]) -> Option<NRef> {
        let mut cur = self.tree.root;
        'seg: for seg in segs {
            for c in self.tree.resolved_children(cur) {
                if title_eq(&self.tree.node(c).title, seg) {
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
                // a word that merely looks like an id may still be a title
                0 => {}
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
            if title_eq(&t.node(r).title, text) {
                found.push(r);
            }
        });
        match found.len() {
            1 => Ok(found[0]),
            0 => Err(format!("no node titled {:?}", text)),
            _ => Err(format!("title {:?} is ambiguous", text)),
        }
    }

}

/// Case-insensitive title comparison, Unicode-aware (§3.4).
pub fn title_eq(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

