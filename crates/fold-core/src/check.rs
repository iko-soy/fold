//! `notes check` diagnostics (§15.7) and `--fix` canonicalization (§4.2).

use crate::ident::{filename, slug, split_filename, Id};
use crate::parse::Kind;
use crate::render::render;
use crate::tree::NRef;
use crate::vault::Vault;

pub struct Diagnostic {
    pub file: String,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

pub fn check(vault: &Vault) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let t = &vault.tree;
    // per-file parse diagnostics and non-canonical notes
    for f in &t.files {
        for d in &f.diagnostics {
            out.push(Diagnostic {
                file: f.path.clone(),
                message: d.message.clone(),
            });
        }
        for n in &f.nodes {
            for nc in &n.noncanonical {
                out.push(Diagnostic {
                    file: f.path.clone(),
                    message: format!("non-canonical: {} ({:?})", nc, n.title),
                });
            }
            if n.title.contains('/') && n.kind != Kind::Root {
                out.push(Diagnostic {
                    file: f.path.clone(),
                    message: format!("title contains '/': {:?}", n.title),
                });
            }
        }
    }
    // embeds: broken, duplicate, cyclic, with children (§6.2, §15.7)
    for f in &t.files {
        for n in &f.nodes {
            if let Some(id) = &n.embed {
                match t.block_by_id(id) {
                    None => out.push(Diagnostic {
                        file: f.path.clone(),
                        message: format!("broken embed: no block with id {}", id),
                    }),
                    Some(_) => {}
                }
                if !n.children.is_empty() {
                    out.push(Diagnostic {
                        file: f.path.clone(),
                        message: format!("embed has children in the parent file: {}", id),
                    });
                }
                let count = t
                    .files
                    .iter()
                    .flat_map(|f2| f2.nodes.iter())
                    .filter(|n2| n2.embed.as_ref() == Some(id))
                    .count();
                if count > 1 {
                    out.push(Diagnostic {
                        file: f.path.clone(),
                        message: format!("duplicate embed for id {}", id),
                    });
                }
            }
        }
    }
    // block files: exactly one root, valid unique ids (§4.9, §6.2)
    let mut seen_ids: Vec<&Id> = Vec::new();
    for (r, id) in &t.blocks {
        if seen_ids.contains(&id) {
            out.push(Diagnostic {
                file: t.node(*r).block.as_ref().unwrap().path.clone(),
                message: format!("duplicate id {}", id),
            });
        }
        seen_ids.push(id);
        let n = t.node(*r);
        let f = &t.files[r.0];
        let top: Vec<usize> = f.nodes[f.root_node].children.clone();
        if top.len() != 1 {
            out.push(Diagnostic {
                file: f.path.clone(),
                message: "block file must have exactly one node at column 0".into(),
            });
        }
        let _ = n;
        // filename checks (§6.4)
        let b = t.node(*r).block.as_ref().unwrap();
        let words = id.words();
        match split_filename(&b.path) {
            Some((prefix, name)) => {
                let plen = prefix.split('-').count();
                if plen > 4 || words[..plen.min(4)].join("-") != prefix {
                    out.push(Diagnostic {
                        file: b.path.clone(),
                        message: format!("filename prefix {} is not a leading run of the id", prefix),
                    });
                }
                let want = slug(&t.node(*r).title);
                if name != want {
                    out.push(Diagnostic {
                        file: b.path.clone(),
                        message: format!("filename name {} does not match the title ({})", name, want),
                    });
                }
            }
            None => out.push(Diagnostic {
                file: b.path.clone(),
                message: "filename is not <prefix>~<name>.md".into(),
            }),
        }
        // task key sanity
        if let Some(todo) = b.prop("todo") {
            if todo != "open" && todo != "done" {
                out.push(Diagnostic {
                    file: b.path.clone(),
                    message: format!("todo: must be open or done, got {:?}", todo),
                });
            }
        }
        for k in ["due", "done"] {
            if let Some(v) = b.prop(k) {
                if !is_iso_date(v) {
                    out.push(Diagnostic {
                        file: b.path.clone(),
                        message: format!("{}: is not an ISO date: {:?}", k, v),
                    });
                }
            }
        }
    }
    // unresolved conflict blocks (§12.5)
    for (r, _) in &t.blocks {
        if let Some(b) = &t.node(*r).block {
            if b.prop("conflict").is_some() {
                out.push(Diagnostic {
                    file: b.path.clone(),
                    message: "unresolved conflict block".into(),
                });
            }
        }
    }
    // duplicate Inbox sections (§7)
    let root = t.root;
    let inboxes: Vec<NRef> = t
        .resolved_children(root)
        .into_iter()
        .filter(|&c| t.node(c).title.eq_ignore_ascii_case("inbox"))
        .collect();
    for extra in inboxes.iter().skip(1) {
        out.push(Diagnostic {
            file: "root.md".into(),
            message: format!("duplicate Inbox section ({:?})", t.node(*extra).title),
        });
    }
    // ignored .md files (§4.1)
    if let Ok(ignored) = vault.ignored_files() {
        for i in ignored {
            out.push(Diagnostic {
                file: i.path,
                message: format!("ignored: {}", i.reason),
            });
        }
    }
    out
}

pub fn is_iso_date(v: &str) -> bool {
    v.len() == 10
        && v.as_bytes()[4] == b'-'
        && v.as_bytes()[7] == b'-'
        && v.bytes()
            .enumerate()
            .all(|(i, b)| (i == 4 || i == 7) || b.is_ascii_digit())
}

/// Canonicalize every file and repair filenames (§4.2, §6.4, §13 `--fix`).
/// Returns the number of files rewritten or renamed.
///
/// Nothing is dropped: root.md keeps its frontmatter and any text before its
/// first node verbatim, embeds stay embeds, and a block file that is
/// malformed (§4.9: read-only until fixed by hand) is left alone.
pub fn fix(vault: &mut Vault) -> std::io::Result<usize> {
    let mut count = 0;
    for i in 0..vault.tree.files.len() {
        let f = &vault.tree.files[i];
        let top: Vec<usize> = f.nodes[f.root_node].children.clone();
        let canonical = if i == 0 {
            let Some(&first) = top.first() else { continue };
            // frontmatter and intro text, exactly as found
            let mut s = f.text[..f.nodes[first].span.start].to_string();
            for (idx, &k) in top.iter().enumerate() {
                let kr = (i, k);
                let kn = vault.tree.node(kr);
                if idx > 0 && (kn.kind == Kind::Section || vault.tree.node((i, top[idx - 1])).kind == Kind::Section) {
                    s.push('\n');
                }
                match &kn.embed {
                    Some(id) => {
                        s.push_str("![[");
                        s.push_str(id.as_str());
                        s.push_str("]]\n");
                    }
                    None => s.push_str(&render(&vault.tree, kr, 1, false)),
                }
            }
            s
        } else {
            let malformed = top.len() != 1
                || f.diagnostics.iter().any(|d| {
                    d.message.contains("more than one node") || d.message.contains("before the block")
                });
            if malformed {
                continue;
            }
            render(&vault.tree, (i, top[0]), 1, false)
        };
        if canonical != f.text {
            vault.write_file_text(i, &canonical)?;
            count += 1;
        }
    }
    // repair filenames (§6.4): an existing prefix that is a leading run of
    // the id is kept — prefixes are never lengthened — unless a file decided
    // earlier already holds it (a collision: the later file moves)
    let current: Vec<(usize, String)> = vault
        .tree
        .blocks
        .iter()
        .map(|(r, _)| {
            let path = &vault.tree.node(*r).block.as_ref().unwrap().path;
            (r.0, split_filename(path).map(|(p, _)| p.to_string()).unwrap_or_default())
        })
        .collect();
    let mut decided: Vec<String> = Vec::new();
    let mut renames: Vec<(usize, String)> = Vec::new();
    for (i, (r, id)) in vault.tree.blocks.iter().enumerate() {
        let b = vault.tree.node(*r).block.as_ref().unwrap();
        let words = id.words();
        let keep = split_filename(&b.path).and_then(|(p, _)| {
            let n = p.split('-').count();
            (n <= 4 && words[..n].join("-") == p && !decided.iter().any(|d| d == p))
                .then(|| p.to_string())
        });
        let prefix = keep.unwrap_or_else(|| {
            let others: Vec<&str> = decided
                .iter()
                .map(String::as_str)
                .chain(current[i + 1..].iter().map(|(_, p)| p.as_str()))
                .collect();
            id.shortest_prefix(&|cand| others.contains(&cand))
        });
        let want = filename(&prefix, &slug(&vault.tree.node(*r).title));
        if want != b.path {
            renames.push((r.0, want));
        }
        decided.push(prefix);
    }
    for (file, want) in renames {
        let old = vault.tree.files[file].path.clone();
        let target = vault.dir.join(&want);
        if target.exists() {
            continue; // never overwrite; check keeps reporting it
        }
        std::fs::rename(vault.dir.join(&old), target)?;
        count += 1;
    }
    if count > 0 {
        vault.reload()?;
    }
    Ok(count)
}
