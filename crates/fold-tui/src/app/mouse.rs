//! The pointer (§10.1): clicks, double-clicks, right-clicks, drags and the
//! wheel, all resolved through the hit map the last frame drew.

use super::ui::{Hit, Press};
use super::{App, Focus, Mode};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use fold_core::ops::{self, Drop};
use fold_core::parse::Kind;
use fold_core::reading::LineRef;
use std::time::{Duration, Instant};

impl App {
    pub fn handle_mouse(&mut self, m: MouseEvent) {
        self.handle_mouse_inner(m);
        self.settle_undo();
    }

    fn handle_mouse_inner(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Moved => self.ui.hover = Some((x, y)),
            MouseEventKind::Down(MouseButton::Left) => {
                self.ui.hover = Some((x, y));
                self.mouse_down(x, y);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.ui.hover = Some((x, y));
                self.mouse_drag(x, y);
            }
            MouseEventKind::Up(MouseButton::Left) => self.mouse_up(),
            MouseEventKind::Down(MouseButton::Right) => self.mouse_right(x, y),
            MouseEventKind::ScrollDown => self.mouse_wheel(x, y, 3),
            MouseEventKind::ScrollUp => self.mouse_wheel(x, y, -3),
            _ => {}
        }
    }

    /// The topmost clickable thing at a point in the last frame.
    pub fn hit_at(&self, x: u16, y: u16) -> Option<Hit> {
        self.ui.hit_at(x, y)
    }

    /// The centre of the first region the last frame drew for `hit` (tests).
    pub fn hit_pos(&self, hit: Hit) -> Option<(u16, u16)> {
        self.ui
            .hits
            .iter()
            .rev()
            .find(|(_, h)| *h == hit)
            .map(|(r, _)| (r.x + r.width / 2, r.y + r.height / 2))
    }

    /// Where the last frame drew a button for `a`, whatever node it carries.
    pub fn button_pos(&self, a: super::Action) -> Option<(u16, u16)> {
        self.ui
            .hits
            .iter()
            .rev()
            .find(|(_, h)| matches!(h, Hit::Button(b, _) if *b == a))
            .map(|(r, _)| (r.x + r.width / 2, r.y))
    }

    fn mouse_down(&mut self, x: u16, y: u16) {
        let double = self
            .ui
            .last_click
            .map(|(lx, ly, t)| lx == x && ly == y && t.elapsed() < Duration::from_millis(400))
            .unwrap_or(false);
        self.ui.last_click = Some((x, y, Instant::now()));
        let Some(hit) = self.ui.hit_at(x, y) else { return };
        match hit {
            Hit::Backdrop => self.close_top(),
            Hit::Button(a, target) => {
                self.action_target = target;
                self.run_action(a);
                self.action_target = None;
            }
            Hit::Crumb(r) => {
                self.zoom_to(r);
                self.focus = Focus::Outline;
            }
            Hit::OutlinePane => self.focus_pane(Focus::Outline),
            Hit::ReadingPane => self.focus_pane(Focus::Reading),
            Hit::Divider => self.ui.resizing = true,
            Hit::Row(i) => {
                self.focus_pane(Focus::Outline);
                self.cursor = i;
                if double {
                    self.run_action(super::Action::Zoom);
                } else {
                    self.ui.press = Some(Press { row: Some(i), x, y, dragging: false });
                }
            }
            Hit::Fold(i) => {
                if let Some(row) = self.rows().get(i) {
                    let r = row.nref;
                    self.toggle_fold(r);
                    self.clamp_cursor();
                }
            }
            Hit::Check(i) => {
                self.focus_pane(Focus::Outline);
                self.cursor = i;
                self.act_toggle_task();
            }
            Hit::RowMenu(i) => {
                self.cursor = i;
                if let Some(r) = self.current() {
                    self.open_menu(r, x.saturating_sub(24), y + 1);
                }
            }
            Hit::DocLine(i) => {
                self.focus_pane(Focus::Reading);
                self.read_cursor = i;
                if double {
                    self.activate_doc_line(i);
                }
            }
            Hit::DocCheck(i) => {
                self.read_cursor = i;
                self.toggle_doc_line(i);
            }
            Hit::Link(i) => {
                self.read_cursor = i;
                let doc = self.reading_doc();
                self.open_link_under_cursor(&doc);
            }
            Hit::EditArea => {
                let p = self.edit_pos(x, y);
                if let Some(ed) = self.editor.as_mut() {
                    if double {
                        ed.select_word(p);
                    } else {
                        ed.click(p);
                    }
                }
                self.ui.edit_drag = true;
                self.edit_last_key = Instant::now();
            }
            Hit::MenuItem(i) => self.run_menu_item(i),
            Hit::PaletteRow(i) => {
                self.palette_sel = i;
                self.key_palette(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ));
            }
            Hit::PickRow(i) => {
                if let Some(mut p) = self.prompt.take() {
                    p.sel = i;
                    self.accept_prompt(p);
                }
            }
            Hit::FilterRow(i) => self.pick_filter(i),
            Hit::PropValue(i) => {
                self.props_sel = i;
                if let Some((k, v, editable)) = self.props_rows.get(i).cloned() {
                    if editable {
                        self.open_prompt(&k, super::PromptAction::PropSet(k.clone()), v);
                    }
                }
            }
            Hit::PropDelete(i) => self.delete_prop(i),
        }
    }

    fn focus_pane(&mut self, f: Focus) {
        if self.mode == Mode::Normal {
            self.focus = f;
        }
    }

    fn mouse_drag(&mut self, x: u16, y: u16) {
        if self.ui.edit_drag {
            // dragging past the top or bottom scrolls the editor
            let area = self.ui.edit_area;
            if y < area.y {
                self.ui.edit_scroll = self.ui.edit_scroll.saturating_sub(1);
            } else if y >= area.y + area.height {
                self.ui.edit_scroll += 1;
            }
            let p = self.edit_pos(x, y);
            if let Some(ed) = self.editor.as_mut() {
                ed.drag_to(p);
            }
            return;
        }
        if self.ui.resizing {
            let w = x.saturating_sub(self.pane_outline.x) + 1;
            self.ui.outline_width = Some(w.max(20));
            return;
        }
        let Some(mut press) = self.ui.press else { return };
        let Some(from) = press.row else { return };
        if !press.dragging {
            if press.y == y && x.abs_diff(press.x) < 3 {
                return;
            }
            press.dragging = true;
            self.ui.press = Some(press);
        }
        // scroll when dragging onto the pane's top or bottom edge
        let pane = self.pane_outline;
        if y <= pane.y && self.outline_scroll > 0 {
            self.outline_scroll -= 1;
        } else if y + 1 >= pane.y + pane.height {
            self.outline_scroll += 1;
        }
        let rows = self.rows();
        let target = self
            .ui
            .rows_geom
            .iter()
            .find(|(_, gy, _)| *gy == y)
            .map(|&(j, _, title_x)| (j, if x < title_x { Drop::Before } else { Drop::Into }));
        self.ui.drop = target.filter(|(j, _)| *j != from);
        let name = |i: usize| rows.get(i).map(|r| self.vault.tree.node(r.nref).title.clone()).unwrap_or_default();
        let msg = match self.ui.drop {
            Some((j, Drop::Before)) => format!("move “{}” before “{}”", name(from), name(j)),
            Some((j, Drop::Into)) => format!("move “{}” into “{}”", name(from), name(j)),
            None => format!("moving “{}” — drop on a title to nest, left of it to place before", name(from)),
        };
        self.say(msg);
    }

    fn mouse_up(&mut self) {
        self.ui.resizing = false;
        if std::mem::take(&mut self.ui.edit_drag) {
            if let Some(ed) = self.editor.as_mut() {
                ed.end_drag();
            }
        }
        let press = self.ui.press.take();
        let drop = self.ui.drop.take();
        let (Some(press), Some((j, how))) = (press, drop) else { return };
        if !press.dragging {
            return;
        }
        let rows = self.rows();
        let (Some(from), Some(to)) = (press.row.and_then(|i| rows.get(i)), rows.get(j)) else { return };
        let (r, target) = (from.nref, to.nref);
        let title = self.vault.tree.node(self.vault.tree.resolved_child(r)).title.clone();
        let target_key = self.vault.key_of(self.vault.tree.resolved_child(target));
        self.push_undo("move");
        match ops::move_node(&mut self.vault, r, target, how) {
            Ok(moved) => {
                self.say(super::with_rule_note(&format!("moved “{}”", title), moved));
                // find it where it landed: among the target's children, or
                // its siblings, nearest the target
                let landed = self.vault.find_by_key(&target_key).and_then(|t| {
                    let pool = match how {
                        Drop::Into => self.vault.tree.resolved_children(t),
                        Drop::Before => self.parent_children(t),
                    };
                    pool.into_iter().rev().find(|&c| self.vault.tree.node(c).title == title)
                });
                match landed {
                    Some(n) => self.reveal(n),
                    None => self.clamp_cursor(),
                }
            }
            Err(e) => self.say(format!("can't move: {}", e)),
        }
    }

    /// A node's siblings (itself included) in the resolved tree.
    fn parent_children(&self, t: fold_core::tree::NRef) -> Vec<fold_core::tree::NRef> {
        let chain = self.chain(t);
        match chain.len() {
            0 | 1 => self.vault.tree.resolved_children(self.vault.tree.root),
            n => self.vault.tree.resolved_children(chain[n - 2]),
        }
    }

    fn mouse_right(&mut self, x: u16, y: u16) {
        match self.ui.hit_at(x, y) {
            Some(Hit::Row(i) | Hit::Fold(i) | Hit::Check(i) | Hit::RowMenu(i)) => {
                self.focus_pane(Focus::Outline);
                self.cursor = i;
                if let Some(r) = self.current() {
                    self.open_menu(r, x, y + 1);
                }
            }
            Some(Hit::DocLine(i) | Hit::DocCheck(i) | Hit::Link(i)) => {
                self.read_cursor = i;
                if let Some(r) = self.doc_line_node(i).or_else(|| self.reading_target()) {
                    self.open_menu(r, x, y + 1);
                }
            }
            Some(Hit::ReadingPane) => {
                if let Some(r) = self.reading_target() {
                    self.open_menu(r, x, y + 1);
                }
            }
            Some(Hit::Backdrop) => self.close_top(),
            _ => {}
        }
    }

    fn mouse_wheel(&mut self, x: u16, y: u16, delta: i32) {
        let step = |v: usize| (v as i64 + delta as i64).max(0) as usize;
        if self.ui.menu.is_some() {
            return;
        }
        if self.prompt.is_some() {
            if let Some(p) = self.prompt.as_mut() {
                p.sel = step(p.sel).min(p.picks.len().saturating_sub(1));
            }
            return;
        }
        match self.mode {
            Mode::Picker => self.palette_sel = step(self.palette_sel),
            Mode::Filter => {
                self.filter_sel = step(self.filter_sel).min(self.filter_rows.len().saturating_sub(1))
            }
            _ => match self.ui.hit_at(x, y) {
                Some(Hit::EditArea) => self.ui.edit_scroll = step(self.ui.edit_scroll),
                Some(Hit::ReadingPane | Hit::DocLine(_) | Hit::DocCheck(_) | Hit::Link(_)) => {
                    self.scroll_reading = step(self.scroll_reading)
                }
                Some(
                    Hit::OutlinePane | Hit::Row(_) | Hit::Fold(_) | Hit::Check(_) | Hit::RowMenu(_),
                ) => self.outline_scroll = step(self.outline_scroll),
                _ => {}
            },
        }
    }

    /// The node a reading-pane line belongs to.
    fn doc_line_node(&self, i: usize) -> Option<fold_core::tree::NRef> {
        let doc = self.reading_doc();
        match fold_core::reading::node_at(&doc, i) {
            Some(LineRef::Title(r)) | Some(LineRef::Body(r)) | Some(LineRef::Embed(r)) => Some(r),
            _ => None,
        }
    }

    fn toggle_doc_line(&mut self, i: usize) {
        if let Some(r) = self.doc_line_node(i) {
            if self.vault.tree.node(self.vault.tree.resolved_child(r)).task.is_some() {
                self.push_undo("toggle");
                let _ = ops::toggle_task(&mut self.vault, r);
            }
        }
    }

    /// Double-click in the reading pane: zoom into a heading, follow an
    /// embed, and otherwise open the editor with the cursor on that line.
    fn activate_doc_line(&mut self, i: usize) {
        let doc = self.reading_doc();
        match fold_core::reading::node_at(&doc, i) {
            Some(LineRef::Title(r)) if self.vault.tree.node(r).kind == Kind::Section && Some(r) != self.reading_target() => {
                self.zoom_into(r)
            }
            Some(LineRef::Embed(e)) => {
                let t = self.vault.tree.resolved_child(e);
                if t != e {
                    self.zoom_into(t);
                }
            }
            _ => {
                self.action_target = self.reading_target();
                self.act_edit();
                self.action_target = None;
                if let Some(ed) = self.editor.as_mut() {
                    let line = i.min(ed.lines() - 1);
                    ed.click(super::editor::Pos::new(line, 0));
                    ed.end_drag();
                }
            }
        }
    }

    /// The text position under a screen cell in the editor.
    fn edit_pos(&self, x: u16, y: u16) -> super::editor::Pos {
        let area = self.ui.edit_area;
        let line = self.ui.edit_scroll + y.saturating_sub(area.y) as usize;
        super::editor::Pos::new(line, x.saturating_sub(area.x) as usize)
    }
}
