use fold_core::edit::{open_editor, title_of_first_line, Owner};
use fold_core::ops;
use fold_core::vault::Vault;

fn vault_with(root: &str) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), root).unwrap();
    let v = Vault::open(dir.path()).unwrap();
    (dir, v)
}

#[test]
fn buffer_tags_lines_with_owners() {
    let (_d, v) = vault_with("# A\n\nbody line\n\n## B\n\n- item\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let buf = open_editor(&v, a);
    let texts: Vec<&str> = buf.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(texts.contains(&"# A"));
    assert!(texts.contains(&"body line"));
    assert!(texts.contains(&"## B"));
    assert!(texts.contains(&"- item"));
    // all lines owned by root.md's block 0
    assert!(buf.lines.iter().all(|l| l.owner.file == 0));
}

#[test]
fn nested_block_lines_carry_own_tag() {
    let (_d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let buf = open_editor(&v, a);
    // the nested block's title line is owned by the block file, not root.md
    let title_line = buf
        .lines
        .iter()
        .find(|l| l.text.contains("task"))
        .expect("task line present");
    assert_eq!(title_line.owner.file, 1, "task line owned by the block file");
    assert!(buf.owners.len() == 2);
}

#[test]
fn splice_roundtrip_is_noop() {
    // splice(node, render(node, 1, true)) is a no-op on disk (§15.6)
    let (_d, mut v) = vault_with("# A\n\nbody\n\n## B\n\n- one\n- two\n");
    let before = v.tree.files[0].text.clone();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    // mark everything dirty without changing text: splice must rewrite
    // byte-identical content
    let owner = buf.lines[0].owner;
    buf.mark_dirty(owner);
    buf.splice(&mut v, owner).unwrap();
    let after = &v.tree.files[0].text;
    assert_eq!(*after, before, "{}", after);
}

#[test]
fn edit_body_splices_back() {
    let (_d, mut v) = vault_with("# A\n\nold body\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let body_idx = buf
        .lines
        .iter()
        .position(|l| l.text == "old body")
        .unwrap();
    buf.set_line(body_idx, "new body".into());
    buf.save_all(&mut v).unwrap();
    assert!(v.tree.files[0].text.contains("new body"), "{}", v.tree.files[0].text);
    assert!(!v.tree.files[0].text.contains("old body"));
}

#[test]
fn edit_nested_block_writes_its_own_file() {
    let (_d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    let root_before = v.tree.files[0].text.clone();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    // retitle the nested block
    let idx = buf
        .lines
        .iter()
        .position(|l| l.text.contains("task"))
        .unwrap();
    let _owner = buf.lines[idx].owner;
    buf.set_line(idx, "- renamed task".into());
    buf.save_all(&mut v).unwrap();
    // root.md untouched
    assert_eq!(v.tree.files[0].text, root_before);
    // block file has the new title
    assert!(
        v.tree.files.iter().any(|f| f.text.contains("renamed task")),
        "{:?}",
        v.tree.files.iter().map(|f| &f.text).collect::<Vec<_>>()
    );
}

#[test]
fn deleting_nested_block_title_trashes_block() {
    // §5.2: deleting a nested block's title line deletes the block; its
    // other lines would need re-tagging (the TUI does that; here we just
    // check the file write path).
    let (_d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let buf = open_editor(&v, a);
    assert_eq!(buf.owners.len(), 2);
    let block_owner = *buf
        .owners
        .keys()
        .find(|o| o.file == 1)
        .unwrap();
    assert_eq!(block_owner, Owner { file: 1, block_ord: 1 });
}

#[test]
fn title_detection() {
    assert_eq!(title_of_first_line("# Hello"), Some("Hello".into()));
    assert_eq!(title_of_first_line("- [ ] Task"), Some("[ ] Task".into()));
    assert_eq!(title_of_first_line("plain"), None);
}

// ---------------------------------------------------------------- regressions

/// Open the editor on `r`, mark every owner dirty and save: nothing on disk
/// may change (§15.6).
fn assert_noop_splice(v: &mut Vault, r: fold_core::tree::NRef) {
    let before: Vec<String> = v.tree.files.iter().map(|f| f.text.clone()).collect();
    let mut buf = open_editor(v, r);
    for o in buf.owners.keys().copied().collect::<Vec<_>>() {
        buf.mark_dirty(o);
    }
    buf.save_all(v).unwrap();
    let after: Vec<String> = v.tree.files.iter().map(|f| f.text.clone()).collect();
    assert_eq!(after, before);
}

#[test]
fn noop_splice_of_zoomed_nodes() {
    for (text, path) in [
        ("# A\n\n## B\n\nbody\n", vec!["A", "B"]),
        ("- A\n  - B\n  - C\n", vec!["A", "B"]),
        ("- A\n  - B\n", vec!["A"]),
        ("# A\n\nbody\n\n# B\n", vec!["A"]),
        ("# Inbox\n\n## day\n\n- Talked\n  ### Options\n  Warehouse.\n- [x] Send\n", vec!["Inbox", "day", "Talked"]),
    ] {
        let (_d, mut v) = vault_with(text);
        let segs: Vec<String> = path.iter().map(|s| s.to_string()).collect();
        let r = v.find_by_path(&segs).unwrap();
        assert_noop_splice(&mut v, r);
        let root = v.tree.root;
        assert_noop_splice(&mut v, root);
    }
}

#[test]
fn noop_splice_with_root_frontmatter() {
    let (_d, mut v) = vault_with("---\nfoo: bar\n---\n\n# A\n\nbody\n");
    let root = v.tree.root;
    assert_noop_splice(&mut v, root);
}

#[test]
fn noop_splice_across_nested_blocks() {
    let (_d, mut v) = vault_with("# A\n\n## Z\n\nbody\n\n### sub\n\n- t\n");
    let z = v.find_by_path(&["A".into(), "Z".into()]).unwrap();
    ops::make_block(&mut v, z).unwrap();
    let t = v.find_by_path(&["A".into(), "Z".into(), "sub".into(), "t".into()]).unwrap();
    ops::make_block(&mut v, t).unwrap();
    let block = v.tree.files.iter().find(|f| f.path.contains("~z")).unwrap();
    // made from a level-2 section: re-levelled to the file's level 1 (§4.9)
    assert!(block.text.contains("\n# Z\n\nbody\n\n## sub\n\n![["), "{}", block.text);
    let a = v.tree.resolved_children(v.tree.root)[0];
    assert_noop_splice(&mut v, a);
    let root = v.tree.root;
    assert_noop_splice(&mut v, root);
    let z = v.find_by_path(&["A".into(), "Z".into()]).unwrap();
    assert_noop_splice(&mut v, z);
}

#[test]
fn splicing_a_parent_keeps_nested_embeds() {
    let (_d, mut v) = vault_with("# NAS\n\n- task\n");
    let task = v.find_by_path(&["NAS".into(), "task".into()]).unwrap();
    ops::make_block(&mut v, task).unwrap();
    let nas = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, nas);
    let i = buf.lines.iter().position(|l| l.text == "# NAS").unwrap();
    buf.set_line(i, "# NAS box".into());
    buf.save_all(&mut v).unwrap();
    let root = &v.tree.files[0].text;
    assert!(root.starts_with("# NAS box\n\n![["), "{}", root);
    assert_eq!(v.tree.resolved_children(v.tree.resolved_children(v.tree.root)[0]).len(), 1);
}

#[test]
fn nested_block_embed_under_item_keeps_indent() {
    let (_d, mut v) = vault_with("- A\n  - B\n");
    let b = v.find_by_path(&["A".into(), "B".into()]).unwrap();
    ops::make_block(&mut v, b).unwrap();
    assert!(v.tree.files[0].text.starts_with("- A\n  ![["), "{}", v.tree.files[0].text);
    let a = v.tree.resolved_children(v.tree.root)[0];
    let buf = open_editor(&v, a);
    let texts: Vec<&str> = buf.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, vec!["- A", "  - B"]);
    assert_noop_splice(&mut v, a);
}

#[test]
fn splice_refuses_to_overwrite_external_change() {
    let (d, mut v) = vault_with("# A\n\nbody\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    std::fs::write(d.path().join("root.md"), "# A\n\nsomeone else\n").unwrap();
    let i = buf.lines.iter().position(|l| l.text == "body").unwrap();
    buf.set_line(i, "mine".into());
    assert!(buf.save_all(&mut v).is_err());
    let disk = std::fs::read_to_string(d.path().join("root.md")).unwrap();
    assert_eq!(disk, "# A\n\nsomeone else\n");
}

#[test]
fn successive_splices_of_one_file_are_allowed() {
    let (_d, mut v) = vault_with("# A\n\nbody\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let i = buf.lines.iter().position(|l| l.text == "body").unwrap();
    buf.set_line(i, "one".into());
    buf.save_all(&mut v).unwrap();
    buf.set_line(i, "two".into());
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, "# A\n\ntwo\n");
}

#[test]
fn noop_splice_of_spec_example_vault() {
    // §4.10, all three files: editing from the root and saving every block
    // changes nothing.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# Homelab\n\nTwo boxes in the closet, one at Hetzner.\n\n## NAS\n\n![[dozzod-binwes-talsun-worbec]]\n\n## Networking\n\n- [ ] Replace the flaky switch\n![[racfer-hattes-mislup-nodrys]]\n\n# Inbox\n\n## 2026-09-10\n\n- Talked to Anya about the venue.\n  ### Options\n  Warehouse on Ligovsky, or the old bakery. Both need a licence.\n- [x] Send the deposit\n").unwrap();
    std::fs::write(dir.path().join("dozzod~zfs-layout.md"), "---\nid: dozzod-binwes-talsun-worbec\nsince: 2024-03\ntags: [storage, homelab]   # user-defined\n---\n\n# ZFS layout\n\nMirrored pairs, no raidz. Snapshots hourly via sanoid.\n\n## [ ] Snapshot policy\n\n- hourly, keep 24\n- [x] Move scratch to its own dataset\n").unwrap();
    std::fs::write(dir.path().join("racfer~order-new-switch.md"), "---\nid: racfer-hattes-mislup-nodrys\ndue: 2026-09-20\n---\n\n- [ ] Order new switch\n  Two options, noted under Networking.\n").unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    let root = v.tree.root;
    let buf = open_editor(&v, root);
    let texts: Vec<&str> = buf.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(texts.contains(&"### ZFS layout"), "{:#?}", texts);
    assert!(texts.contains(&"#### [ ] Snapshot policy"), "{:#?}", texts);
    assert!(texts.contains(&"- [ ] Order new switch"), "{:#?}", texts);
    assert!(!texts.iter().any(|t| t.contains("![[")), "{:#?}", texts);
    assert_noop_splice(&mut v, root);
    let homelab = v.tree.resolved_children(v.tree.root)[0];
    assert_noop_splice(&mut v, homelab);
}
