use notes_core::merge;
use notes_core::vault::Vault;

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
    let trash = notes_core::vault::trash_dir();
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
