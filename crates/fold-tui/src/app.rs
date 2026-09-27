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
use fold_core::ident::Id;
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
mod view;
mod wrap;

pub use action::Action;
pub use editor::Keys as EditKeys;

/// Where an action sits in the node menu (for tests and scripted clicks).
pub fn node_menu_index(a: Action) -> usize {
    action::NODE_MENU
        .iter()
        .chain(action::CONFLICT_MENU)
        .position(|i| *i == Some(a))
        .expect("in the node menu")
}
pub use ui::Hit;
pub use view::View;

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

/// The status bar's greeting, and a shorter one where it doesn't fit.
pub(crate) const HINT: &str = "double-click to zoom · right-click for actions · drag to move · ? help";
pub(crate) const HINT_SHORT: &str = "right-click for actions · ? help";

/// How long a message stays up once something else was done (§10.1).
const MESSAGE_LIFE: Duration = Duration::from_secs(5);

pub struct App {
    pub(crate) vault: Vault,
    mode: Mode,
    focus: Focus,
    /// The zoom root (§10.3). While a verb runs it may be stale: read it
    /// through `zoom()`, and set it through `set_zoom()`.
    zoom_root: Option<NRef>,
    /// The zoom root's key while a verb or undo runs: the files it writes
    /// are re-parsed and their nodes renumbered, so until the verb settles
    /// the zoom is found by key (§11.2), not by its old index.
    zoom_anchor: Option<NodeKey>,
    pub cursor: usize,
    folded: Vec<NodeKey>,
    /// The conflict copies unfolded: a copy starts folded, a fold of this
    /// run's view alone, never remembered (§10.1).
    unfolded_copies: Vec<NodeKey>,
    hide_done: bool,
    raw_mode: bool,
    /// Long lines wrap in the reading pane and the editor (§10.1); `zw`.
    wrap: bool,
    /// The reading pane is shown beside the outline (§10.1); `zp`. Hidden,
    /// the outline takes the screen and the pane appears only to edit.
    pub show_reading: bool,
    register: String,
    /// The node the register holds, as the status bar names it, and its
    /// spelling, for what the ordering rule did with a paste (§3.1).
    copied: (String, Kind),
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
    /// Set when `--keys` or `$FOLD_KEYS` chose this run's keymap: the
    /// keymap the view goes on remembering instead (§10.6), the one it was
    /// loaded with, or none.
    kept_keys: Option<Option<editor::Keys>>,
    edit_clip: editor::Clip,
    edit_last_key: Instant,
    /// The pane that had focus when the editor opened, focused again after.
    edit_return: Focus,
    /// Where an outline verb moved the node the editor is open on: the
    /// editor is re-rendered there after the verb (`follow`).
    edit_moved: Option<NRef>,
    /// A save of the editor's text failed and none has gone through since:
    /// *Revert* keeps a copy of the text in the trash (§10.6).
    edit_refused: bool,
    /// The editor's text as it was when *Revert* could not copy it to the
    /// trash: *Revert* again on the same text drops it without one.
    edit_uncopied: Option<String>,
    /// The file of each block an editor save left in transit (§5.2): its
    /// title line cut, the block it was in written without its embed. With
    /// the op-log entry of that save, by the length of `undo` once it was
    /// in: the save that deletes the block puts the deletion there, and
    /// the one that writes its embed back makes the entries from there to
    /// its own one.
    edit_transit: Vec<(String, usize)>,
    /// Every editor save panics where it writes: for tests of what fold
    /// keeps when it crashes (`panic_in_saves`).
    save_panics: bool,
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
    /// When the view last opened on its own: for a moment it takes no side,
    /// a key on its way being meant for what was there before.
    conflict_opened: Option<Instant>,
    /// Pairs a merge raised while the user was busy, by conflict block:
    /// the view opens on them once the outline is at rest, and the ⚠ count
    /// is lit until it does.
    conflicts_waiting: Vec<NodeKey>,
    /// The last key, paste or click.
    last_input: Option<Instant>,
    // watcher (§11.2)
    watcher: Option<notify::RecommendedWatcher>,
    watch_rx: Option<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
    last_watch_event: Instant,
    self_write_until: Instant,
    pending_reload: bool,
    /// The sync-conflict copies the last reload listed before its merge,
    /// and still there after it: a new one is a change to take in (§11.2).
    conflict_copies: Vec<String>,
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
    /// What the status bar says of a verb held for a selection out of view
    /// (§10.1), until the next key or click.
    held_words: Option<String>,
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
    /// The node *Move to…* moves, fixed when the prompt opens: a menu's
    /// target is dropped once its item has run, long before the pick. A
    /// key, so a reload while the prompt is open cannot leave it stale.
    moving: Option<NodeKey>,
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
        let conflict_copies = vault.conflict_files()?;
        Ok(App {
            vault,
            mode: Mode::Normal,
            focus: Focus::Outline,
            zoom_root: None,
            zoom_anchor: None,
            cursor: 0,
            folded: Vec::new(),
            unfolded_copies: Vec::new(),
            hide_done: false,
            raw_mode: false,
            wrap: true,
            show_reading: false,
            register: String::new(),
            copied: (String::new(), Kind::Item),
            status: HINT.into(),
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
            kept_keys: None,
            edit_clip: editor::Clip::default(),
            edit_last_key: Instant::now(),
            edit_return: Focus::Outline,
            edit_moved: None,
            edit_refused: false,
            edit_uncopied: None,
            edit_transit: Vec::new(),
            save_panics: false,
            props_target: None,
            props_rows: Vec::new(),
            props_sel: 0,
            prompt: None,
            read_cursor: 0,
            read_search: String::new(),
            read_matches: Vec::new(),
            read_match_idx: 0,
            conflict_idx: 0,
            conflict_opened: None,
            conflicts_waiting: Vec::new(),
            last_input: None,
            watcher: None,
            watch_rx: None,
            last_watch_event: Instant::now(),
            self_write_until: Instant::now() - Duration::from_secs(1),
            pending_reload: false,
            conflict_copies,
            pane_outline: Rect::default(),
            pane_reading: Rect::default(),
            outline_scroll: 0,
            ui: ui::Ui::default(),
            action_target: None,
            palette_sel: 0,
            filter_sel: 0,
            pending: None,
            held_words: None,
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

    /// What the status bar says on its left (§10.1), and whether it is a
    /// hint, which the bar shortens by whole parts: while a key sequence is
    /// half typed, what can follow it; else the last message, until it is
    /// stale; then the keys of what is on screen. The greeting is the
    /// outline's: over anything else, that thing's keys.
    pub(crate) fn status_left(&self) -> (String, bool) {
        if let Some(p) = self.pending {
            let follows: Vec<String> = self.follows(p).iter().map(|(k, what)| format!("{} {}", k, what)).collect();
            return (format!("{}… {}", p, follows.join(" · ")), true);
        }
        let over = self.mode != Mode::Normal || self.prompt.is_some() || self.ui.menu.is_some();
        if self.stale() || (self.status == HINT && over) {
            return (self.keys_hint().to_string(), true);
        }
        (self.status.clone(), false)
    }

    /// Whether the message has had its time (§10.1): some 5 s old, with a
    /// key or click since. An error or a refusal waits for a key or click
    /// that came once it was that old: one pressed while reading it does
    /// not take it away.
    fn stale(&self) -> bool {
        let old = self.status_time + MESSAGE_LIFE;
        match self.last_input {
            _ if self.status.is_empty() => true,
            None => false,
            Some(t) if lasting(&self.status) => t >= old,
            Some(t) => t > self.status_time && Instant::now() >= old,
        }
    }

    /// The keys of what is on screen (§10.1): the topmost popup's, else the
    /// mode's; the editor's by its keymap and mode.
    fn keys_hint(&self) -> &'static str {
        if self.ui.menu.is_some() {
            return "↑↓ choose · Enter run · Esc close";
        }
        if let Some(p) = &self.prompt {
            return match p.action {
                PromptAction::Refile | PromptAction::GoTo => "↑↓ pick · Enter ok · Esc cancel",
                _ => "Enter ok · Esc cancel",
            };
        }
        match self.mode {
            Mode::Normal if self.focus == Focus::Reading => "e edit · Enter zoom/follow · Tab outline",
            Mode::Normal => "n new · e edit · x done · m menu · / find · ? help",
            // in Vim and Helix, Esc never leaves (§10.6); :wq, the way on,
            // is last, so it stays where the bar is short
            Mode::Edit => match self.editor.as_ref().map(|e| (e.keys, e.mode)) {
                Some((editor::Keys::Normal, _)) | None => "Esc done · Ctrl-S save · Ctrl-Z undo",
                Some((_, editor::Mode::Normal)) => "i insert · :q! revert · :wq done",
                Some(_) => "Esc normal mode · :q! revert · :wq done",
            },
            Mode::Props => "n add · Enter change · d delete · Esc close",
            Mode::Filter => "↑↓ pick · Enter go · Esc close",
            Mode::Picker => "↑↓ pick · Enter run · Esc close",
            Mode::Conflict if fold_core::merge::conflict_pairs(&self.vault).is_empty() => "Esc close",
            Mode::Conflict => "o ours · t theirs · b both · n next · Esc close",
            Mode::Help => "Esc close",
        }
    }

    /// What can follow the first key of a sequence (§10.3): each second
    /// key, and what it does, in the status bar's words, the one to keep
    /// longest where the bar is short first.
    fn follows(&self, first: char) -> Vec<(char, &'static str)> {
        match first {
            // done hidden, the way back to them leads (§10.1)
            'z' if self.hide_done => vec![('d', "show done"), ('p', "pane"), ('w', "wrap"), ('r', "raw"), ('a', "archive")],
            'z' => vec![('p', "pane"), ('w', "wrap"), ('d', "hide done"), ('r', "raw"), ('a', "archive")],
            'g' => vec![('g', "top")],
            '[' => vec![('[', "previous heading")],
            ']' => vec![(']', "next heading")],
            _ => Vec::new(),
        }
    }

    /// A second key nothing follows the first with: said, not swallowed
    /// (§10.3). `Esc` lets the first key go, quietly.
    fn no_sequence(&mut self, first: char, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            return;
        }
        let seq = match key.code {
            KeyCode::Char(c) if c != ' ' && key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => format!("{}{}", first, c),
            _ => format!("{} {}", first, key_name(key)),
        };
        let keys: Vec<String> = self.follows(first).iter().map(|(k, _)| k.to_string()).collect();
        let keys = match keys.split_last() {
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} or {}", rest.join(", "), last),
            None => String::new(),
        };
        self.say(format!("{} does nothing · after {} press {}", seq, first, keys));
    }

    /// A key fold has no use for, where one often reaches for it (§10.3):
    /// what to press instead. Any other key that does nothing says nothing.
    fn misfire(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let reading = self.focus == Focus::Reading;
        let instead = match key.code {
            KeyCode::Char('z') if ctrl => "u undoes",
            KeyCode::Char('f') if ctrl && reading => "/ searches",
            KeyCode::Char('f') if ctrl => "/ finds",
            _ if ctrl => return,
            KeyCode::Char('i') => "e edits",
            // the reading pane's o opens a link
            KeyCode::Char('o') => "n adds a node below",
            // the reading pane's m is the menu of the line's node; Tab,
            // then d, would delete the outline's selection (§10.4)
            KeyCode::Delete if reading => "m, then choose Delete",
            KeyCode::Delete => "d deletes",
            _ => return,
        };
        self.say(format!("{} does nothing here: {}", key_name(key), instead));
    }

    /// What the main loop does between events: the editor's autosave, then
    /// a debounced reload. True when it reloaded.
    pub fn tick(&mut self) -> bool {
        // autosave after 750 ms without a keystroke (§10.6)
        if self.mode == Mode::Edit && self.editor_dirty() && self.edit_last_key.elapsed() > Duration::from_millis(750) {
            self.save_editor();
        }
        // external changes: debounced reload (§11.2)
        let due = self.poll_watcher();
        if due {
            self.reload_external();
        }
        // pairs that came in while the user was busy, once at rest (§10.7)
        self.open_waiting_conflicts();
        due
    }

    /// Drain watcher events; returns true if a debounced reload should run.
    /// Debounce state: events are drained immediately; a reload is due when
    /// at least one relevant event arrived and 200 ms have passed since the
    /// last one (§11.2), and the files hold something the vault does not.
    pub fn poll_watcher(&mut self) -> bool {
        use notify::event::{AccessKind, AccessMode, EventKind, ModifyKind};
        if let Some(rx) = &self.watch_rx {
            while let Ok(res) = rx.try_recv() {
                if let Ok(event) = res {
                    // only a write can change a file's text: opening or
                    // reading one, as every reload does, or its times and
                    // mode changing is no change (§11.2)
                    let write = match event.kind {
                        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
                        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_)) => false,
                        _ => true,
                    };
                    // ignore our own temp files and ignored patterns (§11.4)
                    let relevant = event.paths.iter().any(|p| {
                        let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                        !name.starts_with('.')
                            && !name.ends_with(".fold-tmp")
                            && !name.ends_with(".tmp")
                    });
                    if write && relevant {
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
        // our own writes are seen once the self-write window is over:
        // nothing to take in, and no reload to save the editor for
        due && self.outside_change()
    }

    /// Whether the files hold something the vault does not (§11.2): a file
    /// another program wrote, added or removed, or a new sync-conflict copy.
    /// An unreadable vault counts: the reload says why.
    fn outside_change(&self) -> bool {
        self.new_conflict_copy() || self.vault.changed_on_disk().unwrap_or(true)
    }

    /// Whether a sync-conflict copy is in the vault that was not at the
    /// last reload (§11.2): one the merge left alone or failed on stays,
    /// and is no change by itself; the next change tries it again.
    fn new_conflict_copy(&self) -> bool {
        let copies = self.vault.conflict_files().unwrap_or_default();
        copies.iter().any(|c| !self.conflict_copies.contains(c))
    }

    /// Reload after an external change (§11.2): editor saves first, then
    /// re-parse, cursor re-attached by id, key, deepest surviving step, and
    /// the editor re-rendered if what it shows changed. A new sync-conflict
    /// file starts the merge flow (§12).
    pub fn reload_external(&mut self) {
        // what popups show is held by key: the editor's save and the reload
        // re-parse files, renumbering their nodes. A closed property form's
        // node is stale after any verb since, and is let go
        let props_key = self.props_target.filter(|_| self.mode == Mode::Props).map(|t| self.vault.key_of(t));
        let filter_key = self.filter_rows.get(self.filter_sel).filter(|_| self.mode == Mode::Filter).map(|&r| self.vault.key_of(r));
        // what came in: the files as they are against what was read, taken
        // before the editor's save writes what was typed; that the save
        // took the typing, or why not, is said beside it
        let before = tops(&self.vault);
        let after = self.vault.on_disk().map(|v| tops(&v));
        let status = std::mem::take(&mut self.status);
        let dirty = self.editor_dirty();
        // a block cut in the editor stays in transit across the save: a
        // change to nothing it shows leaves it as it is (§5.2)
        self.save_editor();
        let edit = self.editor_key().map(|k| (k, self.editor_files()));
        let saved = std::mem::replace(&mut self.status, status);
        let typed = saved.is_empty() && dirty && !self.editor_dirty();
        let cursor_key = self.current().map(|r| self.vault.key_of(r));
        // the zoom is held by key (§11.2): the merge flow re-parses the
        // vault, then may close the editor, which reads the outline, before
        // the reload is over
        self.anchor_zoom();
        // a sync-conflict file starts the merge flow (§11.2): a new one, or
        // one a merge left alone or failed on, which may merge now; what it
        // took in without a pair is said as any change is, with what came in
        // beside it, the save's typing not part of it. A copy that brings
        // nothing in, one left alone or the same as its file, says nothing,
        // as at startup. That the save took the typing is said ahead of what
        // came in, which gives way where the bar is short; news of a pair,
        // or an error, says no more (§10.6)
        let mut came_in = true;
        // listed before the merge: a copy that lands while it runs is new
        // to the next reload, not seen with these
        let copies = self.vault.conflict_files().unwrap_or_default();
        if !copies.is_empty() {
            // the merge reads the files again, as the save left them, typing
            // and all; where nothing was saved, as read above
            let read = after.as_ref().ok().filter(|_| !dirty).cloned();
            let was = read.unwrap_or_else(|| self.vault.on_disk().map(|v| tops(&v)).unwrap_or_else(|_| tops(&self.vault)));
            if !self.merge_conflict_files() {
                // what came in: the files against what was read, then what
                // the merge took in over them. Not the vault as the save left
                // it: the save took in what changed outside in a file it
                // wrote (§5.2)
                let now = merged_over(after.unwrap_or_else(|_| was.clone()), &was, tops(&self.vault));
                came_in = !same_tops(&before, &now);
                if came_in {
                    self.say(changed_outside(&before, &now, typed));
                }
            }
        } else {
            match self.vault.reload() {
                Ok(()) => self.say(changed_outside(&before, &after.unwrap_or_else(|_| tops(&self.vault)), typed)),
                Err(e) => self.say(format!("reload error: {}", e)),
            }
        }
        // a change to what it shows re-renders it (below): its cut blocks
        // are deleted first, before the zoom and the cursor are found again.
        // One elsewhere in a file it holds, as to another node of root.md,
        // is taken in where it is
        let edit = edit.filter(|(key, files)| {
            *files != self.editor_files() && !self.editor_takes_in(key) && self.drop_cut_blocks()
        });
        // a refused save says why beside what came in, or on its own where
        // nothing came in
        if !saved.is_empty() {
            let msg = if came_in { format!("{} · {}", self.status, saved) } else { saved };
            self.say(msg);
        }
        self.conflict_copies = self.vault.conflict_files().unwrap_or_default();
        self.conflict_copies.retain(|c| copies.contains(c));
        self.settle_zoom();
        if let Some(k) = cursor_key {
            if let Some(r) = self.vault.find_by_key(&k) {
                self.move_cursor_to(r);
            }
        }
        self.clamp_cursor();
        // the property form's node is found again, or the form closes; a
        // target prompt lists its candidates again, and the filter its hits,
        // its selection staying on its node (the menu keeps a key)
        self.props_target = props_key.and_then(|k| self.find_exact(&k));
        if self.mode == Mode::Props {
            match self.props_target {
                Some(_) => self.reopen_props(),
                None => self.mode = self.base_mode(),
            }
        }
        self.refresh_picks();
        if self.mode == Mode::Filter {
            self.update_filter();
            let sel = filter_key.and_then(|k| self.find_exact(&k));
            if let Some(i) = self.filter_rows.iter().position(|&r| Some(r) == sel) {
                self.filter_sel = i;
            }
        }
        if let Some((key, _)) = edit {
            self.rebuild_editor(self.find_exact(&key));
        }
    }

    /// The text of each file the open editor holds, found as the editor
    /// finds it: by id, else by path; None for one gone.
    fn editor_files(&self) -> Vec<Option<String>> {
        let Some(ed) = self.editor.as_ref() else { return Vec::new() };
        let tree = &self.vault.tree;
        let file = |i: &fold_core::edit::OwnerInfo| match &i.id {
            Some(id) => tree.block_by_id(id).map(|r| r.0),
            None => self.vault.file_index(&i.path),
        };
        ed.buf.owners.values().map(|i| file(i).map(|f| tree.files[f].text.clone())).collect()
    }

    /// After a reload changed a file the editor holds (§11.2): where its
    /// node renders as the buffer shows it, line for line, the buffer takes
    /// in the files as they are and stays, a block cut there still to paste
    /// (§5.2). False where the node is gone or what it shows changed.
    fn editor_takes_in(&mut self, key: &NodeKey) -> bool {
        let Some(r) = self.find_exact(key) else { return false };
        let now = fold_core::edit::open_editor(&self.vault, r);
        self.editor.as_mut().is_some_and(|ed| ed.buf.take_in(now))
    }

    /// Before the editor is re-rendered over text a reload changed
    /// (§11.2): a block cut there and not pasted back cannot be pasted as
    /// itself any more, and is deleted as on a Revert (§5.2), in the
    /// op-log entry of the save that wrote its embed out (§10.10). True
    /// when the editor can be re-rendered: text a refused save still holds
    /// is not, nor is one whose cut blocks stay.
    fn drop_cut_blocks(&mut self) -> bool {
        if self.editor_dirty() {
            return false;
        }
        let Some(ed) = self.editor.as_mut() else { return false };
        let snap = ops::Snapshot::take(&self.vault, &format!("edit {}", edited(ed, None)));
        let res = ed.buf.discard(&mut self.vault);
        if res.is_ok() {
            // let go of: what it held is deleted here, not by the next save
            ed.release_clip();
            ed.buf.dirty.clear();
        }
        self.record_edit(snap);
        if let Err(e) = &res {
            self.say(format!("error: {}", e));
        }
        res.is_ok()
    }

    /// The merge flow (§12.2): merge the sync-conflict copies, then show
    /// the pairs the merge raised. A copy the merge leaves alone (an
    /// ignored file's, one with nothing to merge against) raises none and
    /// stays, so it must not reopen the view on every reload; nor must
    /// pairs lived with (§12.5). New pairs never take the screen from a
    /// busy user (§10.7): the view opens on them once the outline is at
    /// rest, at once if it is, and an open view stays on its pair. True
    /// when it said something: the pairs raised, or why it failed; what a
    /// merge without pairs took in is the caller's to say. A block cut in
    /// the editor and not pasted back is moved here: a copy from before the
    /// cut does not put it back where it was (§5.2, §11.2).
    fn merge_conflict_files(&mut self) -> bool {
        let before = self.conflict_blocks();
        let shown = before.get(self.conflict_idx).cloned().filter(|_| self.mode == Mode::Conflict);
        let moving: Vec<Id> = self.transit().into_iter().map(|(id, _)| id).collect();
        match fold_core::merge::merge_sync_conflicts_moving(&mut self.vault, false, &moving) {
            Ok(_) => {
                let blocks = self.conflict_blocks();
                let raised: Vec<NodeKey> = blocks.iter().filter(|b| !before.contains(b)).cloned().collect();
                if raised.is_empty() {
                    return false;
                }
                if self.mode == Mode::Conflict {
                    // the pair on screen stays there; a view that showed
                    // none takes no side for a moment, as on opening
                    match shown.and_then(|k| blocks.iter().position(|b| *b == k)) {
                        Some(i) => self.conflict_idx = i,
                        None => self.conflict_opened = Some(Instant::now()),
                    }
                    self.say(self.conflict_news(&self.pairs_among(&raised)));
                    return true;
                }
                self.conflicts_waiting.extend(raised);
                if !self.open_waiting_conflicts() {
                    let news = self.conflict_news(&self.pairs_among(&self.conflicts_waiting));
                    // the lit ⚠ count beside it says how many (§10.7)
                    self.say(format!("{}: click ⚠ to resolve", news));
                }
            }
            Err(e) => self.say(format!("merge error: {}", e)),
        }
        true
    }

    /// The startup scan (§12.2): a sync-conflict file present at startup
    /// starts the merge flow. What it took in without a pair is said as a
    /// reload says it (§11.2); a copy it left alone leaves the greeting be.
    pub fn merge_on_startup(&mut self) {
        if !self.vault.conflict_files().is_ok_and(|files| !files.is_empty()) {
            return;
        }
        let was = tops(&self.vault);
        if !self.merge_conflict_files() {
            let now = tops(&self.vault);
            if !same_tops(&was, &now) {
                self.say(changed_outside(&was, &now, false));
            }
        }
    }

    /// Open the conflict view on its own (§10.7), on the pairs that came in
    /// while the user was busy, once the outline is at rest; true when it
    /// opened. For its first half second it takes no side (`key_conflict`).
    fn open_waiting_conflicts(&mut self) -> bool {
        if self.conflicts_waiting.is_empty() || !self.at_rest() {
            return false;
        }
        // pairs resolved meanwhile, elsewhere, are not waited on
        let keys = std::mem::take(&mut self.conflicts_waiting);
        let waiting = self.pairs_among(&keys);
        let Some(&(i, _)) = waiting.first() else { return false };
        self.say(self.conflict_news(&waiting));
        self.mode = Mode::Conflict;
        self.conflict_idx = i;
        self.conflict_opened = Some(Instant::now());
        true
    }

    /// The outline at rest (§10.7): normal mode with nothing open over it,
    /// no key sequence or drag begun, and no key, paste or click for 2 s.
    fn at_rest(&self) -> bool {
        self.mode == Mode::Normal
            && self.editor.is_none()
            && self.prompt.is_none()
            && self.ui.menu.is_none()
            && self.pending.is_none()
            && !self.ui.press.is_some_and(|p| p.dragging)
            && !self.ui.resizing
            && self.last_input.is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
    }

    /// The unresolved pairs, (ours, theirs), as the conflict view lists
    /// them (§12.5): first in the outline first, as it opens on the first
    /// new one in the outline (§10.7), and `n` goes on down it.
    fn view_pairs(&self) -> Vec<(NRef, NRef)> {
        let mut pairs = fold_core::merge::conflict_pairs(&self.vault);
        if pairs.len() > 1 {
            // found by their blocks' files; the outline is walked for
            // their order only when there is one to find
            let tree = &self.vault.tree;
            let mut order = Vec::new();
            tree.walk(tree.root, &mut |_, r| order.push(r));
            pairs.sort_by_key(|&(ours, _)| order.iter().position(|&r| r == ours));
        }
        pairs
    }

    /// The pairs whose conflict blocks are `keys` (§12.5), first in the
    /// outline first: each one's place in the view's list, and its node.
    fn pairs_among(&self, keys: &[NodeKey]) -> Vec<(usize, NRef)> {
        self.view_pairs()
            .into_iter()
            .enumerate()
            .filter(|(_, (_, theirs))| keys.contains(&self.vault.key_of(*theirs)))
            .map(|(i, (ours, _))| (i, ours))
            .collect()
    }

    /// What came in, by the node of the first of those pairs in the
    /// outline: *sync conflict in “NAS”*, *sync conflicts in “NAS” and 2
    /// more*.
    fn conflict_news(&self, pairs: &[(usize, NRef)]) -> String {
        match pairs {
            [] => "sync conflict".into(),
            [(_, ours)] => format!("sync conflict in {}", self.named(*ours)),
            [(_, ours), rest @ ..] => format!("sync conflicts in {} and {} more", self.named(*ours), rest.len()),
        }
    }

    /// A node as the status bar names it: its title, quoted.
    fn named(&self, r: NRef) -> String {
        quoted(&self.vault.tree.node(self.vault.tree.resolved_child(r)).title)
    }

    /// The conflict blocks of the unresolved pairs, by id, as the view
    /// lists them (§12.5).
    fn conflict_blocks(&self) -> Vec<NodeKey> {
        self.view_pairs()
            .into_iter()
            .map(|(_, theirs)| self.vault.key_of(theirs))
            .collect()
    }

    /// Visible outline rows: the zoom subtree, flattened, honouring folds
    /// and the hide-done toggle.
    pub fn rows(&self) -> Vec<FlatRow> {
        let mut out = Vec::new();
        let root = self.zoom().unwrap_or(self.vault.tree.root);
        self.flatten(root, 0, false, &mut out, &mut Vec::new());
        out
    }

    /// The zoom root (§10.3), found again by key while a verb runs.
    fn zoom(&self) -> Option<NRef> {
        match &self.zoom_anchor {
            Some(k) => self.vault.find_by_key(k).filter(|&r| self.vault.tree.node(r).kind != Kind::Root),
            None => self.zoom_root,
        }
    }

    /// A copy's own fold hides nothing it is zoomed into to show (§10.1),
    /// however the zoom gets there: Enter, Backspace, a breadcrumb or the
    /// view remembered.
    fn set_zoom(&mut self, r: Option<NRef>) {
        if let Some(z) = r.filter(|&z| self.vault.tree.node(z).conflict().is_some()) {
            self.set_folded(z, false);
        }
        self.zoom_root = r;
        self.zoom_anchor = None;
    }

    /// Hold the zoom by key before a verb or undo rewrites files; a zoomed
    /// node that goes away zooms out to its deepest surviving ancestor.
    fn anchor_zoom(&mut self) {
        self.zoom_root = self.zoom();
        self.zoom_anchor = self.zoom_root.map(|z| self.vault.key_of(z));
    }

    /// The verb is over: the zoom it found by key holds, with the fold it
    /// had; one that fell back to an ancestor lands there as any zoom does.
    fn settle_zoom(&mut self) {
        let z = self.zoom();
        if self.zoom_anchor.is_some() && z.map(|z| self.vault.key_of(z)) != self.zoom_anchor {
            self.set_zoom(z);
        } else {
            self.zoom_root = z;
            self.zoom_anchor = None;
        }
    }

    /// A node's parent in the outline, `None` at the top: a block root's
    /// tree parent is its own file's root, but its outline parent is the
    /// node that embeds it (as the breadcrumbs show).
    fn outline_parent(&self, r: NRef) -> Option<NRef> {
        let chain = self.chain(r);
        chain.len().checked_sub(2).map(|i| chain[i])
    }

    /// `seen` holds the blocks already shown: an embed cycle or a second
    /// embed of a block (§6.2 diagnostics) shows it once, as walk and render
    /// do. Only a block can be reached twice, so only blocks are recorded.
    fn flatten(&self, r: NRef, depth: usize, via_embed: bool, out: &mut Vec<FlatRow>, seen: &mut Vec<NRef>) {
        let n = self.vault.tree.node(r);
        if n.is_block() {
            if seen.contains(&r) {
                return;
            }
            seen.push(r);
        }
        if n.kind == Kind::Root {
            for c in self.vault.tree.resolved_children(r) {
                self.flatten(c, depth, false, out, seen);
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
        if self.is_folded(r) {
            return;
        }
        for c in self.vault.tree.resolved_children(r) {
            let through_embed = self.vault.tree.node(c).is_embed()
                && self.vault.tree.resolved_child(c) != c;
            self.flatten(c, depth + 1, through_embed || via_embed, out, seen);
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

    /// `zd` (§8.5): hide or show done tasks. The selection, and with it the
    /// reading pane, stays on its node, not its row number: the next verb
    /// must not act on whatever took its row. A node hidden gives way to
    /// the next shown sibling of the outermost done task it is in, else to
    /// the row above that task.
    fn toggle_hide_done(&mut self) {
        let before = self.rows();
        let at = self.cursor.min(before.len().saturating_sub(1));
        self.hide_done = !self.hide_done;
        let after = self.rows();
        let shown: std::collections::HashSet<NRef> = after.iter().map(|r| r.nref).collect();
        let mut target = before.get(at).map(|r| r.nref).filter(|r| shown.contains(r));
        if target.is_none() && at < before.len() {
            // the outermost hidden row it is under: its ancestors are the
            // rows above it, each shallower than the last
            let (mut top, mut depth) = (at, before[at].depth);
            for i in (0..at).rev() {
                if before[i].depth < depth {
                    depth = before[i].depth;
                    if !shown.contains(&before[i].nref) {
                        top = i;
                    }
                }
            }
            let d = before[top].depth;
            target = before[top + 1..]
                .iter()
                .take_while(|r| r.depth >= d)
                .find(|r| r.depth == d && shown.contains(&r.nref))
                .or_else(|| before[..top].iter().rev().find(|r| shown.contains(&r.nref)))
                .map(|r| r.nref);
        }
        if let Some(i) = target.and_then(|t| after.iter().position(|r| r.nref == t)) {
            self.cursor = i;
        }
        self.clamp_cursor();
        self.say(if self.hide_done { "done hidden" } else { "done shown" });
    }

    /// The node a key names, only if it is that very node: no falling back
    /// to the deepest step that still exists.
    fn find_exact(&self, key: &NodeKey) -> Option<NRef> {
        self.vault.find_by_key(key).filter(|&r| self.vault.key_of(r) == *key)
    }

    /// A conflict copy is folded until unfolded (§10.1); any other node
    /// once folded.
    fn is_folded(&self, r: NRef) -> bool {
        let key = self.vault.key_of(r);
        match self.vault.tree.node(r).conflict() {
            Some(_) => !self.unfolded_copies.contains(&key),
            None => self.folded.contains(&key),
        }
    }

    fn set_folded(&mut self, r: NRef, fold: bool) {
        let key = self.vault.key_of(r);
        let (keys, listed) = match self.vault.tree.node(r).conflict() {
            Some(_) => (&mut self.unfolded_copies, !fold),
            None => (&mut self.folded, fold),
        };
        keys.retain(|k| *k != key);
        if listed {
            keys.push(key);
        }
    }

    fn toggle_fold(&mut self, r: NRef) {
        self.set_folded(r, !self.is_folded(r));
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
        let Some((words, step)) = self.done_words(r) else {
            // nothing changes, and nothing is left to undo
            self.say(self.not_a_task(r));
            return;
        };
        self.push_undo(&step);
        let key = self.vault.key_of(r);
        match ops::toggle_task(&mut self.vault, r) {
            Ok(()) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
                self.refresh_after(&words);
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    /// What `x` says on `r`, a node that is no task (§10.1).
    fn not_a_task(&self, r: NRef) -> String {
        format!("{} isn't a task · {}", self.named(r), self.next_step(Action::ToggleTask, "makes it one"))
    }

    /// The next step a verb's message names (§10.1): `a`'s key and what it
    /// does. The reading pane's keys are its own (§10.4), and Tab, then
    /// the key, would act on the outline's selection: there, a node's
    /// verb is in the menu of the line's node. In the editor every key is
    /// text (§10.6): the step is the pointer's, a row's menu or the top
    /// bar's button.
    fn next_step(&self, a: Action, does: &str) -> String {
        let on_node = action::NODE_MENU.contains(&Some(a));
        if self.mode == Mode::Edit {
            return match a.icon() {
                Some(icon) if !on_node => format!("click {} {}", icon, a.label()),
                _ => format!("right-click a row, then choose {}", a.label()),
            };
        }
        match a.key() {
            Some(k) if self.focus != Focus::Reading || !on_node => format!("{} {}", k, does),
            _ => format!("m, then choose {}", a.label()),
        }
    }

    /// What `x` does to `r`, in the status bar's words (§10.1), and as
    /// its op-log entry names it (§10.10); `None` when it is no task.
    fn done_words(&self, r: NRef) -> Option<(String, String)> {
        let name = self.named(r);
        Some(match self.vault.tree.node(self.vault.tree.resolved_child(r)).task? {
            TaskState::Open => (format!("done: {}", name), format!("mark {} done", name)),
            TaskState::Done => (format!("reopened: {}", name), format!("reopen {}", name)),
        })
    }

    fn act_toggle_taskness(&mut self) {
        let Some(r) = self.subject() else { return };
        let name = self.named(r);
        let (words, step) = match self.vault.tree.node(self.vault.tree.resolved_child(r)).task {
            Some(_) => (format!("removed the checkbox from {}", name), format!("remove the checkbox from {}", name)),
            None => (format!("made {} a task", name), format!("make {} a task", name)),
        };
        self.push_undo(&step);
        let key = self.vault.key_of(r);
        match ops::toggle_taskness(&mut self.vault, r) {
            Ok(()) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
                self.refresh_after(&words);
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_make_block(&mut self) {
        let Some(r) = self.subject() else { return };
        let name = self.named(r);
        self.push_undo(&format!("give {} its own file", name));
        let on = self.on_node(r);
        match ops::make_block(&mut self.vault, r) {
            Ok(id) => {
                // no id, no file name (§1 principle 3b)
                self.say(format!("gave {} its own file", name));
                // the cursor (a zoom, the editor) stays on the node, now a block
                if let Some(nr) = self.vault.tree.block_by_id(&id) {
                    self.follow(on, nr);
                    self.move_cursor_to(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    /// Whether the zoom, and the editor, are on `r`, before a verb moves it.
    fn on_node(&self, r: NRef) -> (bool, bool) {
        let r = self.vault.tree.resolved_child(r);
        (self.zoom() == Some(r), self.editor_node() == Some(r))
    }

    /// A node a verb moved landed at `nr`: a zoom on it stays on it
    /// (§10.3), and the editor open on it is re-rendered there after the
    /// verb (§10.6), not over whatever node its old path now leads to.
    fn follow(&mut self, (zoomed, editing): (bool, bool), nr: NRef) {
        if zoomed {
            self.set_zoom(Some(nr));
        }
        if editing {
            self.edit_moved = Some(nr);
        }
    }

    fn act_delete(&mut self) {
        let Some(r) = self.subject() else { return };
        self.copy(r);
        let name = self.copied.0.clone();
        self.push_undo(&format!("delete {}", name));
        // a node's conflict copies go with it (§12.5)
        let what = match ops::conflict_copies(&self.vault.tree, r).len() {
            0 => name,
            1 => format!("{} and its conflict copy", name),
            n => format!("{} and its {} conflict copies", name, n),
        };
        let undo = self.next_step(Action::Undo, "undoes");
        match ops::delete_subtree(&mut self.vault, r) {
            Ok(1) => self.refresh_after(&format!("deleted {} · {}", what, undo)),
            Ok(n) => self.refresh_after(&format!("deleted {} ({} nodes) · {}", what, n, undo)),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    /// Put `r`'s subtree in the register (§10.3 `y`, `d`), and what it is.
    fn copy(&mut self, r: NRef) {
        self.register = ops::yank(&self.vault, r);
        self.copied = (self.named(r), self.vault.tree.node(self.vault.tree.resolved_child(r)).kind);
    }

    fn act_yank(&mut self) {
        let Some(r) = self.subject() else { return };
        self.copy(r);
        self.say(format!("copied {} · {}", self.copied.0, self.next_step(Action::PasteAfter, "pastes")));
    }

    fn act_paste(&mut self, after: bool) {
        if self.register.is_empty() {
            self.say(format!("nothing copied yet · {}", self.next_step(Action::Copy, "copies a node")));
            return;
        }
        let Some(r) = self.subject() else { return };
        let text = self.register.clone();
        let (name, kind) = self.copied.clone();
        self.push_undo(&format!("paste {}", name));
        match ops::paste(&mut self.vault, r, &text, after) {
            Ok(moved) => self.refresh_after(&with_rule_note(&format!("pasted {}", name), moved, kind)),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_move(&mut self, down: bool) {
        let Some(r) = self.subject() else { return };
        self.push_undo(&format!("move {} {}", self.named(r), if down { "down" } else { "up" }));
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
        let (spelling, kind) = match self.vault.tree.node(self.vault.tree.resolved_child(r)).kind {
            Kind::Section => ("a bullet", Kind::Item),
            _ => ("a heading", Kind::Section),
        };
        let words = format!("made {} {}", self.named(r), spelling);
        // a node in a conflict pair keeps its spelling (§12.5)
        if ops::conflict_pair(&self.vault.tree, r).len() > 1 {
            self.say(format!("can't make {} {}: {}", self.named(r), spelling, ops::PAIR_SPELLING));
            return;
        }
        self.push_undo(&format!("make {} {}", self.named(r), spelling));
        let key = self.vault.key_of(r);
        match ops::toggle_spelling(&mut self.vault, r) {
            Ok(moved) => {
                if let Some(nr) = self.vault.find_by_key(&key) {
                    self.move_cursor_to(nr);
                }
                self.say(with_rule_note(&words, moved, kind));
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_demote(&mut self) {
        let Some(s) = self.subject() else { return };
        let r = self.vault.tree.resolved_child(s);
        self.push_undo(&format!("indent {}", self.named(r)));
        let (key, kind) = (self.vault.key_of(r), self.vault.tree.node(r).kind);
        // it goes under the node before it (§10.3), with its conflict
        // pair, which moves as one (§12.5)
        let sibs = self.vault.tree.resolved_children(self.outline_parent(r).unwrap_or(self.vault.tree.root));
        let first = ops::conflict_pair(&self.vault.tree, r)[0];
        let prev = sibs.iter().position(|&c| c == first).and_then(|i| i.checked_sub(1)).map(|i| sibs[i]);
        let Some(prev) = prev else {
            self.say(format!("can't indent {}: nothing above it", self.named(r)));
            return;
        };
        // nor into a conflict copy, which keeping ours trashes (§12.5); a
        // node in one moves within it
        if self.vault.tree.node(prev).conflict().is_some() {
            self.say(format!("can't indent {} into a conflict copy", self.named(r)));
            return;
        }
        let prev = self.vault.key_of(prev);
        let name = self.named(r);
        let on = self.on_node(r);
        match ops::demote(&mut self.vault, s) {
            Ok(moved) => {
                // the cursor (a zoom, the editor) stays on the node where it
                // landed
                if let Some(nr) = self.moved_node(&key, kind, &prev, None) {
                    self.follow(on, nr);
                    self.move_cursor_to(nr);
                }
                self.say(with_rule_note(&format!("indented {}", name), moved, kind));
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_promote(&mut self) {
        let Some(s) = self.subject() else { return };
        let r = self.vault.tree.resolved_child(s);
        self.push_undo(&format!("outdent {}", self.named(r)));
        let (key, kind) = (self.vault.key_of(r), self.vault.tree.node(r).kind);
        // it goes beside its parent, right after it, under the parent's
        // parent (§10.3)
        let parent = self.outline_parent(r);
        let grand = parent.map(|p| self.outline_parent(p).unwrap_or(self.vault.tree.root));
        let rank = parent.zip(grand).and_then(|(p, g)| {
            let kids = self.vault.tree.resolved_children(g);
            let at = kids.iter().position(|&c| c == p)?;
            Some(self.namesakes_before(r, g, self.past_copies(&kids, at + 1, r)))
        });
        let Some(grand) = grand.map(|g| self.vault.key_of(g)) else {
            self.say(format!("can't outdent {}: it's at the top level", self.named(r)));
            return;
        };
        let name = self.named(r);
        let on = self.on_node(r);
        match ops::promote(&mut self.vault, s) {
            Ok(moved) => {
                // the cursor (a zoom, the editor) stays on the node where it
                // landed
                if let Some(nr) = self.moved_node(&key, kind, &grand, rank) {
                    self.follow(on, nr);
                    self.move_cursor_to(nr);
                }
                self.say(with_rule_note(&format!("outdented {}", name), moved, kind));
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_archive(&mut self) {
        let Some(r) = self.subject() else { return };
        let (name, kind) = (self.named(r), self.vault.tree.node(self.vault.tree.resolved_child(r)).kind);
        self.push_undo(&format!("archive {}", name));
        match ops::archive(&mut self.vault, r) {
            Ok(moved) => self.refresh_after(&with_rule_note(&format!("archived {}", name), moved, kind)),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    fn act_clear_done(&mut self) {
        let zoom = self.zoom();
        let under = zoom.map(|z| format!(" under {}", self.named(z))).unwrap_or_default();
        self.push_undo(&format!("clear done tasks{}", under));
        let target = zoom.unwrap_or(self.vault.tree.root);
        match ops::clear_done(&mut self.vault, target) {
            Ok(0) => self.refresh_after(&format!("no done tasks to clear{}", under)),
            Ok(1) => self.refresh_after(&format!("cleared 1 done task{}", under)),
            Ok(n) => self.refresh_after(&format!("cleared {} done tasks{}", n, under)),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    /// *Move to…* (§6.5): the prompt's moving node goes under `dest`.
    fn refile_to(&mut self, r: NRef, dest: NRef) {
        let (rr, dest_key) = (self.vault.tree.resolved_child(r), self.vault.key_of(self.vault.tree.resolved_child(dest)));
        let (key, kind) = (self.vault.key_of(rr), self.vault.tree.node(rr).kind);
        let words = format!("moved {} to {}", self.named(rr), self.named(dest));
        self.push_undo(&format!("move {} to {}", self.named(rr), self.named(dest)));
        let on = self.on_node(rr);
        match ops::refile(&mut self.vault, r, dest) {
            Ok(moved) => {
                self.refresh_after(&with_rule_note(&words, moved, kind));
                if let Some(nr) = self.moved_node(&key, kind, &dest_key, None) {
                    self.follow(on, nr);
                    self.reveal(nr);
                }
            }
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    /// Where a node a verb moved under `dest` (a key taken before the move:
    /// the move renumbers the nodes of the files it writes) landed: a block
    /// by its id; else, among `dest`'s children with the node's title and
    /// kind, but no conflict copy (one that moved with it has its title,
    /// §12.5), the one the ordering rule (§3.1) put it at. Moved in as the
    /// last child, that is the last of them (an item goes after the items,
    /// a section after the sections); put at a place among them, the one
    /// after the `rank` namesakes that `namesakes_before` counted there.
    fn moved_node(&self, key: &NodeKey, kind: Kind, dest: &NodeKey, rank: Option<usize>) -> Option<NRef> {
        let title = match key {
            NodeKey::Path { steps, .. } => steps.last().map(|(t, _)| t.clone()),
            NodeKey::Id(id) => return self.vault.tree.block_by_id(id),
            NodeKey::Root => None,
        }?;
        let kids = self.vault.tree.resolved_children(self.find_exact(dest)?);
        let mut hits = kids.iter().filter(|&&c| {
            let n = self.vault.tree.node(c);
            n.title == title && n.kind == kind && n.conflict().is_none()
        });
        match rank {
            Some(i) => hits.nth(i).copied(),
            None => hits.next_back().copied(),
        }
    }

    /// Before a verb puts `r` among `dest`'s children, just before the
    /// `at`-th: how many of them, other than `r` and conflict copies,
    /// share its title and kind and come before that place, for
    /// `moved_node`. Clamped by the ordering rule (§3.1), an item goes no
    /// later than the first section and a section no earlier than after
    /// the last item, so it still comes right after these namesakes.
    fn namesakes_before(&self, r: NRef, dest: NRef, at: usize) -> usize {
        let n = self.vault.tree.node(r);
        let kids = self.vault.tree.resolved_children(dest);
        kids.iter()
            .take(at)
            .filter(|&&c| {
                let m = self.vault.tree.node(c);
                c != r && m.title == n.title && m.kind == n.kind && m.conflict().is_none()
            })
            .count()
    }

    /// Where a node that a verb puts just before the `at`-th of `kids`
    /// lands: past the conflict copies there, which pair with the node
    /// before them (§12.5), unless it is `r`, that node, back in its place.
    fn past_copies(&self, kids: &[NRef], at: usize, r: NRef) -> usize {
        let mut at = at;
        if at.checked_sub(1).map(|i| kids[i]) != Some(r) {
            while kids.get(at).is_some_and(|&c| self.vault.tree.node(c).conflict().is_some()) {
                at += 1;
            }
        }
        at
    }

    /// The node an action applies to: a menu's target, else the cursor's.
    fn subject(&self) -> Option<NRef> {
        self.action_target.or_else(|| self.current())
    }

    fn act_new_node(&mut self, child: bool) {
        let Some(r) = self.subject() else { return };
        self.push_undo(&format!("add a node {} {}", if child { "under" } else { "after" }, self.named(r)));
        let res = if child {
            match ops::append_child_public(&mut self.vault, r, "") {
                Ok(nr) => {
                    self.edit_new_node(nr);
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
            match ops::paste(&mut self.vault, r, &format!("{}\n", line), true) {
                Ok(_) => {
                    // the new sibling: the node after the cursor node among
                    // its siblings in the outline, past any conflict copy
                    // of it (§12.5), with an empty title
                    let r = self.vault.find_by_key(&key).unwrap_or(r);
                    let parent = self.outline_parent(r).unwrap_or(self.vault.tree.root);
                    let kids = self.vault.tree.resolved_children(parent);
                    let new = kids
                        .iter()
                        .skip_while(|&&c| c != r)
                        .skip(1)
                        .find(|&&c| self.vault.tree.node(c).conflict().is_none())
                        .copied()
                        .filter(|&c| self.vault.tree.node(c).title.is_empty());
                    if let Some(nr) = new {
                        // a sibling of the zoomed node is outside the zoom:
                        // widen it to their parent
                        if self.zoom() == Some(r) {
                            self.set_zoom(self.outline_parent(r));
                        }
                        self.edit_new_node(nr);
                        return;
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        };
        match res {
            Ok(()) => self.refresh_after("node created"),
            Err(e) => self.say(format!("error: {}", e)),
        }
    }

    /// Open the editor on a node `n` / `N` just made, with the outline
    /// cursor on it (§10.6): the new node, not whatever `subject()` is.
    fn edit_new_node(&mut self, nr: NRef) {
        let focus = self.focus;
        self.reveal(nr);
        self.focus = focus;
        self.open_editor_on(nr);
        // an open editor whose save is refused stays, with its text: the
        // title is typed only into a fresh editor on the new node
        if !self.editor_dirty() && self.editor_node() == Some(nr) {
            self.type_new_title();
        }
    }

    /// After `n` / `N` the editor types the new node's title (§10.6): the
    /// cursor goes after the marker and its space, in insert mode as after
    /// Vim's or Helix's `o`. The space is the buffer's alone until something
    /// is typed, so a node left untitled is written as it was.
    fn type_new_title(&mut self) {
        let Some(ed) = self.editor.as_mut() else { return };
        if let Some(l) = ed.buf.lines.first_mut() {
            if !l.text.ends_with(' ') {
                l.text.push(' ');
            }
        }
        ed.set_cursor(editor::Pos::new(0, ed.len(0)));
        ed.mode = editor::Mode::Insert;
    }

    // -------------------------------------------------------- editor (§10.6)

    pub fn act_edit(&mut self) {
        let Some(r) = self.subject() else { return };
        let target = self.vault.tree.resolved_child(r);
        self.open_editor_on(target);
    }

    /// The view settings to remember (§10.1).
    pub fn view(&self) -> View {
        View {
            show_reading: self.show_reading,
            wrap: self.wrap,
            hide_done: self.hide_done,
            keys: self.kept_keys.unwrap_or(Some(self.edit_keys)),
            outline_width: self.ui.outline_width,
            zoom: self.zoom().map(|z| self.vault.key_of(z)),
            folded: self.folded.clone(),
        }
    }

    /// Restore remembered view settings; `keep_keys` when `--keys` or
    /// `$FOLD_KEYS` chose the keymap for this run: the remembered keymap is
    /// then neither applied nor replaced (§10.6).
    pub fn apply_view(&mut self, v: View, keep_keys: bool) {
        self.show_reading = v.show_reading;
        self.wrap = v.wrap;
        self.hide_done = v.hide_done;
        if keep_keys {
            self.kept_keys = Some(v.keys);
        } else if let Some(k) = v.keys {
            self.edit_keys = k;
        }
        self.ui.outline_width = v.outline_width;
        self.folded = v.folded;
        self.set_zoom(v.zoom.and_then(|k| self.vault.find_by_key(&k)).filter(|&r| self.vault.tree.node(r).kind != Kind::Root));
        self.cursor = 0;
        self.clamp_cursor();
    }

    /// Whether a toggle is on, for buttons and the palette.
    pub fn action_on(&self, a: Action) -> Option<bool> {
        match a {
            Action::ReadingPane => Some(self.show_reading),
            Action::HideDone => Some(self.hide_done),
            Action::Wrap => Some(self.wrap),
            Action::RawMode => Some(self.raw_mode),
            _ => None,
        }
    }

    /// Whether the reading pane is on screen: shown, or holding the editor.
    pub fn reading_visible(&self) -> bool {
        self.show_reading || self.mode == Mode::Edit
    }

    /// Open the built-in editor over a node's subtree (§10.6).
    fn open_editor_on(&mut self, target: NRef) {
        // an editor already open saves first, as on leaving it; one whose
        // save is refused stays, with its text and its clipboard
        if !self.save_editor_releasing() {
            return;
        }
        if let Some(ed) = self.editor.take() {
            self.edit_clip = ed.clip;
        }
        let buf = fold_core::edit::open_editor(&self.vault, target);
        self.editor = Some(editor::Editor::new(buf, self.edit_keys, self.edit_clip.clone()));
        self.edit_refused = false;
        self.edit_uncopied = None;
        if self.mode != Mode::Edit {
            self.edit_return = self.focus;
        }
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

    /// Write the editor's dirty blocks; true when nothing is left unsaved.
    /// A refused save (the file changed on disk) keeps the text in the
    /// editor and says why. One that goes through says nothing: the status
    /// bar's ✓ saved says it (§10.6).
    fn save_editor(&mut self) -> bool {
        self.write_editor(false)
    }

    /// Save the editor to leave it, or before it is re-rendered (§10.6): a
    /// block cut and not pasted back cannot be pasted as itself any more,
    /// so once the save went through it is let go, and deleted (§5.2). It
    /// is saved first still in transit, so a refused save leaves the editor
    /// as it was, its clipboard too; and the deletion goes into the op-log
    /// entry of the save that wrote its embed out, this one or an autosave
    /// before it (`record_edit`), so one undo puts back the block's file
    /// and its embed together.
    fn save_editor_releasing(&mut self) -> bool {
        self.write_editor(true)
    }

    /// `save_editor`; with `release`, `save_editor_releasing`.
    fn write_editor(&mut self, release: bool) -> bool {
        if release && !self.editor_dirty() {
            // nothing to save first, so nothing a refusal keeps in transit
            if let Some(ed) = self.editor.as_mut() {
                ed.release_clip();
            }
        }
        if !self.editor_dirty() {
            return true;
        }
        self.settle_undo();
        let cursor = self.current().map(|r| self.vault.key_of(r));
        // an open property form's node too: its OK saves the editor first
        let props = self.props_target.filter(|_| self.mode == Mode::Props).map(|t| self.vault.key_of(t));
        let on_editor = self.anchor_zoom_for_save();
        // the editor saves where it is: a panic in the save leaves its text
        // for `keep_unsaved` to keep (§10.6)
        let Some(ed) = self.editor.as_mut() else { return true };
        // what another program changed beside the edited blocks is taken in
        // before the snapshot, so undoing this save leaves it be
        ed.buf.rebase_dirty(&mut self.vault);
        let snap = ops::Snapshot::take(&self.vault, &format!("edit {}", edited(ed, None)));
        if self.save_panics {
            panic!("the editor's save panicked");
        }
        let mut res = ed.buf.save_all(&mut self.vault);
        if let Some(n) = res.as_ref().ok().copied().filter(|_| release) {
            ed.release_clip();
            // the block that held the embed is written again: counted once
            res = ed.buf.save_all(&mut self.vault).map(|m| m.max(n));
        }
        self.settle_zoom_after_save(on_editor);
        // blocks written before a refusal are an op too
        self.record_edit(snap);
        // the save re-parsed what it wrote: the outline cursor stays on its
        // node, so a verb that saves the editor first acts on the row
        // clicked, and the property form on its own
        if let Some(r) = cursor.and_then(|k| self.find_exact(&k)) {
            self.move_cursor_to(r);
        }
        if let Some(k) = props {
            self.props_target = self.find_exact(&k);
        }
        let saved = match res {
            Ok(_) => true,
            Err(e) => {
                self.say(format!("error: {}", e));
                false
            }
        };
        self.edit_refused = !saved;
        self.edit_last_key = Instant::now();
        saved
    }

    /// Hold the zoom by key across an editor save (§10.3): the save
    /// re-parses the files it writes, renumbering their nodes, and may
    /// trash a block's file, renumbering the files after it. True when the
    /// zoom is the node the editor is open on.
    fn anchor_zoom_for_save(&mut self) -> bool {
        self.anchor_zoom();
        self.zoom_root.is_some() && self.editor_node() == self.zoom_root
    }

    /// After it, the zoom holds: the editor's own node found as the editor
    /// finds it, since its title, and so its key, may be what was edited;
    /// any other node by its key, or its deepest surviving step.
    fn settle_zoom_after_save(&mut self, on_editor: bool) {
        if let Some(k) = self.editor_key().filter(|_| on_editor) {
            self.zoom_anchor = Some(k);
        }
        self.settle_zoom();
    }

    /// Leave the editor, saving it (§10.6); true when it closed. When the
    /// save is refused the editor stays open with its text: nothing typed
    /// is dropped except by *Revert*.
    fn close_editor(&mut self) -> bool {
        // a block cut and not pasted back is deleted once saved (§5.2)
        if !self.save_editor_releasing() {
            self.say(format!("{} — still editing; Revert (:q!) drops the changes", self.status));
            return false;
        }
        if let Some(ed) = self.editor.take() {
            self.edit_clip = ed.clip;
        }
        self.mode = Mode::Normal;
        self.focus = self.edit_return;
        true
    }

    /// The node the editor is open on, as a key that survives the files
    /// changing under it (§3.4): its render root, found in the tree as the
    /// editor last read or wrote it.
    fn editor_key(&self) -> Option<NodeKey> {
        self.editor_node().map(|r| self.vault.key_of(r))
    }

    /// The editor's render root in the tree as the editor last read or
    /// wrote it.
    fn editor_node(&self) -> Option<NRef> {
        let ed = self.editor.as_ref()?;
        let info = ed.buf.owners.values().find(|i| i.parent.is_none())?;
        let r = match &info.id {
            Some(id) => self.vault.tree.block_by_id(id)?,
            None => {
                let file = self.vault.file_index(&info.path)?;
                let f = &self.vault.tree.files[file];
                if info.is_root {
                    (file, f.root_node)
                } else {
                    let i = f.nodes.iter().position(|n| n.kind != Kind::Root && n.span.start == info.start)?;
                    (file, i)
                }
            }
        };
        Some(r)
    }

    /// Before an outline verb writes files under an open editor (§10.6):
    /// the editor saves first. Returns what it is open on and the files as
    /// they are, for `editor_after_write`.
    fn editor_before_write(&mut self, why: &str) -> Option<(NodeKey, ops::Snapshot)> {
        // the editor is re-rendered after, so a block cut and not pasted
        // back cannot be pasted as itself any more: once the editor is
        // saved, it is deleted (§5.2). A refused save keeps it in transit,
        // as the editor keeps its text (it is not re-rendered)
        self.editor.as_ref()?;
        self.save_editor_releasing();
        let key = self.editor_key()?;
        Some((key, ops::Snapshot::take(&self.vault, why)))
    }

    /// After it: if any file changed, the editor is re-rendered over the
    /// files as they now are, so its next save is not refused: on its node
    /// where the verb moved it (`follow`), else the very node its key names.
    fn editor_after_write(&mut self, before: Option<(NodeKey, ops::Snapshot)>) {
        let moved = self.edit_moved.take();
        let Some((key, snap)) = before else { return };
        if ops::Inverse::since(snap, &self.vault).is_some() {
            self.rebuild_editor(moved.or_else(|| self.find_exact(&key)));
        }
    }

    /// Re-render a clean editor over the files as they are now (§11.2), on
    /// its node found again, with the cursor where it was: over the same
    /// node's text, never an ancestor's its old path falls back to. Text a
    /// refused save still holds is never replaced.
    fn rebuild_editor(&mut self, target: Option<NRef>) {
        if self.editor.is_none() || self.editor_dirty() {
            return;
        }
        let Some(target) = target else {
            // nothing unsaved, and nothing left to edit
            self.close_editor();
            self.say("the node being edited is gone");
            return;
        };
        let buf = fold_core::edit::open_editor(&self.vault, target);
        let Some(ed) = self.editor.as_mut() else { return };
        let same = buf.lines.len() == ed.buf.lines.len()
            && buf.lines.iter().zip(&ed.buf.lines).all(|(a, b)| a.text == b.text && a.owner == b.owner);
        if same {
            // the same text: the cursor, mode and undo history stay
            ed.buf = buf;
        } else {
            // new text: the old undo steps would write the old text back
            let mut fresh = editor::Editor::new(buf, ed.keys, ed.clip.clone());
            if ed.mode == editor::Mode::Insert {
                fresh.mode = editor::Mode::Insert;
            }
            fresh.cursor = ed.cursor;
            fresh.search = ed.search.take();
            fresh.clamp_cursor();
            *ed = fresh;
        }
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
        // `:w`, Ctrl-S: asked for, so answered
        if out.save && self.save_editor() {
            self.say("saved");
        }
        if out.close {
            self.close_editor();
            return;
        }
        // moving out of a dirty block saves it (§10.6)
        if before != after && self.editor.as_ref().is_some_and(|e| e.buf.dirty.contains(&before)) {
            self.settle_undo();
            let on_editor = self.anchor_zoom_for_save();
            // saved where it is, as in `write_editor`
            let ed = self.editor.as_mut().unwrap();
            ed.buf.rebase_dirty(&mut self.vault);
            let snap = ops::Snapshot::take(&self.vault, &format!("edit {}", edited(ed, Some(&before))));
            if self.save_panics {
                panic!("the editor's save panicked");
            }
            let res = ed.buf.splice(&mut self.vault, before);
            self.settle_zoom_after_save(on_editor);
            if res.is_ok() {
                self.record_edit(snap);
            } else {
                self.edit_refused = true;
            }
        }
    }

    /// Text pasted into the terminal (bracketed paste): into the editor or
    /// the open prompt.
    pub fn handle_paste(&mut self, text: &str) {
        self.last_input = Some(Instant::now());
        if let Some(p) = self.prompt.as_mut() {
            p.text.push_str(text.lines().next().unwrap_or(""));
            self.refresh_picks();
        } else if let (Mode::Edit, Some(ed)) = (self.mode, self.editor.as_mut()) {
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
        if self.props_target.is_none() {
            return;
        }
        if !editable {
            self.say("read-only line (preserved verbatim)");
            return;
        }
        let edit = self.editor_before_write("outline verb");
        // the save may renumber the form's node, and finds it again
        if let Some(t) = self.props_target {
            self.push_undo(&format!("remove {} from {}", k, self.named(t)));
            match ops::set_frontmatter_key(&mut self.vault, t.0, &k, None) {
                Ok(()) => self.say(format!("{} removed", k)),
                Err(e) => self.say(format!("error: {}", e)),
            }
            self.reopen_props();
        }
        self.editor_after_write(edit);
    }

    pub fn key_props(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = self.base_mode();
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
            KeyCode::Enter => self.accept_prompt_saving_editor(p),
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
                Ok(dest) => {
                    match p.moving.as_ref().and_then(|k| self.find_exact(k)) {
                        Some(r) => self.refile_to(r, dest),
                        None => self.say("that node is gone"),
                    }
                }
                Err(e) => self.say(e),
            },
            PromptAction::GoTo => match picked.map(Ok).unwrap_or_else(|| self.vault.resolve_target(&p.text)) {
                Ok(r) => self.reveal(r),
                Err(e) => self.say(e),
            },
            PromptAction::CaptureText(task) => {
                self.push_undo(&format!("capture {}", quoted(p.text.trim())));
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
                    self.push_undo(&format!("set {} of {}", k, self.named(t)));
                    let on = self.on_node(t);
                    match ops::set_property(&mut self.vault, t, &k, &p.text) {
                        Ok(block) => {
                            // a first property makes a block (§6.1): a zoom
                            // and the editor follow it
                            self.follow(on, block);
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

    /// Accept a prompt as an outline verb: the editor saves first and is
    /// re-rendered after (§10.6). One that writes nothing, as *Go to…*, or
    /// only opens the next prompt, saves it as a pause in typing does: a
    /// block cut in it stays in transit (§5.2).
    fn accept_prompt_saving_editor(&mut self, mut p: Prompt) {
        // the save re-parses what it wrote: the pick is found again by key
        let picked = p.picks.get(p.sel).map(|&r| self.vault.key_of(r));
        let edit = match p.action {
            PromptAction::GoTo | PromptAction::PropNew | PromptAction::ReadSearch => {
                self.save_editor();
                None
            }
            _ => self.editor_before_write("outline verb"),
        };
        match picked.map(|k| self.find_exact(&k)) {
            Some(None) => self.say("that node is gone"),
            Some(Some(r)) => {
                p.picks = vec![r];
                p.sel = 0;
                self.accept_prompt(p);
            }
            None => self.accept_prompt(p),
        }
        self.editor_after_write(edit);
    }

    fn open_prompt(&mut self, label: &str, action: PromptAction, text: String) {
        let moving = match action {
            PromptAction::Refile => self.subject().map(|r| self.vault.key_of(r)),
            _ => None,
        };
        self.prompt = Some(Prompt {
            label: label.into(),
            text,
            action,
            picks: Vec::new(),
            sel: 0,
            moving,
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
        let refile = matches!(p.action, PromptAction::Refile);
        // a node cannot move into its own subtree, nor into the node its
        // conflict copy is of, which moves with it (§12.5)
        let moving = p.moving.as_ref().and_then(|k| self.find_exact(k));
        let moving = moving.map(|r| ops::conflict_pair(&self.vault.tree, r)).unwrap_or_default();
        let mut nodes: Vec<NRef> = Vec::new();
        self.vault.tree.walk(self.vault.tree.root, &mut |t, r| {
            if t.node(r).kind != Kind::Root && !t.node(r).is_embed() {
                nodes.push(r);
            }
        });
        let mut scored: Vec<(u8, usize, NRef)> = Vec::new();
        for r in nodes {
            let chain = self.chain(r);
            if moving.iter().any(|m| chain.contains(m)) {
                continue;
            }
            // nor into a conflict copy, which keeping ours trashes (§12.5)
            if refile && chain.iter().any(|&c| self.vault.tree.node(c).conflict().is_some()) {
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

    /// Put the cursor on a node, unfolding and zooming out as needed. The
    /// ancestors unfolded are its outline ancestors, through the embeds
    /// that stitch in the blocks it sits in, not only those in its file.
    fn reveal(&mut self, r: NRef) {
        let mut chain = self.chain(r);
        chain.pop();
        for a in chain {
            self.set_folded(a, false);
        }
        if self.zoom().is_some() && !self.rows().iter().any(|row| row.nref == r) {
            self.set_zoom(None);
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
        // the view replaces the editor, which saves and closes first
        if self.editor.is_some() && !self.close_editor() {
            return;
        }
        // on the first of the pairs that came in unseen, if any (§10.7)
        let waiting = std::mem::take(&mut self.conflicts_waiting);
        self.conflict_idx = self.pairs_among(&waiting).first().map_or(0, |&(i, _)| i);
        self.conflict_opened = None;
        self.mode = Mode::Conflict;
    }

    /// The copy in the pair `r` is a side of (§12.5): itself, if it is a
    /// copy with a node to pair with, else the copy right after it.
    fn copy_of_pair(&self, r: NRef) -> Option<NRef> {
        let r = self.vault.tree.resolved_child(r);
        let pairs = fold_core::merge::conflict_pairs(&self.vault);
        let pair = pairs.iter().find(|&&(_, t)| t == r).or_else(|| pairs.iter().find(|&&(o, _)| o == r));
        pair.map(|&(_, t)| t)
    }

    /// The conflict view at the pair `r` is a side of: a click on a copy's
    /// ⚠, or *Resolve conflict…* in either side's node menu (§10.7).
    fn enter_conflict_view_at(&mut self, r: NRef) {
        let copy = self.copy_of_pair(r).map(|t| self.vault.key_of(t));
        self.enter_conflict_view();
        // found again by key: the editor, closing, saves and renumbers
        let at = copy.and_then(|k| self.conflict_blocks().iter().position(|b| *b == k));
        if let Some(i) = at.filter(|_| self.mode == Mode::Conflict) {
            self.conflict_idx = i;
        }
    }

    pub fn key_conflict_pub(&mut self, key: KeyEvent) { self.key_conflict(key) }
    fn key_conflict(&mut self, key: KeyEvent) {
        // a view that just opened on its own takes no side, nor edits: a
        // key on its way was meant for what was there before (§10.7)
        let fresh = self.conflict_opened.is_some_and(|t| t.elapsed() < Duration::from_millis(500));
        if fresh && matches!(key.code, KeyCode::Char('o' | 't' | 'b' | 'e')) {
            return;
        }
        let pairs = self.view_pairs();
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
            KeyCode::Char(c @ ('o' | 't' | 'b')) => {
                if let Some(&(ours, theirs)) = pairs.get(self.conflict_idx) {
                    let side = match c {
                        'o' => "ours",
                        't' => "theirs",
                        _ => "both",
                    };
                    let name = self.named(ours);
                    self.push_undo(&format!("keep {} for {}", side, name));
                    let res = match c {
                        'o' => fold_core::merge::resolve_keep_ours(&mut self.vault, theirs),
                        't' => fold_core::merge::resolve_keep_theirs(&mut self.vault, ours, theirs),
                        _ => fold_core::merge::resolve_keep_both(&mut self.vault, theirs),
                    };
                    match res {
                        Ok(()) => self.say(format!("kept {} for {} · u undoes", side, name)),
                        Err(e) => self.say(format!("error: {}", e)),
                    }
                }
            }
            KeyCode::Char('e') => {
                if let Some(&(ours, _)) = pairs.get(self.conflict_idx) {
                    self.open_editor_on(ours);
                }
            }
            // undo and redo as in the outline (§10.10); a pair put back is
            // the one shown
            KeyCode::Char(c @ ('u' | 'U')) => {
                let before = self.conflict_blocks();
                if c == 'u' {
                    self.act_undo();
                } else {
                    self.act_redo();
                }
                if let Some(i) = self.conflict_blocks().iter().position(|b| !before.contains(b)) {
                    self.conflict_idx = i;
                }
            }
            _ => {}
        }
        // a resolved pair leaves the list: stay on the pair now shown
        let n = fold_core::merge::conflict_pairs(&self.vault).len();
        self.conflict_idx = self.conflict_idx.min(n.saturating_sub(1));
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.last_input = Some(Instant::now());
        let held = self.held_words.take();
        self.handle_key_inner(key);
        self.drop_held_words(held);
        self.settle_undo();
    }

    /// A held verb's words are let go once the next key or click is handled
    /// (§10.1): the verb has acted, or the selection may have moved. What
    /// that key or click said, or a verb held again, stays.
    fn drop_held_words(&mut self, held: Option<String>) {
        if self.held_words.is_none() && held.is_some_and(|w| w == self.status) {
            self.say("");
        }
    }

    fn handle_key_inner(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') && self.mode != Mode::Edit {
            self.quit = true;
            return;
        }
        if let Some(p) = self.pending.take() {
            match (p, key.code) {
                ('z', KeyCode::Char('d')) => self.toggle_hide_done(),
                ('z', KeyCode::Char('r')) => {
                    self.raw_mode = !self.raw_mode;
                    self.say(if self.raw_mode { "raw" } else { "styled" });
                }
                ('z', KeyCode::Char('a')) => {
                    if !self.outline_held("za") {
                        self.act_archive();
                    }
                }
                ('z', KeyCode::Char('w')) => self.run_action(Action::Wrap),
                ('z', KeyCode::Char('p')) => self.run_action(Action::ReadingPane),
                ('g', KeyCode::Char('g')) => match self.focus {
                    Focus::Outline => self.cursor = 0,
                    Focus::Reading => self.read_cursor = 0,
                },
                (']', KeyCode::Char(']')) | ('[', KeyCode::Char('[')) => {
                    self.sync_read_target();
                    let doc = self.reading_doc();
                    self.jump_heading(&doc, if p == ']' { 1 } else { -1 });
                }
                _ => self.no_sequence(p, key),
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
                    KeyCode::Char('z') => true,
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
            // help as from the top bar's ?, which the editor types (§10.8)
            Mode::Edit if key.code == KeyCode::F(1) => self.run_action(Action::Help),
            // the keymap decides what Ctrl-c means (copy, or Vim's escape)
            Mode::Edit => self.key_edit(key),
            Mode::Props => self.key_props(key),
            Mode::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::F(1)
                ) {
                    self.mode = self.base_mode();
                }
            }
            Mode::Conflict => self.key_conflict(key),
        }
    }

    /// Drop the editor's unsaved changes (§10.6: *Revert*, `:q!`). Text a
    /// save was refused for is copied to the trash first (§11.5), and the
    /// status line names the copy; one that cannot be written drops
    /// nothing, but for a second *Revert* on the same text: the way out
    /// where the trash takes nothing.
    fn discard_editor(&mut self) {
        let mut said = String::from("changes discarded");
        if let Some((name, text)) = self.editor_text().filter(|_| self.edit_refused && self.editor_dirty()) {
            match self.vault.trash_text(&name, &text) {
                Ok(p) => {
                    let entry = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    said = format!("{}; a copy is in the trash: {}", said, entry);
                }
                Err(_) if self.edit_uncopied.as_ref() == Some(&text) => said = format!("{}; no copy kept", said),
                Err(e) => {
                    // what to press first: the error is what the bar cuts
                    self.say(format!("can't copy to the trash; Revert (:q!) again drops the changes — {}", e));
                    self.edit_uncopied = Some(text);
                    return;
                }
            }
        }
        self.edit_refused = false;
        self.edit_uncopied = None;
        let ed = self.editor.take();
        self.mode = Mode::Normal;
        self.focus = self.edit_return;
        // the reload may renumber nodes: files can have changed on disk
        self.anchor_zoom();
        let _ = self.vault.reload();
        if let Some(ed) = ed {
            // a block a save took out of its parent (cut and not pasted
            // back, or deleted) cannot be pasted back as itself any more:
            // it is deleted now, as on leaving any other way (§5.2)
            let snap = ops::Snapshot::take(&self.vault, &format!("revert {}", edited(&ed, None)));
            if let Err(e) = ed.buf.discard(&mut self.vault) {
                said = format!("{}; error: {}", said, e);
            }
            self.record_edit(snap);
            self.edit_clip = ed.clip;
        }
        self.settle_zoom();
        self.clamp_cursor();
        self.say(said);
    }

    /// When fold ends any way but a quit — a signal, an error, a panic —
    /// the editor is saved as on a quit (§10.6), and text no save can take
    /// goes to the trash whole (§11.5). What to say once the terminal is
    /// back, if anything.
    pub fn keep_unsaved(&mut self) -> Option<String> {
        // the text is taken first: after a panic, the save may panic too
        let (name, text) = self.editor_text()?;
        let dirty = self.editor_dirty();
        // saved even when clean: a block cut and not pasted back, left in
        // transit by the autosave, is deleted on any end of the app (§5.2)
        let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.save_editor_releasing()));
        if saved.unwrap_or(false) || !dirty {
            return None;
        }
        Some(match self.vault.trash_text(&name, &text) {
            Ok(p) => format!("unsaved text kept in {}", p.display()),
            Err(e) => format!("unsaved text could not go to the trash ({}); here it is:\n\n{}", e, text),
        })
    }

    /// Make every editor save panic where it writes, as a bug in the splice
    /// would at every pause: for tests of what fold keeps when it crashes
    /// (§10.6).
    #[doc(hidden)]
    pub fn panic_in_saves(&mut self) {
        self.save_panics = true;
    }

    /// The editor's whole text, and the name the trash keeps it under:
    /// `unsaved-<title>.md`, by the node it is open on.
    fn editor_text(&self) -> Option<(String, String)> {
        let ed = self.editor.as_ref()?;
        let title = ed.buf.owners.values().find(|i| i.parent.is_none()).map(|i| i.title.as_str()).unwrap_or("");
        let text = ed.buf.lines.iter().map(|l| format!("{}\n", l.text)).collect();
        Some((format!("unsaved-{}.md", fold_core::slug(title)), text))
    }

    /// A key verb on the selection while the wheel has left it out of view
    /// (§10.1): the outline scrolls it back first, a third of the way down.
    /// A verb that changes something is held there, the status bar naming
    /// the node, and acts when pressed again with the node in view; one
    /// that changes nothing goes on. True when the verb is held.
    fn outline_held(&mut self, key: &str) -> bool {
        let (view, len) = (self.ui.outline_view, self.rows().len());
        let at = self.cursor.min(len.saturating_sub(1));
        // before the first frame there is no view to be out of
        if view == 0 || len == 0 || ui::in_view(self.outline_scroll, at, view, len) {
            return false;
        }
        self.outline_scroll = at.saturating_sub(view / 3);
        self.held(key, self.current())
    }

    /// The same for the reading pane's verbs on the line under its cursor
    /// (§10.4).
    fn reading_held(&mut self, key: &str, doc: &fold_core::reading::ReadingDoc) -> bool {
        use fold_core::reading::LineRef;
        let view = self.ui.reading_view;
        let Some(at) = self.ui.read_rows.iter().position(|&d| d == Some(self.read_cursor)) else { return false };
        if view == 0 || ui::in_view(self.scroll_reading, at, view, self.ui.read_rows.len()) {
            return false;
        }
        self.scroll_reading = at.saturating_sub(view / 3);
        // Enter acts on a title line only
        let r = match fold_core::reading::node_at(doc, self.read_cursor) {
            Some(LineRef::Title(r)) => Some(r),
            Some(LineRef::Body(r) | LineRef::Embed(r)) if key != "Enter" => Some(r),
            _ => None,
        };
        self.held(key, r)
    }

    /// Whether a verb just brought into view waits for a second press: one
    /// that would change `r` does, and says so, the key first: the name is
    /// what gives way where the bar is short.
    fn held(&mut self, key: &str, r: Option<NRef>) -> bool {
        let Some(r) = r else { return false };
        let Some(what) = self.change_words(key, r) else { return false };
        let words = format!("press {} again to {}", key, what);
        self.say(words.clone());
        self.held_words = Some(words);
        true
    }

    /// What a key would do to `r`, in the status bar's words, naming it;
    /// `None` when it changes nothing: `e a m y`, the folds, `o`, `x` on a
    /// node that is no task, a paste with nothing copied.
    fn change_words(&self, key: &str, r: NRef) -> Option<String> {
        let n = self.vault.tree.node(self.vault.tree.resolved_child(r));
        let what = match key {
            // Enter zooms into a heading, and toggles a task item (§10.4)
            "Enter" if n.kind == Kind::Section => return None,
            "x" | "Enter" => match n.task? {
                TaskState::Open => "mark {} done",
                TaskState::Done => "reopen {}",
            },
            "t" if n.task.is_some() => "remove the checkbox from {}",
            "t" => "make {} a task",
            "d" => "delete {}",
            "s" => "make {} a block",
            "J" => "move {} down",
            "K" => "move {} up",
            ">" => "indent {}",
            "<" => "outdent {}",
            "~" if n.kind == Kind::Section => "make {} a bullet",
            "~" => "make {} a heading",
            "r" => "move {} elsewhere",
            "za" => "archive {}",
            "p" | "P" if self.register.is_empty() => return None,
            "p" => "paste after {}",
            "P" => "paste before {}",
            _ => return None,
        };
        Some(what.replacen("{}", &self.named(r), 1))
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
        // a verb on a selection the wheel left out of view shows it first
        let verb = match key.code {
            _ if ctrl => None,
            KeyCode::Char(c) if "xtdsJK><~rpPeamyhlHL".contains(c) => Some(c.to_string()),
            KeyCode::Left | KeyCode::Right => Some("h".into()),
            _ => None,
        };
        if verb.is_some_and(|k| self.outline_held(&k)) {
            return;
        }
        match key.code {
            KeyCode::Char('d') if ctrl => {
                self.cursor = (self.cursor + half).min(len.saturating_sub(1));
            }
            KeyCode::Char('u') if ctrl => {
                self.cursor = self.cursor.saturating_sub(half);
            }
            _ if ctrl => self.misfire(key),
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Tab => {
                self.show_reading = true;
                self.focus = Focus::Reading;
            }
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
                    let mut nodes = Vec::new();
                    self.vault.tree.walk(r, &mut |t, n| {
                        if !t.resolved_children(n).is_empty() {
                            nodes.push(n);
                        }
                    });
                    for n in nodes {
                        self.set_folded(n, true);
                    }
                }
            }
            KeyCode::Char('L') => {
                if let Some(r) = self.current() {
                    let mut nodes = Vec::new();
                    self.vault.tree.walk(r, &mut |_, n| nodes.push(n));
                    for n in nodes {
                        self.set_folded(n, false);
                    }
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
            KeyCode::Char('?') | KeyCode::F(1) => self.run_action(Action::Help),
            KeyCode::Char('u') => self.act_undo(),
            KeyCode::Char('U') => self.act_redo(),
            KeyCode::Char('e') => self.act_edit(),
            KeyCode::Char('a') => self.act_props(),
            KeyCode::Char('r') => self.run_action(Action::Refile),
            _ => self.misfire(key),
        }
    }

    fn act_undo(&mut self) {
        self.settle_undo();
        // the entries a block in transit was remembered with may go
        self.edit_transit.clear();
        let Some(inv) = self.undo.pop() else {
            self.say("nothing to undo");
            return;
        };
        self.mark_self_write();
        self.anchor_zoom();
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
        self.edit_transit.clear();
        let Some(inv) = self.redo.pop() else {
            self.say("nothing to redo");
            return;
        };
        self.mark_self_write();
        self.anchor_zoom();
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
        self.anchor_zoom();
    }

    /// Turn the pending snapshot into an op-log entry holding exactly the
    /// files the verb changed (§10.10); a verb that changed nothing leaves
    /// no entry and keeps the redo stack.
    fn settle_undo(&mut self) {
        self.settle_zoom();
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

    /// `record_undo` for a save of the editor, its *Revert*, or a reload
    /// that re-renders it (§11.2): a block an earlier save left in transit
    /// (§5.2) and this one deletes is deleted in the entry of that save,
    /// which wrote its embed out, so one undo puts back its file and its
    /// embed together, never the file embedded nowhere (§10.10). One this
    /// save pastes back, writing its embed again, makes that save, this
    /// one and those between one entry, so no undo stops where its file is
    /// embedded nowhere either. The blocks this save leaves in transit are
    /// remembered with its entry, until their file is deleted or embedded
    /// again.
    fn record_edit(&mut self, snap: ops::Snapshot) {
        // the length of `undo` once this save's entry is in, if it has one
        let mut at = None;
        if let Some(mut inv) = ops::Inverse::since(snap, &self.vault) {
            let (transit, undo) = (&self.edit_transit, &mut self.undo);
            inv.changes.retain(|c| {
                let entry = transit.iter().find(|(p, _)| *p == c.path && c.after.is_none());
                // that entry, and none since, left the file as it was
                let Some(&(_, n)) = entry.filter(|&&(_, n)| n > 0 && n <= undo.len()) else { return true };
                if undo[n - 1..].iter().any(|e| e.changes.iter().any(|x| x.path == c.path)) {
                    return true;
                }
                undo[n - 1].changes.push(c.clone());
                false
            });
            if !inv.changes.is_empty() {
                self.undo.push(inv);
                at = Some(self.undo.len());
            }
            self.redo.clear();
            self.mark_self_write();
        }
        // pasted back: joined from the earliest save that wrote out the
        // embed of a block embedded again now
        let mut back: Vec<usize> =
            self.edit_transit.iter().filter(|(p, _)| self.embedded(p) == Some(true)).map(|&(_, n)| n).collect();
        back.sort_unstable();
        if let Some(n) = back.into_iter().find(|&n| n > 0 && self.join_undo(n - 1)) {
            // the entries from `n - 1` on are the one at `n - 1` now, if any
            let joined = self.undo.len() == n;
            at = at.and(joined.then_some(n));
            self.edit_transit.retain_mut(|(_, m)| {
                *m = (*m).min(n);
                *m < n || joined
            });
        }
        // still in transit, or pasted back with its embed not written yet
        let open = self.editor.is_some();
        let transit = std::mem::take(&mut self.edit_transit);
        self.edit_transit = transit.into_iter().filter(|(p, _)| open && self.embedded(p) == Some(false)).collect();
        if let Some(n) = at {
            for p in self.transit_files() {
                if !self.edit_transit.iter().any(|(q, _)| *q == p) {
                    self.edit_transit.push((p, n));
                }
            }
        }
    }

    /// Whether the block whose file is `path` is embedded now; `None`
    /// where the file is gone.
    fn embedded(&self, path: &str) -> Option<bool> {
        let tree = &self.vault.tree;
        let f = self.vault.file_index(path)?;
        let (_, id) = tree.blocks.iter().find(|(r, _)| r.0 == f)?;
        Some(tree.embed_of(id).is_some())
    }

    /// Make the op-log entries from `from` on one (§10.10): for each file,
    /// its text before the first and after the last; one that ends as it
    /// began is left out, and an entry left with none is dropped. False,
    /// with the entries as they were, where a file changed from outside
    /// between two of them: undoing them as one would drop that change.
    fn join_undo(&mut self, from: usize) -> bool {
        let Some(first) = self.undo.get(from) else { return false };
        let description = first.description.clone();
        let mut changes: Vec<ops::Change> = Vec::new();
        for c in self.undo[from..].iter().flat_map(|e| &e.changes) {
            match changes.iter_mut().find(|x| x.path == c.path) {
                Some(x) if x.after == c.before => x.after = c.after.clone(),
                Some(_) => return false,
                None => changes.push(c.clone()),
            }
        }
        changes.retain(|c| c.before != c.after);
        self.undo.truncate(from);
        if !changes.is_empty() {
            self.undo.push(ops::Inverse { changes, description });
        }
        true
    }

    /// The files of the blocks the open editor holds in transit (§5.2).
    fn transit_files(&self) -> Vec<String> {
        self.transit().into_iter().map(|(_, path)| path).collect()
    }

    /// The blocks the open editor holds in transit (§5.2), with their
    /// files: nested there, with no line left in it, embedded nowhere, and
    /// still in the vault.
    fn transit(&self) -> Vec<(Id, String)> {
        let Some(ed) = self.editor.as_ref() else { return Vec::new() };
        let tree = &self.vault.tree;
        ed.buf
            .owners
            .iter()
            .filter(|&(o, i)| i.parent.is_some() && !ed.buf.lines.iter().any(|l| l.owner == *o))
            .filter_map(|(_, i)| i.id.as_ref())
            .filter(|id| tree.embed_of(id).is_none())
            .filter_map(|id| Some((id.clone(), tree.files[tree.block_by_id(id)?.0].path.clone())))
            .collect()
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
        // a verb on a cursor line the wheel left out of view shows it first
        let verb = match key.code {
            KeyCode::Enter => Some("Enter".to_string()),
            KeyCode::Char(c @ ('x' | 'e' | 'a' | 'm' | 'o')) => Some(c.to_string()),
            _ => None,
        };
        if verb.is_some_and(|k| self.reading_held(&k, &doc)) {
            return;
        }
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
                        // a heading zooms, a task heading too (`x` toggles
                        // it); a task item toggles (§10.4)
                        let n = self.vault.tree.node(r);
                        if n.kind == Kind::Section {
                            self.zoom_into(r);
                        } else if n.task.is_some() {
                            self.toggle_read_task(r);
                        }
                    }
                    Some(LineRef::Embed(e)) => {
                        let t = self.vault.tree.resolved_child(e);
                        if t != e {
                            self.zoom_into(t);
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                if let Some(z) = self.zoom() {
                    self.set_zoom(self.outline_parent(z));
                    self.read_cursor = 0;
                    self.scroll_reading = 0;
                } else {
                    self.focus = Focus::Outline;
                }
            }
            KeyCode::Char('x') => {
                if let Some(r) = self.read_node() {
                    self.toggle_read_task(r);
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
            // what isn't the pane's own works as it does in the outline
            KeyCode::Char('m') => {
                // the menu keeps its own target; a later outline verb must
                // not inherit this one
                self.action_target = self.read_node();
                self.run_action(Action::NodeMenu);
                self.action_target = None;
            }
            KeyCode::Char(':' | '?' | 'c' | 'C' | 'u' | 'U') | KeyCode::F(1) => {
                let rows = self.rows();
                self.key_outline(key, rows);
            }
            _ => self.misfire(key),
        }
    }

    /// Reset the reading cursor when the pane starts showing another node
    /// (outline cursor moved, zoom changed), and keep it inside the doc.
    fn sync_read_target(&mut self) {
        let key = self.zoom().or_else(|| self.current()).map(|r| self.vault.key_of(r));
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
        let target = self.zoom().or_else(|| self.current());
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

    /// Check or uncheck a task from the reading pane (§10.4), keeping the
    /// cursor, and say which by name as `x` in the outline does.
    fn toggle_read_task(&mut self, r: NRef) {
        let Some((words, step)) = self.done_words(r) else {
            self.say(self.not_a_task(r));
            return;
        };
        self.push_undo(&step);
        match ops::toggle_task(&mut self.vault, r) {
            Ok(()) => self.say(words),
            Err(e) => self.say(format!("error: {}", e)),
        }
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
                self.mode = self.base_mode();
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
                self.mode = self.base_mode();
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
            target: self.vault.key_of(r),
            x,
            y,
            sel: 0,
            top: 0,
        });
    }

    /// The node the open menu is on, found again by its key: `None` once
    /// the files changed under the menu and that node is gone (§11.2).
    fn menu_target(&self) -> Option<NRef> {
        self.find_exact(&self.ui.menu.as_ref()?.target)
    }

    /// The node menu's items for its node (§10.1): on either side of a
    /// conflict pair, *Resolve conflict…* after the rest.
    fn menu_items(&self) -> Vec<Option<Action>> {
        let mut items = action::NODE_MENU.to_vec();
        if self.menu_target().is_some_and(|r| self.copy_of_pair(r).is_some()) {
            items.extend_from_slice(action::CONFLICT_MENU);
        }
        items
    }

    fn key_menu(&mut self, key: KeyEvent) {
        let Some(menu) = self.ui.menu.as_ref() else { return };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('m') => self.ui.menu = None,
            KeyCode::Up | KeyCode::Char('k') => self.menu_step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.menu_step(1),
            KeyCode::Enter => self.run_menu_item(menu.sel),
            _ => {}
        }
    }

    /// Move the node menu's highlight `by` items, over its separators: the
    /// keys one, the wheel three (§10.1).
    fn menu_step(&mut self, by: i32) {
        let all = self.menu_items();
        let Some(menu) = self.ui.menu.as_mut() else { return };
        let items: Vec<usize> = (0..all.len()).filter(|&i| all[i].is_some()).collect();
        let pos = items.iter().position(|&i| i == menu.sel).unwrap_or(0) as i64;
        menu.sel = items[(pos + by as i64).clamp(0, items.len() as i64 - 1) as usize];
    }

    /// Run a node-menu item on the menu's target, found by its key; the
    /// verb holds it by key across the editor's save (`run_action`).
    fn run_menu_item(&mut self, i: usize) {
        let items = self.menu_items();
        let Some(menu) = self.ui.menu.take() else { return };
        let Some(Some(a)) = items.get(i) else { return };
        let Some(target) = self.find_exact(&menu.target) else {
            self.say("that node is gone");
            return;
        };
        self.action_target = Some(target);
        self.run_action(*a);
        self.action_target = None;
    }

    /// Run any action by name (§10.8): what buttons, menus and the palette
    /// do. Node actions apply to `subject()`.
    pub fn run_action(&mut self, a: Action) {
        // §10.6: an outline verb saves the editor first, and the editor is
        // then re-rendered over what the verb wrote. Revert must not save,
        // and an action that opens the editor builds it afresh. The save
        // re-parses what it writes, and deletes a block cut and not pasted
        // back, renumbering the files after it (§5.2): a menu's or button's
        // node is held by key across it
        let target = self.action_target.map(|t| self.vault.key_of(t));
        let edit = match a {
            // help, the popups and the view toggles write nothing and go
            // back to the editor as it was: a block cut in it is still
            // there to paste (§5.2)
            Action::EditRevert
            | Action::Help
            | Action::Palette
            | Action::Filter
            | Action::Close
            | Action::HideDone
            | Action::ReadingPane
            | Action::Wrap
            | Action::RawMode
            | Action::EditorKeys => None,
            // nor do a copy, a zoom and the node menu, nor what opens a
            // prompt or the property form, whose OK is the verb
            // (`accept_prompt_saving_editor`): the editor saves as after a
            // pause, the copy taking what was typed, and is not
            // re-rendered, so a block cut in it stays in transit
            Action::Copy
            | Action::Zoom
            | Action::ZoomOut
            | Action::NodeMenu
            | Action::GoTo
            | Action::Refile
            | Action::Props
            | Action::PropAdd
            | Action::Capture
            | Action::CaptureTask => {
                self.save_editor();
                None
            }
            // the prompt's OK saves it as Enter does, its pick held by key
            Action::PromptOk => None,
            _ => self.editor_before_write("outline verb"),
        };
        self.action_target = target.as_ref().and_then(|k| self.find_exact(k));
        let gone = target.is_some() && self.action_target.is_none();
        if gone {
            self.say("that node is gone");
        } else {
            self.run_action_inner(a);
        }
        if gone || !matches!(a, Action::Edit | Action::NewSibling | Action::NewChild | Action::ConflictEdit) {
            self.editor_after_write(edit);
        }
    }

    fn run_action_inner(&mut self, a: Action) {
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
            Action::Undo if self.mode == Mode::Conflict => self.key_conflict(key_of('u')),
            Action::Redo if self.mode == Mode::Conflict => self.key_conflict(key_of('U')),
            Action::Undo => self.act_undo(),
            Action::Redo => self.act_redo(),
            Action::Capture => self.open_prompt("capture", PromptAction::CaptureText(false), String::new()),
            Action::CaptureTask => {
                self.open_prompt("capture task", PromptAction::CaptureText(true), String::new())
            }
            Action::HideDone => self.toggle_hide_done(),
            Action::ReadingPane => {
                self.show_reading = !self.show_reading;
                if !self.show_reading && self.focus == Focus::Reading {
                    self.focus = Focus::Outline;
                }
                self.say(if self.show_reading { "reading pane shown" } else { "reading pane hidden" });
            }
            Action::Wrap => {
                self.wrap = !self.wrap;
                self.say(if self.wrap { "long lines wrap" } else { "long lines are cut at the edge" });
            }
            Action::RawMode => {
                self.raw_mode = !self.raw_mode;
                self.say(if self.raw_mode { "raw" } else { "styled" });
            }
            Action::GoTo => self.open_prompt("go to", PromptAction::GoTo, String::new()),
            Action::ClearDone => self.act_clear_done(),
            Action::Canonicalize => {
                self.anchor_zoom();
                match fold_core::check::fix(&mut self.vault) {
                    Ok(n) => self.say(format!("{} file(s) canonicalized", n)),
                    Err(e) => self.say(format!("error: {}", e)),
                }
            }
            Action::Merge => {
                self.anchor_zoom();
                match fold_core::merge::merge_sync_conflicts(&mut self.vault, false) {
                    Ok(o) => {
                        self.say(format!("{} merge(s)", o.len()));
                        if !fold_core::merge::conflict_pairs(&self.vault).is_empty() {
                            self.enter_conflict_view();
                        }
                    }
                    Err(e) => self.say(format!("error: {}", e)),
                }
            }
            Action::ResolveConflicts => self.enter_conflict_view(),
            Action::ResolveConflict => {
                if let Some(r) = self.subject() {
                    self.enter_conflict_view_at(r);
                }
            }
            Action::EditorKeys => self.set_edit_keys(self.edit_keys.next()),
            Action::EditDone => {
                self.close_editor();
            }
            Action::EditRevert => self.discard_editor(),
            Action::Close => self.close_top(),
            Action::PromptOk => {
                if let Some(p) = self.prompt.take() {
                    self.accept_prompt_saving_editor(p);
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

    /// The mode under a popup (§10.2): the editor if one is open, since a
    /// popup opened over it closes back to it, else normal.
    fn base_mode(&self) -> Mode {
        if self.editor.is_some() {
            Mode::Edit
        } else {
            Mode::Normal
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
            Mode::Edit => {
                self.close_editor();
            }
            Mode::Filter => {
                self.mode = self.base_mode();
                self.filter.clear();
                self.filter_rows.clear();
            }
            Mode::Picker => {
                self.mode = self.base_mode();
                self.palette.clear();
            }
            Mode::Props | Mode::Help | Mode::Conflict => self.mode = self.base_mode(),
            Mode::Normal => {}
        }
    }

    /// Zoom into a node: the reading pane shows it (§10.3 `Enter`).
    fn zoom_into(&mut self, r: NRef) {
        if self.vault.tree.node(r).kind != Kind::Root {
            self.set_zoom(Some(r));
            self.cursor = 0;
            self.read_cursor = 0;
            self.scroll_reading = 0;
            if self.reading_visible() {
                self.focus = Focus::Reading;
            }
        }
    }

    /// Zoom out to the parent (§10.3 `Backspace`): the outline parent,
    /// so a zoomed block goes to the node that embeds it.
    fn zoom_out(&mut self) {
        if let Some(z) = self.zoom() {
            let key = self.vault.key_of(z);
            self.set_zoom(self.outline_parent(z));
            self.cursor = 0;
            if let Some(r) = self.vault.find_by_key(&key) {
                self.move_cursor_to(r);
            }
        }
    }

    /// Zoom straight to a node, or to the root (breadcrumbs, §10.1).
    fn zoom_to(&mut self, r: Option<NRef>) {
        let prev = self.zoom();
        self.set_zoom(r);
        self.cursor = 0;
        if let Some(p) = prev {
            if !self.rows().iter().any(|row| row.nref == p) {
                return;
            }
            self.move_cursor_to(p);
        }
    }

}

/// A status message, saying so when the ordering rule placed a node of
/// `kind` other than asked (§3.1: items before sections).
fn with_rule_note(what: &str, moved: bool, kind: Kind) -> String {
    match (moved, kind) {
        (false, _) => what.to_string(),
        (true, Kind::Section) => format!("{} — placed after the items", what),
        (true, _) => format!("{} — placed before the sections", what),
    }
}

/// The help text, shared by `?` in the TUI and `fold help` (§10.8).
pub fn help_text() -> Vec<Line<'static>> {
    let entries: &[(&str, &str)] = &[
        ("", "fold — notes and tasks as one outline of plain Markdown."),
        ("", ""),
        ("MOUSE", ""),
        ("click", "select · ▸/▾ fold · ☐ toggle · breadcrumb segments zoom out"),
        ("", "  · ⚠ on a conflict copy: both sides, to resolve"),
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
        ("s za zd zr zw zp", "make block · archive · hide done · raw source · wrap lines · reading pane"),
        ("/ : ? F1", "filter · command palette · this help (F1 in the editor too)"),
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

/// What an editor save's op-log entry names (§10.10), *edit “ZFS
/// layout”*: the block it writes when it writes one, else the node the
/// editor is open on.
fn edited(ed: &editor::Editor, owner: Option<&fold_core::edit::Owner>) -> String {
    let one = owner.or(match ed.buf.dirty.as_slice() {
        [o] => Some(o),
        _ => None,
    });
    let info = one.and_then(|o| ed.buf.owners.get(o)).or_else(|| ed.buf.owners.values().find(|i| i.parent.is_none()));
    quoted(info.map_or("", |i| i.title.as_str()))
}

/// A title as the status bar names it: quoted.
fn quoted(title: &str) -> String {
    format!("“{}”", if title.is_empty() { "(untitled)" } else { title })
}

/// The status bar's count of unresolved pairs (§10.1), as its ⚠ reads.
fn conflict_count(n: usize) -> String {
    format!("⚠ {} conflict{}", n, if n == 1 { "" } else { "s" })
}

/// An error or a refusal, which the status bar keeps until the next key
/// after it was read (§10.1).
fn lasting(msg: &str) -> bool {
    msg.starts_with("error:") || msg.starts_with("can't ") || [" error:", " refused:", " failed:"].iter().any(|w| msg.contains(w))
}

/// A key as the status bar names it: *q*, *Enter*, *Ctrl-Z*.
fn key_name(key: KeyEvent) -> String {
    let name = match key.code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => c.to_uppercase().to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("F{}", n),
        KeyCode::Up => "↑".into(),
        KeyCode::Down => "↓".into(),
        KeyCode::Left => "←".into(),
        KeyCode::Right => "→".into(),
        KeyCode::BackTab => "Shift-Tab".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        code => format!("{:?}", code),
    };
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        format!("Ctrl-{}", name)
    } else {
        name
    }
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
/// `vim`, `helix`), overriding `$FOLD_KEYS`. However the session ends — a
/// quit, a signal, an error, a panic — the terminal is put back, and the
/// editor saved or its text kept in the trash (§10.6).
pub fn run(dir: &Path, keys: Option<&str>) -> anyhow::Result<()> {
    let mut app = App::new(dir)?;
    if let Some(k) = keys {
        let parsed = editor::Keys::parse(k)
            .ok_or_else(|| anyhow::anyhow!("unknown keymap {:?}: use normal, vim or helix", k))?;
        app.edit_keys = parsed;
    }
    let keys_chosen = keys.is_some() || std::env::var_os("FOLD_KEYS").is_some();
    if keys_chosen {
        // no view yet: it remembers no keymap from this run either
        app.kept_keys = Some(None);
    }
    if let Some(v) = view::load(dir) {
        app.apply_view(v, keys_chosen);
    }
    app.start_watcher();
    app.merge_on_startup();
    let stop = stop_signals()?;
    let screen = Screen::enter()?;
    // a panic is caught here once its message is out (`Screen`), so the
    // editor is still saved or kept before it goes on
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
        run_loop(&mut terminal, &mut app, &stop)
    }));
    // a quit closed the editor; any other end saves it, or keeps its text
    let kept = app.keep_unsaved();
    let _ = view::save(dir, &app.view());
    drop(screen);
    if let Some(words) = kept {
        use std::io::Write;
        // the terminal may be gone (a closed window): nothing to say it on
        let _ = writeln!(std::io::stderr(), "{}", words);
    }
    match res {
        Ok(res) => res,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// SIGTERM, SIGHUP (a closed window, a dropped ssh session) and SIGINT
/// set a flag the main loop checks, so fold ends as on a quit, the editor
/// saved or its text kept (§10.6), instead of dying mid-sentence.
fn stop_signals() -> std::io::Result<std::sync::Arc<std::sync::atomic::AtomicBool>> {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // set by SIGTERM and SIGINT alone: a SIGHUP before one does not make
    // it a second
    #[cfg(unix)]
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(unix)]
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP, signal_hook::consts::SIGINT] {
        // a second SIGTERM or SIGINT ends fold as the signal would (§10.6):
        // the loop never sees the flag while a write waits on a terminal
        // that reads nothing. not SIGHUP, which a closed window can send twice
        if sig != signal_hook::consts::SIGHUP {
            signal_hook::flag::register_conditional_default(sig, asked.clone())?;
            signal_hook::flag::register(sig, asked.clone())?;
        }
        signal_hook::flag::register(sig, stop.clone())?;
    }
    Ok(stop)
}

thread_local! {
    /// Whether this thread has the terminal as the TUI sets it up.
    static ON_SCREEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The terminal as the TUI sets it up — raw, the alternate screen, mouse
/// capture, bracketed paste — put back as it was when this drops, however
/// the session ends. A panic puts it back before its message is printed,
/// so the message is not lost with the alternate screen.
struct Screen;

impl Screen {
    fn enter() -> std::io::Result<Screen> {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // a panic on another thread leaves the screen be
            leave_screen();
            hook(info);
        }));
        enable_raw_mode()?;
        ON_SCREEN.set(true);
        let screen = Screen;
        std::io::stdout().execute(EnterAlternateScreen)?;
        std::io::stdout().execute(EnableMouseCapture)?;
        std::io::stdout().execute(EnableBracketedPaste)?;
        Ok(screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        leave_screen();
    }
}

/// Put the terminal back, once, if this thread set it up. Errors are
/// ignored: the terminal may be gone.
fn leave_screen() {
    if !ON_SCREEN.try_with(|s| s.replace(false)).unwrap_or(false) {
        return;
    }
    let _ = disable_raw_mode();
    let mut out = std::io::stdout();
    let _ = out.execute(DisableBracketedPaste);
    let _ = out.execute(DisableMouseCapture);
    let _ = out.execute(SetCursorStyle::DefaultUserShape);
    let _ = out.execute(crossterm::cursor::Show);
    let _ = out.execute(LeaveAlternateScreen);
}

/// A top-level node before or after a reload: what it holds, and a hash of
/// its text, to say what changed (§11.2).
#[derive(Clone)]
struct Top {
    key: NodeKey,
    title: String,
    items: usize,
    sections: usize,
    text: u64,
}

/// A vault's top-level nodes, as `Top`s.
fn tops(vault: &Vault) -> Vec<Top> {
    use std::hash::{Hash, Hasher};
    let tree = &vault.tree;
    tree.resolved_children(tree.root)
        .into_iter()
        .map(|top| {
            let (mut items, mut sections) = (0, 0);
            let mut text = std::collections::hash_map::DefaultHasher::new();
            tree.walk(top, &mut |t, r| {
                let n = t.node(r);
                let file = t.text_of(r);
                n.title_span.text(file).hash(&mut text);
                n.text_lines(file).hash(&mut text);
                n.block.as_ref().map(|b| &b.frontmatter_raw).hash(&mut text);
                match n.kind {
                    _ if r == top => {}
                    Kind::Item => items += 1,
                    Kind::Section => sections += 1,
                    Kind::Root => {}
                }
            });
            Top { key: vault.key_of(top), title: tree.node(top).title.clone(), items, sections, text: text.finish() }
        })
        .collect()
}

/// The files as read before the editor's save, `after`, with what a merge
/// of sync-conflict copies then took in over them (§11.2): each top-level
/// node it changed from `was` to `now` moves by as much, one it brought
/// comes in and one gone by then goes. What the save typed, in `was` and
/// `now` both, is not laid over.
fn merged_over(mut after: Vec<Top>, was: &[Top], now: Vec<Top>) -> Vec<Top> {
    after.retain(|a| !was.iter().any(|w| w.key == a.key) || now.iter().any(|n| n.key == a.key));
    for n in now {
        match (was.iter().find(|w| w.key == n.key), after.iter_mut().find(|a| a.key == n.key)) {
            (Some(w), Some(a)) if w.text != n.text => {
                a.items = (a.items + n.items).saturating_sub(w.items);
                a.sections = (a.sections + n.sections).saturating_sub(w.sections);
                a.title = n.title;
                a.text = n.text;
            }
            (None, None) => after.push(n),
            _ => {}
        }
    }
    after
}

/// Whether two outlines' top-level nodes are the same, text and all:
/// nothing came in (§11.2).
fn same_tops(before: &[Top], after: &[Top]) -> bool {
    before.iter().map(|t| (&t.key, t.text)).eq(after.iter().map(|t| (&t.key, t.text)))
}

/// What a reload took in, in outline terms (§11.2): each top-level node
/// that changed, came or went, and the items or sections it gained or lost.
/// With `typed`, that the editor's typing was saved first comes ahead of
/// them, which give way where the status bar is short (§10.6).
fn changed_outside(before: &[Top], after: &[Top], typed: bool) -> String {
    let mut parts = Vec::new();
    for a in after {
        match before.iter().find(|b| b.key == a.key) {
            None => parts.push(format!("{} (new)", a.title)),
            Some(b) if b.text != a.text => {
                let counts: Vec<String> = [(b.items, a.items, "item"), (b.sections, a.sections, "section")]
                    .into_iter()
                    .filter(|(was, now, _)| was != now)
                    .map(|(was, now, what)| {
                        let n = was.abs_diff(now);
                        format!("{}{} {}{}", if now > was { "+" } else { "−" }, n, what, if n == 1 { "" } else { "s" })
                    })
                    .collect();
                let what = if counts.is_empty() { "edited".to_string() } else { counts.join(", ") };
                parts.push(format!("{} ({})", a.title, what));
            }
            Some(_) => {}
        }
    }
    for b in before.iter().filter(|b| !after.iter().any(|a| a.key == b.key)) {
        parts.push(format!("{} (removed)", b.title));
    }
    let lead = if typed { "↻ typing saved; changed outside fold" } else { "↻ changed outside fold" };
    match parts.len() {
        0 => lead.into(),
        n if n > 3 => format!("{}: {} and {} more", lead, parts[..2].join(", "), n - 2),
        _ => format!("{}: {}", lead, parts.join(", ")),
    }
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
    stop: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<()> {
    let events = read_events();
    let mut block = None;
    loop {
        // a signal ends the session (`stop_signals`)
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
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
            // save any open editor on quit (§10.6); one whose save is
            // refused stays open with its text
            if app.editor.is_none() || app.close_editor() {
                return Ok(());
            }
            app.quit = false;
        }
        app.tick();
        let event = match events.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => event?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("no more input from the terminal"),
        };
        match event {
            Event::Mouse(m) => app.handle_mouse(m),
            Event::Key(key) => app.handle_key(key),
            Event::Paste(text) => app.handle_paste(&text),
            _ => {}
        }
    }
}

/// The terminal's events, read on a thread of their own: once the terminal
/// is gone (a closed window), crossterm reads it without end, and the main
/// loop must still come round to the signal that says so.
fn read_events() -> std::sync::mpsc::Receiver<std::io::Result<Event>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || loop {
        let event = event::read();
        let failed = event.is_err();
        if tx.send(event).is_err() || failed {
            return;
        }
    });
    rx
}
