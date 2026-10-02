//! The text field's generation (§10.6), as the app uses it: every result
//! of a verb run while the editor is open names the generation the field's
//! updates carry from then on (`FoldViewModel.settle`).

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
