//! Line-based Markdown styling for the reading pane (§10.9). Nothing is
//! hidden: markers are dimmed, never removed; a checkbox is shown as a glyph.
//! The styler also reports where the clickable parts of the line are.

use super::ui::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// A styled line and the display columns of its clickable parts.
pub struct Styled {
    pub line: Line<'static>,
    /// Column of the checkbox glyph, if the line is a task.
    pub check: Option<u16>,
    /// Columns `[start, end)` of the first link, if any.
    pub link: Option<(u16, u16)>,
}

/// Style one line of the reading pane. `in_fence` is whether the line sits
/// inside a fenced code block (the caller tracks fences).
pub fn style_line(l: &str, in_fence: bool) -> Styled {
    let mut out = Out::default();
    let indent_len = l.len() - l.trim_start().len();
    let (indent, rest) = l.split_at(indent_len);
    out.push(indent, Style::default());

    if in_fence {
        out.push(rest, Style::default().fg(theme::CODE));
        return out.finish();
    }
    if rest.starts_with("```") || rest.starts_with("~~~") {
        let n = rest.chars().take_while(|&c| c == '`' || c == '~').count();
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
    col: u16,
    check: Option<u16>,
    link: Option<(u16, u16)>,
}

impl Out {
    fn push(&mut self, s: &str, style: Style) {
        if s.is_empty() {
            return;
        }
        self.col += s.width() as u16;
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
        let s = style_line("- [ ] Buy **milk** and `oat`", false);
        assert_eq!(text(&s), "- ☐ Buy **milk** and `oat`");
        assert_eq!(s.check, Some(2));
        let s = style_line("#### [x] Snapshot policy", false);
        assert_eq!(text(&s), "#### ☑ Snapshot policy");
        assert_eq!(s.check, Some(5));
    }

    #[test]
    fn links_report_their_columns() {
        let s = style_line("see [docs](https://x.org) now", false);
        assert_eq!(s.link, Some((5, 9)));
        let s = style_line("  https://a.b/c tail", false);
        assert_eq!(s.link, Some((2, 15)));
    }
}
