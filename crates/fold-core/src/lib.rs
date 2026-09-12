//! fold-core: parsing, tree, render/splice, index, store, merge — no TUI deps.

pub mod check;
pub mod edit;
pub mod ident;
pub mod merge;
pub mod ops;
pub mod parse;
pub mod reading;
pub mod render;
pub mod syllables;
pub mod tree;
pub mod vault;

pub use ident::{slug, Id};
pub use parse::{Block, Diag, Kind, Node, ParsedFile, Span, TaskState};
pub use render::render;
pub use tree::{NRef, Tree};
