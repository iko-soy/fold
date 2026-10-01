//! The session as the app drives it: verbs, the op log, the editor's text
//! field, changes from outside.

use super::*;

fn vault(files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<Session>) {
    // the trash is device-local state (§11.5): one for the whole run, as
    // the environment is the process's, whichever test runs
    static STATE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let state = STATE.get_or_init(|| tempfile::tempdir().unwrap());
    std::env::set_var("XDG_STATE_HOME", state.path());
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let s = Session::open(dir.path().to_string_lossy().into_owned()).unwrap();
    (dir, s)
}

fn read(dir: &tempfile::TempDir, name: &str) -> String {
    std::fs::read_to_string(dir.path().join(name)).unwrap()
}

/// `edit_update` with the editor's current generation.
fn upd(s: &Session, text: String, cursor: Option<u32>) -> EditorState {
    let g = s.lock().edit_generation;
    s.edit_update(text, cursor, g)
}

fn all() -> OutlineQuery {
    OutlineQuery { zoom: None, folded: vec![], unfolded: vec![], hide_done: false }
}

fn row(s: &Session, title: &str) -> Row {
    s.outline(all()).rows.into_iter().find(|r| r.title == title).unwrap_or_else(|| panic!("no row {:?}", title))
}

const ROOT: &str = "# Homelab\n\nTwo boxes.\n\n## NAS\n\n- [ ] Replace fan\n- [x] Scrub\n\n## Networking\n\n# Inbox\n";

#[test]
fn outline_rows_counts_and_preview() {
    let (_d, s) = vault(&[("root.md", ROOT)]);
    let o = s.outline(all());
    let titles: Vec<(&str, u32)> = o.rows.iter().map(|r| (r.title.as_str(), r.depth)).collect();
    assert_eq!(
        titles,
        [("Homelab", 0), ("NAS", 1), ("Replace fan", 2), ("Scrub", 2), ("Networking", 1), ("Inbox", 0)]
    );
    let home = &o.rows[0];
    assert_eq!((home.open, home.total), (1, 2));
    assert_eq!(home.preview, "Two boxes.");
    assert_eq!(home.spelling, Spelling::Section);
    assert_eq!(o.rows[2].task, Task::Open);
    // folded, its children go; done hidden, Scrub goes
    let q = OutlineQuery { folded: vec![home.key.clone()], ..all() };
    assert_eq!(s.outline(q).rows.len(), 2);
    let q = OutlineQuery { hide_done: true, ..all() };
    assert!(!s.outline(q).rows.iter().any(|r| r.title == "Scrub"));
    // zoomed: the header and its children
    let nas = row(&s, "NAS");
    let o = s.outline(OutlineQuery { zoom: Some(nas.key.clone()), ..all() });
    assert_eq!(o.zoom.as_ref().unwrap().title, "NAS");
    assert_eq!(o.crumbs.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Homelab", "NAS"]);
    assert_eq!(o.rows.len(), 2);
}

#[test]
fn verbs_write_files_and_undo() {
    let (d, s) = vault(&[("root.md", ROOT)]);
    let fan = row(&s, "Replace fan");
    let r = s.toggle_task(fan.key.clone());
    assert!(r.ok, "{}", r.message);
    assert_eq!(r.message, "done: “Replace fan”");
    assert!(read(&d, "root.md").contains("- [x] Replace fan\n"));
    assert_eq!(s.outline(all()).undo.as_deref(), Some("mark “Replace fan” done"));
    let r = s.undo();
    assert!(r.ok, "{}", r.message);
    assert_eq!(read(&d, "root.md"), ROOT);
    assert!(s.redo().ok);
    assert!(read(&d, "root.md").contains("- [x] Replace fan\n"));
    // a node that is no task says so and writes nothing
    let nas = row(&s, "NAS");
    let r = s.toggle_task(nas.key.clone());
    assert!(!r.ok);
    // a stale key is refused, never another node
    assert!(!s.toggle_task("pnonsense".into()).ok);
}

#[test]
fn add_move_indent_delete() {
    let (d, s) = vault(&[("root.md", ROOT)]);
    let fan = row(&s, "Replace fan");
    let r = s.add_node(Some(fan.key.clone()), "Order fan".into(), true, false);
    assert!(r.ok, "{}", r.message);
    assert!(read(&d, "root.md").contains("- [ ] Replace fan\n- [ ] Order fan\n- [x] Scrub\n"), "{}", read(&d, "root.md"));
    let new = r.node.unwrap();
    assert_eq!(s.node(new.clone()).unwrap().title, "Order fan");
    let r = s.indent(new.clone());
    assert!(r.ok, "{}", r.message);
    assert!(read(&d, "root.md").contains("- [ ] Replace fan\n  - [ ] Order fan\n"), "{}", read(&d, "root.md"));
    let moved = r.node.unwrap();
    let r = s.outdent(moved.clone());
    assert!(r.ok, "{}", r.message);
    let r = s.move_sibling(r.node.unwrap(), false);
    assert!(r.ok, "{}", r.message);
    assert!(read(&d, "root.md").contains("- [ ] Order fan\n- [ ] Replace fan\n"), "{}", read(&d, "root.md"));
    let r = s.delete(r.node.unwrap());
    assert!(r.ok, "{}", r.message);
    assert_eq!(read(&d, "root.md"), ROOT);
    // the register holds it: paste it back after Scrub
    let r = s.paste(row(&s, "Scrub").key, true);
    assert!(r.ok, "{}", r.message);
    assert!(read(&d, "root.md").contains("- [x] Scrub\n- [ ] Order fan\n"), "{}", read(&d, "root.md"));
    // a child under a section with section children is a section
    let r = s.add_node(Some(row(&s, "Homelab").key), "Power".into(), false, true);
    assert!(r.ok, "{}", r.message);
    assert!(read(&d, "root.md").contains("## Networking\n\n## Power\n"), "{}", read(&d, "root.md"));
    assert!(!s.add_node(None, "  ".into(), false, false).ok);
}

#[test]
fn properties_make_a_block() {
    let (d, s) = vault(&[("root.md", ROOT)]);
    let fan = row(&s, "Replace fan");
    assert!(!s.set_property(fan.key.clone(), "due".into(), "next week".into()).ok);
    let r = s.set_property(fan.key.clone(), "due".into(), "2026-10-05".into());
    assert!(r.ok, "{}", r.message);
    let block = r.node.unwrap();
    let props = s.properties(block.clone());
    assert_eq!(props.len(), 1);
    assert_eq!((props[0].key.as_str(), props[0].value.as_str()), ("due", "2026-10-05"));
    let row = row(&s, "Replace fan");
    assert!(row.block);
    assert_eq!(row.due.as_deref(), Some("2026-10-05"));
    let files: Vec<String> = std::fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md") && n != "root.md")
        .collect();
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("~replace-fan.md"), "{:?}", files);
    let text = read(&d, &files[0]);
    assert!(text.starts_with("---\nid: "), "{}", text);
    assert!(text.contains("due: 2026-10-05\n---\n\n- [ ] Replace fan\n"), "{}", text);
    // checking a task block stamps done: (§8.2)
    assert!(s.toggle_task(row.key.clone()).ok);
    assert!(read(&d, &files[0]).contains("done: "));
    assert!(s.remove_property(block, "due".into()).ok);
    assert!(!read(&d, &files[0]).contains("due:"));
}

#[test]
fn capture_lands_in_todays_inbox() {
    let (d, s) = vault(&[("root.md", ROOT)]);
    let r = s.capture("[ ] call the plumber".into(), false);
    assert!(r.ok, "{}", r.message);
    let today = jiff::Zoned::now().date().to_string();
    assert!(read(&d, "root.md").ends_with(&format!("# Inbox\n\n## {}\n\n- [ ] call the plumber\n", today)), "{}", read(&d, "root.md"));
    assert_eq!(s.node(r.node.unwrap()).unwrap().title, "call the plumber");
}

#[test]
fn editor_saves_through_tags() {
    let block = "---\nid: racfer-hattes-mislup-nodrys\ndue: 2026-09-20\n---\n\n- [ ] Order new switch\n  Two options.\n";
    let root = "# Networking\n\n- [ ] Replace the flaky switch\n![[racfer-hattes-mislup-nodrys]]\n";
    let (d, s) = vault(&[("root.md", root), ("racfer~order-new-switch.md", block)]);
    let net = row(&s, "Networking");
    let v = s.edit_open(net.key.clone()).unwrap();
    assert_eq!(v.text, "# Networking\n\n- [ ] Replace the flaky switch\n- [ ] Order new switch\n  Two options.");
    // type into the block's line: only its file changes
    let text = v.text.replace("Two options.", "Two options, both cheap.");
    let st = upd(&s, text.clone(), None);
    assert!(st.dirty);
    let saved = s.edit_save();
    assert!(saved.ok, "{}", saved.message);
    assert_eq!(read(&d, "root.md"), root);
    assert!(read(&d, "racfer~order-new-switch.md").ends_with("- [ ] Order new switch\n  Two options, both cheap.\n"));
    // Enter at the end of the parent's last line: the new line is root.md's
    let at = text.find("\n- [ ] Order").unwrap();
    let text2 = format!("{}\n- Label cables{}", &text[..at], &text[at..]);
    let cursor = text2[..at + "\n- Label cables".len()].chars().count() as u32;
    upd(&s, text2, Some(cursor));
    let saved = s.edit_close();
    assert!(saved.ok, "{}", saved.message);
    assert_eq!(read(&d, "root.md"), "# Networking\n\n- [ ] Replace the flaky switch\n- Label cables\n![[racfer-hattes-mislup-nodrys]]\n");
    // each save is one undo step
    assert_eq!(s.outline(all()).undo.as_deref(), Some("edit “Networking”"));
    assert!(s.undo().ok);
    assert_eq!(read(&d, "root.md"), root);
}

#[test]
fn editor_cut_and_paste_moves_a_block() {
    let block = "---\nid: racfer-hattes-mislup-nodrys\ndue: 2026-09-20\n---\n\n- [ ] Order new switch\n";
    let root = "# Networking\n\n- a\n![[racfer-hattes-mislup-nodrys]]\n- b\n";
    let (d, s) = vault(&[("root.md", root), ("racfer~order-new-switch.md", block)]);
    let v = s.edit_open(row(&s, "Networking").key).unwrap();
    assert_eq!(v.text, "# Networking\n\n- a\n- [ ] Order new switch\n- b");
    // cut the block's line ...
    let cut = "# Networking\n\n- a\n- b";
    upd(&s, cut.into(), Some("# Networking\n\n- a\n".chars().count() as u32));
    // ... an autosave meanwhile keeps its file (in transit)
    assert!(s.edit_save().ok);
    assert!(d.path().join("racfer~order-new-switch.md").exists());
    // ... and paste it on a new line after b: Enter, then paste
    let enter = "# Networking\n\n- a\n- b\n";
    upd(&s, enter.into(), Some(enter.chars().count() as u32));
    let pasted = "# Networking\n\n- a\n- b\n- [ ] Order new switch\n";
    upd(&s, pasted.into(), Some(pasted.chars().count() as u32));
    let saved = s.edit_close();
    assert!(saved.ok, "{}", saved.message);
    assert_eq!(read(&d, "root.md"), "# Networking\n\n- a\n- b\n![[racfer-hattes-mislup-nodrys]]\n");
    assert_eq!(read(&d, "racfer~order-new-switch.md"), block);
    // and again, the lines pasted at the end of the last line in one go
    let (d, s) = vault(&[("root.md", root), ("racfer~order-new-switch.md", block)]);
    s.edit_open(row(&s, "Networking").key).unwrap();
    upd(&s, cut.into(), Some("# Networking\n\n- a\n".chars().count() as u32));
    let pasted = "# Networking\n\n- a\n- b\n- [ ] Order new switch";
    upd(&s, pasted.into(), Some(pasted.chars().count() as u32));
    let saved = s.edit_close();
    assert!(saved.ok, "{}", saved.message);
    assert_eq!(read(&d, "root.md"), "# Networking\n\n- a\n- b\n![[racfer-hattes-mislup-nodrys]]\n");
    assert_eq!(read(&d, "racfer~order-new-switch.md"), block);
}

#[test]
fn editor_deleting_a_block_line_trashes_it_on_close() {
    let block = "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- Order new switch\n";
    let root = "# Networking\n\n- a\n![[racfer-hattes-mislup-nodrys]]\n";
    let (d, s) = vault(&[("root.md", root), ("racfer~order-new-switch.md", block)]);
    s.edit_open(row(&s, "Networking").key).unwrap();
    // the line erased and joined to the one above, as Backspace does
    upd(&s, "# Networking\n\n- a".into(), Some(17));
    let saved = s.edit_close();
    assert!(saved.ok, "{}", saved.message);
    assert_eq!(read(&d, "root.md"), "# Networking\n\n- a\n");
    assert!(!d.path().join("racfer~order-new-switch.md").exists());
    // one undo puts back the embed and the file together (§10.10)
    let r = s.undo();
    assert!(r.ok, "{}", r.message);
    assert_eq!(read(&d, "root.md"), root);
    assert_eq!(read(&d, "racfer~order-new-switch.md"), block);
}

#[test]
fn editor_undo_and_revert() {
    let (d, s) = vault(&[("root.md", "# A\n\ntext\n")]);
    let v = s.edit_open(row(&s, "A").key).unwrap();
    upd(&s, format!("{} more", v.text), None);
    let undone = s.edit_undo().unwrap();
    assert_eq!(undone.text, "# A\n\ntext");
    // a change made to the text before the undo replaced it is dropped
    let stale = s.edit_update("# A\n\ntext more, stale".into(), None, undone.generation - 1);
    assert!(!stale.dirty || s.edit_text().as_deref() == Some("# A\n\ntext"));
    assert_eq!(s.edit_text().as_deref(), Some("# A\n\ntext"));
    assert_eq!(s.edit_redo().map(|t| t.text).as_deref(), Some("# A\n\ntext more"));
    let r = s.edit_revert();
    assert!(r.ok);
    assert_eq!(read(&d, "root.md"), "# A\n\ntext\n");
    assert!(s.edit_text().is_none());
}

#[test]
fn outside_changes_are_taken_in_and_said() {
    let (d, s) = vault(&[("root.md", ROOT)]);
    assert!(!s.refresh(false).changed);
    std::fs::write(d.path().join("root.md"), format!("{}\n- from the phone\n", ROOT.trim_end())).unwrap();
    let r = s.refresh(false);
    assert!(r.changed);
    assert_eq!(r.message.as_deref(), Some("↻ changed outside fold: Inbox (+1 item)"));
    assert!(s.outline(all()).rows.iter().any(|r| r.title == "from the phone"));
    // a verb on a file changed under it refuses, writing nothing
    let fan = row(&s, "Replace fan");
    std::fs::write(d.path().join("root.md"), "# Other\n").unwrap();
    let r = s.toggle_task(fan.key);
    assert!(!r.ok);
    assert_eq!(read(&d, "root.md"), "# Other\n");
}

#[test]
fn sync_conflict_copies_merge_into_pairs() {
    let (d, s) = vault(&[("root.md", "# Notes\n\n- milk\n")]);
    std::fs::write(d.path().join("root.sync-conflict-20260927-100000-PHONE.md"), "# Notes\n\n- milk, oat\n").unwrap();
    let r = s.refresh(false);
    assert!(r.changed);
    assert_eq!(r.raised, 0, "{:?}", r.message);
    // milk and milk, oat are two nodes: nothing is ever treated as deleted
    let o = s.outline(all());
    assert!(o.rows.iter().any(|r| r.title == "milk, oat"));
    std::fs::write(d.path().join("root.md"), "# Notes\n\nmine\n").unwrap();
    std::fs::write(d.path().join("root.sync-conflict-20260927-110000-PHONE.md"), "# Notes\n\ntheirs\n").unwrap();
    let r = s.refresh(false);
    assert_eq!(r.raised, 1, "{:?}", r.message);
    let pairs = s.conflicts();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].from, "PHONE 09-27 11:00");
    let r = s.resolve(pairs[0].theirs.clone(), Keep::Theirs);
    assert!(r.ok, "{}", r.message);
    assert!(s.conflicts().is_empty());
    assert!(read(&d, "root.md").contains("theirs"));
}

#[test]
fn search_and_targets() {
    let (_d, s) = vault(&[("root.md", ROOT)]);
    let hits = s.search("fan".into());
    assert_eq!(hits[0].title, "Replace fan");
    assert_eq!(hits[0].path, "Homelab › NAS");
    let hits = s.search("boxes".into());
    assert_eq!(hits[0].title, "Homelab");
    assert_eq!(hits[0].excerpt.as_deref(), Some("Two boxes."));
    // a node never goes into its own subtree
    let home = row(&s, "Homelab");
    let t = s.targets(String::new(), Some(home.key));
    assert_eq!(t.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), ["Inbox"]);
}

#[test]
fn reading_lines_carry_nodes() {
    let (_d, s) = vault(&[("root.md", ROOT)]);
    let lines = s.reading(Some(row(&s, "NAS").key));
    assert_eq!(lines[0].kind, ReadKind::Title);
    assert_eq!(lines[0].heading, 1);
    assert_eq!(lines[0].text, "# NAS");
    let fan = lines.iter().find(|l| l.title == "Replace fan").unwrap();
    assert_eq!(fan.task, Task::Open);
    assert_eq!(fan.node.as_deref(), Some(row(&s, "Replace fan").key.as_str()));
}

#[test]
fn trash_and_restore() {
    let block = "---\nid: racfer-hattes-mislup-nodrys\n---\n\n- Order new switch\n";
    let root = "# Networking\n\n![[racfer-hattes-mislup-nodrys]]\n";
    let (d, s) = vault(&[("root.md", root), ("racfer~order-new-switch.md", block)]);
    assert!(s.delete(row(&s, "Order new switch").key).ok);
    let trash = s.trash();
    let entry = trash.iter().find(|e| e.name.ends_with("racfer~order-new-switch.md")).unwrap();
    assert_eq!(s.trash_text(entry.name.clone()).as_deref(), Some(block));
    assert!(s.trash_text("../root.md".into()).is_none());
    let r = s.restore(entry.name.clone());
    assert!(r.ok, "{}", r.message);
    assert_eq!(read(&d, "racfer~order-new-switch.md"), block);
}

#[test]
fn a_sync_under_the_editor_re_renders_it_and_drops_stale_typing() {
    let (d, s) = vault(&[("root.md", "# Notes\n\n- milk\n")]);
    let v = s.edit_open(row(&s, "Notes").key).unwrap();
    // the phone adds to the same node meanwhile
    std::fs::write(d.path().join("root.md"), "# Notes\n\n- milk\n- eggs\n").unwrap();
    let r = s.refresh(false);
    assert_eq!(r.editor_text.as_deref(), Some("# Notes\n\n- milk\n- eggs"));
    assert!(r.editor_generation > v.generation);
    // typing that was on its way, made to the old text, is not laid over it
    let st = s.edit_update("# Notes\n\n- milk!".into(), None, v.generation);
    assert!(!st.dirty);
    assert!(s.edit_close().ok);
    assert_eq!(read(&d, "root.md"), "# Notes\n\n- milk\n- eggs\n");
}
