//! The TUI application (§10).

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture};
use crossterm::ExecutableCommand;
use fold_core::ops;
use fold_core::parse::{Kind, TaskState};
use fold_core::tree::NRef;
use fold_core::vault::{NodeKey, Vault};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span as TSpan};
use ratatui::Terminal;
use std::path::Path;
use std::time::{Duration, Instant};

mod action;
mod editor;
mod grammars;
mod highlight;
mod markdown;
mod mouse;
mod ui;

pub use action::Action;
pub use editor::Keys as EditKeys;

/// Where an action sits in the node menu (for tests and scripted clicks).
pub fn node_menu_index(a: Action) -> usize {
    action::NODE_MENU.iter().position(|i| *i == Some(a)).expect("in the node menu")
}
pub use ui::Hit;

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
    // snapshot taken when a verb started, settled into `undo` after it
    pending_undo: Option<ops::Snapshot>,
    filter: String,
    filter_rows: Vec<NRef>,
    palette: String,
    scroll_reading: usize,
    quit: bool,
    // edit mode (§10.6)
    editor: Option<editor::Editor>,
    /// The editor's keymap (normal, Vim, Helix), and its clipboard, kept
    /// across editing sessions.
    edit_keys: editor::Keys,
    edit_clip: editor::Clip,
    edit_last_key: Instant,
    // property editor (§10.6)
    props_target: Option<NRef>,
    props_rows: Vec<(String, String, bool)>,
    props_sel: usize,
    // text prompt (refile destination, capture text)
    prompt: Option<Prompt>,
    // reading pane (§10.4)
    read_cursor: usize,
    read_search: String,
    read_matches: Vec<usize>,
    read_match_idx: usize,
    // conflict view (§10.7)
    conflict_idx: usize,
    // watcher (§11.2)
    watcher: Option<notify::RecommendedWatcher>,
    watch_rx: Option<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
    last_watch_event: Instant,
    self_write_until: Instant,
    pending_reload: bool,
    // layout and pointer state (§10.1): panes, scroll offsets, the hit map
    pane_outline: Rect,
    pane_reading: Rect,
    outline_scroll: usize,
    ui: ui::Ui,
    // the node a menu or button acts on, when it is not the cursor's
    action_target: Option<NRef>,
    palette_sel: usize,
    filter_sel: usize,
    // two-key sequences: `z…`, `gg`, `[[` / `]]`
    pending: Option<char>,
    // the node the reading cursor belongs to; a new target resets it
    read_key: Option<NodeKey>,
    // whether the reading pane shows the property header as line 1
    read_header: bool,
}

#[derive(Clone)]
struct Prompt {
    label: String,
    text: String,
    action: PromptAction,
    /// Candidate nodes for a target prompt (move to, go to), best first;
    /// clicked or picked with ↑/↓ and Enter.
    picks: Vec<NRef>,
    sel: usize,
}

#[derive(Clone)]
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
            status: "click to select · double-click to zoom · right-click for actions · drag to move · ? help".into(),
            status_time: Instant::now(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending_undo: None,
            filter: String::new(),
            filter_rows: Vec::new(),
            palette: String::new(),
            scroll_reading: 0,
            quit: false,
            editor: None,
            edit_keys: std::env::var("FOLD_KEYS")
                .ok()
                .and_then(|k| editor::Keys::parse(&k))
                .unwrap_or_default(),
            edit_clip: editor::Clip::default(),
            edit_last_key: Instant::now(),
            props_target: None,
            props_rows: Vec::new(),
            props_sel: 0,
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
            pane_outline: Rect::default(),
            pane_reading: Rect::default(),
            outline_scroll: 0,
            ui: ui::Ui::default(),
            action_target: None,
            palette_sel: 0,
            filter_sel: 0,
            pending: None,
            read_key: None,
            read_header: false,
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

    /// Reload after an external change (§11.2): editor saves first, then
    /// re-parse, cursor re-attached by id, key, deepest surviving step. A new
    /// sync-conflict file starts the merge flow (§12).
    pub fn reload_external(&mut self) {
        if self.editor.is_some() {
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
        let Some(r) = self.subject() else { return };
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
        let Some(r) = self.subject() else { return };
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
        let Some(r) = self.subject() else { return };
        self.push_undo("act_make_block");
        match ops::make_block(&mut self.vault, r) {
            Ok(id) => {
                self.say(format!("block {}", id));
                // the cursor stays on the node, now a block
                if let Some(nr) = self.vault.tree.block_by_id(&id) {
                    self.move_cursor_to(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_delete(&mut self) {
        let Some(r) = self.subject() else { return };
        self.push_undo("act_delete");
        self.register = ops::yank(&self.vault, r);
        match ops::delete_subtree(&mut self.vault, r) {
            Ok(m) => self.refresh_after(&m),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_yank(&mut self) {
        let Some(r) = self.subject() else { return };
        self.register = ops::yank(&self.vault, r);
        self.say("yanked");
    }

    fn act_paste(&mut self, after: bool) {
        if self.register.is_empty() {
            self.say("register empty");
            return;
        }
        let Some(r) = self.subject() else { return };
        self.push_undo("act_paste");
        let text = self.register.clone();
        match ops::paste(&mut self.vault, r, &text, after) {
            Ok(moved) => self.refresh_after(&with_rule_note("pasted", moved)),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_move(&mut self, down: bool) {
        let Some(r) = self.subject() else { return };
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
        let Some(r) = self.subject() else { return };
        self.push_undo("act_spelling");
        let key = self.vault.key_of(r);
        match ops::toggle_spelling(&mut self.vault, r) {
            Ok(moved) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
                if moved {
                    self.say(with_rule_note("respelled", true));
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_demote(&mut self) {
        let Some(r) = self.subject() else { return };
        self.push_undo("act_demote");
        let key = self.vault.key_of(r);
        match ops::demote(&mut self.vault, r) {
            Ok(moved) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
                if moved {
                    self.say(with_rule_note("demoted", true));
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_promote(&mut self) {
        let Some(r) = self.subject() else { return };
        self.push_undo("act_promote");
        let key = self.vault.key_of(r);
        match ops::promote(&mut self.vault, r) {
            Ok(moved) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
                if moved {
                    self.say(with_rule_note("promoted", true));
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_archive(&mut self) {
        let Some(r) = self.subject() else { return };
        self.push_undo("act_archive");
        match ops::archive(&mut self.vault, r) {
            Ok(moved) => self.refresh_after(&with_rule_note("archived", moved)),
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

    fn refile_to(&mut self, dest: NRef) {
        let Some(r) = self.subject() else { return };
        self.push_undo("move to");
        let key = self.vault.key_of(r);
        match ops::refile(&mut self.vault, r, dest) {
            Ok(moved) => {
                self.refresh_after(&with_rule_note("moved", moved));
                if let Some(nr) = self.moved_node(&key, dest) {
                    self.reveal(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    /// Where a moved node landed: by its key if it survived, else the
    /// destination's child with its title.
    fn moved_node(&self, key: &NodeKey, dest: NRef) -> Option<NRef> {
        let title = match key {
            NodeKey::Path { steps, .. } => steps.last().map(|(t, _)| t.clone()),
            NodeKey::Id(id) => return self.vault.tree.block_by_id(id),
            NodeKey::Root => None,
        }?;
        let dest = self.vault.find_by_key(&self.vault.key_of(dest)).unwrap_or(dest);
        self.vault
            .tree
            .resolved_children(dest)
            .into_iter()
            .rev()
            .find(|&c| self.vault.tree.node(c).title == title)
    }

    /// The node an action applies to: a menu's target, else the cursor's.
    fn subject(&self) -> Option<NRef> {
        self.action_target.or_else(|| self.current())
    }

    fn act_new_node(&mut self, child: bool) {
        let Some(r) = self.subject() else { return };
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
                Ok(_) => {
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
        let Some(r) = self.subject() else { return };
        let target = self.vault.tree.resolved_child(r);
        self.open_editor_on(target);
    }

    /// Open the built-in editor over a node's subtree (§10.6).
    fn open_editor_on(&mut self, target: NRef) {
        let buf = fold_core::edit::open_editor(&self.vault, target);
        self.editor = Some(editor::Editor::new(buf, self.edit_keys, self.edit_clip.clone()));
        self.mode = Mode::Edit;
        self.focus = Focus::Reading;
        self.edit_last_key = Instant::now();
    }

    /// The editor's keymap; switching applies to an open editor at once.
    pub fn set_edit_keys(&mut self, keys: editor::Keys) {
        self.edit_keys = keys;
        if let Some(ed) = self.editor.as_mut() {
            ed.set_keys(keys);
        }
        self.say(format!("editor keys: {}", keys.name()));
    }

    fn save_editor(&mut self, why: &str) {
        let Some(mut ed) = self.editor.take() else { return };
        self.settle_undo();
        let snap = ops::Snapshot::take(&self.vault, "edit");
        match ed.buf.save_all(&mut self.vault) {
            Ok(n) => {
                self.record_undo(snap);
                if n > 0 {
                    self.say(format!("saved {} block(s) ({})", n, why));
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
        self.editor = Some(ed);
        self.edit_last_key = Instant::now();
    }

    fn close_editor(&mut self) {
        self.save_editor("exit");
        if let Some(ed) = self.editor.take() {
            self.edit_clip = ed.clip;
        }
        self.mode = Mode::Normal;
    }

    /// Whether the editor holds changes not yet written.
    pub fn editor_dirty(&self) -> bool {
        self.editor.as_ref().map(|e| !e.buf.dirty.is_empty()).unwrap_or(false)
    }

    pub fn key_edit(&mut self, key: KeyEvent) {
        let Some(ed) = self.editor.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let before = ed.buf.owner_at(ed.cursor.line);
        let out = ed.handle(key);
        let after = ed.buf.owner_at(ed.cursor.line);
        self.edit_last_key = Instant::now();
        if out.revert {
            self.discard_editor();
            return;
        }
        if out.save {
            self.save_editor("save");
        }
        if out.close {
            self.close_editor();
            return;
        }
        // moving out of a dirty block saves it (§10.6)
        if before != after && self.editor.as_ref().is_some_and(|e| e.buf.dirty.contains(&before)) {
            self.settle_undo();
            let snap = ops::Snapshot::take(&self.vault, "edit");
            let mut ed = self.editor.take().unwrap();
            if ed.buf.splice(&mut self.vault, before).is_ok() {
                self.record_undo(snap);
            }
            self.editor = Some(ed);
        }
    }

    /// Text pasted into the terminal (bracketed paste): into the editor or
    /// the open prompt.
    pub fn handle_paste(&mut self, text: &str) {
        if let Some(p) = self.prompt.as_mut() {
            p.text.push_str(text.lines().next().unwrap_or(""));
            self.refresh_picks();
        } else if let Some(ed) = self.editor.as_mut() {
            ed.paste_text(text);
            self.edit_last_key = Instant::now();
        } else if self.mode == Mode::Filter {
            self.filter.push_str(text.lines().next().unwrap_or(""));
            self.update_filter();
        }
    }

    /// The cursor shape the terminal should show: a block in Vim and Helix
    /// normal modes, a bar while typing.
    pub fn cursor_block(&self) -> bool {
        self.editor.as_ref().map(|e| e.block_cursor()).unwrap_or(false)
    }

    /// Text copied in the editor since the last call, for the system
    /// clipboard.
    pub fn take_copied(&mut self) -> Option<String> {
        self.editor.as_mut().and_then(|e| e.copied.take())
    }

    // -------------------------------------------------------- props (§10.6)

    fn act_props(&mut self) {
        if let Some(r) = self.subject() {
            self.open_props(r);
        }
    }

    /// The property form (§10.6) over a node, or the block it embeds.
    fn open_props(&mut self, r: NRef) {
        let target = self.vault.tree.resolved_child(r);
        self.props_target = Some(target);
        self.props_rows = if self.vault.tree.node(target).is_block() {
            ops::frontmatter_lines(&self.vault, target.0)
                .into_iter()
                .filter(|(k, _, _)| k != "id")
                .collect()
        } else {
            Vec::new()
        };
        self.props_sel = self.props_sel.min(self.props_rows.len().saturating_sub(1));
        self.mode = Mode::Props;
    }

    fn reopen_props(&mut self) {
        if let Some(t) = self.props_target {
            self.open_props(t);
        }
    }

    fn delete_prop(&mut self, i: usize) {
        let Some((k, _, editable)) = self.props_rows.get(i).cloned() else { return };
        let Some(t) = self.props_target else { return };
        if !editable {
            self.say("read-only line (preserved verbatim)");
            return;
        }
        self.push_undo("delete property");
        match ops::set_frontmatter_key(&mut self.vault, t.0, &k, None) {
            Ok(()) => self.say(format!("{} removed", k)),
            Err(e) => self.say(format!("error: {}", e)),
        }
        self.reopen_props();
    }

    pub fn key_props(&mut self, key: KeyEvent) {
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
                self.open_prompt("new property", PromptAction::PropNew, String::new());
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some((k, v, editable)) = self.props_rows.get(self.props_sel).cloned() {
                    if editable {
                        self.open_prompt(&k, PromptAction::PropSet(k.clone()), v);
                    } else {
                        self.say("read-only line (preserved verbatim)");
                    }
                }
            }
            KeyCode::Char('d') => self.delete_prop(self.props_sel),
            _ => {}
        }
    }

    pub fn key_prompt(&mut self, key: KeyEvent) {
        self.refresh_picks();
        let Some(mut p) = self.prompt.take() else { return };
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Up => {
                p.sel = p.sel.saturating_sub(1);
                self.prompt = Some(p);
            }
            KeyCode::Down => {
                if p.sel + 1 < p.picks.len() {
                    p.sel += 1;
                }
                self.prompt = Some(p);
            }
            KeyCode::Enter => self.accept_prompt(p),
            KeyCode::Backspace => {
                p.text.pop();
                self.prompt = Some(p);
                self.refresh_picks();
            }
            KeyCode::Char(c) => {
                p.text.push(c);
                self.prompt = Some(p);
                self.refresh_picks();
            }
            _ => {
                self.prompt = Some(p);
            }
        }
    }

    /// Accept a prompt: a target prompt takes its highlighted candidate, or
    /// resolves the typed text as an id, path or title (§3.4).
    fn accept_prompt(&mut self, p: Prompt) {
        let picked = p.picks.get(p.sel).copied();
        match p.action {
            PromptAction::Refile => match picked.map(Ok).unwrap_or_else(|| self.vault.resolve_target(&p.text)) {
                Ok(dest) => self.refile_to(dest),
                Err(e) => self.say(e),
            },
            PromptAction::GoTo => match picked.map(Ok).unwrap_or_else(|| self.vault.resolve_target(&p.text)) {
                Ok(r) => self.reveal(r),
                Err(e) => self.say(e),
            },
            PromptAction::CaptureText(task) => {
                self.push_undo("capture");
                match ops::capture(&mut self.vault, &p.text, task) {
                    Ok(r) => {
                        self.reveal(r);
                        self.say("captured");
                    }
                    Err(e) => self.say(format!("error: {}", e)),
                }
            }
            PromptAction::PropSet(ref k) => {
                let k = k.clone();
                if let Some(t) = self.props_target {
                    // validate dates (§8.4)
                    if (k == "due" || k == "done") && !fold_core::check::is_iso_date(&p.text) {
                        self.say(format!("{}: must be YYYY-MM-DD", k));
                        self.prompt = Some(p);
                        return;
                    }
                    self.push_undo("set property");
                    match ops::set_property(&mut self.vault, t, &k, &p.text) {
                        Ok(block) => {
                            self.say(format!("{} set", k));
                            self.props_target = Some(block);
                            self.reopen_props();
                        }
                        Err(e) => self.say(format!("error: {}", e)),
                    }
                }
            }
            PromptAction::PropNew => {
                if self.props_target.is_some() {
                    let key_name = p.text.trim().to_string();
                    if fold_core::parse::is_valid_key(&key_name) {
                        self.open_prompt(&key_name, PromptAction::PropSet(key_name.clone()), String::new());
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

    fn open_prompt(&mut self, label: &str, action: PromptAction, text: String) {
        self.prompt = Some(Prompt {
            label: label.into(),
            text,
            action,
            picks: Vec::new(),
            sel: 0,
        });
        self.refresh_picks();
    }

    /// Keep a target prompt's candidates in step with its text: nodes whose
    /// path matches, best first — title prefix, then title, then path.
    fn refresh_picks(&mut self) {
        let Some(p) = self.prompt.as_ref() else { return };
        if !matches!(p.action, PromptAction::Refile | PromptAction::GoTo) {
            return;
        }
        let q = p.text.to_lowercase();
        // a node cannot move into its own subtree
        let moving = match p.action {
            PromptAction::Refile => self.subject().map(|r| self.vault.tree.resolved_child(r)),
            _ => None,
        };
        let mut nodes: Vec<NRef> = Vec::new();
        self.vault.tree.walk(self.vault.tree.root, &mut |t, r| {
            if t.node(r).kind != Kind::Root && !t.node(r).is_embed() {
                nodes.push(r);
            }
        });
        let mut scored: Vec<(u8, usize, NRef)> = Vec::new();
        for r in nodes {
            let chain = self.chain(r);
            if moving.map(|m| chain.contains(&m)).unwrap_or(false) {
                continue;
            }
            let title = self.vault.tree.node(r).title.to_lowercase();
            let path = self.path_titles(r).join(" › ").to_lowercase();
            let score = if q.is_empty() || title.starts_with(&q) {
                0
            } else if title.contains(&q) {
                1
            } else if fuzzy_match(&q, &path) {
                2
            } else {
                continue;
            };
            scored.push((score, path.len(), r));
        }
        scored.sort_by_key(|&(s, len, _)| (s, len));
        let picks: Vec<NRef> = scored.into_iter().take(200).map(|(_, _, r)| r).collect();
        if let Some(p) = self.prompt.as_mut() {
            p.sel = p.sel.min(picks.len().saturating_sub(1));
            p.picks = picks;
        }
    }

    /// Put the cursor on a node, unfolding and zooming out as needed.
    fn reveal(&mut self, r: NRef) {
        for a in self.vault.tree.ancestors(r) {
            let k = self.vault.key_of(a);
            self.folded.retain(|f| f != &k);
        }
        if self.zoom_root.is_some() && !self.rows().iter().any(|row| row.nref == r) {
            self.zoom_root = None;
        }
        self.move_cursor_to(r);
        self.focus = Focus::Outline;
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
                    self.open_editor_on(ours);
                }
            }
            _ => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.handle_key_inner(key);
        self.settle_undo();
    }

    fn handle_key_inner(&mut self, key: KeyEvent) {
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
        if self.ui.menu.is_some() {
            self.key_menu(key);
            return;
        }
        if self.prompt.is_some() {
            self.key_prompt(key);
            return;
        }
        match self.mode {
            Mode::Normal => {
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
            // the keymap decides what Ctrl-c means (copy, or Vim's escape)
            Mode::Edit => self.key_edit(key),
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

    /// Drop the editor's unsaved changes (§10.6: *Revert*, `:q!`).
    fn discard_editor(&mut self) {
        if let Some(ed) = self.editor.take() {
            self.edit_clip = ed.clip;
        }
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
            KeyCode::Enter => self.run_action(Action::Zoom),
            KeyCode::Backspace => self.zoom_out(),
            KeyCode::Char('m') => self.run_action(Action::NodeMenu),
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
            KeyCode::Char('c') => self.run_action(Action::Capture),
            KeyCode::Char('C') => self.run_action(Action::CaptureTask),
            KeyCode::Char('/') => self.run_action(Action::Filter),
            KeyCode::Char(':') => self.run_action(Action::Palette),
            KeyCode::Char('?') => self.run_action(Action::Help),
            KeyCode::Char('u') => self.act_undo(),
            KeyCode::Char('U') => self.act_redo(),
            KeyCode::Char('e') => self.act_edit(),
            KeyCode::Char('a') => self.act_props(),
            KeyCode::Char('r') => self.run_action(Action::Refile),
            _ => {}
        }
    }

    fn act_undo(&mut self) {
        self.settle_undo();
        let Some(inv) = self.undo.pop() else {
            self.say("nothing to undo");
            return;
        };
        self.mark_self_write();
        match inv.undo(&mut self.vault) {
            Ok(()) => {
                self.say(format!("undone: {}", inv.description));
                self.redo.push(inv);
                self.clamp_cursor();
            }
            Err(e) => {
                // refused: an external change since; the entry stays
                self.say(format!("undo refused: {}", e));
                self.undo.push(inv);
            }
        }
    }

    fn act_redo(&mut self) {
        self.settle_undo();
        let Some(inv) = self.redo.pop() else {
            self.say("nothing to redo");
            return;
        };
        self.mark_self_write();
        match inv.redo(&mut self.vault) {
            Ok(()) => {
                self.say(format!("redone: {}", inv.description));
                self.undo.push(inv);
                self.clamp_cursor();
            }
            Err(e) => {
                self.say(format!("redo refused: {}", e));
                self.redo.push(inv);
            }
        }
    }

    /// Start an op-log entry: remember the files as they are before a verb
    /// runs. The entry is settled after the key or click that ran it.
    fn push_undo(&mut self, desc: &str) {
        self.settle_undo();
        self.pending_undo = Some(ops::Snapshot::take(&self.vault, desc));
        self.mark_self_write();
    }

    /// Turn the pending snapshot into an op-log entry holding exactly the
    /// files the verb changed (§10.10); a verb that changed nothing leaves
    /// no entry and keeps the redo stack.
    fn settle_undo(&mut self) {
        if let Some(snap) = self.pending_undo.take() {
            self.record_undo(snap);
        }
    }

    fn record_undo(&mut self, snap: ops::Snapshot) {
        if let Some(inv) = ops::Inverse::since(snap, &self.vault) {
            self.undo.push(inv);
            self.redo.clear();
            self.mark_self_write();
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
                    let target = self.vault.tree.resolved_child(r);
                    self.open_editor_on(target);
                }
            }
            KeyCode::Char('a') => {
                if let Some(r) = self.read_node() {
                    self.open_props(r);
                }
            }
            KeyCode::Char('o') => self.open_link_under_cursor(&doc),
            KeyCode::Char('/') => self.open_prompt("search", PromptAction::ReadSearch, String::new()),
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
            KeyCode::Esc => self.close_top(),
            KeyCode::Up => self.filter_sel = self.filter_sel.saturating_sub(1),
            KeyCode::Down => {
                if self.filter_sel + 1 < self.filter_rows.len() {
                    self.filter_sel += 1;
                }
            }
            KeyCode::Enter => self.pick_filter(self.filter_sel),
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

    /// Go to a filter hit (§10.5): unfold its ancestors and select it.
    fn pick_filter(&mut self, i: usize) {
        let hit = self.filter_rows.get(i).copied();
        self.close_top();
        if let Some(r) = hit {
            self.reveal(r);
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
        self.filter_sel = 0;
    }

    /// Actions in the palette matching its query (§10.8).
    fn palette_hits(&self) -> Vec<Action> {
        let q = self.palette.to_lowercase();
        // label matches first, then description, then loose subsequences
        let mut hits: Vec<(u8, Action)> = action::PALETTE
            .iter()
            .filter_map(|&a| {
                let (label, desc) = (a.label().to_lowercase(), a.desc().to_lowercase());
                let rank = if q.is_empty() || label.starts_with(&q) {
                    0
                } else if label.contains(&q) {
                    1
                } else if desc.contains(&q) {
                    2
                } else if fuzzy_match(&q, &label) {
                    3
                } else if fuzzy_match(&q, &desc) {
                    4
                } else {
                    return None;
                };
                Some((rank, a))
            })
            .collect();
        hits.sort_by_key(|&(rank, _)| rank);
        hits.into_iter().map(|(_, a)| a).collect()
    }

    pub fn key_palette(&mut self, key: KeyEvent) {
        let hits = self.palette_hits();
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.palette.clear();
            }
            KeyCode::Up => self.palette_sel = self.palette_sel.saturating_sub(1),
            KeyCode::Down => {
                if self.palette_sel + 1 < hits.len() {
                    self.palette_sel += 1;
                }
            }
            KeyCode::Enter => {
                let chosen = hits.get(self.palette_sel).copied();
                self.mode = Mode::Normal;
                self.palette.clear();
                self.palette_sel = 0;
                if let Some(a) = chosen {
                    self.run_action(a);
                }
            }
            KeyCode::Backspace => {
                self.palette.pop();
                self.palette_sel = 0;
            }
            KeyCode::Char(c) => {
                self.palette.push(c);
                self.palette_sel = 0;
            }
            _ => {}
        }
    }

    /// Open the node menu (§10.3) for `r`, anchored at a screen position.
    fn open_menu(&mut self, r: NRef, x: u16, y: u16) {
        self.ui.menu = Some(ui::Menu {
            target: r,
            x,
            y,
            sel: 0,
        });
    }

    fn key_menu(&mut self, key: KeyEvent) {
        let Some(menu) = self.ui.menu.as_mut() else { return };
        let items: Vec<usize> = (0..action::NODE_MENU.len())
            .filter(|&i| action::NODE_MENU[i].is_some())
            .collect();
        let pos = items.iter().position(|&i| i == menu.sel).unwrap_or(0);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('m') => self.ui.menu = None,
            KeyCode::Up | KeyCode::Char('k') => menu.sel = items[pos.saturating_sub(1)],
            KeyCode::Down | KeyCode::Char('j') => menu.sel = items[(pos + 1).min(items.len() - 1)],
            KeyCode::Enter => {
                let i = menu.sel;
                self.run_menu_item(i);
            }
            _ => {}
        }
    }

    /// Run a node-menu item on the menu's target.
    fn run_menu_item(&mut self, i: usize) {
        let Some(menu) = self.ui.menu.take() else { return };
        let Some(Some(a)) = action::NODE_MENU.get(i) else { return };
        self.action_target = Some(menu.target);
        self.run_action(*a);
        self.action_target = None;
    }

    /// Run any action by name (§10.8): what buttons, menus and the palette
    /// do. Node actions apply to `subject()`.
    pub fn run_action(&mut self, a: Action) {
        // a node action from a menu first selects its node
        if let Some(t) = self.action_target {
            if self.rows().iter().any(|row| row.nref == t) {
                self.move_cursor_to(t);
                self.action_target = None;
            }
        }
        match a {
            Action::Edit => self.act_edit(),
            Action::Zoom => {
                if let Some(r) = self.subject() {
                    self.zoom_into(r);
                }
            }
            Action::NewSibling => self.act_new_node(false),
            Action::NewChild => self.act_new_node(true),
            Action::ToggleDone => self.act_toggle_task(),
            Action::ToggleTask => self.act_toggle_taskness(),
            Action::Props => self.act_props(),
            Action::MakeBlock => self.act_make_block(),
            Action::MoveUp => self.act_move(false),
            Action::MoveDown => self.act_move(true),
            Action::Indent => self.act_demote(),
            Action::Outdent => self.act_promote(),
            Action::Respell => self.act_spelling(),
            Action::Refile => self.open_prompt("move to", PromptAction::Refile, String::new()),
            Action::Archive => self.act_archive(),
            Action::Copy => self.act_yank(),
            Action::Delete => self.act_delete(),
            Action::PasteAfter => self.act_paste(true),
            Action::PasteBefore => self.act_paste(false),
            Action::NodeMenu => {
                if let Some(r) = self.subject() {
                    let (x, y) = self.ui.anchor_for_cursor(self.pane_outline, self.cursor, self.outline_scroll);
                    self.open_menu(r, x, y);
                }
            }
            Action::ZoomOut => self.zoom_out(),
            Action::Filter => {
                self.mode = Mode::Filter;
                self.filter.clear();
                self.filter_rows.clear();
                self.filter_sel = 0;
            }
            Action::Palette => {
                self.mode = Mode::Picker;
                self.palette.clear();
                self.palette_sel = 0;
            }
            Action::Help => self.mode = Mode::Help,
            Action::Quit => self.quit = true,
            Action::Undo => self.act_undo(),
            Action::Redo => self.act_redo(),
            Action::Capture => self.open_prompt("capture", PromptAction::CaptureText(false), String::new()),
            Action::CaptureTask => {
                self.open_prompt("capture task", PromptAction::CaptureText(true), String::new())
            }
            Action::HideDone => {
                self.hide_done = !self.hide_done;
                self.clamp_cursor();
                self.say(if self.hide_done { "done hidden" } else { "done shown" });
            }
            Action::RawMode => {
                self.raw_mode = !self.raw_mode;
                self.say(if self.raw_mode { "raw" } else { "styled" });
            }
            Action::GoTo => self.open_prompt("go to", PromptAction::GoTo, String::new()),
            Action::ClearDone => self.act_clear_done(),
            Action::Canonicalize => match fold_core::check::fix(&mut self.vault) {
                Ok(n) => self.say(format!("{} file(s) canonicalized", n)),
                Err(e) => self.say(format!("error: {}", e)),
            },
            Action::Merge => match fold_core::merge::merge_sync_conflicts(&mut self.vault, false) {
                Ok(o) => {
                    self.say(format!("{} merge(s)", o.len()));
                    if !fold_core::merge::conflict_pairs(&self.vault).is_empty() {
                        self.enter_conflict_view();
                    }
                }
                Err(e) => self.say(format!("error: {}", e)),
            },
            Action::ResolveConflicts => self.enter_conflict_view(),
            Action::EditorKeys => self.set_edit_keys(self.edit_keys.next()),
            Action::EditDone => self.close_editor(),
            Action::EditRevert => self.discard_editor(),
            Action::Close => self.close_top(),
            Action::PromptOk => {
                if let Some(p) = self.prompt.take() {
                    self.accept_prompt(p);
                }
            }
            Action::PropAdd => self.open_prompt("new property", PromptAction::PropNew, String::new()),
            Action::ConflictOurs => self.key_conflict(key_of('o')),
            Action::ConflictTheirs => self.key_conflict(key_of('t')),
            Action::ConflictBoth => self.key_conflict(key_of('b')),
            Action::ConflictEdit => self.key_conflict(key_of('e')),
            Action::ConflictPrev => self.key_conflict(key_of('N')),
            Action::ConflictNext => self.key_conflict(key_of('n')),
        }
    }

    /// Close whatever is on top: a menu, a prompt, then a popup mode.
    fn close_top(&mut self) {
        if self.ui.menu.take().is_some() {
            return;
        }
        if self.prompt.take().is_some() {
            return;
        }
        match self.mode {
            Mode::Edit => self.close_editor(),
            Mode::Filter => {
                self.mode = Mode::Normal;
                self.filter.clear();
                self.filter_rows.clear();
            }
            Mode::Picker => {
                self.mode = Mode::Normal;
                self.palette.clear();
            }
            Mode::Props | Mode::Help | Mode::Conflict => self.mode = Mode::Normal,
            Mode::Normal => {}
        }
    }

    /// Zoom into a node: the reading pane shows it (§10.3 `Enter`).
    fn zoom_into(&mut self, r: NRef) {
        if self.vault.tree.node(r).kind != Kind::Root {
            self.zoom_root = Some(r);
            self.cursor = 0;
            self.read_cursor = 0;
            self.scroll_reading = 0;
            self.focus = Focus::Reading;
        }
    }

    fn zoom_out(&mut self) {
        if let Some(z) = self.zoom_root {
            let key = self.vault.key_of(z);
            self.zoom_root = self.vault.tree.node(z).parent.map(|p| (z.0, p)).filter(|&p| {
                self.vault.tree.node(p).kind != Kind::Root
            });
            self.cursor = 0;
            if let Some(r) = self.vault.find_by_key(&key) {
                self.move_cursor_to(r);
            }
        }
    }

    /// Zoom straight to a node, or to the root (breadcrumbs, §10.1).
    fn zoom_to(&mut self, r: Option<NRef>) {
        let prev = self.zoom_root;
        self.zoom_root = r;
        self.cursor = 0;
        if let Some(p) = prev {
            if !self.rows().iter().any(|row| row.nref == p) {
                return;
            }
            self.move_cursor_to(p);
        }
    }

}

/// A status message, noting when the ordering rule placed the node other
/// than asked (§3.1: items before sections).
fn with_rule_note(what: &str, moved: bool) -> String {
    if moved {
        format!("{} — placed at the item/section boundary (§3.1)", what)
    } else {
        what.to_string()
    }
}

/// The help text, shared by `?` in the TUI and `fold help` (§10.8).
pub fn help_text() -> Vec<Line<'static>> {
    let entries: &[(&str, &str)] = &[
        ("", "fold — notes and tasks as one outline of plain Markdown."),
        ("", ""),
        ("MOUSE", ""),
        ("click", "select · ▸/▾ fold · ☐ toggle · breadcrumb segments zoom out"),
        ("double-click", "a row zooms in · in the text: zoom a heading, follow a"),
        ("", "  block, or edit that very line"),
        ("right-click", "or ⋯ on a row — every action on that node"),
        ("drag", "a row onto a title to nest it, left of a title to put it before"),
        ("wheel", "scroll the pane under the pointer · drag the divider to resize"),
        ("top bar", "⌕ Filter  + Capture  ↶ Undo  ↷ Redo  ☰ Commands  ? Help"),
        ("", ""),
        ("KEYS", ""),
        ("j/k h/l", "move · fold/unfold · Enter zoom in · Backspace zoom out"),
        ("e a m", "edit the Markdown · properties · node menu"),
        ("n/N x t", "new sibling / child · done · task on/off"),
        ("J/K > < ~", "move · indent · outdent · heading ↔ bullet"),
        ("r y d p/P", "move to… · copy · delete · paste after / before"),
        ("s za zd zr", "make block · archive · hide done · raw source"),
        ("/ : ?", "filter · command palette · this help"),
        ("u/U q", "undo / redo · quit (everything is always saved)"),
        ("Tab", "switch panes · in the text: [[ ]] headings, / search, o link"),
        ("", ""),
        ("EDITOR", ""),
        ("keymaps", "normal (like micro) · vim · helix — ⌨ in the editor's border,"),
        ("", "  ☰ Editor keys, --keys or $FOLD_KEYS"),
        ("normal", "type · Shift+arrows select · ^C ^X ^V ^Z ^Y · ^S save · ^F find"),
        ("", "  ^K cut line · ^D duplicate · Alt-↑/↓ move line · Esc done"),
        ("vim/helix", "the usual modes, motions and operators · :w :q :wq :q!"),
        ("mouse", "click places the cursor · drag selects · double-click a word"),
        ("", ""),
        ("", "Every action is also in ☰ Commands and in the node menu."),
    ];
    entries
        .iter()
        .map(|(k, v)| {
            if v.is_empty() && !k.is_empty() {
                Line::from(TSpan::styled(
                    k.to_string(),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(vec![
                    TSpan::styled(format!("{:<13}", k), Style::default().add_modifier(Modifier::BOLD)),
                    TSpan::raw(v.to_string()),
                ])
            }
        })
        .collect()
}

fn key_of(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
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

// ------------------------------------------------------------ main loop

/// Run the TUI on a vault. `keys` picks the editor's keymap (`normal`,
/// `vim`, `helix`), overriding `$FOLD_KEYS`.
pub fn run(dir: &Path, keys: Option<&str>) -> anyhow::Result<()> {
    let mut app = App::new(dir)?;
    if let Some(k) = keys {
        let parsed = editor::Keys::parse(k)
            .ok_or_else(|| anyhow::anyhow!("unknown keymap {:?}: use normal, vim or helix", k))?;
        app.edit_keys = parsed;
    }
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
    std::io::stdout().execute(EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let res = run_loop(&mut terminal, &mut app);
    disable_raw_mode()?;
    std::io::stdout().execute(DisableBracketedPaste)?;
    std::io::stdout().execute(DisableMouseCapture)?;
    std::io::stdout().execute(SetCursorStyle::DefaultUserShape)?;
    std::io::stdout().execute(LeaveAlternateScreen)?;
    res
}

/// Standard base64, for OSC 52.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    let mut block = None;
    loop {
        terminal.draw(|f| app.draw(f))?;
        // the cursor's shape follows the editor's mode
        let want = app.cursor_block();
        if block != Some(want) {
            let style = if want { SetCursorStyle::SteadyBlock } else { SetCursorStyle::DefaultUserShape };
            std::io::stdout().execute(style)?;
            block = Some(want);
        }
        // copied text reaches the system clipboard (OSC 52)
        if let Some(t) = app.take_copied() {
            use std::io::Write;
            let mut out = std::io::stdout();
            write!(out, "\x1b]52;c;{}\x07", base64(t.as_bytes()))?;
            out.flush()?;
        }
        if app.quit {
            // save any open editor on quit (§10.6)
            if app.editor.is_some() {
                app.close_editor();
            }
            return Ok(());
        }
        // autosave after 750 ms without a keystroke (§10.6)
        if app.mode == Mode::Edit && app.editor_dirty() && app.edit_last_key.elapsed() > Duration::from_millis(750) {
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
                Event::Paste(text) => app.handle_paste(&text),
                _ => {}
            }
        }
    }
}
