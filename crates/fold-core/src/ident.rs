//! Ids, names (slugs) and filenames (§3.4, §6.4).

use crate::syllables::{PREFIXES, SUFFIXES};
use unicode_normalization::UnicodeNormalization;

/// A block id: 64 random bits rendered as four `@p` words joined by `-`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(String);

impl Id {
    /// Generate a fresh random id from OS randomness.
    pub fn generate() -> Id {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("OS randomness available");
        Id::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: [u8; 8]) -> Id {
        let mut words = Vec::with_capacity(4);
        for chunk in bytes.chunks(2) {
            let hi = chunk[0] as usize;
            let lo = chunk[1] as usize;
            words.push(format!("{}{}", PREFIXES[hi], SUFFIXES[lo]));
        }
        Id(words.join("-"))
    }

    /// Parse and validate a four-word id against the syllable tables.
    pub fn parse(s: &str) -> Option<Id> {
        let words: Vec<&str> = s.split('-').collect();
        if words.len() != 4 {
            return None;
        }
        for w in &words {
            if !is_word(w) {
                return None;
            }
        }
        Some(Id(s.to_string()))
    }

    /// True if `s` is a valid leading run of id words (1..=4 words), with or
    /// without `![[ ]]` wrapping. Used by target resolution (§3.4).
    pub fn words(&self) -> Vec<&str> {
        self.0.split('-').collect()
    }

    /// The shortest leading run of words not used by any prefix in `taken`
    /// (§6.4). The full id is always a valid prefix: if even it is taken
    /// (an id collision, reported elsewhere), it is returned anyway.
    pub fn shortest_prefix(&self, taken: &dyn Fn(&str) -> bool) -> String {
        let words = self.words();
        for n in 1..4 {
            let cand = words[..n].join("-");
            if !taken(&cand) {
                return cand;
            }
        }
        self.0.clone()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_word(w: &str) -> bool {
    // ASCII first: slicing at byte 3 of a non-ASCII word would panic
    w.len() == 6
        && w.is_ascii()
        && PREFIXES.contains(&&w[..3])
        && SUFFIXES.contains(&&w[3..])
}

/// True if `s` (already stripped of any `![[ ]]`) is a valid leading run of
/// id words — 1 to 4 valid words joined by `-`.
pub fn looks_like_id_prefix(s: &str) -> bool {
    let words: Vec<&str> = s.split('-').collect();
    !words.is_empty() && words.len() <= 4 && words.iter().all(|w| is_word(w))
}

/// Strip `![[...]]` wrapping if present.
pub fn unwrap_embed_text(s: &str) -> &str {
    s.strip_prefix("![[")
        .and_then(|r| r.strip_suffix("]]"))
        .map(str::trim)
        .unwrap_or(s)
}

/// `slug(title)` per §6.4: NFC, lowercase, whitespace runs to `-`, drop
/// anything that is not a letter, number or `-`, collapse `-`, trim, 80 bytes.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in title.nfc().flat_map(char::to_lowercase) {
        if ch.is_whitespace() {
            if !last_dash && !out.is_empty() {
                out.push('-');
                last_dash = true;
            }
        } else if ch.is_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if ch == '-' {
            if !last_dash && !out.is_empty() {
                out.push('-');
                last_dash = true;
            }
        }
        // everything else is removed
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() > 80 {
        let mut end = 80;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

/// `<prefix>~<name>.md` (§6.4).
pub fn filename(prefix: &str, name: &str) -> String {
    format!("{}~{}.md", prefix, name)
}

/// Split a block filename back into (prefix, name) if it has the
/// `<prefix>~<name>.md` shape.
pub fn split_filename(file: &str) -> Option<(&str, &str)> {
    let stem = file.strip_suffix(".md")?;
    let (prefix, name) = stem.split_once('~')?;
    if prefix.is_empty() || name.is_empty() {
        return None;
    }
    Some((prefix, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrip() {
        let id = Id::from_bytes([0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe]);
        let parsed = Id::parse(id.as_str()).unwrap();
        assert_eq!(id, parsed);
        assert_eq!(id.words().len(), 4);
    }

    #[test]
    fn spec_example_words_validate() {
        assert!(Id::parse("racfer-hattes-dozzod-binwes").is_some());
        assert!(Id::parse("dozzod-binwes-talsun-worbec").is_some());
        assert!(Id::parse("racfer-hattes-mislup-nodrys").is_some());
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(Id::parse("hello-world-this-nope").is_none());
        assert!(Id::parse("racfer-hattes-dozzod").is_none());
        assert!(Id::parse("racfer-hattes-dozzod-binwes-extra").is_none());
        assert!(looks_like_id_prefix("racfer"));
        assert!(looks_like_id_prefix("racfer-hattes-dozzod-binwes"));
        assert!(!looks_like_id_prefix("racfer-hattes-dozzod-binwes-wat"));
        assert!(!looks_like_id_prefix("nope"));
        // non-ASCII words must be rejected, not panic
        assert!(Id::parse("ééaa-racfer-hattes-dozzod").is_none());
        assert!(!looks_like_id_prefix("ééé"));
    }

    #[test]
    fn slug_rules() {
        assert_eq!(slug("Order new switch"), "order-new-switch");
        assert_eq!(slug("ZFS layout"), "zfs-layout");
        assert_eq!(slug("  ---  "), "untitled");
        assert_eq!(slug("Café notes!"), "café-notes");
        assert_eq!(slug("a/b"), "ab");
    }

    #[test]
    fn shortest_prefix_grows() {
        let id = Id::parse("racfer-hattes-mislup-nodrys").unwrap();
        assert_eq!(id.shortest_prefix(&|_| false), "racfer");
        assert_eq!(
            id.shortest_prefix(&|p| p == "racfer"),
            "racfer-hattes"
        );
        // every shorter run taken: the full id, never a panic
        assert_eq!(id.shortest_prefix(&|_| true), "racfer-hattes-mislup-nodrys");
    }

    #[test]
    fn filename_split() {
        assert_eq!(
            split_filename("racfer~order-new-switch.md"),
            Some(("racfer", "order-new-switch"))
        );
        assert_eq!(split_filename("racfer.md"), None);
    }
}
