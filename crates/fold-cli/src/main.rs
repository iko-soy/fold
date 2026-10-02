//! `fold` — a tree-shaped plain-text notes and task manager (§13).

use clap::{Parser, Subcommand};
use fold_core::vault::{trash_dir, Vault};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "fold", version, about = "tree-shaped plain-text notes and tasks", disable_help_subcommand = true)]
struct Cli {
    /// Vault directory (default: $FOLD_VAULT, else nearest ancestor of $PWD
    /// containing root.md, else ~/fold).
    #[arg(long, global = true)]
    vault: Option<PathBuf>,

    /// The editor's keymap: normal (micro-like), vim or helix (default:
    /// $FOLD_KEYS, else normal).
    #[arg(long, value_name = "KEYMAP")]
    keys: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Append to the inbox (stdin if no TEXT) (§7).
    Capture {
        text: Option<String>,
        /// Capture under any node instead of the inbox.
        #[arg(long)]
        to: Option<String>,
        /// Make it a task.
        #[arg(long)]
        task: bool,
    },
    /// Diagnostics with source spans; --fix canonicalizes and repairs
    /// filenames (§15.7).
    Check {
        #[arg(long)]
        fix: bool,
    },
    /// Process sync-conflict files non-interactively (§12).
    Merge {
        #[arg(long)]
        dry_run: bool,
    },
    /// Trash management (§11.5).
    Trash {
        #[command(subcommand)]
        action: TrashAction,
    },
    /// How to use fold: keys, concepts, workflow.
    Help,
}

#[derive(Subcommand)]
enum TrashAction {
    List,
    Restore { id: String },
}

fn vault_dir(cli_vault: &Option<PathBuf>) -> PathBuf {
    if let Some(v) = cli_vault {
        return v.clone();
    }
    if let Ok(v) = std::env::var("FOLD_VAULT") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    // nearest ancestor of $PWD containing root.md
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if dir.join("root.md").exists() {
            return dir;
        }
        if !dir.pop() {
            break;
        }
    }
    directories_home().join("fold")
}

fn directories_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = vault_dir(&cli.vault);
    // only the TUI sets a vault up (§13); a mistyped --vault is an error
    // for every other command rather than a new directory
    let needs_vault = !matches!(cli.command, None | Some(Command::Help) | Some(Command::Trash { .. }));
    if needs_vault && !dir.is_dir() {
        anyhow::bail!("no vault at {}", dir.display());
    }
    match cli.command {
        None => {
            // open the TUI (§13)
            fold_tui::run(&dir, cli.keys.as_deref())?;
        }
        Some(Command::Capture { text, to, task }) => {
            let mut text = match text {
                Some(t) => t,
                None => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf.trim_end().to_string()
                }
            };
            if let Some(stripped) = text.strip_prefix("[ ]") {
                text = stripped.trim().to_string();
                return capture(&dir, &text, true, to);
            }
            capture(&dir, &text, task, to)?;
        }
        Some(Command::Check { fix }) => {
            let mut v = Vault::open(&dir)?;
            if fix {
                let n = fold_core::check::fix(&mut v)?;
                println!("{} file(s) rewritten or renamed", n);
            }
            let diags = fold_core::check::check(&v);
            for d in &diags {
                println!("{}", d);
            }
            if !diags.is_empty() && !fix {
                std::process::exit(1);
            }
        }
        Some(Command::Merge { dry_run }) => {
            let mut v = Vault::open(&dir)?;
            let outcomes = fold_core::merge::merge_sync_conflicts(&mut v, dry_run)?;
            if outcomes.is_empty() {
                println!("no sync-conflict files");
            }
            for o in outcomes {
                println!("{}", o);
            }
            // list leftovers: unresolved conflict blocks (§13)
            let pairs = fold_core::merge::conflict_pairs(&v);
            if !pairs.is_empty() {
                println!("{} unresolved conflict pair(s)", pairs.len());
            }
        }
        Some(Command::Trash { action }) => match action {
            TrashAction::List => {
                let trash = trash_dir();
                let mut entries: Vec<_> = std::fs::read_dir(&trash)
                    .map(|rd| rd.filter_map(|e| e.ok()).collect())
                    .unwrap_or_default();
                entries.sort_by_key(|e| e.file_name());
                for e in entries {
                    println!("{}", e.file_name().to_string_lossy());
                }
            }
            TrashAction::Restore { id } => {
                let trash = trash_dir();
                let mut matches: Vec<_> = std::fs::read_dir(&trash)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .filter(|e| e.file_name().to_string_lossy().contains(&id))
                            .collect()
                    })
                    .unwrap_or_default();
                match matches.len() {
                    0 => anyhow::bail!("no trash entry matching {:?}", id),
                    1 => {
                        let e = matches.remove(0);
                        let name = e.file_name().to_string_lossy().to_string();
                        // strip the `<yyyymmdd-hhmmss>-` stamp (§11.5)
                        let restored = if name.len() > 16 && name.as_bytes()[8] == b'-' && name.as_bytes()[15] == b'-' {
                            name[16..].to_string()
                        } else {
                            name.clone()
                        };
                        // text the editor could not save (§10.6) has no id:
                        // moved in, it would be a file fold ignores, so it
                        // is given back and stays where it is. The name
                        // alone does not say so, as a block file may be
                        // named anything (§3.4): one with an id goes back.
                        let unsaved = restored
                            .starts_with("unsaved-")
                            .then(|| std::fs::read_to_string(e.path()))
                            .transpose()?
                            .filter(|text| {
                                fold_core::parse::parse_frontmatter(text).and_then(|fm| fm.id()).is_none()
                            });
                        if let Some(text) = unsaved {
                            print!("{}", text);
                            eprintln!(
                                "{} stays in the trash: it is text the editor could not save, not a file fold reads; paste what you need into the editor",
                                e.path().display()
                            );
                            return Ok(());
                        }
                        if !dir.is_dir() {
                            anyhow::bail!("no vault at {}", dir.display());
                        }
                        // never restore over an existing file, root.md least of all
                        let target = if restored != "root.md" && !dir.join(&restored).exists() {
                            restored.clone()
                        } else {
                            let stem = restored.strip_suffix(".md").unwrap_or(&restored);
                            (2..)
                                .map(|i| format!("{}-restored-{}.md", stem, i))
                                .find(|n| !dir.join(n).exists())
                                .unwrap()
                        };
                        fold_core::vault::move_file(&e.path(), &dir.join(&target))?;
                        println!("restored {}", target);
                    }
                    _ => anyhow::bail!("{:?} matches {} trash entries", id, matches.len()),
                }
            }
        },
        Some(Command::Help) => {
            for line in fold_tui::app::help_text() {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                println!("{}", text);
            }
        }
    }
    Ok(())
}

fn capture(dir: &Path, text: &str, task: bool, to: Option<String>) -> anyhow::Result<()> {
    let mut v = Vault::open(dir)?;
    match to {
        Some(target) => {
            let r = v
                .resolve_target(&target)
                .map_err(|e| anyhow::anyhow!(e))?;
            fold_core::ops::capture_to(&mut v, text, task, r)?;
        }
        None => {
            fold_core::ops::capture(&mut v, text, task)?;
        }
    }
    Ok(())
}
