//! Walking the resolved tree: a node's place in the outline runs up through
//! the embeds that stitch blocks in (§4.7).

use fold_core::tree::NRef;
use fold_core::vault::Vault;

const A: &str = "racfer-hattes-mislup-nodrys";
const B: &str = "dozzod-binwes-talsun-worbec";

fn vault(files: &[(&str, String)]) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let v = Vault::open(dir.path()).unwrap();
    (dir, v)
}

/// The node titled `title`, in whichever file it is.
fn node(v: &Vault, title: &str) -> NRef {
    let t = &v.tree;
    (0..t.files.len())
        .flat_map(|f| (0..t.files[f].nodes.len()).map(move |n| (f, n)))
        .find(|&r| t.node(r).title == title && !t.node(r).is_embed())
        .unwrap_or_else(|| panic!("no node {}", title))
}

fn titles(v: &Vault, chain: &[NRef]) -> Vec<String> {
    chain.iter().map(|&r| v.tree.node(r).title.clone()).collect()
}

#[test]
fn a_chain_runs_up_through_the_embeds_and_leaves_them_out() {
    let (_d, v) = vault(&[
        ("root.md", format!("# Home\n\n- top\n  ![[{}]]\n", A)),
        ("racfer~x.md", format!("---\nid: {}\n---\n\n- x\n  - y\n", A)),
    ]);
    let y = node(&v, "y");
    assert_eq!(titles(&v, &v.tree.chain(y)), ["Home", "top", "x", "y"]);
    // its outline parent is x; x's is the embed under top
    let x = v.tree.resolved_parent(y).unwrap();
    assert_eq!(v.tree.node(x).title, "x");
    let embed = v.tree.resolved_parent(x).unwrap();
    assert!(v.tree.node(embed).is_embed());
    assert_eq!(v.tree.node(v.tree.resolved_parent(embed).unwrap()).title, "top");
}

#[test]
fn a_chain_ends_where_an_embed_cycle_comes_round() {
    // two blocks that embed each other (§6.2): neither reaches root.md
    let (_d, v) = vault(&[
        ("root.md", "# Home\n".to_string()),
        ("racfer~x.md", format!("---\nid: {}\n---\n\n- x\n  ![[{}]]\n", A, B)),
        ("dozzod~y.md", format!("---\nid: {}\n---\n\n- y\n  ![[{}]]\n", B, A)),
    ]);
    assert_eq!(titles(&v, &v.tree.chain(node(&v, "y"))), ["x", "y"]);
    assert_eq!(titles(&v, &v.tree.chain(node(&v, "x"))), ["y", "x"]);
}
