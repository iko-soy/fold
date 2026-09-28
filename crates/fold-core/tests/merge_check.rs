use fold_core::merge;
use fold_core::vault::Vault;

#[test]
fn merge_identical_is_identity() {
    let o = "# A\n\n- one\n- two\n";
    let out = merge::merge_texts(o, o, "dev", "2026-09-12-100000");
    assert_eq!(out.conflicts, 0);
    assert!(out.conflict_blocks.is_empty());
    assert!(out.text.contains("- one"), "{}", out.text);
    assert!(out.text.contains("- two"), "{}", out.text);
}

#[test]
fn merge_keeps_every_node_once() {
    let o = "# A\n\n- one\n- two\n";
    let t = "# A\n\n- one\n- three\n";
    let out = merge::merge_texts(o, t, "dev", "ts");
    assert!(out.text.contains("- one"));
    assert!(out.text.contains("- two"));
    assert!(out.text.contains("- three"));
}

#[test]
fn merge_field_difference_raises_conflict_pair() {
    let o = "# A\n\n- [ ] task\n";
    let t = "# A\n\n- [x] task\n";
    let out = merge::merge_texts(o, t, "phone", "2026-09-12-100000");
    assert_eq!(out.conflicts, 1);
    assert_eq!(out.conflict_blocks.len(), 1);
    let (fname, text) = &out.conflict_blocks[0];
    assert!(fname.ends_with("~task.md"), "{}", fname);
    assert!(text.contains("conflict: \"phone 2026-09-12-100000\""), "{}", text);
    assert!(text.contains("- [x] task"), "{}", text);
    // ours stays in place, embed goes right after
    assert!(out.text.contains("- [ ] task"), "{}", out.text);
    assert!(out.text.contains("![["), "{}", out.text);
}

#[test]
fn merge_body_difference_conflicts() {
    let o = "# Note\n\nversion one\n";
    let t = "# Note\n\nversion two\n";
    let out = merge::merge_texts(o, t, "d", "ts");
    assert_eq!(out.conflicts, 1);
    assert!(out.text.contains("version one"));
    assert!(out.conflict_blocks[0].1.contains("version two"));
}

#[test]
fn merge_is_idempotent_on_output() {
    let o = "# A\n\n- one\n- [ ] t\n";
    let t = "# A\n\n- two\n- [x] t\n";
    let first = merge::merge_texts(o, t, "d", "ts");
    // merging the output with itself is a no-op
    let second = merge::merge_texts(&first.text, &first.text, "d", "ts");
    assert_eq!(second.conflicts, 0);
    assert!(second.conflict_blocks.is_empty());
}

#[test]
fn sync_conflict_files_merge() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- ours\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n- theirs\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    let outcomes = merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert_eq!(outcomes.len(), 1);
    let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(text.contains("- ours"), "{}", text);
    assert!(text.contains("- theirs"), "{}", text);
    assert!(!dir.path().join("root.sync-conflict-20260912-100000-phone.md").exists());
}

#[test]
fn prefix_collision_renames_instead_of_merging() {
    let dir = tempfile::tempdir().unwrap();
    let id1 = fold_core::Id::parse("racfer-hattes-mislup-nodrys").unwrap();
    let id2 = fold_core::Id::parse("racfer-wolsun-dozzod-binwes").unwrap();
    std::fs::write(
        dir.path().join("root.md"),
        "# A\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("racfer~notes.md"),
        format!("---\nid: {}\n---\n\n# Notes\n", id1),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("racfer~notes.sync-conflict-20260912-100000-phone.md"),
        format!("---\nid: {}\n---\n\n# Other\n", id2),
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    let outcomes = merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert!(outcomes[0].contains("prefix collision"), "{:?}", outcomes);
    // the conflict file was renamed with a longer prefix
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("racfer-wolsun~")),
        "{:?}",
        names
    );
}

#[test]
fn check_reports_broken_embed_and_fix_canonicalizes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("root.md"),
        "# A\n\n![[racfer-hattes-mislup-nodrys]]\n\n* [X] old style\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    let diags = fold_core::check::check(&v);
    assert!(diags.iter().any(|d| d.message.contains("broken embed")), "{:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
    assert!(diags.iter().any(|d| d.message.contains("non-canonical")), "");
    let n = fold_core::check::fix(&mut v).unwrap();
    assert!(n > 0);
    let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(text.contains("- [x] old style"), "{}", text);
}

#[test]
fn check_reports_ignored_md_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n").unwrap();
    std::fs::write(dir.path().join("foreign.md"), "# no id here\n").unwrap();
    let v = Vault::open(dir.path()).unwrap();
    let diags = fold_core::check::check(&v);
    assert!(diags.iter().any(|d| d.file == "foreign.md" && d.message.contains("ignored")));
}

#[test]
fn check_reports_bad_dates() {
    let dir = tempfile::tempdir().unwrap();
    let id = fold_core::Id::parse("racfer-hattes-mislup-nodrys").unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n").unwrap();
    std::fs::write(
        dir.path().join("racfer~task.md"),
        format!("---\nid: {}\ndue: next friday\n---\n\n- [ ] task\n", id),
    )
    .unwrap();
    let v = Vault::open(dir.path()).unwrap();
    let diags = fold_core::check::check(&v);
    assert!(diags.iter().any(|d| d.message.contains("not an ISO date")), "{:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
}

// ---------------------------------------------------------------- regressions

const BID: &str = "racfer-hattes-mislup-nodrys";

#[test]
fn merge_of_identical_texts_is_byte_identical() {
    // §15.6: merge(O, O) == O, loose lists and all
    for o in [
        "# A\n\n- a\n\n- b\n",
        "---\nvault: x\n---\n\nIntro.\n\n# A\n\nbody\n\n## B\n\n- one\n  - two\n",
        "# A\n\n- [ ] t\n\n# B\n",
    ] {
        let out = merge::merge_texts(o, o, "d", "ts");
        assert_eq!(out.conflicts, 0);
        assert_eq!(out.text, o);
    }
}

#[test]
fn merge_block_file_keeps_frontmatter_out_of_the_tree() {
    let o = format!("---\nid: {}\ndue: 2026-09-20\n---\n\n- t\n", BID);
    let t = format!("---\nid: {}\ndue: 2026-09-20\n---\n\n- t\n  extra line\n", BID);
    let out = merge::merge_texts(&o, &t, "d", "ts");
    assert_eq!(out.conflicts, 1);
    assert!(!out.text.contains("# due"), "{}", out.text);
    assert!(out.text.starts_with(&format!("---\nid: {}\ndue: 2026-09-20\n---\n\n- t\n", BID)), "{}", out.text);
    // one column-0 node: the conflict embed is not placed inside the block file
    assert!(!out.text.contains("![["), "{}", out.text);
    assert_eq!(out.sibling_embeds.len(), 1);
}

#[test]
fn merge_block_frontmatter_difference_raises_conflict_with_their_props() {
    let o = format!("---\nid: {}\ndue: 2026-09-20\n---\n\n- t\n", BID);
    let t = format!("---\nid: {}\ndue: 2026-10-01\n---\n\n- t\n", BID);
    let out = merge::merge_texts(&o, &t, "phone", "ts");
    assert_eq!(out.conflicts, 1);
    assert_eq!(out.conflict_blocks.len(), 1);
    let cb = &out.conflict_blocks[0].1;
    assert!(cb.contains("due: 2026-10-01"), "{}", cb);
    assert!(cb.contains("conflict: \"phone ts\""), "{}", cb);
    assert!(!cb.contains(BID), "{}", cb);
    assert!(out.text.contains("due: 2026-09-20"), "{}", out.text);
}

#[test]
fn sync_conflict_on_block_places_embed_after_its_embed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), format!("# A\n\n![[{}]]\n\n# B\n", BID)).unwrap();
    std::fs::write(
        dir.path().join("racfer~t.md"),
        format!("---\nid: {}\n---\n\n- [ ] t\n", BID),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("racfer~t.sync-conflict-20260912-100000-phone.md"),
        format!("---\nid: {}\n---\n\n- [x] t\n", BID),
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let block = std::fs::read_to_string(dir.path().join("racfer~t.md")).unwrap();
    assert!(!block.contains("![["), "{}", block);
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1);
    assert_eq!(v.tree.node(pairs[0].0).block.as_ref().unwrap().id.as_ref().unwrap().to_string(), BID);
}

#[test]
fn section_conflict_pairs_with_the_section() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\nbody o\n\n- x\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\nbody t\n\n- x\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1, "{}", v.tree.files[0].text);
    assert_eq!(v.tree.node(pairs[0].0).title, "A");
    // no children: still listed
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\nbody o\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\nbody t\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1, "{}", v.tree.files[0].text);
    assert_eq!(v.tree.node(pairs[0].0).title, "A");
}

#[test]
fn keep_theirs_puts_checkbox_after_the_heading_marker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# Fix - thing\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# [ ] Fix - thing\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1, "{}", v.tree.files[0].text);
    merge::resolve_keep_theirs(&mut v, pairs[0].0, pairs[0].1).unwrap();
    let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(text.contains("# [ ] Fix - thing"), "{}", text);
}

#[test]
fn keep_ours_handles_multibyte_text_before_embed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "- café\n- other\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "- café\n- [x] other\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1);
    merge::resolve_keep_ours(&mut v, pairs[0].1).unwrap();
    let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(!text.contains("![["), "{}", text);
    assert!(text.contains("- café") && text.contains("- other"), "{}", text);
}

#[test]
fn retitled_block_root_conflicts_and_keep_theirs_keeps_the_filename() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), format!("# A\n\n![[{}]]\n", BID)).unwrap();
    std::fs::write(dir.path().join("racfer~old.md"), format!("---\nid: {}\n---\n\n- old\n", BID)).unwrap();
    std::fs::write(
        dir.path().join("racfer~old.sync-conflict-20260912-100000-phone.md"),
        format!("---\nid: {}\n---\n\n- new\n", BID),
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let block = std::fs::read_to_string(dir.path().join("racfer~old.md")).unwrap();
    assert_eq!(block, format!("---\nid: {}\n---\n\n- old\n", BID));
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1);
    merge::resolve_keep_theirs(&mut v, pairs[0].0, pairs[0].1).unwrap();
    // the title changed, the filename did not (§6.4)
    let text = std::fs::read_to_string(dir.path().join("racfer~old.md")).unwrap();
    assert!(text.contains(&format!("id: {}", BID)) && text.contains("- new"), "{}", text);
    assert!(merge::conflict_pairs(&v).is_empty());
}

#[test]
fn deep_headings_are_canonical() {
    // §3.1 / §4.2: no upper bound on heading level; levels beyond six are ours
    // (§4.2 "What is ours"), not non-canonical syntax for `notes check` to report.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("root.md"),
        "# 1\n\n## 2\n\n### 3\n\n#### 4\n\n##### 5\n\n###### 6\n\n####### 7\n",
    )
    .unwrap();
    let v = Vault::open(dir.path()).unwrap();
    let diags = fold_core::check::check(&v);
    assert!(
        diags.is_empty(),
        "{:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn crlf_separated_text_is_not_flagged_or_padded() {
    // "text" already follows `a` after a blank line; only the line endings
    // are CRLF, which is read cleanly (§4.2) and must not count as missing
    // the separator, nor make --fix invent a second blank line (§3.3).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\r\n\r\n- a\r\n\r\ntext\r\n").unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    let diags = fold_core::check::check(&v);
    assert!(
        !diags.iter().any(|d| d.message.contains("text right after a child node")),
        "{:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    fold_core::check::fix(&mut v).unwrap();
    let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert_eq!(text, "# A\n\n- a\n\ntext\n");
}

#[test]
fn merge_rewrite_keeps_embeds() {
    // §12.4: embeds match by id and every node from O and T survives; a
    // re-rendered merge (T adds a node) must not turn `![[id]]` into `- `
    let o = format!("# A\n\n![[{}]]\n\n# B\n", BID);
    let t = format!("# A\n\n![[{}]]\n\n# B\n\n- new\n", BID);
    let out = merge::merge_texts(&o, &t, "d", "ts");
    assert!(out.text.contains(&format!("![[{}]]", BID)), "{}", out.text);
    assert!(out.text.contains("- new"), "{}", out.text);
    // a T-only embed (block made on the other device) must survive too
    let o2 = "# A\n\n- x\n";
    let t2 = format!("# A\n\n- x\n![[{}]]\n", BID);
    let out2 = merge::merge_texts(o2, &t2, "d", "ts");
    assert!(out2.text.contains(&format!("![[{}]]", BID)), "{}", out2.text);
    // heading form
    let o3 = format!("# A\n\n## ![[{}]]\n\n# B\n", BID);
    let t3 = format!("# A\n\n## ![[{}]]\n\n# B\n\n- new\n", BID);
    let out3 = merge::merge_texts(&o3, &t3, "d", "ts");
    assert!(out3.text.contains(&format!("## ![[{}]]", BID)), "{}", out3.text);
    // idempotent (§12.4): merging T into the result again adds nothing
    for (out, t) in [(&out, &t), (&out2, &t2), (&out3, &t3)] {
        assert_eq!(merge::merge_texts(&out.text, t, "d", "ts").text, out.text);
    }
}

#[test]
fn sync_conflict_merge_keeps_embeds_in_the_merged_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), format!("# A\n\n![[{}]]\n\n# B\n", BID)).unwrap();
    std::fs::write(dir.path().join("racfer~t.md"), format!("---\nid: {}\n---\n\n- t\n", BID)).unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        format!("# A\n\n![[{}]]\n\n# B\n\n- new\n", BID),
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(text.contains(&format!("![[{}]]", BID)), "{}", text);
    assert!(text.contains("- new"), "{}", text);
}

#[test]
fn a_copy_from_before_a_block_was_moved_is_merged_without_its_embed() {
    // the block was cut from NAS in the editor, which wrote root.md
    // without its embed, and is not pasted yet: moved on this device. The
    // phone's copy, from before the cut, still embeds it in NAS; its own
    // change comes in, the embed does not, nor into NAS's conflict copy
    // where the phone changed NAS's text too
    let ours = "# Homelab\n\n## NAS\n\nMirrored pairs.\n\n### [x] Replace fan\n\n# Inbox\n\n- [ ] a\n";
    let before = ours.replace("### [x]", &format!("### ![[{}]]\n\n### [x]", BID));
    for nas in ["Mirrored pairs.", "Snapshots every 2h."] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("root.md"), ours).unwrap();
        let block = dir.path().join("racfer~snapshot-policy.md");
        let text = format!("---\nid: {}\n---\n\n# [ ] Snapshot policy\n\n- hourly\n", BID);
        std::fs::write(&block, &text).unwrap();
        let copy = "root.sync-conflict-20260927-101010-PHONE77.md";
        let theirs = before.replace("- [ ] a\n", "- [ ] a\n- [ ] from phone\n").replace("Mirrored pairs.", nas);
        std::fs::write(dir.path().join(copy), theirs).unwrap();
        let mut v = Vault::open(dir.path()).unwrap();
        let moving = [fold_core::ident::Id::parse(BID).unwrap()];
        merge::merge_sync_conflicts_moving(&mut v, false, &moving).unwrap();
        assert!(!dir.path().join(copy).exists(), "{}", nas);
        let merged = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
        let embed = format!("![[{}]]", BID);
        assert!(!merged.contains(&embed), "{}: {}", nas, merged);
        assert_eq!(std::fs::read_to_string(&block).unwrap(), text, "{}", nas);
        if nas == "Mirrored pairs." {
            assert_eq!(merged, ours.replace("- [ ] a\n", "- [ ] a\n- [ ] from phone\n"));
            assert!(merge::conflict_pairs(&v).is_empty(), "{}", merged);
            continue;
        }
        assert!(merged.contains("- [ ] from phone\n"), "{}", merged);
        let pairs = merge::conflict_pairs(&v);
        assert_eq!(pairs.len(), 1, "{}", merged);
        let theirs = &v.tree.files[pairs[0].1 .0].text;
        assert!(theirs.contains("Snapshots every 2h.") && !theirs.contains(&embed), "{}", theirs);
    }
}

#[test]
fn merge_leaves_ignored_files_alone() {
    // §4.1/§11.4: a `.md` file without an id is ignored — never parsed,
    // never written. A sync-conflict copy of it is not ours to merge.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n").unwrap();
    std::fs::write(dir.path().join("readme.md"), "* [ ] first\n").unwrap();
    std::fs::write(
        dir.path().join("readme.sync-conflict-20260912-100000-phone.md"),
        "* [x] first\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    let outcomes = merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert!(outcomes.iter().any(|o| o.contains("left alone")), "{:?}", outcomes);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("readme.md")).unwrap(),
        "* [ ] first\n"
    );
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "readme.md".to_string(),
            "readme.sync-conflict-20260912-100000-phone.md".to_string(),
            "root.md".to_string(),
        ]
    );
    let diags = fold_core::check::check(&v);
    assert!(
        diags.iter().all(|d| !d.message.contains("unresolved conflict")),
        "{:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn merge_with_missing_base_keeps_theirs_frontmatter() {
    // X.md is gone (deleted, or its deletion synced in first): with no O to
    // merge against, T must survive with its frontmatter (id, due) intact
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), format!("# A\n\n![[{}]]\n", BID)).unwrap();
    let theirs = format!("---\nid: {}\ndue: 2026-09-20\n---\n\n- t\n", BID);
    let c = "racfer~t.sync-conflict-20260912-100000-phone.md";
    std::fs::write(dir.path().join(c), &theirs).unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let text = std::fs::read_to_string(dir.path().join("racfer~t.md")).unwrap();
    assert_eq!(text, theirs);
    assert!(!dir.path().join(c).exists());
    assert!(v.tree.block_by_id(&fold_core::Id::parse(BID).unwrap()).is_some());
    // a block renamed since (§6.4) is still O: found by its id, merged
    // into, never duplicated
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), format!("# A\n\n![[{}]]\n", BID)).unwrap();
    std::fs::write(dir.path().join("racfer~new.md"), format!("---\nid: {}\n---\n\n- [ ] t\n", BID)).unwrap();
    std::fs::write(dir.path().join(c), format!("---\nid: {}\n---\n\n- [x] t\n", BID)).unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert!(!dir.path().join("racfer~t.md").exists());
    assert_eq!(merge::conflict_pairs(&v).len(), 1, "{}", v.tree.files[0].text);
    // the copy of a foreign file that is gone is not ours either (§4.1)
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n").unwrap();
    let c = "readme.sync-conflict-20260912-100000-phone.md";
    std::fs::write(dir.path().join(c), "* [x] first\n").unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    let outcomes = merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert!(outcomes.iter().any(|o| o.contains("left alone")), "{:?}", outcomes);
    assert!(dir.path().join(c).exists() && !dir.path().join("readme.md").exists());
}

#[test]
fn merge_takes_a_block_that_came_in_with_its_copy_for_one() {
    // the vault as read before a sync brought a new block, its embed and
    // a copy of it in one go: X.md is a block on disk, not an ignored
    // file, and its copy is merged into it (§12.2)
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n").unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    std::fs::write(dir.path().join("root.md"), format!("# A\n\n![[{}]]\n", BID)).unwrap();
    std::fs::write(dir.path().join("racfer~t.md"), format!("---\nid: {}\n---\n\n- [ ] t\n", BID)).unwrap();
    let c = "racfer~t.sync-conflict-20260912-100000-phone.md";
    std::fs::write(dir.path().join(c), format!("---\nid: {}\n---\n\n- [x] t\n", BID)).unwrap();
    let outcomes = merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert!(!dir.path().join(c).exists(), "{:?}", outcomes);
    assert_eq!(merge::conflict_pairs(&v).len(), 1, "{}", v.tree.files[0].text);
}

#[test]
fn merge_places_insertions_by_their_neighbours() {
    // §12.4: a node present on one side only is an insertion, placed
    // relative to its matched neighbours — `new` sits between `a` and `b`
    let o = "# A\n\n- a\n- b\n- c\n";
    let t = "# A\n\n- a\n- new\n- b\n- c\n";
    let out = merge::merge_texts(o, t, "dev", "ts");
    assert_eq!(out.conflicts, 0, "{}", out.text);
    let p = |s: &str| out.text.find(s).unwrap();
    assert!(p("- a") < p("- new") && p("- new") < p("- b"), "{}", out.text);
    // one inserted after the text that follows its neighbour in T stays
    // after that text, so merging T in again changes nothing
    let o = "# P\n\n- a\n\nnote\n";
    let t = "# P\n\n- a\n\nnote\n\n- new\n";
    let out = merge::merge_texts(o, t, "dev", "ts");
    assert_eq!(out.conflicts, 0, "{}", out.text);
    assert_eq!(out.text, t);
}

#[test]
fn merge_places_inserted_sections_by_their_neighbours() {
    // the same for a section: `B` sits between `A` and `C`, not last
    let o = "# A\n\n# C\n";
    let t = "# A\n\n# B\n\n# C\n";
    let out = merge::merge_texts(o, t, "dev", "ts");
    assert_eq!(out.conflicts, 0, "{}", out.text);
    let p = |s: &str| out.text.find(s).unwrap();
    assert!(p("# A") < p("# B") && p("# B") < p("# C"), "{}", out.text);
}

#[test]
fn check_reports_embed_cycles() {
    // two blocks that embed each other: neither is reachable from root.md,
    // and §6.2 / §15.7 say a cycle is a diagnostic
    let a = "racfer-hattes-mislup-nodrys";
    let b = "dozzod-binwes-talsun-worbec";
    let c = "lacnum-walbyn-dirlyn-havtyp";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n").unwrap();
    std::fs::write(
        dir.path().join("racfer~x.md"),
        format!("---\nid: {}\n---\n\n- x\n  ![[{}]]\n", a, b),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("dozzod~y.md"),
        format!("---\nid: {}\n---\n\n- y\n  ![[{}]]\n  ![[{}]]\n", b, a, c),
    )
    .unwrap();
    // c hangs off the cycle without being on it
    std::fs::write(dir.path().join("lacnum~z.md"), format!("---\nid: {}\n---\n\n- z\n", c)).unwrap();
    let v = Vault::open(dir.path()).unwrap();
    // every block loads and every embed resolves: the only fault is the cycle
    for id in [a, b, c] {
        let id = fold_core::Id::parse(id).unwrap();
        assert!(v.tree.block_by_id(&id).is_some(), "block {} not loaded", id);
        assert!(v.tree.embed_of(&id).is_some(), "embed of {} not parsed", id);
    }
    let diags = fold_core::check::check(&v);
    let msgs: Vec<&String> = diags.iter().map(|d| &d.message).collect();
    assert_eq!(
        diags.iter().filter(|d| d.message.contains("cycl")).count(),
        2,
        "one cyclic-embed diagnostic per block on the cycle: {:?}",
        msgs
    );
    // the same blocks in a chain from root.md: no cycle
    std::fs::write(dir.path().join("root.md"), format!("# A\n\n![[{}]]\n", a)).unwrap();
    std::fs::write(
        dir.path().join("dozzod~y.md"),
        format!("---\nid: {}\n---\n\n- y\n  ![[{}]]\n", b, c),
    )
    .unwrap();
    let v = Vault::open(dir.path()).unwrap();
    let diags = fold_core::check::check(&v);
    assert!(diags.is_empty(), "{:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
}

#[test]
fn orphan_block_conflict_is_not_paired_with_an_unrelated_node() {
    // the block racfer~t.md is embedded nowhere: its conflict block has no
    // embed to follow, so it must not end up as the next sibling of some
    // unrelated node that 'keep theirs' would then overwrite (§12.4, §12.5)
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- keep me\n").unwrap();
    std::fs::write(
        dir.path().join("racfer~t.md"),
        format!("---\nid: {}\n---\n\n- [ ] t\n", BID),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("racfer~t.sync-conflict-20260912-100000-phone.md"),
        format!("---\nid: {}\n---\n\n- [x] t\n", BID),
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    let pairs = merge::conflict_pairs(&v);
    for &(ours, _) in &pairs {
        assert_ne!(v.tree.node(ours).title, "keep me", "root.md:\n{}", root);
    }
    // it pairs with the block it conflicts with, and keep theirs lands there
    assert_eq!(pairs.len(), 1, "root.md:\n{}", root);
    let bid = fold_core::Id::parse(BID).unwrap();
    assert_eq!(Some(pairs[0].0), v.tree.block_by_id(&bid), "root.md:\n{}", root);
    merge::resolve_keep_theirs(&mut v, pairs[0].0, pairs[0].1).unwrap();
    assert!(std::fs::read_to_string(dir.path().join("root.md")).unwrap().contains("- keep me"));
    let block = std::fs::read_to_string(dir.path().join("racfer~t.md")).unwrap();
    assert!(block.contains("- [x] t"), "{}", block);
    assert!(fold_core::check::check(&v).is_empty());
    // a section-spelled orphan: heading embeds, still a pair (§4.7)
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- keep me\n").unwrap();
    std::fs::write(dir.path().join("racfer~t.md"), format!("---\nid: {}\n---\n\n# T\n\nbody o\n", BID))
        .unwrap();
    std::fs::write(
        dir.path().join("racfer~t.sync-conflict-20260912-100000-phone.md"),
        format!("---\nid: {}\n---\n\n# T\n\nbody t\n", BID),
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.starts_with(&format!("# A\n\n- keep me\n\n# ![[{}]]\n\n# ![[", BID)), "{}", root);
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1, "root.md:\n{}", root);
    assert_eq!(Some(pairs[0].0), v.tree.block_by_id(&bid), "root.md:\n{}", root);
    let diags = fold_core::check::check(&v);
    assert!(
        diags.iter().all(|d| d.message == "unresolved conflict block"),
        "{:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn crlf_root_md_keeps_its_blank_lines_under_fix() {
    // check --fix lays root.md's top-level nodes out itself; a blank line
    // is blank whatever its line ending, so a loose list stays loose
    // (§4.2) and none is added after text that already ends in one (§3.3)
    for (before, after) in [
        ("- a\r\n\r\n- b\r\n", "- a\n\n- b\n"),
        ("intro\r\n\r\n- a\r\n", "intro\r\n\r\n- a\n"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("root.md"), before).unwrap();
        let mut v = Vault::open(dir.path()).unwrap();
        fold_core::check::fix(&mut v).unwrap();
        let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
        assert_eq!(text, after, "{:?}", before);
    }
}

#[test]
fn sync_conflict_on_a_block_embedded_twice_pairs_where_it_is_shown() {
    // t is shown under X, where the walk from the root meets it first; its
    // second embed, in root.md under B, comes first in file order and reads
    // as broken (§6.2). The conflict block goes after t's own embed, so it
    // pairs with t (§12.4), not with the broken line
    let x = "dozzod-binwes-talsun-worbec";
    let dir = tempfile::tempdir().unwrap();
    let root = format!("# A\n\n![[{}]]\n\n# B\n\n![[{}]]\n", x, BID);
    std::fs::write(dir.path().join("root.md"), &root).unwrap();
    std::fs::write(
        dir.path().join("dozzod~x.md"),
        format!("---\nid: {}\n---\n\n- X\n  ![[{}]]\n", x, BID),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("racfer~t.md"),
        format!("---\nid: {}\n---\n\n- [ ] t\n", BID),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("racfer~t.sync-conflict-20260912-100000-phone.md"),
        format!("---\nid: {}\n---\n\n- [x] t\n", BID),
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let pairs = merge::conflict_pairs(&v);
    assert_eq!(pairs.len(), 1);
    let ours = v.tree.node(pairs[0].0).block.as_ref().and_then(|b| b.id.as_ref());
    assert_eq!(ours.map(|i| i.to_string()), Some(BID.to_string()));
    assert_eq!(std::fs::read_to_string(dir.path().join("root.md")).unwrap(), root);
}

/// A vault merged with a phone's copy of its root.md: B's text and the
/// third task differ, so B and three each get a conflict copy (§12.4).
fn merged_with_copies() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("root.md"),
        "# A\n\n## B\n\nours\n\n- [ ] one\n- [x] two\n\n## C\n\n- [ ] three\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n## B\n\ntheirs\n\n- [ ] one\n- [x] two\n\n## C\n\n- [x] three\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    assert_eq!(merge::conflict_pairs(&v).len(), 2);
    (dir, v)
}

#[test]
fn a_conflict_copy_s_tasks_are_left_out_of_the_counts_above_it() {
    // §3.5: a copy repeats the node before it, so its tasks would count
    // twice, the one the phone checked as open and done at once
    let (_dir, v) = merged_with_copies();
    let a = v.find_by_path(&["A".into()]).unwrap();
    assert_eq!(v.tree.task_counts(a), (2, 3));
    let c = v.find_by_path(&["A".into(), "C".into()]).unwrap();
    assert_eq!(v.tree.task_counts(c), (1, 1));
    // a copy counts its own
    let copy = merge::conflict_pairs(&v).into_iter().map(|(_, t)| t).find(|&t| v.tree.node(t).title == "B").unwrap();
    assert_eq!(v.tree.task_counts(copy), (1, 2));
}

#[test]
fn clear_done_keeps_a_done_task_with_an_open_one_in_a_conflict_copy_below() {
    // counts leave copies out, clearing does not: the phone reopened e, and
    // its copy is not trashed with d (§8.5, §12.5)
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- [x] d\n  - [x] e\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n- [x] d\n  - [ ] e\n",
    )
    .unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    merge::merge_sync_conflicts(&mut v, false).unwrap();
    let a = v.find_by_path(&["A".into()]).unwrap();
    fold_core::ops::clear_done(&mut v, a).unwrap();
    let text = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(text.contains("- [x] d\n"), "{}", text);
    assert!(text.contains("![["), "{}", text);
}
