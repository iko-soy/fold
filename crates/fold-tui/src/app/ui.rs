//! Drawing (§10.1). Everything clickable registers a hit region while it
//! is drawn, so what the pointer can do is exactly what is on screen.

use super::action::{Action, NODE_MENU};
use super::markdown::style_line;
use super::{App, Focus, Mode, PromptAction};
use fold_core::ops::Drop;
use fold_core::parse::{Kind, TaskState};
use fold_core::render::render;
use fold_core::tree::NRef;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

/// Colours, in one place (256-colour indices where the defaults are too
/// harsh on dark and light themes alike).
pub mod theme {
    use ratatui::style::Color;
    pub const ACCENT: Color = Color::Cyan;
    pub const DIM: Color = Color::DarkGray;
    pub const DONE: Color = Color::DarkGray;
    pub const H1: Color = Color::Cyan;
    pub const H2: Color = Color::LightBlue;
    pub const H3: Color = Color::White;
    pub const LINK: Color = Color::LightCyan;
    pub const CODE: Color = Color::Yellow;
    pub const CODE_BG: Color = Color::Indexed(236);
    pub const QUOTE: Color = Color::Green;
    pub const WARN: Color = Color::Yellow;
    pub const DANGER: Color = Color::LightRed;
    pub const SEL: Color = Color::Indexed(24);
    pub const SEL_BLUR: Color = Color::Indexed(238);
    pub const HOVER: Color = Color::Indexed(236);
    pub const DROP: Color = Color::Indexed(22);
    pub const BUTTON: Color = Color::Indexed(237);
    pub const BUTTON_HOVER: Color = Color::Indexed(240);
    pub const BAR: Color = Color::Indexed(235);
}

/// What is under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Everything outside an open popup: a click closes it.
    Backdrop,
    /// A button; with a node, the action applies to that node.
    Button(Action, Option<NRef>),
    /// A breadcrumb segment; `None` is the vault root.
    Crumb(Option<NRef>),
    OutlinePane,
    ReadingPane,
    /// The border between the panes: drag to resize.
    Divider,
    Row(usize),
    Fold(usize),
    Check(usize),
    RowMenu(usize),
    DocLine(usize),
    DocCheck(usize),
    Link(usize),
    EditArea,
    MenuItem(usize),
    PaletteRow(usize),
    PickRow(usize),
    FilterRow(usize),
    PropValue(usize),
    PropDelete(usize),
}

/// The node menu: which node, where, and the highlighted item.
#[derive(Debug, Clone, Copy)]
pub struct Menu {
    pub target: NRef,
    pub x: u16,
    pub y: u16,
    pub sel: usize,
}

/// The left button is down: where it went down, on which outline row, and
/// whether the pointer has moved enough to be a drag.
#[derive(Debug, Clone, Copy)]
pub struct Press {
    pub row: Option<usize>,
    pub x: u16,
    pub y: u16,
    pub dragging: bool,
}

/// Pointer and layout state that lives between frames.
#[derive(Default)]
pub struct Ui {
    pub hits: Vec<(Rect, Hit)>,
    pub hover: Option<(u16, u16)>,
    pub last_click: Option<(u16, u16, Instant)>,
    pub press: Option<Press>,
    /// While dragging an outline row: the row it would land on, and how.
    pub drop: Option<(usize, Drop)>,
    pub resizing: bool,
    /// Outline pane width set by dragging the divider.
    pub outline_width: Option<u16>,
    pub menu: Option<Menu>,
    /// Visible outline rows: (row index, screen y, column where the title starts).
    pub rows_geom: Vec<(usize, u16, u16)>,
    pub last_cursor: Option<usize>,
    pub last_read_cursor: Option<usize>,
    pub last_edit_line: Option<usize>,
    pub edit_scroll: usize,
    pub edit_area: Rect,
    /// The left button went down in the editor: moving selects.
    pub edit_drag: bool,
    pub outline_view: usize,
    pub reading_view: usize,
    pub reading_len: usize,
    /// Highlighted code blocks by (info string, code).
    pub highlight_cache: std::collections::HashMap<(String, String), Vec<Vec<Span<'static>>>>,
}

impl Ui {
    fn push(&mut self, r: Rect, h: Hit) {
        if r.width > 0 && r.height > 0 {
            self.hits.push((r, h));
        }
    }

    /// The topmost hit at a point.
    pub fn hit_at(&self, x: u16, y: u16) -> Option<Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .map(|(_, h)| *h)
    }

    /// Where to open the node menu for the keyboard: beside the cursor row.
    pub fn anchor_for_cursor(&self, pane: Rect, cursor: usize, scroll: usize) -> (u16, u16) {
        let dy = cursor.saturating_sub(scroll) as u16;
        (pane.x + 4, (pane.y + 2 + dy).min(pane.y + pane.height.saturating_sub(1)))
    }

    fn hovered(&self, r: Rect) -> bool {
        self.hover
            .map(|(x, y)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .unwrap_or(false)
    }
}

/// Keep `cursor` visible in a view of `view` lines when it has moved;
/// otherwise leave the scroll where the wheel put it. Always clamp.
fn follow(scroll: &mut usize, cursor: usize, moved: bool, view: usize, len: usize) {
    if moved && view > 0 {
        if cursor < *scroll {
            *scroll = cursor;
        } else if cursor >= *scroll + view {
            *scroll = cursor + 1 - view;
        }
    }
    *scroll = (*scroll).min(len.saturating_sub(view.max(1)));
}

fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, max: u16, style: Style) -> u16 {
    if max == 0 {
        return 0;
    }
    let (end, _) = buf.set_stringn(x, y, s, max as usize, style);
    end.saturating_sub(x)
}

/// Cut a string to `w` columns, ending in `…` if it had to be cut.
fn fit(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw + 1 > w {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('…');
    out
}

fn rounded(title: Line<'static>, focused: bool) -> WBlock<'static> {
    WBlock::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused { theme::ACCENT } else { theme::DIM }))
        .title(title)
}

impl App {
    pub fn draw(&mut self, f: &mut Frame) {
        self.refresh_picks();
        self.ui.hits.clear();
        self.ui.rows_geom.clear();
        let area = f.area();
        if area.height < 3 || area.width < 10 {
            return;
        }
        let top = Rect { height: 1, ..area };
        let status = Rect { y: area.y + area.height - 1, height: 1, ..area };
        let body = Rect { y: area.y + 1, height: area.height - 2, ..area };

        if self.mode == Mode::Conflict {
            self.draw_conflict(f, body);
        } else {
            let narrow = area.width < 80;
            let (outline, reading) = if narrow {
                let h = body.height / 2;
                (Rect { height: h, ..body }, Rect { y: body.y + h, height: body.height - h, ..body })
            } else {
                let want = self.ui.outline_width.unwrap_or((body.width / 3).max(30));
                let w = want.clamp(20, body.width.saturating_sub(30).max(20));
                (Rect { width: w, ..body }, Rect { x: body.x + w, width: body.width - w, ..body })
            };
            self.pane_outline = outline;
            self.pane_reading = reading;
            self.ui.push(outline, Hit::OutlinePane);
            self.ui.push(reading, Hit::ReadingPane);
            self.draw_outline(f, outline);
            if self.mode == Mode::Edit {
                self.draw_editor(f, reading);
            } else {
                self.draw_reading(f, reading);
            }
            if !narrow {
                // the outline's right border is the handle
                let handle = Rect { x: outline.x + outline.width - 1, y: outline.y + 1, width: 1, height: outline.height.saturating_sub(2) };
                self.ui.push(handle, Hit::Divider);
                if self.ui.resizing || self.ui.hovered(handle) {
                    for y in handle.y..handle.y + handle.height {
                        f.buffer_mut()[(handle.x, y)].set_style(Style::default().fg(theme::ACCENT));
                    }
                }
            }
        }
        self.draw_topbar(f, top);
        self.draw_status(f, status);

        match self.mode {
            Mode::Props => self.draw_props(f, area),
            Mode::Filter => self.draw_filter(f, area),
            Mode::Picker => self.draw_palette(f, area),
            Mode::Help => self.draw_help(f, area),
            _ => {}
        }
        if self.prompt.is_some() {
            self.draw_prompt(f, area);
        }
        if self.ui.menu.is_some() {
            self.draw_menu(f, area);
        }
    }

    // ------------------------------------------------------------ bars

    /// Ancestors of the zoom root through embeds, top first.
    fn crumbs(&self) -> Vec<NRef> {
        self.zoom_root.map(|z| self.chain(z)).unwrap_or_default()
    }

    /// A node and its ancestors, top first, following block roots up to the
    /// embeds that stitch them in (so a block's path is its place in the
    /// tree, not its file).
    pub(super) fn chain(&self, r: NRef) -> Vec<NRef> {
        let tree = &self.vault.tree;
        let mut out = Vec::new();
        let mut cur = Some(r);
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 1000 {
                break;
            }
            let n = tree.node(c);
            if n.kind == Kind::Root {
                break;
            }
            if !n.is_embed() {
                out.push(c);
            }
            cur = match n.parent.map(|p| (c.0, p)) {
                Some(p) if tree.node(p).kind != Kind::Root => Some(p),
                Some(_) if c.0 != tree.root.0 => n
                    .block
                    .as_ref()
                    .and_then(|b| b.id.as_ref())
                    .and_then(|id| tree.embed_of(id)),
                _ => None,
            };
        }
        out.reverse();
        out
    }

    fn button(&mut self, buf: &mut Buffer, x: u16, y: u16, a: Action, target: Option<NRef>, compact: bool, max: u16) -> u16 {
        let compact = compact || a == Action::NodeMenu;
        let text = match (a.icon(), compact) {
            (Some(i), true) => format!(" {} ", i),
            (Some(i), false) => format!(" {} {} ", i, a.label()),
            (None, _) => format!(" {} ", a.label()),
        };
        let w = (text.width() as u16).min(max);
        let r = Rect { x, y, width: w, height: 1 };
        let bg = if self.ui.hovered(r) { theme::BUTTON_HOVER } else { theme::BUTTON };
        put(buf, x, y, &text, w, Style::default().bg(bg).fg(ratatui::style::Color::White));
        self.ui.push(r, Hit::Button(a, target));
        w
    }

    /// A node's path through embeds, as titles.
    pub(super) fn path_titles(&self, r: NRef) -> Vec<String> {
        self.chain(r).iter().map(|&c| self.vault.tree.node(c).title.clone()).collect()
    }

    /// Width a row of buttons needs.
    fn buttons_width(actions: &[Action], compact: bool) -> u16 {
        actions
            .iter()
            .map(|&a| {
                let compact = compact || a == Action::NodeMenu;
                let t = match (a.icon(), compact) {
                    (Some(i), true) => format!(" {} ", i),
                    (Some(i), false) => format!(" {} {} ", i, a.label()),
                    (None, _) => format!(" {} ", a.label()),
                };
                t.width() as u16 + 1
            })
            .sum()
    }

    /// Right-aligned buttons ending at `right`; returns where they start.
    fn buttons_right(&mut self, buf: &mut Buffer, right: u16, y: u16, actions: &[Action], target: Option<NRef>, room: u16) -> u16 {
        let compact = Self::buttons_width(actions, false) > room;
        let w = Self::buttons_width(actions, compact).min(room);
        let mut x = right.saturating_sub(w);
        let start = x;
        for &a in actions {
            let used = self.button(buf, x, y, a, target, compact, right.saturating_sub(x));
            x += used + 1;
            if x >= right {
                break;
            }
        }
        start
    }

    fn draw_topbar(&mut self, f: &mut Frame, area: Rect) {
        let buf = f.buffer_mut();
        buf.set_style(area, Style::default().bg(theme::BAR));
        let actions = [Action::Filter, Action::Capture, Action::Undo, Action::Redo, Action::Palette, Action::Help];
        let crumbs = self.crumbs();
        let crumb_w: u16 = crumbs.iter().map(|&c| self.vault.tree.node(c).title.width() as u16 + 3).sum::<u16>() + 6;
        let room = area.width.saturating_sub(crumb_w.min(area.width / 2));
        let right = area.x + area.width;
        let start = self.buttons_right(buf, right, area.y, &actions, None, room);
        // breadcrumb: "fold › Homelab › NAS", cut from the left if long
        let mut x = area.x + 1;
        let home = Rect { x, y: area.y, width: 4, height: 1 };
        let home_style = Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD).bg(theme::BAR);
        put(buf, x, area.y, "fold", 4, if self.ui.hovered(home) { home_style.add_modifier(Modifier::UNDERLINED) } else { home_style });
        self.ui.push(home, Hit::Crumb(None));
        x += 4;
        let limit = start.saturating_sub(1);
        let titles: Vec<String> = crumbs.iter().map(|&c| self.vault.tree.node(c).title.clone()).collect();
        let total: u16 = titles.iter().map(|t| t.width() as u16 + 3).sum();
        let mut skip = 0;
        let mut need = total;
        while x + need > limit && skip < titles.len() {
            need -= titles[skip].width() as u16 + 3;
            skip += 1;
        }
        if skip > 0 {
            x += put(buf, x, area.y, " › …", limit.saturating_sub(x), Style::default().fg(theme::DIM).bg(theme::BAR));
        }
        for (i, (&c, t)) in crumbs.iter().zip(&titles).enumerate().skip(skip) {
            x += put(buf, x, area.y, " › ", limit.saturating_sub(x), Style::default().fg(theme::DIM).bg(theme::BAR));
            let w = fit(t, limit.saturating_sub(x) as usize);
            let r = Rect { x, y: area.y, width: w.width() as u16, height: 1 };
            let last = i + 1 == crumbs.len();
            let mut style = Style::default().bg(theme::BAR);
            if last {
                style = style.add_modifier(Modifier::BOLD);
            }
            if self.ui.hovered(r) {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            x += put(buf, x, area.y, &w, limit.saturating_sub(x), style);
            self.ui.push(r, Hit::Crumb(Some(c)));
        }
    }

    fn draw_status(&mut self, f: &mut Frame, area: Rect) {
        let file = self
            .current()
            .map(|r| self.vault.tree.files[r.0].path.clone())
            .unwrap_or_else(|| "root.md".into());
        let unsaved = self.mode == Mode::Edit
            && self.editor_dirty();
        let buf = f.buffer_mut();
        buf.set_style(area, Style::default().bg(theme::BAR));
        let mut x = area.x + 1;
        let badge = match self.mode {
            Mode::Edit => Some(" EDIT "),
            Mode::Filter => Some(" FILTER "),
            Mode::Picker => Some(" COMMANDS "),
            Mode::Conflict => Some(" CONFLICTS "),
            Mode::Props => Some(" PROPERTIES "),
            Mode::Help => Some(" HELP "),
            Mode::Normal => None,
        };
        if let Some(b) = badge {
            x += put(buf, x, area.y, b, area.width, Style::default().bg(theme::ACCENT).fg(ratatui::style::Color::Black).add_modifier(Modifier::BOLD));
            x += 1;
        }
        // right side: conflicts · hidden done · file · saved · help
        let conflicts = fold_core::merge::conflict_pairs(&self.vault).len();
        let mut right: Vec<(String, Style, Option<Action>)> = Vec::new();
        if conflicts > 0 {
            right.push((format!(" ⚠ {} conflict{} ", conflicts, if conflicts == 1 { "" } else { "s" }), Style::default().fg(theme::WARN).bg(theme::BAR), Some(Action::ResolveConflicts)));
        }
        if self.hide_done {
            right.push((" done hidden ".into(), Style::default().fg(theme::DIM).bg(theme::BAR), Some(Action::HideDone)));
        }
        right.push((format!(" {} ", file), Style::default().fg(theme::DIM).bg(theme::BAR), None));
        right.push((
            if unsaved { " ● unsaved ".into() } else { " ✓ saved ".into() },
            Style::default().fg(if unsaved { theme::WARN } else { theme::DIM }).bg(theme::BAR),
            None,
        ));
        let rw: u16 = right.iter().map(|(t, _, _)| t.width() as u16).sum();
        let mut rx = (area.x + area.width).saturating_sub(rw);
        let msg_room = rx.saturating_sub(x + 1);
        put(buf, x, area.y, &fit(&self.status, msg_room as usize), msg_room, Style::default().bg(theme::BAR));
        for (t, style, a) in right {
            let w = t.width() as u16;
            let r = Rect { x: rx, y: area.y, width: w, height: 1 };
            let style = if a.is_some() && self.ui.hovered(r) { style.add_modifier(Modifier::UNDERLINED) } else { style };
            put(buf, rx, area.y, &t, w, style);
            if let Some(a) = a {
                self.ui.push(r, Hit::Button(a, None));
            }
            rx += w;
        }
    }

    // ------------------------------------------------------------ outline

    fn draw_outline(&mut self, f: &mut Frame, area: Rect) {
        let rows = self.rows();
        let focused = self.focus == Focus::Outline && self.mode != Mode::Edit;
        let title = Line::from(Span::styled(" Outline ", Style::default().add_modifier(Modifier::BOLD)));
        let block = rounded(title, focused);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let view = inner.height as usize;
        self.ui.outline_view = view;
        let moved = self.ui.last_cursor != Some(self.cursor);
        self.ui.last_cursor = Some(self.cursor);
        follow(&mut self.outline_scroll, self.cursor, moved, view, rows.len());
        if rows.is_empty() {
            let msg = "Nothing here yet — click ＋ Capture, or press n";
            let p = Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(theme::DIM))));
            f.render_widget(p, Rect { y: inner.y + inner.height / 2, height: 1, ..inner });
            return;
        }
        let dragging_from = self.ui.press.filter(|p| p.dragging).and_then(|p| p.row);
        for (vi, row) in rows.iter().enumerate().skip(self.outline_scroll).take(view) {
            let y = inner.y + (vi - self.outline_scroll) as u16;
            let line = Rect { x: inner.x, y, width: inner.width, height: 1 };
            let n = self.vault.tree.node(row.nref);
            let selected = vi == self.cursor;
            let hovered = self.ui.hovered(line) && self.ui.menu.is_none();
            let drop = self.ui.drop.filter(|(j, _)| *j == vi).map(|(_, d)| d);
            let bg = if drop == Some(Drop::Into) {
                Some(theme::DROP)
            } else if selected {
                Some(if focused { theme::SEL } else { theme::SEL_BLUR })
            } else if hovered {
                Some(theme::HOVER)
            } else {
                None
            };
            let buf = f.buffer_mut();
            let base = bg.map(|b| Style::default().bg(b)).unwrap_or_default();
            buf.set_style(line, base);
            self.ui.push(line, Hit::Row(vi));

            let mut x = inner.x + (row.depth as u16) * 2;
            let kids = !self.vault.tree.resolved_children(row.nref).is_empty();
            if kids {
                let folded = self.is_folded(row.nref);
                put(buf, x, y, if folded { "▸" } else { "▾" }, 1, base.fg(theme::DIM));
                self.ui.push(Rect { x, y, width: 2, height: 1 }, Hit::Fold(vi));
            }
            x += 2;
            if let Some(st) = n.task {
                let (g, c) = match st {
                    TaskState::Open => ("☐", theme::ACCENT),
                    TaskState::Done => ("☑", theme::DONE),
                };
                put(buf, x, y, g, 1, base.fg(c));
                self.ui.push(Rect { x, y, width: 2, height: 1 }, Hit::Check(vi));
                x += 2;
            }
            let title_x = x;
            self.ui.rows_geom.push((vi, y, title_x));

            // right-hand meta: due date, open/total, and the ⋯ handle
            let mut meta = String::new();
            if let Some(due) = n.block.as_ref().and_then(|b| b.prop("due")) {
                meta.push_str(&format!("⏲ {}", due.get(5..).unwrap_or(due)));
            }
            let (open, total) = self.vault.tree.task_counts(row.nref);
            let own = n.task.is_some() as usize;
            if kids && total > own {
                if !meta.is_empty() {
                    meta.push_str("  ");
                }
                meta.push_str(&format!("{}/{}", open.saturating_sub((n.task == Some(TaskState::Open)) as usize), total - own));
            }
            let handle = (hovered || selected) && inner.width > 12;
            let right = inner.x + inner.width - if handle { 2 } else { 0 };
            let meta_w = meta.width() as u16;
            let meta_x = right.saturating_sub(meta_w + 1);
            let title_room = meta_x.saturating_sub(title_x + 1);
            let mut style = base;
            if n.task == Some(TaskState::Done) {
                style = style.fg(theme::DONE).add_modifier(Modifier::CROSSED_OUT);
            } else if n.kind == Kind::Section {
                style = style.add_modifier(Modifier::BOLD);
            }
            if dragging_from == Some(vi) {
                style = style.add_modifier(Modifier::DIM);
            }
            let title = if n.title.is_empty() { "(untitled)".to_string() } else { n.title.clone() };
            let marker = if n.is_block() || n.is_embed() { " ▤" } else { "" };
            let shown = fit(&format!("{}{}", title, marker), title_room as usize);
            put(buf, title_x, y, &shown, title_room, style);
            if meta_w > 0 && meta_x > title_x {
                put(buf, meta_x, y, &meta, meta_w, base.fg(theme::DIM));
            }
            if handle {
                let hx = inner.x + inner.width - 2;
                put(buf, hx, y, "⋯", 1, base.fg(theme::ACCENT));
                self.ui.push(Rect { x: hx, y, width: 2, height: 1 }, Hit::RowMenu(vi));
            }
            if drop == Some(Drop::Before) {
                // an insertion bar across the top of the row
                for cx in title_x.saturating_sub(1)..inner.x + inner.width {
                    buf[(cx, y)].set_style(Style::default().add_modifier(Modifier::UNDERLINED));
                }
                // and a pointer in the indent, where there is room for one
                if row.depth > 0 {
                    let mx = inner.x + (row.depth as u16) * 2 - 1;
                    put(buf, mx, y, "▶", 1, Style::default().fg(theme::QUOTE));
                }
            }
        }
        if rows.len() > view {
            let mut st = ScrollbarState::new(rows.len().saturating_sub(view)).position(self.outline_scroll);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None),
                Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area },
                &mut st,
            );
        }
    }

    // ------------------------------------------------------------ reading

    /// The node the reading pane shows.
    pub(super) fn reading_target(&self) -> Option<NRef> {
        self.zoom_root.or_else(|| self.current())
    }

    fn pane_title(&self, r: Option<NRef>, prefix: &str) -> Line<'static> {
        let mut spans = vec![Span::raw(" ")];
        if !prefix.is_empty() {
            spans.push(Span::styled(prefix.to_string(), Style::default().fg(theme::ACCENT)));
        }
        if let Some(r) = r {
            let n = self.vault.tree.node(r);
            let t = if n.kind == Kind::Root { "fold".into() } else { n.title.clone() };
            spans.push(Span::styled(t, Style::default().add_modifier(Modifier::BOLD)));
            if n.is_block() {
                spans.push(Span::styled(" ▤", Style::default().fg(theme::DIM)));
            }
        }
        spans.push(Span::raw(" "));
        Line::from(spans)
    }

    fn draw_reading(&mut self, f: &mut Frame, area: Rect) {
        self.sync_read_target();
        let focused = self.focus == Focus::Reading;
        let target = self.reading_target();
        let block = rounded(self.pane_title(target, ""), focused);
        let inner = block.inner(area);
        f.render_widget(block, area);
        // a column of air on the left of the text
        let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner };
        if let Some(t) = target {
            let room = area.width.saturating_sub(4) / 2;
            self.buttons_right(f.buffer_mut(), area.x + area.width - 2, area.y, &[Action::Edit, Action::NodeMenu], Some(t), room);
        }
        let Some(r) = target else {
            let p = Paragraph::new(Span::styled("Select a node to read it", Style::default().fg(theme::DIM)));
            f.render_widget(p, inner);
            return;
        };
        // display lines: (doc line or None for the property header, text)
        let mut shown: Vec<(Option<usize>, String)> = Vec::new();
        self.read_header = false;
        if self.raw_mode {
            for l in render(&self.vault.tree, r, 1, false).lines() {
                shown.push((None, l.to_string()));
            }
        } else {
            let doc = fold_core::reading::build(&self.vault, r);
            for (i, l) in doc.lines.iter().enumerate() {
                shown.push((Some(i), l.clone()));
            }
            if let Some(b) = &self.vault.tree.node(r).block {
                let props: Vec<String> = b.props.iter().filter(|(k, _)| k.as_str() != "id").map(|(k, v)| format!("{} {}", k, v)).collect();
                if !props.is_empty() {
                    shown.insert(1.min(shown.len()), (None, format!("⚑ {}", props.join(" · "))));
                    self.read_header = true;
                }
            }
        }
        let view = inner.height as usize;
        self.ui.reading_view = view;
        self.ui.reading_len = shown.len();
        let cursor_shown = self.read_cursor + (self.read_header && self.read_cursor >= 1) as usize;
        let moved = self.ui.last_read_cursor != Some(self.read_cursor);
        self.ui.last_read_cursor = Some(self.read_cursor);
        follow(&mut self.scroll_reading, cursor_shown, moved, view, shown.len());
        let texts: Vec<&str> = shown.iter().map(|(_, t)| t.as_str()).collect();
        let code = self.code_lines(&texts);
        for (si, (doc_line, text)) in shown.iter().enumerate() {
            if si < self.scroll_reading || si >= self.scroll_reading + view {
                continue;
            }
            let y = inner.y + (si - self.scroll_reading) as u16;
            let line_rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
            let buf = f.buffer_mut();
            let Some(di) = doc_line else {
                put(buf, inner.x, y, &fit(text, inner.width as usize), inner.width, Style::default().fg(theme::DIM).add_modifier(Modifier::ITALIC));
                continue;
            };
            let styled = match &code[si] {
                Some(spans) => super::markdown::Styled { line: Line::from(spans.clone()), check: None, link: None },
                None => style_line(text, false),
            };
            let cur = focused && *di == self.read_cursor;
            let matched = self.read_matches.contains(di);
            let bg = if cur {
                Some(theme::SEL_BLUR)
            } else if matched {
                Some(ratatui::style::Color::Indexed(58))
            } else if self.ui.hovered(line_rect) && self.ui.menu.is_none() {
                Some(theme::HOVER)
            } else if code[si].is_some() {
                Some(theme::CODE_BG)
            } else {
                None
            };
            if let Some(b) = bg {
                buf.set_style(line_rect, Style::default().bg(b));
            }
            buf.set_line(inner.x, y, &styled.line, inner.width);
            self.ui.push(line_rect, Hit::DocLine(*di));
            if let Some(c) = styled.check {
                self.ui.push(Rect { x: inner.x + c, y, width: 1, height: 1 }, Hit::DocCheck(*di));
            }
            if let Some((a, b)) = styled.link {
                self.ui.push(Rect { x: inner.x + a, y, width: b - a, height: 1 }, Hit::Link(*di));
            }
        }
        if shown.len() > view {
            let mut st = ScrollbarState::new(shown.len().saturating_sub(view)).position(self.scroll_reading);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None),
                Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area },
                &mut st,
            );
        }
    }

    /// The lines inside fenced code blocks, styled: highlighted by the
    /// fence's language (§10.9), else in the plain code colour. `None` for
    /// every line that is not code, fences included.
    fn code_lines(&mut self, texts: &[&str]) -> Vec<Option<Vec<Span<'static>>>> {
        let mut out: Vec<Option<Vec<Span<'static>>>> = vec![None; texts.len()];
        let mut i = 0;
        while i < texts.len() {
            let t = texts[i].trim_start();
            let indent = texts[i].len() - t.len();
            let fc = match t.chars().next() {
                Some(c @ ('`' | '~')) => c,
                _ => {
                    i += 1;
                    continue;
                }
            };
            let n = t.chars().take_while(|&c| c == fc).count();
            if n < 3 {
                i += 1;
                continue;
            }
            let info = t[n..].trim().to_string();
            // the block runs to a fence of the same character, at least as long
            let end = (i + 1..texts.len())
                .find(|&j| {
                    let u = texts[j].trim_start();
                    u.chars().take_while(|&c| c == fc).count() >= n && u.trim_end().chars().all(|c| c == fc)
                })
                .unwrap_or(texts.len());
            let body: Vec<&str> = texts[i + 1..end]
                .iter()
                .map(|l| {
                    let cut = l.len() - l.trim_start_matches(' ').len();
                    &l[cut.min(indent)..]
                })
                .collect();
            let mut code = body.join("\n");
            code.push('\n');
            let lit = self.highlighted(&info, &code);
            for (k, j) in (i + 1..end).enumerate() {
                let mut spans = vec![Span::raw(" ".repeat(indent))];
                match lit.as_ref().and_then(|l| l.get(k)) {
                    Some(line) => spans.extend(line.iter().cloned()),
                    None => spans.push(Span::styled(body[k].to_string(), Style::default().fg(theme::CODE))),
                }
                out[j] = Some(spans);
            }
            i = end + 1;
        }
        out
    }

    /// Highlight a block, remembering the result: the reading pane redraws
    /// on every event, the code rarely changes.
    fn highlighted(&mut self, info: &str, code: &str) -> Option<Vec<Vec<Span<'static>>>> {
        if !super::highlight::supported(info) {
            return None;
        }
        let key = (info.to_string(), code.to_string());
        if let Some(hit) = self.ui.highlight_cache.get(&key) {
            return Some(hit.clone());
        }
        let lit = super::highlight::highlight(info, code)?;
        if self.ui.highlight_cache.len() > 64 {
            self.ui.highlight_cache.clear();
        }
        self.ui.highlight_cache.insert(key, lit.clone());
        Some(lit)
    }

    fn draw_editor(&mut self, f: &mut Frame, area: Rect) {
        let Some(ed) = &self.editor else { return };
        let owner = ed.buf.owner_at(ed.cursor.line);
        let owner_title = ed.buf.owners.get(&owner).map(|o| o.title.clone()).unwrap_or_default();
        let mut title = vec![
            Span::styled(" Editing ", Style::default().fg(theme::ACCENT)),
            Span::styled(owner_title, Style::default().add_modifier(Modifier::BOLD)),
            if ed.buf.dirty.is_empty() { Span::raw(" ") } else { Span::styled(" ● ", Style::default().fg(theme::WARN)) },
        ];
        let mode = ed.mode_name();
        if !mode.is_empty() {
            let bg = match mode {
                "INSERT" => ratatui::style::Color::Green,
                "NORMAL" => theme::ACCENT,
                _ => ratatui::style::Color::Magenta,
            };
            title.push(Span::styled(format!(" {} ", mode), Style::default().bg(bg).fg(ratatui::style::Color::Black).add_modifier(Modifier::BOLD)));
            title.push(Span::raw(" "));
        }
        let keys_label = ed.keys.name();
        let block = rounded(Line::from(title), true);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let room = area.width.saturating_sub(4) / 2;
        let start = self.buttons_right(f.buffer_mut(), area.x + area.width - 2, area.y, &[Action::EditDone, Action::EditRevert], None, room);
        // the keymap, a click away from the next one
        let label = format!(" ⌨ {} ", keys_label);
        let lw = label.width() as u16;
        if start > area.x + lw + 2 {
            let r = Rect { x: start - lw - 1, y: area.y, width: lw, height: 1 };
            let bg = if self.ui.hovered(r) { theme::BUTTON_HOVER } else { theme::BUTTON };
            put(f.buffer_mut(), r.x, r.y, &label, lw, Style::default().bg(bg).fg(ratatui::style::Color::White));
            self.ui.push(r, Hit::Button(Action::EditorKeys, None));
        }
        self.ui.edit_area = inner;
        self.ui.push(inner, Hit::EditArea);
        // a command, search or message takes the pane's last line
        let ed = self.editor.as_ref().unwrap();
        let bottom = match (&ed.cmdline, &ed.message) {
            (Some(c), _) => Some((format!("{}{}", c.kind, c.text), true)),
            (None, Some(m)) => Some((m.clone(), false)),
            _ => None,
        };
        let view = inner.height as usize - bottom.is_some() as usize;
        let (cursor, sel, block_cur) = (ed.cursor, ed.selection(), ed.block_cursor());
        let lines: Vec<(String, bool)> = ed.buf.lines.iter().map(|l| (l.text.clone(), l.owner == owner)).collect();
        let moved = self.ui.last_edit_line != Some(cursor.line);
        self.ui.last_edit_line = Some(cursor.line);
        follow(&mut self.ui.edit_scroll, cursor.line, moved, view, lines.len());
        if let Some(ed) = self.editor.as_mut() {
            ed.page = view.max(1);
        }
        let buf = f.buffer_mut();
        for (i, (text, own)) in lines.iter().enumerate().skip(self.ui.edit_scroll).take(view) {
            let y = inner.y + (i - self.ui.edit_scroll) as u16;
            if i == cursor.line {
                buf.set_style(Rect { x: inner.x, y, width: inner.width, height: 1 }, Style::default().bg(theme::HOVER));
            }
            // the block being edited is bright, the rest of the subtree is
            // context
            let style = if *own { Style::default() } else { Style::default().fg(ratatui::style::Color::Gray) };
            put(buf, inner.x, y, text, inner.width, style);
            if let Some((s, e)) = sel {
                if i >= s.line && i <= e.line {
                    let len = text.chars().count();
                    let from = if i == s.line { s.col } else { 0 };
                    // a selected line end shows as one cell
                    let to = if i == e.line { e.col } else { len + 1 };
                    for col in from..to {
                        let x = inner.x + col as u16;
                        if x < inner.x + inner.width {
                            buf[(x, y)].set_style(Style::default().bg(theme::SEL));
                        }
                    }
                }
            }
        }
        if let Some((text, input)) = bottom {
            let y = inner.y + inner.height - 1;
            let style = if input { Style::default() } else { Style::default().fg(theme::WARN) };
            buf.set_style(Rect { x: inner.x, y, width: inner.width, height: 1 }, Style::default().bg(theme::BAR));
            put(buf, inner.x, y, &text, inner.width, style.bg(theme::BAR));
            if input {
                let cx = inner.x + (text.width() as u16).min(inner.width.saturating_sub(1));
                f.set_cursor_position((cx, y));
                return;
            }
        }
        if cursor.line >= self.ui.edit_scroll && cursor.line < self.ui.edit_scroll + view {
            let cx = inner.x + (cursor.col as u16).min(inner.width.saturating_sub(1));
            let cy = inner.y + (cursor.line - self.ui.edit_scroll) as u16;
            if block_cur {
                f.buffer_mut()[(cx, cy)].set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
            f.set_cursor_position((cx, cy));
        }
    }

    // ------------------------------------------------------------ popups

    /// A modal popup: everything else becomes a backdrop that closes it.
    fn popup(&mut self, f: &mut Frame, screen: Rect, r: Rect, title: &str, buttons: &[Action]) -> Rect {
        let r = r.intersection(screen);
        self.ui.push(screen, Hit::Backdrop);
        self.ui.push(r, Hit::ReadingPane);
        f.render_widget(Clear, r);
        let block = rounded(Line::from(Span::styled(format!(" {} ", title), Style::default().add_modifier(Modifier::BOLD))), true);
        let inner = block.inner(r);
        f.render_widget(block, r);
        if !buttons.is_empty() && r.height > 2 {
            let y = r.y + r.height - 1;
            self.buttons_right(f.buffer_mut(), r.x + r.width - 2, y, buttons, None, r.width.saturating_sub(4));
        }
        inner
    }

    fn centered(screen: Rect, w: u16, h: u16) -> Rect {
        let w = w.min(screen.width.saturating_sub(4)).max(10);
        let h = h.min(screen.height.saturating_sub(2)).max(3);
        Rect { x: screen.x + (screen.width - w) / 2, y: screen.y + (screen.height.saturating_sub(h)) / 3, width: w, height: h }
    }

    fn list_row(&mut self, buf: &mut Buffer, r: Rect, sel: bool, hit: Hit, spans: Vec<(String, Style)>) {
        let hovered = self.ui.hovered(r);
        let bg = if sel { Some(theme::SEL) } else if hovered { Some(theme::HOVER) } else { None };
        let base = bg.map(|b| Style::default().bg(b)).unwrap_or_default();
        buf.set_style(r, base);
        let mut x = r.x + 1;
        for (t, s) in spans {
            let room = (r.x + r.width).saturating_sub(x + 1);
            x += put(buf, x, r.y, &fit(&t, room as usize), room, base.patch(s));
        }
        self.ui.push(r, hit);
    }

    fn input_line(buf: &mut Buffer, r: Rect, icon: &str, text: &str, placeholder: &str) {
        let mut x = r.x + 1;
        x += put(buf, x, r.y, icon, 2, Style::default().fg(theme::ACCENT));
        x += 1;
        if text.is_empty() {
            put(buf, x, r.y, placeholder, r.width.saturating_sub(x - r.x), Style::default().fg(theme::DIM));
        } else {
            put(buf, x, r.y, text, r.width.saturating_sub(x - r.x), Style::default());
        }
        let cx = x + text.width() as u16;
        if cx < r.x + r.width {
            buf[(cx, r.y)].set_style(Style::default().add_modifier(Modifier::REVERSED));
        }
    }

    fn draw_menu(&mut self, f: &mut Frame, screen: Rect) {
        let Some(menu) = self.ui.menu else { return };
        let w: u16 = 28;
        let h = NODE_MENU.len() as u16 + 2;
        let x = menu.x.min(screen.x + screen.width.saturating_sub(w));
        let y = if menu.y + h > screen.y + screen.height { screen.y + screen.height.saturating_sub(h) } else { menu.y };
        let r = Rect { x, y, width: w, height: h }.intersection(screen);
        self.ui.push(screen, Hit::Backdrop);
        f.render_widget(Clear, r);
        let title = self.vault.tree.node(menu.target).title.clone();
        let block = rounded(Line::from(Span::styled(format!(" {} ", fit(&title, 20)), Style::default().add_modifier(Modifier::BOLD))), true);
        let inner = block.inner(r);
        f.render_widget(block, r);
        for (i, item) in NODE_MENU.iter().enumerate() {
            let yy = inner.y + i as u16;
            if yy >= inner.y + inner.height {
                break;
            }
            let row = Rect { x: inner.x, y: yy, width: inner.width, height: 1 };
            let buf = f.buffer_mut();
            match item {
                None => {
                    put(buf, inner.x, yy, &"─".repeat(inner.width as usize), inner.width, Style::default().fg(theme::DIM));
                }
                Some(a) => {
                    let hovered = self.ui.hovered(row);
                    if hovered {
                        if let Some(m) = self.ui.menu.as_mut() {
                            m.sel = i;
                        }
                    }
                    let sel = self.ui.menu.map(|m| m.sel == i).unwrap_or(false);
                    let base = if sel { Style::default().bg(theme::SEL) } else { Style::default() };
                    buf.set_style(row, base);
                    let label_style = if *a == Action::Delete { base.fg(theme::DANGER) } else { base };
                    put(buf, inner.x + 1, yy, a.label(), inner.width.saturating_sub(2), label_style);
                    if let Some(k) = a.key() {
                        let kw = k.width() as u16;
                        put(buf, inner.x + inner.width - kw - 1, yy, k, kw, base.fg(theme::DIM));
                    }
                    self.ui.push(row, Hit::MenuItem(i));
                }
            }
        }
    }

    fn draw_palette(&mut self, f: &mut Frame, screen: Rect) {
        let hits = self.palette_hits();
        let h = (hits.len() as u16 + 4).min(screen.height.saturating_sub(4));
        let r = Rect { y: screen.y + 2, ..Self::centered(screen, 72, h) };
        let inner = self.popup(f, screen, r, "Commands", &[Action::Close]);
        if inner.height == 0 {
            return;
        }
        Self::input_line(f.buffer_mut(), Rect { height: 1, ..inner }, "›", &self.palette, "type to filter commands");
        let list = inner.height.saturating_sub(1) as usize;
        self.palette_sel = self.palette_sel.min(hits.len().saturating_sub(1));
        let scroll = self.palette_sel.saturating_sub(list.saturating_sub(1));
        for (i, a) in hits.iter().enumerate().skip(scroll).take(list) {
            let y = inner.y + 1 + (i - scroll) as u16;
            let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
            let key = a.key().map(|k| format!("{:>6}  ", k)).unwrap_or_else(|| "        ".into());
            let spans = vec![
                (key, Style::default().fg(theme::DIM)),
                (format!("{:<22}", a.label()), Style::default().add_modifier(Modifier::BOLD)),
                (a.desc().to_string(), Style::default().fg(theme::DIM)),
            ];
            let sel = i == self.palette_sel;
            self.list_row(f.buffer_mut(), row, sel, Hit::PaletteRow(i), spans);
        }
    }

    fn draw_filter(&mut self, f: &mut Frame, screen: Rect) {
        let h = (self.filter_rows.len() as u16 + 4).max(5).min(screen.height.saturating_sub(2));
        let r = Rect { y: screen.y + 1, ..Self::centered(screen, 80, h) };
        let inner = self.popup(f, screen, r, "Filter", &[Action::Close]);
        if inner.height == 0 {
            return;
        }
        Self::input_line(f.buffer_mut(), Rect { height: 1, ..inner }, "⌕", &self.filter, "titles and text, fuzzy");
        let list = inner.height.saturating_sub(1) as usize;
        if self.filter_rows.is_empty() && !self.filter.is_empty() {
            put(f.buffer_mut(), inner.x + 1, inner.y + 1, "no matches", inner.width, Style::default().fg(theme::DIM));
        }
        let scroll = self.filter_sel.saturating_sub(list.saturating_sub(1));
        let rows: Vec<NRef> = self.filter_rows.clone();
        for (i, &r) in rows.iter().enumerate().skip(scroll).take(list) {
            let y = inner.y + 1 + (i - scroll) as u16;
            let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
            let spans = self.path_spans(r);
            self.list_row(f.buffer_mut(), row, i == self.filter_sel, Hit::FilterRow(i), spans);
        }
    }

    /// A list row for a node: its title, then its parents dimmed.
    fn path_spans(&self, r: NRef) -> Vec<(String, Style)> {
        let path = self.path_titles(r);
        let (last, parents) = path.split_last().map(|(l, p)| (l.clone(), p.join(" › "))).unwrap_or_default();
        let last = if last.is_empty() { "(untitled)".into() } else { last };
        vec![
            (last, Style::default().add_modifier(Modifier::BOLD)),
            (if parents.is_empty() { String::new() } else { format!("  {}", parents) }, Style::default().fg(theme::DIM)),
        ]
    }

    fn draw_prompt(&mut self, f: &mut Frame, screen: Rect) {
        let Some(p) = self.prompt.clone() else { return };
        let targets = matches!(p.action, PromptAction::Refile | PromptAction::GoTo);
        let h = if targets { (p.picks.len() as u16 + 4).clamp(6, 20) } else { 4 };
        let title = match p.action {
            PromptAction::Refile => "Move to — pick a node".to_string(),
            PromptAction::GoTo => "Go to — pick a node".to_string(),
            _ => p.label.clone(),
        };
        let r = Self::centered(screen, 70, h);
        let inner = self.popup(f, screen, r, &title, &[Action::PromptOk, Action::Close]);
        if inner.height == 0 {
            return;
        }
        let placeholder = match p.action {
            PromptAction::Refile | PromptAction::GoTo => "type to narrow, or an id / path",
            PromptAction::CaptureText(_) => "what's on your mind?",
            PromptAction::ReadSearch => "search this document",
            PromptAction::PropNew => "key (letters, digits, - and _)",
            PromptAction::PropSet(_) => "value",
        };
        Self::input_line(f.buffer_mut(), Rect { height: 1, ..inner }, "›", &p.text, placeholder);
        if targets {
            let list = inner.height.saturating_sub(1) as usize;
            let scroll = p.sel.saturating_sub(list.saturating_sub(1));
            for (i, &r) in p.picks.iter().enumerate().skip(scroll).take(list) {
                let y = inner.y + 1 + (i - scroll) as u16;
                let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
                let spans = self.path_spans(r);
                self.list_row(f.buffer_mut(), row, i == p.sel, Hit::PickRow(i), spans);
            }
        }
    }

    fn draw_props(&mut self, f: &mut Frame, screen: Rect) {
        let title = self.props_target.map(|t| self.vault.tree.node(t).title.clone()).unwrap_or_default();
        let h = (self.props_rows.len() as u16 + 4).max(5);
        let r = Self::centered(screen, 64, h);
        let inner = self.popup(f, screen, r, &format!("Properties — {}", fit(&title, 30)), &[Action::PropAdd, Action::Close]);
        if self.props_rows.is_empty() {
            put(f.buffer_mut(), inner.x + 1, inner.y, "No properties. Adding one gives the node its own file.", inner.width.saturating_sub(2), Style::default().fg(theme::DIM));
            return;
        }
        let keyw = self.props_rows.iter().map(|(k, _, _)| k.width()).max().unwrap_or(0).min(16);
        let rows = self.props_rows.clone();
        for (i, (k, v, editable)) in rows.iter().enumerate().take(inner.height as usize) {
            let y = inner.y + i as u16;
            let row = Rect { x: inner.x, y, width: inner.width.saturating_sub(3), height: 1 };
            let vstyle = if *editable { Style::default() } else { Style::default().fg(theme::DIM) };
            let spans = vec![
                (format!("{:<w$}  ", k, w = keyw), Style::default().fg(theme::ACCENT)),
                (v.clone(), vstyle),
            ];
            self.list_row(f.buffer_mut(), row, i == self.props_sel, Hit::PropValue(i), spans);
            if *editable {
                let dx = inner.x + inner.width - 2;
                let d = Rect { x: dx, y, width: 2, height: 1 };
                let style = if self.ui.hovered(d) { Style::default().fg(theme::DANGER).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme::DIM) };
                put(f.buffer_mut(), dx, y, "✕", 1, style);
                self.ui.push(d, Hit::PropDelete(i));
            }
        }
    }

    fn draw_help(&mut self, f: &mut Frame, screen: Rect) {
        let lines = super::help_text();
        let r = Self::centered(screen, 78, lines.len() as u16 + 2);
        let inner = self.popup(f, screen, r, "Help", &[Action::Close]);
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_conflict(&mut self, f: &mut Frame, area: Rect) {
        let pairs = fold_core::merge::conflict_pairs(&self.vault);
        let buf = f.buffer_mut();
        let head = Rect { height: 1, ..area };
        buf.set_style(head, Style::default().bg(theme::BAR));
        let label = if pairs.is_empty() {
            " No unresolved conflicts ".to_string()
        } else {
            format!(" Conflict {} of {} ", (self.conflict_idx + 1).min(pairs.len()), pairs.len())
        };
        put(buf, area.x, area.y, &label, area.width, Style::default().bg(theme::BAR).add_modifier(Modifier::BOLD));
        let actions: Vec<Action> = if pairs.is_empty() {
            vec![Action::Close]
        } else {
            vec![Action::ConflictPrev, Action::ConflictNext, Action::ConflictOurs, Action::ConflictTheirs, Action::ConflictBoth, Action::ConflictEdit, Action::Close]
        };
        self.buttons_right(buf, area.x + area.width, area.y, &actions, None, area.width.saturating_sub(label.width() as u16 + 1));
        if pairs.is_empty() {
            return;
        }
        let (ours, theirs) = pairs[self.conflict_idx.min(pairs.len() - 1)];
        let body = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
        let half = body.width / 2;
        let left = Rect { width: half, ..body };
        let right = Rect { x: body.x + half, width: body.width - half, ..body };
        let who = self.vault.tree.node(theirs).block.as_ref().and_then(|b| b.prop("conflict").map(|s| s.trim_matches('"').to_string())).unwrap_or_default();
        for (rect, r, title, color) in [
            (left, ours, " This device ".to_string(), theme::ACCENT),
            (right, theirs, format!(" Other device · {} ", who), theme::WARN),
        ] {
            let text = render(&self.vault.tree, r, 1, true);
            let block = WBlock::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(color)).title(title);
            let lines: Vec<Line> = text.lines().map(|l| style_line(l, false).line).collect();
            f.render_widget(Paragraph::new(lines).block(block), rect);
        }
    }
}
