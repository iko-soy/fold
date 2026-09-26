//! fold-tui: the ratatui application.

use std::path::Path;

pub mod app;

/// Run the TUI; `keys` picks the editor keymap (`normal`, `vim`, `helix`).
pub fn run(vault_dir: &Path, keys: Option<&str>) -> anyhow::Result<()> {
    app::run(vault_dir, keys)
}
