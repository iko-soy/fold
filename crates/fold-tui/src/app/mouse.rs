//! The pointer (§10.1): clicks, double-clicks, right-clicks, drags and the
//! wheel, all resolved through the hit map the last frame drew.

use super::ui::{Hit, Press};
use super::{App, Focus, Mode};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use fold_core::ops::{self, Drop};
use fold_core::parse::Kind;
use fold_core::reading::LineRef;
use fold_core::tree::NRef;
use std::time::{Duration, Instant};

impl App {
    pub fn handle_mouse(&mut self, m: MouseEvent) {
        // a pointer only passing over is no use of it (§10.7); nor is the
        // button let go after a click, or wobbled while down: that is the
        // click, not one since what it said (§10.1). A drag's moves and
        // release are.
        let dragging = self.ui.press.is_some_and(|p| p.dragging) || self.ui.resizing;
        let used = match m.kind {
            MouseEventKind::Moved => false,
            MouseEventKind::Drag(_) | MouseEventKind::Up(_) => dragging,
            _ => true,
        };
        if used {
            self.last_input = Some(Instant::now());
        }
        let held = match m.kind {
            MouseEventKind::Down(_) => self.held_words.take(),
            _ => None,
        };
        // a click lets a half-typed key go (§10.3): what it opens takes the
        // next key, which no `z` or `g` before it turns into a verb
        if matches!(m.kind, MouseEventKind::Down(_)) {
            self.pending = None;
        }
        self.handle_mouse_inner(m);
        self.drop_held_words(held);
        self.settle_undo();
    }

    fn handle_mouse_inner(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Moved => {
                self.ui.hover = Some((x, y));
                // the pointer moving onto a menu item highlights it; one
                // resting there leaves the keys and the wheel to move it
                if let (Some(Hit::MenuItem(i)), Some(m)) = (self.ui.hit_at(x, y), self.ui.menu.as_mut()) {
                    m.sel = i;
                }
            }
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
            Hit::Popup => {}
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
                    let sel = self.current();
                    self.toggle_fold(r);
                    // a fold click only folds (§10.1): the selection stays on
                    // its node, or lands on the folded row that now hides it
                    self.cursor = i;
                    if let Some(s) = sel {
                        self.move_cursor_to(s);
                    }
                    self.clamp_cursor();
                }
            }
            Hit::Check(i) => {
                self.focus_pane(Focus::Outline);
                self.cursor = i;
                self.run_action(super::Action::ToggleDone);
            }
            Hit::RowMenu(i) => {
                self.cursor = i;
                if let Some(r) = self.current() {
                    self.open_menu(r, x.saturating_sub(24), y + 1);
                }
            }
            Hit::Conflict(i) => {
                self.focus_pane(Focus::Outline);
                self.cursor = i;
                self.run_action(super::Action::ResolveConflict);
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
            Hit::DocConflict(i) => {
                self.read_cursor = i;
                if let Some(r) = self.doc_line_node(i) {
                    self.enter_conflict_view_at(r);
                }
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
                    self.accept_prompt_saving_editor(p);
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
        // only over an outline row (§10.1): released anywhere else, over the
        // reading pane or a border, the drag does nothing
        let target = match self.ui.hit_at(x, y) {
            Some(Hit::Row(j) | Hit::Fold(j) | Hit::Check(j) | Hit::RowMenu(j) | Hit::Conflict(j)) => self
                .ui
                .rows_geom
                .iter()
                .find(|(gj, _, _)| *gj == j)
                .map(|&(j, _, title_x)| (j, if x < title_x { Drop::Before } else { Drop::Into })),
            _ => None,
        };
        let target = target.filter(|(j, _)| *j != from);
        // nor into a conflict copy, which keeping ours trashes (§12.5): the
        // copy is no target, and the bar says why
        let into_copy = target.is_some_and(|(j, how)| {
            rows.get(from).zip(rows.get(j)).is_some_and(|(f, t)| self.drop_into_copy(f.nref, t.nref, how))
        });
        self.ui.drop = target.filter(|_| !into_copy);
        let name = |i: usize| rows.get(i).map(|r| self.vault.tree.node(r.nref).title.clone()).unwrap_or_default();
        let msg = match self.ui.drop {
            _ if into_copy => format!("can't move “{}” into a conflict copy", name(from)),
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
        // the rows may have changed since the drag last said where it goes
        if self.drop_into_copy(from.nref, to.nref, how) {
            self.say(format!("can't move {} into a conflict copy", self.named(from.nref)));
            return;
        }
        // a drop is an outline verb: the editor saves first (§10.6), which
        // re-parses what it wrote, so both nodes are found again by key
        let keys = (self.vault.key_of(from.nref), self.vault.key_of(to.nref));
        let edit = self.editor_before_write("outline verb");
        let (Some(r), Some(target)) = (self.find_exact(&keys.0), self.find_exact(&keys.1)) else {
            self.say("can't move: the outline changed");
            return;
        };
        let (rr, rt) = (self.vault.tree.resolved_child(r), self.vault.tree.resolved_child(target));
        let kind = self.vault.tree.node(rr).kind;
        let name = self.named(rr);
        let on = self.on_node(r);
        self.push_undo(&format!("move {} {} {}", name, if how == Drop::Into { "into" } else { "before" }, self.named(rt)));
        match ops::move_node(&mut self.vault, r, target, how) {
            Ok(p) => {
                self.say(super::with_rule_note(&format!("moved {}", name), p.clamped, kind));
                // the editor open on it follows it (§10.6)
                self.follow(on, p.node);
                self.reveal(p.node);
            }
            Err(e) => self.say(format!("can't move: {}", e)),
        }
        self.editor_after_write(edit);
    }

    /// Whether dropping `r` on `target` puts it into a conflict copy it is
    /// not in already (§12.5): onto the copy's title, or onto or left of a
    /// row in it. Left of the copy's own row, it goes after the copy.
    fn drop_into_copy(&self, r: NRef, target: NRef, how: Drop) -> bool {
        let tree = &self.vault.tree;
        let own = self.chain(tree.resolved_child(r));
        let mut dest = self.chain(tree.resolved_child(target));
        if how == Drop::Before {
            dest.pop();
        }
        dest.iter().any(|c| tree.node(*c).conflict().is_some() && !own.contains(c))
    }

    fn mouse_right(&mut self, x: u16, y: u16) {
        match self.ui.hit_at(x, y) {
            Some(Hit::Row(i) | Hit::Fold(i) | Hit::Check(i) | Hit::RowMenu(i) | Hit::Conflict(i)) => {
                self.focus_pane(Focus::Outline);
                self.cursor = i;
                if let Some(r) = self.current() {
                    self.open_menu(r, x, y + 1);
                }
            }
            Some(Hit::DocLine(i) | Hit::DocCheck(i) | Hit::DocConflict(i) | Hit::Link(i)) => {
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
            self.menu_step(delta);
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
            Mode::Help => self.help_scroll = step(self.help_scroll),
            Mode::Filter => {
                self.filter_sel = step(self.filter_sel).min(self.filter_rows.len().saturating_sub(1))
            }
            _ => match self.ui.hit_at(x, y) {
                Some(Hit::EditArea) => self.ui.edit_scroll = step(self.ui.edit_scroll),
                Some(Hit::ReadingPane | Hit::DocLine(_) | Hit::DocCheck(_) | Hit::DocConflict(_) | Hit::Link(_)) => {
                    self.scroll_reading = step(self.scroll_reading)
                }
                Some(
                    Hit::OutlinePane | Hit::Row(_) | Hit::Fold(_) | Hit::Check(_) | Hit::RowMenu(_) | Hit::Conflict(_),
                ) => self.outline_scroll = step(self.outline_scroll),
                _ => {}
            },
        }
    }

    /// The node a reading-pane line belongs to.
    fn doc_line_node(&self, i: usize) -> Option<NRef> {
        let doc = self.reading_doc();
        match fold_core::reading::node_at(&doc, i) {
            Some(LineRef::Title(r)) | Some(LineRef::Body(r)) | Some(LineRef::Embed(r)) => Some(r),
            _ => None,
        }
    }

    fn toggle_doc_line(&mut self, i: usize) {
        if let Some(r) = self.doc_line_node(i) {
            if self.vault.tree.node(self.vault.tree.resolved_child(r)).task.is_some() {
                self.toggle_read_task(r);
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

    /// The text position under a screen cell in the editor, through the
    /// wrapped layout.
    fn edit_pos(&mut self, x: u16, y: u16) -> super::editor::Pos {
        let area = self.ui.edit_area;
        let row = self.ui.edit_scroll + y.saturating_sub(area.y) as usize;
        let col = x.saturating_sub(area.x) as usize;
        match self.editor.as_mut() {
            Some(ed) => ed.screen_to_pos(row, col),
            None => super::editor::Pos::new(row, col),
        }
    }
}
