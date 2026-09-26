//! `notes check` diagnostics (§15.7) and `--fix` canonicalization (§4.2).

use crate::ident::{filename, slug, split_filename, Id};
use crate::parse::{ends_with_blank_line, Content, Kind};
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
    for (fi, f) in t.files.iter().enumerate() {
        for (ni, n) in f.nodes.iter().enumerate() {
            if let Some(id) = &n.embed {
                match t.block_by_id(id) {
                    None => out.push(Diagnostic {
                        file: f.path.clone(),
                        message: format!("broken embed: no block with id {}", id),
                    }),
                    Some(b) if form_mismatch(n.kind, t.node(b).kind) => out.push(Diagnostic {
                        file: f.path.clone(),
                        message: format!(
                            "embed form does not match the block's spelling: {} (§4.7)",
                            id
                        ),
                    }),
                    Some(_) => {}
                }
                if let Some(w) = embed_level_mismatch(t, (fi, ni)) {
                    out.push(Diagnostic {
                        file: f.path.clone(),
                        message: format!(
                            "heading embed at level {} where its position gives {}: {} (§4.7)",
                            w,
                            t.derived_level((fi, ni)),
                            id
                        ),
                    });
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
    // cyclic embeds (§6.2): follow each block up through the embed that
    // holds it until root.md, a block embedded nowhere, or the block itself
    for (r, id) in &t.blocks {
        let mut seen: Vec<&Id> = vec![id];
        let mut cur = id;
        while let Some(e) = t.embed_of(cur) {
            // the block whose file holds that embed; none: root.md
            let Some((_, holder)) = t.blocks.iter().find(|(b, _)| b.0 == e.0) else { break };
            if holder == id {
                out.push(Diagnostic {
                    file: t.node(*r).block.as_ref().unwrap().path.clone(),
                    message: format!("cyclic embed: block {} is embedded inside itself", id),
                });
                break;
            }
            if seen.contains(&holder) {
                break; // a cycle above it, reported by the blocks on it
            }
            seen.push(holder);
            cur = holder;
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
                        message: format!(
                            "filename name {} does not match the title ({}): stale, harmless; --fix renames",
                            name, want
                        ),
                    });
                }
            }
            None => out.push(Diagnostic {
                file: b.path.clone(),
                message: "filename is not <prefix>~<name>.md".into(),
            }),
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

/// A heading embed's written level, when it is not the level its position
/// gives (§4.7). Levels are honoured as written, like any heading's, so a
/// stale one is reported rather than trusted to mean something.
fn embed_level_mismatch(t: &crate::tree::Tree, e: NRef) -> Option<usize> {
    let n = t.node(e);
    let written = n.level?;
    (n.embed.is_some() && n.kind == Kind::Section && written != t.derived_level(e))
        .then_some(written)
}

/// The text of `e`'s file with heading embed `e` at the level of its
/// position and the structure unchanged (§4.7). Its later sibling sections
/// written deeper than that level move to it too (it is their position's
/// level as well): left alone, the shallower embed would take them as its
/// children. `None` when `e` has no stale level, or when no level keeps it
/// where it is (its parent section is itself written at that level or
/// deeper), and `check` goes on reporting it.
fn embed_level_fix(t: &crate::tree::Tree, e: NRef) -> Option<String> {
    embed_level_mismatch(t, e)?;
    let lvl = t.derived_level(e);
    let f = &t.files[e.0];
    let parent = &f.nodes[f.nodes[e.1].parent?];
    if parent.kind == Kind::Section && parent.level.unwrap_or(1) >= lvl {
        return None;
    }
    let from = parent.children.iter().position(|&c| c == e.1)?;
    let mut text = f.text.clone();
    // last to first, so the spans before each edit stay valid
    for &c in parent.children[from..].iter().rev() {
        let n = &f.nodes[c];
        let Some(w) = n.level.filter(|&w| w > lvl || c == e.1) else { continue };
        let line = n.title_span.text(&f.text);
        let at = n.title_span.start + line.len() - line.trim_start_matches([' ', '\t']).len();
        if !f.text[at..].starts_with(&"#".repeat(w)) {
            return None; // not an ATX heading
        }
        text.replace_range(at..at + w, &"#".repeat(lvl));
    }
    Some(text)
}

/// A heading embed must embed a section, a bare one an item (§4.7).
fn form_mismatch(embed: Kind, block: Kind) -> bool {
    (embed == Kind::Section) != (block == Kind::Section)
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
    // a blank line before every text child that follows a child node (§4.2)
    for i in 0..vault.tree.files.len() {
        let f = &vault.tree.files[i];
        let mut at: Vec<usize> = f
            .nodes
            .iter()
            .flat_map(|n| crate::parse::unseparated_text(n, &f.text))
            .collect();
        if at.is_empty() {
            continue;
        }
        at.sort_unstable();
        let mut text = f.text.clone();
        for &p in at.iter().rev() {
            text.insert(p, '\n');
        }
        vault.write_file_text(i, &text)?;
        count += 1;
    }
    for i in 0..vault.tree.files.len() {
        let f = &vault.tree.files[i];
        let top: Vec<usize> = f.nodes[f.root_node].children.clone();
        let canonical = if i == 0 {
            let Some(&first) = top.first() else { continue };
            // frontmatter and intro text, exactly as found; then the root's
            // children in order — nodes rendered, text children verbatim,
            // one blank line between a node and what follows it unless both
            // are items of a tight list (§4.2); a line is blank whatever its
            // line ending, so a CRLF file keeps its loose lists
            let mut s = f.text[..f.nodes[first].span.start].to_string();
            let root = &f.nodes[f.root_node];
            let from = root
                .content
                .iter()
                .position(|c| *c == Content::Node(first))
                .unwrap_or(0);
            let mut prev: Option<usize> = None;
            for c in &root.content[from..] {
                let loose_after_prev = prev.map(|p| {
                    let pn = &f.nodes[p];
                    pn.kind == Kind::Section || ends_with_blank_line(pn.span.text(&f.text))
                });
                match *c {
                    Content::Node(k) => {
                        let kn = &f.nodes[k];
                        let blank = match loose_after_prev {
                            Some(loose) => loose || kn.kind == Kind::Section,
                            None => !s.is_empty() && !ends_with_blank_line(&s),
                        };
                        if blank && !ends_with_blank_line(&s) {
                            s.push('\n');
                        }
                        s.push_str(&render(&vault.tree, (i, k), 1, false));
                        prev = Some(k);
                    }
                    Content::Text(sp) => {
                        if prev.is_some() && !ends_with_blank_line(&s) {
                            s.push('\n');
                        }
                        let t = sp.text(&f.text);
                        s.push_str(t.trim_end_matches('\n'));
                        s.push('\n');
                        prev = None;
                    }
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
    // embeds in the form of their block's spelling (§4.7), one at a time:
    // each respell may move an embed and re-parse its file
    for _ in 0..vault.tree.blocks.len() {
        let t = &vault.tree;
        let wrong = t.blocks.iter().find_map(|(b, id)| {
            let e = t.embed_of(id)?;
            form_mismatch(t.node(e).kind, t.node(*b).kind)
                .then(|| (e, t.node(*b).kind == Kind::Section))
        });
        let Some((e, to_section)) = wrong else { break };
        crate::ops::respell_embed(vault, e, to_section)?;
        count += 1;
    }
    // heading embeds at the level of their position (§4.7), one at a time,
    // with no line changing parent
    for _ in 0..10_000 {
        let t = &vault.tree;
        let fixed = t.files.iter().enumerate().find_map(|(fi, f)| {
            (0..f.nodes.len()).find_map(|ni| embed_level_fix(t, (fi, ni)).map(|text| (fi, text)))
        });
        let Some((file, text)) = fixed else { break };
        vault.write_file_text(file, &text)?;
        count += 1;
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
