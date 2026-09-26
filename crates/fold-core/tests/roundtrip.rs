use fold_core::parse::{parse_file, Block, Kind, TaskState};
use fold_core::render::render;
use fold_core::tree::{NRef, Tree};
use fold_core::Id;

fn tree_of(text: &str) -> Tree {
    let block = Block {
        id: None,
        path: "root.md".into(),
        props: Default::default(),
        frontmatter_raw: String::new(),
        frontmatter_span: None,
    };
    let pf = parse_file("root.md", text, 0, Some(block));
    assert!(pf.diagnostics.is_empty(), "diags: {:?}", pf.diagnostics);
    Tree {
        files: vec![pf],
        root: (0, 0),
        blocks: vec![],
    }
}

#[test]
fn parse_simple_sections() {
    let text = "# Homelab\n\nTwo boxes.\n\n## NAS\n\n- [ ] Replace switch\n- [x] Done thing\n\n# Inbox\n";
    let t = tree_of(text);
    let root = t.root;
    let kids = t.resolved_children(root);
    assert_eq!(kids.len(), 2);
    let homelab = kids[0];
    assert_eq!(t.node(homelab).title, "Homelab");
    assert_eq!(t.node(homelab).kind, Kind::Section);
    assert_eq!(t.level(homelab), 1);
    let nas = t.resolved_children(homelab)[0];
    assert_eq!(t.node(nas).title, "NAS");
    assert_eq!(t.level(nas), 2);
    let tasks = t.resolved_children(nas);
    assert_eq!(tasks.len(), 2);
    assert_eq!(t.node(tasks[0]).task, Some(TaskState::Open));
    assert_eq!(t.node(tasks[1]).task, Some(TaskState::Done));
    assert_eq!(t.node(nas).body_lines(t.text_of(nas)), Vec::<&str>::new());
}

#[test]
fn body_of_section() {
    let text = "# A\n\nline one\nline two\n\n## B\n";
    let t = tree_of(text);
    let a = t.resolved_children(t.root)[0];
    assert_eq!(t.node(a).body_lines(t.text_of(a)), vec!["", "line one", "line two", ""]);
}

#[test]
fn section_under_item() {
    let text = "- Talked to Anya\n  ### Options\n  Warehouse, or the bakery.\n";
    let t = tree_of(text);
    let item = t.resolved_children(t.root)[0];
    assert_eq!(t.node(item).kind, Kind::Item);
    let opts = t.resolved_children(item)[0];
    assert_eq!(t.node(opts).kind, Kind::Section);
    assert_eq!(t.node(opts).title, "Options");
    assert_eq!(t.level(opts), 3);
    assert_eq!(t.indent(opts), 2);
    assert_eq!(
        t.node(opts).body_lines(t.text_of(opts)),
        vec!["  Warehouse, or the bakery."]
    );
}

#[test]
fn item_body_lines_are_not_children() {
    let text = "- Task title\n  a note on the task\n  another note\n- sibling\n";
    let t = tree_of(text);
    let item = t.resolved_children(t.root)[0];
    assert_eq!(t.resolved_children(item).len(), 0);
    assert_eq!(
        t.node(item).body_lines(t.text_of(item)),
        vec!["  a note on the task", "  another note"]
    );
}

#[test]
fn fences_hide_structure() {
    let text = "# A\n\n```\n# not a heading\n- not a bullet\n```\n\n## B\n";
    let t = tree_of(text);
    let a = t.resolved_children(t.root)[0];
    assert_eq!(t.resolved_children(a).len(), 1);
    assert_eq!(t.node(t.resolved_children(a)[0]).title, "B");
}

#[test]
fn render_roundtrip_root() {
    // For a top-level node of root.md, render(node, 1, false) is its span
    // modulo the trailing blank line that separates it from the next sibling
    // (the separator belongs to the gap, not the node).
    let text = "# Homelab\n\nTwo boxes in the closet.\n\n## NAS\n\n- [ ] Replace the flaky switch\n\n## Networking\n\n# Inbox\n\n## 2026-09-10\n\n- Talked to Anya about the venue.\n- [x] Send the deposit\n";
    let t = tree_of(text);
    let kids = t.resolved_children(t.root);
    for k in kids {
        let rendered = render(&t, k, 1, false);
        let span_text = t.node(k).span.text(t.text_of(k));
        let trimmed = span_text.trim_end_matches('\n');
        assert_eq!(rendered, format!("{}\n", trimmed), "node {:?}", t.node(k).title);
    }
}

#[test]
fn render_item_subtree() {
    let text = "# X\n\n- parent\n  - child one\n  - [ ] child two\n";
    let t = tree_of(text);
    let x = t.resolved_children(t.root)[0];
    let parent = t.resolved_children(x)[0];
    let r = render(&t, parent, 1, false);
    assert_eq!(r, "- parent\n  - child one\n  - [ ] child two\n");
}

#[test]
fn render_section_relevels() {
    // Descendants are re-levelled by base − level(node) in both spellings
    // (§5.1 step 4); this is what a block file made from B contains (§4.9).
    let text = "# A\n\n## B\n\n### C\n\nbody\n";
    let t = tree_of(text);
    let b = t.resolved_children(t.resolved_children(t.root)[0])[0];
    let r = render(&t, b, 1, false);
    assert_eq!(r, "# B\n\n## C\n\nbody\n");
}

#[test]
fn resolved_render_relevels() {
    // The resolved (user-facing) spelling re-levels by base − level(node)
    // (§5.1.4).
    let text = "# A\n\n## B\n\n### C\n\nbody\n";
    let t = tree_of(text);
    let b = t.resolved_children(t.resolved_children(t.root)[0])[0];
    let r = render(&t, b, 1, true);
    assert_eq!(r, "# B\n\n## C\n\nbody\n");
}

#[test]
fn written_levels_are_honoured() {
    // A heading under a bullet keeps its written level in the on-disk
    // spelling (§3.1): the app never re-levels a node it didn't touch.
    let text = "- item\n  ### deep section\n";
    let t = tree_of(text);
    let item = t.resolved_children(t.root)[0];
    let r = render(&t, item, 1, false);
    assert!(r.contains("### deep section"), "{}", r);
}

#[test]
fn spec_example_root_parses() {
    let text = "# Homelab\n\nTwo boxes in the closet, one at Hetzner.\n\n## NAS\n\n![[dozzod-binwes-talsun-worbec]]\n\n## Networking\n\n- [ ] Replace the flaky switch\n![[racfer-hattes-mislup-nodrys]]\n\n# Inbox\n\n## 2026-09-10\n\n- Talked to Anya about the venue.\n  ### Options\n  Warehouse on Ligovsky, or the old bakery. Both need a licence.\n- [x] Send the deposit\n";
    let t = tree_of(text);
    let homelab = t.resolved_children(t.root)[0];
    let nas = t.resolved_children(homelab)[0];
    let embed = t.resolved_children(nas)[0];
    assert!(t.node(embed).is_embed());
    assert_eq!(
        t.node(embed).embed.as_ref().unwrap(),
        &Id::parse("dozzod-binwes-talsun-worbec").unwrap()
    );
    let inbox = t.resolved_children(t.root)[1];
    let day = t.resolved_children(inbox)[0];
    let first = t.resolved_children(day)[0];
    assert_eq!(t.node(first).kind, Kind::Item);
    let options = t.resolved_children(first)[0];
    assert_eq!(t.node(options).title, "Options");
}

fn block_file_tree() -> Tree {
    let root_text = "# NAS\n\n![[dozzod-binwes-talsun-worbec]]\n";
    let root_block = Block {
        id: None,
        path: "root.md".into(),
        props: Default::default(),
        frontmatter_raw: String::new(),
        frontmatter_span: None,
    };
    let root = parse_file("root.md", root_text, 0, Some(root_block));

    let block_text = "---\nid: dozzod-binwes-talsun-worbec\nsince: 2024-03\ntags: [storage, homelab]   # user comment\n---\n\n# ZFS layout\n\nMirrored pairs, no raidz. Snapshots hourly via sanoid.\n\n## [ ] Snapshot policy\n\n- hourly, keep 24\n- [x] Move scratch to its own dataset\n";
    let fm = fold_core::parse::parse_frontmatter(block_text).unwrap();
    let id = Id::parse("dozzod-binwes-talsun-worbec").unwrap();
    let block = Block {
        id: Some(id.clone()),
        path: "dozzod~zfs-layout.md".into(),
        props: fm.props,
        frontmatter_raw: fm.raw,
        frontmatter_span: Some(fm.span),
    };
    let bf = parse_file("dozzod~zfs-layout.md", block_text, 1, Some(block));
    assert!(bf.diagnostics.is_empty(), "diags: {:?}", bf.diagnostics);
    Tree {
        root: (0, 0),
        blocks: vec![((1, bf.nodes[0].children[0]), id)],
        files: vec![root, bf],
    }
}

#[test]
fn block_file_parses() {
    let t = block_file_tree();
    let zfs: NRef = t.blocks[0].0;
    assert_eq!(t.node(zfs).title, "ZFS layout");
    let b = t.node(zfs).block.as_ref().unwrap();
    assert_eq!(b.prop("id"), Some("dozzod-binwes-talsun-worbec"));
    assert_eq!(b.prop("since"), Some("2024-03"));
    // unknown keys preserved in raw
    assert!(b.frontmatter_raw.contains("tags: [storage, homelab]"));
    let policy = t.resolved_children(zfs)[0];
    assert_eq!(t.node(policy).task, Some(TaskState::Open));
    assert_eq!(t.node(policy).title, "Snapshot policy");
}

#[test]
fn block_file_roundtrips() {
    let t = block_file_tree();
    let zfs = t.blocks[0].0;
    let r = render(&t, zfs, 1, false);
    assert_eq!(r, t.files[1].text, "render(block, 1, false) must be byte-identical to the file");
}

#[test]
fn resolved_render_has_no_boundaries() {
    let t = block_file_tree();
    let nas = t.resolved_children(t.root)[0];
    let r = render(&t, nas, 1, true);
    assert!(!r.contains("![["));
    assert!(r.contains("# NAS"));
    assert!(r.contains("## ZFS layout"));
    assert!(r.contains("### [ ] Snapshot policy"));
    assert!(r.contains("- hourly, keep 24"));
}

#[test]
fn bullet_root_block_file() {
    // A block whose root is an item: the file starts with a bullet (§4.9).
    let text = "---\nid: racfer-hattes-mislup-nodrys\ndue: 2026-09-20\n---\n\n- [ ] Order new switch\n  Two options, noted under Networking.\n";
    let fm = fold_core::parse::parse_frontmatter(text).unwrap();
    let id = Id::parse("racfer-hattes-mislup-nodrys").unwrap();
    let block = Block {
        id: Some(id),
        path: "racfer~order-new-switch.md".into(),
        props: fm.props,
        frontmatter_raw: fm.raw,
        frontmatter_span: Some(fm.span),
    };
    let pf = parse_file("racfer~order-new-switch.md", text, 0, Some(block));
    assert!(pf.diagnostics.is_empty(), "diags: {:?}", pf.diagnostics);
    let root_node = pf.nodes[0].children[0];
    let n = &pf.nodes[root_node];
    assert_eq!(n.kind, Kind::Item);
    assert_eq!(n.title, "Order new switch");
    assert_eq!(n.task, Some(TaskState::Open));
    let t = Tree {
        files: vec![pf],
        root: (0, 0),
        blocks: vec![],
    };
    let r = render(&t, (0, root_node), 1, false);
    assert_eq!(r, text);
}

// ---------------------------------------------------------------- regressions

fn tree_with_block(root_text: &str, block_text: &str, block_path: &str) -> Tree {
    let root_block = Block {
        id: None,
        path: "root.md".into(),
        props: Default::default(),
        frontmatter_raw: String::new(),
        frontmatter_span: None,
    };
    let root = parse_file("root.md", root_text, 0, Some(root_block));
    let fm = fold_core::parse::parse_frontmatter(block_text).unwrap();
    let id = Id::parse(fm.props.get("id").unwrap()).unwrap();
    let block = Block {
        id: Some(id.clone()),
        path: block_path.into(),
        props: fm.props,
        frontmatter_raw: fm.raw,
        frontmatter_span: Some(fm.span),
    };
    let bf = parse_file(block_path, block_text, 1, Some(block));
    Tree {
        root: (0, 0),
        blocks: vec![((1, bf.nodes[0].children[0]), id)],
        files: vec![root, bf],
    }
}

const SPEC_ROOT: &str = "# Homelab\n\nTwo boxes in the closet, one at Hetzner.\n\n## NAS\n\n![[dozzod-binwes-talsun-worbec]]\n\n## Networking\n\n- [ ] Replace the flaky switch\n![[racfer-hattes-mislup-nodrys]]\n\n# Inbox\n\n## 2026-09-10\n\n- Talked to Anya about the venue.\n  ### Options\n  Warehouse on Ligovsky, or the old bakery. Both need a licence.\n- [x] Send the deposit\n";

#[test]
fn item_after_section_under_item_is_a_sibling() {
    // §4.10: "Send the deposit" is a sibling of "Talked to Anya", not a
    // child of the section nested under it.
    let t = tree_of(SPEC_ROOT);
    let inbox = t.resolved_children(t.root)[1];
    let day = t.resolved_children(inbox)[0];
    let kids = t.resolved_children(day);
    assert_eq!(kids.len(), 2);
    assert_eq!(t.node(kids[1]).title, "Send the deposit");
    // and the whole file round-trips through render(root)
    assert_eq!(render(&t, t.root, 1, false), SPEC_ROOT);
}

#[test]
fn outdented_heading_leaves_section_under_item() {
    let t = tree_of("- a\n  ## T\n### X\n");
    let top = t.resolved_children(t.root);
    assert_eq!(top.len(), 2, "X is top-level");
    assert_eq!(t.node(top[1]).title, "X");
}

#[test]
fn body_after_children_does_not_swallow_them() {
    let text = "# A\n\npara\n\n- a\n- b\n\nmore text\n";
    let t = tree_of(text);
    let a = t.resolved_children(t.root)[0];
    assert_eq!(t.resolved_children(a).len(), 2);
    assert_eq!(render(&t, a, 1, false), text);
    assert_eq!(render(&t, a, 1, true), text);
}

#[test]
fn text_between_children_keeps_its_place() {
    let text = "# A\n\n- a\n\ntext\n\n- b\n";
    let t = tree_of(text);
    let a = t.resolved_children(t.root)[0];
    assert_eq!(render(&t, a, 1, false), text);
}

#[test]
fn crlf_input_is_read_cleanly() {
    let text = "# A\r\n\r\nbody\r\n\r\n## B\r\n";
    let t = tree_of(text);
    let a = t.resolved_children(t.root)[0];
    assert_eq!(t.node(a).body_lines(t.text_of(a)), vec!["", "body", ""]);
    assert_eq!(render(&t, a, 1, false), "# A\n\nbody\n\n## B\n");
    let fm = fold_core::parse::parse_frontmatter("---\r\nid: x\r\n---\r\n\r\n# T\r\n").unwrap();
    assert_eq!(fm.props.get("id").map(String::as_str), Some("x"));
    assert_eq!(fm.span.end, "---\r\nid: x\r\n---\r\n\r\n".len());
}

#[test]
fn spacing_is_reproduced() {
    for text in [
        "- A\n  note\n  - B\n",
        "- A\n    code\n",
        "- A\n  ## S\n  - b\n",
        "# S\n\n- A\n\n- B\n",
        "- A\n  - B\n",
        "# A\n\n- x\n  note\n\n  more\n- y\n",
    ] {
        let t = tree_of(text);
        assert_eq!(render(&t, t.root, 1, false), text);
        let first = t.resolved_children(t.root)[0];
        assert_eq!(render(&t, first, 1, false), text);
        assert_eq!(render(&t, first, 1, true), text);
    }
}

#[test]
fn render_of_root_starts_at_level_one() {
    let text = "# A\n\nbody\n\n## B\n";
    let t = tree_of(text);
    assert_eq!(render(&t, t.root, 1, false), text);
    assert_eq!(render(&t, t.root, 1, true), text);
}

#[test]
fn unresolved_render_keeps_embeds() {
    let t = block_file_tree();
    let nas = t.resolved_children(t.root)[0];
    let r = render(&t, nas, 1, false);
    assert_eq!(r, "# NAS\n\n![[dozzod-binwes-talsun-worbec]]\n");
}

#[test]
fn embed_under_item_keeps_its_indent_when_resolved() {
    let t = tree_with_block(
        "- A\n  ![[racfer-hattes-mislup-nodrys]]\n",
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- B\n",
        "racfer~b.md",
    );
    let a = t.resolved_children(t.root)[0];
    assert_eq!(render(&t, a, 1, true), "- A\n  - B\n");
    assert_eq!(render(&t, a, 1, false), "- A\n  ![[racfer-hattes-mislup-nodrys]]\n");
}

#[test]
fn spans_stay_inside_a_file_without_final_newline() {
    let text = "- a\n- b";
    let t = tree_of(text);
    for k in t.resolved_children(t.root) {
        assert!(t.node(k).span.end <= text.len());
    }
    assert_eq!(render(&t, t.root, 1, false), "- a\n- b\n");
}

#[test]
fn dashes_are_a_thematic_break_not_a_setext_underline() {
    // §4.4: a `---` line outside frontmatter is body text.
    let text = "# A\n\ntext\n---\nmore\n";
    let t = tree_of(text);
    let a = t.resolved_children(t.root)[0];
    assert!(t.resolved_children(a).is_empty());
    assert_eq!(render(&t, a, 1, false), text);
    // `=` underlines are still setext headings, converted on write (§4.2)
    let t = tree_of("Title\n===\n\nbody\n");
    let s = t.resolved_children(t.root)[0];
    assert_eq!(t.node(s).title, "Title");
    assert_eq!(render(&t, s, 1, false), "# Title\n\nbody\n");
}

#[test]
fn empty_title_nodes_parse_everywhere() {
    // n / N create empty nodes (§10.3); they must parse as nodes, never as
    // setext underlines, wherever they land.
    let cases: &[(&str, usize)] = &[
        ("# A\n\n- \n", 1),
        ("# A\n- \n", 1),
        ("# A\n\npara\n- \n", 1),
        ("# A\n\npara\n-\n", 1),
        ("# A\n\n## \n", 1),
        ("# A\n\npara\n## \n", 1),
        ("# A\n\n- [ ] \n", 1),
        ("# A\n\n- x\n  - \n", 1),
    ];
    for (text, kids) in cases {
        let pf = parse_file("root.md", text, 0, None);
        let t = Tree {
            files: vec![pf],
            root: (0, 0),
            blocks: vec![],
        };
        let a = t.resolved_children(t.root)[0];
        let ch = t.resolved_children(a);
        assert_eq!(ch.len(), *kids, "{:?}", text);
        let mut n = ch[0];
        while let Some(&k) = t.resolved_children(n).first() {
            n = k;
        }
        assert_eq!(t.node(n).title, "", "{:?}", text);
        // rendering keeps the node (without trailing whitespace)
        let r = render(&t, t.root, 1, false);
        let pf2 = parse_file("root.md", &r, 0, None);
        assert_eq!(pf2.nodes.len(), t.files[0].nodes.len(), "{:?} → {:?}", text, r);
    }
    let pf = parse_file("root.md", "# A\n\n- [ ] \n", 0, None);
    assert!(pf.nodes.iter().any(|n| n.task == Some(TaskState::Open) && n.title.is_empty()));
}

#[test]
fn render_keeps_child_sections_under_items_nested() {
    // A skipped-level section (§3.1) re-levelled by base − level(node)
    // (§5.1) can push a section under an item below level 1; its child
    // section must still print deeper than it, so that
    // parse(render(t, 1, false)) keeps the tree's shape.
    let t = tree_of("### Meeting\n- topic\n  ## Notes\n  ### Sub\n");
    let m = t.resolved_children(t.root)[0];
    let topic = t.resolved_children(m)[0];
    let notes = t.resolved_children(topic)[0];
    assert_eq!(t.resolved_children(notes).len(), 1, "Sub is a child of Notes");
    let r = render(&t, m, 1, false);
    assert_eq!(r, "# Meeting\n- topic\n  # Notes\n  ## Sub\n");
    let t2 = tree_of(&r);
    let m2 = t2.resolved_children(t2.root)[0];
    let topic2 = t2.resolved_children(m2)[0];
    assert_eq!(t2.resolved_children(topic2).len(), 1, "Notes and Sub became siblings:\n{r}");
    let notes2 = t2.resolved_children(topic2)[0];
    assert_eq!(t2.node(notes2).title, "Notes");
    assert_eq!(t2.resolved_children(notes2).len(), 1, "{r}");
}

#[test]
fn inline_triple_backticks_do_not_open_a_fence() {
    // CommonMark: a backtick fence's info string may not contain backticks,
    // so "```npm i``` first" is a paragraph with inline code, not a fence.
    let t = tree_of("# A\n\n```npm i``` first\n\n## B\n\n- item\n");
    let a = t.resolved_children(t.root)[0];
    let kids = t.resolved_children(a);
    assert_eq!(kids.len(), 1);
    assert_eq!(t.node(kids[0]).title, "B");
    assert_eq!(t.resolved_children(kids[0]).len(), 1);
}

#[test]
fn fence_line_with_info_string_does_not_close_a_fence() {
    // CommonMark: a closing fence may be followed only by spaces or tabs,
    // so "```rust" inside an open ``` fence is code, not a closer.
    let t = tree_of("# A\n\n```\n```rust\n# not a heading\n```\n\n## B\n");
    let top = t.resolved_children(t.root);
    assert_eq!(top.len(), 1);
    let kids = t.resolved_children(top[0]);
    assert_eq!(kids.len(), 1);
    assert_eq!(t.node(kids[0]).title, "B");
}

#[test]
fn shifting_keeps_code_after_a_fence_line_with_info_string() {
    // the re-levelling passes of paste/refile and of splice read fences as
    // the parser does: the code line stays as written
    let doc = "# B\n\n```\n```rust\n# not a heading\n```\n";
    assert_eq!(
        fold_core::ops::shift_document(doc, 1, 0),
        "## B\n\n```\n```rust\n# not a heading\n```\n"
    );
    let dir = tempfile::tempdir().unwrap();
    let text = "# Top\n\n## B\n\n```\n```rust\n# not a heading\n```\n";
    std::fs::write(dir.path().join("root.md"), text).unwrap();
    let mut v = fold_core::vault::Vault::open(dir.path()).unwrap();
    let top = v.tree.resolved_children(v.tree.root)[0];
    let b = v.tree.resolved_children(top)[0];
    let mut buf = fold_core::edit::open_editor(&v, b);
    for o in buf.owners.keys().copied().collect::<Vec<_>>() {
        buf.mark_dirty(o);
    }
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, text);
}

#[test]
fn spaced_thematic_breaks_are_text() {
    // §4.2/§4.4: CommonMark thematic breaks (which win over list items) are
    // body text, however they are spelled — not bullets titled "* *" / "- -".
    for text in [
        "# A\n\npara\n\n* * *\n\nmore\n",
        "# A\n\npara\n\n- - -\n\nmore\n",
        "# A\n\npara\n\n-  -  - -\n\nmore\n",
        "# A\n\n- x\n\n  * * *\n",
    ] {
        let t = tree_of(text);
        let a = t.resolved_children(t.root)[0];
        let mut kids: Vec<String> = Vec::new();
        t.walk(a, &mut |t, k| kids.push(t.node(k).title.clone()));
        kids.retain(|k| k != "A" && k != "x");
        assert!(kids.is_empty(), "{:?} parsed as nodes {:?}", text, kids);
        assert_eq!(render(&t, a, 1, false), text);
    }
    // two marks are not a break: still an (empty-ish) bullet
    let t = tree_of("# A\n\n- -\n");
    let a = t.resolved_children(t.root)[0];
    assert_eq!(t.resolved_children(a).len(), 1);
}
