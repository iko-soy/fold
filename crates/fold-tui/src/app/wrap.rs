//! Soft wrapping (§10.1): where a long line breaks into screen rows. Prose
//! breaks after a space, with continuation rows hung under the text of a
//! list item; code breaks hard, one column early, to leave room for `↪`.

use unicode_width::UnicodeWidthChar;

/// One screen row of a line: the characters `[start, end)`, drawn after
/// `indent` columns (zero on the first row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    pub indent: usize,
}

/// How far continuation rows are indented: under the text, past leading
/// spaces, a list marker and a checkbox.
pub fn hang(text: &str) -> usize {
    let lead = text.chars().take_while(|c| *c == ' ').count();
    let rest: String = text.chars().skip(lead).collect();
    let mut h = lead;
    let marker = ["- ", "* ", "+ "].iter().find(|m| rest.starts_with(**m)).map(|m| m.len()).or_else(|| {
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        (digits > 0 && rest[digits..].starts_with(". ")).then_some(digits + 2)
    });
    if let Some(m) = marker {
        h += m;
        let after = &rest[m..];
        for cb in ["[ ] ", "[x] ", "[X] ", "[-] "] {
            if after.starts_with(cb) {
                h += 4;
            }
        }
        for cb in ["☐ ", "☑ "] {
            if after.starts_with(cb) {
                h += 2;
            }
        }
    }
    h
}

fn width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// The rows of a line at `cols` columns. `hard` breaks anywhere and keeps
/// the last column free for a `↪` (code); otherwise lines break after a
/// space when they can. A line always has at least one row.
pub fn wrap(text: &str, cols: usize, hard: bool) -> Vec<Row> {
    let chars: Vec<char> = text.chars().collect();
    if cols < 4 || chars.iter().map(|c| width(*c)).sum::<usize>() <= cols {
        return vec![Row { start: 0, end: chars.len(), indent: 0 }];
    }
    let indent = if hard { 0 } else { hang(text).min(cols / 2) };
    let mut rows = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let ind = if rows.is_empty() { 0 } else { indent };
        let avail = cols - ind;
        // how many characters fit
        let mut w = 0;
        let mut i = start;
        while i < chars.len() && w + width(chars[i]) <= avail {
            w += width(chars[i]);
            i += 1;
        }
        let end = if i >= chars.len() {
            chars.len()
        } else if hard {
            // one column for the ↪
            (i - 1).max(start + 1)
        } else {
            // after the last space that fits, else wherever it fills
            match (start + 1..=i).rev().find(|&k| chars[k - 1] == ' ') {
                Some(k) => k,
                None => i.max(start + 1),
            }
        };
        rows.push(Row { start, end, indent: ind });
        start = end;
    }
    rows
}

/// The row and screen column of a character column, given a line's rows.
/// A column at a row's end belongs to the next row, except at the line's end.
pub fn locate(rows: &[Row], text: &str, col: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let r = rows.iter().rposition(|r| col >= r.start).unwrap_or(0);
    let row = rows[r];
    let x = row.indent + chars[row.start..col.min(chars.len()).max(row.start)].iter().map(|c| width(*c)).sum::<usize>();
    (r, x)
}

/// The character column under screen column `x` of row `r`.
pub fn column_at(rows: &[Row], text: &str, r: usize, x: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let row = rows[r.min(rows.len() - 1)];
    let mut w = row.indent;
    let mut col = row.start;
    while col < row.end && w + width(chars[col]) <= x {
        w += width(chars[col]);
        col += 1;
    }
    // the last row reaches the line's end; earlier rows stop before theirs
    let last = r + 1 >= rows.len();
    if !last && col >= row.end {
        col = row.end.saturating_sub(1).max(row.start);
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(t: &str, cols: usize, hard: bool) -> Vec<String> {
        let chars: Vec<char> = t.chars().collect();
        wrap(t, cols, hard)
            .iter()
            .map(|r| format!("{}{}", " ".repeat(r.indent), chars[r.start..r.end].iter().collect::<String>()))
            .collect()
    }

    #[test]
    fn prose_breaks_after_spaces_with_a_hanging_indent() {
        assert_eq!(texts("short", 20, false), ["short"]);
        assert_eq!(
            texts("- [ ] buy milk and bread today", 16, false),
            ["- [ ] buy milk ", "      and bread ", "      today"]
        );
        assert_eq!(texts("averyveryverylongword end", 10, false), ["averyveryv", "erylongwor", "d end"]);
    }

    #[test]
    fn code_breaks_hard_leaving_a_column() {
        assert_eq!(texts("let x = compute(a, b);", 10, true), ["let x = c", "ompute(a,", " b);"]);
    }

    #[test]
    fn locate_and_back() {
        let t = "- one two three four";
        let rows = wrap(t, 10, false);
        let (r, x) = locate(&rows, t, 12);
        assert_eq!((r, x), (1, 4));
        assert_eq!(column_at(&rows, t, r, x), 12);
        // the end of the line sits on the last row
        let (r, _) = locate(&rows, t, t.len());
        assert_eq!(r, rows.len() - 1);
    }
}
