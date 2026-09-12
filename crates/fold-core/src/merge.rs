//! Two-way node-level merge (§12). One algorithm, one code path: sync
//! conflicts (`X.sync-conflict-*.md`) and in-session splice conflicts.

use crate::ident::{filename, slug, Id};
use crate::parse::{parse_file, Block, Kind, TaskState};
use crate::tree::{NRef, Tree};
use crate::vault::Vault;

/// The outcome of merging one file pair.
pub struct MergeOutcome {
    /// The merged text for `X.md`.
    pub text: String,
    /// Conflict blocks created: (filename, text) — the "theirs" copies (§12.4).
    pub conflict_blocks: Vec<(String, String)>,
    /// Number of conflict pairs raised.
    pub conflicts: usize,
}

/// Merge two versions of one file. `device`/`timestamp` stamp the conflict
/// blocks (§12.4).
pub fn merge_texts(ours: &str, theirs: &str, device: &str, timestamp: &str) -> MergeOutcome {
    let o = standalone_tree(ours);
    let t = standalone_tree(theirs);
    // If both are block files, ids must match (§12.2); the caller handles
    // the prefix-collision case before calling us.
    let mut conflicts = 0usize;
    let mut conflict_blocks = Vec::new();
    let merged_root = merge_children(
        &o,
        &t,
        o.root,
        t.root,
        &mut conflicts,
        &mut conflict_blocks,
        device,
        timestamp,
        1,
        0,
    );
    let mut text = merged_root;
    // frontmatter merge for block files: key by key (§12.4)
    text = merge_frontmatter(ours, theirs, &text, &mut conflicts, &mut conflict_blocks, device, timestamp);
    MergeOutcome {
        text,
        conflict_blocks,
        conflicts,
    }
}

fn standalone_tree(text: &str) -> Tree {
    let block = Block {
        id: None,
        path: "m.md".into(),
        props: Default::default(),
        frontmatter_raw: String::new(),
        frontmatter_span: None,
        edge_span: None,
    };
    let pf = parse_file("m.md", text, 0, Some(block));
    Tree {
        files: vec![pf],
        root: (0, 0),
        blocks: vec![],
    }
}

fn node_sig(t: &Tree, r: NRef) -> (String, Option<TaskState>, String) {
    let n = t.node(r);
    let body = n.body_lines(t.text_of(r)).join("\n");
    (n.title.clone(), n.task, body)
}

/// Match children of two parents: by embed id, then exact title (§12.4).
/// Returns the merged children rendered at (`level`, `indent`).
#[allow(clippy::too_many_arguments)]
fn merge_children(
    o: &Tree,
    t: &Tree,
    op: NRef,
    tp: NRef,
    conflicts: &mut usize,
    conflict_blocks: &mut Vec<(String, String)>,
    device: &str,
    timestamp: &str,
    level: usize,
    indent: usize,
) -> String {
    let o_kids = o.resolved_children(op);
    let t_kids = t.resolved_children(tp);
    // match: embed by id, else by exact title among unmatched siblings
    let mut t_used = vec![false; t_kids.len()];
    let mut pairs: Vec<(Option<NRef>, Option<NRef>)> = Vec::new();
    for &ok in &o_kids {
        let on = o.node(ok);
        let mut found = None;
        for (i, &tk) in t_kids.iter().enumerate() {
            if t_used[i] {
                continue;
            }
            let tn = t.node(tk);
            if let (Some(a), Some(b)) = (&on.embed, &tn.embed) {
                if a == b {
                    found = Some(i);
                    break;
                }
                continue;
            }
            if on.embed.is_none() && tn.embed.is_none() && on.title == tn.title {
                found = Some(i);
                break;
            }
        }
        if let Some(i) = found {
            t_used[i] = true;
            pairs.push((Some(ok), Some(t_kids[i])));
        } else {
            pairs.push((Some(ok), None));
        }
    }
    for (i, &tk) in t_kids.iter().enumerate() {
        if !t_used[i] {
            pairs.push((None, Some(tk)));
        }
    }
    // Emit: matched pairs merge field-by-field; single-sided nodes are
    // insertions; differing fields raise conflict pairs (§12.4).
    let mut out = String::new();
    for (ok, tk) in pairs {
        match (ok, tk) {
            (Some(a), Some(b)) => {
                out.push_str(&merge_pair(
                    o, t, a, b, conflicts, conflict_blocks, device, timestamp, level, indent,
                ));
            }
            (Some(a), None) => {
                out.push_str(&emit_subtree(o, a, level, indent));
            }
            (None, Some(b)) => {
                out.push_str(&emit_subtree(t, b, level, indent));
            }
            (None, None) => {}
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn merge_pair(
    o: &Tree,
    t: &Tree,
    a: NRef,
    b: NRef,
    conflicts: &mut usize,
    conflict_blocks: &mut Vec<(String, String)>,
    device: &str,
    timestamp: &str,
    level: usize,
    indent: usize,
) -> String {
    let on = o.node(a);
    let tn = t.node(b);
    // embeds matched by id: identical reference, emit ours
    if on.embed.is_some() {
        return emit_subtree(o, a, level, indent);
    }
    let (otitle, otask, obody) = node_sig(o, a);
    let (_ttitle, ttask, tbody) = node_sig(t, b);
    let differ = otitle != tn.title || otask != ttask || obody != tbody;
    // children merge
    let child_level = match on.kind {
        Kind::Section => level + 1,
        _ => level,
    };
    let child_indent = match on.kind {
        Kind::Item => indent + 2,
        _ => indent,
    };
    let merged_kids = merge_children(
        o, t, a, b, conflicts, conflict_blocks, device, timestamp, child_level, child_indent,
    );
    let mut out = emit_node_with(o, a, level, indent, &merged_kids);
    if differ {
        *conflicts += 1;
        // theirs becomes a conflict block stitched in next to ours (§12.4)
        let name = slug(&otitle);
        let id = Id::generate();
        let prefix = id.words()[0].to_string();
        let fname = filename(&prefix, &name);
        let mut block_text = format!("---\nid: {}\nconflict: \"{} {}\"\n", id, device, timestamp);
        if let Some(ts) = ttask {
            block_text.push_str(&format!(
                "todo: {}\n",
                match ts {
                    TaskState::Open => "open",
                    TaskState::Done => "done",
                }
            ));
        }
        block_text.push_str("---\n\n");
        block_text.push_str(&emit_subtree_with_state(t, b, 1, 0));
        conflict_blocks.push((fname, block_text));
        // the embed goes right after ours
        out.push_str(&format!("{}![[{}]]\n", " ".repeat(indent), id));
    }
    out
}

/// Render a node at (level, indent) with given already-rendered children.
fn emit_node_with(o: &Tree, a: NRef, level: usize, indent: usize, kids: &str) -> String {
    emit_node_with_impl(o, a, level, indent, kids, false)
}

fn emit_node_with_impl(
    o: &Tree,
    a: NRef,
    level: usize,
    indent: usize,
    kids: &str,
    show_block_state: bool,
) -> String {
    let n = o.node(a);
    let mut out = String::new();
    let ind = " ".repeat(indent);
    // A block root's task state normally lives only in frontmatter (§4.9);
    // when emitting for resolution we surface it as a checkbox so the
    // receiving side parses it back as state.
    let is_block_root = n.is_block() && !show_block_state;
    match n.kind {
        Kind::Section => {
            out.push_str(&ind);
            out.push_str(&"#".repeat(level.max(1)));
            out.push(' ');
            if !is_block_root {
                push_task(n.task, &mut out);
            }
            out.push_str(&n.title);
            out.push('\n');
        }
        Kind::Item => {
            out.push_str(&ind);
            out.push_str("- ");
            if !is_block_root {
                push_task(n.task, &mut out);
            }
            out.push_str(&n.title);
            out.push('\n');
        }
        Kind::Root => {}
    }
    let body = n.body_lines(o.text_of(a));
    let mut body: Vec<&str> = body;
    while body.first().map(|l| l.trim().is_empty()) == Some(true) {
        body.remove(0);
    }
    while body.last().map(|l| l.trim().is_empty()) == Some(true) {
        body.pop();
    }
    if !body.is_empty() && n.kind != Kind::Root {
        out.push('\n');
    }
    let dedent_by = o.indent(a);
    for l in &body {
        out.push_str(&ind);
        out.push_str(dedent(l, dedent_by));
        out.push('\n');
    }
    if !kids.trim().is_empty() {
        if n.kind != Kind::Root || !body.is_empty() {
            out.push('\n');
        }
        out.push_str(kids);
    }
    out
}

fn emit_subtree(t: &Tree, r: NRef, level: usize, indent: usize) -> String {
    emit_subtree_impl(t, r, level, indent, false)
}

/// Emit with the block root's `todo:` shown as a checkbox — used for
/// conflict blocks so resolution can lift their state back (§12.5).
fn emit_subtree_with_state(t: &Tree, r: NRef, level: usize, indent: usize) -> String {
    emit_subtree_impl(t, r, level, indent, true)
}

fn emit_subtree_impl(t: &Tree, r: NRef, level: usize, indent: usize, show_block_state: bool) -> String {
    let kids = t.resolved_children(r);
    let child_level = match t.node(r).kind {
        Kind::Section => level + 1,
        _ => level,
    };
    let child_indent = match t.node(r).kind {
        Kind::Item => indent + 2,
        _ => indent,
    };
    let mut kid_text = String::new();
    for (i, &k) in kids.iter().enumerate() {
        if i > 0 && t.node(k).kind == Kind::Section {
            kid_text.push('\n');
        }
        kid_text.push_str(&emit_subtree_impl(t, k, child_level, child_indent, show_block_state));
    }
    emit_node_with_impl(t, r, level, indent, &kid_text, show_block_state)
}

fn push_task(task: Option<TaskState>, out: &mut String) {
    match task {
        Some(TaskState::Open) => out.push_str("[ ] "),
        Some(TaskState::Done) => out.push_str("[x] "),
        None => {}
    }
}

fn dedent(line: &str, cols: usize) -> &str {
    if cols == 0 {
        return line;
    }
    let mut removed = 0;
    for (i, ch) in line.char_indices() {
        if removed >= cols {
            return &line[i..];
        }
        match ch {
            ' ' => removed += 1,
            '\t' => removed += 4,
            _ => return &line[i..],
        }
    }
    ""
}

/// Merge the frontmatter of two block files key by key (§12.4). Ours wins
/// equal values; differing values keep ours in place and the whole "theirs"
/// document becomes a conflict block (already handled at the node level for
/// title/body; here we only fold in keys).
fn merge_frontmatter(
    ours: &str,
    theirs: &str,
    merged: &str,
    conflicts: &mut usize,
    _conflict_blocks: &mut Vec<(String, String)>,
    _device: &str,
    _timestamp: &str,
) -> String {
    use crate::parse::parse_frontmatter;
    let ofm = parse_frontmatter(ours);
    let tfm = parse_frontmatter(theirs);
    let (Some(ofm), Some(tfm)) = (ofm, tfm) else {
        return merged.to_string();
    };
    // key-level: every key that differs counts as a conflict; value from O
    // stays. We re-emit frontmatter with O's raw text plus T-only keys dropped
    // (they live in the conflict block's own frontmatter).
    let raw = ofm.raw.clone();
    for (k, v) in &tfm.props {
        match ofm.props.get(k) {
            Some(ov) if ov == v => {}
            Some(_) => *conflicts += 1,
            None => *conflicts += 1,
        }
    }
    // Rebuild the file: merged frontmatter + merged body (strip merged's
    // frontmatter if any).
    let body = match crate::parse::parse_frontmatter(merged) {
        Some(mfm) => merged[mfm.span.end..].to_string(),
        None => merged.to_string(),
    };
    if raw.trim().is_empty() {
        body
    } else {
        format!("---\n{}---\n\n{}", raw, body)
    }
}

// ------------------------------------------------------------ vault-level

/// Process every `*.sync-conflict-*.md` in the vault (§12.2, §13 `notes merge`).
/// Returns a list of human-readable outcomes.
pub fn merge_sync_conflicts(vault: &mut Vault, dry_run: bool) -> std::io::Result<Vec<String>> {
    let mut outcomes = Vec::new();
    for cfile in vault.conflict_files()? {
        let base = cfile.split(".sync-conflict-").next().unwrap().to_string() + ".md";
        let cpath = vault.dir.join(&cfile);
        let bpath = vault.dir.join(&base);
        let theirs = std::fs::read_to_string(&cpath)?;
        let ours = std::fs::read_to_string(&bpath).unwrap_or_default();
        // device + timestamp from the filename
        let (device, stamp) = parse_conflict_name(&cfile);
        // id check: differing ids mean a prefix collision, not a conflict (§12.2)
        let oid = crate::parse::parse_frontmatter(&ours)
            .and_then(|f| f.props.get("id").cloned())
            .and_then(|v| Id::parse(&v));
        let tid = crate::parse::parse_frontmatter(&theirs)
            .and_then(|f| f.props.get("id").cloned())
            .and_then(|v| Id::parse(&v));
        match (&oid, &tid) {
            (Some(a), Some(b)) if a != b => {
                outcomes.push(format!(
                    "{}: prefix collision ({} vs {}), renaming",
                    cfile, a, b
                ));
                if !dry_run {
                    // rename the conflict file with a unique prefix of its id
                    vault.reload()?;
                    let prefix = vault.unique_prefix(b);
                    let name = crate::ident::split_filename(&base)
                        .map(|(_, n)| n.to_string())
                        .unwrap_or_else(|| "untitled".into());
                    let new_name = filename(&prefix, &name);
                    std::fs::rename(&cpath, vault.dir.join(&new_name))?;
                }
                continue;
            }
            _ => {}
        }
        let outcome = merge_texts(&ours, &theirs, &device, &stamp);
        outcomes.push(format!(
            "{}: {} conflict pair(s)",
            cfile, outcome.conflicts
        ));
        if !dry_run {
            crate::vault::atomic_write(&bpath, &outcome.text)?;
            for (fname, text) in &outcome.conflict_blocks {
                crate::vault::atomic_write(&vault.dir.join(fname), text)?;
            }
            // the conflict file goes to trash (§12.4)
            let trash = crate::vault::trash_dir();
            std::fs::create_dir_all(&trash)?;
            let stamp2 = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
            std::fs::rename(&cpath, trash.join(format!("{}-{}", stamp2, cfile.to_lowercase())))?;
        }
    }
    if !dry_run {
        vault.reload()?;
    }
    Ok(outcomes)
}

fn parse_conflict_name(name: &str) -> (String, String) {
    // name.sync-conflict-<date>-<time>-<device>.md
    let mut device = "unknown".to_string();
    let mut stamp = String::new();
    if let Some(rest) = name.split(".sync-conflict-").nth(1) {
        let rest = rest.trim_end_matches(".md");
        let parts: Vec<&str> = rest.split('-').collect();
        if parts.len() >= 3 {
            stamp = format!("{}-{}", parts[0], parts[1]);
            device = parts[2..].join("-");
        } else {
            device = rest.to_string();
        }
    }
    (device, stamp)
}

/// Find unresolved conflict blocks: blocks with a `conflict:` key (§10.8).
/// Returns (ours, theirs) pairs; theirs is the block right after ours.
pub fn conflict_pairs(vault: &Vault) -> Vec<(NRef, NRef)> {
    conflict_pairs_tree(&vault.tree)
}

pub fn conflict_pairs_tree(tree: &Tree) -> Vec<(NRef, NRef)> {
    let mut out = Vec::new();
    for (r, _id) in &tree.blocks {
        let n = tree.node(*r);
        if let Some(b) = &n.block {
            if b.prop("conflict").is_some() {
                let bid = b.id.clone().unwrap();
                for (fi, f) in tree.files.iter().enumerate() {
                    for (ni, nd) in f.nodes.iter().enumerate() {
                        if nd.embed.as_ref() == Some(&bid) {
                            let p = nd.parent.map(|pp| (fi, pp)).unwrap();
                            let sibs = tree.raw_children(p);
                            if let Some(pos) = sibs.iter().position(|&s| s == (fi, ni)) {
                                if pos > 0 {
                                    let ours = sibs[pos - 1];
                                    let ours = {
                                        let rc = tree.resolved_child(ours);
                                        if rc != ours { rc } else { ours }
                                    };
                                    out.push((ours, *r));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

// ----------------------------------------------------------- resolution

/// keep ours: delete the conflict block and its embed (to trash) (§12.5).
pub fn resolve_keep_ours(vault: &mut Vault, theirs: NRef) -> std::io::Result<()> {
    let b = vault.tree.node(theirs).block.clone();
    let Some(b) = b else { return Ok(()) };
    delete_embed_and_block(vault, &b, theirs)
}

/// keep theirs: replace ours' title/body/children and frontmatter (minus
/// `id`/`conflict`) with the conflict block's, then delete it (§12.5).
pub fn resolve_keep_theirs(vault: &mut Vault, ours: NRef, theirs: NRef) -> std::io::Result<()> {
    let tb = vault.tree.node(theirs).block.clone();
    let Some(tb) = tb else { return Ok(()) };
    // 1. replace ours' subtree text with theirs' (re-levelled to ours' spot)
    let mut theirs_body = crate::render::render(&vault.tree, theirs, 1, true);
    // The conflict block's state lives in its frontmatter (§4.9); surface it
    // as a checkbox so the receiving side parses it back as task state.
    if let Some(state) = vault.tree.node(theirs).task {
        let marker = match state {
            TaskState::Open => "[ ] ",
            TaskState::Done => "[x] ",
        };
        let mut lines = theirs_body.lines();
        if let Some(first) = lines.next() {
            let inserted = if let Some(pos) = first.find("- ") {
                let mut s = first.to_string();
                s.insert_str(pos + 2, marker);
                s
            } else if let Some(pos) = first.find('#') {
                let h = first[pos..].find(' ').map(|i| pos + i + 1).unwrap_or(first.len());
                let mut s = first.to_string();
                s.insert_str(h, marker);
                s
            } else {
                first.to_string()
            };
            let rest: Vec<&str> = lines.collect();
            theirs_body = if rest.is_empty() {
                format!("{}\n", inserted)
            } else {
                format!("{}\n{}\n", inserted, rest.join("\n"))
            };
        }
    }
    let on = vault.tree.node(ours);
    if on.is_block() {
        // ours is a block: rewrite its file (after frontmatter) with theirs'
        // content; frontmatter replaced below.
        let file = ours.0;
        let f = &vault.tree.files[file];
        let fm_end = f.nodes[f.root_node]
            .children
            .first()
            .and_then(|&rn| f.nodes[rn].block.as_ref())
            .and_then(|b| b.frontmatter_span)
            .map(|s| s.end)
            .unwrap_or(0);
        let mut new_text = f.text[..fm_end].to_string();
        new_text.push_str(&theirs_body);
        vault.write_file_text(file, &new_text)?;
        // 2. frontmatter: theirs' minus id and conflict
        let tprops: Vec<(String, String)> = tb
            .props
            .iter()
            .filter(|(k, _)| k.as_str() != "id" && k.as_str() != "conflict")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // remove ours' existing keys (except id), then set theirs
        let okeys: Vec<String> = vault.tree.files[file]
            .nodes
            .iter()
            .find_map(|n| n.block.as_ref())
            .map(|b| {
                b.props
                    .keys()
                    .filter(|k| k.as_str() != "id")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for k in okeys {
            crate::ops::set_frontmatter_key(vault, file, &k, None)?;
        }
        for (k, v) in tprops {
            crate::ops::set_frontmatter_key(vault, file, &k, Some(&v))?;
        }
        // rename the file if the title changed (§6.4)
        let new_title = vault.tree.node(ours).title.clone();
        let _ = new_title;
        vault.reload()?;
    } else {
        // plain node: replace its span with theirs' re-levelled text
        let level = vault.tree.level(ours);
        let indent = vault.tree.indent(ours);
        let shifted = crate::ops::shift_document(&theirs_body, level, indent);
        let file = ours.0;
        let span = on.span;
        vault.write_span(file, span, &shifted)?;
    }
    // 3. delete the conflict block and its embed (to trash)
    let theirs_key = vault.key_of(theirs);
    if let Some(t) = vault.find_by_key(&theirs_key) {
        delete_embed_and_block(vault, &tb, t)?;
    }
    Ok(())
}

/// keep both: drop the `conflict` key; the block stays as an ordinary
/// sibling (§12.5).
pub fn resolve_keep_both(vault: &mut Vault, theirs: NRef) -> std::io::Result<()> {
    crate::ops::set_frontmatter_key(vault, theirs.0, "conflict", None)
}

/// Remove a block's embed from its parent file and trash the block file.
fn delete_embed_and_block(
    vault: &mut Vault,
    b: &crate::parse::Block,
    block_ref: NRef,
) -> std::io::Result<()> {
    // remove the embed line from whichever file holds it
    if let Some(id) = &b.id {
        for (fi, f) in vault.tree.files.iter().enumerate() {
            if let Some(ni) = f.nodes.iter().position(|nd| nd.embed.as_ref() == Some(id)) {
                let span = vault.tree.files[fi].nodes[ni].span;
                let text = vault.tree.files[fi].text.clone();
                let mut start = span.start;
                let mut end = span.end;
                if start >= 2 && &text[start - 2..start] == "\n\n" {
                    start -= 1;
                } else if end < text.len() && text.as_bytes()[end] == b'\n' {
                    end += 1;
                }
                vault.write_span(fi, crate::parse::Span { start, end }, "")?;
                break;
            }
        }
    }
    let file = block_ref.0;
    vault.trash_file(file)
}
