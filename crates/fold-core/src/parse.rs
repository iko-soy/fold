//! Line-oriented, fence-aware parser (§4, §15.3) and the tree it builds.

use crate::ident::Id;
use indexmap::IndexMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Root,
    Section,
    Item,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Open,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn text<'a>(&self, file: &'a str) -> &'a str {
        &file[self.start..self.end]
    }
}

/// A block: a node with an id and a file (§2).
#[derive(Debug, Clone)]
pub struct Block {
    pub id: Option<Id>, // None only for root.md
    pub path: String,   // relative to the vault, e.g. "racfer~order-new-switch.md"
    /// Top-level `key: value` lines, order-preserving; includes `id`.
    pub props: IndexMap<String, String>,
    /// The frontmatter body (between the `---` fences), kept verbatim for
    /// lossless rewrite. Empty means the file has no frontmatter.
    pub frontmatter_raw: String,
    /// Byte span of the whole frontmatter (fences plus one trailing blank
    /// line), if present.
    pub frontmatter_span: Option<Span>,
}

impl Block {
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props.get(key).map(String::as_str)
    }
}

/// One entry in a node's ordered content (§3.1): a run of text lines or a
/// child node. Text runs are never adjacent to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    Text(Span),
    Node(usize),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    pub title: String,
    /// From the checkbox on the title line (§4.5).
    pub task: Option<TaskState>,
    /// Byte span of the title line (no trailing newline).
    pub title_span: Span,
    /// The node's children in document order: text runs and child nodes,
    /// interleaved (§3.1, §3.3).
    pub content: Vec<Content>,
    /// Byte span: title line through the whole subtree.
    pub span: Span,
    /// The node children alone, in order (derived from `content`).
    pub children: Vec<usize>,
    pub parent: Option<usize>,
    /// Which file's text this node's spans index into.
    pub file: usize,
    /// `Some` if this node is the root of its own file.
    pub block: Option<Block>,
    /// `Some(id)` if this node is an embed reference in a parent file.
    pub embed: Option<Id>,
    /// Indentation in columns of this node's title line.
    pub indent: usize,
    /// Heading level as written (sections only).
    pub level: Option<usize>,
    /// Non-canonical observations made while reading (§15.3).
    pub noncanonical: Vec<String>,
}

impl Node {
    /// The leading text children: what precedes the first child node.
    pub fn body_span(&self) -> Option<Span> {
        match self.content.first() {
            Some(Content::Text(sp)) => Some(*sp),
            _ => None,
        }
    }

    /// Every text child, with the number of child nodes before it.
    pub fn text_runs(&self) -> Vec<(usize, Span)> {
        let mut out = Vec::new();
        let mut before = 0;
        for c in &self.content {
            match c {
                Content::Text(sp) => out.push((before, *sp)),
                Content::Node(_) => before += 1,
            }
        }
        out
    }
}

/// One parsed file: its text and the arena of nodes found in it.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub path: String,
    pub text: String,
    pub nodes: Vec<Node>,
    pub root_node: usize,
    /// Diagnostics that apply to the file as a whole.
    pub diagnostics: Vec<Diag>,
}

impl ParsedFile {
    /// The block-root node: the single top-level node of a block file, or the
    /// implicit Root for `root.md`.
    pub fn block_root(&self) -> usize {
        if self.nodes[self.root_node].block.is_some() {
            self.root_node
        } else {
            self.nodes[self.root_node].children[0]
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub span: Span,
    pub message: String,
}

#[derive(Clone)]
struct Line {
    start: usize,
    end: usize,  // excluding the line ending (`\n` or `\r\n`)
    next: usize, // start of the next line (past the line ending, or EOF)
    raw: String,
}

fn split_lines(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut off = 0usize;
    for l in text.split_inclusive('\n') {
        let raw = l.strip_suffix('\n').unwrap_or(l);
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        lines.push(Line {
            start: off,
            end: off + raw.len(),
            next: off + l.len(),
            raw: raw.to_string(),
        });
        off += l.len();
    }
    lines
}

/// Frontmatter scan result (§4.4).
pub struct Frontmatter {
    pub raw: String,
    pub span: Span,
    pub props: IndexMap<String, String>,
}

/// Scan frontmatter at byte 0 of a file. Returns None if absent.
pub fn parse_frontmatter(text: &str) -> Option<Frontmatter> {
    if !text.starts_with("---\n") && !text.starts_with("---\r\n") {
        return None;
    }
    let lines = split_lines(text);
    let mut close = None;
    for (i, l) in lines.iter().enumerate().skip(1) {
        if l.raw.trim_end() == "---" {
            close = Some(i);
            break;
        }
    }
    let close = close?;
    let raw_start = lines[1].start;
    let raw_end = lines[close].start;
    let raw = text[raw_start..raw_end].to_string();
    let mut span_end = lines[close].next;
    if close + 1 < lines.len() && lines[close + 1].raw.is_empty() {
        span_end = lines[close + 1].next;
    }
    let mut props = IndexMap::new();
    for l in &lines[1..close] {
        let raw_line = &l.raw;
        if raw_line.starts_with(|c: char| c.is_whitespace()) || raw_line.is_empty() {
            continue;
        }
        if let Some((k, v)) = raw_line.split_once(':') {
            let key = k.trim_end();
            if is_valid_key(key) {
                props.insert(key.to_string(), v.trim().to_string());
            }
        }
    }
    Some(Frontmatter {
        raw,
        span: Span { start: 0, end: span_end },
        props,
    })
}

pub fn is_valid_key(k: &str) -> bool {
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn indent_cols(raw: &str, tabbed: &mut bool) -> usize {
    let mut cols = 0usize;
    for ch in raw.chars() {
        match ch {
            ' ' => cols += 1,
            '\t' => {
                cols += 4;
                *tabbed = true;
            }
            _ => break,
        }
    }
    cols
}

enum TitleKind {
    Section { level: usize },
    Item,
    /// `![[id]]`, bare (item position) or as a heading (`## ![[id]]`,
    /// section position, with its level) (§4.7).
    Embed(Id, Option<usize>),
}

struct TitleInfo {
    kind: TitleKind,
    task: Option<TaskState>,
    title: String,
    noncanonical: Vec<String>,
}

fn classify_title(raw: &str) -> Option<(usize, TitleInfo)> {
    let mut tabbed = false;
    let indent = indent_cols(raw, &mut tabbed);
    let rest = raw.trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return None;
    }
    let mut noncanonical = Vec::new();
    if tabbed {
        noncanonical.push("tab indent".into());
    }
    // embed: ![[id]] alone on the line
    if let Some(inner) = rest
        .strip_prefix("![[")
        .and_then(|s| s.strip_suffix("]]"))
    {
        return Id::parse(inner.trim()).map(|id| {
            (
                indent,
                TitleInfo {
                    kind: TitleKind::Embed(id, None),
                    task: None,
                    title: String::new(),
                    noncanonical,
                },
            )
        });
    }
    let (kind, after_marker) = if rest.starts_with('#') {
        // any number: levels are unbounded, beyond six too (§3.1, §4.2)
        let hashes = rest.chars().take_while(|&c| c == '#').count();
        let after = &rest[hashes..];
        if !after.is_empty() && !after.starts_with(' ') {
            return None; // `#tag` is text
        }
        // a heading embed: the heading holds nothing but `![[id]]`
        if let Some(inner) = after
            .trim()
            .strip_prefix("![[")
            .and_then(|s| s.strip_suffix("]]"))
        {
            if let Some(id) = Id::parse(inner.trim()) {
                return Some((
                    indent,
                    TitleInfo {
                        kind: TitleKind::Embed(id, Some(hashes)),
                        task: None,
                        title: String::new(),
                        noncanonical,
                    },
                ));
            }
        }
        (
            TitleKind::Section { level: hashes },
            after.strip_prefix(' ').unwrap_or(after),
        )
    } else if is_thematic_break(rest) {
        return None; // `- - -`, `* * *`: a break, not a bullet (§4.4)
    } else if let Some(after) = rest.strip_prefix("- ") {
        (TitleKind::Item, after)
    } else if let Some(after) = rest.strip_prefix("* ").or_else(|| rest.strip_prefix("+ ")) {
        noncanonical.push("non-canonical bullet marker".into());
        (TitleKind::Item, after)
    } else if rest == "-" || rest == "*" || rest == "+" {
        if rest != "-" {
            noncanonical.push("non-canonical bullet marker".into());
        }
        (TitleKind::Item, "")
    } else {
        return None;
    };
    let mut task = None;
    let mut title_text = after_marker;
    let b = after_marker.as_bytes();
    if b.len() >= 3 && b[0] == b'[' && b[2] == b']' {
        let state = match b[1] {
            b' ' => Some(TaskState::Open),
            b'x' => Some(TaskState::Done),
            b'X' => {
                noncanonical.push("uppercase [X] checkbox".into());
                Some(TaskState::Done)
            }
            b'-' => {
                noncanonical.push("[-] read as done".into());
                Some(TaskState::Done)
            }
            _ => None,
        };
        if let Some(st) = state {
            let rt = &after_marker[3..];
            if rt.is_empty() {
                task = Some(st);
                title_text = "";
            } else if let Some(t) = rt.strip_prefix(' ') {
                task = Some(st);
                title_text = t;
            }
        }
    }
    Some((
        indent,
        TitleInfo {
            kind,
            task,
            title: title_text.to_string(),
            noncanonical,
        },
    ))
}

/// Track fenced code blocks (§3.3) one line at a time: true when `raw` opens
/// or closes one. Fences are CommonMark's: a backtick fence's info string
/// holds no backtick (a line that starts with inline code is text), and a
/// closing fence is followed by nothing but spaces or tabs. Public so that
/// what the TUI draws as code is what the parser reads as code.
pub fn fence_transition(raw: &str, open: &mut Option<(char, usize)>) -> bool {
    let t = raw.trim_start_matches([' ', '\t']);
    let first = match t.chars().next() {
        Some(c) if c == '`' || c == '~' => c,
        _ => return false,
    };
    let count = t.chars().take_while(|&c| c == first).count();
    if count < 3 {
        return false;
    }
    let rest = &t[count..]; // '`' and '~' are one byte each
    match open {
        None => {
            if first == '`' && rest.contains('`') {
                return false;
            }
            *open = Some((first, count));
            true
        }
        Some((c, n)) => {
            if *c == first && count >= *n && rest.trim_matches([' ', '\t']).is_empty() {
                *open = None;
                true
            } else {
                false
            }
        }
    }
}

/// A thematic break, however it is spaced: three or more of one of `-`, `*`,
/// `_` with nothing but spaces or tabs between. CommonMark reads it before a
/// list item, so `- - -` and `* * *` are body text like `---` (§4.4).
/// `rest` starts at the line's first non-blank character.
fn is_thematic_break(rest: &str) -> bool {
    let Some(mark) = rest.chars().next().filter(|c| matches!(c, '-' | '*' | '_')) else {
        return false;
    };
    let mut n = 0;
    for c in rest.chars() {
        if c == mark {
            n += 1;
        } else if c != ' ' && c != '\t' {
            return false;
        }
    }
    n >= 3
}

/// A setext underline. Only `=` underlines count: a line of dashes is a
/// thematic break and body text (§4.4, §5.2 step 2), and a lone `-` is an
/// empty bullet.
fn setext_level(raw: &str) -> Option<usize> {
    let t = raw.trim();
    if !t.is_empty() && t.chars().all(|c| c == '=') {
        Some(1)
    } else {
        None
    }
}

struct Frame {
    node: usize,
    indent: usize,
    level: Option<usize>,
    is_embed: bool,
}

/// Parse one file's text into nodes. `file_idx` tags every node's `file`.
/// `block` (with `id: Some`) marks a block file; `id: None` is `root.md`;
/// `None` means "plain text, no block attachment" (used by tests).
pub fn parse_file(path: &str, text: &str, file_idx: usize, block: Option<Block>) -> ParsedFile {
    let lines = split_lines(text);
    let n = lines.len();
    let mut nodes: Vec<Node> = Vec::new();
    let mut diagnostics = Vec::new();

    let root_idx = 0;
    nodes.push(Node {
        kind: Kind::Root,
        title: String::new(),
        task: None,
        title_span: Span::default(),
        content: Vec::new(),
        span: Span {
            start: 0,
            end: text.len(),
        },
        children: Vec::new(),
        parent: None,
        file: file_idx,
        block: None,
        embed: None,
        indent: 0,
        level: Some(0),
        noncanonical: Vec::new(),
    });

    let is_block_file = block.as_ref().and_then(|b| b.id.as_ref()).is_some();

    // Structural parse starts after frontmatter.
    let start_line = match block.as_ref().and_then(|b| b.frontmatter_span) {
        Some(fs) => lines.iter().position(|l| l.start >= fs.end).unwrap_or(n),
        None => 0,
    };

    let mut stack: Vec<Frame> = vec![Frame {
        node: root_idx,
        indent: 0,
        level: Some(0),
        is_embed: false,
    }];
    let mut fence: Option<(char, usize)> = None;
    let mut root_title_seen = !is_block_file;
    let mut root_level_nodes = 0usize;
    // A block file's root node, once seen: later column-0 nodes and text
    // are adopted as its children (§4.9) — what a phone appending to the
    // file produces.
    let mut block_root: Option<usize> = None;

    // Pending setext: (title line index, title line indent) — a plain body
    // line that may become a section if the next line is an underline.
    let mut pending_setext: Option<(usize, usize)> = None;

    let mut i = start_line;
    while i < n {
        let raw = lines[i].raw.clone();
        let lstart = lines[i].start;
        let lend = lines[i].end;
        let lnext = lines[i].next;

        if fence_transition(&raw, &mut fence) {
            pending_setext = None;
            push_body(&mut nodes, &mut stack, &lines, i, block_root);
            i += 1;
            continue;
        }
        if fence.is_some() {
            push_body(&mut nodes, &mut stack, &lines, i, block_root);
            i += 1;
            continue;
        }
        // Setext underline?
        if let Some(level) = setext_level(&raw) {
            if let Some((title_li, title_indent)) = pending_setext.take() {
                let idx = make_setext_section(
                    &mut nodes,
                    &mut stack,
                    &lines,
                    title_li,
                    title_indent,
                    i,
                    level,
                    block_root,
                );
                // like any title line, a setext heading can be a block
                // file's root (§4.9); its title line was then reported as
                // text before the root, which it is not
                if nodes[idx].parent == Some(root_idx) {
                    root_level_nodes += 1;
                    if is_block_file && !root_title_seen {
                        root_title_seen = true;
                        block_root = Some(idx);
                        let at = nodes[idx].title_span.start;
                        diagnostics.retain(|d: &Diag| d.span.start != at);
                    }
                }
                i += 1;
                continue;
            }
            // Not a setext: a `---` after a blank or structure is a break (body).
            push_body(&mut nodes, &mut stack, &lines, i, block_root);
            i += 1;
            continue;
        }
        match classify_title(&raw) {
            Some((indent, info)) => {
                pending_setext = None;
                let is_embed = matches!(info.kind, TitleKind::Embed(..));
                // Pop until the top can contain this node (§3.1). A node's
                // parent is the nearest section above it, or the nearest item
                // whose child indent it reaches — items that can't contain it
                // are looked through rather than made parents.
                loop {
                    let top = stack.last().unwrap();
                    let tnode = &nodes[top.node];
                    let can = match tnode.kind {
                        Kind::Root => true,
                        // lines under a bare embed are a diagnostic, but keep
                        // spans sane; a heading embed nests like a section
                        Kind::Item if top.is_embed => indent > top.indent,
                        Kind::Item => {
                            // An item contains only what reaches its child
                            // indent; otherwise look through it.
                            if indent <= top.indent {
                                stack.pop();
                                continue;
                            }
                            true
                        }
                        Kind::Section if indent < top.indent => {
                            // a section nested under an item holds only
                            // what reaches its indent
                            stack.pop();
                            continue;
                        }
                        Kind::Section => {
                            // A section is contained by the nearest section
                            // with a smaller level — not by its lexical
                            // predecessor, so `## C` after `###### B` is a
                            // sibling of B, not a child. Look through
                            // intervening items.
                            if let TitleKind::Section { level } | TitleKind::Embed(_, Some(level)) =
                                &info.kind
                            {
                                let mut nearest = None;
                                for f in stack.iter().rev() {
                                    if nodes[f.node].kind == Kind::Section {
                                        nearest = f.level;
                                        break;
                                    }
                                }
                                *level > nearest.unwrap_or(0)
                            } else {
                                true
                            }
                        },
                    };
                    if can {
                        break;
                    }
                    stack.pop();
                }
                let mut parent = stack.last().unwrap().node;
                let mut adopted = false;
                if parent == root_idx {
                    if let Some(br) = block_root {
                        parent = br;
                        adopted = true;
                    }
                }
                let mut node = Node {
                    kind: match info.kind {
                        TitleKind::Section { .. } | TitleKind::Embed(_, Some(_)) => Kind::Section,
                        TitleKind::Item | TitleKind::Embed(_, None) => Kind::Item,
                    },
                    title: info.title,
                    task: info.task,
                    title_span: Span {
                        start: lstart,
                        end: lend,
                    },
                    content: Vec::new(),
                    span: Span {
                        start: lstart,
                        end: lnext,
                    },
                    children: Vec::new(),
                    parent: Some(parent),
                    file: file_idx,
                    block: None,
                    embed: match &info.kind {
                        TitleKind::Embed(id, _) => Some(id.clone()),
                        _ => None,
                    },
                    indent,
                    level: match &info.kind {
                        TitleKind::Section { level } | TitleKind::Embed(_, Some(level)) => {
                            Some(*level)
                        }
                        _ => None,
                    },
                    noncanonical: info.noncanonical,
                };
                if adopted {
                    node.noncanonical
                        .push("column-0 node in a block file, adopted by its root".into());
                }
                let first_root = parent == root_idx && is_block_file && !root_title_seen
                    && node.embed.is_none();
                if parent == root_idx {
                    root_level_nodes += 1;
                    if is_block_file {
                        if first_root {
                            root_title_seen = true;
                        } else if root_level_nodes > 1 {
                            diagnostics.push(Diag {
                                span: node.title_span,
                                message: "block file has more than one node at column 0".into(),
                            });
                        }
                    }
                }
                if node.title.is_empty() && node.embed.is_none() {
                    node.noncanonical.push("empty title".into());
                    diagnostics.push(Diag {
                        span: node.title_span,
                        message: "title line with an empty title".into(),
                    });
                }
                if node.title.contains('/') {
                    diagnostics.push(Diag {
                        span: node.title_span,
                        message: "title contains '/'".into(),
                    });
                }
                let idx = nodes.len();
                if first_root {
                    block_root = Some(idx);
                }
                nodes[parent].children.push(idx);
                nodes[parent].content.push(Content::Node(idx));
                nodes.push(node);
                extend_spans(&mut nodes, idx, lnext);
                stack.push(Frame {
                    node: idx,
                    indent,
                    level: nodes[idx].level,
                    is_embed,
                });
                i += 1;
            }
            None => {
                // Candidate setext title: a single non-blank plain line that
                // belongs to the current node (its indent reaches the node).
                if !raw.trim().is_empty() {
                    pending_setext = Some((i, indent_cols(&raw, &mut false)));
                } else {
                    pending_setext = None;
                }
                if is_block_file && !root_title_seen && !raw.trim().is_empty() {
                    diagnostics.push(Diag {
                        span: Span {
                            start: lstart,
                            end: lend,
                        },
                        message: "text before the block's root node".into(),
                    });
                }
                push_body(&mut nodes, &mut stack, &lines, i, block_root);
                i += 1;
            }
        }
    }

    // A text child right after a child node, with no blank line between,
    // reads here as the parent's text but in CommonMark as a continuation of
    // that node: canonical form separates them (§4.2).
    for n in 0..nodes.len() {
        let tight = unseparated_text(&nodes[n], text);
        if !tight.is_empty() {
            nodes[n]
                .noncanonical
                .push("text right after a child node without a blank line".into());
        }
    }

    let mut pf = ParsedFile {
        path: path.to_string(),
        text: text.to_string(),
        nodes,
        root_node: root_idx,
        diagnostics,
    };

    if let Some(b) = block {
        if is_block_file {
            // The block root is the first top-level node.
            if let Some(&first) = pf.nodes[root_idx].children.first() {
                pf.nodes[first].block = Some(b);
            }
        } else {
            // root.md: attach to the implicit Root.
            pf.nodes[root_idx].block = Some(b);
            pf.nodes[root_idx].task = None;
        }
    }
    pf
}

/// Starts of the text children that follow a child node with no blank line
/// between them (§4.2): where canonical form inserts one.
pub fn unseparated_text(n: &Node, text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for w in n.content.windows(2) {
        if let [Content::Node(_), Content::Text(sp)] = w {
            if sp.start > 0 && !ends_with_blank_line(&text[..sp.start]) {
                out.push(sp.start);
            }
        }
    }
    out
}

/// Whether `s`, which ends at a line start, ends with a blank line: one that
/// is empty or whitespace, whatever its line ending (`\r\n` is read too).
pub(crate) fn ends_with_blank_line(s: &str) -> bool {
    let Some(s) = s.strip_suffix('\n') else {
        return false;
    };
    s[s.rfind('\n').map_or(0, |i| i + 1)..].trim().is_empty()
}

fn push_body(
    nodes: &mut Vec<Node>,
    stack: &mut Vec<Frame>,
    lines: &[Line],
    i: usize,
    block_root: Option<usize>,
) {
    let line = &lines[i];
    // A body line belongs to the deepest node whose region it reaches
    // (§3.3): for items, lines indented at least indent+2; for sections and
    // the root, anything that is not a more-indented item's region. Items
    // that own the line stop the walk; items that don't are looked through.
    while stack.len() > 1 {
        let top = stack.last().unwrap();
        let tnode = &nodes[top.node];
        let blank = line.raw.trim().is_empty();
        let reaches = if blank {
            true
        } else {
            match tnode.kind {
                Kind::Item => {
                    let mut t = false;
                    let ind = indent_cols(&line.raw, &mut t);
                    if ind >= top.indent + 2 {
                        true // the item owns this line
                    } else {
                        stack.pop();
                        continue; // look through to the item's parent
                    }
                }
                Kind::Section => {
                    let mut t = false;
                    if indent_cols(&line.raw, &mut t) < top.indent {
                        stack.pop();
                        continue; // outdented past a section under an item
                    }
                    true
                }
                Kind::Root => true,
            }
        };
        if reaches {
            break;
        }
        stack.pop();
    }
    let mut top = stack.last().unwrap().node;
    // column-0 text after a block file's root belongs to that root (§4.9)
    if top == 0 {
        if let Some(br) = block_root {
            if !line.raw.trim().is_empty() {
                nodes[br]
                    .noncanonical
                    .push("column-0 text in a block file, adopted by its root".into());
            }
            top = br;
        }
    }
    // consecutive lines extend the node's last text child; a line after a
    // child node starts a new one, so text never spans a child (§3.3)
    match nodes[top].content.last_mut() {
        Some(Content::Text(sp)) if sp.end == line.start => sp.end = line.next,
        _ => nodes[top].content.push(Content::Text(Span {
            start: line.start,
            end: line.next,
        })),
    }
    extend_spans(nodes, top, line.next);
}

/// Turn the pending title line and its underline into a section; returns
/// its index.
#[allow(clippy::too_many_arguments)]
fn make_setext_section(
    nodes: &mut Vec<Node>,
    stack: &mut Vec<Frame>,
    lines: &[Line],
    title_li: usize,
    title_indent: usize,
    underline_li: usize,
    level: usize,
    block_root: Option<usize>,
) -> usize {
    // The title line is the last text line pushed, so it ends the last text
    // child of the node push_body gave it to: the top frame's, or a block
    // file's root when that is the Root (§4.9). Take it back out, with the
    // adoption note that came with it.
    let title = lines[title_li].raw.trim().to_string();
    let underline = &lines[underline_li];
    let tl = &lines[title_li];
    {
        let mut owner = stack.last().unwrap().node;
        if owner == 0 {
            if let Some(br) = block_root {
                owner = br;
                let note = "column-0 text in a block file, adopted by its root";
                if nodes[br].noncanonical.last().is_some_and(|n| n == note) {
                    nodes[br].noncanonical.pop();
                }
            }
        }
        let nd = &mut nodes[owner];
        if let Some(Content::Text(sp)) = nd.content.last_mut() {
            if sp.end == tl.next {
                sp.end = tl.start;
                if sp.start >= sp.end {
                    nd.content.pop();
                }
            }
        }
        // push_body grew the owner's span, and its ancestors', over the
        // title line: give that back too, so a node the new section does not
        // sit in ends before it (sibling spans never overlap). The section's
        // own ancestors are grown again below.
        let mut cur = Some(owner);
        while let Some(c) = cur {
            if nodes[c].span.end == tl.next {
                nodes[c].span.end = tl.start;
            }
            cur = nodes[c].parent;
        }
    }
    // Pop frames that can't contain a section at this position.
    loop {
        let top = stack.last().unwrap();
        let tnode = &nodes[top.node];
        let can = match tnode.kind {
            Kind::Root => true,
            Kind::Item => title_indent > top.indent,
            Kind::Section => title_indent > top.indent || level > top.level.unwrap_or(0),
        };
        if can {
            break;
        }
        stack.pop();
    }
    // a column-0 section after a block file's root is adopted by it (§4.9)
    let mut parent = stack.last().unwrap().node;
    let mut noncanonical = vec!["setext heading".to_string()];
    if parent == 0 {
        if let Some(br) = block_root {
            parent = br;
            noncanonical.push("column-0 node in a block file, adopted by its root".into());
        }
    }
    let idx = nodes.len();
    nodes.push(Node {
        kind: Kind::Section,
        title,
        task: None,
        title_span: Span {
            start: lines[title_li].start,
            end: lines[title_li].end,
        },
        content: Vec::new(),
        span: Span {
            start: lines[title_li].start,
            end: underline.next,
        },
        children: Vec::new(),
        parent: Some(parent),
        file: nodes[parent].file,
        block: None,
        embed: None,
        indent: title_indent,
        level: Some(level),
        noncanonical,
    });
    nodes[parent].children.push(idx);
    nodes[parent].content.push(Content::Node(idx));
    extend_spans(nodes, idx, underline.next);
    stack.push(Frame {
        node: idx,
        indent: title_indent,
        level: Some(level),
        is_embed: false,
    });
    idx
}

fn extend_spans(nodes: &mut Vec<Node>, idx: usize, end: usize) {
    let mut cur = Some(idx);
    while let Some(c) = cur {
        if nodes[c].span.end < end {
            nodes[c].span.end = end;
        }
        cur = nodes[c].parent;
    }
}
