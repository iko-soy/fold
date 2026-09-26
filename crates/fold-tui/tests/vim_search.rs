//! Vim's `n` / `N` repeat the last search in the direction it was made (§10.6).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fold_tui::app::App;

fn app_with(text: &str) -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("root.md"), text).unwrap();
    let mut app = App::new(d.path()).unwrap();
    app.set_edit_keys(fold_tui::app::EditKeys::Vim);
    (d, app)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        let k = match c {
            '⎋' => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            '⏎' => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            c => KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        };
        app.handle_key(k);
    }
}

fn root(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join("root.md")).unwrap()
}

#[test]
fn vim_n_after_backward_search_goes_backward() {
    // `?` searches backward, so `n` goes on backward: from `foo 2` to `foo 1`.
    let (d, mut app) = app_with("# A\n\nfoo 1\nfoo 2\nfoo 3\n");
    keys(&mut app, "eG?foo⏎nx:w⏎");
    assert_eq!(root(&d), "# A\n\noo 1\nfoo 2\nfoo 3\n");
}

#[test]
fn vim_n_after_hash_goes_backward() {
    // `#` is a backward search: from `foo 2` it lands on `foo 1`, and `n`
    // wraps on backward to `foo 3`.
    let (d, mut app) = app_with("# A\n\nfoo 1\nfoo 2\nfoo 3\n");
    keys(&mut app, "e4G#nx:w⏎");
    assert_eq!(root(&d), "# A\n\nfoo 1\nfoo 2\noo 3\n");
}

#[test]
fn vim_n_after_forward_search_goes_forward() {
    // the control: `/` then `n` goes forward, as it does today.
    let (d, mut app) = app_with("# A\n\nfoo 1\nfoo 2\nfoo 3\n");
    keys(&mut app, "e3G/foo⏎nx:w⏎");
    assert_eq!(root(&d), "# A\n\nfoo 1\nfoo 2\noo 3\n");
}

#[test]
fn vim_shift_n_after_backward_search_goes_forward() {
    // `N` is the other way from the search: after `?` from `foo 2` lands on
    // `foo 1`, forward to `foo 2` again (not back around to `foo 3`).
    let (d, mut app) = app_with("# A\n\nfoo 1\nfoo 2\nfoo 3\n");
    keys(&mut app, "e4G?foo⏎Nx:w⏎");
    assert_eq!(root(&d), "# A\n\nfoo 1\noo 2\nfoo 3\n");
}
