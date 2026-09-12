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
pub fn fix(vault: &mut Vault) -> std::io::Result<usize> {
    let mut count = 0;
    // rewrite each block file from its canonical render
    for i in 0..vault.tree.files.len() {
        let f = &vault.tree.files[i];
        let canonical = if i == 0 {
            // root.md: re-emit the whole file's top-level nodes
            let mut s = String::new();
            let kids = vault.tree.resolved_children(vault.tree.root);
            for (idx, k) in kids.iter().enumerate() {
                if idx > 0 && vault.tree.node(*k).kind == Kind::Section {
                    s.push('\n');
                }
                s.push_str(&render(&vault.tree, *k, 1, false));
            }
            s
        } else {
            let root_node = f.nodes[f.root_node].children[0];
            render(&vault.tree, (i, root_node), 1, false)
        };
        if canonical != f.text {
            vault.write_file_text(i, &canonical)?;
            count += 1;
        }
    }
    // repair filenames (§6.4)
    let renames: Vec<(usize, String)> = vault
        .tree
        .blocks
        .iter()
        .filter_map(|(r, id)| {
            let b = vault.tree.node(*r).block.as_ref().unwrap();
            let prefix = vault.unique_prefix(id);
            let want = filename(&prefix, &slug(&vault.tree.node(*r).title));
            if want != b.path {
                Some((r.0, want))
            } else {
                None
            }
        })
        .collect();
    for (file, want) in renames {
        let old = vault.tree.files[file].path.clone();
        std::fs::rename(vault.dir.join(&old), vault.dir.join(&want))?;
        count += 1;
    }
    if count > 0 {
        vault.reload()?;
    }
    Ok(count)
}
