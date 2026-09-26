use fold_core::ops;
use fold_core::vault::Vault;
use fold_core::TaskState;

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
    // the checkbox stays on the title line: it is the state (§4.5)
    assert!(bf.text.starts_with(&format!("---\nid: {}\n---\n\n", id)), "{}", bf.text);
    assert!(bf.text.contains("- [ ] Order new switch"), "{}", bf.text);
    assert!(bf.text.contains("Two options."), "{}", bf.text);
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
    assert!(bf.text.contains("- [x] Order new switch"), "{}", bf.text);
    assert!(bf.text.contains("done: 20"), "{}", bf.text);
    ops::toggle_task(&mut v, block).unwrap();
    let bf = &v.tree.files[1];
    assert!(bf.text.contains("- [ ] Order new switch"), "{}", bf.text);
    assert!(!bf.text.contains("done:"), "{}", bf.text);
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
    let trash = fold_core::vault::trash_dir();
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
fn rename_block_title_keeps_the_filename_until_fix() {
    let (_d, mut v) = vault_with("# A\n\n- task\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    let task = v.tree.resolved_children(a)[0];
    ops::make_block(&mut v, task).unwrap();
    let embed = v.tree.resolved_children(v.tree.resolved_children(v.tree.root)[0])[0];
    let block = v.tree.resolved_child(embed);
    let old_path = v.tree.files[1].path.clone();
    let _ = block;
    // retitle it the way any editor would: by changing the title line
    let path = v.dir.join(&old_path);
    let text = std::fs::read_to_string(&path).unwrap().replace("- task", "- renamed title");
    std::fs::write(&path, text).unwrap();
    v.reload().unwrap();
    assert_eq!(v.tree.files.len(), 2);
    // names are set once (§6.4): the file keeps its name ...
    assert_eq!(v.tree.files[1].path, old_path);
    assert!(v.tree.files[1].text.contains("- renamed title"));
    // ... check calls it stale, and --fix renames it on request
    assert!(fold_core::check::check(&v).iter().any(|d| d.message.contains("does not match the title")));
    fold_core::check::fix(&mut v).unwrap();
    assert!(v.tree.files[1].path.ends_with("~renamed-title.md"), "{}", v.tree.files[1].path);
}

#[test]
fn prefix_collision_grows_prefix() {
    let (d, _v) = vault_with("# A\n");
    // two ids sharing the first word
    let id1 = fold_core::Id::parse("racfer-hattes-mislup-nodrys").unwrap();
    let id2 = fold_core::Id::parse("racfer-wolsun-dozzod-binwes").unwrap();
    std::fs::write(
        d.path().join("racfer~notes.md"),
        format!("---\nid: {}\n---\n\n# Notes\n", id1),
    )
    .unwrap();
    let v = Vault::open(d.path()).unwrap();
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

// ------------------------------------------------------------ regressions

const ID_A: &str = "racfer-hattes-dozzod-binwes";
const ID_B: &str = "dozzod-binwes-talsun-worbec";

fn vault_files(root: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), root).unwrap();
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let v = Vault::open(dir.path()).unwrap();
    (dir, v)
}

fn at(v: &Vault, path: &[&str]) -> fold_core::tree::NRef {
    let segs: Vec<String> = path.iter().map(|s| s.to_string()).collect();
    v.find_by_path(&segs).unwrap()
}

fn read(d: &tempfile::TempDir, name: &str) -> String {
    std::fs::read_to_string(d.path().join(name)).unwrap()
}

#[test]
fn non_ascii_id_does_not_panic() {
    let (_d, v) = vault_files(
        "# A\n\n![[ééaa-racfer-hattes-dozzod]]\n",
        &[("x.md", "---\nid: ééé-ééé-ééé-ééé\n---\n\n# X\n")],
    );
    assert_eq!(v.tree.files.len(), 1);
    assert!(v.resolve_target("ééé").is_err());
}

#[test]
fn unreadable_and_dot_md_files_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n").unwrap();
    std::fs::create_dir(dir.path().join("attachments.md")).unwrap();
    std::fs::write(dir.path().join("latin1.md"), [0x63u8, 0x61, 0x66, 0xe9, b'\n']).unwrap();
    std::fs::write(
        dir.path().join(".hidden.md"),
        format!("---\nid: {}\n---\n\n# Hidden\n", ID_A),
    )
    .unwrap();
    let v = Vault::open(dir.path()).unwrap();
    assert_eq!(v.tree.files.len(), 1);
    assert!(v.ignored_files().unwrap().iter().any(|i| i.path == "latin1.md"));
}

#[test]
fn trash_copies_in_the_same_second_do_not_collide() {
    let (_d, v) = vault_with("# A\n");
    let a = v.trash_text("same.md", "one").unwrap();
    let b = v.trash_text("same.md", "two").unwrap();
    assert_ne!(a, b);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "one");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "two");
}

#[test]
fn clear_done_with_duplicate_titles_keeps_the_open_one() {
    let (_d, mut v) = vault_with("# P\n\n- [ ] Water plants\n- [x] Water plants\n");
    let root = v.tree.root;
    assert_eq!(ops::clear_done(&mut v, root).unwrap(), 1);
    assert_eq!(v.tree.files[0].text, "# P\n\n- [ ] Water plants\n");
}

#[test]
fn delete_after_multibyte_text_does_not_panic() {
    let (_d, mut v) = vault_with("- café\n- other\n");
    let other = at(&v, &["other"]);
    ops::delete_subtree(&mut v, other).unwrap();
    assert_eq!(v.tree.files[0].text, "- café\n");
}

#[test]
fn delete_first_child_keeps_the_separator_after_the_title() {
    let (_d, mut v) = vault_with("# A\n\n- x\n- y\n");
    let x = at(&v, &["A", "x"]);
    ops::delete_subtree(&mut v, x).unwrap();
    assert_eq!(v.tree.files[0].text, "# A\n\n- y\n");
}

#[test]
fn refile_into_own_subtree_through_an_embed_is_refused() {
    let (d, mut v) = vault_files(
        &format!("# A\n\nbody of A\n\n![[{}]]\n\n# B\n", ID_A),
        &[("racfer~blk.md", &format!("---\nid: {}\n---\n\n- blk\n  - inner\n", ID_A))],
    );
    let a = at(&v, &["A"]);
    let inner = at(&v, &["A", "blk", "inner"]);
    assert!(ops::refile(&mut v, a, inner).is_err());
    assert!(read(&d, "root.md").contains("body of A"));
}

#[test]
fn set_property_inside_a_block_file_that_sorts_late() {
    let (d, mut v) = vault_files(
        &format!("# A\n\n![[{}]]\n", ID_A),
        &[(
            "zzzz~notes.md",
            &format!("---\nid: {}\n---\n\n- notes\n  - one\n  - three\n", ID_A),
        )],
    );
    let three = at(&v, &["A", "notes", "three"]);
    ops::set_property(&mut v, three, "due", "2026-10-01").unwrap();
    let three = at(&v, &["A", "notes", "three"]);
    let b = v.tree.node(three).block.as_ref().unwrap();
    assert_eq!(b.prop("due"), Some("2026-10-01"));
    assert!(!read(&d, "zzzz~notes.md").contains("three"));
}

#[test]
fn promote_out_of_a_block_root_goes_beside_the_embed() {
    let (d, mut v) = vault_files(
        &format!("# A\n\n- x\n![[{}]]\n- y\n", ID_A),
        &[("racfer~blk.md", &format!("---\nid: {}\n---\n\n- blk\n  - child\n", ID_A))],
    );
    let child = at(&v, &["A", "blk", "child"]);
    ops::promote(&mut v, child).unwrap();
    assert_eq!(read(&d, "racfer~blk.md"), format!("---\nid: {}\n---\n\n- blk\n", ID_A));
    assert_eq!(read(&d, "root.md"), format!("# A\n\n- x\n![[{}]]\n- child\n- y\n", ID_A));
}

#[test]
fn move_sibling_swaps_with_an_embed() {
    let (d, mut v) = vault_files(
        &format!("- a\n![[{}]]\n", ID_A),
        &[("racfer~b.md", &format!("---\nid: {}\n---\n\n- b\n", ID_A))],
    );
    let a = at(&v, &["a"]);
    ops::move_sibling(&mut v, a, true).unwrap();
    assert_eq!(read(&d, "root.md"), format!("![[{}]]\n- a\n", ID_A));
    // and back, starting from the block itself
    let b = at(&v, &["b"]);
    ops::move_sibling(&mut v, b, true).unwrap();
    assert_eq!(read(&d, "root.md"), format!("- a\n![[{}]]\n", ID_A));
}

#[test]
fn move_sibling_without_final_newline() {
    let (_d, mut v) = vault_with("- a\n- b");
    let b = at(&v, &["b"]);
    ops::move_sibling(&mut v, b, false).unwrap();
    assert_eq!(v.tree.files[0].text, "- b\n- a\n");
}

#[test]
fn move_sibling_keeps_list_spacing() {
    let (_d, mut v) = vault_with("# S\n\n- a\n- b\n\n# T\n");
    let b = at(&v, &["S", "b"]);
    ops::move_sibling(&mut v, b, false).unwrap();
    assert_eq!(v.tree.files[0].text, "# S\n\n- b\n- a\n\n# T\n");
}

#[test]
fn toggle_taskness_on_an_indented_heading() {
    let (_d, mut v) = vault_with("- item\n  ## Sub\n");
    let sub = at(&v, &["item", "Sub"]);
    ops::toggle_taskness(&mut v, sub).unwrap();
    assert_eq!(v.tree.files[0].text, "- item\n  ## [ ] Sub\n");
}

#[test]
fn toggle_task_reopens_capital_x_and_dash() {
    let (_d, mut v) = vault_with("- [X] one\n- [-] two\n");
    let one = at(&v, &["one"]);
    ops::toggle_task(&mut v, one).unwrap();
    let two = at(&v, &["two"]);
    ops::toggle_task(&mut v, two).unwrap();
    assert_eq!(v.tree.files[0].text, "- [ ] one\n- [ ] two\n");
}

#[test]
fn make_block_keeps_checkbox_like_title_text() {
    let (_d, mut v) = vault_with("# A\n\n- [ ] Review [x] marks\n");
    let t = at(&v, &["A", "Review [x] marks"]);
    ops::make_block(&mut v, t).unwrap();
    assert!(v.find_by_path(&["A".into(), "Review [x] marks".into()]).is_some());
}

#[test]
fn refile_indented_embed_is_not_double_indented() {
    let (d, mut v) = vault_files(
        &format!("- parent\n  ![[{}]]\n\n# Dest\n", ID_A),
        &[("racfer~b.md", &format!("---\nid: {}\n---\n\n- b\n", ID_A))],
    );
    let b = at(&v, &["parent", "b"]);
    let dest = at(&v, &["Dest"]);
    ops::refile(&mut v, b, dest).unwrap();
    assert_eq!(read(&d, "root.md"), format!("- parent\n\n# Dest\n\n![[{}]]\n", ID_A));
}

#[test]
fn multi_line_capture_nests_under_the_item() {
    let (_d, mut v) = vault_with("# Inbox\n");
    let r = ops::capture(&mut v, "line one\n# Injected\nsome text", false).unwrap();
    assert_eq!(v.tree.node(r).title, "line one");
    let top: Vec<String> = v
        .tree
        .resolved_children(v.tree.root)
        .iter()
        .map(|&c| v.tree.node(c).title.clone())
        .collect();
    assert_eq!(top, vec!["Inbox"]);
    let kids = v.tree.resolved_children(r);
    assert_eq!(v.tree.node(kids[0]).title, "Injected");
}

#[test]
fn targets_resolve_unicode_case_and_id_like_titles() {
    let (_d, v) = vault_with("# Заметки\n\n## Проект\n\n# dozzod\n");
    assert!(v.resolve_target("заметки/проект").is_ok());
    assert!(v.resolve_target("dozzod").is_ok());
}

#[test]
fn new_last_child_after_a_section_child_is_a_section() {
    let (_d, mut v) = vault_with("# A\n\n## B\n");
    let a = at(&v, &["A"]);
    let r = ops::append_child_public(&mut v, a, "C").unwrap();
    assert_eq!(v.tree.node(r).title, "C");
    assert_eq!(v.tree.files[0].text, "# A\n\n## B\n\n## C\n");
    let kids = v.tree.resolved_children(a);
    assert_eq!(kids.len(), 2);
}

#[test]
fn capture_to_a_node_with_sections_stays_an_item() {
    let (_d, mut v) = vault_with("# P\n\n- a\n\n## S\n");
    let p = at(&v, &["P"]);
    let r = ops::capture_to(&mut v, "new", false, p).unwrap();
    assert_eq!(v.tree.node(r).title, "new");
    assert_eq!(v.tree.files[0].text, "# P\n\n- a\n- new\n\n## S\n");
}

#[test]
fn check_fix_keeps_root_frontmatter_intro_and_adopted_nodes() {
    // a second column-0 node is adopted by the block's root (§4.9): --fix
    // writes it as the root's child and loses nothing
    let malformed = format!("---\nid: {}\n---\n\n# One\n\n# Two\n\nprecious\n", ID_A);
    let fm_only = format!("---\nid: {}\n---\n", ID_B);
    let (d, mut v) = vault_files(
        "---\nvault: x\n---\n\nIntro text.\n\n# Inbox\n",
        &[("racfer~one.md", &malformed), ("dozzod~empty.md", &fm_only)],
    );
    fold_core::check::fix(&mut v).unwrap();
    assert_eq!(read(&d, "root.md"), "---\nvault: x\n---\n\nIntro text.\n\n# Inbox\n");
    assert_eq!(
        read(&d, "racfer~one.md"),
        format!("---\nid: {}\n---\n\n# One\n\n## Two\n\nprecious\n", ID_A)
    );
}

#[test]
fn check_fix_keeps_correct_prefixes_and_is_idempotent() {
    let (d, mut v) = vault_files(
        &format!("# A\n\n![[{}]]\n", ID_A),
        &[("racfer~blk.md", &format!("---\nid: {}\n---\n\n- blk\n", ID_A))],
    );
    fold_core::check::fix(&mut v).unwrap();
    assert!(d.path().join("racfer~blk.md").exists());
    assert_eq!(fold_core::check::fix(&mut v).unwrap(), 0);
}

#[test]
fn setext_sibling_spans_do_not_overlap() {
    // the setext title line is taken back out of A's text: A's span must
    // shrink with it, or A and Title overlap
    let (_d, mut v) = vault_with("# A\n\nTitle\n===\n\nbody\n");
    let kids = v.tree.resolved_children(v.tree.root);
    assert_eq!(kids.len(), 2);
    let (a, b) = (v.tree.node(kids[0]).span, v.tree.node(kids[1]).span);
    assert!(a.end <= b.start, "A {:?} overlaps Title {:?}", a, b);
    ops::move_sibling(&mut v, kids[0], true).unwrap();
    let kids = v.tree.resolved_children(v.tree.root);
    assert_eq!(v.tree.node(kids[0]).title, "Title");
    assert_eq!(v.tree.node(kids[1]).title, "A");
}

#[test]
fn move_sibling_before_a_setext_sibling_does_not_panic() {
    let (_d, mut v) = vault_with("# A\n\nTitle\n===\n\nbody\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    ops::move_sibling(&mut v, a, true).unwrap();
    let kids = v.tree.resolved_children(v.tree.root);
    assert_eq!(v.tree.node(kids[0]).title, "Title", "{}", v.tree.files[0].text);
    assert_eq!(v.tree.node(kids[1]).title, "A", "{}", v.tree.files[0].text);
}

#[test]
fn delete_before_a_setext_sibling_keeps_its_title() {
    let (_d, mut v) = vault_with("# A\n\nTitle\n===\n\nbody\n");
    let a = v.tree.resolved_children(v.tree.root)[0];
    ops::delete_subtree(&mut v, a).unwrap();
    let text = v.tree.files[0].text.clone();
    assert!(text.contains("Title"), "{}", text);
    let top = v.tree.resolved_children(v.tree.root);
    assert_eq!(top.len(), 1, "{}", text);
    assert_eq!(v.tree.node(top[0]).title, "Title", "{}", text);
}

#[test]
fn capture_before_a_setext_sibling_keeps_it() {
    let (_d, mut v) = vault_with("# Inbox\n\nMeeting notes\n=============\n\n- action\n");
    let inbox = v.tree.resolved_children(v.tree.root)[0];
    ops::capture_to(&mut v, "new", false, inbox).unwrap();
    let text = v.tree.files[0].text.clone();
    let top = v.tree.resolved_children(v.tree.root);
    assert_eq!(top.len(), 2, "{}", text);
    assert_eq!(v.tree.node(top[1]).title, "Meeting notes", "{}", text);
    let kids = v.tree.resolved_children(top[1]);
    assert_eq!(kids.len(), 1, "{}", text);
    assert_eq!(v.tree.node(kids[0]).title, "action", "{}", text);
}

#[test]
fn archiving_keeps_hashtag_body_lines() {
    // `#done` has no space after the hash: it is body text, not a heading
    // (§4.2), so re-levelling the moved node must leave it as written
    let (_d, mut v) = vault_with("# Old project\n\n#done wrap-up notes\n");
    let r = at(&v, &["Old project"]);
    ops::archive(&mut v, r).unwrap();
    let t = &v.tree.files[0].text;
    assert!(t.contains("## Old project\n"), "{}", t);
    assert!(t.contains("\n#done wrap-up notes\n"), "{}", t);
    assert!(!t.contains("##done"), "{}", t);
}

#[test]
fn respelling_a_setext_heading_keeps_its_title() {
    // §4.2: setext headings are read, and converted on write
    let (_d, mut v) = vault_with("Title\n=====\n\nbody\n");
    let r = at(&v, &["Title"]);
    ops::toggle_spelling(&mut v, r).unwrap();
    let text = &v.tree.files[0].text;
    assert_eq!(text, "- Title\n\n  body\n");
    assert!(v.find_by_path(&["Title".into()]).is_some(), "{}", text);
}

#[test]
fn toggle_taskness_on_a_setext_heading_makes_it_a_task() {
    // §4.2, §10.3 `t`: the checkbox goes between the marker and the title
    let (_d, mut v) = vault_with("Title\n=====\n\nbody\n");
    let r = at(&v, &["Title"]);
    ops::toggle_taskness(&mut v, r).unwrap();
    let text = v.tree.files[0].text.clone();
    assert_eq!(text, "# [ ] Title\n\nbody\n");
    let r = v.find_by_path(&["Title".into()]);
    assert!(r.is_some(), "title changed: {}", text);
    assert_eq!(v.tree.node(r.unwrap()).task, Some(TaskState::Open), "{}", text);
    // and back: the checkbox goes, the ATX heading stays
    ops::toggle_taskness(&mut v, r.unwrap()).unwrap();
    assert_eq!(v.tree.files[0].text, "# Title\n\nbody\n");
}

#[test]
fn demote_keeps_a_fenced_code_body_with_its_node() {
    // b's body is a fenced code block; demoting b under a must re-indent the
    // fence with b, or the code falls out of b's region and becomes a's text
    let (d, mut v) = vault_with("- a\n- b\n  ```\n  code\n  ```\n");
    let b = at(&v, &["b"]);
    ops::demote(&mut v, b).unwrap();
    assert_eq!(read(&d, "root.md"), "- a\n  - b\n    ```\n    code\n    ```\n");
    let b = at(&v, &["a", "b"]);
    let text = v.tree.files[0].text.clone();
    assert!(
        v.tree.node(b).text_lines(&text).iter().any(|l| l.contains("code")),
        "code block is no longer b's body:\n{}",
        text
    );
}

#[test]
fn respelling_keeps_code_in_a_fence_as_written() {
    // a fence moves with its node; the code inside keeps its own indent, a
    // tab included, and a `#` line in it is code, not a heading (§3.3)
    let src = "# P\n\n## S\n\n```\n\tx\n  # not a heading\n```\n";
    let (_d, mut v) = vault_with(src);
    let s = at(&v, &["P", "S"]);
    ops::toggle_spelling(&mut v, s).unwrap();
    let text = v.tree.files[0].text.clone();
    assert_eq!(text, "# P\n\n- S\n\n  ```\n  \tx\n    # not a heading\n  ```\n");
    let s = at(&v, &["P", "S"]);
    ops::toggle_spelling(&mut v, s).unwrap();
    assert_eq!(v.tree.files[0].text, src);
}

// Nesting written with 4 spaces or a tab is accepted on read (§4.2); a child
// written under such an item must still nest under it, not beside it.
#[test]
fn refile_under_a_four_space_nested_item_nests_under_it() {
    let (_d, mut v) = vault_with("- a\n    - b\n- c\n");
    let (c, b) = (at(&v, &["c"]), at(&v, &["a", "b"]));
    ops::refile(&mut v, c, b).unwrap();
    assert!(
        v.find_by_path(&["a".into(), "b".into(), "c".into()]).is_some(),
        "{}",
        v.tree.files[0].text
    );
}

#[test]
fn demote_among_four_space_siblings_nests_the_node() {
    let (_d, mut v) = vault_with("- a\n    - b\n    - c\n");
    let c = at(&v, &["a", "c"]);
    ops::demote(&mut v, c).unwrap();
    assert!(
        v.find_by_path(&["a".into(), "b".into(), "c".into()]).is_some(),
        "{}",
        v.tree.files[0].text
    );
}

#[test]
fn new_child_of_a_tab_nested_item() {
    let (_d, mut v) = vault_with("- a\n\t- b\n");
    let b = at(&v, &["a", "b"]);
    let r = ops::append_child_public(&mut v, b, "x");
    let text = v.tree.files[0].text.clone();
    let r = r.unwrap_or_else(|e| panic!("{e}\n{text:?}"));
    assert_eq!(v.tree.node(r).title, "x");
    assert!(v.find_by_path(&["a".into(), "b".into(), "x".into()]).is_some(), "{:?}", text);
}

#[test]
fn new_section_child_beside_a_deeper_written_section() {
    // N on b, whose section child is written further in than b's derived
    // child indent: the new section must still land under b
    let (_d, mut v) = vault_with("- a\n    - b\n      ## S\n");
    let b = at(&v, &["a", "b"]);
    ops::append_child_public(&mut v, b, "x").unwrap();
    let text = v.tree.files[0].text.clone();
    assert!(v.find_by_path(&["a".into(), "b".into(), "x".into()]).is_some(), "{:?}", text);
    assert!(v.find_by_path(&["a".into(), "b".into(), "S".into()]).is_some(), "{:?}", text);
}

#[test]
fn capture_to_a_four_space_nested_item_nests_under_it() {
    let (_d, mut v) = vault_with("- a\n    - b\n");
    let b = at(&v, &["a", "b"]);
    let r = ops::capture_to(&mut v, "x\nmore", false, b).unwrap();
    let text = v.tree.files[0].text.clone();
    assert!(v.find_by_path(&["a".into(), "b".into(), "x".into()]).is_some(), "{:?}", text);
    let body = v.tree.node(r).text_lines(&text).join("\n");
    assert!(body.contains("more"), "{:?}", text);
}

#[test]
fn deleting_a_section_block_keeps_lines_nested_under_its_embed() {
    // text appended under a heading embed by another editor parses as the
    // embed's content; the reading pane shows it in the parent, so deleting
    // the block must not take it along (§4.7, §11.5)
    let (d, mut v) = vault_files(
        &format!("# A\n\n## ![[{}]]\n\nnote under the embed\n", ID_A),
        &[("racfer~s.md", &format!("---\nid: {}\n---\n\n# S\n\nbody\n", ID_A))],
    );
    let s = at(&v, &["A", "S"]);
    ops::delete_subtree(&mut v, s).unwrap();
    assert!(!d.path().join("racfer~s.md").exists());
    assert_eq!(read(&d, "root.md"), "# A\n\nnote under the embed\n");
}

#[test]
fn clearing_a_done_section_block_keeps_lines_nested_under_its_embed() {
    let (d, mut v) = vault_files(
        &format!("# A\n\n## ![[{}]]\n\nnote under the embed\n\n## B\n", ID_A),
        &[("racfer~s.md", &format!("---\nid: {}\n---\n\n# [x] S\n\nbody\n", ID_A))],
    );
    let a = at(&v, &["A"]);
    assert_eq!(ops::clear_done(&mut v, a).unwrap(), 1);
    assert!(!d.path().join("racfer~s.md").exists());
    assert_eq!(read(&d, "root.md"), "# A\n\nnote under the embed\n\n## B\n");
}

#[test]
fn deleting_a_broken_embed_keeps_lines_nested_under_it() {
    let (d, mut v) = vault_with(&format!("# A\n\n## ![[{}]]\n\nnote under the embed\n", ID_A));
    let a = at(&v, &["A"]);
    let e = v.tree.resolved_children(a)[0];
    assert!(v.tree.node(e).is_embed());
    ops::delete_subtree(&mut v, e).unwrap();
    assert_eq!(read(&d, "root.md"), "# A\n\nnote under the embed\n");
}
