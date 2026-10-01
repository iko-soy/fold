//! Node keys as strings (§3.4): what the app holds to name a node across
//! re-parses — the selection, the zoom, the folds — opaque to it.

use fold_core::vault::NodeKey;
use fold_core::Id;

/// Between the steps of a path key, and between a step's ordinal and its
/// title: control characters no title line holds.
const STEP: char = '\u{1f}';
const ORDINAL: char = '\u{1e}';

pub fn encode(k: &NodeKey) -> String {
    match k {
        NodeKey::Root => "r".into(),
        NodeKey::Id(id) => format!("i{}", id),
        NodeKey::Path { block, steps } => {
            let mut s = String::from("p");
            if let Some(b) = block {
                s.push_str(b.as_str());
            }
            for (title, ordinal) in steps {
                s.push(STEP);
                s.push_str(&ordinal.to_string());
                s.push(ORDINAL);
                s.push_str(title);
            }
            s
        }
    }
}

pub fn decode(s: &str) -> Option<NodeKey> {
    match s.chars().next()? {
        'r' if s.len() == 1 => Some(NodeKey::Root),
        'i' => Id::parse(&s[1..]).map(NodeKey::Id),
        'p' => {
            let mut parts = s[1..].split(STEP);
            let block = match parts.next()? {
                "" => None,
                b => Some(Id::parse(b)?),
            };
            let steps = parts
                .map(|step| {
                    let (ordinal, title) = step.split_once(ORDINAL)?;
                    Some((title.to_string(), ordinal.parse().ok()?))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(NodeKey::Path { block, steps })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let id = Id::parse("racfer-hattes-mislup-nodrys").unwrap();
        for k in [
            NodeKey::Root,
            NodeKey::Id(id.clone()),
            NodeKey::Path { block: None, steps: vec![("Homelab".into(), 0), ("".into(), 2)] },
            NodeKey::Path { block: Some(id), steps: vec![("a/b · “c”".into(), 1)] },
        ] {
            assert_eq!(decode(&encode(&k)), Some(k));
        }
        assert_eq!(decode("x"), None);
        assert_eq!(decode("inot-an-id"), None);
    }
}
