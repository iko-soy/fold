//! The session op log (§10.10): an entry holds exactly the files an
//! operation touched; undo and redo refuse when one changed since.

use fold_core::ops::{self, Inverse, Snapshot};
use fold_core::vault::Vault;

fn vault_with(root: &str) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), root).unwrap();
    std::fs::write(
        dir.path().join("racfer~other.md"),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- other\n",
    )
    .unwrap();
    let v = Vault::open(dir.path()).unwrap();
    (dir, v)
}

fn read(d: &tempfile::TempDir, f: &str) -> String {
    std::fs::read_to_string(d.path().join(f)).unwrap()
}

#[test]
fn an_entry_holds_only_the_touched_files() {
    let (_d, mut v) = vault_with("# A\n\n- [ ] t\n\n![[racfer-hattes-mislup-nodrys]]\n");
    let snap = Snapshot::take(&v, "toggle");
    let t = v.find_by_path(&["A".into(), "t".into()]).unwrap();
    ops::toggle_task(&mut v, t).unwrap();
    let inv = Inverse::since(snap, &v).unwrap();
    let paths: Vec<&str> = inv.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, ["root.md"]);
}

#[test]
fn nothing_changed_means_no_entry() {
    let (_d, v) = vault_with("# A\n");
    let snap = Snapshot::take(&v, "nothing");
    assert!(Inverse::since(snap, &v).is_none());
}

#[test]
fn undo_keeps_external_edits_to_untouched_files() {
    let (d, mut v) = vault_with("# A\n\n- [ ] t\n\n![[racfer-hattes-mislup-nodrys]]\n");
    let snap = Snapshot::take(&v, "toggle");
    let t = v.find_by_path(&["A".into(), "t".into()]).unwrap();
    ops::toggle_task(&mut v, t).unwrap();
    let inv = Inverse::since(snap, &v).unwrap();
    // another editor changes the block file meanwhile
    let edited = "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- other, edited\n";
    std::fs::write(d.path().join("racfer~other.md"), edited).unwrap();
    inv.undo(&mut v).unwrap();
    assert_eq!(read(&d, "root.md"), "# A\n\n- [ ] t\n\n![[racfer-hattes-mislup-nodrys]]\n");
    assert_eq!(read(&d, "racfer~other.md"), edited);
}

#[test]
fn undo_refuses_when_a_touched_file_changed() {
    let (d, mut v) = vault_with("# A\n\n- [ ] t\n");
    let snap = Snapshot::take(&v, "toggle");
    let t = v.find_by_path(&["A".into(), "t".into()]).unwrap();
    ops::toggle_task(&mut v, t).unwrap();
    let inv = Inverse::since(snap, &v).unwrap();
    std::fs::write(d.path().join("root.md"), "# A\n\n- [x] t\n- added elsewhere\n").unwrap();
    let err = inv.undo(&mut v).unwrap_err();
    assert!(err.to_string().contains("root.md"), "{}", err);
    assert_eq!(read(&d, "root.md"), "# A\n\n- [x] t\n- added elsewhere\n");
}

#[test]
fn undo_and_redo_of_make_block_create_and_remove_the_file() {
    let (d, mut v) = vault_with("# A\n\n## B\n\nbody\n");
    let snap = Snapshot::take(&v, "make block");
    let b = v.find_by_path(&["A".into(), "B".into()]).unwrap();
    ops::make_block(&mut v, b).unwrap();
    let inv = Inverse::since(snap, &v).unwrap();
    let count = || std::fs::read_dir(d.path()).unwrap().count();
    let with_block = count();
    inv.undo(&mut v).unwrap();
    assert_eq!(count(), with_block - 1);
    assert_eq!(read(&d, "root.md"), "# A\n\n## B\n\nbody\n");
    inv.redo(&mut v).unwrap();
    assert_eq!(count(), with_block);
    assert!(v.find_by_path(&["A".into(), "B".into()]).is_some());
}

/// A verb computes what it writes from the parsed files. When a sync has
/// changed a file since and the change is not reloaded yet (the watcher's
/// debounce), the verb refuses rather than write its stale text back over
/// the change, which undo could not bring back either (§1 principle 4,
/// §11.2).
#[test]
fn a_verb_refuses_to_overwrite_a_change_not_reloaded_yet() {
    let (d, mut v) = vault_with("- [ ] a\n- [ ] b\n");
    let synced = "- [ ] a\n- [ ] b\n- [ ] from phone\n";
    std::fs::write(d.path().join("root.md"), synced).unwrap();
    let a = v.find_by_path(&["a".into()]).unwrap();
    let err = ops::toggle_task(&mut v, a).unwrap_err();
    assert!(err.to_string().contains("root.md"), "{}", err);
    assert_eq!(read(&d, "root.md"), synced);
    // once reloaded, it runs on what is there
    v.reload().unwrap();
    let a = v.find_by_path(&["a".into()]).unwrap();
    ops::toggle_task(&mut v, a).unwrap();
    assert_eq!(read(&d, "root.md"), "- [x] a\n- [ ] b\n- [ ] from phone\n");
}
