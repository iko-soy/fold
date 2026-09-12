use notes_core::ops;
use notes_core::vault::Vault;
use notes_core::TaskState;

fn vault_with(root: &str) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), root).unwrap();
    let v = Vault::open(dir.path()).unwrap();
    (dir, v)
}

#[test]
fn capture_creates_day_and_item() {
    let (_d, mut v) = vault_with("# Inbox\n");
    let r = ops::capture(&mut v, "hello world", false).unwrap();
    assert_eq!(v.tree.node(r).title, "hello world");
    let day = v.tree.ancestors(r).last().copied().unwrap();
    let day_title = &v.tree.node(day).title;
    assert!(day_title.chars().take(4).all(|c| c.is_ascii_digit()));
    let text = &v.tree.files[0].text;
    assert!(text.contains("- hello world"), "{}", text);
    // second capture lands under the same day
    let r2 = ops::capture(&mut v, "second", true).unwrap();
    assert_eq!(v.tree.node(r2).task, Some(TaskState::Open));
}

#[test]
fn capture_creates_inbox_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# Projects\n").unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    ops::capture(&mut v, "x", false).unwrap();
    assert!(v.tree.files[0].text.contains("# Inbox"));
}

#[test]
fn toggle_checkbox_task() {
    let (_d, mut v) = vault_with("# A\n\n- [ ] do it\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::toggle_task(&mut v, task).unwrap();
    assert!(v.tree.files[0].text.contains("- [x] do it"));
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::toggle_task(&mut v, task).unwrap();
    assert!(v.tree.files[0].text.contains("- [ ] do it"));
}

#[test]
fn make_block_writes_file_and_embed() {
    let (_d, mut v) = vault_with("# A\n\n- [ ] Order new switch\n  Two options.\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    let id = ops::make_block(&mut v, task).unwrap();
    // root.md now has an embed
    assert!(v.tree.files[0].text.contains(&format!("![[{}]]", id)));
    assert!(!v.tree.files[0].text.contains("Order new switch"));
    // the block file round-trips
    assert_eq!(v.tree.files.len(), 2);
    let bf = &v.tree.files[1];
    assert!(bf.text.starts_with(&format!("---\nid: {}\ntodo: open\n---\n\n", id)));
    assert!(bf.text.contains("- Order new switch"), "{}", bf.text);
    assert!(bf.text.contains("Two options."), "{}", bf.text);
    assert!(!bf.text.contains("[ ]"));
    // filename: <prefix>~<name>.md
    assert!(bf.path.ends_with("~order-new-switch.md"), "{}", bf.path);
}

#[test]
fn task_block_toggle_stamps_done() {
    let (_d, mut v) = vault_with("# A\n\n- [ ] Order new switch\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    let embed = v.tree.resolved_children(v.tree.resolved_children(v.tree.root)[0])[0];
    let block = v.tree.resolved_child(embed);
    ops::toggle_task(&mut v, block).unwrap();
    let bf = &v.tree.files[1];
    assert!(bf.text.contains("todo: done"), "{}", bf.text);
    assert!(bf.text.contains("done: 20"), "{}", bf.text);
}

#[test]
fn set_property_makes_block() {
    let (_d, mut v) = vault_with("# A\n\n- plain note\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let note = v.tree.resolved_children(a)[0];
    ops::set_property(&mut v, note, "due", "2026-09-20").unwrap();
    assert_eq!(v.tree.files.len(), 2);
    let bf = &v.tree.files[1];
    assert!(bf.text.contains("due: 2026-09-20"), "{}", bf.text);
    assert!(bf.text.contains("id: "), "{}", bf.text);
}

#[test]
fn refile_moves_subtree() {
    let (_d, mut v) = vault_with("# A\n\n- move me\n  - child\n\n# B\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let b = v.tree.resolved_children(v.tree.root)[1];
    let node = v.tree.resolved_children(a)[0];
    ops::refile(&mut v, node, b).unwrap();
    let text = &v.tree.files[0].text;
    let b_pos = text.find("# B").unwrap();
    let move_pos = text.find("- move me").unwrap();
    assert!(move_pos > b_pos, "{}", text);
    // the subtree comes along, children still under it
    assert!(text.contains("- move me"), "{}", text);
    let b2 = v.tree.resolved_children(v.tree.root)[1];
    let moved = v.tree.resolved_children(b2)[0];
    assert_eq!(v.tree.node(moved).title, "move me");
    assert_eq!(v.tree.resolved_children(moved).len(), 1);
    assert_eq!(v.tree.node(v.tree.resolved_children(moved)[0]).title, "child");
    // nothing under A anymore
    let a = v.tree.resolved_children(v.tree.root)[0];
    assert_eq!(v.tree.resolved_children(a).len(), 0);
}

#[test]
fn refile_block_moves_only_embed() {
    let (_d, mut v) = vault_with("# A\n\n- [ ] task\n\n# B\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    let id = ops::make_block(&mut v, task).unwrap();
    let b = v.tree.resolved_children(v.tree.root)[1];
    let embed = v.tree.resolved_children(v.tree.resolved_children(v.tree.root)[0])[0];
    ops::refile(&mut v, embed, b).unwrap();
    let text = &v.tree.files[0].text;
    let b_pos = text.find("# B").unwrap();
    let embed_pos = text.find("![[").unwrap();
    assert!(embed_pos > b_pos, "{}", text);
    // block file untouched
    assert!(v.tree.files[1].text.contains(&id.to_string()));
}

#[test]
fn archive_creates_section() {
    let (_d, mut v) = vault_with("# A\n\n- old stuff\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let node = v.tree.resolved_children(a)[0];
    ops::archive(&mut v, node).unwrap();
    let text = &v.tree.files[0].text;
    assert!(text.contains("# Archive"));
    let arch_pos = text.find("# Archive").unwrap();
    let node_pos = text.find("- old stuff").unwrap();
    assert!(node_pos > arch_pos);
}

#[test]
fn clear_done_trashes_completed() {
    let (_d, mut v) = vault_with("# A\n\n- [x] done one\n- [ ] still open\n- [x] done two\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let n = ops::clear_done(&mut v, a).unwrap();
    assert_eq!(n, 2);
    let text = &v.tree.files[0].text;
    assert!(!text.contains("done one"));
    assert!(!text.contains("done two"));
    assert!(text.contains("still open"));
}

#[test]
fn delete_goes_to_trash() {
    let (d, mut v) = vault_with("# A\n\n- delete me\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let node = v.tree.resolved_children(a)[0];
    ops::delete_subtree(&mut v, node).unwrap();
    assert!(!v.tree.files[0].text.contains("delete me"));
    let _ = d;
    // trash dir has a copy
    let trash = notes_core::vault::trash_dir();
    let entries: Vec<_> = std::fs::read_dir(&trash)
        .map(|rd| rd.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();
    assert!(entries.iter().any(|e| e
        .file_name()
        .to_string_lossy()
        .contains("delete-me")));
}

#[test]
fn move_sibling_swaps() {
    let (_d, mut v) = vault_with("# A\n\n- one\n- two\n- three\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let two = v.tree.resolved_children(a)[1];
    ops::move_sibling(&mut v, two, false).unwrap();
    let text = &v.tree.files[0].text;
    let one_pos = text.find("- one").unwrap();
    let two_pos = text.find("- two").unwrap();
    assert!(two_pos < one_pos, "{}", text);
}

#[test]
fn toggle_spelling_works() {
    let (_d, mut v) = vault_with("# A\n\n- item node\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let node = v.tree.resolved_children(a)[0];
    ops::toggle_spelling(&mut v, node).unwrap();
    assert!(v.tree.files[0].text.contains("## item node"), "{}", v.tree.files[0].text);
    let a = v.tree.resolved_children(v.tree.root)[0];
    let node = v.tree.resolved_children(a)[0];
    ops::toggle_spelling(&mut v, node).unwrap();
    assert!(v.tree.files[0].text.contains("- item node"), "{}", v.tree.files[0].text);
}

#[test]
fn demote_and_promote() {
    let (_d, mut v) = vault_with("# A\n\n- one\n- two\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let two = v.tree.resolved_children(a)[1];
    ops::demote(&mut v, two).unwrap();
    let text = &v.tree.files[0].text;
    assert!(text.contains("- one\n\n  - two") || text.contains("- one\n  - two"), "{}", text);
    let a = v.tree.resolved_children(v.tree.root)[0];
    let one = v.tree.resolved_children(a)[0];
    let two = v.tree.resolved_children(one)[0];
    ops::promote(&mut v, two).unwrap();
    let text = &v.tree.files[0].text;
    assert!(text.contains("- one\n\n- two") || text.contains("- one\n- two"), "{}", text);
}

#[test]
fn rename_block_title_renames_file() {
    let (_d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    let embed = v.tree.resolved_children(v.tree.resolved_children(v.tree.root)[0])[0];
    let block = v.tree.resolved_child(embed);
    ops::rename_title(&mut v, block, "renamed title").unwrap();
    assert_eq!(v.tree.files.len(), 2);
    assert!(v.tree.files[1].path.ends_with("~renamed-title.md"), "{}", v.tree.files[1].path);
    assert!(v.tree.files[1].text.contains("- renamed title"));
}

#[test]
fn prefix_collision_grows_prefix() {
    let (d, _v) = vault_with("# A\n");
    // two ids sharing the first word
    let id1 = notes_core::Id::parse("racfer-hattes-mislup-nodrys").unwrap();
    let id2 = notes_core::Id::parse("racfer-wolsun-dozzod-binwes").unwrap();
    std::fs::write(
        d.path().join("racfer~notes.md"),
        format!("---\nid: {}\n---\n\n# Notes\n", id1),
    )
    .unwrap();
    let mut v = Vault::open(d.path()).unwrap();
    let prefix = v.unique_prefix(&id2);
    assert_eq!(prefix, "racfer-wolsun");
}

#[test]
fn frontmatter_key_edits_preserve_unknown_lines() {
    let (d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    let id = ops::make_block(&mut v, task).unwrap();
    // hand-edit the file to add unknown structure
    let path = d.path().join(format!("{}~task.md", id.words()[0]));
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text.replacen("---\n\n", "---\ntags: [a, b]   # comment\nnested:\n  deep: true\n---\n\n", 1);
    std::fs::write(&path, &text).unwrap();
    v.reload().unwrap();
    let embed = v.tree.resolved_children(v.tree.resolved_children(v.tree.root)[0])[0];
    let block = v.tree.resolved_child(embed);
    ops::set_frontmatter_key(&mut v, block.0, "due", Some("2026-09-20")).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("tags: [a, b]   # comment"), "{}", after);
    assert!(after.contains("nested:\n  deep: true"), "{}", after);
    assert!(after.contains("due: 2026-09-20"), "{}", after);
    // remove it again
    let embed = v.tree.resolved_children(v.tree.resolved_children(v.tree.root)[0])[0];
    let block = v.tree.resolved_child(embed);
    ops::set_frontmatter_key(&mut v, block.0, "due", None).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(!after.contains("due:"), "{}", after);
}

#[test]
fn paste_inserts_siblings() {
    let (_d, mut v) = vault_with("# A\n\n- one\n- three\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let one = v.tree.resolved_children(a)[0];
    ops::paste(&mut v, one, "- two\n", true).unwrap();
    let text = &v.tree.files[0].text;
    let one_pos = text.find("- one").unwrap();
    let two_pos = text.find("- two").unwrap();
    let three_pos = text.find("- three").unwrap();
    assert!(one_pos < two_pos && two_pos < three_pos, "{}", text);
}
