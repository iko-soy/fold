use fold_core::edit::{open_editor, Owner};
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
    // other lines are re-tagged to the enclosing block on save (see
    // deleting_nested_block_title_keeps_its_text_reachable). Here: the
    // block's lines carry its own owner.
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

/// A vault whose root.md embeds one block file with the given body.
fn vault_with_block(body: &str) -> (tempfile::TempDir, Vault, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let id = "racfer-hattes-mislup-nodrys";
    std::fs::write(dir.path().join("root.md"), format!("# ![[{id}]]\n")).unwrap();
    let bf = dir.path().join("racfer~inbox.md");
    std::fs::write(&bf, format!("---\nid: {id}\n---\n\n{body}")).unwrap();
    let v = Vault::open(dir.path()).unwrap();
    (dir, v, bf)
}

#[test]
fn setext_heading_in_block_file_survives_noop_splice() {
    // §4.9: a column-0 node after a block file's root — here a setext
    // heading a phone editor appended — is adopted by the root, so it shows
    // under the block and a no-op splice keeps it on disk (§15.6), written
    // in place under the root.
    let (_d, mut v, bf) = vault_with_block("# Inbox\n\nNotes\n=====\n\nprecious\n");
    let inbox = v.tree.resolved_children(v.tree.root)[0];
    assert_eq!(v.tree.node(inbox).title, "Inbox");
    let kids = v.tree.resolved_children(inbox);
    assert_eq!(kids.len(), 1);
    assert_eq!(v.tree.node(kids[0]).title, "Notes");
    let mut buf = open_editor(&v, inbox);
    let owner = buf.lines[0].owner;
    buf.mark_dirty(owner);
    buf.splice(&mut v, owner).unwrap();
    let disk = std::fs::read_to_string(&bf).unwrap();
    assert_eq!(
        disk,
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n# Inbox\n\n## Notes\n\nprecious\n"
    );
}

#[test]
fn setext_heading_after_a_bullet_block_root_is_adopted_once() {
    // the title line was adopted into the bullet root's text before its
    // underline showed up; it must become the node, not stay behind as text
    let (_d, mut v, bf) = vault_with_block("- Order switch\nTitle\n=====\nmore\n");
    let order = v.tree.resolved_children(v.tree.root)[0];
    let kids = v.tree.resolved_children(order);
    assert_eq!(kids.len(), 1);
    assert_eq!(v.tree.node(kids[0]).title, "Title");
    let diags: Vec<String> = fold_core::check::check(&v)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(!diags.iter().any(|m| m.contains("column-0 text")), "{diags:?}");
    assert!(diags.iter().any(|m| m.contains("column-0 node")), "{diags:?}");
    let mut buf = open_editor(&v, order);
    let owner = buf.lines[0].owner;
    buf.mark_dirty(owner);
    buf.splice(&mut v, owner).unwrap();
    let disk = std::fs::read_to_string(&bf).unwrap();
    assert_eq!(
        disk,
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- Order switch\n  # Title\n  more\n"
    );
}

#[test]
fn setext_heading_can_be_a_block_files_root() {
    // its title line is the root's, not text before the root (§4.9)
    let (_d, v, _bf) = vault_with_block("Inbox\n=====\n\nbody\n");
    let inbox = v.tree.resolved_children(v.tree.root)[0];
    assert_eq!(v.tree.node(inbox).title, "Inbox");
    let diags: Vec<String> = fold_core::check::check(&v)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(!diags.iter().any(|m| m.contains("before the block")), "{diags:?}");
}

/// Two embeds of one block (§6.2: a diagnostic; the second renders as
/// broken). Returns the vault dir, the vault and the block file's path.
fn vault_with_duplicate_embed() -> (tempfile::TempDir, Vault, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("root.md"),
        "# Top\n\n## A\n\n![[racfer-hattes-mislup-nodrys]]\n\n## B\n\n![[racfer-hattes-mislup-nodrys]]\n",
    )
    .unwrap();
    let bf = dir.path().join("racfer~task.md");
    std::fs::write(&bf, "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- task\n").unwrap();
    let v = Vault::open(dir.path()).unwrap();
    (dir, v, bf)
}

#[test]
fn duplicate_embed_does_not_duplicate_block_text() {
    let (_d, mut v, bf) = vault_with_duplicate_embed();
    let top = v.tree.resolved_children(v.tree.root)[0];
    // the block is inlined at its first embed only; the second stays an
    // embed line, as a broken one would
    let r = fold_core::render::render(&v.tree, top, 1, true);
    assert_eq!(r.matches("- task").count(), 1, "{r}");
    assert_eq!(r.matches("![[racfer-hattes-mislup-nodrys]]").count(), 1, "{r}");
    let mut buf = open_editor(&v, top);
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    buf.set_line(i, "- task!".into());
    buf.save_all(&mut v).unwrap();
    assert_eq!(
        std::fs::read_to_string(&bf).unwrap(),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- task!\n"
    );
}

#[test]
fn duplicate_embed_survives_saving_the_parent() {
    let (d, mut v, _bf) = vault_with_duplicate_embed();
    let top = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, top);
    let i = buf.lines.iter().position(|l| l.text == "# Top").unwrap();
    buf.set_line(i, "# Top!".into());
    buf.save_all(&mut v).unwrap();
    assert_eq!(
        std::fs::read_to_string(d.path().join("root.md")).unwrap(),
        "# Top!\n\n## A\n\n![[racfer-hattes-mislup-nodrys]]\n\n## B\n\n![[racfer-hattes-mislup-nodrys]]\n"
    );
    // zoomed into B, the second embed is still the broken one
    let top = v.tree.resolved_children(v.tree.root)[0];
    let b = v.tree.resolved_children(top)[1];
    assert_eq!(
        fold_core::render::render(&v.tree, b, 1, true),
        "# B\n\n![[racfer-hattes-mislup-nodrys]]\n"
    );
    assert_noop_splice(&mut v, top);
}

#[test]
fn line_opened_above_zoomed_title_is_not_duplicated() {
    // Editing a plain (non-block) section: a line opened above its title
    // (vim `O` on line 0, or Enter at column 0 of line 0) is saved, then a
    // later edit is saved from the same buffer. The second splice must
    // rewrite the region the first one wrote, not whatever node now starts
    // at the section's original byte offset.
    let (_d, mut v) = vault_with("# A\n\nbody\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    // what `O` / Enter at column 0 of line 0 does: split line 0
    buf.set_line(0, "# Intro".into());
    buf.insert_line(0, "# A".into());
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, "# Intro\n# A\n\nbody\n");
    let i = buf.lines.iter().position(|l| l.text == "body").unwrap();
    buf.set_line(i, "body!".into());
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, "# Intro\n# A\n\nbody!\n");
    // a plain first line: no node starts where the section did any more
    let (_d, mut v) = vault_with("# A\n\nbody\n\n# B\n\nb\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    buf.set_line(0, "intro".into());
    buf.insert_line(0, "# A".into());
    buf.save_all(&mut v).unwrap();
    let i = buf.lines.iter().position(|l| l.text == "body").unwrap();
    buf.set_line(i, "body!".into());
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, "intro\n# A\n\nbody!\n\n# B\n\nb\n");
}

#[test]
fn splice_keeps_text_before_block_root() {
    // §4.9: text before a block file's root is a diagnostic and the file is
    // read-only until fixed; §11.5: never delete user content without a
    // trash copy. Saving the block from the editor must not drop it.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("root.md"),
        "# A\n\n![[racfer-hattes-mislup-nodrys]]\n",
    )
    .unwrap();
    let bf = dir.path().join("racfer~task.md");
    let malformed = "---\nid: racfer-hattes-mislup-nodrys\n---\n\nphone note\n- task\n";
    std::fs::write(&bf, malformed).unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    buf.set_line(i, "- task!".into());
    // refused (read-only), and the block stays unsaved
    assert!(buf.save_all(&mut v).is_err());
    assert_eq!(std::fs::read_to_string(&bf).unwrap(), malformed);
    assert!(buf.dirty.iter().any(|o| o.file == 1));
}

#[test]
fn splice_never_writes_a_block_file_without_its_root() {
    // §5.2 step 2: a block's text parses to exactly one root-level node.
    // Editing a block itself and deleting its title line leaves no root;
    // splice refuses rather than write a file with only frontmatter and
    // text, which would drop the block from the tree.
    let (d, mut v) = vault_with("# A\n\n- task\n  note\n");
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    ops::make_block(&mut v, t).unwrap();
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let path = d.path().join(&v.tree.files[1].path);
    let before = std::fs::read_to_string(&path).unwrap();
    let mut buf = open_editor(&v, t);
    let texts: Vec<&str> = buf.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["- task", "  note"]);
    buf.delete_line(0);
    assert!(buf.save_all(&mut v).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    // text typed above the title line would be text before the root
    let mut buf = open_editor(&v, t);
    buf.set_line(0, "intro".into());
    buf.insert_line(0, "- task".into());
    assert!(buf.save_all(&mut v).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn deleting_nested_block_title_keeps_its_text_reachable() {
    // §5.2: deleting a nested block's title line deletes the block; the
    // lines it still owned become plain text of the enclosing block. The
    // block file must not be rewritten without a root (which drops the
    // block from the tree, breaks the parent's embed and hides "note").
    let (d, mut v) = vault_with("# A\n\n- task\n  note\n");
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    ops::make_block(&mut v, t).unwrap();
    let block_path = v.tree.files[1].path.clone();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let texts: Vec<&str> = buf.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["# A", "", "- task", "  note"]);
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    buf.delete_line(i);
    buf.save_all(&mut v).unwrap();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let shown = fold_core::render(&v.tree, a, 1, true);
    let files: Vec<(&str, &str)> = v
        .tree
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.text.as_str()))
        .collect();
    assert!(
        shown.contains("note") && !shown.contains("![["),
        "shown:\n{}\nfiles: {:?}",
        shown,
        files
    );
    assert_eq!(v.tree.files[0].text, "# A\n\n  note\n");
    // the block's file went to the trash (§11.5)
    assert!(!d.path().join(&block_path).exists());
    assert_eq!(v.tree.files.len(), 1);
    assert!(buf.dirty.is_empty(), "{:?}", buf.dirty);
}

#[test]
fn deleted_block_title_hands_nested_blocks_to_the_enclosing_block() {
    // the block nested in the deleted one survives, embedded where its
    // title line sits, now in the enclosing block's text
    let (_d, mut v) = vault_with("# A\n\n## B\n\nb text\n\n### C\n\nc\n");
    let c = v.find_by_path(&["A".into(), "B".into(), "C".into()]).unwrap();
    let c_id = ops::make_block(&mut v, c).unwrap();
    let b = v.find_by_path(&["A".into(), "B".into()]).unwrap();
    ops::make_block(&mut v, b).unwrap();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let texts: Vec<&str> = buf.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["# A", "", "## B", "", "b text", "", "### C", "", "c"]);
    buf.delete_line(2);
    buf.delete_line(2);
    buf.save_all(&mut v).unwrap();
    // B's other lines are A's; the deeper heading C was never B's title
    assert_eq!(
        v.tree.files[0].text,
        format!("# A\n\nb text\n\n### ![[{}]]\n", c_id.as_str())
    );
    assert_eq!(v.tree.files.len(), 2);
    let a = v.tree.resolved_children(v.tree.root)[0];
    let shown = fold_core::render(&v.tree, a, 1, true);
    assert!(
        shown.starts_with("# A\n\nb text\n\n#") && shown.ends_with(" C\n\nc\n") && !shown.contains("![["),
        "{}",
        shown
    );
}

#[test]
fn line_typed_above_nested_block_title_is_the_enclosing_blocks() {
    // Enter at column 0 of a nested block's title line opens a line above
    // it with the block's tag; what is typed there sits above the block, in
    // its parent (§5.2: lines no longer with the title line are re-tagged
    // to the block they sit in), not before the block file's root (§4.9).
    let (_d, mut v) = vault_with("# A\n\n- task\n");
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = ops::make_block(&mut v, t).unwrap();
    let block_before = v.tree.files[1].text.clone();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    buf.set_line(2, String::new());
    buf.insert_line(2, "- task".into());
    buf.set_line(2, "Intro".into());
    assert_eq!(buf.lines[2].owner.file, 1);
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, format!("# A\n\nIntro\n![[{}]]\n", id.as_str()));
    assert_eq!(v.tree.files[1].text, block_before);
    assert_eq!(buf.lines[2].owner.file, 0);
    assert!(buf.dirty.is_empty(), "{:?}", buf.dirty);
}

#[test]
fn emptied_nested_block_title_is_not_a_deleted_one() {
    // vim `cc` on a nested block's title line, then a pause (autosave): the
    // title is being retyped, so the block is not deleted; it is not written
    // without its root either, and saves once it has a title again.
    let (_d, mut v) = vault_with("# A\n\n- task\n  note\n");
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = ops::make_block(&mut v, t).unwrap();
    let root_before = v.tree.files[0].text.clone();
    let block_before = v.tree.files[1].text.clone();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    buf.set_line(2, String::new());
    assert!(buf.save_all(&mut v).is_err());
    assert_eq!(v.tree.files[0].text, root_before);
    assert_eq!(v.tree.files[1].text, block_before);
    buf.set_line(2, "- renamed".into());
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, root_before);
    assert_eq!(
        v.tree.files[1].text,
        format!("---\nid: {}\n---\n\n- renamed\n  note\n", id.as_str())
    );
}

#[test]
fn deleting_all_lines_of_nested_block_clears_dirty() {
    // §5.2: deleting a nested block's title line deletes the block (its
    // embed leaves the parent, its file goes to trash). The save must not
    // leave its owner dirty forever with nothing written.
    let (_d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    assert!(v.tree.files[0].text.contains("![["), "{}", v.tree.files[0].text);
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    buf.delete_line(i);
    assert_eq!(buf.save_all(&mut v).unwrap(), 1);
    assert!(buf.dirty.is_empty(), "still dirty after save: {:?}", buf.dirty);
    assert!(!v.tree.files[0].text.contains("![["), "{}", v.tree.files[0].text);
    assert_eq!(v.tree.files.len(), 1);
    // a block with nothing at all left to write (every line of the edited
    // node gone) writes nothing, is not counted as saved, and is not left
    // dirty to be saved again on every pause
    let (_d, mut v) = vault_with("# A\n\nbody\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    while !buf.lines.is_empty() {
        buf.delete_line(0);
    }
    assert_eq!(buf.save_all(&mut v).unwrap(), 0);
    assert!(buf.dirty.is_empty(), "still dirty after save: {:?}", buf.dirty);
    assert_eq!(v.tree.files[0].text, "# A\n\nbody\n");
}

#[test]
fn one_refused_block_does_not_block_other_saves() {
    // §5.2: each dirty block is written by its own splice. A block whose file
    // changed on disk is refused and stays dirty; the other dirty blocks are
    // still written.
    let (d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    // edit the nested block first, then root.md's heading: dirty = [block, root]
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    assert_eq!(buf.lines[i].owner.file, 1);
    buf.set_line(i, "- task (mine)".into());
    assert_eq!(buf.lines[0].owner.file, 0);
    buf.set_line(0, "# A renamed".into());
    // another editor changes the block file meanwhile
    let p = d.path().join(&v.tree.files[1].path);
    let s = std::fs::read_to_string(&p).unwrap().replace("- task", "- task (theirs)");
    std::fs::write(&p, &s).unwrap();
    let err = buf.save_all(&mut v).unwrap_err();
    assert!(err.to_string().contains("changed on disk"), "{}", err);
    // the refused block is not overwritten and stays dirty
    assert_eq!(std::fs::read_to_string(&p).unwrap(), s);
    assert!(buf.dirty.iter().any(|o| o.file == 1));
    // root.md's edit is written anyway
    let disk = std::fs::read_to_string(d.path().join("root.md")).unwrap();
    assert!(disk.starts_with("# A renamed"), "{}", disk);
    assert!(v.tree.files[0].text.starts_with("# A renamed"), "{}", v.tree.files[0].text);
    assert!(!buf.dirty.iter().any(|o| o.file == 0));
}

#[test]
fn a_save_goes_through_when_the_file_changed_only_outside_the_block() {
    // §5.2 step 5 compares the block's source span, not the whole file:
    // a sync that changed another section of root.md (and moved this one
    // down) while the user typed does not refuse the save, and both
    // changes are kept
    let (d, mut v) = vault_with("# A\n\nbody\n\n# B\n\nother\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let synced = "intro\n\n# A\n\nbody\n\n# B\n\nother, from Helix\n";
    std::fs::write(d.path().join("root.md"), synced).unwrap();
    let i = buf.lines.iter().position(|l| l.text == "body").unwrap();
    buf.set_line(i, "mine".into());
    assert_eq!(buf.save_all(&mut v).unwrap(), 1);
    let disk = std::fs::read_to_string(d.path().join("root.md")).unwrap();
    assert_eq!(disk, "intro\n\n# A\n\nmine\n\n# B\n\nother, from Helix\n");
    assert_eq!(v.tree.files[0].text, disk);
    assert!(buf.dirty.is_empty());
    // the next save starts from what this one wrote
    buf.set_line(i, "mine again".into());
    assert_eq!(buf.save_all(&mut v).unwrap(), 1);
    let disk = std::fs::read_to_string(d.path().join("root.md")).unwrap();
    assert_eq!(disk, "intro\n\n# A\n\nmine again\n\n# B\n\nother, from Helix\n");
    // a change to the block's own text is still refused (§5.2 step 5)
    std::fs::write(d.path().join("root.md"), disk.replace("mine again", "theirs")).unwrap();
    buf.set_line(i, "mine, third".into());
    let err = buf.save_all(&mut v).unwrap_err();
    assert!(err.to_string().contains("changed on disk"), "{}", err);
    assert!(std::fs::read_to_string(d.path().join("root.md")).unwrap().contains("theirs"));
}

#[test]
fn a_block_save_keeps_a_property_set_on_disk_meanwhile() {
    // a block's source span is its file after the frontmatter: a property
    // another device set while the block was edited stays
    let (d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let p = d.path().join(&v.tree.files[1].path);
    let theirs = std::fs::read_to_string(&p).unwrap().replace("\n---\n", "\ndue: 2026-10-01\n---\n");
    std::fs::write(&p, &theirs).unwrap();
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    buf.set_line(i, "- task (mine)".into());
    assert_eq!(buf.save_all(&mut v).unwrap(), 1);
    let disk = std::fs::read_to_string(&p).unwrap();
    assert_eq!(disk, theirs.replace("- task", "- task (mine)"));
    assert!(disk.contains("due: 2026-10-01\n---\n\n- task (mine)\n"), "{}", disk);
}

#[test]
fn a_held_block_title_is_in_transit_not_deleted() {
    // §5.2: cutting a nested block's title line and pasting it moves the
    // block. While the editor's clipboard holds the line, a save writes the
    // parent without the embed and leaves the block and its file alone; put
    // back, the block is embedded where it now sits; released without being
    // put back, it is deleted like any block whose title line is.
    let (d, mut v) = vault_with("# A\n\n- one\n- task\n- two\n");
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = ops::make_block(&mut v, t).unwrap();
    let block_path = v.tree.files[1].path.clone();
    let block_before = v.tree.files[1].text.clone();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    let cut = buf.lines[i].clone();
    buf.delete_line(i);
    buf.hold(vec![cut.owner]);
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, "# A\n\n- one\n- two\n");
    assert_eq!(std::fs::read_to_string(d.path().join(&block_path)).unwrap(), block_before);
    // pasted below "- two" with its tag: the block and its parent are dirty
    let two = buf.lines.iter().position(|l| l.text == "- two").unwrap();
    buf.lines.insert(two + 1, cut.clone());
    buf.mark_dirty(cut.owner);
    buf.mark_dirty(buf.lines[0].owner);
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, format!("# A\n\n- one\n- two\n![[{}]]\n", id.as_str()));
    assert_eq!(std::fs::read_to_string(d.path().join(&block_path)).unwrap(), block_before);
    // cut again and saved, then the clipboard replaced: the block is deleted
    buf.delete_line(two + 1);
    buf.save_all(&mut v).unwrap();
    assert!(d.path().join(&block_path).exists());
    buf.hold(Vec::new());
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, "# A\n\n- one\n- two\n");
    assert!(!d.path().join(&block_path).exists());
    assert!(buf.dirty.is_empty(), "{:?}", buf.dirty);
}

#[test]
fn a_held_block_still_in_the_buffer_is_not_in_transit() {
    // the clipboard holding a copy of a block's title line keeps the block
    // only while nothing of it is in the buffer; a title line re-spelled in
    // place, or deleted while a block nested in it stays, deletes the block
    // (§5.2) at once
    let (d, mut v) = vault_with("# A\n\n- one\n- task\n");
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    ops::make_block(&mut v, t).unwrap();
    let block_path = v.tree.files[1].path.clone();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    buf.hold(vec![buf.lines[i].owner]);
    buf.set_line(i, "task".into());
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, "# A\n\n- one\ntask\n");
    assert!(!d.path().join(&block_path).exists());

    let (d, mut v) = vault_with("# A\n\n- one\n- b\n  - c\n- two\n");
    let c = v.find_by_path(&["A".into(), "b".into(), "c".into()]).unwrap();
    let c = ops::make_block(&mut v, c).unwrap();
    let b = v.find_by_path(&["A".into(), "b".into()]).unwrap();
    ops::make_block(&mut v, b).unwrap();
    let b_path = v.tree.files.iter().find(|f| f.text.contains(&format!("![[{}]]", c.as_str()))).unwrap().path.clone();
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let i = buf.lines.iter().position(|l| l.text == "- b").unwrap();
    buf.hold(vec![buf.lines[i].owner]);
    buf.delete_line(i);
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, format!("# A\n\n- one\n  ![[{}]]\n- two\n", c.as_str()));
    assert!(!d.path().join(&b_path).exists());
}

#[test]
fn review_external_edit_of_the_zoomed_node_never_lands_on_an_identical_one() {
    // §5.2 step 5: the zoomed item's own text changed on disk (another
    // device edited it) while it was edited here. Its old bytes still occur
    // once in the file — as an identical item under another section — but
    // that is another node: the save must be refused, not written over it.
    let (d, mut v) = vault_with("# X\n\n- call mom\n\n# Y\n\n- call mom\n");
    let x_item = v.find_by_path(&["X".into(), "call mom".into()]).unwrap();
    let mut buf = open_editor(&v, x_item);
    assert_eq!(buf.lines[0].text, "- call mom");
    let theirs = "# X\n\n- call mom (phone)\n\n# Y\n\n- call mom\n";
    std::fs::write(d.path().join("root.md"), theirs).unwrap();
    buf.set_line(0, "- call mom tomorrow".into());
    let res = buf.save_all(&mut v);
    let disk = std::fs::read_to_string(d.path().join("root.md")).unwrap();
    assert!(
        res.is_err() && disk == theirs,
        "save {:?}; Y's item overwritten:\n{}",
        res.map_err(|e| e.to_string()),
        disk
    );
    // changes elsewhere, one of them an identical item added above, still
    // let the save through, onto the zoomed node
    let (d, mut v) = vault_with("# X\n\n- call mom\n\n# Y\n\n- call mom\n");
    let x_item = v.find_by_path(&["X".into(), "call mom".into()]).unwrap();
    let mut buf = open_editor(&v, x_item);
    let theirs = "# W\n\n- call mom\n\n# X\n\n- call mom\n\n# Y\n\n- call mom, from phone\n";
    std::fs::write(d.path().join("root.md"), theirs).unwrap();
    buf.set_line(0, "- call mom tomorrow".into());
    assert_eq!(buf.save_all(&mut v).unwrap(), 1);
    assert_eq!(
        std::fs::read_to_string(d.path().join("root.md")).unwrap(),
        "# W\n\n- call mom\n\n# X\n\n- call mom tomorrow\n\n# Y\n\n- call mom, from phone\n"
    );
}

#[test]
fn review_undoing_an_editor_save_leaves_another_files_sync_alone() {
    // §10.10: an op-log entry holds exactly the files the operation
    // changed. An editor save that deletes a nested block trashes its file,
    // and the trash reloads every file: a sync to another block's file not
    // reloaded yet must not become part of the save's entry, or undoing
    // the save writes that file's old text back over the sync
    let (d, mut v) = vault_with("# A\n\n- task\n\n# N\n\n- note\n");
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    ops::make_block(&mut v, t).unwrap();
    let n = v.find_by_path(&["N".into(), "note".into()]).unwrap();
    let note_id = ops::make_block(&mut v, n).unwrap();
    let note = d.path().join(&v.tree.files[v.tree.block_by_id(&note_id).unwrap().0].path);
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    buf.delete_line(i);
    let synced = std::fs::read_to_string(&note).unwrap().replace("- note", "- note, from phone");
    std::fs::write(&note, &synced).unwrap();
    // what the app does on a save (§10.10)
    buf.rebase_dirty(&mut v);
    let snap = ops::Snapshot::take(&v, "edit");
    buf.save_all(&mut v).unwrap();
    let inv = ops::Inverse::since(snap, &v).unwrap();
    inv.undo(&mut v).unwrap();
    assert_eq!(
        std::fs::read_to_string(&note).unwrap(),
        synced,
        "undoing the save reverted another device's change to a file it never touched"
    );
}

#[test]
fn review_a_block_is_not_trashed_while_the_file_on_disk_embeds_it() {
    // A block cut in the editor is in transit: the parent is saved without
    // its embed. Another device's root.md, which still embeds it, then
    // lands (§12.1). Releasing the clipboard deletes the block only once no
    // file embeds it (§5.2): the parent's save is refused (changed on
    // disk), so root.md on disk still embeds the block and its file must
    // stay, not go to the trash leaving that embed broken
    let (d, mut v) = vault_with("# A\n\n- one\n- task\n- two\n");
    let t = v.find_by_path(&["A".into(), "task".into()]).unwrap();
    let id = ops::make_block(&mut v, t).unwrap();
    let block = d.path().join(&v.tree.files[1].path);
    let a = v.tree.resolved_children(v.tree.root)[0];
    let mut buf = open_editor(&v, a);
    let i = buf.lines.iter().position(|l| l.text == "- task").unwrap();
    let cut = buf.lines[i].owner;
    buf.delete_line(i);
    buf.hold(vec![cut]);
    buf.save_all(&mut v).unwrap();
    assert_eq!(v.tree.files[0].text, "# A\n\n- one\n- two\n");
    assert!(block.exists(), "in transit");
    let theirs = format!("# A\n\n- one\n![[{}]]\n- two, from phone\n", id.as_str());
    std::fs::write(d.path().join("root.md"), &theirs).unwrap();
    buf.hold(Vec::new());
    let res = buf.save_all(&mut v);
    assert_eq!(std::fs::read_to_string(d.path().join("root.md")).unwrap(), theirs);
    assert!(
        block.exists(),
        "save {:?}: root.md embeds {} but its file went to the trash",
        res.map_err(|e| e.to_string()),
        id.as_str()
    );
}
