use assert_cmd::Command;
use predicates::prelude::*;

fn notes(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("fold").unwrap();
    c.arg("--vault").arg(dir);
    c
}

#[test]
fn empty_vault_gets_fresh_root() {
    let dir = tempfile::tempdir().unwrap();
    notes(dir.path())
        .arg("check")
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.starts_with("# Inbox"), "{}", root);
}

#[test]
fn capture_appends_under_today() {
    let dir = tempfile::tempdir().unwrap();
    notes(dir.path())
        .args(["capture", "hello from cli"])
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- hello from cli"), "{}", root);
    // dated day section
    assert!(root.contains("## 20"), "{}", root);
}

#[test]
fn capture_task_and_to() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# Projects\n").unwrap();
    notes(dir.path())
        .args(["capture", "buy milk", "--task", "--to", "Projects"])
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- [ ] buy milk"), "{}", root);
    let proj = root.find("# Projects").unwrap();
    let milk = root.find("buy milk").unwrap();
    assert!(milk > proj);
}

#[test]
fn capture_from_stdin() {
    let dir = tempfile::tempdir().unwrap();
    notes(dir.path())
        .args(["capture"])
        .write_stdin("from stdin\n")
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- from stdin"), "{}", root);
}

#[test]
fn check_reports_and_fix_canonicalizes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n* [X] old style\n").unwrap();
    notes(dir.path())
        .args(["check"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("non-canonical"));
    notes(dir.path())
        .args(["check", "--fix"])
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- [x] old style"), "{}", root);
}

#[test]
fn merge_processes_conflict_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- ours\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n- theirs\n",
    )
    .unwrap();
    notes(dir.path())
        .args(["merge"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync-conflict"));
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- ours") && root.contains("- theirs"), "{}", root);
}

#[test]
fn trash_list_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    // put something in the trash via a delete through the core
    std::fs::write(dir.path().join("root.md"), "# A\n\n- doomed\n").unwrap();
    {
        let mut v = fold_core::vault::Vault::open(dir.path()).unwrap();
        let a = v.tree.resolved_children(v.tree.root)[0];
        let node = v.tree.resolved_children(a)[0];
        fold_core::ops::delete_subtree(&mut v, node).unwrap();
    }
    notes(dir.path())
        .args(["trash", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("doomed"));
    // restore by a unique substring of the trash file name
    notes(dir.path())
        .args(["trash", "restore", "doomed"])
        .assert()
        .success();
    assert!(dir.path().read_dir().unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("doomed")));
}

#[test]
fn help_prints_usage() {
    let dir = tempfile::tempdir().unwrap();
    notes(dir.path())
        .args(["help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("command palette"))
        .stdout(predicate::str::contains("make block"));
}

#[test]
fn missing_vault_is_an_error_not_a_new_directory() {
    let dir = tempfile::tempdir().unwrap();
    let typo = dir.path().join("typo");
    notes(&typo).arg("check").assert().failure();
    assert!(!typo.exists());
}

#[test]
fn trash_restore_never_overwrites_root_md() {
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# Keep\n").unwrap();
    let trash = state.path().join("fold").join("trash");
    std::fs::create_dir_all(&trash).unwrap();
    std::fs::write(trash.join("20260926-101010-root.md"), "# Root\n").unwrap();
    notes(dir.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["trash", "restore", "root"])
        .assert()
        .success();
    assert_eq!(std::fs::read_to_string(dir.path().join("root.md")).unwrap(), "# Keep\n");
    assert!(dir.path().join("root-restored-2.md").exists());
}
