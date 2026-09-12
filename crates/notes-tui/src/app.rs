//! The TUI application (§10).

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use notes_core::ops;
use notes_core::parse::{Kind, TaskState};
use notes_core::render::render;
use notes_core::tree::NRef;
use notes_core::vault::{NodeKey, Vault};
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
    edit_buf: Option<notes_core::edit::EditBuffer>,
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
        })
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
    pub fn mode_pub(&self) -> &'static str {
        match self.mode {
            Mode::Normal => "normal",
            Mode::Edit => "edit",
            Mode::Filter => "filter",
            Mode::Picker => "picker",
            Mode::Conflict => "conflict",
            Mode::Props => "props",
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
                            && !name.ends_with(".notes-tmp")
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
                match notes_core::merge::merge_sync_conflicts(&mut self.vault, false) {
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
        self.push_undo("act_toggle_task");
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_toggle_taskness");
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_make_block");
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_delete");
        let Some(r) = self.current() else { return };
        self.register = ops::yank(&self.vault, r);
        match ops::delete_subtree(&mut self.vault, r) {
            Ok(m) => self.refresh_after(&m),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_yank(&mut self) {
        self.push_undo("act_yank");
        let Some(r) = self.current() else { return };
        self.register = ops::yank(&self.vault, r);
        self.say("yanked");
    }

    fn act_paste(&mut self, after: bool) {
        self.push_undo("act_paste");
        if self.register.is_empty() {
            self.say("register empty");
            return;
        }
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_move");
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_spelling");
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_demote");
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_promote");
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_archive");
        let Some(r) = self.current() else { return };
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
        self.push_undo("act_refile");
        let Some(r) = self.current() else { return };
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
        let res = if child {
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
            match ops::paste(&mut self.vault, r, &format!("{}\n", line), true) {
                Ok(()) => {
                    // move to the new sibling (next row with an empty title)
                    let rows = self.rows();
                    if let Some(i) = rows
                        .iter()
                        .position(|row| self.vault.tree.node(row.nref).title.is_empty())
                    {
                        self.cursor = i;
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
        self.edit_buf = Some(notes_core::edit::open_editor(&self.vault, target));
        self.edit_cursor = (0, 0);
        self.edit_saved_dot = false;
        self.mode = Mode::Edit;
        self.focus = Focus::Reading;
    }

    fn save_editor(&mut self, why: &str) {
        let Some(mut buf) = self.edit_buf.take() else { return };
        match buf.save_all(&mut self.vault) {
            Ok(n) => {
                if n > 0 {
                    self.say(format!("saved {} block(s) ({})", n, why));
                    self.mark_self_write();
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
            KeyCode::Up => {
                if line > 0 {
                    self.edit_cursor.0 = line - 1;
                }
            }
            KeyCode::Down => {
                if line + 1 < nlines {
                    self.edit_cursor.0 = line + 1;
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
                    let mut tmp = self.edit_buf.take().unwrap();
                    let _ = tmp.splice(&mut self.vault, old);
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
                                && !notes_core::check::is_iso_date(&p.text)
                            {
                                self.say(format!("{}: must be YYYY-MM-DD", k));
                                return;
                            }
                            match ops::set_property(&mut self.vault, t, &k, &p.text) {
                                Ok(()) => self.say(format!("{} set", k)),
                                Err(e) => self.say(format!("error: {}", e)),
                            }
                        }
                    }
                    PromptAction::PropNew => {
                        if self.props_target.is_some() {
                            let key_name = p.text.trim().to_string();
                            if notes_core::parse::is_valid_key(&key_name) {
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
        if notes_core::merge::conflict_pairs(&self.vault).is_empty() {
            self.say("no conflicts");
            return;
        }
        self.mode = Mode::Conflict;
        self.conflict_idx = 0;
    }

    pub fn key_conflict_pub(&mut self, key: KeyEvent) { self.key_conflict(key) }
    fn key_conflict(&mut self, key: KeyEvent) {
        let pairs = notes_core::merge::conflict_pairs(&self.vault);
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
                    match notes_core::merge::resolve_keep_ours(&mut self.vault, theirs) {
                        Ok(()) => self.say("kept ours"),
                        Err(e) => self.say(format!("error: {}", e)),
                    }
                    self.conflict_idx = self.conflict_idx.saturating_sub(0).min(
                        notes_core::merge::conflict_pairs(&self.vault)
                            .len()
                            .saturating_sub(1),
                    );
                }
            }
            KeyCode::Char('t') => {
                if let Some(&(ours, theirs)) = pairs.get(self.conflict_idx) {
                    self.push_undo("keep theirs");
                    match notes_core::merge::resolve_keep_theirs(&mut self.vault, ours, theirs) {
                        Ok(()) => self.say("kept theirs"),
                        Err(e) => self.say(format!("error: {}", e)),
                    }
                }
            }
            KeyCode::Char('b') => {
                if let Some(&(_, theirs)) = pairs.get(self.conflict_idx) {
                    self.push_undo("keep both");
                    match notes_core::merge::resolve_keep_both(&mut self.vault, theirs) {
                        Ok(()) => self.say("kept both"),
                        Err(e) => self.say(format!("error: {}", e)),
                    }
                }
            }
            KeyCode::Char('e') => {
                if let Some(&(ours, _)) = pairs.get(self.conflict_idx) {
                    self.edit_buf = Some(notes_core::edit::open_editor(&self.vault, ours));
                    self.edit_cursor = (0, 0);
                    self.edit_saved_dot = false;
                    self.mode = Mode::Edit;
                }
            }
            _ => {}
        }
    }

    fn draw_conflict(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let pairs = notes_core::merge::conflict_pairs(&self.vault);
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

    pub fn key_normal(&mut self, key: KeyEvent) {
        let rows = self.rows();
        match self.focus {
            Focus::Outline => self.key_outline(key, rows),
            Focus::Reading => self.key_reading(key),
        }
    }

    pub fn key_outline(&mut self, key: KeyEvent, rows: Vec<FlatRow>) {
        let len = rows.len();
        match key.code {
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
                    } else if let Some(p) = self.vault.tree.node(r).parent {
                        let pr = (r.0, p);
                        if self.vault.tree.node(pr).kind != Kind::Root {
                            self.move_cursor_to(pr);
                        }
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
            KeyCode::Char('z') => {
                // zd / zr / za
                self.say("z… (d: hide done, r: raw, a: archive)");
                // handled via pending key in main loop
            }
            KeyCode::Char('-') => {
                if let Some(r) = self.current() {
                    if let Some(p) = self.vault.tree.node(r).parent {
                        let pr = (r.0, p);
                        if self.vault.tree.node(pr).kind != Kind::Root {
                            self.move_cursor_to(pr);
                        }
                    }
                }
            }
            KeyCode::Char('{') => self.sibling(-1),
            KeyCode::Char('}') => self.sibling(1),
            KeyCode::Char('g') => self.cursor = 0, // gg handled via pending
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
            KeyCode::Char(':') | KeyCode::Char('?') => {
                self.mode = Mode::Picker;
                self.palette.clear();
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
            match inv.apply(&mut self.vault) {
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
            match inv.apply(&mut self.vault) {
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
        self.undo.push(self.snapshot(desc));
        self.redo.clear();
        self.mark_self_write();
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

    fn sibling(&mut self, dir: i32) {
        let Some(r) = self.current() else { return };
        let Some(p) = self.vault.tree.node(r).parent else { return };
        let sibs = self.vault.tree.resolved_children((r.0, p));
        if let Some(pos) = sibs.iter().position(|&s| s == r) {
            let np = pos as i32 + dir;
            if np >= 0 && (np as usize) < sibs.len() {
                self.move_cursor_to(sibs[np as usize]);
            }
        }
    }

    pub fn key_reading_pub(&mut self, key: KeyEvent) { self.key_reading(key) }
    fn key_reading(&mut self, key: KeyEvent) {
        let doc = self.reading_doc();
        let nlines = doc.lines.len();
        match key.code {
            KeyCode::Tab => self.focus = Focus::Outline,
            KeyCode::Char('j') | KeyCode::Down => {
                if self.read_cursor + 1 < nlines.max(1) {
                    self.read_cursor += 1;
                }
                self.scroll_reading = self.read_cursor;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.read_cursor = self.read_cursor.saturating_sub(1);
                self.scroll_reading = self.read_cursor;
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.read_cursor = (self.read_cursor + 10).min(nlines.saturating_sub(1));
                self.scroll_reading = self.read_cursor;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.read_cursor = self.read_cursor.saturating_sub(10);
                self.scroll_reading = self.read_cursor;
            }
            KeyCode::Char('G') => {
                self.read_cursor = nlines.saturating_sub(1);
                self.scroll_reading = self.read_cursor;
            }
            KeyCode::Char('g') => {} // gg via pending
            KeyCode::Enter => {
                use notes_core::reading::LineRef;
                match notes_core::reading::node_at(&doc, self.read_cursor) {
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
                    self.edit_buf = Some(notes_core::edit::open_editor(&self.vault, target));
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
            KeyCode::Char(']') => self.jump_heading(&doc, 1),
            KeyCode::Char('[') => self.jump_heading(&doc, -1),
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

    pub fn reading_doc_pub(&self) -> notes_core::reading::ReadingDoc { self.reading_doc() }
    fn reading_doc(&self) -> notes_core::reading::ReadingDoc {
        let target = self.zoom_root.or_else(|| self.current());
        match target {
            Some(r) => notes_core::reading::build(&self.vault, r),
            None => notes_core::reading::ReadingDoc {
                lines: Vec::new(),
                refs: Vec::new(),
            },
        }
    }

    /// The node under the reading cursor (title/body/embed lines only).
    fn read_node(&self) -> Option<NRef> {
        let doc = self.reading_doc();
        use notes_core::reading::LineRef;
        match notes_core::reading::node_at(&doc, self.read_cursor) {
            Some(LineRef::Title(r)) | Some(LineRef::Body(r)) | Some(LineRef::Embed(r)) => Some(r),
            _ => None,
        }
    }

    /// Run a mutation on a node from the reading pane, keeping the cursor.
    fn act_on_node(&mut self, r: NRef, f: impl Fn(&mut App, NRef)) {
        let key = self.vault.key_of(r);
        f(self, r);
        let _ = key;
    }

    fn jump_heading(&mut self, doc: &notes_core::reading::ReadingDoc, dir: i32) {
        use notes_core::reading::LineRef;
        let mut i = self.read_cursor as i32 + dir;
        while i >= 0 && (i as usize) < doc.lines.len() {
            if matches!(doc.refs[i as usize], LineRef::Title(_)) {
                let l = &doc.lines[i as usize];
                if l.trim_start().starts_with('#') {
                    self.read_cursor = i as usize;
                    self.scroll_reading = self.read_cursor;
                    return;
                }
            }
            i += dir;
        }
    }

    fn update_read_matches(&mut self, doc: &notes_core::reading::ReadingDoc) {
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
            self.scroll_reading = m;
        }
        self.say(format!("{} match(es)", self.read_matches.len()));
    }

    fn next_match(&mut self, doc: &notes_core::reading::ReadingDoc, dir: i32) {
        if self.read_matches.is_empty() {
            self.say("no search (use /)");
            return;
        }
        let _ = doc;
        let n = self.read_matches.len() as i32;
        self.read_match_idx = ((self.read_match_idx as i32 + dir).rem_euclid(n)) as usize;
        let m = self.read_matches[self.read_match_idx];
        self.read_cursor = m;
        self.scroll_reading = m;
    }

    fn open_link_under_cursor(&mut self, doc: &notes_core::reading::ReadingDoc) {
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
            let body = t.node(r).body_lines(t.text_of(r)).join("\n").to_lowercase();
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
            "canonicalize" | "check" => match notes_core::check::fix(&mut self.vault) {
                Ok(n) => self.say(format!("{} file(s) canonicalized", n)),
                Err(e) => self.say(format!("error: {}", e)),
            },
            "merge" => match notes_core::merge::merge_sync_conflicts(&mut self.vault, false) {
                Ok(o) => {
                    self.say(format!("{} merge(s)", o.len()));
                    if !notes_core::merge::conflict_pairs(&self.vault).is_empty() {
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
            "quit" => self.quit = true,
            _ => self.say(format!("{}: no handler", name)),
        }
    }

    // -------------------------------------------------------- render

    pub fn draw(&mut self, f: &mut ratatui::Frame) {
        let size = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
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
            self.draw_status(f, chunks[1]);
            return;
        }
        self.draw_outline(f, panes[0]);
        match self.mode {
            Mode::Edit => self.draw_editor(f, panes[1]),
            _ => self.draw_reading(f, panes[1]),
        }
        self.draw_status(f, chunks[1]);
        match self.mode {
            Mode::Filter => self.draw_filter(f, size),
            Mode::Picker => self.draw_palette(f, size),
            Mode::Props => self.draw_props(f, size),
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
                let doc = notes_core::reading::build(&self.vault, r);
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
                }
            }
        }
        let inner = area.height.saturating_sub(2) as usize;
        // keep the reading cursor visible
        let skip = if focused && self.read_cursor >= inner && inner > 0 {
            self.read_cursor + 1 - inner
        } else {
            self.scroll_reading.min(lines.len().saturating_sub(1))
        };
        let lines: Vec<Line> = lines.into_iter().skip(skip).collect();
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
        let h = (self.props_rows.len() as u16 + 4).min(area.height - 4).max(4);
        let rect = Rect {
            x: area.x + 6,
            y: area.y + 3,
            width: area.width.saturating_sub(12).min(60),
            height: h,
        };
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
        for (i, (k, v, editable)) in self.props_rows.iter().enumerate() {
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

    fn draw_status(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let file = self
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
        };
        let conflicts = notes_core::merge::conflict_pairs(&self.vault).len();
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
        };
        f.render_widget(Clear, rect);
        let mut lines = vec![Line::from(vec![
            TSpan::styled("/ ", Style::default().add_modifier(Modifier::BOLD)),
            TSpan::raw(self.filter.clone()),
        ])];
        for r in self.filter_rows.iter().take(h as usize - 2) {
            let path = self.vault.tree.path(*r).join(" › ");
            lines.push(Line::from(TSpan::raw(format!("  {}", path))));
        }
        let block = WBlock::default().borders(Borders::ALL).title(" filter ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
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
        let h = (hits.len() as u16 + 3).min(area.height - 4).max(3);
        let rect = Rect {
            x: area.x + 4,
            y: area.y + 2,
            width: area.width.saturating_sub(8),
            height: h,
        };
        f.render_widget(Clear, rect);
        let mut lines = vec![Line::from(vec![
            TSpan::styled(": ", Style::default().add_modifier(Modifier::BOLD)),
            TSpan::raw(self.palette.clone()),
        ])];
        for a in hits.iter().take(h as usize - 2) {
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
        PaletteAction { name: "quit", key: Some("q"), desc: "save and exit" },
    ]
}

// ------------------------------------------------------------ main loop

pub fn run(dir: &Path) -> anyhow::Result<()> {
    let mut app = App::new(dir)?;
    app.start_watcher();
    // a sync-conflict file present at startup starts the merge flow (§12.2)
    if let Ok(files) = app.vault.conflict_files() {
        if !files.is_empty() {
            match notes_core::merge::merge_sync_conflicts(&mut app.vault, false) {
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
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let res = run_loop(&mut terminal, &mut app);
    disable_raw_mode()?;
    std::io::stdout().execute(LeaveAlternateScreen)?;
    res
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    let mut pending_z = false;
    let mut pending_g = false;
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
            if let Event::Key(key) = event::read()? {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
                {
                    return Ok(());
                }
                // two-key sequences
                if pending_z {
                    pending_z = false;
                    match key.code {
                        KeyCode::Char('d') => {
                            app.hide_done = !app.hide_done;
                            app.say(if app.hide_done { "done hidden" } else { "done shown" });
                        }
                        KeyCode::Char('r') => {
                            app.raw_mode = !app.raw_mode;
                            app.say(if app.raw_mode { "raw" } else { "styled" });
                        }
                        KeyCode::Char('a') => app.act_archive(),
                        _ => {}
                    }
                    continue;
                }
                if pending_g {
                    pending_g = false;
                    if key.code == KeyCode::Char('g') {
                        app.cursor = 0;
                    }
                    continue;
                }
                match app.mode {
                    Mode::Normal => {
                        if app.prompt.is_some() {
                            app.key_prompt(key);
                            continue;
                        }
                        if key.code == KeyCode::Char('z') {
                            pending_z = true;
                            continue;
                        }
                        if key.code == KeyCode::Char('g') {
                            pending_g = true;
                            continue;
                        }
                        app.key_normal(key)
                    }
                    Mode::Filter => app.key_filter(key),
                    Mode::Picker => app.key_palette(key),
                    Mode::Edit => {
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('c')
                        {
                            // discard changes since last save (§10.6)
                            app.edit_buf = None;
                            app.mode = Mode::Normal;
                            app.vault.reload().ok();
                            app.say("changes discarded");
                            continue;
                        }
                        app.key_edit(key)
                    }
                    Mode::Props => app.key_props(key),
                    Mode::Conflict => app.key_conflict(key),
                }
            }
        }
    }
}
