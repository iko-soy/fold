# fold

One outline of notes and tasks, stored as plain Markdown, in the terminal.

Everything you write — projects, notes, tasks, an inbox — is one tree. Headings and
bullets are its nodes; you read and edit any subtree on its own, move subtrees around,
and check tasks off. The files stay ordinary Markdown that `rg`, your editor and your
phone can read, and they sync with Syncthing.

```
 fold › Homelab › NAS                 ⌕ Filter  + Capture  ↶ Undo  ↷ Redo  ☰ Commands  ? Help
╭ Outline ───────────────────────╮╭ ZFS layout ▤ ─────────────────────────── ✎ Edit ─ ⋯ ─╮
│▾ Homelab                  2/4  ││ # ZFS layout                                          │
│  ▾ NAS                    1/2  ││ ⚑ since 2024-03                                       │
│    ▾ ZFS layout ▤         1/2 ⋯││                                                       │
│      ▾ ☐ Snapshot policy       ││ Mirrored pairs, no raidz. Snapshots hourly.           │
│  ▸ Networking             1/2  ││ ## ☐ Snapshot policy                                  │
│▾ Inbox                         ││ - hourly, keep 24                                     │
╰────────────────────────────────╯╰───────────────────────────────────────────────────────╯
```

## Run it

With Nix:

```sh
nix run github:iko-soy/fold            # the TUI on your vault
nix run github:iko-soy/fold -- --keys vim
```

With Cargo (a recent stable Rust, and a C compiler for the syntax-highlighting grammars):

```sh
cargo install --path crates/fold-cli
fold
```

The vault is `--vault PATH`, else `$FOLD_VAULT`, else the nearest directory above the
current one that contains `root.md`, else `~/fold`. Pointed at an empty directory, the TUI
starts a new vault with an `# Inbox`.

## Using it

The TUI is mouse-first, and every action also has a key.

- **Click** a row to select it. Click `◨` in the top bar (or press `zp`) to show the
  reading pane beside the outline, which shows the selected node. Click `▸`/`▾` to
  fold, `☐` to check a task off, a breadcrumb in the top bar to zoom out.
- **Double-click** a row to zoom into it. Double-click a line of text to edit it there.
- **Right-click** a row (or click its `⋯`) for everything you can do to a node: edit, new
  sibling or child, done, properties, move, indent, make a block, copy, paste, delete.
- **Drag** a row onto another row's title to nest it there, or to the left of a title to
  put it just before.
- **Filter** (`/`) finds nodes by title and text; **Commands** (`:`) lists every action by
  name; **Capture** (`c`) drops a note into today's inbox.

Keys, in the outline: `j`/`k` move, `h`/`l` fold, `Enter` zooms, `e` edits, `x` checks a
task, `n`/`N` add a sibling or child, `J`/`K` move a node, `>`/`<` indent, `r` moves a
subtree elsewhere, `u`/`U` undo and redo, `?` shows everything. Everything is saved as you
go; `q` quits.

### The editor

`e` edits the selected subtree's Markdown in the reading pane. It saves on its own — when
you pause, when you leave a part of the tree that lives in another file, and when you're
done. Pick a keymap with `--keys` or `$FOLD_KEYS`, or click `⌨` in the editor's border:

- **normal** (default) — a conventional editor like micro: Shift+arrows select,
  `Ctrl-C`/`X`/`V`, `Ctrl-Z`/`Y`, `Ctrl-S`, `Ctrl-F`.
- **vim** — modes, counts, motions, operators, text objects, `.`, `/`, `:w`/`:q`.
- **helix** — selection first: `w`/`x` select, `d`/`c`/`y` act on the selection.

Long lines wrap (`zw` toggles), fenced code is syntax-highlighted in about 140 languages,
and pasting from the terminal and copying to the system clipboard both work.

## The files

A vault is a directory with `root.md` and, optionally, one file per **block**:

```markdown
# Homelab

Two boxes in the closet, one at Hetzner.

## NAS

### ![[dozzod-binwes-talsun-worbec]]

## Networking

- [ ] Replace the flaky switch
![[racfer-hattes-mislup-nodrys]]
```

- A heading or a bullet is a node; nesting is the tree. Headings can sit under bullets,
  and levels go past six.
- A checkbox after the marker makes any node a task: `- [ ]`, `## [x]`.
- A node that gets a property (a due date, say) becomes a **block**: its own file,
  `racfer~order-new-switch.md`, with YAML frontmatter and an `id`, stitched into its parent
  by `![[id]]`. That's how a single task gets a due date, and how the inbox becomes a small
  file you can append to from a phone.

Nothing else is special. Deleting goes to a trash (`$XDG_STATE_HOME/fold/trash`), and
Syncthing conflict copies are merged node by node, with anything that differs kept as a
`conflict:` block to resolve in the app.

## The command line

```sh
fold                                  # the TUI
fold capture "call the plumber"       # append to today's inbox (stdin if no text)
fold capture --task --to Homelab/NAS "check the scrub"
fold check [--fix]                    # diagnostics; --fix rewrites in canonical form
fold merge [--dry-run]                # fold Syncthing conflict files in
fold trash list | fold trash restore ID
fold help
```

## Developing

```sh
nix develop          # cargo, rustc, clippy, rustfmt, rust-analyzer
cargo test --workspace
nix flake check      # the same tests, in the Nix sandbox
```

The code is three crates: `fold-core` (parsing, rendering, the operations, merge),
`fold-tui` (the ratatui app) and `fold-cli` (the `fold` binary). [SPEC.md](SPEC.md) is the
design: the data model, the file format, every operation and the laws they keep.
