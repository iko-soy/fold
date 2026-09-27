use fold_core::merge;
use fold_core::ops;
use fold_core::tree::NRef;
use fold_core::vault::Vault;

fn make_conflict_vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- [ ] task\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n- [x] task\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    (dir, v)
}

#[test]
fn pairs_found_after_merge() {
    let (_d, v) = make_conflict_vault();
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1);
    let (ours, theirs) = pairs[0];
    assert_eq!(v.tree.node(ours).title, "task");
    assert_eq!(v.tree.node(theirs).title, "task");
    assert!(v.tree.node(theirs).block.as_ref().unwrap().prop("conflict").is_some());
}

#[test]
fn keep_ours_removes_conflict_block() {
    let (_d, mut v) = make_conflict_vault();
    let pairs = merge::conflict_pairs(&v);
    let (_, theirs) = pairs[0];
    merge::resolve_keep_ours(&mut v, theirs).unwrap();
    assert!(merge::conflict_pairs(&v).is_empty());
    // ours' text is still there; theirs is gone
    let root = &v.tree.files[0].text;
    assert!(root.contains("- [ ] task"), "{}", root);
    assert_eq!(v.tree.files.len(), 1, "conflict block file trashed");
    // the trash has a copy
    let trash = fold_core::vault::trash_dir();
    let entries: Vec<_> = std::fs::read_dir(&trash)
        .map(|rd| rd.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();
    assert!(entries.iter().any(|e| e.file_name().to_string_lossy().contains("task")));
}

#[test]
fn keep_theirs_replaces_ours() {
    let (_d, mut v) = make_conflict_vault();
    let pairs = merge::conflict_pairs(&v);
    let (ours, theirs) = pairs[0];
    merge::resolve_keep_theirs(&mut v, ours, theirs).unwrap();
    assert!(merge::conflict_pairs(&v).is_empty());
    let root = &v.tree.files[0].text;
    assert!(root.contains("- [x] task"), "{}", root);
    assert!(!root.contains("- [ ] task"), "{}", root);
}

#[test]
fn keep_both_drops_conflict_key() {
    let (_d, mut v) = make_conflict_vault();
    let pairs = merge::conflict_pairs(&v);
    let (_, theirs) = pairs[0];
    merge::resolve_keep_both(&mut v, theirs).unwrap();
    assert!(merge::conflict_pairs(&v).is_empty());
    // both copies remain
    let root = &v.tree.files[0].text;
    assert!(root.contains("- [ ] task"), "{}", root);
    assert_eq!(v.tree.files.len(), 2, "the conflict block stays as a sibling");
    let bf = &v.tree.files[1];
    assert!(!bf.text.contains("conflict:"), "{}", bf.text);
    assert!(bf.text.contains("- [x] task"), "{}", bf.text);
}

/// A copy of `task`, which the phone checked, right after it and its
/// child, among its siblings `one` and `two` (§12.4).
fn pair_among_siblings() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- one\n- [ ] task\n  - sub\n- two\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n- one\n- [x] task\n  - sub\n- two\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert_eq!(ours(&v), ["task"]);
    (dir, v)
}

/// The node each copy pairs with: the one before it (§12.5).
fn ours(v: &Vault) -> Vec<String> {
    merge::conflict_pairs(v).into_iter().map(|(o, _)| v.tree.node(o).title.clone()).collect()
}

fn node(v: &Vault, path: &[&str]) -> NRef {
    v.find_by_path(&path.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
}

fn copy(v: &Vault) -> NRef {
    merge::conflict_pairs(v)[0].1
}

/// Titles of A's children, in order: the copy as `task⚠`.
fn order(v: &Vault) -> Vec<String> {
    let a = node(v, &["A"]);
    v.tree
        .resolved_children(a)
        .into_iter()
        .map(|c| {
            let n = v.tree.node(c);
            format!("{}{}", n.title, if n.conflict().is_some() { "⚠" } else { "" })
        })
        .collect()
}

#[test]
fn move_sibling_moves_a_node_and_its_conflict_copy_as_one() {
    let (_d, mut v) = pair_among_siblings();
    let text = v.tree.files[0].text.clone();
    let mut step = |at: &dyn Fn(&Vault) -> NRef, down: bool| {
        let r = at(&v);
        ops::move_sibling(&mut v, r, down).unwrap();
        assert_eq!(ours(&v), ["task"], "{}", v.tree.files[0].text);
        (order(&v), v.tree.files[0].text.clone())
    };
    // a node steps over the pair, up or down
    let two = |v: &Vault| node(v, &["A", "two"]);
    assert_eq!(step(&two, false).0, ["one", "two", "task", "task⚠"]);
    assert_eq!(step(&two, true).1, text);
    let one = |v: &Vault| node(v, &["A", "one"]);
    assert_eq!(step(&one, true).0, ["task", "task⚠", "one", "two"]);
    assert_eq!(step(&one, false).1, text);
    // the pair moves as one, by either side
    assert_eq!(step(&|v| node(v, &["A", "task"]), true).0, ["one", "two", "task", "task⚠"]);
    assert_eq!(step(&copy, false).1, text);
    assert_eq!(step(&copy, false).0, ["task", "task⚠", "one", "two"]);
}

#[test]
fn a_node_placed_between_a_node_and_its_conflict_copy_goes_after_the_copy() {
    let after_copy = ["one", "task", "task⚠", "new", "two"];
    // pasted after ours, or before the copy
    let (_d, mut v) = pair_among_siblings();
    let task = node(&v, &["A", "task"]);
    assert!(!ops::paste(&mut v, task, "- new\n", true).unwrap());
    assert_eq!(order(&v), after_copy);
    let (_d, mut v) = pair_among_siblings();
    let c = copy(&v);
    ops::paste(&mut v, c, "- new\n", false).unwrap();
    assert_eq!(order(&v), after_copy);
    // dropped before the copy
    let (_d, mut v) = pair_among_siblings();
    let (one, c) = (node(&v, &["A", "one"]), copy(&v));
    ops::move_node(&mut v, one, c, ops::Drop::Before).unwrap();
    assert_eq!(order(&v), ["task", "task⚠", "one", "two"]);
    // outdented out of ours
    let (_d, mut v) = pair_among_siblings();
    let sub = node(&v, &["A", "task", "sub"]);
    ops::promote(&mut v, sub).unwrap();
    assert_eq!(order(&v), ["one", "task", "task⚠", "sub", "two"]);
    assert_eq!(ours(&v), ["task"]);
    // ours itself, dropped before its own copy, stays where it is
    let (_d, mut v) = pair_among_siblings();
    let text = v.tree.files[0].text.clone();
    let (task, c) = (node(&v, &["A", "task"]), copy(&v));
    ops::move_node(&mut v, task, c, ops::Drop::Before).unwrap();
    assert_eq!(v.tree.files[0].text, text);
}

#[test]
fn a_pair_written_deeper_than_the_sibling_it_moves_below_stays_its_sibling() {
    // X and its copy are written a level deeper than Y, as a hand edit may
    // leave them (§4.7): below Y they are written at its level, or they
    // would nest under it
    let dir = tempfile::tempdir().unwrap();
    let id = "bacbec-bacbec-bacbec-bacbec";
    std::fs::write(dir.path().join("root.md"), format!("# A\n\n### X\n\nmine\n\n### ![[{}]]\n\n## Y\n\nwhy\n", id)).unwrap();
    std::fs::write(
        dir.path().join("bacbec~x.md"),
        format!("---\nid: {}\nconflict: \"PHONE 20260926-090000\"\n---\n\n# X\n\ntheirs\n", id),
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    assert_eq!(ours(&v), ["X"]);
    let y = node(&v, &["A", "Y"]);
    ops::move_sibling(&mut v, y, false).unwrap();
    assert_eq!(v.tree.files[0].text, format!("# A\n\n## Y\n\nwhy\n\n## X\n\nmine\n\n## ![[{}]]\n", id));
    assert_eq!(order(&v), ["Y", "X", "X⚠"]);
    assert_eq!(ours(&v), ["X"]);
}
