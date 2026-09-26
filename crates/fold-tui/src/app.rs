//! The TUI application (§10).

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::ExecutableCommand;
use fold_core::ops;
use fold_core::parse::{Kind, TaskState};
use fold_core::render::render;
use fold_core::tree::NRef;
use fold_core::vault::{NodeKey, Vault};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span as TSpan};
use ratatui::widgets::{Block as WBlock, Borders, Clear, Paragraph};
use ratatui::Terminal;
use std::path::Path;
use std::time::{Duration, Instant};

// ------------------------------------------------------------ state

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Mode {
    Normal,
    Edit,
    Filter,
    Picker,
    Conflict,
    Props,
    Help,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum Focus {
    Outline,
    Reading,
}

pub struct FlatRow {
    pub nref: NRef,
    pub depth: usize,
    pub via_embed: bool,
}

pub struct App {
    pub(crate) vault: Vault,
    mode: Mode,
    focus: Focus,
    zoom_root: Option<NRef>,
    pub cursor: usize,
    folded: Vec<NodeKey>,
    hide_done: bool,
    raw_mode: bool,
    register: String,
    status: String,
    status_time: Instant,
    undo: Vec<ops::Inverse>,
    redo: Vec<ops::Inverse>,
    filter: String,
    filter_rows: Vec<NRef>,
    palette: String,
    scroll_reading: usize,
    quit: bool,
    // edit mode (§10.6)
    edit_buf: Option<fold_core::edit::EditBuffer>,
    edit_cursor: (usize, usize), // (line, col)
    edit_last_key: Instant,
    edit_saved_dot: bool,
    // property editor (§10.6)
    props_target: Option<NRef>,
    props_rows: Vec<(String, String, bool)>,
    props_sel: usize,
    props_editing: Option<String>,
    // text prompt (refile destination, capture text)
    prompt: Option<Prompt>,
    // reading pane (§10.4)
    read_cursor: usize,
    read_search: String,
    read_matches: Vec<usize>,
    read_match_idx: usize,
    // conflict view (§10.8)
    conflict_idx: usize,
    // watcher (§11.2)
    watcher: Option<notify::RecommendedWatcher>,
    watch_rx: Option<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
    last_watch_event: Instant,
    self_write_until: Instant,
    pending_reload: bool,
    // mouse support
    mouse_enabled: bool,
    pane_outline: Rect,
    pane_reading: Rect,
    outline_scroll: usize,
    last_click: Option<(u16, u16, Instant)>,
    toolbar_rect: Rect,
    toolbar: Vec<(Rect, &'static str)>,
    kb_rects: Vec<(Rect, char)>,
    kb_special: Vec<(Rect, &'static str)>,
    palette_rows: Vec<(Rect, &'static str)>,
    filter_rows_rects: Vec<(Rect, NRef)>,
    props_row_rects: Vec<Rect>,
    // two-key sequences: `z…`, `gg`, `[[` / `]]`
    pending: Option<char>,
    // the node the reading cursor belongs to; a new target resets it
    read_key: Option<NodeKey>,
    // whether the reading pane shows the property header as line 1
    read_header: bool,
}

struct Prompt {
    label: String,
    text: String,
    action: PromptAction,
}

enum PromptAction {
    Refile,
    GoTo,
    CaptureText(bool),
    PropSet(String),
    PropNew,
    ReadSearch,
}

impl App {
    pub fn new(dir: &Path) -> std::io::Result<App> {
        let vault = Vault::open(dir)?;
        Ok(App {
            vault,
            mode: Mode::Normal,
            focus: Focus::Outline,
            zoom_root: None,
            cursor: 0,
            folded: Vec::new(),
            hide_done: false,
            raw_mode: false,
            register: String::new(),
            status: "welcome".into(),
            status_time: Instant::now(),
            undo: Vec::new(),
            redo: Vec::new(),
            filter: String::new(),
            filter_rows: Vec::new(),
            palette: String::new(),
            scroll_reading: 0,
            quit: false,
            edit_buf: None,
            edit_cursor: (0, 0),
            edit_last_key: Instant::now(),
            edit_saved_dot: false,
            props_target: None,
            props_rows: Vec::new(),
            props_sel: 0,
            props_editing: None,
            prompt: None,
            read_cursor: 0,
            read_search: String::new(),
            read_matches: Vec::new(),
            read_match_idx: 0,
            conflict_idx: 0,
            watcher: None,
            watch_rx: None,
            last_watch_event: Instant::now(),
            self_write_until: Instant::now() - Duration::from_secs(1),
            pending_reload: false,
            mouse_enabled: false,
            pane_outline: Rect::default(),
            pane_reading: Rect::default(),
            outline_scroll: 0,
            last_click: None,
            toolbar_rect: Rect::default(),
            toolbar: Vec::new(),
            kb_rects: Vec::new(),
            kb_special: Vec::new(),
            palette_rows: Vec::new(),
            filter_rows_rects: Vec::new(),
            props_row_rects: Vec::new(),
            pending: None,
            read_key: None,
            read_header: false,
        })
    }

    pub fn pane_outline_pub(&self) -> Rect { self.pane_outline }
    pub fn toolbar_labels(&self) -> Vec<String> {
        self.toolbar.iter().map(|(_, a)| a.to_string()).collect()
    }
    pub fn toolbar_pos(&self, label: &str) -> Option<(u16, u16)> {
        self.toolbar
            .iter()
            .find(|(_, a)| *a == label)
            .map(|(r, _)| (r.x, r.y))
    }
    pub fn kb_pos(&self, c: char) -> Option<(u16, u16)> {
        self.kb_rects
            .iter()
            .find(|(_, k)| *k == c)
            .map(|(r, _)| (r.x, r.y))
    }
    pub fn kb_special_pos(&self, s: &str) -> Option<(u16, u16)> {
        self.kb_special
            .iter()
            .find(|(_, k)| *k == s)
            .map(|(r, _)| (r.x, r.y))
    }
    pub fn palette_row_pos(&self, name: &str) -> Option<(u16, u16)> {
        self.palette_rows
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(r, _)| (r.x, r.y))
    }
    pub fn filter_row_pos(&self, i: usize) -> Option<(u16, u16)> {
        self.filter_rows_rects.get(i).map(|(r, _)| (r.x, r.y))
    }
    pub fn props_row_pos(&self, i: usize) -> Option<(u16, u16)> {
        self.props_row_rects.get(i).map(|r| (r.x, r.y))
    }
    pub fn pane_reading_pub(&self) -> Rect { self.pane_reading }

    /// Turn mouse support on (called by run(); off in tests).
    pub fn enable_mouse(&mut self) {
        self.mouse_enabled = true;
    }

    /// Start the vault watcher (§11.2): recursive, events on a channel.
    pub fn start_watcher(&mut self) {
        use notify::{RecursiveMode, Watcher};
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = self.vault.dir.clone();
        match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(mut w) => {
                if w.watch(&dir, RecursiveMode::Recursive).is_ok() {
                    self.watcher = Some(w);
                    self.watch_rx = Some(rx);
                }
            }
            Err(_) => {}
        }
    }

    pub fn read_cursor_pub(&self) -> usize { self.read_cursor }
    pub fn quit_requested(&self) -> bool { self.quit }
    pub fn mode_pub(&self) -> &'static str {
        match self.mode {
            Mode::Normal => "normal",
            Mode::Edit => "edit",
            Mode::Filter => "filter",
            Mode::Picker => "picker",
            Mode::Conflict => "conflict",
            Mode::Props => "props",
            Mode::Help => "help",
        }
    }
    pub fn title_of(&self, r: NRef) -> String { self.vault.tree.node(r).title.clone() }
    pub fn vault_conflict_files(&self) -> std::io::Result<Vec<String>> { self.vault.conflict_files() }
    pub fn vault_mut(&mut self) -> &mut Vault { &mut self.vault }
    pub fn vault_dir(&self) -> std::path::PathBuf {
        self.vault.dir.clone()
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_time = Instant::now();
    }

    /// Drain watcher events; returns true if a debounced reload should run.
    /// Debounce state: events are drained immediately; a reload is due when
    /// at least one relevant event arrived and 200 ms have passed since the
    /// last one (§11.2).
    pub fn poll_watcher(&mut self) -> bool {
        if let Some(rx) = &self.watch_rx {
            while let Ok(res) = rx.try_recv() {
                if let Ok(event) = res {
                    // ignore our own temp files and ignored patterns (§11.4)
                    let relevant = event.paths.iter().any(|p| {
                        let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                        !name.starts_with('.')
                            && !name.ends_with(".fold-tmp")
                            && !name.ends_with(".tmp")
                    });
                    if relevant {
                        self.pending_reload = true;
                        self.last_watch_event = Instant::now();
                    }
                }
            }
        }
        let due = self.pending_reload
            && self.last_watch_event.elapsed() > Duration::from_millis(200)
            && Instant::now() > self.self_write_until;
        if due {
            self.pending_reload = false;
        }
        due
    }

    pub fn poll_watcher_debug(&mut self) -> String {
        let mut n = 0;
        let mut msgs = Vec::new();
        if let Some(rx) = &self.watch_rx {
            while let Ok(res) = rx.try_recv() {
                n += 1;
                msgs.push(format!("{:?}", res.map(|e| e.kind)));
            }
        }
        let poll = self.poll_watcher();
        format!("{} events: {:?} poll={} self_write={:?}", n, msgs, poll, self.self_write_until.elapsed())
    }

    /// Reload after an external change (§11.2): editor saves first, then
    /// re-parse, cursor re-attached by id, path, nearest ancestor. A new
    /// sync-conflict file starts the merge flow (§12).
    pub fn reload_external(&mut self) {
        if self.edit_buf.is_some() {
            self.save_editor("external change");
        }
        let cursor_key = self.current().map(|r| self.vault.key_of(r));
        let zoom_key = self.zoom_root.map(|z| self.vault.key_of(z));
        // sync-conflict files start the merge flow (§11.2)
        match self.vault.conflict_files() {
            Ok(files) if !files.is_empty() => {
                match fold_core::merge::merge_sync_conflicts(&mut self.vault, false) {
                    Ok(outcomes) => {
                        self.say(format!("merged: {}", outcomes.join("; ")));
                        self.mode = Mode::Conflict;
                        self.conflict_idx = 0;
                    }
                    Err(e) => self.say(format!("merge error: {}", e)),
                }
            }
            _ => {
                if let Err(e) = self.vault.reload() {
                    self.say(format!("reload error: {}", e));
                }
            }
        }
        self.zoom_root = zoom_key.and_then(|k| self.vault.find_by_key(&k));
        if let Some(k) = cursor_key {
            if let Some(r) = self.vault.find_by_key(&k) {
                self.move_cursor_to(r);
            }
        }
        self.clamp_cursor();
    }

    /// Visible outline rows: the zoom subtree, flattened, honouring folds
    /// and the hide-done toggle.
    pub fn rows(&self) -> Vec<FlatRow> {
        let mut out = Vec::new();
        let root = self.zoom_root.unwrap_or(self.vault.tree.root);
        self.flatten(root, 0, false, &mut out);
        out
    }

    fn flatten(&self, r: NRef, depth: usize, via_embed: bool, out: &mut Vec<FlatRow>) {
        let n = self.vault.tree.node(r);
        if n.kind == Kind::Root {
            for c in self.vault.tree.resolved_children(r) {
                self.flatten(c, depth, false, out);
            }
            return;
        }
        if self.hide_done && n.task == Some(TaskState::Done) {
            return;
        }
        out.push(FlatRow {
            nref: r,
            depth,
            via_embed,
        });
        let key = self.vault.key_of(r);
        if self.folded.contains(&key) {
            return;
        }
        for c in self.vault.tree.resolved_children(r) {
            let through_embed = self.vault.tree.node(c).is_embed()
                && self.vault.tree.resolved_child(c) != c;
            self.flatten(c, depth + 1, through_embed || via_embed, out);
        }
    }

    pub fn current(&self) -> Option<NRef> {
        let rows = self.rows();
        rows.get(self.cursor.min(rows.len().saturating_sub(1)))
            .map(|r| r.nref)
    }

    fn clamp_cursor(&mut self) {
        let len = self.rows().len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    fn move_cursor_to(&mut self, target: NRef) {
        let rows = self.rows();
        if let Some(i) = rows.iter().position(|r| r.nref == target) {
            self.cursor = i;
        }
    }

    fn is_folded(&self, r: NRef) -> bool {
        self.folded.contains(&self.vault.key_of(r))
    }

    fn toggle_fold(&mut self, r: NRef) {
        let key = self.vault.key_of(r);
        if let Some(i) = self.folded.iter().position(|k| *k == key) {
            self.folded.remove(i);
        } else {
            self.folded.push(key);
        }
    }

    fn refresh_after(&mut self, action: &str) {
        self.clamp_cursor();
        self.self_write_until = Instant::now() + Duration::from_millis(500);
        self.say(action);
    }

    /// Mark a self-write window so the watcher ignores our own saves (§11.2).
    pub fn mark_self_write(&mut self) {
        self.self_write_until = Instant::now() + Duration::from_millis(500);
    }

    // -------------------------------------------------------- actions

    fn act_toggle_task(&mut self) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_toggle_task");
        let key = self.vault.key_of(r);
        match ops::toggle_task(&mut self.vault, r) {
            Ok(()) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
                self.refresh_after("toggled");
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_toggle_taskness(&mut self) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_toggle_taskness");
        let key = self.vault.key_of(r);
        match ops::toggle_taskness(&mut self.vault, r) {
            Ok(()) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
                self.refresh_after("task-ness toggled");
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_make_block(&mut self) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_make_block");
        let path = self.vault.tree.path(r);
        match ops::make_block(&mut self.vault, r) {
            Ok(id) => {
                self.say(format!("block {}", id));
                // cursor lands on the embed, which resolves to the block
                if let Some(nr) = self.vault.find_by_path(&path) {
                    self.move_cursor_to(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_delete(&mut self) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_delete");
        self.register = ops::yank(&self.vault, r);
        match ops::delete_subtree(&mut self.vault, r) {
            Ok(m) => self.refresh_after(&m),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_yank(&mut self) {
        let Some(r) = self.current() else { return };
        self.register = ops::yank(&self.vault, r);
        self.say("yanked");
    }

    fn act_paste(&mut self, after: bool) {
        if self.register.is_empty() {
            self.say("register empty");
            return;
        }
        let Some(r) = self.current() else { return };
        self.push_undo("act_paste");
        let text = self.register.clone();
        match ops::paste(&mut self.vault, r, &text, after) {
            Ok(()) => self.refresh_after("pasted"),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_capture(&mut self, task: bool) {
        self.push_undo("act_capture");
        match ops::capture(&mut self.vault, "", task) {
            Ok(r) => {
                self.move_cursor_to(r);
                self.say("captured (edit the empty title with e)");
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_move(&mut self, down: bool) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_move");
        let key = self.vault.key_of(r);
        match ops::move_sibling(&mut self.vault, r, down) {
            Ok(()) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_spelling(&mut self) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_spelling");
        let key = self.vault.key_of(r);
        match ops::toggle_spelling(&mut self.vault, r) {
            Ok(()) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_demote(&mut self) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_demote");
        let key = self.vault.key_of(r);
        match ops::demote(&mut self.vault, r) {
            Ok(()) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_promote(&mut self) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_promote");
        let key = self.vault.key_of(r);
        match ops::promote(&mut self.vault, r) {
            Ok(()) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_archive(&mut self) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_archive");
        match ops::archive(&mut self.vault, r) {
            Ok(()) => self.refresh_after("archived"),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_clear_done(&mut self) {
        self.push_undo("act_clear_done");
        let target = self.zoom_root.unwrap_or(self.vault.tree.root);
        match ops::clear_done(&mut self.vault, target) {
            Ok(n) => self.refresh_after(&format!("{} done item(s) trashed", n)),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_refile(&mut self, dest_text: &str) {
        let Some(r) = self.current() else { return };
        self.push_undo("act_refile");
        match self.vault.resolve_target(dest_text) {
            Ok(dest) => match ops::refile(&mut self.vault, r, dest) {
                Ok(()) => self.refresh_after("refiled"),
                Err(e) => self.say(format!("error: {}", e)),
            },
            Err(e) => self.say(e),
        }
    }

    fn act_new_node(&mut self, child: bool) {
        let Some(r) = self.current() else { return };
        self.push_undo("new node");
        let res = if child {
            // the new child must be visible to put the cursor on it
            if self.is_folded(r) {
                self.toggle_fold(r);
            }
            match ops::append_child_public(&mut self.vault, r, "") {
                Ok(nr) => {
                    self.move_cursor_to(nr);
                    self.act_edit();
                    return;
                }
                Err(e) => Err(e),
            }
        } else {
            let n = self.vault.tree.node(r);
            let line = match n.kind {
                Kind::Section => format!("{} ", "#".repeat(self.vault.tree.level(r))),
                _ => "- ".to_string(),
            };
            let key = self.vault.key_of(r);
            let depth = self.rows().get(self.cursor).map(|row| row.depth).unwrap_or(0);
            match ops::paste(&mut self.vault, r, &format!("{}\n", line), true) {
                Ok(()) => {
                    // move to the new sibling: the first row after the
                    // cursor node's subtree, at its depth, with an empty title
                    if let Some(nr) = self.vault.find_by_key(&key) {
                        self.move_cursor_to(nr);
                    }
                    let rows = self.rows();
                    if let Some(i) = rows.iter().enumerate().skip(self.cursor + 1).position(
                        |(_, row)| {
                            row.depth <= depth
                                && self.vault.tree.node(row.nref).title.is_empty()
                        },
                    ) {
                        self.cursor += 1 + i;
                    }
                    self.act_edit();
                    return;
                }
                Err(e) => Err(e),
            }
        };
        match res {
            Ok(()) => self.refresh_after("node created"),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    // -------------------------------------------------------- editor (§10.6)

    pub fn act_edit(&mut self) {
        let Some(r) = self.current() else { return };
        let target = if self.vault.tree.node(r).is_embed() {
            self.vault.tree.resolved_child(r)
        } else {
            r
        };
        self.edit_buf = Some(fold_core::edit::open_editor(&self.vault, target));
        self.edit_cursor = (0, 0);
        self.edit_saved_dot = false;
        self.mode = Mode::Edit;
        self.focus = Focus::Reading;
    }

    fn save_editor(&mut self, why: &str) {
        let Some(mut buf) = self.edit_buf.take() else { return };
        let snap = self.snapshot("edit");
        match buf.save_all(&mut self.vault) {
            Ok(n) => {
                if n > 0 {
                    self.record_undo(snap);
                    self.say(format!("saved {} block(s) ({})", n, why));
                }
                self.edit_saved_dot = false;
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
        self.edit_buf = Some(buf);
        self.edit_last_key = Instant::now();
    }

    fn close_editor(&mut self) {
        self.save_editor("exit");
        self.edit_buf = None;
        self.mode = Mode::Normal;
    }

    pub fn key_edit(&mut self, key: KeyEvent) {
        let Some(buf) = self.edit_buf.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let (line, col) = self.edit_cursor;
        let nlines = buf.lines.len();
        match key.code {
            KeyCode::Esc => {
                self.close_editor();
                return;
            }
            KeyCode::Up | KeyCode::Down => {
                let target = if key.code == KeyCode::Up {
                    line.checked_sub(1)
                } else {
                    Some(line + 1).filter(|&l| l < nlines)
                };
                if let Some(t) = target {
                    let len = buf.lines[t].text.chars().count();
                    self.edit_cursor = (t, col.min(len));
                }
            }
            KeyCode::Left => {
                self.edit_cursor.1 = col.saturating_sub(1);
            }
            KeyCode::Right => {
                let len = buf.lines.get(line).map(|l| l.text.chars().count()).unwrap_or(0);
                if col < len {
                    self.edit_cursor.1 = col + 1;
                }
            }
            KeyCode::Enter => {
                // split the line at the cursor
                let cur = buf.lines.get(line).map(|l| l.text.clone()).unwrap_or_default();
                let byte_col = cur
                    .char_indices()
                    .nth(col)
                    .map(|(i, _)| i)
                    .unwrap_or(cur.len());
                let (a, b) = cur.split_at(byte_col);
                buf.set_line(line, a.to_string());
                buf.insert_line(line, b.to_string());
                self.edit_cursor = (line + 1, 0);
            }
            KeyCode::Backspace => {
                if col > 0 {
                    if let Some(l) = buf.lines.get(line) {
                        let byte_col = l
                            .text
                            .char_indices()
                            .nth(col)
                            .map(|(i, _)| i)
                            .unwrap_or(l.text.len());
                        let prev = l.text[..byte_col]
                            .char_indices()
                            .last()
                            .map(|(i, c)| (i, c.len_utf8()))
                            .unwrap_or((0, 0));
                        let mut t = l.text.clone();
                        t.replace_range(prev.0..byte_col, "");
                        buf.set_line(line, t);
                        self.edit_cursor.1 = col - 1;
                    }
                } else if line > 0 {
                    // join with the previous line
                    let cur = buf.lines.get(line).map(|l| l.text.clone()).unwrap_or_default();
                    let prev_len = buf
                        .lines
                        .get(line - 1)
                        .map(|l| l.text.chars().count())
                        .unwrap_or(0);
                    let prev = buf.lines.get(line - 1).map(|l| l.text.clone()).unwrap_or_default();
                    buf.set_line(line - 1, format!("{}{}", prev, cur));
                    buf.delete_line(line);
                    self.edit_cursor = (line - 1, prev_len);
                }
            }
            KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            KeyCode::Char(c) => {
                if let Some(l) = buf.lines.get(line) {
                    let byte_col = l
                        .text
                        .char_indices()
                        .nth(col)
                        .map(|(i, _)| i)
                        .unwrap_or(l.text.len());
                    let mut t = l.text.clone();
                    t.insert(byte_col, c);
                    buf.set_line(line, t);
                    self.edit_cursor.1 = col + 1;
                    self.edit_saved_dot = true;
                }
            }
            _ => {}
        }
        self.edit_last_key = Instant::now();
        // moving out of a dirty block saves it (§10.6)
        let new_owner = self
            .edit_buf
            .as_ref()
            .map(|b| b.owner_at(self.edit_cursor.0));
        let old_owner = self.edit_buf.as_ref().map(|b| b.owner_at(line));
        if new_owner != old_owner {
            if let (Some(old), Some(buf2)) = (old_owner, self.edit_buf.as_mut()) {
                if buf2.dirty.contains(&old) {
                    let snap = self.snapshot("edit");
                    let mut tmp = self.edit_buf.take().unwrap();
                    if tmp.splice(&mut self.vault, old).is_ok() {
                        self.record_undo(snap);
                    }
                    self.edit_buf = Some(tmp);
                }
            }
        }
        self.edit_saved_dot = self
            .edit_buf
            .as_ref()
            .map(|b| !b.dirty.is_empty())
            .unwrap_or(false);
    }

    // -------------------------------------------------------- props (§10.6)

    fn act_props(&mut self) {
        let Some(r) = self.current() else { return };
        let target = if self.vault.tree.node(r).is_embed() {
            self.vault.tree.resolved_child(r)
        } else {
            r
        };
        self.props_target = Some(target);
        self.props_rows = if self.vault.tree.node(target).is_block() {
            ops::frontmatter_lines(&self.vault, target.0)
                .into_iter()
                .filter(|(k, _, _)| k != "id")
                .collect()
        } else {
            Vec::new()
        };
        self.props_sel = 0;
        self.props_editing = None;
        self.mode = Mode::Props;
    }

    pub fn key_props(&mut self, key: KeyEvent) {
        if self.props_editing.is_some() {
            // handled by the prompt path
            self.mode = Mode::Normal;
            self.props_editing = None;
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.props_sel + 1 < self.props_rows.len() {
                    self.props_sel += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.props_sel = self.props_sel.saturating_sub(1);
            }
            KeyCode::Char('n') => {
                self.prompt = Some(Prompt {
                    label: "new key".into(),
                    text: String::new(),
                    action: PromptAction::PropNew,
                });
                self.mode = Mode::Normal;
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some((k, _, editable)) = self.props_rows.get(self.props_sel).cloned() {
                    if editable {
                        self.prompt = Some(Prompt {
                            label: format!("{}", k),
                            text: String::new(),
                            action: PromptAction::PropSet(k),
                        });
                        self.mode = Mode::Normal;
                    } else {
                        self.say("read-only line (preserved verbatim)");
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some((k, _, editable)) = self.props_rows.get(self.props_sel).cloned() {
                    if editable {
                        if let Some(t) = self.props_target {
                            self.push_undo("delete property");
                            let _ = ops::set_frontmatter_key(&mut self.vault, t.0, &k, None);
                            self.props_rows.remove(self.props_sel);
                            self.props_sel = self.props_sel.saturating_sub(1);
                            self.say(format!("{} removed", k));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn key_prompt(&mut self, key: KeyEvent) {
        let Some(mut p) = self.prompt.take() else { return };
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                match p.action {
                    PromptAction::Refile => self.act_refile(&p.text),
                    PromptAction::GoTo => match self.vault.resolve_target(&p.text) {
                        Ok(r) => self.move_cursor_to(r),
                        Err(e) => self.say(e),
                    },
                    PromptAction::CaptureText(task) => {
                        self.push_undo("capture");
                        match ops::capture(&mut self.vault, &p.text, task) {
                            Ok(r) => {
                                self.move_cursor_to(r);
                                self.say("captured");
                            }
                            Err(e) => self.say(format!("error: {}", e)),
                        }
                    }
                    PromptAction::PropSet(k) => {
                        if let Some(t) = self.props_target {
                            // validate dates (§8.4)
                            if (k == "due" || k == "done")
                                && !fold_core::check::is_iso_date(&p.text)
                            {
                                self.say(format!("{}: must be YYYY-MM-DD", k));
                                return;
                            }
                            self.push_undo("set property");
                            match ops::set_property(&mut self.vault, t, &k, &p.text) {
                                Ok(()) => self.say(format!("{} set", k)),
                                Err(e) => self.say(format!("error: {}", e)),
                            }
                        }
                    }
                    PromptAction::PropNew => {
                        if self.props_target.is_some() {
                            let key_name = p.text.trim().to_string();
                            if fold_core::parse::is_valid_key(&key_name) {
                                self.prompt = Some(Prompt {
                                    label: format!("{}", key_name),
                                    text: String::new(),
                                    action: PromptAction::PropSet(key_name),
                                });
                                return;
                            }
                            self.say("invalid key");
                        }
                    }
                    PromptAction::ReadSearch => {
                        self.read_search = p.text.clone();
                        let doc = self.reading_doc();
                        self.update_read_matches(&doc);
                    }
                }
            }
            KeyCode::Backspace => {
                p.text.pop();
                self.prompt = Some(p);
            }
            KeyCode::Char(c) => {
                p.text.push(c);
                self.prompt = Some(p);
            }
            _ => {}
        }
    }

    // -------------------------------------------------------- input

    pub fn enter_conflict_view(&mut self) {
        if fold_core::merge::conflict_pairs(&self.vault).is_empty() {
            self.say("no conflicts");
            return;
        }
        self.mode = Mode::Conflict;
        self.conflict_idx = 0;
    }

    pub fn key_conflict_pub(&mut self, key: KeyEvent) { self.key_conflict(key) }
    fn key_conflict(&mut self, key: KeyEvent) {
        let pairs = fold_core::merge::conflict_pairs(&self.vault);
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.say(format!("{} conflict pair(s) left", pairs.len()));
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                self.say(format!("{} conflict pair(s) left", pairs.len()));
            }
            KeyCode::Char('n') => {
                if self.conflict_idx + 1 < pairs.len() {
                    self.conflict_idx += 1;
                }
            }
            KeyCode::Char('N') => {
                self.conflict_idx = self.conflict_idx.saturating_sub(1);
            }
            KeyCode::Char('o') => {
                if let Some(&(_, theirs)) = pairs.get(self.conflict_idx) {
                    self.push_undo("keep ours");
                    match fold_core::merge::resolve_keep_ours(&mut self.vault, theirs) {
                        Ok(()) => self.say("kept ours"),
                        Err(e) => self.say(format!("error: {}", e)),
                    }
                    self.conflict_idx = self.conflict_idx.saturating_sub(0).min(
                        fold_core::merge::conflict_pairs(&self.vault)
                            .len()
                            .saturating_sub(1),
                    );
                }
            }
            KeyCode::Char('t') => {
                if let Some(&(ours, theirs)) = pairs.get(self.conflict_idx) {
                    self.push_undo("keep theirs");
                    match fold_core::merge::resolve_keep_theirs(&mut self.vault, ours, theirs) {
                        Ok(()) => self.say("kept theirs"),
                        Err(e) => self.say(format!("error: {}", e)),
                    }
                }
            }
            KeyCode::Char('b') => {
                if let Some(&(_, theirs)) = pairs.get(self.conflict_idx) {
                    self.push_undo("keep both");
                    match fold_core::merge::resolve_keep_both(&mut self.vault, theirs) {
                        Ok(()) => self.say("kept both"),
                        Err(e) => self.say(format!("error: {}", e)),
                    }
                }
            }
            KeyCode::Char('e') => {
                if let Some(&(ours, _)) = pairs.get(self.conflict_idx) {
                    self.edit_buf = Some(fold_core::edit::open_editor(&self.vault, ours));
                    self.edit_cursor = (0, 0);
                    self.edit_saved_dot = false;
                    self.mode = Mode::Edit;
                }
            }
            _ => {}
        }
    }

    fn draw_conflict(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let pairs = fold_core::merge::conflict_pairs(&self.vault);
        let block = WBlock::default()
            .borders(Borders::ALL)
            .title(format!(
                " conflicts — pair {}/{} · o ours · t theirs · b both · e edit · n/N · Enter done ",
                (self.conflict_idx + 1).min(pairs.len()),
                pairs.len()
            ))
            .border_style(Style::default().fg(Color::Yellow));
        if pairs.is_empty() {
            f.render_widget(
                Paragraph::new("no unresolved conflicts — Enter to close").block(block),
                area,
            );
            return;
        }
        let (ours, theirs) = pairs[self.conflict_idx.min(pairs.len() - 1)];
        let halves = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let ours_text = render(&self.vault.tree, ours, 1, true);
        let theirs_text = render(&self.vault.tree, theirs, 1, true);
        let ours_title = format!(" ours: {} ", self.vault.tree.node(ours).title);
        let theirs_title = format!(
            " theirs ({}) ",
            self.vault.tree.node(theirs)
                .block
                .as_ref()
                .and_then(|b| b.prop("conflict").map(|s| s.to_string()))
                .unwrap_or_default()
        );
        f.render_widget(
            Paragraph::new(ours_text).block(
                WBlock::default().borders(Borders::ALL).title(ours_title),
            ),
            halves[0],
        );
        f.render_widget(
            Paragraph::new(theirs_text).block(
                WBlock::default()
                    .borders(Borders::ALL)
                    .title(theirs_title)
                    .border_style(Style::default().fg(Color::Red)),
            ),
            halves[1],
        );
        let _ = block;
    }

    // -------------------------------------------------------- mouse

    /// Handle a mouse event. Layout: outline clicks select/fold/zoom, wheel
    /// scrolls the pane under the pointer, reading clicks move its cursor,
    /// editor clicks place the text cursor. Double-click zooms/follows.
    pub fn handle_mouse(&mut self, m: MouseEvent) {
        if !self.mouse_enabled {
            return;
        }
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::ScrollDown => self.mouse_scroll(x, y, 3),
            MouseEventKind::ScrollUp => self.mouse_scroll(x, y, -3),
            MouseEventKind::Down(MouseButton::Left) => self.mouse_click(x, y),
            _ => {}
        }
    }

    fn mouse_scroll(&mut self, x: u16, y: u16, delta: i32) {
        if self.point_in(self.pane_outline, x, y) {
            let len = self.rows().len() as i32;
            let mut c = self.cursor as i32 + delta;
            c = c.clamp(0, (len - 1).max(0));
            self.cursor = c as usize;
            self.focus = Focus::Outline;
        } else if self.point_in(self.pane_reading, x, y) {
            self.sync_read_target();
            let doc = self.reading_doc();
            let max = doc.lines.len().saturating_sub(1) as i32;
            let mut c = self.read_cursor as i32 + delta;
            c = c.clamp(0, max.max(0));
            self.read_cursor = c as usize;
            if self.mode != Mode::Edit {
                self.focus = Focus::Reading;
            } else {
                // scroll the editor buffer instead
                let el = self.edit_cursor.0 as i32 + delta;
                self.edit_cursor.0 = el.clamp(
                    0,
                    self.edit_buf
                        .as_ref()
                        .map(|b| b.lines.len().saturating_sub(1))
                        .unwrap_or(0) as i32,
                ) as usize;
            }
        }
    }

    fn mouse_click(&mut self, x: u16, y: u16) {
        let double = self
            .last_click
            .map(|(lx, ly, t)| lx == x && ly == y && t.elapsed() < Duration::from_millis(500))
            .unwrap_or(false);
        self.last_click = Some((x, y, Instant::now()));

        // the action bar wins over everything
        if self.mouse_enabled && self.toolbar_click(x, y) {
            return;
        }
        // the conflict view has no panes; only its toolbar is clickable
        if self.mode == Mode::Conflict {
            return;
        }
        // on-screen keyboard (edit, prompts, filter, palette)
        if self.kb_click(x, y) {
            return;
        }
        // Help closes on any click outside the toolbar
        if self.mode == Mode::Help {
            self.mode = Mode::Normal;
            return;
        }
        // palette rows are clickable
        if self.mode == Mode::Picker {
            if let Some((_, name)) = self
                .palette_rows
                .iter()
                .find(|(r, _)| self.point_in(*r, x, y))
            {
                let name = *name;
                self.mode = Mode::Normal;
                self.palette.clear();
                self.run_palette(name);
                return;
            }
        }
        // filter results are clickable
        if self.mode == Mode::Filter {
            if let Some((_, r)) = self
                .filter_rows_rects
                .iter()
                .find(|(r, _)| self.point_in(*r, x, y))
            {
                let r = *r;
                let path = self.vault.tree.ancestors(r);
                for a in &path {
                    let k = self.vault.key_of(*a);
                    self.folded.retain(|f| f != &k);
                }
                self.zoom_root = None;
                self.move_cursor_to(r);
                self.mode = Mode::Normal;
                self.filter.clear();
                self.filter_rows.clear();
                return;
            }
        }
        // props rows are clickable (select, then toolbar Edit/Del acts)
        if self.mode == Mode::Props {
            if let Some(i) = self
                .props_row_rects
                .iter()
                .position(|r| self.point_in(*r, x, y))
            {
                self.props_sel = i;
                return;
            }
        }
        // prompt: Enter on click outside the keyboard acts as accept
        if self.prompt.is_some() {
            self.text_input(KeyCode::Enter);
            return;
        }
        if self.mode == Mode::Edit && self.point_in(self.pane_reading, x, y) {
            // place the text cursor
            let inner_top = self.pane_reading.y + 1;
            let inner_left = self.pane_reading.x + 1;
            if y >= inner_top && x >= inner_left {
                let inner_height = self.pane_reading.height.saturating_sub(2) as usize;
                let scroll = if self.edit_cursor.0 >= inner_height {
                    self.edit_cursor.0 + 1 - inner_height
                } else {
                    0
                };
                let line = scroll + (y - inner_top) as usize;
                if let Some(buf) = self.edit_buf.as_mut() {
                    if line < buf.lines.len() {
                        self.edit_cursor.0 = line;
                        let col_target = (x - inner_left) as usize;
                        let len = buf.lines[line].text.chars().count();
                        self.edit_cursor.1 = col_target.min(len);
                        self.edit_last_key = Instant::now();
                    }
                }
            }
            return;
        }
        if self.point_in(self.pane_outline, x, y) {
            self.focus = Focus::Outline;
            let inner_top = self.pane_outline.y + 1;
            if y < inner_top {
                return;
            }
            let idx = self.outline_scroll + (y - inner_top) as usize;
            let rows = self.rows();
            let Some(row) = rows.get(idx) else { return };
            self.cursor = idx;
            // click on the fold marker toggles the fold
            let inner_left = self.pane_outline.x + 1;
            let marker_x = inner_left + (row.depth * 2) as u16;
            if x >= marker_x && x <= marker_x + 1 && !self.vault.tree.resolved_children(row.nref).is_empty()
            {
                self.toggle_fold(row.nref);
                return;
            }
            if double {
                // double-click: zoom into the node (like Enter)
                if self.vault.tree.node(row.nref).kind != Kind::Root {
                    self.zoom_root = Some(row.nref);
                    self.cursor = 0;
                    self.read_cursor = 0;
                    self.scroll_reading = 0;
                    self.focus = Focus::Reading;
                }
            }
        } else if self.point_in(self.pane_reading, x, y) {
            self.focus = Focus::Reading;
            let inner_top = self.pane_reading.y + 1;
            if y < inner_top {
                return;
            }
            self.sync_read_target();
            let doc = self.reading_doc();
            let mut line = self.scroll_reading + (y - inner_top) as usize;
            if self.read_header && line >= 1 {
                // the property header is drawn as line 1 but is not a doc line
                if line == 1 {
                    return;
                }
                line -= 1;
            }
            if line < doc.lines.len() {
                self.read_cursor = line;
                if double {
                    use fold_core::reading::LineRef;
                    match fold_core::reading::node_at(&doc, line) {
                        Some(LineRef::Title(r)) => {
                            let n = self.vault.tree.node(r);
                            if n.task.is_some() {
                                self.act_on_node(r, |app, r| {
                                    let _ = ops::toggle_task(&mut app.vault, r);
                                });
                            } else if n.kind == Kind::Section {
                                self.zoom_root = Some(r);
                                self.read_cursor = 0;
                                self.scroll_reading = 0;
                            }
                        }
                        Some(LineRef::Embed(e)) => {
                            let t = self.vault.tree.resolved_child(e);
                            if t != e {
                                self.zoom_root = Some(t);
                                self.read_cursor = 0;
                                self.scroll_reading = 0;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Handle a toolbar button click. Returns true if a button was hit.
    fn toolbar_click(&mut self, x: u16, y: u16) -> bool {
        let hit = self
            .toolbar
            .iter()
            .find(|(r, _)| self.point_in(*r, x, y))
            .map(|(_, a)| *a);
        let Some(action) = hit else { return false };
        // labels shared between modes act on the mode's own target
        let mode_key = match (self.mode, action) {
            (Mode::Conflict, "Done") => Some(KeyCode::Enter),
            (Mode::Conflict, "Edit") | (Mode::Props, "Edit") => Some(KeyCode::Char('e')),
            (Mode::Props, "New") => Some(KeyCode::Char('n')),
            _ => None,
        };
        if let Some(code) = mode_key {
            self.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
            return true;
        }
        match action {
            "New" => self.act_new_node(false),
            "Edit" => self.act_edit(),
            "Props" => self.act_props(),
            "Done" => self.act_toggle_task(),
            "Task" => self.act_toggle_taskness(),
            "Block" => self.act_make_block(),
            "Yank" => self.act_yank(),
            "Trash" => self.act_delete(),
            "Paste" => self.act_paste(true),
            "Capture" => {
                self.prompt = Some(Prompt {
                    label: "capture".into(),
                    text: String::new(),
                    action: PromptAction::CaptureText(false),
                });
            }
            "Archive" => self.act_archive(),
            "Refile" => {
                self.prompt = Some(Prompt {
                    label: "refile to".into(),
                    text: String::new(),
                    action: PromptAction::Refile,
                });
            }
            "Undo" => self.act_undo(),
            "Redo" => self.act_redo(),
            "Filter" => {
                self.mode = Mode::Filter;
                self.filter.clear();
            }
            "Menu" => {
                self.mode = Mode::Picker;
                self.palette.clear();
            }
            "Help" => self.mode = Mode::Help,
            "Quit" => self.quit = true,
            "Save" => self.save_editor("toolbar"),
            "Close" => match self.mode {
                Mode::Edit => self.close_editor(),
                Mode::Props | Mode::Filter | Mode::Picker | Mode::Help => {
                    self.mode = Mode::Normal
                }
                _ => {}
            },
            "Discard" => self.discard_editor(),
            "Ours" => self.key_conflict(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
            "Theirs" => self.key_conflict(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
            "Both" => self.key_conflict(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE)),
            "Prev" => self.key_conflict(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE)),
            "Next" => self.key_conflict(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
            "Del" => self.key_props(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)),
            _ => {}
        }
        true
    }

    /// Handle a click on the on-screen keyboard. Routes to whichever input
    /// is active: the editor, a prompt, the filter box or the palette.
    fn kb_click(&mut self, x: u16, y: u16) -> bool {
        if let Some((_, c)) = self.kb_rects.iter().find(|(r, _)| self.point_in(*r, x, y)) {
            let c = *c;
            self.text_input(KeyCode::Char(c));
            return true;
        }
        if let Some((_, s)) = self.kb_special.iter().find(|(r, _)| self.point_in(*r, x, y)) {
            let s = *s;
            match s {
                "Space" => self.text_input(KeyCode::Char(' ')),
                "Enter" => self.text_input(KeyCode::Enter),
                "Bksp" => self.text_input(KeyCode::Backspace),
                "↑" => self.text_input(KeyCode::Up),
                "↓" => self.text_input(KeyCode::Down),
                "←" => self.text_input(KeyCode::Left),
                "→" => self.text_input(KeyCode::Right),
                _ => {}
            }
            return true;
        }
        false
    }

    /// Route a synthesized key to the active text input.
    fn text_input(&mut self, code: KeyCode) {
        let key = KeyEvent::new(code, KeyModifiers::NONE);
        if self.prompt.is_some() {
            self.key_prompt(key);
        } else {
            match self.mode {
                Mode::Edit => self.key_edit(key),
                Mode::Filter => self.key_filter(key),
                Mode::Picker => self.key_palette(key),
                _ => {}
            }
        }
    }

    fn point_in(&self, r: Rect, x: u16, y: u16) -> bool {
        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
    }

    /// Dispatch a key press by mode, including the two-key sequences
    /// (`zd`/`zr`/`za`, `gg`, `[[`/`]]`).
    pub fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') && self.mode != Mode::Edit {
            self.quit = true;
            return;
        }
        if let Some(p) = self.pending.take() {
            match (p, key.code) {
                ('z', KeyCode::Char('d')) => {
                    self.hide_done = !self.hide_done;
                    self.clamp_cursor();
                    self.say(if self.hide_done { "done hidden" } else { "done shown" });
                }
                ('z', KeyCode::Char('r')) => {
                    self.raw_mode = !self.raw_mode;
                    self.say(if self.raw_mode { "raw" } else { "styled" });
                }
                ('z', KeyCode::Char('a')) => self.act_archive(),
                ('g', KeyCode::Char('g')) => match self.focus {
                    Focus::Outline => self.cursor = 0,
                    Focus::Reading => self.read_cursor = 0,
                },
                (']', KeyCode::Char(']')) | ('[', KeyCode::Char('[')) => {
                    self.sync_read_target();
                    let doc = self.reading_doc();
                    self.jump_heading(&doc, if p == ']' { 1 } else { -1 });
                }
                _ => {}
            }
            return;
        }
        match self.mode {
            Mode::Normal => {
                if self.prompt.is_some() {
                    self.key_prompt(key);
                    return;
                }
                let starts_seq = match key.code {
                    KeyCode::Char('z') => self.focus == Focus::Outline,
                    KeyCode::Char('g') => true,
                    KeyCode::Char('[') | KeyCode::Char(']') => self.focus == Focus::Reading,
                    _ => false,
                };
                if starts_seq && !ctrl {
                    if let KeyCode::Char(c) = key.code {
                        self.pending = Some(c);
                    }
                    return;
                }
                self.key_normal(key)
            }
            Mode::Filter => self.key_filter(key),
            Mode::Picker => self.key_palette(key),
            Mode::Edit => {
                if ctrl && key.code == KeyCode::Char('c') {
                    self.discard_editor();
                    return;
                }
                self.key_edit(key)
            }
            Mode::Props => self.key_props(key),
            Mode::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')
                ) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Conflict => self.key_conflict(key),
        }
    }

    /// Drop the editor's unsaved changes (§10.6 `Ctrl-c`).
    fn discard_editor(&mut self) {
        self.edit_buf = None;
        self.mode = Mode::Normal;
        let _ = self.vault.reload();
        self.clamp_cursor();
        self.say("changes discarded");
    }

    pub fn key_normal(&mut self, key: KeyEvent) {
        let rows = self.rows();
        match self.focus {
            Focus::Outline => self.key_outline(key, rows),
            Focus::Reading => self.key_reading(key),
        }
    }

    pub fn key_outline(&mut self, key: KeyEvent, rows: Vec<FlatRow>) {
        let len = rows.len();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half = (self.pane_outline.height.saturating_sub(2) as usize / 2).max(1);
        match key.code {
            KeyCode::Char('d') if ctrl => {
                self.cursor = (self.cursor + half).min(len.saturating_sub(1));
            }
            KeyCode::Char('u') if ctrl => {
                self.cursor = self.cursor.saturating_sub(half);
            }
            _ if ctrl => {}
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Tab => self.focus = Focus::Reading,
            KeyCode::Char('j') | KeyCode::Down => {
                if self.cursor + 1 < len {
                    self.cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(r) = self.current() {
                    let has_kids = !self.vault.tree.resolved_children(r).is_empty();
                    if has_kids && !self.is_folded(r) {
                        self.toggle_fold(r);
                    } else {
                        self.goto_parent_row(&rows);
                    }
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(r) = self.current() {
                    if self.is_folded(r) {
                        self.toggle_fold(r);
                    } else {
                        let kids = self.vault.tree.resolved_children(r);
                        if let Some(&first) = kids.first() {
                            self.move_cursor_to(first);
                        }
                    }
                }
            }
            KeyCode::Char('H') => {
                if let Some(r) = self.current() {
                    // fold everything under cursor
                    let mut keys = Vec::new();
                    self.vault.tree.walk(r, &mut |t, n| {
                        if !t.resolved_children(n).is_empty() {
                            keys.push(self.vault.key_of(n));
                        }
                    });
                    for k in keys {
                        if !self.folded.contains(&k) {
                            self.folded.push(k);
                        }
                    }
                }
            }
            KeyCode::Char('L') => {
                if let Some(r) = self.current() {
                    let mut keys = Vec::new();
                    self.vault.tree.walk(r, &mut |t, n| {
                        keys.push(self.vault.key_of(n));
                        let _ = t;
                    });
                    self.folded.retain(|k| !keys.contains(k));
                }
            }
            KeyCode::Char('-') => self.goto_parent_row(&rows),
            KeyCode::Char('{') => self.sibling(&rows, -1),
            KeyCode::Char('}') => self.sibling(&rows, 1),
            KeyCode::Char('G') => {
                if len > 0 {
                    self.cursor = len - 1;
                }
            }
            KeyCode::Enter => {
                if let Some(r) = self.current() {
                    if self.vault.tree.node(r).kind != Kind::Root {
                        self.zoom_root = Some(r);
                        self.cursor = 0;
                        self.focus = Focus::Reading;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(z) = self.zoom_root {
                    if let Some(p) = self.vault.tree.node(z).parent {
                        let pr = (z.0, p);
                        if self.vault.tree.node(pr).kind != Kind::Root {
                            self.zoom_root = Some(pr);
                        } else {
                            self.zoom_root = None;
                        }
                        self.cursor = 0;
                    } else {
                        self.zoom_root = None;
                        self.cursor = 0;
                    }
                }
            }
            KeyCode::Char('>') => self.act_demote(),
            KeyCode::Char('<') => self.act_promote(),
            KeyCode::Char('~') => self.act_spelling(),
            KeyCode::Char('J') => self.act_move(true),
            KeyCode::Char('K') => self.act_move(false),
            KeyCode::Char('n') => self.act_new_node(false),
            KeyCode::Char('N') => self.act_new_node(true),
            KeyCode::Char('x') => self.act_toggle_task(),
            KeyCode::Char('t') => self.act_toggle_taskness(),
            KeyCode::Char('s') => self.act_make_block(),
            KeyCode::Char('y') => self.act_yank(),
            KeyCode::Char('d') => self.act_delete(),
            KeyCode::Char('p') => self.act_paste(true),
            KeyCode::Char('P') => self.act_paste(false),
            KeyCode::Char('c') => {
                self.prompt = Some(Prompt {
                    label: "capture".into(),
                    text: String::new(),
                    action: PromptAction::CaptureText(false),
                });
            }
            KeyCode::Char('C') => {
                self.prompt = Some(Prompt {
                    label: "capture task".into(),
                    text: String::new(),
                    action: PromptAction::CaptureText(true),
                });
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                self.filter.clear();
                self.filter_rows.clear();
            }
            KeyCode::Char(':') => {
                self.mode = Mode::Picker;
                self.palette.clear();
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
            }
            KeyCode::Char('u') => self.act_undo(),
            KeyCode::Char('U') => self.act_redo(),
            KeyCode::Char('e') => self.act_edit(),
            KeyCode::Char('a') => self.act_props(),
            KeyCode::Char('r') => {
                self.prompt = Some(Prompt {
                    label: "refile to".into(),
                    text: String::new(),
                    action: PromptAction::Refile,
                });
            }
            _ => {}
        }
    }

    fn act_undo(&mut self) {
        if let Some(inv) = self.undo.pop() {
            let redo_snapshot = self.snapshot("redo point");
            match self.restore(inv) {
                Ok(()) => {
                    self.redo.push(redo_snapshot);
                    self.clamp_cursor();
                    self.say("undone");
                }
                Err(e) => self.say(format!("undo failed: {}", e)),
            }
        } else {
            self.say("nothing to undo");
        }
    }

    fn act_redo(&mut self) {
        if let Some(inv) = self.redo.pop() {
            let undo_snapshot = self.snapshot("undo point");
            match self.restore(inv) {
                Ok(()) => {
                    self.undo.push(undo_snapshot);
                    self.clamp_cursor();
                    self.say("redone");
                }
                Err(e) => self.say(format!("redo failed: {}", e)),
            }
        } else {
            self.say("nothing to redo");
        }
    }

    fn push_undo(&mut self, desc: &str) {
        let snap = self.snapshot(desc);
        self.record_undo(snap);
    }

    /// Record a snapshot taken before a mutation as one op-log entry.
    fn record_undo(&mut self, snap: ops::Inverse) {
        self.undo.push(snap);
        self.redo.clear();
        self.mark_self_write();
    }

    /// Bring the vault back to a snapshot: rewrite its files and remove
    /// files created since it was taken (new blocks), so undo of a
    /// file-creating op leaves no orphan (§10.11).
    fn restore(&mut self, mut inv: ops::Inverse) -> std::io::Result<()> {
        for f in &self.vault.tree.files {
            if !inv.files.iter().any(|(p, _)| *p == f.path) {
                inv.files.push((f.path.clone(), None));
            }
        }
        self.mark_self_write();
        inv.apply(&mut self.vault)
    }

    fn snapshot(&self, desc: &str) -> ops::Inverse {
        ops::Inverse {
            files: self
                .vault
                .tree
                .files
                .iter()
                .map(|f| (f.path.clone(), Some(f.text.clone())))
                .collect(),
            description: desc.into(),
        }
    }

    /// Parent and siblings are found on the flattened rows rather than in
    /// the tree: a block row's tree parent is its own file's root, but its
    /// outline parent is the row that embeds it.
    fn goto_parent_row(&mut self, rows: &[FlatRow]) {
        let Some(cur) = rows.get(self.cursor) else { return };
        if let Some(i) = rows[..self.cursor].iter().rposition(|r| r.depth < cur.depth) {
            self.cursor = i;
        }
    }

    fn sibling(&mut self, rows: &[FlatRow], dir: i32) {
        let Some(cur) = rows.get(self.cursor) else { return };
        let d = cur.depth;
        let mut i = self.cursor as i64 + dir as i64;
        while i >= 0 && (i as usize) < rows.len() {
            let rd = rows[i as usize].depth;
            if rd < d {
                return;
            }
            if rd == d {
                self.cursor = i as usize;
                return;
            }
            i += dir as i64;
        }
    }

    pub fn key_reading_pub(&mut self, key: KeyEvent) { self.key_reading(key) }
    fn key_reading(&mut self, key: KeyEvent) {
        self.sync_read_target();
        let doc = self.reading_doc();
        let nlines = doc.lines.len();
        let half = (self.pane_reading.height.saturating_sub(2) as usize / 2).max(1);
        match key.code {
            KeyCode::Tab => self.focus = Focus::Outline,
            KeyCode::Char('j') | KeyCode::Down => {
                if self.read_cursor + 1 < nlines.max(1) {
                    self.read_cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.read_cursor = self.read_cursor.saturating_sub(1);
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.read_cursor = (self.read_cursor + half).min(nlines.saturating_sub(1));
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.read_cursor = self.read_cursor.saturating_sub(half);
            }
            KeyCode::Char('G') => {
                self.read_cursor = nlines.saturating_sub(1);
            }
            KeyCode::Enter => {
                use fold_core::reading::LineRef;
                match fold_core::reading::node_at(&doc, self.read_cursor) {
                    Some(LineRef::Title(r)) => {
                        let n = self.vault.tree.node(r);
                        if n.task.is_some() {
                            self.act_on_node(r, |app, r| {
                                let _ = ops::toggle_task(&mut app.vault, r);
                            });
                        } else if n.kind == Kind::Section {
                            self.zoom_root = Some(r);
                            self.read_cursor = 0;
                            self.scroll_reading = 0;
                        }
                    }
                    Some(LineRef::Embed(e)) => {
                        let t = self.vault.tree.resolved_child(e);
                        if t != e {
                            self.zoom_root = Some(t);
                            self.read_cursor = 0;
                            self.scroll_reading = 0;
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                if let Some(z) = self.zoom_root {
                    if let Some(p) = self.vault.tree.node(z).parent {
                        let pr = (z.0, p);
                        self.zoom_root = if self.vault.tree.node(pr).kind != Kind::Root {
                            Some(pr)
                        } else {
                            None
                        };
                    } else {
                        self.zoom_root = None;
                    }
                    self.read_cursor = 0;
                    self.scroll_reading = 0;
                } else {
                    self.focus = Focus::Outline;
                }
            }
            KeyCode::Char('x') => {
                if let Some(r) = self.read_node() {
                    self.act_on_node(r, |app, r| {
                        let _ = ops::toggle_task(&mut app.vault, r);
                    });
                }
            }
            KeyCode::Char('e') => {
                if let Some(r) = self.read_node() {
                    let target = if self.vault.tree.node(r).is_embed() {
                        self.vault.tree.resolved_child(r)
                    } else {
                        r
                    };
                    self.edit_buf = Some(fold_core::edit::open_editor(&self.vault, target));
                    self.edit_cursor = (0, 0);
                    self.edit_saved_dot = false;
                    self.mode = Mode::Edit;
                }
            }
            KeyCode::Char('a') => {
                if let Some(r) = self.read_node() {
                    let target = if self.vault.tree.node(r).is_embed() {
                        self.vault.tree.resolved_child(r)
                    } else {
                        r
                    };
                    self.props_target = Some(target);
                    self.props_rows = if self.vault.tree.node(target).is_block() {
                        ops::frontmatter_lines(&self.vault, target.0)
                            .into_iter()
                            .filter(|(k, _, _)| k != "id")
                            .collect()
                    } else {
                        Vec::new()
                    };
                    self.props_sel = 0;
                    self.mode = Mode::Props;
                }
            }
            KeyCode::Char('o') => self.open_link_under_cursor(&doc),
            KeyCode::Char('/') => {
                self.prompt = Some(Prompt {
                    label: "search".into(),
                    text: String::new(),
                    action: PromptAction::ReadSearch,
                });
            }
            KeyCode::Char('n') => self.next_match(&doc, 1),
            KeyCode::Char('N') => self.next_match(&doc, -1),
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    /// Reset the reading cursor when the pane starts showing another node
    /// (outline cursor moved, zoom changed), and keep it inside the doc.
    fn sync_read_target(&mut self) {
        let key = self.zoom_root.or_else(|| self.current()).map(|r| self.vault.key_of(r));
        if key != self.read_key {
            self.read_key = key;
            self.read_cursor = 0;
            self.scroll_reading = 0;
            self.read_matches.clear();
        }
        let n = self.reading_doc().lines.len();
        self.read_cursor = self.read_cursor.min(n.saturating_sub(1));
    }

    pub fn reading_doc_pub(&self) -> fold_core::reading::ReadingDoc { self.reading_doc() }
    fn reading_doc(&self) -> fold_core::reading::ReadingDoc {
        let target = self.zoom_root.or_else(|| self.current());
        match target {
            Some(r) => fold_core::reading::build(&self.vault, r),
            None => fold_core::reading::ReadingDoc {
                lines: Vec::new(),
                refs: Vec::new(),
            },
        }
    }

    /// The node under the reading cursor (title/body/embed lines only).
    fn read_node(&self) -> Option<NRef> {
        let doc = self.reading_doc();
        use fold_core::reading::LineRef;
        match fold_core::reading::node_at(&doc, self.read_cursor) {
            Some(LineRef::Title(r)) | Some(LineRef::Body(r)) | Some(LineRef::Embed(r)) => Some(r),
            _ => None,
        }
    }

    /// Run a mutation on a node from the reading pane, keeping the cursor.
    fn act_on_node(&mut self, r: NRef, f: impl Fn(&mut App, NRef)) {
        self.push_undo("reading-pane edit");
        f(self, r);
    }

    fn jump_heading(&mut self, doc: &fold_core::reading::ReadingDoc, dir: i32) {
        use fold_core::reading::LineRef;
        let mut i = self.read_cursor as i32 + dir;
        while i >= 0 && (i as usize) < doc.lines.len() {
            if matches!(doc.refs[i as usize], LineRef::Title(_)) {
                let l = &doc.lines[i as usize];
                if l.trim_start().starts_with('#') {
                    self.read_cursor = i as usize;
                    return;
                }
            }
            i += dir;
        }
    }

    fn update_read_matches(&mut self, doc: &fold_core::reading::ReadingDoc) {
        let q = self.read_search.to_lowercase();
        self.read_matches = doc
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !q.is_empty() && l.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.read_match_idx = 0;
        if let Some(&m) = self.read_matches.first() {
            self.read_cursor = m;
        }
        self.say(format!("{} match(es)", self.read_matches.len()));
    }

    fn next_match(&mut self, doc: &fold_core::reading::ReadingDoc, dir: i32) {
        if self.read_matches.is_empty() {
            self.say("no search (use /)");
            return;
        }
        let _ = doc;
        let n = self.read_matches.len() as i32;
        self.read_match_idx = ((self.read_match_idx as i32 + dir).rem_euclid(n)) as usize;
        let m = self.read_matches[self.read_match_idx];
        self.read_cursor = m;
    }

    fn open_link_under_cursor(&mut self, doc: &fold_core::reading::ReadingDoc) {
        let Some(line) = doc.lines.get(self.read_cursor) else { return };
        if let Some(url) = extract_url(line) {
            let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
            match std::process::Command::new(opener).arg(&url).spawn() {
                Ok(_) => self.say(format!("opened {}", url)),
                Err(e) => self.say(format!("open failed: {}", e)),
            }
        } else {
            self.say("no link on this line");
        }
    }

    pub fn key_filter(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.filter.clear();
                self.filter_rows.clear();
            }
            KeyCode::Enter => {
                if let Some(&r) = self.filter_rows.first() {
                    // zoom to the hit
                    let path = self.vault.tree.ancestors(r);
                    // unfold everything along the way
                    for a in &path {
                        let k = self.vault.key_of(*a);
                        self.folded.retain(|f| f != &k);
                    }
                    self.zoom_root = None;
                    self.move_cursor_to(r);
                }
                self.mode = Mode::Normal;
                self.filter.clear();
                self.filter_rows.clear();
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.update_filter();
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.update_filter();
            }
            _ => {}
        }
    }

    fn update_filter(&mut self) {
        let q = self.filter.to_lowercase();
        if q.is_empty() {
            self.filter_rows.clear();
            return;
        }
        let mut hits = Vec::new();
        self.vault.tree.walk(self.vault.tree.root, &mut |t, r| {
            let title = t.node(r).title.to_lowercase();
            if fuzzy_match(&q, &title) {
                hits.push(r);
                return;
            }
            // full-text over body
            let body = t.node(r).text_lines(t.text_of(r)).join("\n").to_lowercase();
            if body.contains(&q) {
                hits.push(r);
            }
        });
        self.filter_rows = hits;
    }

    pub fn key_palette(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.palette.clear();
            }
            KeyCode::Enter => {
                let actions = palette_actions();
                let q = self.palette.to_lowercase();
                let hits: Vec<&PaletteAction> = actions
                    .iter()
                    .filter(|a| {
                        q.is_empty()
                            || fuzzy_match(&q, &a.name.to_lowercase())
                            || fuzzy_match(&q, &a.desc.to_lowercase())
                    })
                    .collect();
                if let Some(a) = hits.first() {
                    let name = a.name;
                    self.mode = Mode::Normal;
                    self.palette.clear();
                    self.run_palette(name);
                } else {
                    self.mode = Mode::Normal;
                    self.palette.clear();
                }
            }
            KeyCode::Backspace => {
                self.palette.pop();
            }
            KeyCode::Char(c) => self.palette.push(c),
            _ => {}
        }
    }

    fn run_palette(&mut self, name: &str) {
        match name {
            "clear done" => self.act_clear_done(),
            "canonicalize" | "check" => match fold_core::check::fix(&mut self.vault) {
                Ok(n) => self.say(format!("{} file(s) canonicalized", n)),
                Err(e) => self.say(format!("error: {}", e)),
            },
            "merge" => match fold_core::merge::merge_sync_conflicts(&mut self.vault, false) {
                Ok(o) => {
                    self.say(format!("{} merge(s)", o.len()));
                    if !fold_core::merge::conflict_pairs(&self.vault).is_empty() {
                        self.enter_conflict_view();
                    }
                }
                Err(e) => self.say(format!("error: {}", e)),
            },
            "resolve conflicts" => self.enter_conflict_view(),
            "toggle task" => self.act_toggle_task(),
            "toggle task-ness" => self.act_toggle_taskness(),
            "make block" => self.act_make_block(),
            "archive" => self.act_archive(),
            "delete" => self.act_delete(),
            "yank" => self.act_yank(),
            "capture" => self.act_capture(false),
            "capture task" => self.act_capture(true),
            "go to" => {
                self.prompt = Some(Prompt {
                    label: "go to".into(),
                    text: String::new(),
                    action: PromptAction::GoTo,
                });
            }
            "refile" => {
                self.prompt = Some(Prompt {
                    label: "refile to".into(),
                    text: String::new(),
                    action: PromptAction::Refile,
                });
            }
            "zoom out" => {
                self.zoom_root = None;
                self.cursor = 0;
            }
            "help" => self.mode = Mode::Help,
            "quit" => self.quit = true,
            _ => self.say(format!("{}: no handler", name)),
        }
    }

    // -------------------------------------------------------- render

    pub fn draw(&mut self, f: &mut ratatui::Frame) {
        let size = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(size);
        let narrow = size.width < 80;
        let panes = if narrow {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[0])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
                .split(chunks[0])
        };
        if self.mode == Mode::Conflict {
            self.draw_conflict(f, chunks[0]);
            self.draw_status(f, chunks[2]);
            self.draw_toolbar(f, chunks[1]);
            return;
        }
        self.pane_outline = panes[0];
        self.pane_reading = panes[1];
        self.draw_outline(f, panes[0]);
        match self.mode {
            Mode::Edit => self.draw_editor(f, panes[1]),
            _ => self.draw_reading(f, panes[1]),
        }
        let needs_keyboard = self.mouse_enabled
            && (self.mode == Mode::Edit
                || self.prompt.is_some()
                || self.mode == Mode::Filter
                || self.mode == Mode::Picker);
        if needs_keyboard {
            let kb = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(4)])
                .split(chunks[0]);
            self.draw_keyboard(f, kb[1]);
        }
        self.draw_status(f, chunks[2]);
        self.draw_toolbar(f, chunks[1]);
        match self.mode {
            Mode::Filter => self.draw_filter(f, size),
            Mode::Picker => self.draw_palette(f, size),
            Mode::Props => self.draw_props(f, size),
            Mode::Help => self.draw_help(f, size),
            _ => {}
        }
        if let Some(p) = self.prompt.as_ref() {
            let p = Prompt {
                label: p.label.clone(),
                text: p.text.clone(),
                action: match &p.action {
                    PromptAction::Refile => PromptAction::Refile,
                    PromptAction::GoTo => PromptAction::GoTo,
                    PromptAction::CaptureText(t) => PromptAction::CaptureText(*t),
                    PromptAction::PropSet(k) => PromptAction::PropSet(k.clone()),
                    PromptAction::PropNew => PromptAction::PropNew,
                    PromptAction::ReadSearch => PromptAction::ReadSearch,
                },
            };
            self.draw_prompt(f, size, &p);
        }
    }

    fn breadcrumb(&self) -> String {
        match self.zoom_root {
            None => "root".to_string(),
            Some(z) => self.vault.tree.path(z).join(" › "),
        }
    }

    fn draw_outline(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let rows = self.rows();
        let focused = self.focus == Focus::Outline;
        let title = format!(" {} ", self.breadcrumb());
        let block = WBlock::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(if focused {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            });
        let inner_height = area.height.saturating_sub(2) as usize;
        // keep cursor visible
        let scroll = if self.cursor >= inner_height {
            self.cursor + 1 - inner_height
        } else {
            0
        };
        self.outline_scroll = scroll;
        let mut lines: Vec<Line> = Vec::new();
        for (i, row) in rows.iter().enumerate().skip(scroll).take(inner_height) {
            let n = self.vault.tree.node(row.nref);
            let mut spans: Vec<TSpan> = Vec::new();
            let indent = "  ".repeat(row.depth);
            spans.push(TSpan::raw(indent));
            let kids = self.vault.tree.resolved_children(row.nref);
            if !kids.is_empty() {
                spans.push(TSpan::raw(if self.is_folded(row.nref) {
                    "▸ "
                } else {
                    "▾ "
                }));
            } else {
                spans.push(TSpan::raw("  "));
            }
            let (glyph, style) = match n.task {
                Some(TaskState::Open) => ("☐ ", Style::default()),
                Some(TaskState::Done) => (
                    "☑ ",
                    Style::default().fg(Color::DarkGray),
                ),
                None => ("", Style::default()),
            };
            spans.push(TSpan::styled(glyph, style));
            let title_style = if n.task == Some(TaskState::Done) {
                Style::default().fg(Color::DarkGray)
            } else if n.kind == Kind::Section {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let marker = if n.is_embed() || n.is_block() { " ▤" } else { "" };
            spans.push(TSpan::styled(format!("{}{}", n.title, marker), title_style));
            let (open, total) = self.vault.tree.task_counts(row.nref);
            if total > 0 {
                spans.push(TSpan::styled(
                    format!("  {}/{}", open, total),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            // due date for blocks (§10.1)
            if let Some(b) = &n.block {
                if let Some(due) = b.prop("due") {
                    spans.push(TSpan::styled(
                        format!("  due {}", due),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            let line = if i == self.cursor && focused {
                Line::from(spans).style(Style::default().bg(Color::DarkGray))
            } else {
                Line::from(spans)
            };
            lines.push(line);
        }
        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn draw_reading(&mut self, f: &mut ratatui::Frame, area: Rect) {
        self.sync_read_target();
        self.read_header = false;
        let focused = self.focus == Focus::Reading;
        let block = WBlock::default()
            .borders(Borders::ALL)
            .title(" reading ")
            .border_style(if focused {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            });
        let target = self.zoom_root.or_else(|| self.current());
        let mut lines: Vec<Line> = Vec::new();
        if let Some(r) = target {
            if self.raw_mode {
                let text = render(&self.vault.tree, r, 1, false);
                for l in text.lines() {
                    lines.push(style_markdown_line(l, &self.vault, r));
                }
            } else {
                let doc = fold_core::reading::build(&self.vault, r);
                // block properties as a dimmed header (§4.4, §10.1)
                let mut prop_header: Option<Line> = None;
                if let Some(b) = &self.vault.tree.node(r).block {
                    let props: Vec<String> = b
                        .props
                        .iter()
                        .filter(|(k, _)| k.as_str() != "id")
                        .map(|(k, v)| format!("{} {}", k, v))
                        .collect();
                    if !props.is_empty() {
                        prop_header = Some(Line::from(TSpan::styled(
                            props.join(" · "),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
                for (i, l) in doc.lines.iter().enumerate() {
                    let mut line = style_markdown_line(l, &self.vault, r);
                    if focused {
                        if i == self.read_cursor {
                            line = line.style(Style::default().bg(Color::DarkGray));
                        } else if self.read_matches.contains(&i) {
                            line = line.style(Style::default().bg(Color::Indexed(58)));
                        }
                    }
                    lines.push(line);
                }
                if let Some(h) = prop_header {
                    lines.insert(1.min(lines.len()), h);
                    self.read_header = true;
                }
            }
        }
        let inner = area.height.saturating_sub(2) as usize;
        // scroll only when the reading cursor leaves the viewport
        let shown = self.read_cursor + (self.read_header && self.read_cursor >= 1) as usize;
        if shown < self.scroll_reading {
            self.scroll_reading = shown;
        } else if inner > 0 && shown >= self.scroll_reading + inner {
            self.scroll_reading = shown + 1 - inner;
        }
        self.scroll_reading = self.scroll_reading.min(lines.len().saturating_sub(1));
        let lines: Vec<Line> = lines.into_iter().skip(self.scroll_reading).collect();
        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn draw_editor(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let block = WBlock::default()
            .borders(Borders::ALL)
            .title(" editing ")
            .border_style(Style::default().fg(Color::White));
        let Some(buf) = &self.edit_buf else {
            f.render_widget(block, area);
            return;
        };
        let inner_height = area.height.saturating_sub(2) as usize;
        let (cl, _cc) = self.edit_cursor;
        let scroll = if cl >= inner_height {
            cl + 1 - inner_height
        } else {
            0
        };
        let mut lines: Vec<Line> = Vec::new();
        for (i, l) in buf.lines.iter().enumerate().skip(scroll).take(inner_height) {
            let style = if i == cl {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            let mut line = Line::from(TSpan::styled(l.text.clone(), style));
            if i == cl {
                // place a visible cursor by styling the cell
                line = Line::from(TSpan::styled(l.text.clone(), style));
            }
            lines.push(line);
        }
        f.render_widget(Paragraph::new(lines).block(block), area);
        // terminal cursor
        let x = area.x + 1 + self.edit_cursor.1 as u16;
        let y = area.y + 1 + (cl - scroll) as u16;
        if x < area.x + area.width && y < area.y + area.height {
            f.set_cursor_position((x, y));
        }
    }

    fn draw_props(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let h = (self.props_rows.len() as u16 + 4).min(area.height.saturating_sub(4)).max(4);
        let rect = Rect {
            x: area.x + 6,
            y: area.y + 3,
            width: area.width.saturating_sub(12).min(60),
            height: h,
        }
        .intersection(area);
        self.props_row_rects.clear();
        if rect.height < 3 {
            return;
        }
        f.render_widget(Clear, rect);
        let mut lines: Vec<Line> = Vec::new();
        let title = self
            .props_target
            .map(|t| self.vault.tree.node(t).title.clone())
            .unwrap_or_default();
        lines.push(Line::from(TSpan::styled(
            format!("properties of {}", title),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if self.props_rows.is_empty() {
            lines.push(Line::from(TSpan::styled(
                "  (none — n adds one; the first makes this node a block)",
                Style::default().fg(Color::DarkGray),
            )));
        }
        self.props_row_rects.clear();
        for (i, (k, v, editable)) in self.props_rows.iter().enumerate() {
            self.props_row_rects.push(Rect {
                x: rect.x + 1,
                y: rect.y + 2 + i as u16,
                width: rect.width.saturating_sub(2),
                height: 1,
            });
            let style = if i == self.props_sel {
                Style::default().bg(Color::DarkGray)
            } else if !editable {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            lines.push(Line::from(TSpan::styled(
                format!("  {}: {}", k, v),
                style,
            )));
        }
        lines.push(Line::from(TSpan::styled(
            "  n new · Enter/e edit · d delete · Esc close",
            Style::default().fg(Color::DarkGray),
        )));
        let block = WBlock::default().borders(Borders::ALL).title(" properties ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }

    fn draw_prompt(&mut self, f: &mut ratatui::Frame, area: Rect, p: &Prompt) {
        let rect = Rect {
            x: area.x + 4,
            y: area.y + area.height.saturating_sub(3),
            width: area.width.saturating_sub(8),
            height: 3,
        };
        f.render_widget(Clear, rect);
        let line = Line::from(vec![
            TSpan::styled(format!("{}: ", p.label), Style::default().add_modifier(Modifier::BOLD)),
            TSpan::raw(p.text.clone()),
        ]);
        let block = WBlock::default().borders(Borders::ALL);
        f.render_widget(Paragraph::new(line).block(block), rect);
    }

    /// The clickable action bar above the status line (mouse support).
    fn draw_toolbar(&mut self, f: &mut ratatui::Frame, area: Rect) {
        if !self.mouse_enabled {
            self.toolbar.clear();
            return;
        }
        self.toolbar_rect = area;
        let actions: &[&str] = match self.mode {
            Mode::Normal => &[
                "New", "Edit", "Props", "Done", "Task", "Block", "Yank", "Trash", "Paste",
                "Capture", "Archive", "Refile", "Undo", "Redo", "Filter", "Menu", "Help", "Quit",
            ],
            Mode::Edit => &["Save", "Close", "Discard"],
            Mode::Conflict => &["Ours", "Theirs", "Both", "Edit", "Prev", "Next", "Done"],
            Mode::Props => &["New", "Edit", "Del", "Close"],
            Mode::Filter | Mode::Picker | Mode::Help => &["Close"],
        };
        self.toolbar.clear();
        let mut spans: Vec<TSpan> = Vec::new();
        let mut x = area.x + 1;
        for a in actions {
            let label = format!(" {} ", a);
            let w = label.chars().count() as u16;
            if x + w + 1 > area.x + area.width {
                break;
            }
            self.toolbar.push((
                Rect {
                    x,
                    y: area.y,
                    width: w,
                    height: 1,
                },
                a,
            ));
            spans.push(TSpan::styled(
                label,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::DarkGray),
            ));
            spans.push(TSpan::raw(" "));
            x += w + 1;
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// A small on-screen keyboard for the built-in editor (mouse support):
    /// every character clickable, plus Enter/Backspace/arrows.
    fn draw_keyboard(&mut self, f: &mut ratatui::Frame, area: Rect) {
        self.kb_rects.clear();
        self.kb_special.clear();
        let rows: [&str; 3] = [
            "1234567890-=",
            "qwertyuiop[]",
            "asdfghjkl;'",
        ];
        let mut y = area.y;
        for row in rows {
            let mut x = area.x + 1;
            for c in row.chars() {
                self.kb_rects.push((
                    Rect {
                        x,
                        y,
                        width: 1,
                        height: 1,
                    },
                    c,
                ));
                x += 1;
            }
            y += 1;
        }
        // fourth row: the rest of the letters + specials
        let letters = "zxcvbnm,./";
        let mut x = area.x + 1;
        for c in letters.chars() {
            self.kb_rects.push((
                Rect {
                    x,
                    y,
                    width: 1,
                    height: 1,
                },
                c,
            ));
            x += 1;
        }
        let specials: &[&str] = &["Space", "Enter", "Bksp", "↑", "↓", "←", "→"];
        for s in specials {
            let w = s.chars().count() as u16;
            self.kb_special.push((
                Rect {
                    x,
                    y,
                    width: w,
                    height: 1,
                },
                s,
            ));
            x += w + 1;
        }
        // render all keys
        for (r, c) in &self.kb_rects {
            f.render_widget(
                Paragraph::new(c.to_string()).style(Style::default().fg(Color::Cyan)),
                *r,
            );
        }
        for (r, s) in &self.kb_special {
            f.render_widget(
                Paragraph::new(s.to_string()).style(Style::default().fg(Color::Cyan)),
                *r,
            );
        }
    }

    fn draw_status(&mut self, f: &mut ratatui::Frame, area: Rect) {        let file = self
            .current()
            .map(|r| self.vault.tree.files[r.0].path.clone())
            .unwrap_or_else(|| "root.md".into());
        // in edit mode the status names the block the cursor is in (§10.6)
        let edit_part = if self.mode == Mode::Edit {
            match &self.edit_buf {
                Some(buf) => {
                    let owner = buf.owner_at(self.edit_cursor.0);
                    let title = buf
                        .owners
                        .get(&owner)
                        .map(|o| o.title.clone())
                        .unwrap_or_default();
                    let dot = if self.edit_saved_dot || !buf.dirty.is_empty() {
                        " ●"
                    } else {
                        ""
                    };
                    format!(" {}· {}", title, dot)
                }
                None => String::new(),
            }
        } else {
            String::new()
        };
        let mode = match self.mode {
            Mode::Normal => "",
            Mode::Edit => " EDIT",
            Mode::Filter => " FILTER",
            Mode::Picker => " :",
            Mode::Conflict => " CONFLICT",
            Mode::Props => " PROPS",
            Mode::Help => " HELP",
        };
        let conflicts = fold_core::merge::conflict_pairs(&self.vault).len();
        let cpart = if conflicts > 0 {
            format!(" · {} conflicts", conflicts)
        } else {
            String::new()
        };
        let time = jiff::Zoned::now().strftime("%H:%M").to_string();
        let line = Line::from(vec![
            TSpan::styled(mode, Style::default().add_modifier(Modifier::BOLD)),
            TSpan::raw(format!(
                "  {} · {}{} · saved{}{}",
                self.status, file, edit_part, cpart, ""
            )),
            TSpan::styled(format!("  {}", time), Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn draw_filter(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let h = (self.filter_rows.len() as u16 + 3).min(area.height / 2).max(3);
        let rect = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: h,
        }
        .intersection(area);
        self.filter_rows_rects.clear();
        if rect.height < 3 {
            return;
        }
        f.render_widget(Clear, rect);
        let mut lines = vec![Line::from(vec![
            TSpan::styled("/ ", Style::default().add_modifier(Modifier::BOLD)),
            TSpan::raw(self.filter.clone()),
        ])];
        self.filter_rows_rects.clear();
        for (i, r) in self.filter_rows.iter().take(rect.height as usize - 2).enumerate() {
            let path = self.vault.tree.path(*r).join(" › ");
            self.filter_rows_rects.push((
                Rect {
                    x: rect.x + 1,
                    y: rect.y + 2 + i as u16,
                    width: rect.width.saturating_sub(2),
                    height: 1,
                },
                *r,
            ));
            lines.push(Line::from(TSpan::raw(format!("  {}", path))));
        }
        let block = WBlock::default().borders(Borders::ALL).title(" filter ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }

    fn draw_help(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let w = area.width.saturating_sub(8).min(76);
        let h = area.height.saturating_sub(4);
        let rect = Rect {
            x: (area.width.saturating_sub(w)) / 2,
            y: (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        };
        f.render_widget(Clear, rect);
        let block = WBlock::default()
            .borders(Borders::ALL)
            .title(" fold — help (?/Esc closes) ")
            .border_style(Style::default().fg(Color::Cyan));
        f.render_widget(Paragraph::new(help_text()).block(block), rect);
    }

    fn draw_palette(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let actions = palette_actions();
        let q = self.palette.to_lowercase();
        let hits: Vec<&PaletteAction> = actions
            .iter()
            .filter(|a| {
                q.is_empty()
                    || fuzzy_match(&q, &a.name.to_lowercase())
                    || fuzzy_match(&q, &a.desc.to_lowercase())
            })
            .collect();
        let h = (hits.len() as u16 + 3).min(area.height.saturating_sub(4)).max(3);
        let rect = Rect {
            x: area.x + 4,
            y: area.y + 2,
            width: area.width.saturating_sub(8),
            height: h,
        }
        .intersection(area);
        if rect.height < 3 {
            self.palette_rows.clear();
            return;
        }
        f.render_widget(Clear, rect);
        let mut lines = vec![Line::from(vec![
            TSpan::styled(": ", Style::default().add_modifier(Modifier::BOLD)),
            TSpan::raw(self.palette.clone()),
        ])];
        self.palette_rows.clear();
        for (i, a) in hits.iter().take(rect.height as usize - 2).enumerate() {
            self.palette_rows.push((
                Rect {
                    x: rect.x + 1,
                    y: rect.y + 2 + i as u16,
                    width: rect.width.saturating_sub(2),
                    height: 1,
                },
                a.name,
            ));
            lines.push(Line::from(vec![
                TSpan::raw(format!("  {:<20}", a.name)),
                TSpan::styled(
                    format!("{:<12}", a.key.unwrap_or("")),
                    Style::default().fg(Color::DarkGray),
                ),
                TSpan::styled(a.desc, Style::default().fg(Color::DarkGray)),
            ]));
        }
        let block = WBlock::default().borders(Borders::ALL).title(" commands ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }
}

fn style_markdown_line(l: &str, vault: &Vault, _ctx: NRef) -> Line<'static> {
    let trimmed = l.trim_start();
    if trimmed.starts_with('#') {
        let level = trimmed.chars().take_while(|&c| c == '#').count();
        let color = match level {
            1 => Color::Cyan,
            2 => Color::Blue,
            _ => Color::White,
        };
        return Line::from(TSpan::styled(
            l.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    if trimmed.starts_with("- [ ] ") || trimmed.starts_with("[ ] ") {
        return Line::from(TSpan::styled(
            l.replacen("[ ]", "☐", 1),
            Style::default(),
        ));
    }
    if trimmed.starts_with("- [x] ") || trimmed.starts_with("[x] ") {
        return Line::from(TSpan::styled(
            l.replacen("[x]", "☑", 1),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if trimmed.starts_with("![[") {
        // unresolved embed in raw mode
        let _ = vault;
        return Line::from(TSpan::styled(
            l.to_string(),
            Style::default().fg(Color::Yellow),
        ));
    }
    if trimmed.starts_with(">") {
        return Line::from(TSpan::styled(
            l.to_string(),
            Style::default().fg(Color::Green),
        ));
    }
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        return Line::from(TSpan::styled(
            l.to_string(),
            Style::default().fg(Color::Magenta),
        ));
    }
    Line::from(TSpan::raw(l.to_string()))
}

/// The help text, shared by `?` in the TUI and `fold help` (§10.9).
pub fn help_text() -> Vec<Line<'static>> {
    let entries: &[(&str, &str)] = &[
        ("", "fold — a tree of notes and tasks in plain Markdown."),
        ("", "One outline; zoom is the unit of reading and editing."),
        ("", ""),
        ("MOVING", ""),
        ("j/k ←/→", "move · h/l fold & unfold · gg/G first/last"),
        ("Enter", "zoom into the node (reading pane takes focus)"),
        ("Backspace", "zoom out · - parent · {/} prev/next sibling"),
        ("Tab", "switch outline ↔ reading pane · Ctrl-d/u scroll"),
        ("", ""),
        ("NODES", ""),
        ("n / N", "new sibling / new child (opens the editor)"),
        ("e", "edit the subtree's Markdown (saves as you type)"),
        ("a", "properties form — the first property makes a block"),
        ("x / t", "toggle done / toggle task-ness"),
        ("s", "make block: give the node its own file + id"),
        ("~", "spelling: heading ↔ bullet · >/< demote/promote"),
        ("J / K", "move among siblings · r refile (id, path or title)"),
        ("y / d", "yank / trash subtree · p/P paste after/before"),
        ("u / U", "undo / redo"),
        ("", ""),
        ("FINDING", ""),
        ("/", "filter box: fuzzy titles + full text, Enter zooms"),
        (":", "command palette — every action by name"),
        ("?", "this help"),
        ("", ""),
        ("TASKS & CAPTURE", ""),
        ("c / C", "capture a note / a task into today's inbox day"),
        ("za", "archive subtree under # Archive · zd hide done"),
        ("zr", "raw mode: exact source, frontmatter included"),
        (":clear done", "trash done items under the zoom root"),
        ("", ""),
        ("READING PANE", ""),
        ("Enter", "toggle task / zoom heading / follow embed"),
        ("x e a o", "toggle · edit · properties · open link"),
        ("[[ / ]]", "previous / next heading · / search, n/N next"),
        ("", ""),
        ("MOUSE", ""),
        ("click", "select · ▾/▸ folds · double-click zooms/follows/toggles"),
        ("wheel", "scroll the pane under the pointer"),
        ("toolbar", "the button bar above the status line has every action"),
        ("keyboard", "editing/prompts show an on-screen keyboard"),
        ("", ""),
        ("CONFLICTS & QUIT", ""),
        (":merge", "fold sync-conflict files in, then resolve:"),
        ("o t b", "keep ours / theirs / both · n/N pairs · Enter done"),
        ("q", "quit — everything is always saved"),
    ];
    entries
        .iter()
        .map(|(k, v)| {
            if v.is_empty() && !k.is_empty() {
                Line::from(TSpan::styled(
                    k.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(vec![
                    TSpan::styled(
                        format!("{:<11}", k),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    TSpan::raw(v.to_string()),
                ])
            }
        })
        .collect()
}

fn fuzzy_match(needle: &str, hay: &str) -> bool {
    // simple subsequence match (nucleo would weight; this is the picker path)
    let mut n = needle.chars().peekable();
    for c in hay.chars() {
        if let Some(&nc) = n.peek() {
            if c == nc {
                n.next();
            }
        }
    }
    n.peek().is_none()
}

/// First `[text](url)` or bare http(s) URL on a line (§4.6, §10.4 `o`).
fn extract_url(line: &str) -> Option<String> {
    // [text](url)
    if let Some(open) = line.find("](") {
        if let Some(close) = line[open + 2..].find(')') {
            let url = &line[open + 2..open + 2 + close];
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
    }
    // bare URL
    for scheme in ["https://", "http://"] {
        if let Some(i) = line.find(scheme) {
            let rest = &line[i..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ')' || c == '"')
                .unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
    }
    None
}

struct PaletteAction {
    name: &'static str,
    key: Option<&'static str>,
    desc: &'static str,
}

fn palette_actions() -> Vec<PaletteAction> {
    vec![
        PaletteAction { name: "toggle task", key: Some("x"), desc: "open ↔ done" },
        PaletteAction { name: "toggle task-ness", key: Some("t"), desc: "make/unmake a task" },
        PaletteAction { name: "make block", key: Some("s"), desc: "give the node its own file" },
        PaletteAction { name: "archive", key: Some("za"), desc: "refile under Archive" },
        PaletteAction { name: "delete", key: Some("d"), desc: "trash the subtree" },
        PaletteAction { name: "yank", key: Some("y"), desc: "copy subtree to the register" },
        PaletteAction { name: "capture", key: Some("c"), desc: "append to the inbox" },
        PaletteAction { name: "capture task", key: Some("C"), desc: "append a task to the inbox" },
        PaletteAction { name: "clear done", key: None, desc: "trash done items under the zoom root" },
        PaletteAction { name: "canonicalize", key: None, desc: "rewrite the vault in canonical form" },
        PaletteAction { name: "check", key: None, desc: "diagnostics (same as canonicalize here)" },
        PaletteAction { name: "merge", key: None, desc: "process sync-conflict files" },
        PaletteAction { name: "resolve conflicts", key: None, desc: "review and resolve conflict pairs" },
        PaletteAction { name: "go to", key: None, desc: "jump to a node by id, path or title" },
        PaletteAction { name: "refile", key: Some("r"), desc: "move the subtree under a new parent" },
        PaletteAction { name: "zoom out", key: Some("Backspace"), desc: "up one zoom level" },
        PaletteAction { name: "help", key: Some("?"), desc: "how to use fold" },
        PaletteAction { name: "quit", key: Some("q"), desc: "save and exit" },
    ]
}

// ------------------------------------------------------------ main loop

pub fn run(dir: &Path) -> anyhow::Result<()> {
    let mut app = App::new(dir)?;
    app.enable_mouse();
    app.start_watcher();
    // a sync-conflict file present at startup starts the merge flow (§12.2)
    if let Ok(files) = app.vault.conflict_files() {
        if !files.is_empty() {
            match fold_core::merge::merge_sync_conflicts(&mut app.vault, false) {
                Ok(o) => {
                    app.say(format!("merged on startup: {}", o.join("; ")));
                    app.enter_conflict_view();
                }
                Err(e) => app.say(format!("merge error: {}", e)),
            }
        }
    }
    enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;
    std::io::stdout().execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let res = run_loop(&mut terminal, &mut app);
    disable_raw_mode()?;
    std::io::stdout().execute(DisableMouseCapture)?;
    std::io::stdout().execute(LeaveAlternateScreen)?;
    res
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| app.draw(f))?;
        if app.quit {
            // save any open editor on quit (§10.6)
            if app.edit_buf.is_some() {
                app.close_editor();
            }
            return Ok(());
        }
        // autosave after 750 ms without a keystroke (§10.6)
        if app.mode == Mode::Edit
            && app.edit_buf.as_ref().map(|b| !b.dirty.is_empty()).unwrap_or(false)
            && app.edit_last_key.elapsed() > Duration::from_millis(750)
        {
            app.save_editor("pause");
        }
        // external changes: debounced reload (§11.2)
        if app.poll_watcher() {
            app.reload_external();
        }
        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Mouse(m) => app.handle_mouse(m),
                Event::Key(key) => app.handle_key(key),
                _ => {}
            }
        }
    }
}
