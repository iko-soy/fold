//! The session as the app uses it (`FoldViewModel`).

use fold_ffi::{OutlineQuery, Session};
use std::sync::Arc;

fn open(root: &str) -> (tempfile::TempDir, Arc<Session>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), root).unwrap();
    let s = Session::open(dir.path().to_string_lossy().into_owned()).unwrap();
    (dir, s)
}

fn key(s: &Session, title: &str) -> String {
    let q = OutlineQuery { zoom: None, folded: vec![], unfolded: vec![], hide_done: false };
    s.outline(q).rows.into_iter().find(|r| r.title == title).unwrap().key
}

/// Every result of a verb run while the editor is open names the
/// generation the text field's updates carry from then on (§10.6), and the
/// app takes it (`settle`).
#[test]
fn verbs_that_leave_the_editor_alone_keep_its_generation() {
    let (dir, s) = open("- [ ] a\n- b\n");
    let a = key(&s, "a");
    assert!(s.toggle_task(a.clone()).ok);
    let view = s.edit_open(key(&s, "b")).unwrap();
    let mut generation = view.generation;
    // the snackbar's Undo, Redo and Copy, tapped while editing
    for r in [s.undo(None), s.redo(), s.copy(a)] {
        generation = r.editor_generation;
    }
    assert_eq!(generation, view.generation);
    // what is typed after them is taken in, and saved
    s.edit_update(format!("{} and c", view.text.trim_end()), None, generation);
    assert!(s.edit_close().ok);
    let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(text.contains("- b and c"), "{}", text);
}

/// A message offers to undo the entry its own verb made (§10.10), named
/// with the verb: a second change made before the message shows does not
/// take its place.
#[test]
fn each_verb_names_its_own_undo_entry() {
    let (_dir, s) = open("- [ ] a\n- [ ] b\n");
    let first = s.toggle_task(key(&s, "a")).undo.unwrap();
    let second = s.toggle_task(key(&s, "b")).undo.unwrap();
    assert_ne!(first.description, second.description);
    // nothing written, nothing to offer
    assert!(s.copy(key(&s, "a")).undo.is_none());
    // the first message's Undo, tapped after the second change, takes
    // nothing back; the second's takes back its own
    assert!(!s.undo(Some(first)).ok);
    let r = s.undo(Some(second));
    assert!(r.ok, "{}", r.message);
    assert!(r.message.contains("b"), "{}", r.message);
}
