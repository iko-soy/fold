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
    ops::paste(&mut v, r, "- new\n", true).unwrap();
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
