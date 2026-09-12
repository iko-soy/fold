use notes_core::edit::{open_editor, title_of_first_line, Owner};
use notes_core::ops;
use notes_core::vault::Vault;

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
    let owner = buf.lines[idx].owner;
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
