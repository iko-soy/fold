use notes_core::parse::{parse_file, Block, Kind, TaskState};
use notes_core::render::render;
use notes_core::tree::{NRef, Tree};
use notes_core::Id;

fn tree_of(text: &str) -> Tree {
    let block = Block {
        id: None,
        path: "root.md".into(),
        props: Default::default(),
        frontmatter_raw: String::new(),
        frontmatter_span: None,
        edge_span: None,
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
    let text = "# A\n\n## B\n\n### C\n\nbody\n";
    let t = tree_of(text);
    let b = t.resolved_children(t.resolved_children(t.root)[0])[0];
    let r = render(&t, b, 1, false);
    // base − level(node): B re-levels from 2 to 1, C from 3 to 2 (§5.1.4)
    assert_eq!(r, "# B\n\n## C\n\nbody\n");
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
        edge_span: None,
    };
    let root = parse_file("root.md", root_text, 0, Some(root_block));

    let block_text = "---\nid: dozzod-binwes-talsun-worbec\nsince: 2024-03\ntags: [storage, homelab]   # user comment\n---\n\n# ZFS layout\n\nMirrored pairs, no raidz. Snapshots hourly via sanoid.\n\n## [ ] Snapshot policy\n\n- hourly, keep 24\n- [x] Move scratch to its own dataset\n";
    let fm = notes_core::parse::parse_frontmatter(block_text).unwrap();
    let id = Id::parse("dozzod-binwes-talsun-worbec").unwrap();
    let block = Block {
        id: Some(id.clone()),
        path: "dozzod~zfs-layout.md".into(),
        props: fm.props,
        frontmatter_raw: fm.raw,
        frontmatter_span: Some(fm.span),
        edge_span: None,
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
    let text = "---\nid: racfer-hattes-mislup-nodrys\ntodo: open\ndue: 2026-09-20\n---\n\n- Order new switch\n  Two options, noted under Networking.\n";
    let fm = notes_core::parse::parse_frontmatter(text).unwrap();
    let id = Id::parse("racfer-hattes-mislup-nodrys").unwrap();
    let block = Block {
        id: Some(id),
        path: "racfer~order-new-switch.md".into(),
        props: fm.props,
        frontmatter_raw: fm.raw,
        frontmatter_span: Some(fm.span),
        edge_span: None,
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
