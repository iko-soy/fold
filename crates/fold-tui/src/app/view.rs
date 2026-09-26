//! Remembered view settings (§10.1): the reading pane, wrapping, hidden
//! done tasks, the editor's keymap, the divider, folds and the zoom, one
//! small line-based file per vault beside the trash.

use super::editor::Keys;
use fold_core::ident::Id;
use fold_core::vault::NodeKey;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct View {
    pub show_reading: bool,
    pub wrap: bool,
    pub hide_done: bool,
    pub keys: Option<Keys>,
    pub outline_width: Option<u16>,
    pub zoom: Option<NodeKey>,
    pub folded: Vec<NodeKey>,
}

/// Where a vault's view lives: `$XDG_STATE_HOME/fold/views/<vault path>`,
/// the path's separators escaped.
pub fn path_for(vault: &Path) -> Option<PathBuf> {
    let vault = vault.canonicalize().ok()?;
    let name: String = vault
        .to_string_lossy()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '%',
            c => c,
        })
        .collect();
    let state = fold_core::vault::trash_dir().parent()?.to_path_buf();
    Some(state.join("views").join(name))
}

pub fn load(vault: &Path) -> Option<View> {
    View::parse(&std::fs::read_to_string(path_for(vault)?).ok()?)
}

pub fn save(vault: &Path, v: &View) -> std::io::Result<()> {
    let Some(p) = path_for(vault) else { return Ok(()) };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    fold_core::vault::atomic_write(&p, &v.to_text())
}

impl View {
    pub fn to_text(&self) -> String {
        let flag = |b: bool| if b { "1" } else { "0" };
        let mut out = format!(
            "reading {}\nwrap {}\nhide-done {}\n",
            flag(self.show_reading),
            flag(self.wrap),
            flag(self.hide_done)
        );
        if let Some(k) = self.keys {
            out.push_str(&format!("keys {}\n", k.name()));
        }
        if let Some(w) = self.outline_width {
            out.push_str(&format!("outline-width {}\n", w));
        }
        if let Some(k) = self.zoom.as_ref().and_then(encode_key) {
            out.push_str(&format!("zoom {}\n", k));
        }
        for k in self.folded.iter().filter_map(encode_key) {
            out.push_str(&format!("fold {}\n", k));
        }
        out
    }

    /// Unknown or malformed lines are skipped: an old or damaged file costs
    /// a setting, never the start.
    pub fn parse(text: &str) -> Option<View> {
        let mut v = View { wrap: true, ..View::default() };
        for line in text.lines() {
            let (name, value) = line.split_once(' ').unwrap_or((line, ""));
            match name {
                "reading" => v.show_reading = value == "1",
                "wrap" => v.wrap = value == "1",
                "hide-done" => v.hide_done = value == "1",
                "keys" => v.keys = Keys::parse(value),
                "outline-width" => v.outline_width = value.parse().ok(),
                "zoom" => v.zoom = decode_key(value),
                "fold" => v.folded.extend(decode_key(value)),
                _ => {}
            }
        }
        Some(v)
    }
}

/// `root`, `id <id>`, or `path <block id or ->` then tab-separated ordinal
/// and title pairs. Titles holding a tab or newline aren't saved.
fn encode_key(k: &NodeKey) -> Option<String> {
    match k {
        NodeKey::Root => Some("root".into()),
        NodeKey::Id(id) => Some(format!("id {}", id.as_str())),
        NodeKey::Path { block, steps } => {
            let mut s = format!("path {}", block.as_ref().map(|b| b.as_str()).unwrap_or("-"));
            for (title, ordinal) in steps {
                if title.contains(['\t', '\n', '\r']) {
                    return None;
                }
                s.push_str(&format!("\t{}\t{}", ordinal, title));
            }
            Some(s)
        }
    }
}

fn decode_key(s: &str) -> Option<NodeKey> {
    let (kind, rest) = s.split_once(' ').unwrap_or((s, ""));
    match kind {
        "root" => Some(NodeKey::Root),
        "id" => Id::parse(rest).map(NodeKey::Id),
        "path" => {
            let mut parts = rest.split('\t');
            let block = match parts.next()? {
                "-" => None,
                b => Some(Id::parse(b)?),
            };
            let mut steps = Vec::new();
            while let Some(n) = parts.next() {
                steps.push((parts.next()?.to_string(), n.parse().ok()?));
            }
            Some(NodeKey::Path { block, steps })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let v = View {
            show_reading: true,
            wrap: false,
            hide_done: true,
            keys: Some(Keys::Helix),
            outline_width: Some(42),
            zoom: Some(NodeKey::Path { block: None, steps: vec![("Homelab".into(), 0), ("NAS · 2".into(), 1)] }),
            folded: vec![NodeKey::Root, NodeKey::Path { block: None, steps: vec![("A b".into(), 0)] }],
        };
        assert_eq!(View::parse(&v.to_text()), Some(v));
    }

    #[test]
    fn junk_is_skipped_and_wrap_defaults_on() {
        let v = View::parse("reading 1\nwhat\nfold path -\t0\nkeys nope\n").unwrap();
        assert!(v.show_reading && v.wrap && v.keys.is_none() && v.folded.is_empty());
    }
}
