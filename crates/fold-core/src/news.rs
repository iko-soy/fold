//! What a reload took in, in outline terms (§11.2), as the TUI's status bar
//! and the app's message say it: *↻ changed outside fold: Inbox (+1 item)*.

use crate::parse::Kind;
use crate::vault::{NodeKey, Vault};

/// A top-level node before or after a reload: what it holds, and a hash of
/// its text, to say what changed.
#[derive(Clone)]
pub struct Top {
    key: NodeKey,
    title: String,
    items: usize,
    sections: usize,
    text: u64,
}

/// A vault's top-level nodes, as `Top`s.
pub fn tops(vault: &Vault) -> Vec<Top> {
    use std::hash::{Hash, Hasher};
    let tree = &vault.tree;
    tree.resolved_children(tree.root)
        .into_iter()
        .map(|top| {
            let (mut items, mut sections) = (0, 0);
            let mut text = std::collections::hash_map::DefaultHasher::new();
            tree.walk(top, &mut |t, r| {
                let n = t.node(r);
                let file = t.text_of(r);
                n.title_span.text(file).hash(&mut text);
                n.text_lines(file).hash(&mut text);
                n.block.as_ref().map(|b| &b.frontmatter_raw).hash(&mut text);
                match n.kind {
                    _ if r == top => {}
                    Kind::Item => items += 1,
                    Kind::Section => sections += 1,
                    Kind::Root => {}
                }
            });
            Top { key: vault.key_of(top), title: tree.node(top).title.clone(), items, sections, text: text.finish() }
        })
        .collect()
}

/// The files as read before the editor's save, `after`, with what a merge
/// of sync-conflict copies then took in over them (§11.2): each top-level
/// node it changed from `was` to `now` moves by as much, one it brought
/// comes in and one gone by then goes. What the save typed, in `was` and
/// `now` both, is not laid over.
pub fn merged_over(mut after: Vec<Top>, was: &[Top], now: Vec<Top>) -> Vec<Top> {
    after.retain(|a| !was.iter().any(|w| w.key == a.key) || now.iter().any(|n| n.key == a.key));
    for n in now {
        match (was.iter().find(|w| w.key == n.key), after.iter_mut().find(|a| a.key == n.key)) {
            (Some(w), Some(a)) if w.text != n.text => {
                a.items = (a.items + n.items).saturating_sub(w.items);
                a.sections = (a.sections + n.sections).saturating_sub(w.sections);
                a.title = n.title;
                a.text = n.text;
            }
            (None, None) => after.push(n),
            _ => {}
        }
    }
    after
}

/// Whether two outlines' top-level nodes are the same, text and all:
/// nothing came in (§11.2).
pub fn same_tops(before: &[Top], after: &[Top]) -> bool {
    before.iter().map(|t| (&t.key, t.text)).eq(after.iter().map(|t| (&t.key, t.text)))
}

/// What a reload took in, in outline terms (§11.2): each top-level node
/// that changed, came or went, and the items or sections it gained or lost.
/// With `typed`, that the editor's typing was saved first comes ahead of
/// them, which give way where the status bar is short (§10.6).
pub fn changed_outside(before: &[Top], after: &[Top], typed: bool) -> String {
    let mut parts = Vec::new();
    for a in after {
        match before.iter().find(|b| b.key == a.key) {
            None => parts.push(format!("{} (new)", a.title)),
            Some(b) if b.text != a.text => {
                let counts: Vec<String> = [(b.items, a.items, "item"), (b.sections, a.sections, "section")]
                    .into_iter()
                    .filter(|(was, now, _)| was != now)
                    .map(|(was, now, what)| {
                        let n = was.abs_diff(now);
                        format!("{}{} {}{}", if now > was { "+" } else { "−" }, n, what, if n == 1 { "" } else { "s" })
                    })
                    .collect();
                let what = if counts.is_empty() { "edited".to_string() } else { counts.join(", ") };
                parts.push(format!("{} ({})", a.title, what));
            }
            Some(_) => {}
        }
    }
    for b in before.iter().filter(|b| !after.iter().any(|a| a.key == b.key)) {
        parts.push(format!("{} (removed)", b.title));
    }
    let lead = if typed { "↻ typing saved; changed outside fold" } else { "↻ changed outside fold" };
    match parts.len() {
        0 => lead.into(),
        n if n > 3 => format!("{}: {} and {} more", lead, parts[..2].join(", "), n - 2),
        _ => format!("{}: {}", lead, parts.join(", ")),
    }
}
