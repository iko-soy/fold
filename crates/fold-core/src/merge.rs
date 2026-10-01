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
    /// Conflict blocks raised against the file's own block root. Their embeds
    /// cannot go inside a block file (a second column-0 node, §4.9); they
    /// belong right after this file's embed in its parent file (§12.4).
    pub sibling_embeds: Vec<Id>,
}

/// Merge state shared across the recursion.
struct Ctx<'a> {
    conflicts: usize,
    conflict_blocks: Vec<(String, String)>,
    sibling_embeds: Vec<Id>,
    /// Nodes inserted from `T` only; with no conflicts and no insertions the
    /// merge is `O` itself, byte for byte (§15.6).
    insertions: usize,
    device: &'a str,
    timestamp: &'a str,
    /// `O`'s and `T`'s block-file frontmatter (block files only).
    o_fm: Option<crate::parse::Frontmatter>,
    t_fm: Option<crate::parse::Frontmatter>,
}

/// Merge two versions of one file. `device`/`timestamp` stamp the conflict
/// blocks (§12.4).
pub fn merge_texts(ours: &str, theirs: &str, device: &str, timestamp: &str) -> MergeOutcome {
    use crate::parse::parse_frontmatter;
    let o = standalone_tree(ours);
    let t = standalone_tree(theirs);
    // If both are block files, ids must match (§12.2); the caller handles
    // the prefix-collision case before calling us.
    let is_block = |fm: &Option<crate::parse::Frontmatter>| {
        fm.as_ref().map(|f| f.props.contains_key("id")).unwrap_or(false)
    };
    let o_fm = parse_frontmatter(ours);
    let t_fm = parse_frontmatter(theirs);
    let block_file = is_block(&o_fm) && is_block(&t_fm);
    let mut ctx = Ctx {
        conflicts: 0,
        conflict_blocks: Vec::new(),
        sibling_embeds: Vec::new(),
        insertions: 0,
        device,
        timestamp,
        o_fm: if block_file { parse_frontmatter(ours) } else { None },
        t_fm: if block_file { t_fm } else { None },
    };
    let merged = merge_children(&o, &t, o.root, t.root, &mut ctx, 1, 0);
    let text = if ctx.conflicts == 0 && ctx.insertions == 0 {
        // T adds nothing: the merge is O, formatting included
        ours.to_string()
    } else {
        // O's frontmatter stays verbatim; differing keys were raised as a
        // conflict on the block root (§12.4)
        match &o_fm {
            Some(fm) => {
                let mut head = ours[..fm.span.end].to_string();
                if !head.ends_with("\n\n") {
                    head.push('\n');
                }
                head + &merged
            }
            None => merged,
        }
    };
    MergeOutcome {
        text,
        conflict_blocks: ctx.conflict_blocks,
        conflicts: ctx.conflicts,
        sibling_embeds: ctx.sibling_embeds,
    }
}

/// Parse one version on its own, frontmatter included, so a block file's
/// frontmatter is never mistaken for body text (§12.4).
fn standalone_tree(text: &str) -> Tree {
    let fm = crate::parse::parse_frontmatter(text);
    let id = fm
        .as_ref()
        .and_then(|f| f.props.get("id"))
        .and_then(|v| Id::parse(v));
    let block = Block {
        id,
        path: "m.md".into(),
        props: fm.as_ref().map(|f| f.props.clone()).unwrap_or_default(),
        frontmatter_raw: fm.as_ref().map(|f| f.raw.clone()).unwrap_or_default(),
        frontmatter_span: fm.as_ref().map(|f| f.span),
    };
    let pf = parse_file("m.md", text, Some(block));
    Tree::new(vec![pf])
}

fn node_sig(t: &Tree, r: NRef) -> (String, Option<TaskState>, String) {
    let n = t.node(r);
    (n.title.clone(), n.task, text_sig(t, r))
}

/// A node's text children as one field (§12.4): every run's lines, with the
/// number of child nodes before it, blank separator lines ignored.
fn text_sig(t: &Tree, r: NRef) -> String {
    let n = t.node(r);
    let mut out = String::new();
    for (before, sp) in n.text_runs() {
        let lines = run_lines(t, r, sp);
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!("{}\u{1}{}\u{2}", before, lines.join("\n")));
    }
    out
}

/// A text run's lines, dedented from the node's content indent, without
/// leading or trailing blank lines.
fn run_lines(t: &Tree, r: NRef, sp: crate::parse::Span) -> Vec<String> {
    let n = t.node(r);
    let from = match n.kind {
        Kind::Item => n.indent + 2,
        Kind::Section => n.indent,
        Kind::Root => 0,
    };
    let mut lines: Vec<String> = sp
        .text(t.text_of(r))
        .lines()
        .map(|l| {
            let l = l.strip_suffix('\r').unwrap_or(l);
            if l.trim().is_empty() {
                String::new()
            } else {
                dedent(l, from).to_string()
            }
        })
        .collect();
    while lines.first().map(|l| l.is_empty()) == Some(true) {
        lines.remove(0);
    }
    while lines.last().map(|l| l.is_empty()) == Some(true) {
        lines.pop();
    }
    lines
}

/// A text run emitted at `indent`, ending in a newline ("" if it is blank).
fn emit_text(t: &Tree, r: NRef, sp: crate::parse::Span, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let mut out = String::new();
    for l in run_lines(t, r, sp) {
        if !l.is_empty() {
            out.push_str(&ind);
            out.push_str(&l);
        }
        out.push('\n');
    }
    out
}

/// One emitted child: a node's text or a text child.
enum Piece {
    Node(Kind, String),
    Text(String),
}

/// Join emitted children: a blank line before and after text and before a
/// section; items follow each other tightly.
fn join_pieces(pieces: Vec<Piece>) -> String {
    let mut out = String::new();
    let mut prev_node: Option<Option<Kind>> = None; // Some(None) = text
    for p in pieces {
        let (kind, text) = match p {
            Piece::Node(k, t) => (Some(k), t),
            Piece::Text(t) => (None, t),
        };
        if text.trim().is_empty() {
            continue;
        }
        let blank = match prev_node {
            None => false,
            Some(prev) => {
                kind.is_none()
                    || prev.is_none()
                    || kind == Some(Kind::Section)
                    || prev == Some(Kind::Section)
            }
        };
        if blank && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&text);
        prev_node = Some(kind);
    }
    out
}

/// Text runs of `r` after its child nodes: child position (0-based) → the
/// run that follows that child.
fn trailing_runs(t: &Tree, r: NRef) -> Vec<(usize, crate::parse::Span)> {
    t.node(r)
        .text_runs()
        .into_iter()
        .filter(|(before, _)| *before > 0)
        .map(|(before, sp)| (before - 1, sp))
        .collect()
}

/// Match children of two parents: by embed id, then exact title (§12.4).
/// Returns the merged children rendered at (`level`, `indent`).
#[allow(clippy::too_many_arguments)]
fn merge_children(
    o: &Tree,
    t: &Tree,
    op: NRef,
    tp: NRef,
    ctx: &mut Ctx,
    level: usize,
    indent: usize,
) -> String {
    let o_kids = o.resolved_children(op);
    let t_kids = t.resolved_children(tp);
    // match: embed by id, else by exact title among unmatched siblings;
    // `t_match[i]` is the O kid that T kid `i` matched
    let mut t_match: Vec<Option<usize>> = vec![None; t_kids.len()];
    let mut pairs: Vec<(NRef, Option<NRef>)> = Vec::new();
    // a block file's root is the same node on both sides, whatever its
    // title: match it by the file's id (§12.4 rule 1)
    let file_id = |tr: &Tree, r: NRef| tr.node(r).block.as_ref().and_then(|b| b.id.clone());
    let root_pair = match (o_kids.first(), t_kids.first()) {
        (Some(&ok), Some(&tk)) if file_id(o, ok).is_some() && file_id(o, ok) == file_id(t, tk) => {
            Some((ok, tk))
        }
        _ => None,
    };
    for (j, &ok) in o_kids.iter().enumerate() {
        let on = o.node(ok);
        if let Some((rok, rtk)) = root_pair {
            if ok == rok {
                t_match[0] = Some(j);
                pairs.push((ok, Some(rtk)));
                continue;
            }
        }
        let mut found = None;
        for (i, &tk) in t_kids.iter().enumerate() {
            if t_match[i].is_some() {
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
            t_match[i] = Some(j);
        }
        pairs.push((ok, found.map(|i| t_kids[i])));
    }
    // T-only nodes are insertions, placed relative to their matched
    // neighbours (§12.4): after the O partner of the nearest matched node
    // before them in T — and after O's text there too, when T has text
    // between the two — or first, when no matched node precedes them
    let t_trailing = trailing_runs(t, tp);
    let mut first: Vec<NRef> = Vec::new();
    let mut after_node: Vec<Vec<NRef>> = vec![Vec::new(); o_kids.len()];
    let mut after_text: Vec<Vec<NRef>> = vec![Vec::new(); o_kids.len()];
    let mut anchor: Option<(usize, usize)> = None; // (T kid, its O kid)
    for (i, &tk) in t_kids.iter().enumerate() {
        match (t_match[i], anchor) {
            (Some(j), _) => anchor = Some((i, j)),
            (None, None) => first.push(tk),
            (None, Some((ti, j))) => {
                let text_between = t_trailing
                    .iter()
                    .any(|&(pos, sp)| (ti..i).contains(&pos) && !run_lines(t, tp, sp).is_empty());
                if text_between {
                    after_text[j].push(tk);
                } else {
                    after_node[j].push(tk);
                }
            }
        }
    }
    ctx.insertions += t_match.iter().filter(|m| m.is_none()).count();
    let inserted = |&b: &NRef| Piece::Node(t.node(b).kind, emit_subtree(t, b, level, indent));
    // Emit: matched pairs merge field-by-field; single-sided nodes are
    // insertions; differing fields raise conflict pairs (§12.4). O's text
    // children stay where O has them.
    let mut pieces: Vec<Piece> = Vec::new();
    if o.node(op).kind == Kind::Root {
        // the root's own text: O's, plus T's when it differs (never lost)
        if let Some(sp) = o.node(op).body_span() {
            pieces.push(Piece::Text(emit_text(o, op, sp, indent)));
        }
        if text_sig(o, op) != text_sig(t, tp) {
            ctx.insertions += 1;
            for (_, sp) in t.node(tp).text_runs() {
                pieces.push(Piece::Text(emit_text(t, tp, sp, indent)));
            }
        }
    }
    pieces.extend(first.iter().map(inserted));
    let o_trailing = trailing_runs(o, op);
    for (j, (a, tk)) in pairs.into_iter().enumerate() {
        let text = match tk {
            Some(b) => merge_pair(o, t, a, b, ctx, level, indent),
            None => emit_subtree(o, a, level, indent),
        };
        pieces.push(Piece::Node(o.node(a).kind, text));
        pieces.extend(after_node[j].iter().map(inserted));
        for (_, sp) in o_trailing.iter().filter(|(pos, _)| *pos == j) {
            pieces.push(Piece::Text(emit_text(o, op, *sp, indent)));
        }
        pieces.extend(after_text[j].iter().map(inserted));
    }
    // clamped by the ordering rule (§3.1): an inserted item or text goes no
    // later than just before the first section, an inserted section no
    // earlier than just after the last item; O's own order already obeys it
    let (mut pieces, sections): (Vec<Piece>, Vec<Piece>) = pieces
        .into_iter()
        .partition(|p| !matches!(p, Piece::Node(Kind::Section, _)));
    pieces.extend(sections);
    join_pieces(pieces)
}

fn merge_pair(
    o: &Tree,
    t: &Tree,
    a: NRef,
    b: NRef,
    ctx: &mut Ctx,
    level: usize,
    indent: usize,
) -> String {
    let on = o.node(a);
    let tn = t.node(b);
    // embeds matched by id: identical reference, emit ours
    if on.embed.is_some() {
        return emit_subtree(o, a, level, indent);
    }
    // the file's own block root: its frontmatter keys are fields too (§12.4)
    let is_file_block = on.block.as_ref().map(|bl| bl.id.is_some()).unwrap_or(false);
    let (otitle, otask, obody) = node_sig(o, a);
    let (_ttitle, ttask, tbody) = node_sig(t, b);
    let mut differ = otitle != tn.title || otask != ttask || obody != tbody;
    if is_file_block {
        if let (Some(ofm), Some(tfm)) = (&ctx.o_fm, &ctx.t_fm) {
            differ |= ofm.props != tfm.props;
        }
    }
    // children merge
    let child_level = match on.kind {
        Kind::Section => level + 1,
        _ => level,
    };
    let child_indent = match on.kind {
        Kind::Item => indent + 2,
        _ => indent,
    };
    let merged_kids = merge_children(o, t, a, b, ctx, child_level, child_indent);
    if !differ {
        return emit_node_with(o, a, level, indent, &merged_kids);
    }
    ctx.conflicts += 1;
    // theirs becomes a conflict block stitched in next to ours (§12.4)
    let name = slug(&otitle);
    let id = Id::generate();
    let prefix = id.words()[0].to_string();
    let fname = filename(&prefix, &name);
    let mut block_text = format!(
        "---\nid: {}\nconflict: \"{} {}\"\n",
        id, ctx.device, ctx.timestamp
    );
    let t_props = if is_file_block {
        ctx.t_fm.as_ref().map(|f| f.props.clone())
    } else {
        None
    };
    match t_props {
        // a block conflict carries T's own properties (§12.4)
        Some(props) => {
            for (k, v) in props.iter().filter(|(k, _)| *k != "id" && *k != "conflict") {
                block_text.push_str(&format!("{}: {}\n", k, v));
            }
        }
        // T's task state travels as the checkbox on its title line (§4.5)
        None => {}
    }
    block_text.push_str("---\n\n");
    block_text.push_str(&emit_subtree(t, b, 1, 0));
    ctx.conflict_blocks.push((fname, block_text));
    if is_file_block {
        // a block file has one column-0 node: the embed goes after this
        // file's own embed in the parent file (the caller places it)
        ctx.sibling_embeds.push(id);
        return emit_node_with(o, a, level, indent, &merged_kids);
    }
    // the embed goes right after ours, as its next sibling, in the form of
    // ours' spelling (§4.7): a heading embed after a section's subtree
    let mut out = emit_node_with(o, a, level, indent, &merged_kids);
    if on.kind == Kind::Section {
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&format!("{}{} ![[{}]]\n", " ".repeat(indent), "#".repeat(level.max(1)), id));
    } else {
        out.push_str(&format!("{}![[{}]]\n", " ".repeat(indent), id));
    }
    out
}

/// Render a node at (level, indent) with given already-rendered children.
fn emit_node_with(o: &Tree, a: NRef, level: usize, indent: usize, kids: &str) -> String {
    let n = o.node(a);
    let mut out = String::new();
    let ind = " ".repeat(indent);
    match n.kind {
        // embeds stay unresolved here (§12.4): the line is the reference
        // itself, in its own form (§4.7), never an empty title
        Kind::Section | Kind::Item if n.embed.is_some() => {
            out.push_str(&crate::render::embed_line(n, level, indent));
            out.push('\n');
        }
        Kind::Section => {
            out.push_str(&ind);
            out.push_str(&"#".repeat(level.max(1)));
            out.push(' ');
            push_task(n.task, &mut out);
            out.push_str(&n.title);
            out.push('\n');
        }
        Kind::Item => {
            out.push_str(&ind);
            out.push_str("- ");
            push_task(n.task, &mut out);
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
    let kids = t.resolved_children(r);
    let child_level = match t.node(r).kind {
        Kind::Section => level + 1,
        _ => level,
    };
    let child_indent = match t.node(r).kind {
        Kind::Item => indent + 2,
        _ => indent,
    };
    // children in order: nodes and the text runs after them (the leading
    // run is the body, emitted with the title)
    let trailing = trailing_runs(t, r);
    let mut pieces = Vec::new();
    for (j, &k) in kids.iter().enumerate() {
        pieces.push(Piece::Node(
            t.node(k).kind,
            emit_subtree(t, k, child_level, child_indent),
        ));
        for (_, sp) in trailing.iter().filter(|(pos, _)| *pos == j) {
            pieces.push(Piece::Text(emit_text(t, r, *sp, child_indent)));
        }
    }
    emit_node_with(t, r, level, indent, &join_pieces(pieces))
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

// ------------------------------------------------------------ vault-level

/// Process every `*.sync-conflict-*.md` in the vault (§12.2, §13 `notes merge`).
/// Returns a list of human-readable outcomes.
pub fn merge_sync_conflicts(vault: &mut Vault, dry_run: bool) -> std::io::Result<Vec<String>> {
    merge_sync_conflicts_moving(vault, dry_run, &[])
}

/// `merge_sync_conflicts` while the blocks `moving` are moved on this
/// device: cut in the editor and not pasted back, in transit and embedded
/// nowhere (§5.2). A copy made before the cut still embeds one where it
/// was, and the merge never deletes (§12.4): it would put the block back
/// there, and the paste would embed it a second time. Each copy is read
/// without their embeds, as it would be had it been made after the cut.
pub fn merge_sync_conflicts_moving(
    vault: &mut Vault,
    dry_run: bool,
    moving: &[Id],
) -> std::io::Result<Vec<String>> {
    let mut outcomes = Vec::new();
    // the files as they are, not as last read: a block that came in with
    // its copy is ours to merge into, not an ignored file (§12.2)
    vault.reload()?;
    for cfile in vault.conflict_files()? {
        let base = cfile.split(".sync-conflict-").next().unwrap().to_string() + ".md";
        let cpath = vault.dir.join(&cfile);
        let bpath = vault.dir.join(&base);
        // only root.md and block files are ours to merge: the conflict copy
        // of an ignored file is left alone, like the file (§4.1, §11.4)
        let parsed = base == "root.md" || vault.file_index(&base).is_some();
        if !parsed && bpath.exists() {
            outcomes.push(format!("{}: {} is not a vault file, left alone", cfile, base));
            continue;
        }
        let theirs = std::fs::read_to_string(&cpath)?;
        let tid = crate::parse::parse_frontmatter(&theirs)
            .and_then(|f| f.props.get("id").cloned())
            .and_then(|v| Id::parse(&v));
        // X.md gone: a block renamed since is still found by its id (§6.4)
        let bpath = match tid.as_ref().and_then(|id| vault.tree.block_by_id(id)) {
            Some(r) if !bpath.exists() => {
                vault.dir.join(&vault.tree.node(r).block.as_ref().unwrap().path)
            }
            _ => bpath,
        };
        let ours = match std::fs::read_to_string(&bpath) {
            Ok(text) => text,
            // nothing to merge against: T wins as it is, frontmatter and
            // all, rather than being merged into an empty O that has none
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && (parsed || tid.is_some()) => {
                outcomes.push(format!("{}: {} missing, keeping theirs", cfile, base));
                if !dry_run {
                    std::fs::rename(&cpath, &bpath)?;
                }
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                outcomes.push(format!("{}: not a block file, left alone", cfile));
                continue;
            }
            // an O that cannot be read is never overwritten
            Err(e) => {
                outcomes.push(format!("{}: cannot read {}: {}, left alone", cfile, base, e));
                continue;
            }
        };
        // device + timestamp from the filename
        let (device, stamp) = parse_conflict_name(&cfile);
        // id check: differing ids mean a prefix collision, not a conflict (§12.2)
        let oid = crate::parse::parse_frontmatter(&ours)
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
        let outcome = merge_texts(&ours, &without_embeds(&theirs, moving), &device, &stamp);
        outcomes.push(format!(
            "{}: {} conflict pair(s)",
            cfile, outcome.conflicts
        ));
        if !dry_run {
            // the trash first, so a trash that cannot be made stops the
            // merge before anything is written
            let trash = crate::vault::trash_dir();
            std::fs::create_dir_all(&trash)?;
            crate::vault::atomic_write(&bpath, &outcome.text)?;
            for (fname, text) in &outcome.conflict_blocks {
                let fname = fresh_block_name(vault, fname, text);
                crate::vault::atomic_write(&vault.dir.join(&fname), text)?;
            }
            if !outcome.sibling_embeds.is_empty() {
                vault.reload()?;
                place_sibling_embeds(vault, oid.as_ref(), &outcome.sibling_embeds)?;
            }
            // the conflict file goes to trash (§12.4), which is often on
            // another filesystem: a rename alone would fail there and leave
            // it to be merged again
            let stamp2 = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
            crate::vault::move_file(&cpath, &trash.join(format!("{}-{}", stamp2, cfile.to_lowercase())))?;
        }
    }
    if !dry_run {
        vault.reload()?;
    }
    Ok(outcomes)
}

/// `text` without its embed lines of the blocks `ids` (§4.7), each with
/// the blank line after it where one is before it too, so the text around
/// it reads as it does where the embed was written out.
fn without_embeds(text: &str, ids: &[Id]) -> String {
    if ids.is_empty() {
        return text.to_string();
    }
    let t = standalone_tree(text);
    let mut lines: Vec<usize> = t.files[0]
        .nodes
        .iter()
        .filter(|n| n.embed.as_ref().is_some_and(|id| ids.contains(id)))
        .map(|n| n.title_span.start)
        .collect();
    lines.sort_unstable();
    let line_end = |at: usize| text[at..].find('\n').map_or(text.len(), |i| at + i + 1);
    // the line before `at` is blank, or there is none
    let blank_before = |at: usize| {
        let before = text[..at].strip_suffix('\n');
        before.is_none_or(|s| s.rsplit('\n').next().is_some_and(|l| l.trim().is_empty()))
    };
    let mut out = String::with_capacity(text.len());
    let mut kept = 0;
    for start in lines {
        let mut end = line_end(start);
        let after = &text[end..line_end(end)];
        if blank_before(start) && !after.is_empty() && after.trim().is_empty() {
            end += after.len();
        }
        out.push_str(&text[kept..start]);
        kept = end;
    }
    out.push_str(&text[kept..]);
    out
}

/// A conflict block's filename that does not clobber an existing file:
/// lengthen the prefix of its id until the name is free (§6.4).
fn fresh_block_name(vault: &Vault, fname: &str, text: &str) -> String {
    let id = crate::parse::parse_frontmatter(text)
        .and_then(|f| f.props.get("id").cloned())
        .and_then(|v| Id::parse(&v));
    let (Some(id), Some((_, name))) = (id, crate::ident::split_filename(fname)) else {
        return fname.to_string();
    };
    let words = id.words();
    for n in 1..=words.len() {
        let cand = filename(&words[..n].join("-"), name);
        if !vault.dir.join(&cand).exists() {
            return cand;
        }
    }
    fname.to_string()
}

/// Insert embeds right after the embed of block `owner` in its parent file,
/// at its indent, so each conflict block is the next sibling of the block it
/// conflicts with (§12.4). Without an embed to follow (an orphan block),
/// the owner is embedded at the end of `root.md` with them right after it:
/// placed alone, they would pair with whatever node happened to be last.
fn place_sibling_embeds(vault: &mut Vault, owner: Option<&Id>, ids: &[Id]) -> std::io::Result<()> {
    // the embed the owner is stitched in at: a second one reads as broken
    // (§6.2) and pairs with nothing
    let found = owner.and_then(|oid| vault.tree.embed_of(oid)).map(|(fi, ni)| {
        let f = &vault.tree.files[fi];
        let nd = &f.nodes[ni];
        // the same form as the owner's embed (§4.7)
        let heading = (nd.kind == Kind::Section).then(|| nd.level.unwrap_or(1));
        (fi, nd.title_span.end.min(f.text.len()), nd.indent, heading)
    });
    let mut insert = String::new();
    match found {
        Some((fi, end, indent, heading)) => {
            for id in ids {
                match heading {
                    Some(h) => insert.push_str(&format!(
                        "\n\n{}{} ![[{}]]",
                        " ".repeat(indent),
                        "#".repeat(h),
                        id
                    )),
                    None => insert.push_str(&format!("\n{}![[{}]]", " ".repeat(indent), id)),
                }
            }
            // right after the embed's own line, before its blank separator
            vault.write_span(fi, crate::parse::Span { start: end, end }, &insert)
        }
        None => {
            // in the form of the owner's spelling (§4.7); a heading embed at
            // the end of root.md is top-level
            let heading = owner
                .and_then(|oid| vault.tree.block_by_id(oid))
                .is_some_and(|r| vault.tree.node(r).kind == Kind::Section);
            let lines: Vec<String> = owner
                .into_iter()
                .chain(ids)
                .map(|id| format!("{}![[{}]]\n", if heading { "# " } else { "" }, id))
                .collect();
            let mut new_text = vault.tree.files[0].text.clone();
            if !new_text.is_empty() && !new_text.ends_with("\n\n") {
                new_text.push_str(if new_text.ends_with('\n') { "\n" } else { "\n\n" });
            }
            // sibling sections a blank line apart, items tight (§4.2)
            new_text.push_str(&lines.join(if heading { "\n" } else { "" }));
            vault.write_file_text(0, &new_text)
        }
    }
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

/// Find unresolved conflict blocks: blocks with a `conflict:` key (§10.7).
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
                                // the conflict block is ours' next sibling (§12.4)
                                if let Some(ours) = pos.checked_sub(1).map(|i| sibs[i]) {
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
    let theirs_body = crate::render::render(&vault.tree, theirs, 1, true);
    // theirs' task state is the checkbox on its title line, already in the
    // rendered text (§4.5)
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
        // a title change does not rename the file (§6.4)
    } else {
        // plain node: replace its span with theirs' re-levelled text
        let parent_level = on.parent.map(|p| vault.tree.level((ours.0, p))).unwrap_or(0);
        let indent = vault.tree.indent(ours);
        let shifted = crate::ops::shift_document(&theirs_body, parent_level, indent);
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
                if start >= 2 && &text.as_bytes()[start - 2..start] == b"\n\n" {
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
