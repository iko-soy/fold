//! Every action the TUI has, by name (§10.8): what buttons, the node menu
//! and the command palette run. Keys call the same `act_*` functions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // on a node
    Edit,
    Zoom,
    NewSibling,
    NewChild,
    ToggleDone,
    ToggleTask,
    Props,
    MakeBlock,
    MoveUp,
    MoveDown,
    Indent,
    Outdent,
    Respell,
    Refile,
    Archive,
    Copy,
    Delete,
    PasteAfter,
    PasteBefore,
    NodeMenu,
    // views and the vault
    ZoomOut,
    Filter,
    Palette,
    Help,
    Quit,
    Undo,
    Redo,
    Capture,
    CaptureTask,
    HideDone,
    RawMode,
    Wrap,
    ReadingPane,
    GoTo,
    ClearDone,
    Canonicalize,
    Merge,
    ResolveConflicts,
    ResolveConflict,
    // editor
    EditorKeys,
    EditDone,
    EditRevert,
    // popups
    Close,
    PromptOk,
    PropAdd,
    ConflictOurs,
    ConflictTheirs,
    ConflictBoth,
    ConflictEdit,
    ConflictPrev,
    ConflictNext,
}

impl Action {
    /// The name shown in menus and the palette.
    pub fn label(self) -> &'static str {
        self.shown().0
    }

    /// A one-cell icon for compact buttons.
    pub fn icon(self) -> Option<&'static str> {
        Some(self.shown().1).filter(|i| !i.is_empty())
    }

    /// The key that does the same, shown beside the label.
    pub fn key(self) -> Option<&'static str> {
        Some(self.shown().2).filter(|k| !k.is_empty())
    }

    /// What it does, for the palette.
    pub fn desc(self) -> &'static str {
        self.shown().3
    }

    /// Everything shown for an action, one line each: its label, icon and
    /// key (empty for none), and what it does.
    fn shown(self) -> (&'static str, &'static str, &'static str, &'static str) {
        use Action::*;
        match self {
            Edit => ("Edit", "✎", "e", "edit the subtree's Markdown"),
            Zoom => ("Zoom in", "", "Enter", "read this node on its own"),
            NewSibling => ("New sibling", "", "n", "insert an empty node after this one"),
            NewChild => ("New child", "", "N", "insert an empty last child"),
            ToggleDone => ("Done / reopen", "", "x", "check or uncheck the task"),
            ToggleTask => ("Task on / off", "", "t", "add or remove the checkbox"),
            Props => ("Properties…", "", "a", "edit the node's properties"),
            MakeBlock => ("Make block", "", "s", "give the node its own file"),
            MoveUp => ("Move up", "", "K", "swap with the previous sibling"),
            MoveDown => ("Move down", "", "J", "swap with the next sibling"),
            Indent => ("Indent", "", ">", "become the last child of the previous sibling"),
            Outdent => ("Outdent", "", "<", "become the next sibling of the parent"),
            Respell => ("Heading ↔ bullet", "", "~", "toggle heading / bullet spelling"),
            Refile => ("Move to…", "", "r", "move the subtree under another node"),
            Archive => ("Archive", "", "za", "move the subtree under # Archive"),
            Copy => ("Copy", "", "y", "copy the subtree"),
            Delete => ("Delete", "", "d", "move the subtree to the trash"),
            PasteAfter => ("Paste after", "", "p", "paste the copied subtree after this node"),
            PasteBefore => ("Paste before", "", "P", "paste the copied subtree before this node"),
            NodeMenu => ("Node menu", "⋯", "m", "every action on the selected node"),
            ZoomOut => ("Zoom out", "", "Bksp", "up one level"),
            Filter => ("Filter", "⌕", "/", "find nodes by title and text"),
            Palette => ("Commands", "☰", ":", "every action by name"),
            Help => ("Help", "?", "?", "how to use fold"),
            Quit => ("Quit", "", "q", "save and exit"),
            Undo => ("Undo", "↶", "u", "undo the last change"),
            Redo => ("Redo", "↷", "U", "redo the last undone change"),
            Capture => ("Capture", "+", "c", "append a note to today's inbox"),
            CaptureTask => ("Capture task", "", "C", "append a task to today's inbox"),
            HideDone => ("Hide / show done", "", "zd", "hide or show finished tasks"),
            RawMode => ("Raw / styled", "", "zr", "show the exact source"),
            Wrap => ("Wrap lines", "", "zw", "wrap long lines, or cut them at the edge"),
            ReadingPane => ("Reading pane", "◨", "zp", "show or hide the pane beside the outline"),
            GoTo => ("Go to…", "", "", "jump to a node"),
            ClearDone => ("Clear done", "", "", "trash finished tasks under the zoom"),
            Canonicalize => ("Canonicalize", "", "", "rewrite the vault in canonical form"),
            Merge => ("Merge sync conflicts", "", "", "fold sync-conflict files in"),
            ResolveConflicts => ("Resolve conflicts", "", "", "review conflict pairs"),
            ResolveConflict => ("Resolve conflict…", "", "", "the conflict view at this node's pair"),
            EditorKeys => ("Editor keys", "", "", "switch the editor between normal, Vim and Helix keys"),
            EditDone => ("Done", "✓", "Esc", "save and leave the editor"),
            EditRevert => ("Revert", "↺", ":q!", "drop changes since the last save"),
            Close => ("Close", "✕", "Esc", "close"),
            PromptOk => ("OK", "", "Enter", "accept"),
            PropAdd => ("Add", "+", "", "add a property"),
            ConflictOurs => ("Keep ours", "", "o", "keep this device's version"),
            ConflictTheirs => ("Keep theirs", "", "t", "keep the other device's version"),
            ConflictBoth => ("Keep both", "", "b", "keep both as siblings"),
            ConflictEdit => ("Edit ours", "", "e", "edit this device's version"),
            ConflictPrev => ("Previous", "◀", "N", "previous pair"),
            ConflictNext => ("Next", "▶", "n", "next pair"),
        }
    }
}

/// The node menu (right-click, `⋯`, `m`): `None` is a separator.
pub const NODE_MENU: &[Option<Action>] = &[
    Some(Action::Edit),
    Some(Action::Zoom),
    Some(Action::Props),
    None,
    Some(Action::NewSibling),
    Some(Action::NewChild),
    None,
    Some(Action::ToggleDone),
    Some(Action::ToggleTask),
    Some(Action::Respell),
    Some(Action::MakeBlock),
    None,
    Some(Action::MoveUp),
    Some(Action::MoveDown),
    Some(Action::Indent),
    Some(Action::Outdent),
    Some(Action::Refile),
    Some(Action::Archive),
    None,
    Some(Action::Copy),
    Some(Action::PasteAfter),
    Some(Action::PasteBefore),
    Some(Action::Delete),
];

/// What the node menu adds on either side of a conflict pair (§12.5).
pub const CONFLICT_MENU: &[Option<Action>] = &[None, Some(Action::ResolveConflict)];

/// Everything in the command palette, in order.
pub const PALETTE: &[Action] = &[
    Action::Edit,
    Action::Zoom,
    Action::ZoomOut,
    Action::GoTo,
    Action::Filter,
    Action::NewSibling,
    Action::NewChild,
    Action::ToggleDone,
    Action::ToggleTask,
    Action::Props,
    Action::MakeBlock,
    Action::Respell,
    Action::MoveUp,
    Action::MoveDown,
    Action::Indent,
    Action::Outdent,
    Action::Refile,
    Action::Archive,
    Action::Copy,
    Action::PasteAfter,
    Action::PasteBefore,
    Action::Delete,
    Action::Capture,
    Action::CaptureTask,
    Action::Undo,
    Action::Redo,
    Action::HideDone,
    Action::RawMode,
    Action::Wrap,
    Action::ReadingPane,
    Action::EditorKeys,
    Action::ClearDone,
    Action::Canonicalize,
    Action::Merge,
    Action::ResolveConflicts,
    Action::Help,
    Action::Quit,
];
