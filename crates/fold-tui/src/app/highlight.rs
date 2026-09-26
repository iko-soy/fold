//! Syntax highlighting of fenced code in the reading pane (§10.9), with
//! tree-sitter grammars compiled into the binary: every grammar published on
//! crates.io that builds against tree-sitter 0.27 and has a highlights query
//! (the table is `grammars.rs`). A fence's info string picks the language;
//! anything unknown stays plain.

use super::ui::theme;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use std::collections::HashMap;
use std::sync::OnceLock;
use super::grammars::GRAMMARS;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Capture names to recognise. A query's capture maps to the longest of
/// these that is a dot-separated prefix of it (`keyword.control.return` →
/// `keyword`), so the roots cover every grammar's naming; the longer names
/// are the ones styled differently from their root.
const NAMES: &[&str] = &[
    "attribute", "boolean", "character", "comment", "conditional", "constant", "constant.builtin",
    "constructor", "define", "delimiter", "embedded", "error", "escape", "exception", "field", "float",
    "function", "function.builtin", "function.macro", "include", "keyword", "label", "macro", "markup",
    "markup.heading", "markup.italic", "markup.link", "markup.raw", "markup.strong", "method", "module",
    "namespace", "number", "operator", "parameter", "preproc", "property", "punctuation", "repeat", "special",
    "storageclass", "string", "string.escape", "string.regex", "string.regexp", "string.special", "structure",
    "symbol", "tag", "text", "text.literal", "text.title", "text.uri", "title", "type", "type.builtin", "uri",
    "variable", "variable.builtin", "variable.member", "variable.parameter",
];

fn style_for(name: &str) -> Style {
    let root = name.split('.').next().unwrap_or(name);
    let s = Style::default();
    match (root, name) {
        ("comment", _) => s.fg(theme::DIM).add_modifier(Modifier::ITALIC),
        (_, "string.escape") | ("escape", _) => s.fg(Color::Cyan),
        (_, "string.regex") | (_, "string.regexp") => s.fg(Color::LightGreen),
        ("string", _) | ("character", _) | (_, "text.literal") | (_, "markup.raw") => s.fg(Color::Green),
        ("keyword", _) | ("conditional", _) | ("repeat", _) | ("exception", _) | ("include", _)
        | ("storageclass", _) | ("define", _) | ("preproc", _) => s.fg(Color::Magenta),
        (_, "function.macro") | ("macro", _) => s.fg(Color::LightMagenta),
        ("function", _) | ("method", _) | ("constructor", _) => s.fg(Color::LightBlue),
        ("type", _) | ("module", _) | ("namespace", _) | ("structure", _) => s.fg(Color::Yellow),
        ("number", _) | ("float", _) | ("boolean", _) | ("constant", _) | ("symbol", _) => s.fg(Color::Indexed(209)),
        ("property", _) | ("field", _) | (_, "variable.member") | ("attribute", _) | ("label", _) => s.fg(Color::Cyan),
        ("tag", _) => s.fg(Color::LightRed),
        (_, "variable.builtin") => s.fg(Color::LightRed),
        (_, "markup.heading") | ("title", _) | (_, "text.title") => s.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        (_, "markup.strong") => s.add_modifier(Modifier::BOLD),
        (_, "markup.italic") => s.add_modifier(Modifier::ITALIC),
        (_, "markup.link") | ("uri", _) | (_, "text.uri") => s.fg(Color::LightCyan).add_modifier(Modifier::UNDERLINED),
        ("error", _) => s.fg(Color::LightRed),
        ("operator", _) | ("punctuation", _) | ("delimiter", _) => s.fg(Color::Gray),
        _ => s,
    }
}

/// Index into `GRAMMARS` by every name a fence may use (ids and aliases,
/// lower-case).
fn by_name() -> &'static HashMap<String, usize> {
    static MAP: OnceLock<HashMap<String, usize>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        for (i, g) in GRAMMARS.iter().enumerate() {
            m.insert(g.id.to_string(), i);
            for a in g.aliases {
                m.entry(a.to_string()).or_insert(i);
            }
        }
        m
    })
}

/// A language's configuration, compiled on first use (a query costs a few
/// milliseconds; most vaults use a handful of languages).
fn config(i: usize) -> Option<&'static HighlightConfiguration> {
    static CONFIGS: OnceLock<Vec<OnceLock<Option<HighlightConfiguration>>>> = OnceLock::new();
    let all = CONFIGS.get_or_init(|| GRAMMARS.iter().map(|_| OnceLock::new()).collect());
    all.get(i)?
        .get_or_init(|| {
            let g = &GRAMMARS[i];
            let mut c = HighlightConfiguration::new((g.language)(), g.id, &(g.highlights)(), "", "").ok()?;
            c.configure(NAMES);
            Some(c)
        })
        .as_ref()
}

/// The ids of every language that can be highlighted.
#[cfg(test)]
fn languages() -> impl Iterator<Item = &'static str> {
    GRAMMARS.iter().map(|g| g.id)
}

/// Whether a fence's info string names a language we can highlight.
pub fn supported(info: &str) -> bool {
    info_language(info).is_some()
}

fn info_language(info: &str) -> Option<usize> {
    // `rust`, `rust,ignore`, `{.python}`, `py title="x"`: the first word
    let word = info
        .trim()
        .trim_start_matches(['{', '.'])
        .split(|c: char| c.is_whitespace() || c == ',' || c == '}')
        .next()?;
    by_name().get(&word.to_ascii_lowercase()).copied()
}

/// Highlight a code block: one list of spans per line of `code`, or `None`
/// if the language is unknown or its grammar could not parse the text.
pub fn highlight(info: &str, code: &str) -> Option<Vec<Vec<Span<'static>>>> {
    let cfg = config(info_language(info)?)?;
    let mut hl = Highlighter::new();
    let events = hl.highlight(cfg, code.as_bytes(), None, None, |_| None).ok()?;
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut stack: Vec<Style> = Vec::new();
    for ev in events {
        match ev.ok()? {
            HighlightEvent::HighlightStart(h) => stack.push(style_for(NAMES[h.0])),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let style = stack.last().copied().unwrap_or_default();
                let text = &code[start..end];
                for (i, part) in text.split('\n').enumerate() {
                    if i > 0 {
                        lines.push(Vec::new());
                    }
                    if !part.is_empty() {
                        lines.last_mut().unwrap().push(Span::styled(part.to_string(), style));
                    }
                }
            }
        }
    }
    // `code` ends with a newline: the empty line after it is not a line
    if code.ends_with('\n') {
        lines.pop();
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_grammar_compiles_its_query() {
        let failed: Vec<&str> = (0..GRAMMARS.len()).filter(|&i| config(i).is_none()).map(|i| GRAMMARS[i].id).collect();
        assert!(failed.is_empty(), "queries that do not compile: {:?}", failed);
        assert!(GRAMMARS.len() >= 140);
    }

    #[test]
    fn names_are_unique_and_common_fences_resolve() {
        let mut ids: Vec<&str> = languages().collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), GRAMMARS.len());
        for fence in ["rust", "py", "js", "ts", "tsx", "sh", "zsh", "json", "toml", "yaml", "go", "c", "c++", "c#",
            "java", "kotlin", "swift", "ruby", "php", "lua", "haskell", "ocaml", "elixir", "sql", "html", "css",
            "xml", "nix", "dockerfile", "makefile", "diff", "zig", "scala", "dart", "r", "julia"] {
            assert!(supported(fence), "{} should be highlighted", fence);
        }
    }

    #[test]
    fn highlights_by_line_and_keeps_the_text() {
        let code = "fn main() {\n    // hi\n    let s = \"x\";\n}\n";
        let lines = highlight("rust", code).unwrap();
        assert_eq!(lines.len(), 4);
        let text: Vec<String> = lines.iter().map(|l| l.iter().map(|s| s.content.as_ref()).collect()).collect();
        assert_eq!(text, ["fn main() {", "    // hi", "    let s = \"x\";", "}"]);
        // `fn` is a keyword, the comment is dimmed, the string green
        assert_eq!(lines[0][0].content, "fn");
        assert_eq!(lines[0][0].style.fg, Some(Color::Magenta));
        assert!(lines[1].iter().any(|s| s.content.contains("// hi") && s.style.fg == Some(theme::DIM)));
        assert!(lines[2].iter().any(|s| s.content.contains("\"x\"") && s.style.fg == Some(Color::Green)));
    }

    #[test]
    fn info_strings_and_unknown_languages() {
        assert!(supported("rust,ignore") && supported("{.python}") && supported("sh title=x"));
        assert!(!supported("brainfuck") && !supported(""));
        assert!(highlight("brainfuck", "+++").is_none());
    }
}
