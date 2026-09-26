//! Syntax highlighting of fenced code in the reading pane (§10.9), with
//! tree-sitter grammars compiled into the binary. A fence's info string picks
//! the language; anything unknown stays plain.

use super::ui::theme;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use std::collections::HashMap;
use std::sync::OnceLock;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Capture names the grammars' queries use, in the order styles are looked up.
const NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "escape",
    "function",
    "function.builtin",
    "function.macro",
    "function.method",
    "keyword",
    "label",
    "module",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.escape",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

fn style_for(name: &str) -> Style {
    let root = name.split('.').next().unwrap_or(name);
    let s = Style::default();
    match (root, name) {
        ("comment", _) => s.fg(theme::DIM).add_modifier(Modifier::ITALIC),
        ("keyword", _) => s.fg(Color::Magenta),
        ("function", _) | ("constructor", _) => s.fg(Color::LightBlue),
        ("type", _) | ("module", _) => s.fg(Color::Yellow),
        (_, "string.escape") | ("escape", _) => s.fg(Color::Cyan),
        ("string", _) => s.fg(Color::Green),
        ("number", _) | ("constant", _) => s.fg(Color::Indexed(209)),
        ("property", _) | ("attribute", _) | ("label", _) => s.fg(Color::Cyan),
        ("tag", _) => s.fg(Color::LightRed),
        (_, "variable.builtin") => s.fg(Color::LightRed),
        ("operator", _) | ("punctuation", _) => s.fg(Color::Gray),
        _ => s,
    }
}

/// The languages, by the names a fence's info string may use.
fn language(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "rust" | "rs" => "rust",
        "python" | "py" | "python3" => "python",
        "javascript" | "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "typescript" | "ts" => "typescript",
        "tsx" => "tsx",
        "bash" | "sh" | "shell" | "zsh" | "console" => "bash",
        "json" | "jsonc" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "go" | "golang" => "go",
        "c" | "h" => "c",
        "nix" => "nix",
        "html" | "htm" => "html",
        "css" => "css",
        _ => return None,
    })
}

fn build(lang: &'static str) -> Option<HighlightConfiguration> {
    use tree_sitter::Language;
    let (language, highlights): (Language, String) = match lang {
        "rust" => (tree_sitter_rust::LANGUAGE.into(), tree_sitter_rust::HIGHLIGHTS_QUERY.into()),
        "python" => (tree_sitter_python::LANGUAGE.into(), tree_sitter_python::HIGHLIGHTS_QUERY.into()),
        "javascript" => (
            tree_sitter_javascript::LANGUAGE.into(),
            format!("{}\n{}", tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY),
        ),
        // TypeScript's query extends JavaScript's
        "typescript" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            format!("{}\n{}", tree_sitter_typescript::HIGHLIGHTS_QUERY, tree_sitter_javascript::HIGHLIGHT_QUERY),
        ),
        "tsx" => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            format!(
                "{}\n{}\n{}",
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            ),
        ),
        "bash" => (tree_sitter_bash::LANGUAGE.into(), tree_sitter_bash::HIGHLIGHT_QUERY.into()),
        "json" => (tree_sitter_json::LANGUAGE.into(), tree_sitter_json::HIGHLIGHTS_QUERY.into()),
        "toml" => (tree_sitter_toml_ng::LANGUAGE.into(), tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into()),
        "yaml" => (tree_sitter_yaml::LANGUAGE.into(), tree_sitter_yaml::HIGHLIGHTS_QUERY.into()),
        "go" => (tree_sitter_go::LANGUAGE.into(), tree_sitter_go::HIGHLIGHTS_QUERY.into()),
        "c" => (tree_sitter_c::LANGUAGE.into(), tree_sitter_c::HIGHLIGHT_QUERY.into()),
        "nix" => (tree_sitter_nix::LANGUAGE.into(), tree_sitter_nix::HIGHLIGHTS_QUERY.into()),
        "html" => (tree_sitter_html::LANGUAGE.into(), tree_sitter_html::HIGHLIGHTS_QUERY.into()),
        "css" => (tree_sitter_css::LANGUAGE.into(), tree_sitter_css::HIGHLIGHTS_QUERY.into()),
        _ => return None,
    };
    let mut config = HighlightConfiguration::new(language, lang, &highlights, "", "").ok()?;
    config.configure(NAMES);
    Some(config)
}

/// Each language's configuration, compiled on first use (a query costs a few
/// milliseconds; most vaults use two or three languages).
fn config(lang: &'static str) -> Option<&'static HighlightConfiguration> {
    static CONFIGS: OnceLock<HashMap<&'static str, OnceLock<Option<HighlightConfiguration>>>> = OnceLock::new();
    let all = CONFIGS.get_or_init(|| {
        [
            "rust", "python", "javascript", "typescript", "tsx", "bash", "json", "toml", "yaml", "go", "c", "nix",
            "html", "css",
        ]
        .into_iter()
        .map(|l| (l, OnceLock::new()))
        .collect()
    });
    all.get(lang)?.get_or_init(|| build(lang)).as_ref()
}

/// Whether a fence's info string names a language we can highlight.
pub fn supported(info: &str) -> bool {
    info_language(info).is_some()
}

fn info_language(info: &str) -> Option<&'static str> {
    // `rust`, `rust,ignore`, `{.python}`, `py title="x"`: the first word
    let word = info
        .trim()
        .trim_start_matches(['{', '.'])
        .split(|c: char| c.is_whitespace() || c == ',' || c == '}')
        .next()?;
    language(word)
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
        for lang in [
            "rust", "python", "javascript", "typescript", "tsx", "bash", "json", "toml", "yaml", "go", "c", "nix",
            "html", "css",
        ] {
            assert!(config(lang).is_some(), "{} does not compile", lang);
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
