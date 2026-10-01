//! The session op log (§10.10): an entry holds exactly the files an
//! operation touched; undo and redo refuse when one changed since.

use fold_core::ops::{self, Inverse, OpLog, Snapshot};
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

const OTHER_SYNCED: &str = "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- other, from phone\n";

/// A sync changes a file the verb does not touch, and it is not reloaded
/// yet (the watcher's debounce): the verb's entry holds only what the verb
/// wrote, so undoing it leaves the sync's change be (§10.10). A verb that
/// re-read the whole vault would take the change in as its own, and its
/// undo would revert it.
fn keeps_a_sync_it_did_not_make(d: &tempfile::TempDir, v: &mut Vault, verb: impl FnOnce(&mut Vault)) {
    std::fs::write(d.path().join("racfer~other.md"), OTHER_SYNCED).unwrap();
    let snap = Snapshot::take(v, "verb");
    verb(v);
    let inv = Inverse::since(snap, v).expect("the verb changed something");
    let paths: Vec<&str> = inv.changes.iter().map(|c| c.path.as_str()).collect();
    assert!(!paths.contains(&"racfer~other.md"), "{:?}", paths);
    inv.undo(v).unwrap();
    assert_eq!(read(d, "racfer~other.md"), OTHER_SYNCED);
}

/// `keeps_a_sync_it_did_not_make` on a vault with `WITH_OTHER` as its root.
fn verb_keeps_a_sync(verb: impl FnOnce(&mut Vault)) {
    let (d, mut v) = vault_with("# A\n\n- t\n- u\n\n![[racfer-hattes-mislup-nodrys]]\n\n# B\n");
    keeps_a_sync_it_did_not_make(&d, &mut v, verb);
}

fn at(v: &Vault, path: &[&str]) -> fold_core::tree::NRef {
    v.find_by_path(&path.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
}

#[test]
fn make_block_keeps_a_sync_it_did_not_make() {
    verb_keeps_a_sync(|v| {
        ops::make_block(v, at(v, &["A", "t"])).unwrap();
    });
}

#[test]
fn set_property_keeps_a_sync_it_did_not_make() {
    verb_keeps_a_sync(|v| {
        ops::set_property(v, at(v, &["A", "t"]), "due", "2026-10-01").unwrap();
    });
}

#[test]
fn refile_keeps_a_sync_it_did_not_make() {
    verb_keeps_a_sync(|v| {
        ops::refile(v, at(v, &["A", "t"]), at(v, &["B"])).unwrap();
    });
}

#[test]
fn paste_keeps_a_sync_it_did_not_make() {
    verb_keeps_a_sync(|v| {
        ops::paste(v, at(v, &["A", "u"]), "- pasted\n", true).unwrap();
    });
}

#[test]
fn capture_keeps_a_sync_it_did_not_make() {
    verb_keeps_a_sync(|v| {
        ops::capture(v, "call the plumber", false).unwrap();
    });
}

#[test]
fn a_new_child_keeps_a_sync_it_did_not_make() {
    verb_keeps_a_sync(|v| {
        ops::append_child_public(v, at(v, &["B"]), "new").unwrap();
    });
}

#[test]
fn respelling_keeps_a_sync_it_did_not_make() {
    verb_keeps_a_sync(|v| {
        ops::toggle_spelling(v, at(v, &["B"])).unwrap();
    });
}

#[test]
fn canonicalizing_keeps_a_sync_it_did_not_make() {
    // a non-canonical bullet to rewrite, and a block whose name is stale,
    // to rename (§4.2, §6.4)
    let (d, mut v) = vault_with("# A\n\n* t\n\n![[racfer-hattes-mislup-nodrys]]\n![[dozzod-binwes-talsun-worbec]]\n");
    let block = "---\nid: dozzod-binwes-talsun-worbec\n---\n\n- renamed since\n";
    std::fs::write(d.path().join("dozzod~old-name.md"), block).unwrap();
    v.reload().unwrap();
    keeps_a_sync_it_did_not_make(&d, &mut v, |v| {
        assert_eq!(fold_core::check::fix(v).unwrap(), 2);
        // the index knows the file by its new name
        assert!(v.file_index("dozzod~renamed-since.md").is_some());
        assert!(v.file_index("dozzod~old-name.md").is_none());
    });
    assert_eq!(read(&d, "dozzod~old-name.md"), block);
}

#[test]
fn keeping_theirs_keeps_a_sync_it_did_not_make() {
    // ours is a block, its conflict copy right after it (§12.4)
    let (d, mut v) = vault_with(
        "- other\n![[racfer-hattes-mislup-nodrys]]\n![[dozzod-binwes-talsun-worbec]]\n![[lacnum-walbyn-dirlyn-havtyp]]\n",
    );
    std::fs::write(d.path().join("dozzod~ours.md"), "---\nid: dozzod-binwes-talsun-worbec\ndue: 2026-10-01\n---\n\n- ours\n").unwrap();
    std::fs::write(
        d.path().join("lacnum~ours.md"),
        "---\nid: lacnum-walbyn-dirlyn-havtyp\nconflict: \"PHONE 20260927-100000\"\n---\n\n- theirs\n",
    )
    .unwrap();
    v.reload().unwrap();
    keeps_a_sync_it_did_not_make(&d, &mut v, |v| {
        let (ours, theirs) = fold_core::merge::conflict_pairs(v)[0];
        fold_core::merge::resolve_keep_theirs(v, ours, theirs).unwrap();
    });
    assert_eq!(read(&d, "lacnum~ours.md").lines().last(), Some("- theirs"));
}

#[test]
fn the_op_log_steps_back_and_forth() {
    let (d, mut v) = vault_with("# A\n\n- [ ] t\n");
    let mut log = OpLog::default();
    assert!(log.step(&mut v, true).is_none(), "nothing to undo");
    let snap = Snapshot::take(&v, "mark “t” done");
    let t = at(&v, &["A", "t"]);
    ops::toggle_task(&mut v, t).unwrap();
    assert!(log.record(Inverse::since(snap, &v)));
    // a verb that changed nothing leaves no entry
    assert!(!log.record(Inverse::since(Snapshot::take(&v, "nothing"), &v)));
    assert_eq!(log.depth(), 1);
    assert_eq!(log.step(&mut v, true).unwrap().unwrap(), "mark “t” done");
    assert_eq!(read(&d, "root.md"), "# A\n\n- [ ] t\n");
    assert_eq!(log.last_redo().map(|e| e.description.as_str()), Some("mark “t” done"));
    log.step(&mut v, false).unwrap().unwrap();
    assert_eq!(read(&d, "root.md"), "# A\n\n- [x] t\n");
    // refused when the file changed since: the entry stays
    std::fs::write(d.path().join("root.md"), "# A\n\n- [x] t\n- more\n").unwrap();
    assert!(log.step(&mut v, true).unwrap().is_err());
    assert_eq!(log.depth(), 1);
}
