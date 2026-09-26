//! Ordered children (§3.1, §3.3): text and nodes interleave, children obey
//! `(text | item)* section*`, every verb keeps that rule and leaves text
//! children where they are; section blocks are embedded as headings (§4.7).

use fold_core::check;
use fold_core::merge;
use fold_core::ops;
use fold_core::parse::{Content, Kind};
use fold_core::render::render;
use fold_core::tree::NRef;
use fold_core::vault::Vault;

fn vault_with(root: &str) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), root).unwrap();
    let v = Vault::open(dir.path()).unwrap();
    (dir, v)
}

fn at(v: &Vault, path: &str) -> NRef {
    let segs: Vec<String> = path.split('/').map(String::from).collect();
    v.find_by_path(&segs).unwrap_or_else(|| panic!("no node at {}", path))
}

fn root_text(v: &Vault) -> String {
    v.tree.files[0].text.clone()
}

/// Every node's title → its parent's title, over the resolved tree.
fn parents(v: &Vault) -> Vec<(String, String)> {
    let mut out = Vec::new();
    v.tree.walk(v.tree.root, &mut |t, r| {
        for c in t.resolved_children(r) {
            let p = if r == t.root { "<root>".to_string() } else { t.node(r).title.clone() };
            out.push((t.node(c).title.clone(), p));
        }
    });
    out.sort();
    out
}

/// No item follows a section among any node's children (§3.1).
fn assert_ordered(v: &Vault) {
    v.tree.walk(v.tree.root, &mut |t, r| {
        let kinds: Vec<Kind> = t.raw_children(r).iter().map(|&c| t.node(c).kind).collect();
        if let Some(fs) = kinds.iter().position(|k| *k == Kind::Section) {
            assert!(
                kinds[fs..].iter().all(|k| *k == Kind::Section),
                "item after a section under {:?}",
                t.node(r).title
            );
        }
    });
}

/// `parents` with one node's entry dropped: what a verb on it must not change.
fn others(p: &[(String, String)], moved: &str) -> Vec<(String, String)> {
    p.iter().filter(|(c, _)| c != moved).cloned().collect()
}

// ------------------------------------------------------------- the model

#[test]
fn text_and_nodes_interleave_in_order() {
    let src = "# Trip\n\nPlan below.\n\n- book flights\n- book hotel\n\nBudget is tight.\n";
    let (_d, v) = vault_with(src);
    let trip = at(&v, "Trip");
    let content = &v.tree.node(trip).content;
    let shape: Vec<&str> = content
        .iter()
        .map(|c| match c {
            Content::Text(_) => "text",
            Content::Node(_) => "node",
        })
        .collect();
    assert_eq!(shape, ["text", "node", "node", "text"]);
    let n = v.tree.node(trip);
    assert_eq!(n.body_lines(&v.tree.files[0].text), ["", "Plan below.", ""]);
    let all = n.text_lines(&v.tree.files[0].text);
    assert!(all.contains(&"Budget is tight."), "{:?}", all);
    assert_eq!(render(&v.tree, trip, 1, false), src);
}

#[test]
fn unindented_text_after_a_list_belongs_to_the_parent() {
    let (_d, v) = vault_with("# Trip\n\n- book flights\nBudget is tight.\n");
    let flights = at(&v, "Trip/book flights");
    assert!(v.tree.node(flights).text_lines(&v.tree.files[0].text).is_empty());
    let trip = at(&v, "Trip");
    assert_eq!(v.tree.node(trip).text_lines(&v.tree.files[0].text), ["Budget is tight."]);
}

// ------------------------------------------------------------- verbs

#[test]
fn toggle_item_to_section_moves_to_boundary() {
    let (_d, mut v) = vault_with("# P\n\n- a\n- b\n");
    let before = parents(&v);
    let r = at(&v, "P/a");
    ops::toggle_spelling(&mut v, r).unwrap();
    assert_eq!(root_text(&v), "# P\n\n- b\n\n## a\n");
    assert_eq!(v.tree.node(at(&v, "P/a")).kind, Kind::Section);
    assert_eq!(others(&parents(&v), "a"), others(&before, "a"));
    assert_ordered(&v);
}

#[test]
fn toggle_section_to_item_moves_to_boundary() {
    let (_d, mut v) = vault_with("# P\n\n- x\n\n## a\n\n## b\n");
    let r = at(&v, "P/b");
    ops::toggle_spelling(&mut v, r).unwrap();
    assert_eq!(v.tree.node(at(&v, "P/b")).kind, Kind::Item);
    let kids: Vec<String> = v
        .tree
        .resolved_children(at(&v, "P"))
        .iter()
        .map(|&c| v.tree.node(c).title.clone())
        .collect();
    assert_eq!(kids, ["x", "b", "a"]);
    assert_ordered(&v);
}

#[test]
fn toggle_in_place_reshapes_the_subtree_and_back() {
    let src = "- a\n  note\n  - c\n";
    let (_d, mut v) = vault_with(src);
    let r = at(&v, "a");
    ops::toggle_spelling(&mut v, r).unwrap();
    assert_eq!(root_text(&v), "# a\nnote\n- c\n");
    assert_eq!(v.tree.node(at(&v, "a")).kind, Kind::Section);
    assert!(v.find_by_path(&["a".into(), "c".into()]).is_some());
    let r = at(&v, "a");
    ops::toggle_spelling(&mut v, r).unwrap();
    assert_eq!(root_text(&v), src);
}

#[test]
fn toggle_keeps_a_done_checkbox() {
    let (_d, mut v) = vault_with("- [x] a\n");
    let r = at(&v, "a");
    ops::toggle_spelling(&mut v, r).unwrap();
    assert_eq!(root_text(&v), "# [x] a\n");
}

#[test]
fn move_refuses_to_cross_the_boundary() {
    let src = "# P\n\n- a\n\n## S\n";
    let (_d, mut v) = vault_with(src);
    let r = at(&v, "P/a");
    assert!(ops::move_sibling(&mut v, r, true).is_err());
    let r = at(&v, "P/S");
    assert!(ops::move_sibling(&mut v, r, false).is_err());
    assert_eq!(root_text(&v), src);
}

#[test]
fn move_leaves_text_children_in_place() {
    let (_d, mut v) = vault_with("# P\n\n- a\n\nmiddle\n\n- b\n");
    let r = at(&v, "P/a");
    ops::move_sibling(&mut v, r, true).unwrap();
    assert_eq!(root_text(&v), "# P\n\n- b\n\nmiddle\n\n- a\n");
}

#[test]
fn delete_leaves_text_children_in_place() {
    let (_d, mut v) = vault_with("# P\n\n- a\n\nmiddle\n\n- b\n");
    let r = at(&v, "P/a");
    ops::delete_subtree(&mut v, r).unwrap();
    assert_eq!(root_text(&v), "# P\n\nmiddle\n\n- b\n");
}

#[test]
fn promote_item_out_of_a_section_lands_before_the_sections() {
    let (_d, mut v) = vault_with("# G\n\n## P\n\n- x\n");
    let r = at(&v, "G/P/x");
    ops::promote(&mut v, r).unwrap();
    assert_eq!(root_text(&v), "# G\n\n- x\n\n## P\n");
    assert_ordered(&v);
}

#[test]
fn demote_item_into_a_node_with_section_children() {
    let (_d, mut v) = vault_with("- a\n  ## S\n  s body\n- b\n");
    let r = at(&v, "b");
    ops::demote(&mut v, r).unwrap();
    let kids: Vec<String> = v
        .tree
        .resolved_children(at(&v, "a"))
        .iter()
        .map(|&c| v.tree.node(c).title.clone())
        .collect();
    assert_eq!(kids, ["b", "S"]);
    assert!(v.find_by_path(&["a".into(), "S".into()]).is_some());
    assert_ordered(&v);
}

#[test]
fn refile_item_under_sections_is_clamped() {
    let (_d, mut v) = vault_with("# D\n\n## X\n\n# Src\n\n- it\n");
    let dest = at(&v, "D");
    let r = at(&v, "Src/it");
    ops::refile(&mut v, r, dest).unwrap();
    assert!(v.find_by_path(&["D".into(), "it".into()]).is_some(), "{}", root_text(&v));
    assert!(v.find_by_path(&["D".into(), "X".into(), "it".into()]).is_none());
    assert_ordered(&v);
}

#[test]
fn refile_item_keeps_nested_section_levels() {
    let (_d, mut v) = vault_with("# D\n\n# Src\n\n- a\n  ## S\n");
    let dest = at(&v, "D");
    let r = at(&v, "Src/a");
    ops::refile(&mut v, r, dest).unwrap();
    assert_eq!(root_text(&v), "# D\n\n- a\n  ## S\n\n# Src\n");
}

#[test]
fn paste_item_after_a_section_is_clamped() {
    let (_d, mut v) = vault_with("# P\n\n- a\n\n## S\n");
    let r = at(&v, "P/S");
    // clamped, and reported as such
    assert!(ops::paste(&mut v, r, "- new\n", true).unwrap());
    assert!(v.find_by_path(&["P".into(), "new".into()]).is_some(), "{}", root_text(&v));
    assert!(v.find_by_path(&["P".into(), "S".into(), "new".into()]).is_none());
    assert_ordered(&v);
}

#[test]
fn paste_section_before_an_item_is_clamped() {
    let (_d, mut v) = vault_with("# P\n\n- a\n- b\n");
    let r = at(&v, "P/a");
    ops::paste(&mut v, r, "# T\n", false).unwrap();
    let kids: Vec<String> = v
        .tree
        .resolved_children(at(&v, "P"))
        .iter()
        .map(|&c| v.tree.node(c).title.clone())
        .collect();
    assert_eq!(kids, ["a", "b", "T"]);
    assert_ordered(&v);
}

#[test]
fn capture_to_a_node_with_sections_lands_before_them() {
    let (_d, mut v) = vault_with("# P\n\n## S\n");
    let p = at(&v, "P");
    ops::capture_to(&mut v, "x", false, p).unwrap();
    assert!(v.find_by_path(&["P".into(), "x".into()]).is_some(), "{}", root_text(&v));
    assert_ordered(&v);
}

// ------------------------------------------------------------- heading embeds

#[test]
fn make_block_of_a_later_section_keeps_its_parent() {
    let (_d, mut v) = vault_with("# A\n\n## B\n\nb body\n\n## C\n\nc body\n");
    let before = parents(&v);
    let r = at(&v, "A/C");
    let id = ops::make_block(&mut v, r).unwrap();
    assert_eq!(
        root_text(&v),
        format!("# A\n\n## B\n\nb body\n\n## ![[{}]]\n", id)
    );
    assert_eq!(parents(&v), before);
    // and a section block before another section
    let r = at(&v, "A/B");
    let id2 = ops::make_block(&mut v, r).unwrap();
    assert_eq!(root_text(&v), format!("# A\n\n## ![[{}]]\n\n## ![[{}]]\n", id2, id));
    assert_eq!(parents(&v), before);
}

#[test]
fn item_blocks_keep_bare_embeds() {
    let (_d, mut v) = vault_with("- a\n- b\n");
    let r = at(&v, "a");
    let id = ops::make_block(&mut v, r).unwrap();
    assert_eq!(root_text(&v), format!("![[{}]]\n- b\n", id));
}

#[test]
fn toggling_a_block_rewrites_its_embed() {
    let (_d, mut v) = vault_with("# P\n\n- a\n- b\n");
    let r = at(&v, "P/a");
    let id = ops::make_block(&mut v, r).unwrap();
    let r = at(&v, "P/a");
    ops::toggle_spelling(&mut v, r).unwrap();
    assert_eq!(root_text(&v), format!("# P\n\n- b\n\n## ![[{}]]\n", id));
    let a = at(&v, "P/a");
    assert_eq!(v.tree.node(a).kind, Kind::Section);
    assert!(v.tree.files[a.0].text.ends_with("# a\n"), "{}", v.tree.files[a.0].text);
    assert!(check::check(&v).iter().all(|d| !d.message.contains("embed form")));
}

#[test]
fn check_reports_and_fixes_embed_forms() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("root.md"),
        "# A\n\n## B\n\n- x\n![[racfer-hattes-mislup-nodrys]]\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("racfer~s.md"),
        "---\nid: racfer-hattes-mislup-nodrys\n---\n\n# S\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    assert!(check::check(&v).iter().any(|d| d.message.contains("embed form")));
    check::fix(&mut v).unwrap();
    assert!(check::check(&v).iter().all(|d| !d.message.contains("embed form")));
    assert!(root_text(&v).contains("### ![[racfer-hattes-mislup-nodrys]]"), "{}", root_text(&v));
    assert_ordered(&v);
}

#[test]
fn fix_keeps_root_level_text_between_nodes() {
    let (_d, mut v) = vault_with("- a\ntext between\n- b\n");
    check::fix(&mut v).unwrap();
    assert!(root_text(&v).contains("text between"), "{}", root_text(&v));
    assert!(v.find_by_path(&["b".into()]).is_some());
}

// ------------------------------------------------------------- merge

#[test]
fn merge_keeps_text_after_children() {
    let o = "# P\n\n- a\n\nafter the list\n";
    let t = "# P\n\n- a\n\nafter the list\n\n# Q\n";
    let m = merge::merge_texts(o, t, "phone", "20260926-1200");
    assert_eq!(m.conflicts, 0);
    assert!(m.text.contains("after the list"), "{}", m.text);
    assert!(m.text.contains("# Q"), "{}", m.text);
}

#[test]
fn merge_text_child_difference_conflicts() {
    let o = "# P\n\n- a\n\nafter\n";
    let t = "# P\n\n- a\n\nafter, edited\n";
    let m = merge::merge_texts(o, t, "phone", "20260926-1200");
    assert_eq!(m.conflicts, 1);
    assert!(m.conflict_blocks[0].1.contains("after, edited"));
}

#[test]
fn merge_places_inserted_items_before_sections() {
    let o = "# P\n\n- a\n\n## S\n";
    let t = "# P\n\n- a\n- new\n\n## S\n";
    let m = merge::merge_texts(o, t, "phone", "20260926-1200");
    let (_d, v) = vault_with(&m.text);
    assert!(v.find_by_path(&["P".into(), "new".into()]).is_some(), "{}", m.text);
    assert_ordered(&v);
}

#[test]
fn section_conflict_is_a_heading_embed_next_sibling() {
    let o = "# A\n\nbody o\n\n- x\n\n# B\n";
    let t = "# A\n\nbody t\n\n- x\n\n# B\n";
    let m = merge::merge_texts(o, t, "phone", "20260926-1200");
    assert_eq!(m.conflicts, 1);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), &m.text).unwrap();
    for (name, text) in &m.conflict_blocks {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let v = Vault::open(dir.path()).unwrap();
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1, "{}", m.text);
    assert_eq!(v.tree.node(pairs[0].0).title, "A");
    // x is still A's child, and B is still top-level
    assert!(v.find_by_path(&["A".into(), "x".into()]).is_some());
    assert!(v.find_by_path(&["B".into()]).is_some());
    assert_ordered(&v);
}

fn vault_with_block(root: &str, block: &str) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), root).unwrap();
    std::fs::write(
        dir.path().join("racfer~s.md"),
        format!("---\nid: racfer-hattes-mislup-nodrys\n---\n\n{}", block),
    )
    .unwrap();
    let v = Vault::open(dir.path()).unwrap();
    (dir, v)
}

#[test]
fn refiling_a_section_block_relevels_its_heading_embed() {
    let (_d, mut v) = vault_with_block(
        "# A\n\n## ![[racfer-hattes-mislup-nodrys]]\n\n# B\n\n## C\n",
        "# S\n\nbody\n",
    );
    let dest = at(&v, "B/C");
    let r = at(&v, "A/S");
    ops::refile(&mut v, r, dest).unwrap();
    assert_eq!(root_text(&v), "# A\n\n# B\n\n## C\n\n### ![[racfer-hattes-mislup-nodrys]]\n");
    assert!(v.find_by_path(&["B".into(), "C".into(), "S".into()]).is_some());
}

#[test]
fn promote_out_of_a_block_root_lands_beside_its_embed() {
    let (_d, mut v) = vault_with_block(
        "# A\n\n- x\n![[racfer-hattes-mislup-nodrys]]\n- y\n",
        "- s\n  - kid\n",
    );
    let r = at(&v, "A/s/kid");
    ops::promote(&mut v, r).unwrap();
    let kids: Vec<String> = v
        .tree
        .resolved_children(at(&v, "A"))
        .iter()
        .map(|&c| v.tree.node(c).title.clone())
        .collect();
    assert_eq!(kids, ["x", "s", "kid", "y"], "{}", root_text(&v));
    assert!(v.tree.resolved_children(at(&v, "A/s")).is_empty());
}

// ------------------------------------------------------------- adoption

#[test]
fn phone_appends_to_an_item_block_are_adopted() {
    // a phone editor appends `- new` at column 0 to an Inbox block spelled
    // as a bullet (§4.9): it becomes the root's child, not a second root
    let (_d, mut v) = vault_with_block(
        "# A\n\n![[racfer-hattes-mislup-nodrys]]\n",
        "- Inbox\n  - old\n- new\nloose text\n",
    );
    let kids: Vec<String> = v
        .tree
        .resolved_children(at(&v, "A/Inbox"))
        .iter()
        .map(|&c| v.tree.node(c).title.clone())
        .collect();
    assert_eq!(kids, ["old", "new"]);
    let inbox = at(&v, "A/Inbox");
    let text = v.tree.node(inbox).text_lines(&v.tree.files[inbox.0].text);
    assert!(text.contains(&"loose text"), "{:?}", text);
    let diags = check::check(&v);
    assert!(diags.iter().all(|d| !d.message.contains("exactly one node")));
    assert!(diags.iter().any(|d| d.message.contains("adopted")));
    // --fix writes it canonical: the adopted lines nest under the root
    check::fix(&mut v).unwrap();
    let inbox = at(&v, "A/Inbox");
    assert!(
        v.tree.files[inbox.0].text.ends_with("- Inbox\n  - old\n  - new\n\n  loose text\n"),
        "{}",
        v.tree.files[inbox.0].text
    );
}

#[test]
fn a_second_heading_in_a_section_block_is_adopted() {
    let (_d, v) = vault_with_block(
        "# A\n\n## ![[racfer-hattes-mislup-nodrys]]\n",
        "# S\n\nbody\n\n# T\n",
    );
    assert!(v.find_by_path(&["A".into(), "S".into(), "T".into()]).is_some());
}

// ------------------------------------------------------------- node keys

#[test]
fn keys_tell_same_titled_siblings_apart() {
    let (_d, mut v) = vault_with("# P\n\n- same\n- same\n- \n- \n");
    let kids = v.tree.resolved_children(at(&v, "P"));
    let keys: Vec<_> = kids.iter().map(|&k| v.key_of(k)).collect();
    for (i, k) in keys.iter().enumerate() {
        assert_eq!(v.find_by_key(k), Some(kids[i]));
        assert!(keys.iter().filter(|o| *o == k).count() == 1);
    }
    // the second "same" survives an edit to the first by key
    let first = kids[0];
    ops::toggle_task(&mut v, first).unwrap();
    let second = v.find_by_key(&keys[1]).unwrap();
    assert_eq!(v.tree.node(second).task, None);
    assert_eq!(v.tree.resolved_children(at(&v, "P"))[1], second);
}

#[test]
fn keys_of_nodes_inside_nested_blocks_resolve() {
    let (_d, v) = vault_with_block(
        "# A\n\n## B\n\n### ![[racfer-hattes-mislup-nodrys]]\n",
        "# S\n\n- deep\n",
    );
    let deep = at(&v, "A/B/S/deep");
    let key = v.key_of(deep);
    assert_eq!(v.find_by_key(&key), Some(deep));
}

#[test]
fn check_reports_and_fixes_heading_embed_levels() {
    let (_d, mut v) = vault_with_block(
        "# A\n\n#### ![[racfer-hattes-mislup-nodrys]]\n\n## B\n",
        "# S\n",
    );
    let before = parents(&v);
    assert!(check::check(&v).iter().any(|d| d.message.contains("level 4 where its position gives 2")));
    check::fix(&mut v).unwrap();
    assert_eq!(root_text(&v), "# A\n\n## ![[racfer-hattes-mislup-nodrys]]\n\n## B\n");
    assert_eq!(parents(&v), before);
    assert!(check::check(&v).iter().all(|d| !d.message.contains("heading embed at level")));
}

#[test]
fn placement_as_asked_is_not_reported_as_moved() {
    let (_d, mut v) = vault_with("# P\n\n- a\n- b\n");
    let r = at(&v, "P/a");
    assert!(!ops::paste(&mut v, r, "- new\n", true).unwrap());
    let kids: Vec<String> = v
        .tree
        .resolved_children(at(&v, "P"))
        .iter()
        .map(|&c| v.tree.node(c).title.clone())
        .collect();
    assert_eq!(kids, ["a", "new", "b"]);
}

// ------------------------------------------------------------- drag and drop

#[test]
fn drop_before_and_into() {
    let (_d, mut v) = vault_with("# A\n\n- a1\n- a2\n\n# B\n\n- b1\n");
    let (r, t) = (at(&v, "B/b1"), at(&v, "A/a2"));
    ops::move_node(&mut v, r, t, ops::Drop::Before).unwrap();
    assert_eq!(root_text(&v), "# A\n\n- a1\n- b1\n- a2\n\n# B\n");
    let (r, t) = (at(&v, "A/a1"), at(&v, "B"));
    ops::move_node(&mut v, r, t, ops::Drop::Into).unwrap();
    assert!(v.find_by_path(&["B".into(), "a1".into()]).is_some(), "{}", root_text(&v));
    assert_ordered(&v);
}

#[test]
fn drop_refuses_own_subtree() {
    let src = "# A\n\n- a1\n  - deep\n";
    let (_d, mut v) = vault_with(src);
    let (r, t) = (at(&v, "A"), at(&v, "A/a1/deep"));
    assert!(ops::move_node(&mut v, r, t, ops::Drop::Before).is_err());
    let (r, t) = (at(&v, "A/a1"), at(&v, "A/a1/deep"));
    assert!(ops::move_node(&mut v, r, t, ops::Drop::Into).is_err());
    assert_eq!(root_text(&v), src);
}

#[test]
fn drop_section_before_an_item_is_clamped() {
    let (_d, mut v) = vault_with("# P\n\n- a\n- b\n\n# S\n");
    let (r, t) = (at(&v, "S"), at(&v, "P/a"));
    assert!(ops::move_node(&mut v, r, t, ops::Drop::Before).unwrap());
    assert_ordered(&v);
    assert!(v.find_by_path(&["P".into(), "S".into()]).is_some(), "{}", root_text(&v));
}

#[test]
fn drop_before_a_later_sibling_lands_before_it() {
    let src = "# A\n\n- a\n- b\n- c\n";
    let (_d, mut v) = vault_with(src);
    let (r, t) = (at(&v, "A/a"), at(&v, "A/c"));
    assert!(!ops::move_node(&mut v, r, t, ops::Drop::Before).unwrap());
    assert_eq!(root_text(&v), "# A\n\n- b\n- a\n- c\n");
    // dropping a node before its immediate next sibling leaves it in place,
    // in a tight list or a loose one (§4.2: preserved as found)
    for src in [src, "# A\n\n- a\n\n- b\n\n- c\n"] {
        let (_d, mut v) = vault_with(src);
        let (r, t) = (at(&v, "A/a"), at(&v, "A/b"));
        ops::move_node(&mut v, r, t, ops::Drop::Before).unwrap();
        assert_eq!(root_text(&v), src);
    }
}

#[test]
fn respelled_item_does_not_adopt_deeper_written_sibling_sections() {
    // top-level sections written at `##`: honoured as written, still the root's
    let (_d, mut v) = vault_with("- note\n\n## Projects\n\n## Areas\n");
    let before = parents(&v);
    assert!(v.find_by_path(&["Projects".into()]).is_some());
    let r = at(&v, "note");
    ops::toggle_spelling(&mut v, r).unwrap();
    // `~`: "so no sibling changes parent" (§3.1, §10.3)
    assert_eq!(others(&parents(&v), "note"), others(&before, "note"), "{}", root_text(&v));
    assert!(v.find_by_path(&["Projects".into()]).is_some(), "{}", root_text(&v));
    assert_ordered(&v);
}

#[test]
fn pasted_section_does_not_adopt_deeper_written_next_sibling() {
    let (_d, mut v) = vault_with("## A\n\n## B\n");
    let before = parents(&v);
    let r = at(&v, "A");
    ops::paste(&mut v, r, "# T\n\n## T1\n", true).unwrap();
    // T lands between A and B as a sibling, its child one below it; B keeps
    // the root as its parent
    assert_eq!(root_text(&v), "## A\n\n## T\n\n### T1\n\n## B\n");
    assert_eq!(others(&others(&parents(&v), "T"), "T1"), others(&before, "T"));
    assert!(v.find_by_path(&["T".into(), "T1".into()]).is_some(), "{}", root_text(&v));
}

#[test]
fn promoted_section_does_not_adopt_deeper_written_next_sibling() {
    // `<` on S lands it between P and R, top-level sections written at `##`
    let (_d, mut v) = vault_with("## P\n\n### S\n\n## R\n");
    let before = parents(&v);
    let s = at(&v, "P/S");
    ops::promote(&mut v, s).unwrap();
    assert_eq!(root_text(&v), "## P\n\n## S\n\n## R\n");
    assert_eq!(others(&parents(&v), "S"), others(&before, "S"));
}

#[test]
fn fixing_a_heading_embed_level_keeps_following_sections_in_place() {
    // B is a sibling of the embed under A; making the embed shallower must
    // not make B nest under it (§4.7: "the structure does not change")
    let (_d, mut v) = vault_with_block(
        "# A\n\n### ![[racfer-hattes-mislup-nodrys]]\n\n### B\n\n#### C\n",
        "# S\n",
    );
    let before = parents(&v);
    assert!(before.contains(&("B".to_string(), "A".to_string())), "{:?}", before);
    assert!(check::check(&v).iter().any(|d| d.message.contains("level 3 where its position gives 2")));
    check::fix(&mut v).unwrap();
    assert_eq!(parents(&v), before, "{}", root_text(&v));
    assert_eq!(root_text(&v), "# A\n\n## ![[racfer-hattes-mislup-nodrys]]\n\n## B\n\n#### C\n");
    assert!(check::check(&v).iter().all(|d| !d.message.contains("embed has children")
        && !d.message.contains("heading embed at level")));
    // a parent section written deeper than its own position leaves the
    // embed no shallower level that keeps it there: it stays, reported
    let root = "# A\n\n#### B\n\n###### ![[racfer-hattes-mislup-nodrys]]\n";
    let (_d, mut v) = vault_with_block(root, "# S\n");
    let before = parents(&v);
    assert!(before.contains(&("S".to_string(), "B".to_string())), "{:?}", before);
    check::fix(&mut v).unwrap();
    assert_eq!(parents(&v), before, "{}", root_text(&v));
    assert_eq!(root_text(&v), root);
}

#[test]
fn move_up_past_a_deeper_written_sibling_keeps_parents() {
    // `### Note` skips a level but is still Doc's child, a sibling of Part
    let (_d, mut v) = vault_with("# Doc\n\n### Note\n\n## Part\n");
    assert_eq!(v.tree.node(at(&v, "Doc/Note")).kind, Kind::Section);
    let before = others(&parents(&v), "Part");
    let r = at(&v, "Doc/Part");
    ops::move_sibling(&mut v, r, false).unwrap();
    assert_eq!(others(&parents(&v), "Part"), before, "{}", root_text(&v));
    assert_ordered(&v);
    // the node moving down takes the level of the one moving up, and its
    // subtree comes with it
    let (_d, mut v) = vault_with("# Doc\n\n### Note\n\ntext\n\n#### Sub\n\n## Part\n");
    let before = parents(&v);
    let r = at(&v, "Doc/Part");
    ops::move_sibling(&mut v, r, false).unwrap();
    assert_eq!(root_text(&v), "# Doc\n\n## Part\n\n## Note\n\ntext\n\n### Sub\n");
    assert_eq!(parents(&v), before);
    // siblings all written deeper stay as written: none nests under another
    let src = "# Doc\n\n### A\n\n### B\n\n### C\n";
    let (_d, mut v) = vault_with(src);
    let before = parents(&v);
    let r = at(&v, "Doc/A");
    ops::move_sibling(&mut v, r, true).unwrap();
    assert_eq!(root_text(&v), "# Doc\n\n### B\n\n### A\n\n### C\n");
    assert_eq!(parents(&v), before);
    // an item indented deeper than the sibling after it (accepted on read)
    let (_d, mut v) = vault_with("- P\n    - a\n      - kid\n  - b\n");
    let before = parents(&v);
    let r = at(&v, "P/b");
    ops::move_sibling(&mut v, r, false).unwrap();
    assert_eq!(parents(&v), before, "{}", root_text(&v));
    assert_eq!(root_text(&v), "- P\n  - b\n  - a\n    - kid\n");
}

// P sits under an item under `# S`, so its position gives it level 2 while
// it and its sections are written a level shallower (§3.1: honoured as
// written). A section written after one of them at the level P's position
// gives would parse as that one's child.
const SHALLOW: &str = "# S\n\n- i\n  # P\n  ## C\n";

#[test]
fn section_refiled_after_a_shallower_written_sibling_is_not_adopted_by_it() {
    let (_d, mut v) = vault_with(&format!("{}\n# X\n", SHALLOW));
    let before = others(&parents(&v), "X");
    let (x, p) = (at(&v, "X"), at(&v, "S/i/P"));
    ops::refile(&mut v, x, p).unwrap();
    assert!(v.find_by_path(&["S".into(), "i".into(), "P".into(), "X".into()]).is_some(), "{}", root_text(&v));
    assert_eq!(others(&parents(&v), "X"), before, "{}", root_text(&v));
    assert_eq!(root_text(&v), format!("{}\n  ## X\n", SHALLOW));
    assert_ordered(&v);
}

#[test]
fn section_pasted_between_shallower_written_siblings_is_not_adopted() {
    // no deeper than C before it, no shallower than D after it
    let src = format!("{}  ## D\n", SHALLOW);
    let (_d, mut v) = vault_with(&src);
    let before = parents(&v);
    let c = at(&v, "S/i/P/C");
    ops::paste(&mut v, c, "# T\n", true).unwrap();
    assert!(v.find_by_path(&["S".into(), "i".into(), "P".into(), "T".into()]).is_some(), "{}", root_text(&v));
    assert_eq!(others(&parents(&v), "T"), before, "{}", root_text(&v));
    assert_ordered(&v);
}

#[test]
fn new_last_child_section_after_a_shallower_written_sibling_is_not_adopted() {
    // N on P: a section beside C, not under it
    let (_d, mut v) = vault_with(SHALLOW);
    let before = parents(&v);
    let p = at(&v, "S/i/P");
    let x = ops::append_child_public(&mut v, p, "x").unwrap();
    assert_eq!(v.tree.node(x).title, "x");
    assert!(v.find_by_path(&["S".into(), "i".into(), "P".into(), "x".into()]).is_some(), "{}", root_text(&v));
    assert_eq!(others(&parents(&v), "x"), before, "{}", root_text(&v));
}

#[test]
fn making_a_block_of_a_section_after_a_shallower_written_sibling_keeps_its_parent() {
    // the embed replaces P where it is written; at the level P's position
    // gives it would parse as C's child
    let (_d, mut v) = vault_with("# S\n\n- i\n  # C\n  # P\n");
    let before = parents(&v);
    let p = at(&v, "S/i/P");
    ops::make_block(&mut v, p).unwrap();
    assert_eq!(parents(&v), before, "{}", root_text(&v));
}

#[test]
fn capture_after_a_shallower_written_day_starts_a_day_beside_it() {
    // an Inbox spelled as an item, its day written by hand at `#`
    let (_d, mut v) = vault_with("- Inbox\n  # 2000-01-01\n  - old\n");
    let r = ops::capture(&mut v, "new", false).unwrap();
    let path = v.tree.path(r);
    assert_eq!(path.len(), 3, "{:?}\n{}", path, root_text(&v));
    assert_eq!(path[0], "Inbox");
    assert_ne!(path[1], "2000-01-01", "{}", root_text(&v));
    assert!(v.find_by_path(&["Inbox".into(), "2000-01-01".into(), "old".into()]).is_some());
}

#[test]
fn drop_before_the_next_sibling_in_a_four_space_list_keeps_it_a_sibling() {
    // §4.2: 4-space nesting is accepted on read. Dropping a before b, its
    // next sibling, leaves it where it was (a no-op); b must not become a's
    // child because a was rewritten at two spaces in front of it
    let (_d, mut v) = vault_with("# T\n\n- P\n    - a\n    - b\n");
    let (a, b) = (at(&v, "T/P/a"), at(&v, "T/P/b"));
    ops::move_node(&mut v, a, b, ops::Drop::Before).unwrap();
    assert!(v.find_by_path(&["T".into(), "P".into(), "b".into()]).is_some(), "{}", root_text(&v));
    assert!(v.find_by_path(&["T".into(), "P".into(), "a".into(), "b".into()]).is_none(), "{}", root_text(&v));
    assert_eq!(root_text(&v), "# T\n\n- P\n    - a\n    - b\n");
}

#[test]
fn a_node_placed_before_a_sibling_written_deeper_keeps_it_a_sibling() {
    // siblings nested with a tab or 4 spaces (§4.2), and items indented
    // under a section: a node pasted or dragged in front of one goes at
    // its indent, not at the canonical one, where the sibling would parse
    // as its child
    for src in [
        "# T\n\n- P\n    - a\n    - b\n",
        "# T\n\n- P\n\t- a\n\t- b\n",
        "# T\n\n- P\n    # a\n    # b\n",
        "# T\n\n## P\n\n  - a\n  - b\n",
    ] {
        let (_d, mut v) = vault_with(src);
        let before = parents(&v);
        let b = at(&v, "T/P/b");
        let item = v.tree.node(b).kind == Kind::Item;
        ops::paste(&mut v, b, if item { "- c\n" } else { "# c\n" }, false).unwrap();
        assert_eq!(others(&parents(&v), "c"), before, "paste in {:?}:\n{}", src, root_text(&v));
        assert!(parents(&v).contains(&("c".into(), "P".into())), "{}", root_text(&v));
        let (_d, mut v) = vault_with(src);
        let (a, b) = (at(&v, "T/P/a"), at(&v, "T/P/b"));
        ops::move_node(&mut v, b, a, ops::Drop::Before).unwrap();
        assert_eq!(parents(&v), before, "drag in {:?}:\n{}", src, root_text(&v));
        let p = at(&v, "T/P");
        let kids: Vec<String> =
            v.tree.resolved_children(p).iter().map(|&k| v.tree.node(k).title.clone()).collect();
        assert_eq!(kids, ["b", "a"], "{}", root_text(&v));
    }
    // an item before a section child written deeper
    let (_d, mut v) = vault_with("# T\n\n- P\n    # a\n");
    let before = parents(&v);
    let a = at(&v, "T/P/a");
    ops::paste(&mut v, a, "- c\n", false).unwrap();
    assert_eq!(others(&parents(&v), "c"), before, "{}", root_text(&v));
    assert!(parents(&v).contains(&("c".into(), "P".into())), "{}", root_text(&v));
}

#[test]
fn making_a_block_after_a_shallower_sibling_written_deeper_keeps_the_next_sibling() {
    // Notes is written with a tab (§4.2: accepted on read), Meeting and
    // Followup at the app's two spaces: all three are children of Project.
    // Notes' shallower level says nothing about Meeting's embed, which sits
    // at a smaller indent; clamped to it, the embed takes Followup in.
    let (_d, mut v) = vault_with("## S\n\n- Project\n\t# Notes\n  ### Meeting\n  ### Followup\n");
    let before = parents(&v);
    assert!(before.contains(&("Followup".into(), "Project".into())));
    let m = at(&v, "S/Project/Meeting");
    ops::make_block(&mut v, m).unwrap();
    assert_eq!(parents(&v), before, "{}", root_text(&v));
}

#[test]
fn a_section_placed_after_a_shallower_sibling_written_deeper_keeps_the_next_sibling() {
    // the same for a section pasted between Notes and Meeting, or after
    // Notes when it is Project's last child: Notes, closed by indentation,
    // bounds neither
    let (_d, mut v) = vault_with("## S\n\n- Project\n\t# Notes\n  ### Meeting\n  ### Followup\n");
    let before = parents(&v);
    let notes = at(&v, "S/Project/Notes");
    ops::paste(&mut v, notes, "# X\n", true).unwrap();
    assert_eq!(others(&parents(&v), "X"), before, "{}", root_text(&v));
    assert!(parents(&v).contains(&("X".into(), "Project".into())), "{}", root_text(&v));
    let (_d, mut v) = vault_with("## S\n\n- Project\n  ### Meeting\n\t# Notes\n");
    let notes = at(&v, "S/Project/Notes");
    ops::paste(&mut v, notes, "# X\n", true).unwrap();
    assert!(root_text(&v).ends_with("\t# Notes\n\n  ### X\n"), "{}", root_text(&v));
}

#[test]
fn refile_into_a_section_followed_by_an_indented_setext_heading() {
    // a setext heading is read (§4.2) as the ATX heading of its level:
    // `  Sub\n  ===` after `## P` is where `  # Sub` would be, a level-1
    // heading that no indent puts under a section. Read as P's child, it
    // made the verbs that write it, or write beside it, at its level 1
    // (refile into P, `t` on it) move nodes out of P and Top
    const SETEXT: &str = "# Top\n\n## P\n\n  Sub\n  ===\n\n# X\n";
    let (_d, mut v) = vault_with(SETEXT);
    let before = parents(&v);
    assert_eq!(before, parents(&vault_with("# Top\n\n## P\n\n  # Sub\n\n# X\n").1));
    let (x, p) = (at(&v, "X"), at(&v, "Top/P"));
    ops::refile(&mut v, x, p).unwrap();
    assert!(v.find_by_path(&["Top".into(), "P".into(), "X".into()]).is_some(), "{}", root_text(&v));
    assert_eq!(others(&parents(&v), "X"), others(&before, "X"), "{}", root_text(&v));
    let (_d, mut v) = vault_with(SETEXT);
    let sub = at(&v, "Sub");
    ops::toggle_taskness(&mut v, sub).unwrap();
    assert_eq!(parents(&v), before, "{}", root_text(&v));
    // under an item, indentation nests it, as it does an ATX heading
    let (_d, v) = vault_with("# Top\n\n- i\n\n  Sub\n  ===\n");
    assert_eq!(parents(&v), parents(&vault_with("# Top\n\n- i\n\n  # Sub\n").1));
    assert!(parents(&v).contains(&("Sub".into(), "i".into())));
}

#[test]
fn drop_second_of_two_loose_items_before_the_first_keeps_the_list_loose() {
    // §4.2: loose/tight lists are preserved as found
    let (_d, mut v) = vault_with("# A\n\n- a\n\n- b\n");
    let (b, a) = (at(&v, "A/b"), at(&v, "A/a"));
    ops::move_node(&mut v, b, a, ops::Drop::Before).unwrap();
    assert_eq!(root_text(&v), "# A\n\n- b\n\n- a\n");
    // and dropping the first before the second, where it is, changes nothing
    let (_d, mut v) = vault_with("# A\n\n- a\n\n- b\n");
    let (a, b) = (at(&v, "A/a"), at(&v, "A/b"));
    ops::move_node(&mut v, a, b, ops::Drop::Before).unwrap();
    assert_eq!(root_text(&v), "# A\n\n- a\n\n- b\n");
    // a tight list stays tight
    let (_d, mut v) = vault_with("# A\n\n- a\n- b\n");
    let (b, a) = (at(&v, "A/b"), at(&v, "A/a"));
    ops::move_node(&mut v, b, a, ops::Drop::Before).unwrap();
    assert_eq!(root_text(&v), "# A\n\n- b\n- a\n");
}
