//! Line-based Markdown styling for the reading pane (§10.9). Nothing is
//! hidden: markers are dimmed, never removed; a checkbox is shown as a glyph.
//! The styler also reports where the clickable parts of the line are.

use super::ui::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// A styled line and where its clickable parts are, as character columns
/// of the styled text: the reading pane knows where each character is drawn
/// (wide characters, tabs, wrapped rows), so it maps them to the screen.
pub struct Styled {
    pub line: Line<'static>,
    /// Column of the checkbox glyph, if the line is a task.
    pub check: Option<usize>,
    /// Columns `[start, end)` of the first link, if any.
    pub link: Option<(usize, usize)>,
}

/// Where a line stands to fenced code (§10.9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    /// Markdown.
    Text,
    /// A fence that opens or closes a block.
    Fence,
    /// A line inside a block.
    Inside,
}

/// Where each of a run of lines stands to fenced code, reading fences as
/// the parser does (§3.3), so what is drawn as code is what is parsed as
/// code: `` ```a`b `` is inline code, and ```` ``` x ```` closes nothing.
pub fn fences<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<Code> {
    let mut open = None;
    lines
        .into_iter()
        .map(|l| match fold_core::parse::fence_transition(l, &mut open) {
            true => Code::Fence,
            false if open.is_some() => Code::Inside,
            false => Code::Text,
        })
        .collect()
}

/// Style one line of the reading pane; `code` is where it stands to fenced
/// code (the caller tracks fences with `fences`).
pub fn style_line(l: &str, code: Code) -> Styled {
    let mut out = Out::default();
    let indent_len = l.len() - l.trim_start().len();
    let (indent, rest) = l.split_at(indent_len);
    out.push(indent, Style::default());

    if code == Code::Inside {
        out.push(rest, Style::default().fg(theme::CODE));
        return out.finish();
    }
    if code == Code::Fence {
        let fc = rest.chars().next();
        let n = rest.chars().take_while(|&c| Some(c) == fc).count();
        out.push(&rest[..n], Style::default().fg(theme::DIM));
        out.push(&rest[n..], Style::default().fg(theme::ACCENT).add_modifier(Modifier::ITALIC));
        return out.finish();
    }
    if rest.starts_with("![[") {
        out.push(rest, Style::default().fg(theme::WARN));
        return out.finish();
    }
    // headings: `#`s dimmed, an optional checkbox, the title bold
    let hashes = rest.chars().take_while(|&c| c == '#').count();
    if hashes > 0 && (rest.len() == hashes || rest[hashes..].starts_with(' ')) {
        out.push(&rest[..hashes], Style::default().fg(theme::DIM));
        let after = &rest[hashes..];
        let (after, done) = out.checkbox(after);
        let color = match hashes {
            1 => theme::H1,
            2 => theme::H2,
            _ => theme::H3,
        };
        let mut style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        if done {
            style = style.fg(theme::DIM).add_modifier(Modifier::CROSSED_OUT);
        }
        out.inline(after, style);
        return out.finish();
    }
    // bullets: marker dimmed, an optional checkbox
    if rest.starts_with("- ") || rest.starts_with("* ") {
        out.push(&rest[..1], Style::default().fg(theme::DIM));
        let (after, done) = out.checkbox(&rest[1..]);
        let style = if done {
            Style::default().fg(theme::DIM).add_modifier(Modifier::CROSSED_OUT)
        } else {
            Style::default()
        };
        out.inline(after, style);
        return out.finish();
    }
    if let Some(q) = rest.strip_prefix('>') {
        out.push(">", Style::default().fg(theme::QUOTE));
        out.inline(q, Style::default().fg(theme::QUOTE).add_modifier(Modifier::ITALIC));
        return out.finish();
    }
    out.inline(rest, Style::default());
    out.finish()
}

#[derive(Default)]
struct Out {
    spans: Vec<Span<'static>>,
    col: usize,
    check: Option<usize>,
    link: Option<(usize, usize)>,
}

impl Out {
    fn push(&mut self, s: &str, style: Style) {
        if s.is_empty() {
            return;
        }
        self.col += s.chars().count();
        self.spans.push(Span::styled(s.to_string(), style));
    }

    /// A leading ` [ ]` / ` [x]` becomes ` ☐` / ` ☑`; returns the rest and
    /// whether the box is checked.
    fn checkbox<'a>(&mut self, s: &'a str) -> (&'a str, bool) {
        let t = s.strip_prefix(' ').unwrap_or(s);
        let lead = s.len() - t.len();
        let b = t.as_bytes();
        if b.len() >= 3 && b[0] == b'[' && b[2] == b']' && matches!(b[1], b' ' | b'x' | b'X' | b'-') {
            let done = b[1] != b' ';
            self.push(&s[..lead], Style::default());
            self.check = Some(self.col);
            if done {
                self.push("☑", Style::default().fg(theme::DONE));
            } else {
                self.push("☐", Style::default().fg(theme::ACCENT));
            }
            return (&t[3..], done);
        }
        (s, false)
    }

    /// Inline styling: `**bold**`, `*em*`, `` `code` ``, `[text](url)` and
    /// bare URLs. Markers stay visible, dimmed.
    fn inline(&mut self, s: &str, base: Style) {
        let dim = base.fg(theme::DIM).remove_modifier(Modifier::BOLD);
        let mut rest = s;
        while !rest.is_empty() {
            // the nearest special token
            let next = [
                rest.find("**"),
                rest.find('`'),
                rest.find('['),
                rest.find("http://"),
                rest.find("https://"),
                rest.find('*'),
            ]
            .into_iter()
            .flatten()
            .min();
            let Some(i) = next else {
                self.push(rest, base);
                return;
            };
            self.push(&rest[..i], base);
            let t = &rest[i..];
            if let Some(inner) = t.strip_prefix("**") {
                if let Some(end) = inner.find("**") {
                    self.push("**", dim);
                    self.push(&inner[..end], base.add_modifier(Modifier::BOLD));
                    self.push("**", dim);
                    rest = &inner[end + 2..];
                    continue;
                }
            } else if let Some(inner) = t.strip_prefix('`') {
                if let Some(end) = inner.find('`') {
                    self.push("`", dim);
                    self.push(&inner[..end], Style::default().fg(theme::CODE).bg(theme::CODE_BG));
                    self.push("`", dim);
                    rest = &inner[end + 1..];
                    continue;
                }
            } else if let Some(inner) = t.strip_prefix('[') {
                if let Some(close) = inner.find("](") {
                    if let Some(paren) = inner[close + 2..].find(')') {
                        let text = &inner[..close];
                        let url = &inner[close + 2..close + 2 + paren];
                        self.push("[", dim);
                        let start = self.col;
                        self.push(text, base.fg(theme::LINK).add_modifier(Modifier::UNDERLINED));
                        self.link.get_or_insert((start, self.col));
                        self.push("](", dim);
                        self.push(url, dim);
                        self.push(")", dim);
                        rest = &inner[close + 2 + paren + 1..];
                        continue;
                    }
                }
            } else if t.starts_with("http://") || t.starts_with("https://") {
                let end = t
                    .find(|c: char| c.is_whitespace() || c == ')' || c == '"')
                    .unwrap_or(t.len());
                let start = self.col;
                self.push(&t[..end], base.fg(theme::LINK).add_modifier(Modifier::UNDERLINED));
                self.link.get_or_insert((start, self.col));
                rest = &t[end..];
                continue;
            } else if let Some(inner) = t.strip_prefix('*') {
                if let Some(end) = inner.find('*') {
                    if end > 0 && !inner.starts_with(' ') {
                        self.push("*", dim);
                        self.push(&inner[..end], base.add_modifier(Modifier::ITALIC));
                        self.push("*", dim);
                        rest = &inner[end + 1..];
                        continue;
                    }
                }
            }
            // not a token after all: emit one character and go on
            let ch = t.chars().next().unwrap();
            self.push(&t[..ch.len_utf8()], base);
            rest = &t[ch.len_utf8()..];
        }
    }

    fn finish(self) -> Styled {
        Styled {
            line: Line::from(self.spans),
            check: self.check,
            link: self.link,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &Styled) -> String {
        s.line.spans.iter().map(|sp| sp.content.as_ref()).collect()
    }

    #[test]
    fn nothing_is_hidden_but_checkboxes_become_glyphs() {
        let s = style_line("- [ ] Buy **milk** and `oat`", Code::Text);
        assert_eq!(text(&s), "- ☐ Buy **milk** and `oat`");
        assert_eq!(s.check, Some(2));
        let s = style_line("#### [x] Snapshot policy", Code::Text);
        assert_eq!(text(&s), "#### ☑ Snapshot policy");
        assert_eq!(s.check, Some(5));
    }

    #[test]
    fn links_report_their_columns() {
        let s = style_line("see [docs](https://x.org) now", Code::Text);
        assert_eq!(s.link, Some((5, 9)));
        let s = style_line("  https://a.b/c tail", Code::Text);
        assert_eq!(s.link, Some((2, 15)));
        // character columns, however wide the characters are drawn
        let s = style_line("漢字 [docs](x)", Code::Text);
        assert_eq!(s.link, Some((4, 8)));
    }
}
