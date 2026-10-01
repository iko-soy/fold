//! fold-ffi: fold-core for other languages, through UniFFI. It is what the
//! Android app (`android/`) calls: one `Session` over a vault, holding what
//! the TUI holds between keys (the op log, the register, the open editor)
//! and answering in the terms a screen needs — outline rows, reading lines,
//! a message after every verb. Nodes are named by keys (§3.4), strings the
//! app keeps and hands back; every write goes through fold-core, so the
//! files come out exactly as the TUI and the CLI write them.

mod edit;
mod keys;
mod news;

use edit::TextEditor;
use fold_core::edit::{open_editor, OwnerInfo};
use fold_core::ident::Id;
use fold_core::ops;
use fold_core::parse::{Kind, TaskState};
use fold_core::render::{render, render_lines, LineKind};
use fold_core::tree::NRef;
use fold_core::vault::{NodeKey, Vault};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

uniffi::setup_scaffolding!();

// ------------------------------------------------------------ the types

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FoldError {
    #[error("{0}")]
    Failed(String),
}

/// A node's checkbox (§4.5).
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    None,
    Open,
    Done,
}

/// How a node is spelled (§3.1).
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spelling {
    Section,
    Item,
}

/// What a verb did, in the status bar's words (§10.1).
#[derive(uniffi::Record, Clone, Debug)]
pub struct OpResult {
    pub ok: bool,
    pub message: String,
    /// The node the verb acted on or made, where it is now.
    pub node: Option<String>,
    /// A verb run while the editor is open saves it first and re-renders it
    /// over what it wrote (§10.6): its new text, the generation the text
    /// field's updates now name, and whether its node is gone and it closed.
    pub editor_text: Option<String>,
    pub editor_generation: u32,
    pub editor_closed: bool,
}

impl OpResult {
    fn new(ok: bool, message: impl Into<String>, node: Option<String>) -> OpResult {
        OpResult {
            ok,
            message: message.into(),
            node,
            editor_text: None,
            editor_generation: 0,
            editor_closed: false,
        }
    }
}

/// The op-log entry an undo is meant for: the one a message offered to
/// undo, so a tap on it never undoes a later change (§10.10).
#[derive(uniffi::Record, Clone, Debug)]
pub struct UndoMark {
    /// How many entries the op log held with it on top.
    pub depth: u32,
    pub description: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct OutlineQuery {
    /// The zoom root; none for the whole vault.
    pub zoom: Option<String>,
    /// Folded nodes (§10.1).
    pub folded: Vec<String>,
    /// Conflict copies unfolded: a copy starts folded (§10.1).
    pub unfolded: Vec<String>,
    /// `zd`: done tasks hidden (§8.5).
    pub hide_done: bool,
}

/// One row of the outline (§10.1).
#[derive(uniffi::Record, Clone, Debug)]
pub struct Row {
    pub key: String,
    pub title: String,
    pub depth: u32,
    pub spelling: Spelling,
    pub task: Task,
    /// Has its own file (`▤`).
    pub block: bool,
    /// A conflict copy: whose copy it is, *PHONE 09-27 10:00* (§12.5).
    pub conflict: Option<String>,
    /// An embed whose block is missing or embedded elsewhere (§6.2).
    pub broken: bool,
    pub has_children: bool,
    pub folded: bool,
    /// Open and total tasks below it, itself not counted (§3.5).
    pub open: u32,
    pub total: u32,
    pub due: Option<String>,
    /// The first line of its own text (§10.1).
    pub preview: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct Crumb {
    pub key: String,
    pub title: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct Property {
    pub key: String,
    pub value: String,
    /// False for a frontmatter line fold does not understand: kept as it
    /// is, never edited (§3.2).
    pub editable: bool,
}

/// The zoom root, shown above its children.
#[derive(uniffi::Record, Clone, Debug)]
pub struct Header {
    pub key: String,
    pub title: String,
    pub spelling: Spelling,
    pub task: Task,
    pub block: bool,
    pub conflict: Option<String>,
    pub due: Option<String>,
    pub open: u32,
    pub total: u32,
    /// Its body: the text before its first child (§3.3), as rendered.
    pub body: Vec<String>,
    pub props: Vec<Property>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct Outline {
    pub zoom: Option<Header>,
    pub crumbs: Vec<Crumb>,
    pub rows: Vec<Row>,
    /// Unresolved conflict pairs (§12.5).
    pub conflicts: u32,
    /// What undo and redo would do, named (§10.10).
    pub undo: Option<String>,
    pub redo: Option<String>,
    /// The op log's depth, for `UndoMark`.
    pub undo_depth: u32,
    /// What the register holds, for *Paste*.
    pub copied: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct NodeInfo {
    pub key: String,
    pub title: String,
    pub spelling: Spelling,
    pub task: Task,
    pub block: bool,
    pub conflict: Option<String>,
    /// One side of a conflict pair (§12.5).
    pub paired: bool,
    pub path: Vec<Crumb>,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadKind {
    Title,
    Body,
    Embed,
    Blank,
}

/// One line of the reading view: `render(node, 1, true)` (§5.1).
#[derive(uniffi::Record, Clone, Debug)]
pub struct ReadLine {
    pub kind: ReadKind,
    pub text: String,
    pub node: Option<String>,
    /// For a title line: its indent, heading level (0 for a bullet), task
    /// and title, and a block's properties as one dimmed line (§10.1).
    pub indent: u32,
    pub heading: u32,
    pub task: Task,
    pub title: String,
    pub props: Option<String>,
    pub conflict: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct Hit {
    pub key: String,
    pub title: String,
    /// Its ancestors' titles, `Homelab › NAS`.
    pub path: String,
    /// Its parent in the outline; none at the top.
    pub parent: Option<String>,
    pub task: Task,
    pub conflict: bool,
    /// For a match in its text rather than its title: that line.
    pub excerpt: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ConflictPair {
    pub ours: String,
    pub theirs: String,
    pub title: String,
    pub from: String,
    pub ours_text: String,
    pub theirs_text: String,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    Ours,
    Theirs,
    Both,
}

/// What a look at the files found (§11.2).
#[derive(uniffi::Record, Clone, Debug)]
pub struct Refresh {
    /// The vault was re-read: everything shown should be asked for again.
    pub changed: bool,
    pub message: Option<String>,
    /// Conflict pairs a merge just raised (§12.4).
    pub raised: u32,
    /// The open editor was re-rendered over a change from outside: its new
    /// text, and the generation the text field's updates now name.
    pub editor_text: Option<String>,
    pub editor_generation: u32,
    /// The node the editor was open on is gone, and the editor closed.
    pub editor_closed: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct EditorView {
    pub text: String,
    pub title: String,
    pub node: String,
    /// What `edit_update` names the text by (`Session::edit_update`).
    pub generation: u32,
}

/// The editor's text after its own undo or redo, and its new generation.
#[derive(uniffi::Record, Clone, Debug)]
pub struct EditorText {
    pub text: String,
    pub generation: u32,
}

/// After a save or a close of the editor.
#[derive(uniffi::Record, Clone, Debug)]
pub struct Saved {
    pub ok: bool,
    pub message: String,
    pub dirty: bool,
    /// Where the edited node is now: its title, and so its key, may be what
    /// was edited.
    pub node: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct EditorState {
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TrashEntry {
    pub name: String,
    pub bytes: u64,
}

// ------------------------------------------------------------ the session

#[derive(uniffi::Object)]
pub struct Session {
    state: Mutex<State>,
}

struct State {
    vault: Vault,
    undo: Vec<ops::Inverse>,
    redo: Vec<ops::Inverse>,
    /// The register (§10.3 `y`/`d`): text, what it is, its spelling.
    register: Option<(String, String, Kind)>,
    editor: Option<TextEditor>,
    /// Bumped whenever the editor's text is replaced from this side (a
    /// re-render, its undo): an update from the text field made before the
    /// field took the new text names an older one, and is dropped.
    edit_generation: u32,
    /// A save of the editor's text failed and none has gone through since.
    edit_refused: bool,
    /// The editor's text as it was when *Revert* could not copy it to the
    /// trash: *Revert* again on the same text drops it.
    edit_uncopied: Option<String>,
    /// The file of each block an editor save left in transit (§5.2), with
    /// the length of `undo` once that save's entry was in (§10.10).
    edit_transit: Vec<(String, usize)>,
    /// The sync-conflict copies the last look at the files listed, and
    /// still there after it: a new one is a change to take in (§11.2).
    conflict_copies: Vec<String>,
}

#[uniffi::export]
impl Session {
    /// Open a vault, creating `root.md` in an empty directory (§4.1.1).
    #[uniffi::constructor]
    pub fn open(dir: String) -> Result<Arc<Session>, FoldError> {
        let vault = Vault::open(Path::new(&dir)).map_err(|e| FoldError::Failed(format!("{}: {}", dir, e)))?;
        Ok(Arc::new(Session {
            state: Mutex::new(State {
                vault,
                undo: Vec::new(),
                redo: Vec::new(),
                register: None,
                editor: None,
                edit_generation: 0,
                edit_refused: false,
                edit_uncopied: None,
                edit_transit: Vec::new(),
                // none yet: the first look at the files merges any there
                // (§12.2, the startup scan)
                conflict_copies: Vec::new(),
            }),
        }))
    }

    pub fn dir(&self) -> String {
        self.lock().vault.dir.to_string_lossy().into_owned()
    }

    // ------------------------------------------------------------ reading

    pub fn outline(&self, query: OutlineQuery) -> Outline {
        self.lock().outline(&query)
    }

    pub fn node(&self, key: String) -> Option<NodeInfo> {
        let s = self.lock();
        let r = s.find(&key)?;
        Some(s.node_info(r))
    }

    /// The reading view of a node, the vault's root for none (§5.4).
    pub fn reading(&self, key: Option<String>) -> Vec<ReadLine> {
        let s = self.lock();
        let r = match key {
            Some(k) => match s.find(&k) {
                Some(r) => r,
                None => return Vec::new(),
            },
            None => s.vault.tree.root,
        };
        s.reading(r)
    }

    /// Exact source, frontmatter included (§10.3 `zr`).
    pub fn source(&self, key: String) -> Option<String> {
        let s = self.lock();
        let r = s.find(&key)?;
        let tree = &s.vault.tree;
        let n = tree.node(r);
        let text = tree.text_of(r);
        Some(if n.is_block() { text.to_string() } else { n.span.text(text).to_string() })
    }

    /// The filter box (§10.5): titles first, then text.
    pub fn search(&self, query: String) -> Vec<Hit> {
        self.lock().search(&query)
    }

    /// *Move to…* candidates (§10.1): every node `moving` may go under.
    pub fn targets(&self, query: String, moving: Option<String>) -> Vec<Hit> {
        self.lock().targets(&query, moving.as_deref())
    }

    pub fn properties(&self, key: String) -> Vec<Property> {
        let s = self.lock();
        s.find(&key).map(|r| s.properties(r)).unwrap_or_default()
    }

    pub fn conflicts(&self) -> Vec<ConflictPair> {
        self.lock().conflicts()
    }

    /// `fold check` (§15.7).
    pub fn diagnostics(&self) -> Vec<String> {
        let s = self.lock();
        let mut out: Vec<String> = fold_core::check::check(&s.vault).iter().map(|d| d.to_string()).collect();
        if let Ok(ignored) = s.vault.ignored_files() {
            out.extend(ignored.into_iter().map(|f| format!("{}: ignored: {}", f.path, f.reason)));
        }
        out
    }

    // ------------------------------------------------------------ verbs

    /// `x`: toggle a task open / done (§8.1).
    pub fn toggle_task(&self, key: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.toggle_task(&key))
    }

    /// `t`: add or remove the checkbox (§10.3).
    pub fn toggle_taskness(&self, key: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.toggle_taskness(&key))
    }

    /// `n` / `N`: a new node titled `title` after `key`, or as its last
    /// child; under the vault's root for no key.
    pub fn add_node(&self, key: Option<String>, title: String, task: bool, child: bool) -> OpResult {
        self.lock().with_editor_saved(|s| s.add_node(key.as_deref(), &title, task, child))
    }

    pub fn delete(&self, key: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.delete(&key))
    }

    pub fn copy(&self, key: String) -> OpResult {
        self.lock().copy(&key)
    }

    pub fn paste(&self, key: String, after: bool) -> OpResult {
        self.lock().with_editor_saved(|s| s.paste(&key, after))
    }

    /// `J` / `K` (§10.3).
    pub fn move_sibling(&self, key: String, down: bool) -> OpResult {
        self.lock().with_editor_saved(|s| s.move_sibling(&key, down))
    }

    /// `>` (§10.3).
    pub fn indent(&self, key: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.indent(&key))
    }

    /// `<` (§10.3).
    pub fn outdent(&self, key: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.outdent(&key))
    }

    /// *Move to…* (§6.5).
    pub fn move_to(&self, key: String, dest: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.move_to(&key, &dest))
    }

    /// `za` (§6.5).
    pub fn archive(&self, key: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.archive(&key))
    }

    /// `~` (§10.3).
    pub fn toggle_spelling(&self, key: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.toggle_spelling(&key))
    }

    /// `s` (§6.1).
    pub fn make_block(&self, key: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.make_block(&key))
    }

    /// A property, from the property form (§10.6); the first one on a plain
    /// node makes it a block.
    pub fn set_property(&self, key: String, name: String, value: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.set_property(&key, &name, &value))
    }

    pub fn remove_property(&self, key: String, name: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.remove_property(&key, &name))
    }

    /// `c` / `C` (§7).
    pub fn capture(&self, text: String, task: bool) -> OpResult {
        self.lock().with_editor_saved(|s| s.capture(&text, task))
    }

    /// *Clear done* (§8.5), under `zoom` or everywhere.
    pub fn clear_done(&self, zoom: Option<String>) -> OpResult {
        self.lock().with_editor_saved(|s| s.clear_done(zoom.as_deref()))
    }

    /// The conflict view's choices (§12.5), on the pair whose copy is
    /// `theirs`.
    pub fn resolve(&self, theirs: String, keep: Keep) -> OpResult {
        self.lock().with_editor_saved(|s| s.resolve(&theirs, keep))
    }

    /// *Canonicalize*: `fold check --fix` (§4.2).
    pub fn canonicalize(&self) -> OpResult {
        self.lock().with_editor_saved(|s| s.canonicalize())
    }

    /// Undo the last change; with `mark`, only if it is still the change
    /// the mark names.
    pub fn undo(&self, mark: Option<UndoMark>) -> OpResult {
        let mut s = self.lock();
        if let Some(m) = mark {
            let top = s.undo.last().map(|e| e.description.as_str());
            if s.undo.len() as u32 != m.depth || top != Some(m.description.as_str()) {
                return State::refused(match top {
                    Some(d) => format!("not undone: the last change is now {}", d),
                    None => "nothing to undo".into(),
                });
            }
        }
        s.undo_redo(true)
    }

    pub fn redo(&self) -> OpResult {
        self.lock().undo_redo(false)
    }

    // ------------------------------------------------------------ files

    /// Look at the files (§11.2): take in what another program or a sync
    /// changed, merge new sync-conflict copies (§12.2), save the editor
    /// first and re-render it where what it shows changed. With `force`,
    /// re-read the vault even when nothing seems to have changed.
    pub fn refresh(&self, force: bool) -> Refresh {
        self.lock().refresh(force)
    }

    pub fn trash(&self) -> Vec<TrashEntry> {
        let mut out: Vec<TrashEntry> = std::fs::read_dir(fold_core::vault::trash_dir())
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| TrashEntry {
                        name: e.file_name().to_string_lossy().into_owned(),
                        bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| b.name.cmp(&a.name));
        out
    }

    pub fn trash_text(&self, name: String) -> Option<String> {
        let path = trash_entry(&name)?;
        std::fs::read_to_string(path).ok()
    }

    /// `fold trash restore` (§11.5).
    pub fn restore(&self, name: String) -> OpResult {
        self.lock().with_editor_saved(|s| s.restore(&name))
    }

    // ------------------------------------------------------------ editor

    /// Open the editor on a node: `render(node, 1, true)` with every line
    /// tagged with its block (§5.2, §10.6).
    pub fn edit_open(&self, key: String) -> Result<EditorView, FoldError> {
        self.lock().edit_open(&key).map_err(FoldError::Failed)
    }

    /// The text field's text after a change; `cursor`, the caret after it,
    /// in characters. `generation` is the one the field's text came with:
    /// a change typed while the editor's text was being replaced (a sync
    /// re-rendered it, its undo ran) was made to text that is gone, and is
    /// dropped rather than laid over the new text.
    pub fn edit_update(&self, text: String, cursor: Option<u32>, generation: u32) -> EditorState {
        let mut s = self.lock();
        let current = s.edit_generation == generation;
        if let Some(ed) = s.editor.as_mut().filter(|_| current) {
            ed.apply(&text, cursor.map(|c| c as usize));
        }
        s.editor_state()
    }

    /// The editor's text, as the buffer has it.
    pub fn edit_text(&self) -> Option<String> {
        self.lock().editor.as_ref().map(|e| e.text())
    }

    /// Save the dirty blocks: after a pause in typing, or when asked (§10.6).
    pub fn edit_save(&self) -> Saved {
        let mut s = self.lock();
        let res = s.write_editor(false);
        s.saved(res)
    }

    /// Done: save, and leave the editor. Refused saves keep it open with
    /// its text (§10.6).
    pub fn edit_close(&self) -> Saved {
        let mut s = self.lock();
        let res = s.write_editor(true);
        let saved = s.saved(res);
        if saved.ok {
            s.editor = None;
        }
        saved
    }

    /// *Revert* (§10.6): drop what was typed since the last save.
    pub fn edit_revert(&self) -> OpResult {
        self.lock().edit_revert()
    }

    /// The editor's own undo, inside it (§10.6); the new text, if any.
    pub fn edit_undo(&self) -> Option<EditorText> {
        self.lock().edit_history(true)
    }

    pub fn edit_redo(&self) -> Option<EditorText> {
        self.lock().edit_history(false)
    }

    /// The app is going away, maybe for good (§10.6): save what can be
    /// saved; text no save can take goes to the trash whole (§11.5). What to
    /// say, if anything. With `release`, as when the vault closes, a block
    /// cut and not pasted back is deleted as on leaving the editor (§5.2);
    /// without, as when the app only goes to the background, it stays in
    /// transit for the paste the user may be on the way to.
    pub fn edit_keep(&self, release: bool) -> Option<String> {
        self.lock().edit_keep(release)
    }
}

/// A trash entry by name, never outside the trash.
fn trash_entry(name: &str) -> Option<PathBuf> {
    if name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return None;
    }
    let p = fold_core::vault::trash_dir().join(name);
    p.is_file().then_some(p)
}

impl Session {
    fn lock(&self) -> MutexGuard<'_, State> {
        // a panic in one call leaves the state as it was written so far;
        // the files are the truth and the next look at them reloads
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A title as messages name it: quoted.
fn quoted(title: &str) -> String {
    format!("“{}”", if title.is_empty() { "(untitled)" } else { title })
}

fn task_of(t: Option<TaskState>) -> Task {
    match t {
        None => Task::None,
        Some(TaskState::Open) => Task::Open,
        Some(TaskState::Done) => Task::Done,
    }
}

fn spelling_of(k: Kind) -> Spelling {
    match k {
        Kind::Section => Spelling::Section,
        _ => Spelling::Item,
    }
}

/// Where a conflict copy came from (§12.4), as the screen says it: the
/// merge's *PHONE 20260927-100000* reads *PHONE 09-27 10:00*.
fn copy_from(conflict: &str) -> String {
    let Some((device, t)) = conflict.rsplit_once(' ') else { return conflict.to_string() };
    let digits = |r: std::ops::Range<usize>| t.get(r).is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit()));
    if t.len() == 15 && t.as_bytes()[8] == b'-' && digits(0..8) && digits(9..15) {
        format!("{} {}-{} {}:{}", device, &t[4..6], &t[6..8], &t[9..11], &t[11..13])
    } else {
        conflict.to_string()
    }
}

/// What a verb's message adds when the ordering rule chose the place (§3.1).
fn with_rule_note(what: &str, moved: bool, kind: Kind) -> String {
    match (moved, kind) {
        (false, _) => what.to_string(),
        (true, Kind::Section) => format!("{} — placed after the items", what),
        (true, _) => format!("{} — placed before the sections", what),
    }
}

fn fuzzy_match(needle: &str, hay: &str) -> bool {
    let mut n = needle.chars().peekable();
    for c in hay.chars() {
        if n.peek() == Some(&c) {
            n.next();
        }
    }
    n.peek().is_none()
}

type Verb<'a> = Box<dyn FnOnce(&mut State) -> Result<(String, Option<String>), String> + 'a>;

impl State {
    // ------------------------------------------------------------ finding

    fn key(&self, r: NRef) -> String {
        keys::encode(&self.vault.key_of(r))
    }

    /// The node a key names, only if it is that very node: no falling back
    /// to the deepest step that still exists.
    fn find(&self, key: &str) -> Option<NRef> {
        let k = keys::decode(key)?;
        self.find_exact(&k)
    }

    fn find_exact(&self, k: &NodeKey) -> Option<NRef> {
        self.vault.find_by_key(k).filter(|&r| self.vault.key_of(r) == *k)
    }

    fn named(&self, r: NRef) -> String {
        let tree = &self.vault.tree;
        quoted(&tree.node(tree.resolved_child(r)).title)
    }

    /// A node and its ancestors, top first, following block roots up to the
    /// embeds that stitch them in (so a block's path is its place in the
    /// tree, not its file).
    fn chain(&self, r: NRef) -> Vec<NRef> {
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
                Some(_) if c.0 != tree.root.0 => {
                    n.block.as_ref().and_then(|b| b.id.as_ref()).and_then(|id| tree.embed_of(id))
                }
                _ => None,
            };
        }
        out.reverse();
        out
    }

    /// A node's parent in the outline, `None` at the top.
    fn outline_parent(&self, r: NRef) -> Option<NRef> {
        let chain = self.chain(r);
        chain.len().checked_sub(2).map(|i| chain[i])
    }

    fn crumbs(&self, r: NRef) -> Vec<Crumb> {
        self.chain(r)
            .into_iter()
            .map(|c| Crumb { key: self.key(c), title: self.vault.tree.node(c).title.clone() })
            .collect()
    }

    // ------------------------------------------------------------ outline

    fn outline(&self, q: &OutlineQuery) -> Outline {
        let tree = &self.vault.tree;
        // a zoom whose node went away zooms out to its deepest surviving
        // ancestor (§10.1)
        let zoom = q
            .zoom
            .as_deref()
            .and_then(keys::decode)
            .and_then(|k| self.vault.find_by_key(&k))
            .filter(|&r| tree.node(r).kind != Kind::Root);
        let folded: Vec<NodeKey> = q.folded.iter().filter_map(|k| keys::decode(k)).collect();
        let unfolded: Vec<NodeKey> = q.unfolded.iter().filter_map(|k| keys::decode(k)).collect();
        let mut rows = Vec::new();
        let mut seen = Vec::new();
        let root = zoom.unwrap_or(tree.root);
        if let Some(z) = zoom {
            seen.push(z);
        }
        for c in tree.resolved_children(root) {
            self.flatten(c, 0, q.hide_done, &folded, &unfolded, &mut rows, &mut seen);
        }
        Outline {
            zoom: zoom.map(|z| self.header(z)),
            crumbs: zoom.map(|z| self.crumbs(z)).unwrap_or_default(),
            rows,
            conflicts: fold_core::merge::conflict_pairs(&self.vault).len() as u32,
            undo: self.undo.last().map(|e| e.description.clone()),
            redo: self.redo.last().map(|e| e.description.clone()),
            undo_depth: self.undo.len() as u32,
            copied: self.register.as_ref().map(|(_, name, _)| name.clone()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn flatten(
        &self,
        r: NRef,
        depth: u32,
        hide_done: bool,
        folded: &[NodeKey],
        unfolded: &[NodeKey],
        out: &mut Vec<Row>,
        seen: &mut Vec<NRef>,
    ) {
        let tree = &self.vault.tree;
        let n = tree.node(r);
        // an embed cycle or a second embed of a block shows it once (§6.2)
        if n.is_block() {
            if seen.contains(&r) {
                return;
            }
            seen.push(r);
        }
        if hide_done && n.task == Some(TaskState::Done) {
            return;
        }
        let key = self.vault.key_of(r);
        let kids = tree.resolved_children(r);
        let is_folded = match n.conflict() {
            Some(_) => !unfolded.contains(&key),
            None => folded.contains(&key),
        };
        let (open, total) = self.counts(r, !kids.is_empty());
        out.push(Row {
            key: keys::encode(&key),
            title: n.title.clone(),
            depth,
            spelling: spelling_of(n.kind),
            task: task_of(n.task),
            block: n.is_block(),
            conflict: n.conflict().map(copy_from),
            broken: n.is_embed(),
            has_children: !kids.is_empty(),
            folded: is_folded,
            open,
            total,
            due: n.block.as_ref().and_then(|b| b.prop("due")).map(str::to_string),
            preview: self.preview(r),
        });
        if is_folded {
            return;
        }
        for c in kids {
            self.flatten(c, depth + 1, hide_done, folded, unfolded, out, seen);
        }
    }

    /// The open/total count of the tasks below a node, itself not counted
    /// (§3.5); a leaf shows none.
    fn counts(&self, r: NRef, kids: bool) -> (u32, u32) {
        let n = self.vault.tree.node(r);
        let (open, total) = self.vault.tree.task_counts(r);
        let own = n.task.is_some() as usize;
        if !kids || total <= own {
            return (0, 0);
        }
        let own_open = (n.task == Some(TaskState::Open)) as usize;
        ((open.saturating_sub(own_open)) as u32, (total - own) as u32)
    }

    /// The first line of a node's own text, whitespace collapsed.
    fn preview(&self, r: NRef) -> String {
        let tree = &self.vault.tree;
        let t = tree.resolved_child(r);
        tree.node(t)
            .text_lines(tree.text_of(t))
            .into_iter()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("```"))
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    }

    fn header(&self, r: NRef) -> Header {
        let tree = &self.vault.tree;
        let n = tree.node(r);
        let (open, total) = self.counts(r, !tree.resolved_children(r).is_empty());
        // the body: the rendered lines after the title, up to its first child
        let body: Vec<String> = render_lines(tree, r, 1, true)
            .into_iter()
            .skip(1)
            .take_while(|l| l.node == r && l.kind != LineKind::Title)
            .map(|l| l.text)
            .collect();
        let mut body = body;
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
        }
        while body.first().is_some_and(|l| l.trim().is_empty()) {
            body.remove(0);
        }
        Header {
            key: self.key(r),
            title: n.title.clone(),
            spelling: spelling_of(n.kind),
            task: task_of(n.task),
            block: n.is_block(),
            conflict: n.conflict().map(copy_from),
            due: n.block.as_ref().and_then(|b| b.prop("due")).map(str::to_string),
            open,
            total,
            body,
            props: self.properties(r),
        }
    }

    fn node_info(&self, r: NRef) -> NodeInfo {
        let n = self.vault.tree.node(r);
        NodeInfo {
            key: self.key(r),
            title: n.title.clone(),
            spelling: spelling_of(n.kind),
            task: task_of(n.task),
            block: n.is_block(),
            conflict: n.conflict().map(copy_from),
            paired: ops::conflict_pair(&self.vault.tree, r).len() > 1 || n.conflict().is_some(),
            path: self.crumbs(r),
        }
    }

    fn properties(&self, r: NRef) -> Vec<Property> {
        let tree = &self.vault.tree;
        let r = tree.resolved_child(r);
        if !tree.node(r).is_block() {
            return Vec::new();
        }
        ops::frontmatter_lines(&self.vault, r.0)
            .into_iter()
            .filter(|(k, _, _)| k != "id")
            .map(|(key, value, editable)| Property { key, value, editable })
            .collect()
    }

    fn reading(&self, root: NRef) -> Vec<ReadLine> {
        let tree = &self.vault.tree;
        render_lines(tree, root, 1, true)
            .into_iter()
            .map(|l| {
                let node = (l.kind != LineKind::Blank).then(|| self.key(l.node));
                let mut line = ReadLine {
                    kind: match l.kind {
                        LineKind::Title => ReadKind::Title,
                        LineKind::Body => ReadKind::Body,
                        LineKind::Embed => ReadKind::Embed,
                        LineKind::Blank => ReadKind::Blank,
                    },
                    text: l.text.clone(),
                    node,
                    indent: 0,
                    heading: 0,
                    task: Task::None,
                    title: String::new(),
                    props: None,
                    conflict: None,
                };
                if l.kind == LineKind::Title {
                    let n = tree.node(l.node);
                    let trimmed = l.text.trim_start_matches(' ');
                    line.indent = (l.text.len() - trimmed.len()) as u32;
                    line.heading = match n.kind {
                        Kind::Section => trimmed.bytes().take_while(|&b| b == b'#').count() as u32,
                        _ => 0,
                    };
                    line.task = task_of(n.task);
                    line.title = n.title.clone();
                    line.conflict = n.conflict().map(copy_from);
                    let props: Vec<String> = self
                        .properties(l.node)
                        .into_iter()
                        .filter(|p| p.editable && p.key != "conflict")
                        .map(|p| format!("{} {}", p.key, p.value))
                        .collect();
                    line.props = (!props.is_empty()).then(|| props.join(" · "));
                }
                line
            })
            .collect()
    }

    fn hit(&self, r: NRef, excerpt: Option<String>) -> Hit {
        let tree = &self.vault.tree;
        let n = tree.node(r);
        let mut chain = self.chain(r);
        chain.pop();
        Hit {
            key: self.key(r),
            parent: chain.last().map(|&p| self.key(p)),
            title: n.title.clone(),
            path: chain.iter().map(|&c| tree.node(c).title.as_str()).collect::<Vec<_>>().join(" › "),
            task: task_of(n.task),
            conflict: n.conflict().is_some() || chain.iter().any(|&c| tree.node(c).conflict().is_some()),
            excerpt,
        }
    }

    fn search(&self, query: &str) -> Vec<Hit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let tree = &self.vault.tree;
        let mut scored: Vec<(u8, usize, NRef, Option<String>)> = Vec::new();
        let mut order = 0;
        tree.walk(tree.root, &mut |t, r| {
            let n = t.node(r);
            if n.kind == Kind::Root || n.is_embed() {
                return;
            }
            order += 1;
            let title = n.title.to_lowercase();
            let score = if title.starts_with(&q) {
                Some(0)
            } else if title.contains(&q) {
                Some(1)
            } else if fuzzy_match(&q, &title) {
                Some(2)
            } else {
                None
            };
            if let Some(s) = score {
                scored.push((s, order, r, None));
                return;
            }
            // full text over its own text children (§10.5)
            let line = n.text_lines(t.text_of(r)).into_iter().find(|l| l.to_lowercase().contains(&q));
            if let Some(l) = line {
                scored.push((3, order, r, Some(l.trim().to_string())));
            }
        });
        scored.sort_by_key(|&(s, o, _, _)| (s, o));
        scored.into_iter().take(200).map(|(_, _, r, ex)| self.hit(r, ex)).collect()
    }

    fn targets(&self, query: &str, moving: Option<&str>) -> Vec<Hit> {
        let q = query.trim().to_lowercase();
        let tree = &self.vault.tree;
        // a node cannot move into its own subtree, nor into the node its
        // conflict copy is of, which moves with it (§12.5)
        let moving = moving.and_then(|k| self.find(k));
        let moving = moving.map(|r| ops::conflict_pair(tree, r)).unwrap_or_default();
        let mut nodes: Vec<NRef> = Vec::new();
        tree.walk(tree.root, &mut |t, r| {
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
            if chain.iter().any(|&c| tree.node(c).conflict().is_some()) {
                continue;
            }
            let title = tree.node(r).title.to_lowercase();
            let path = chain.iter().map(|&c| tree.node(c).title.as_str()).collect::<Vec<_>>().join(" › ").to_lowercase();
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
        if q.is_empty() {
            // nothing typed: the outline's order
            scored.sort_by_key(|&(s, _, _)| s);
        } else {
            scored.sort_by_key(|&(s, len, _)| (s, len));
        }
        scored.into_iter().take(200).map(|(_, _, r)| self.hit(r, None)).collect()
    }

    /// The unresolved pairs, first in the outline first (§10.7).
    fn view_pairs(&self) -> Vec<(NRef, NRef)> {
        let mut pairs = fold_core::merge::conflict_pairs(&self.vault);
        if pairs.len() > 1 {
            let tree = &self.vault.tree;
            let mut order = Vec::new();
            tree.walk(tree.root, &mut |_, r| order.push(r));
            pairs.sort_by_key(|&(ours, _)| order.iter().position(|&r| r == ours));
        }
        pairs
    }

    fn conflicts(&self) -> Vec<ConflictPair> {
        let tree = &self.vault.tree;
        self.view_pairs()
            .into_iter()
            .map(|(ours, theirs)| ConflictPair {
                ours: self.key(ours),
                theirs: self.key(theirs),
                title: tree.node(ours).title.clone(),
                from: tree.node(theirs).conflict().map(copy_from).unwrap_or_default(),
                ours_text: render(tree, ours, 1, true),
                theirs_text: render(tree, theirs, 1, true),
            })
            .collect()
    }

    // ------------------------------------------------------------ verbs

    /// Run a verb from the app (`fold-tui`'s `editor_before_write` and
    /// `editor_after_write`, §10.6): an open editor saves first, and only
    /// then does the verb find its nodes, as the save re-parses what it
    /// writes; after it, the editor is re-rendered over what the verb wrote,
    /// and the result says so, so the text field takes the new text.
    fn with_editor_saved(&mut self, f: impl FnOnce(&mut State) -> OpResult) -> OpResult {
        let open = self.editor.is_some();
        let edit = self.editor_before_write();
        let generation = self.edit_generation;
        let mut r = f(self);
        self.editor_after_write(edit);
        if open {
            match &self.editor {
                None => r.editor_closed = true,
                Some(ed) if self.edit_generation != generation => r.editor_text = Some(ed.text()),
                Some(_) => {}
            }
        }
        r.editor_generation = self.edit_generation;
        r
    }

    /// Run a verb as one op-log entry (§10.10): the entry holds exactly the
    /// files the verb changed, written before a failure too.
    fn verb(&mut self, desc: &str, f: Verb<'_>) -> OpResult {
        let snap = ops::Snapshot::take(&self.vault, desc);
        let res = f(self);
        self.record_undo(snap);
        match res {
            Ok((message, node)) => OpResult::new(true, message, node),
            Err(message) => OpResult::new(false, message, None),
        }
    }

    fn refused(message: impl Into<String>) -> OpResult {
        OpResult::new(false, message, None)
    }

    fn gone() -> OpResult {
        Self::refused("that node is gone")
    }

    fn record_undo(&mut self, snap: ops::Snapshot) {
        if let Some(inv) = ops::Inverse::since(snap, &self.vault) {
            self.undo.push(inv);
            self.redo.clear();
        }
    }

    fn toggle_task(&mut self, key: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let name = self.named(r);
        let tree = &self.vault.tree;
        let (words, step) = match tree.node(tree.resolved_child(r)).task {
            Some(TaskState::Open) => (format!("done: {}", name), format!("mark {} done", name)),
            Some(TaskState::Done) => (format!("reopened: {}", name), format!("reopen {}", name)),
            None => return Self::refused(format!("{} isn't a task · Task on makes it one", name)),
        };
        let k = self.vault.key_of(r);
        self.verb(
            &step,
            Box::new(move |s| {
                ops::toggle_task(&mut s.vault, r).map_err(|e| format!("error: {}", e))?;
                Ok((words, s.vault.find_by_key(&k).map(|r| s.key(r))))
            }),
        )
    }

    fn toggle_taskness(&mut self, key: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let name = self.named(r);
        let tree = &self.vault.tree;
        let (words, step) = match tree.node(tree.resolved_child(r)).task {
            Some(_) => (format!("removed the checkbox from {}", name), format!("remove the checkbox from {}", name)),
            None => (format!("made {} a task", name), format!("make {} a task", name)),
        };
        let k = self.vault.key_of(r);
        self.verb(
            &step,
            Box::new(move |s| {
                ops::toggle_taskness(&mut s.vault, r).map_err(|e| format!("error: {}", e))?;
                Ok((words, s.vault.find_by_key(&k).map(|r| s.key(r))))
            }),
        )
    }

    fn add_node(&mut self, key: Option<&str>, title: &str, task: bool, child: bool) -> OpResult {
        let title = title.trim();
        if title.is_empty() {
            return Self::refused("a node needs a title");
        }
        if title.contains('\n') {
            return Self::refused("a title is one line");
        }
        let at = match key {
            Some(k) => match self.find(k) {
                Some(r) => r,
                None => return Self::gone(),
            },
            None => self.vault.tree.root,
        };
        let root = self.vault.tree.node(at).kind == Kind::Root;
        let child = child || root;
        let shown = if task { format!("[ ] {}", title) } else { title.to_string() };
        let desc = if root {
            "add a node".to_string()
        } else {
            format!("add a node {} {}", if child { "under" } else { "after" }, self.named(at))
        };
        let words = format!("added {}", quoted(title));
        let title = title.to_string();
        self.verb(
            &desc,
            Box::new(move |s| {
                if child {
                    let r = ops::append_child_public(&mut s.vault, at, &shown).map_err(|e| format!("error: {}", e))?;
                    return Ok((words, Some(s.key(r))));
                }
                // a sibling, spelled like the node it follows (§10.3 `n`)
                let line = match s.vault.tree.node(at).kind {
                    Kind::Section => format!("# {}\n", shown),
                    _ => format!("- {}\n", shown),
                };
                let k = s.vault.key_of(at);
                let moved = ops::paste(&mut s.vault, at, &line, true).map_err(|e| format!("error: {}", e))?;
                // the new sibling: the node after the one it follows among
                // its siblings, past any conflict copy of it (§12.5)
                let at = s.vault.find_by_key(&k).unwrap_or(at);
                let parent = s.outline_parent(at).unwrap_or(s.vault.tree.root);
                let tree = &s.vault.tree;
                let kids = tree.resolved_children(parent);
                let new = kids
                    .iter()
                    .skip_while(|&&c| c != at)
                    .skip(1)
                    .find(|&&c| tree.node(c).conflict().is_none() && tree.node(c).title == title)
                    .copied()
                    .or_else(|| kids.iter().rev().find(|&&c| tree.node(c).title == title).copied());
                let kind = tree.node(at).kind;
                Ok((with_rule_note(&words, moved, kind), new.map(|r| s.key(r))))
            }),
        )
    }

    fn delete(&mut self, key: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        self.put_register(r);
        let name = self.named(r);
        // a node's conflict copies go with it (§12.5)
        let what = match ops::conflict_copies(&self.vault.tree, r).len() {
            0 => name.clone(),
            1 => format!("{} and its conflict copy", name),
            n => format!("{} and its {} conflict copies", name, n),
        };
        self.verb(
            &format!("delete {}", name),
            Box::new(move |s| match ops::delete_subtree(&mut s.vault, r) {
                Ok(1) => Ok((format!("deleted {}", what), None)),
                Ok(n) => Ok((format!("deleted {} ({} nodes)", what, n), None)),
                Err(e) => Err(format!("error: {}", e)),
            }),
        )
    }

    /// Put `r`'s subtree in the register (§10.3 `y`, `d`).
    fn put_register(&mut self, r: NRef) {
        let tree = &self.vault.tree;
        let kind = tree.node(tree.resolved_child(r)).kind;
        self.register = Some((ops::yank(&self.vault, r), self.named(r), kind));
    }

    fn copy(&mut self, key: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        self.put_register(r);
        OpResult::new(true, format!("copied {}", self.named(r)), Some(key.to_string()))
    }

    fn paste(&mut self, key: &str, after: bool) -> OpResult {
        let Some((text, name, kind)) = self.register.clone() else {
            return Self::refused("nothing copied yet · Copy a node first");
        };
        let Some(r) = self.find(key) else { return Self::gone() };
        self.verb(
            &format!("paste {}", name),
            Box::new(move |s| {
                let moved = ops::paste(&mut s.vault, r, &text, after).map_err(|e| format!("error: {}", e))?;
                Ok((with_rule_note(&format!("pasted {}", name), moved, kind), None))
            }),
        )
    }

    fn move_sibling(&mut self, key: &str, down: bool) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let name = self.named(r);
        let k = self.vault.key_of(r);
        let dir = if down { "down" } else { "up" };
        self.verb(
            &format!("move {} {}", name, dir),
            Box::new(move |s| {
                ops::move_sibling(&mut s.vault, r, down).map_err(|e| format!("can't move {} {}: {}", name, dir, e))?;
                Ok((format!("moved {} {}", name, dir), s.vault.find_by_key(&k).map(|r| s.key(r))))
            }),
        )
    }

    fn indent(&mut self, key: &str) -> OpResult {
        let Some(sub) = self.find(key) else { return Self::gone() };
        let tree = &self.vault.tree;
        let r = tree.resolved_child(sub);
        let (k, kind) = (self.vault.key_of(r), tree.node(r).kind);
        // it goes under the node before it (§10.3), with its conflict
        // pair, which moves as one (§12.5)
        let sibs = tree.resolved_children(self.outline_parent(r).unwrap_or(tree.root));
        let first = ops::conflict_pair(tree, r)[0];
        let prev = sibs.iter().position(|&c| c == first).and_then(|i| i.checked_sub(1)).map(|i| sibs[i]);
        let Some(prev) = prev else {
            return Self::refused(format!("can't indent {}: nothing above it", self.named(r)));
        };
        if tree.node(prev).conflict().is_some() {
            return Self::refused(format!("can't indent {} into a conflict copy", self.named(r)));
        }
        let prev = self.vault.key_of(prev);
        let name = self.named(r);
        self.verb(
            &format!("indent {}", name),
            Box::new(move |s| {
                let moved = ops::demote(&mut s.vault, sub).map_err(|e| format!("can't indent {}: {}", name, e))?;
                let node = s.moved_node(&k, kind, &prev, None).map(|r| s.key(r));
                Ok((with_rule_note(&format!("indented {}", name), moved, kind), node))
            }),
        )
    }

    fn outdent(&mut self, key: &str) -> OpResult {
        let Some(sub) = self.find(key) else { return Self::gone() };
        let tree = &self.vault.tree;
        let r = tree.resolved_child(sub);
        let (k, kind) = (self.vault.key_of(r), tree.node(r).kind);
        // it goes beside its parent, right after it (§10.3)
        let parent = self.outline_parent(r);
        let grand = parent.map(|p| self.outline_parent(p).unwrap_or(tree.root));
        let rank = parent.zip(grand).and_then(|(p, g)| {
            let kids = tree.resolved_children(g);
            let at = kids.iter().position(|&c| c == p)?;
            Some(self.namesakes_before(r, g, self.past_copies(&kids, at + 1, r)))
        });
        let Some(grand) = grand.map(|g| self.vault.key_of(g)) else {
            return Self::refused(format!("can't outdent {}: it's at the top level", self.named(r)));
        };
        let name = self.named(r);
        self.verb(
            &format!("outdent {}", name),
            Box::new(move |s| {
                let moved = ops::promote(&mut s.vault, sub).map_err(|e| format!("can't outdent {}: {}", name, e))?;
                let node = s.moved_node(&k, kind, &grand, rank).map(|r| s.key(r));
                Ok((with_rule_note(&format!("outdented {}", name), moved, kind), node))
            }),
        )
    }

    fn move_to(&mut self, key: &str, dest: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let Some(dest) = self.find(dest) else { return Self::refused("that destination is gone") };
        let tree = &self.vault.tree;
        let (rr, dest_key) = (tree.resolved_child(r), self.vault.key_of(tree.resolved_child(dest)));
        let (k, kind) = (self.vault.key_of(rr), tree.node(rr).kind);
        let (name, to) = (self.named(rr), self.named(dest));
        self.verb(
            &format!("move {} to {}", name, to),
            Box::new(move |s| {
                let moved = ops::refile(&mut s.vault, r, dest).map_err(|e| format!("can't move {}: {}", name, e))?;
                let node = s.moved_node(&k, kind, &dest_key, None).map(|r| s.key(r));
                Ok((with_rule_note(&format!("moved {} to {}", name, to), moved, kind), node))
            }),
        )
    }

    /// Where a node a verb moved under `dest` landed (`fold-tui`'s
    /// `moved_node`): a block by its id; else, among `dest`'s children with
    /// its title and kind, but no conflict copy, the one the ordering rule
    /// put it at.
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

    fn past_copies(&self, kids: &[NRef], at: usize, r: NRef) -> usize {
        let mut at = at;
        if at.checked_sub(1).map(|i| kids[i]) != Some(r) {
            while kids.get(at).is_some_and(|&c| self.vault.tree.node(c).conflict().is_some()) {
                at += 1;
            }
        }
        at
    }

    fn archive(&mut self, key: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let tree = &self.vault.tree;
        let (name, kind) = (self.named(r), tree.node(tree.resolved_child(r)).kind);
        self.verb(
            &format!("archive {}", name),
            Box::new(move |s| {
                let moved = ops::archive(&mut s.vault, r).map_err(|e| format!("can't archive {}: {}", name, e))?;
                Ok((with_rule_note(&format!("archived {}", name), moved, kind), None))
            }),
        )
    }

    fn toggle_spelling(&mut self, key: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let tree = &self.vault.tree;
        let (spelling, kind) = match tree.node(tree.resolved_child(r)).kind {
            Kind::Section => ("a bullet", Kind::Item),
            _ => ("a heading", Kind::Section),
        };
        let name = self.named(r);
        // a node in a conflict pair keeps its spelling (§12.5)
        if ops::conflict_pair(tree, r).len() > 1 {
            return Self::refused(format!("can't respell {}: {}", name, ops::PAIR_SPELLING));
        }
        let k = self.vault.key_of(r);
        self.verb(
            &format!("make {} {}", name, spelling),
            Box::new(move |s| {
                let moved = ops::toggle_spelling(&mut s.vault, r).map_err(|e| format!("error: {}", e))?;
                let node = s.vault.find_by_key(&k).map(|r| s.key(r));
                Ok((with_rule_note(&format!("made {} {}", name, spelling), moved, kind), node))
            }),
        )
    }

    fn make_block(&mut self, key: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let tree = &self.vault.tree;
        if tree.node(tree.resolved_child(r)).is_block() {
            return Self::refused(format!("{} has its own file already", self.named(r)));
        }
        let name = self.named(r);
        self.verb(
            &format!("give {} its own file", name),
            Box::new(move |s| {
                let id = ops::make_block(&mut s.vault, r).map_err(|e| format!("error: {}", e))?;
                // no id, no file name (§1 principle 3b)
                let node = s.vault.tree.block_by_id(&id).map(|r| s.key(r));
                Ok((format!("gave {} its own file", name), node))
            }),
        )
    }

    fn set_property(&mut self, key: &str, name: &str, value: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let (name, value) = (name.trim().to_string(), value.trim().to_string());
        if !fold_core::parse::is_valid_key(&name) || name == "id" {
            return Self::refused(format!("“{}” can't be a property name: letters, digits, _ and -", name));
        }
        // dates are checked (§8.4)
        if (name == "due" || name == "done") && !fold_core::check::is_iso_date(&value) {
            return Self::refused(format!("{}: must be YYYY-MM-DD", name));
        }
        if value.contains('\n') {
            return Self::refused("a property is one line");
        }
        let node = self.named(r);
        self.verb(
            &format!("set {} of {}", name, node),
            Box::new(move |s| {
                let block = ops::set_property(&mut s.vault, r, &name, &value).map_err(|e| format!("error: {}", e))?;
                Ok((format!("{} set", name), Some(s.key(block))))
            }),
        )
    }

    fn remove_property(&mut self, key: &str, name: &str) -> OpResult {
        let Some(r) = self.find(key) else { return Self::gone() };
        let r = self.vault.tree.resolved_child(r);
        if !self.vault.tree.node(r).is_block() {
            return Self::refused("no properties to remove");
        }
        match self.properties(r).iter().find(|p| p.key == name) {
            None => return Self::refused(format!("no property {}", name)),
            Some(p) if !p.editable => return Self::refused("read-only line (preserved verbatim)"),
            Some(_) => {}
        }
        let (name, node) = (name.to_string(), self.named(r));
        self.verb(
            &format!("remove {} from {}", name, node),
            Box::new(move |s| {
                ops::set_frontmatter_key(&mut s.vault, r.0, &name, None).map_err(|e| format!("error: {}", e))?;
                Ok((format!("{} removed", name), None))
            }),
        )
    }

    fn capture(&mut self, text: &str, task: bool) -> OpResult {
        let mut text = text.trim().to_string();
        let mut task = task;
        if let Some(rest) = text.strip_prefix("[ ]") {
            text = rest.trim().to_string();
            task = true;
        }
        if text.is_empty() {
            return Self::refused("nothing to capture");
        }
        let first = text.lines().next().unwrap_or("").to_string();
        self.verb(
            &format!("capture {}", quoted(&first)),
            Box::new(move |s| {
                let r = ops::capture(&mut s.vault, &text, task).map_err(|e| format!("error: {}", e))?;
                Ok(("captured".into(), Some(s.key(r))))
            }),
        )
    }

    fn clear_done(&mut self, zoom: Option<&str>) -> OpResult {
        let zoom = zoom.and_then(|k| self.find(k));
        let under = zoom.map(|z| format!(" under {}", self.named(z))).unwrap_or_default();
        let target = zoom.unwrap_or(self.vault.tree.root);
        self.verb(
            &format!("clear done tasks{}", under),
            Box::new(move |s| match ops::clear_done(&mut s.vault, target) {
                Ok(0) => Ok((format!("no done tasks to clear{}", under), None)),
                Ok(1) => Ok((format!("cleared 1 done task{}", under), None)),
                Ok(n) => Ok((format!("cleared {} done tasks{}", n, under), None)),
                Err(e) => Err(format!("error: {}", e)),
            }),
        )
    }

    fn resolve(&mut self, theirs: &str, keep: Keep) -> OpResult {
        let Some(t) = self.find(theirs) else { return Self::gone() };
        let Some((ours, theirs)) = self.view_pairs().into_iter().find(|&(_, x)| x == t) else {
            return Self::refused("that conflict is resolved already");
        };
        let side = match keep {
            Keep::Ours => "ours",
            Keep::Theirs => "theirs",
            Keep::Both => "both",
        };
        let name = self.named(ours);
        let k = self.vault.key_of(ours);
        self.verb(
            &format!("keep {} for {}", side, name),
            Box::new(move |s| {
                match keep {
                    Keep::Ours => fold_core::merge::resolve_keep_ours(&mut s.vault, theirs),
                    Keep::Theirs => fold_core::merge::resolve_keep_theirs(&mut s.vault, ours, theirs),
                    Keep::Both => fold_core::merge::resolve_keep_both(&mut s.vault, theirs),
                }
                .map_err(|e| format!("error: {}", e))?;
                Ok((format!("kept {} for {}", side, name), s.vault.find_by_key(&k).map(|r| s.key(r))))
            }),
        )
    }

    fn canonicalize(&mut self) -> OpResult {
        self.verb(
            "canonicalize",
            Box::new(|s| {
                // fix keeps the index as it renames: nothing else is read
                // again, so the entry holds what it wrote and no more
                let n = fold_core::check::fix(&mut s.vault).map_err(|e| format!("error: {}", e))?;
                Ok((format!("{} file{} rewritten or renamed", n, if n == 1 { "" } else { "s" }), None))
            }),
        )
    }

    fn undo_redo(&mut self, undo: bool) -> OpResult {
        if self.editor.is_some() {
            return Self::refused("finish editing first");
        }
        // the entries a block in transit was remembered with may go
        self.edit_transit.clear();
        let entry = if undo { self.undo.pop() } else { self.redo.pop() };
        let Some(inv) = entry else {
            return Self::refused(if undo { "nothing to undo" } else { "nothing to redo" });
        };
        let res = if undo { inv.undo(&mut self.vault) } else { inv.redo(&mut self.vault) };
        let (done, verb) = if undo { ("undone", "undo") } else { ("redone", "redo") };
        match res {
            Ok(()) => {
                let message = format!("{}: {}", done, inv.description);
                if undo { self.redo.push(inv) } else { self.undo.push(inv) }
                OpResult::new(true, message, None)
            }
            Err(e) => {
                // refused: an external change since; the entry stays
                if undo { self.undo.push(inv) } else { self.redo.push(inv) }
                Self::refused(format!("{} refused: {}", verb, e))
            }
        }
    }

    fn restore(&mut self, name: &str) -> OpResult {
        let Some(path) = trash_entry(name) else { return Self::refused("no such trash entry") };
        // strip the `<yyyymmdd-hhmmss>-` stamp
        let restored = if name.len() > 16 && name.as_bytes()[8] == b'-' && name.as_bytes()[15] == b'-' {
            name[16..].to_string()
        } else {
            name.to_string()
        };
        // text the editor could not save has no id: moved in, it would be a
        // file fold ignores, so it stays in the trash (§11.5)
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let has_id = fold_core::parse::parse_frontmatter(&text)
            .and_then(|fm| fm.props.get("id").and_then(|v| Id::parse(v)))
            .is_some();
        if !has_id {
            return Self::refused("this is text, not a file fold reads: copy what you need from it");
        }
        let dir = self.vault.dir.clone();
        // never restore over an existing file, root.md least of all
        let target = if restored != "root.md" && !dir.join(&restored).exists() {
            restored.clone()
        } else {
            let stem = restored.strip_suffix(".md").unwrap_or(&restored);
            (2..).map(|i| format!("{}-restored-{}.md", stem, i)).find(|n| !dir.join(n).exists()).unwrap()
        };
        if let Err(e) = fold_core::vault::move_file(&path, &dir.join(&target)) {
            return Self::refused(format!("error: {}", e));
        }
        // the file came in: read the vault again
        if let Err(e) = self.vault.reload() {
            return Self::refused(format!("error: {}", e));
        }
        OpResult::new(true, format!("restored {} — check lists it until it is embedded somewhere", target), None)
    }

    // ------------------------------------------------------------ files

    fn refresh(&mut self, force: bool) -> Refresh {
        let copies = self.vault.conflict_files().unwrap_or_default();
        let new_copy = copies.iter().any(|c| !self.conflict_copies.contains(c));
        let changed = self.vault.changed_on_disk().unwrap_or(true);
        let mut out = Refresh {
            changed: false,
            message: None,
            raised: 0,
            editor_text: None,
            editor_generation: self.edit_generation,
            editor_closed: false,
        };
        if !force && !new_copy && !changed {
            return out;
        }
        out.changed = true;
        // what came in: the files as they are against what was read, taken
        // before the editor's save writes what was typed
        let before = news::tops(&self.vault);
        let after = self.vault.on_disk().map(|v| news::tops(&v));
        let dirty = self.editor_dirty();
        let refused = self.write_editor(false).err();
        let edit = self.editor_key().map(|k| (k, self.editor_files()));
        let typed = dirty && !self.editor_dirty();
        let mut message = None;
        if !copies.is_empty() {
            let read = after.as_ref().ok().filter(|_| !dirty).cloned();
            let was = read.unwrap_or_else(|| self.vault.on_disk().map(|v| news::tops(&v)).unwrap_or_else(|_| news::tops(&self.vault)));
            match self.merge_conflict_files() {
                Ok((0, _)) => {
                    let now = news::merged_over(after.unwrap_or_else(|_| was.clone()), &was, news::tops(&self.vault));
                    if !news::same_tops(&before, &now) {
                        message = Some(news::changed_outside(&before, &now, typed));
                    }
                }
                Ok((n, said)) => {
                    out.raised = n;
                    message = Some(said);
                }
                Err(e) => message = Some(format!("merge error: {}", e)),
            }
        } else {
            match self.vault.reload() {
                Ok(()) => {
                    let after = after.unwrap_or_else(|_| news::tops(&self.vault));
                    if !news::same_tops(&before, &after) || typed {
                        message = Some(news::changed_outside(&before, &after, typed));
                    }
                }
                Err(e) => message = Some(format!("reload error: {}", e)),
            }
        }
        // a change to what the editor shows re-renders it; one elsewhere in
        // a file it holds is taken in where it is (§11.2)
        if let Some((key, files)) = edit {
            if files != self.editor_files() && !self.editor_takes_in(&key) && self.drop_cut_blocks() {
                match self.find_exact(&key) {
                    Some(r) => {
                        self.editor = Some(TextEditor::new(open_editor(&self.vault, r)));
                        self.edit_generation += 1;
                        out.editor_text = self.editor.as_ref().map(|e| e.text());
                        out.editor_generation = self.edit_generation;
                    }
                    None => {
                        self.editor = None;
                        out.editor_closed = true;
                    }
                }
            }
        }
        if let Some(e) = refused {
            message = Some(match message {
                Some(m) => format!("{} · {}", m, e),
                None => e,
            });
        }
        self.conflict_copies = self.vault.conflict_files().unwrap_or_default();
        self.conflict_copies.retain(|c| copies.contains(c));
        out.message = message;
        out
    }

    /// The merge flow (§12.2): merge the sync-conflict copies. The pairs it
    /// raised and what to say of them; a block cut in the editor and not
    /// pasted back is moved, and not put back where it was (§5.2).
    fn merge_conflict_files(&mut self) -> std::io::Result<(u32, String)> {
        let before: Vec<NodeKey> = self.view_pairs().into_iter().map(|(_, t)| self.vault.key_of(t)).collect();
        let moving: Vec<Id> = self.transit().into_iter().map(|(id, _)| id).collect();
        fold_core::merge::merge_sync_conflicts_moving(&mut self.vault, false, &moving)?;
        let raised: Vec<NRef> = self
            .view_pairs()
            .into_iter()
            .filter(|&(_, t)| !before.contains(&self.vault.key_of(t)))
            .map(|(o, _)| o)
            .collect();
        let said = match raised.as_slice() {
            [] => String::new(),
            [ours] => format!("sync conflict in {}", self.named(*ours)),
            [ours, rest @ ..] => format!("sync conflicts in {} and {} more", self.named(*ours), rest.len()),
        };
        Ok((raised.len() as u32, said))
    }

    // ------------------------------------------------------------ editor

    fn edit_history(&mut self, undo: bool) -> Option<EditorText> {
        let ed = self.editor.as_mut()?;
        let done = if undo { ed.undo() } else { ed.redo() };
        if !done {
            return None;
        }
        let text = ed.text();
        self.edit_generation += 1;
        Some(EditorText { text, generation: self.edit_generation })
    }

    fn editor_dirty(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.dirty())
    }

    fn editor_state(&self) -> EditorState {
        let ed = self.editor.as_ref();
        EditorState {
            dirty: ed.is_some_and(|e| e.dirty()),
            can_undo: ed.is_some_and(|e| e.can_undo()),
            can_redo: ed.is_some_and(|e| e.can_redo()),
        }
    }

    fn edit_open(&mut self, key: &str) -> Result<EditorView, String> {
        // an editor already open saves first, as on leaving it
        self.write_editor(true)?;
        let Some(r) = self.find(key) else { return Err("that node is gone".into()) };
        let r = self.vault.tree.resolved_child(r);
        let ed = TextEditor::new(open_editor(&self.vault, r));
        self.edit_generation += 1;
        let view = EditorView {
            text: ed.text(),
            title: self.vault.tree.node(r).title.clone(),
            node: self.key(r),
            generation: self.edit_generation,
        };
        self.editor = Some(ed);
        self.edit_refused = false;
        self.edit_uncopied = None;
        Ok(view)
    }

    fn saved(&self, res: Result<(), String>) -> Saved {
        let node = self.editor_node().map(|r| self.key(r));
        match res {
            Ok(()) => Saved { ok: true, message: "saved".into(), dirty: self.editor_dirty(), node },
            Err(e) => Saved { ok: false, message: e, dirty: self.editor_dirty(), node },
        }
    }

    /// Write the editor's dirty blocks (`fold-tui`'s `write_editor`); with
    /// `release`, to leave it or before it is re-rendered: a block cut and
    /// not pasted back is let go, and deleted (§5.2). A refused save keeps
    /// the text, and says why.
    fn write_editor(&mut self, release: bool) -> Result<(), String> {
        if release && !self.editor_dirty() {
            if let Some(ed) = self.editor.as_mut() {
                ed.release_clip();
            }
        }
        if !self.editor_dirty() {
            return Ok(());
        }
        let Some(ed) = self.editor.as_mut() else { return Ok(()) };
        // what another program changed beside the edited blocks is taken in
        // before the snapshot, so undoing this save leaves it be
        ed.buf.rebase_dirty(&mut self.vault);
        let snap = ops::Snapshot::take(&self.vault, &format!("edit {}", edited(ed)));
        let mut res = ed.buf.save_all(&mut self.vault);
        if let Some(n) = res.as_ref().ok().copied().filter(|_| release) {
            ed.release_clip();
            res = ed.buf.save_all(&mut self.vault).map(|m| m.max(n));
        }
        self.record_edit(snap);
        self.edit_refused = res.is_err();
        res.map(|_| ()).map_err(|e| format!("error: {}", e))
    }

    /// `record_undo` for a save of the editor (`fold-tui`'s `record_edit`,
    /// §10.10): a block an earlier save left in transit and this one
    /// deletes is deleted in the entry of that save, which wrote its embed
    /// out; one this save pastes back makes that save, this one and those
    /// between one entry.
    fn record_edit(&mut self, snap: ops::Snapshot) {
        let mut at = None;
        if let Some(mut inv) = ops::Inverse::since(snap, &self.vault) {
            let (transit, undo) = (&self.edit_transit, &mut self.undo);
            inv.changes.retain(|c| {
                let entry = transit.iter().find(|(p, _)| *p == c.path && c.after.is_none());
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
        }
        let mut back: Vec<usize> =
            self.edit_transit.iter().filter(|(p, _)| self.embedded(p) == Some(true)).map(|&(_, n)| n).collect();
        back.sort_unstable();
        if let Some(n) = back.into_iter().find(|&n| n > 0 && self.join_undo(n - 1)) {
            let joined = self.undo.len() == n;
            at = at.and(joined.then_some(n));
            self.edit_transit.retain_mut(|(_, m)| {
                *m = (*m).min(n);
                *m < n || joined
            });
        }
        let open = self.editor.is_some();
        let transit = std::mem::take(&mut self.edit_transit);
        self.edit_transit = transit.into_iter().filter(|(p, _)| open && self.embedded(p) == Some(false)).collect();
        if let Some(n) = at {
            for (_, p) in self.transit() {
                if !self.edit_transit.iter().any(|(q, _)| *q == p) {
                    self.edit_transit.push((p, n));
                }
            }
        }
    }

    /// Whether the block whose file is `path` is embedded now; `None` where
    /// the file is gone.
    fn embedded(&self, path: &str) -> Option<bool> {
        let tree = &self.vault.tree;
        let f = self.vault.file_index(path)?;
        let (_, id) = tree.blocks.iter().find(|(r, _)| r.0 == f)?;
        Some(tree.embed_of(id).is_some())
    }

    /// Make the op-log entries from `from` on one (§10.10); false where a
    /// file changed from outside between two of them.
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

    /// The blocks the open editor holds in transit (§5.2), with their files:
    /// nested there, with no line left in it, embedded nowhere, and still in
    /// the vault.
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

    /// The editor's render root in the tree as the editor last read or
    /// wrote it.
    fn editor_node(&self) -> Option<NRef> {
        let ed = self.editor.as_ref()?;
        let info = ed.buf.owners.values().find(|i| i.parent.is_none())?;
        match &info.id {
            Some(id) => self.vault.tree.block_by_id(id),
            None => {
                let file = self.vault.file_index(&info.path)?;
                let f = &self.vault.tree.files[file];
                if info.is_root {
                    Some((file, f.root_node))
                } else {
                    let i = f.nodes.iter().position(|n| n.kind != Kind::Root && n.span.start == info.start)?;
                    Some((file, i))
                }
            }
        }
    }

    fn editor_key(&self) -> Option<NodeKey> {
        self.editor_node().map(|r| self.vault.key_of(r))
    }

    /// The text of each file the open editor holds; None for one gone.
    fn editor_files(&self) -> Vec<Option<String>> {
        let Some(ed) = self.editor.as_ref() else { return Vec::new() };
        let tree = &self.vault.tree;
        let file = |i: &OwnerInfo| match &i.id {
            Some(id) => tree.block_by_id(id).map(|r| r.0),
            None => self.vault.file_index(&i.path),
        };
        ed.buf.owners.values().map(|i| file(i).map(|f| tree.files[f].text.clone())).collect()
    }

    /// After a reload changed a file the editor holds (§11.2): where its
    /// node renders as the buffer shows it, the buffer takes in the files as
    /// they are and stays.
    fn editor_takes_in(&mut self, key: &NodeKey) -> bool {
        let Some(r) = self.find_exact(key) else { return false };
        let now = open_editor(&self.vault, r);
        self.editor.as_mut().is_some_and(|ed| ed.buf.take_in(now))
    }

    /// Before the editor is re-rendered over text a reload changed: a block
    /// cut there and not pasted back is deleted, as on a Revert (§5.2).
    /// True when the editor can be re-rendered.
    fn drop_cut_blocks(&mut self) -> bool {
        if self.editor_dirty() {
            return false;
        }
        let Some(ed) = self.editor.as_mut() else { return false };
        let snap = ops::Snapshot::take(&self.vault, &format!("edit {}", edited(ed)));
        let res = ed.buf.discard(&mut self.vault);
        if res.is_ok() {
            ed.release_clip();
            ed.buf.dirty.clear();
        }
        self.record_edit(snap);
        res.is_ok()
    }

    /// Before a verb writes files under an open editor (§10.6): the editor
    /// saves first. Returns what it is open on and the files as they are.
    fn editor_before_write(&mut self) -> Option<(NodeKey, ops::Snapshot)> {
        self.editor.as_ref()?;
        let _ = self.write_editor(true);
        let key = self.editor_key()?;
        Some((key, ops::Snapshot::take(&self.vault, "")))
    }

    /// After it: if any file changed, the editor is re-rendered over the
    /// files as they now are, so its next save is not refused.
    fn editor_after_write(&mut self, before: Option<(NodeKey, ops::Snapshot)>) {
        let Some((key, snap)) = before else { return };
        if ops::Inverse::since(snap, &self.vault).is_none() || self.editor_dirty() {
            return;
        }
        self.editor = self.find_exact(&key).map(|r| TextEditor::new(open_editor(&self.vault, r)));
        self.edit_generation += 1;
    }

    fn edit_revert(&mut self) -> OpResult {
        let mut said = String::from("changes discarded");
        if let Some((name, text)) = self.editor_text().filter(|_| self.edit_refused && self.editor_dirty()) {
            match self.vault.trash_text(&name, &text) {
                Ok(p) => {
                    let entry = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    said = format!("{}; a copy is in the trash: {}", said, entry);
                }
                Err(_) if self.edit_uncopied.as_ref() == Some(&text) => said = format!("{}; no copy kept", said),
                Err(e) => {
                    self.edit_uncopied = Some(text);
                    return Self::refused(format!("can't copy to the trash; Revert again drops the changes — {}", e));
                }
            }
        }
        self.edit_refused = false;
        self.edit_uncopied = None;
        let ed = self.editor.take();
        let _ = self.vault.reload();
        if let Some(ed) = ed {
            // a block a save took out of its parent cannot be pasted back
            // as itself any more: it is deleted now (§5.2)
            let snap = ops::Snapshot::take(&self.vault, &format!("revert {}", edited(&ed)));
            if let Err(e) = ed.buf.discard(&mut self.vault) {
                said = format!("{}; error: {}", said, e);
            }
            self.record_edit(snap);
        }
        OpResult::new(true, said, None)
    }

    /// The editor's whole text, and the name the trash keeps it under.
    fn editor_text(&self) -> Option<(String, String)> {
        let ed = self.editor.as_ref()?;
        let title = ed.buf.owners.values().find(|i| i.parent.is_none()).map(|i| i.title.as_str()).unwrap_or("");
        let text = ed.buf.lines.iter().map(|l| format!("{}\n", l.text)).collect();
        Some((format!("unsaved-{}.md", fold_core::slug(title)), text))
    }

    fn edit_keep(&mut self, release: bool) -> Option<String> {
        let (name, text) = self.editor_text()?;
        if self.write_editor(release).is_ok() || !self.editor_dirty() {
            return None;
        }
        Some(match self.vault.trash_text(&name, &text) {
            Ok(p) => format!(
                "unsaved text kept in the trash: {}",
                p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            ),
            Err(e) => format!("unsaved text could not go to the trash: {}", e),
        })
    }
}

/// The block a save is named by: the one dirty block, else the edited node.
fn edited(ed: &TextEditor) -> String {
    let one = match ed.buf.dirty.as_slice() {
        [o] => Some(o),
        _ => None,
    };
    let info = one.and_then(|o| ed.buf.owners.get(o)).or_else(|| ed.buf.owners.values().find(|i| i.parent.is_none()));
    quoted(info.map_or("", |i| i.title.as_str()))
}

#[cfg(test)]
mod tests;
