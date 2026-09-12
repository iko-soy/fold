//! fold-tui: the ratatui application.

use std::path::Path;

pub mod app;

pub fn run(vault_dir: &Path) -> anyhow::Result<()> {
    app::run(vault_dir)
}
