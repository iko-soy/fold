# SPEC — a tree-shaped plain-text notes and task manager for the terminal

Status: draft 0.38 · 2026-09-12
Working name: not chosen yet. This document uses `notes` as the binary name; rename freely.
Language: Rust · TUI: ratatui · Sync: Syncthing · History: file versioning on one node

Changes in 0.38: no external editor in 1.0. `E`, the inline-frontmatter spelling, the
`inline` render mode and `$EDITOR` are gone; the built-in editor is the only editor (§10.6,
§17). Frontmatter never appears as text anywhere.

Changes in 0.37: `E` is scoped to the selected node and replaces its subtree wholesale on
exit. The text handed to `$EDITOR` carries every nested block's frontmatter **inline**, as a
`---` block under its title line, `id` included, so blocks come back matched exactly by id;
no line diff, no heuristics (§4.4, §10.6). `e` is unchanged.

Changes in 0.36: editing is continuous. The built-in editor is one text over the resolved
subtree with no locked lines, no dimming and no commit key: every line carries its owning
block invisibly, edits apply to whichever block the cursor is in, and blocks are saved
automatically when the cursor leaves them, after a short pause, and on exit (§5.2, §10.6).
The *make block* / *cut* distinction is not visible while typing.

Changes in 0.35: vocabulary. *Document*, *part* and *cut* were one concept and are now
**block**: a node with an id, a file and properties, edited alone. *Edge* is **embed**,
which is what `![[id]]` is. The *document pane* is the *reading pane*. Storage sections
still say "file" where they mean bytes on disk.

Changes in 0.34: every edit changes exactly one file. The editor still shows the whole
resolved subtree, but only the lines owned by one **part** — the file the cursor node lives
in — are editable; the rest is context, shown but locked, and the status line names the
part. Splice is single-file again; the multi-file distributor is gone (§5.2, §10.6).

Changes in 0.33: properties never appear in text. The resolved rendering the user reads and
edits has no frontmatter and no property blocks; properties are edited in a form (`a`,
§10.6) and shown as a dimmed header in the document pane. Splice never touches
frontmatter.

Changes in 0.32: files are invisible when editing. The user edits a slice of one large
document: `e`/`E` open the **resolved** subtree — documents inlined, no `![[id]]`, no ids —
with each node's properties shown as a `---` block under its title line (§4.4). Splice
parses the result and distributes it back to whichever files own the text, cutting,
moving and trashing files as the text requires (§5.2). Files are storage, not UI.

Changes in 0.31: one editing model. `e` opens the selected subtree's Markdown —
`render(cursor, 1, false)`, frontmatter included — in an editor and splices it back; `E`
does the same in `$EDITOR`. Title editing, the block editor and the property editor are
gone: a title, a property, a body are all just text in the subtree (§10.6).

Changes in 0.30: the `:` command line is a command palette — a fuzzy picker over every
action, showing its key, prompting for arguments after selection (§10.9). No command syntax
to learn or type.

Changes in 0.29: frontmatter is text. No value types, no `tags` key, no `--set`/`--due` on
capture, no relative date forms. The app knows five keys — `id`, `todo`, `due`, `done`,
`conflict` — reads them as strings, validates the two dates as ISO when it writes them, and
preserves every other line verbatim (§3.2, §4.4).

Changes in 0.28: the query language, saved views, query blocks, the `v` picker, result
views, `notes query` and `--json` are out of 1.0 (§17). `query` fences are reserved. The
fresh vault is `# Inbox` alone. The filter box (§10.5) is the only search.

Changes in 0.27: no config file. `config.toml` and its seven keys are gone; the vault comes
from `$NOTES_VAULT` or the working directory, the editor from `$EDITOR`, and the rest is
fixed or a session toggle (`zd`, `zr`). The trash is the only thing outside the vault (§14).

Changes in 0.26: the CLI is six commands: `notes`, `capture`, `query`, `check [--fix]`,
`merge`, `trash`. `init`, `config`, `view`, `ls`, `set`, `unset`, `cut`, `archive`,
`canonicalize` and `adopt` are gone; the TUI does each of them, and `check --fix` absorbs
canonicalize (§13). Foreign `.md` files can no longer be adopted; paste their text instead.

Changes in 0.25: more removals. The cancelled task state (`[-]`, `todo: cancelled`,
`cancelled:`, `X`) and the `priority` and `repeat` keys are gone. No tabs, no `:sort`, no
create-from-filter-box, no `notes inbox` / `props` / `id`, no `keys.*` rebinding or theme,
no `Ctrl-w`. The `Waiting` default view is gone and the `Inbox` view is now `Queue`.
`notes render`, `:export` and the `--plain` lowering are removed: there is no export in 1.0
(§17); `render` remains an internal function.

Changes in 0.24: seven removals. One edge shape, `![[id]]`, and a document's file starts
with the node as spelled — no more `#`-always rule (§4.7, §4.9). No in-session three-way
merge: a splice that finds its span changed writes a conflict document like any other
conflict (§5.2). No fuzzy title matching in merge (§12.4). Files without a valid `id` are
ignored, not "unmanaged" (§4.1). Archive is plain refile under `Archive` (§6.5). No
`--as dir/`: the vault is flat and prefixes are unique vault-wide (§6.4). No `conflicts/`
directory and no `conflict_with` key: conflict copies are ordinary documents marked
`conflict:`, stitched in next to ours (§12.4).

Changes in 0.23: no cache and no per-device state. The index is rebuilt from the vault on
every start; there are no shadow copies, so sync-conflict merging is two-way (§11.3, §12.3).
The in-session splice conflict keeps its three-way base in memory.

Changes in 0.22: `uncut` removed. A cut is permanent; a document stays a file until deleted.
Conflict resolution copies text instead of uncutting (§12.5).

Changes in 0.21: a document's task state is the frontmatter key `todo: open | done |
cancelled`, not a checkbox on its title line. Checkboxes remain the syntax for nodes that
are not files. Cut and uncut convert between the two (§4.5, §8).

Changes in 0.20: the separator between id prefix and name in filenames is `~`:
`racfer~order-new-switch.md`. Words inside the prefix stay `-`-joined.

Changes in 0.19: filename prefixes are unique on their own, ignoring the title: no two files
in a directory share a prefix, whatever their names (§6.4).

Changes in 0.18: ids are 64 bits, four `@p` words (`racfer-hattes-dozzod-binwes`). Filenames
start with the shortest prefix of the id — one word by default — that is unique in the
directory, and grow a word at a time when a collision is found, locally or at merge (§6.4,
§12.2). The `-2` suffix is gone.

Changes in 0.17: filenames are `<word>-<name>.md`, where `<word>` is the first word of the
id (`racfer-order-new-switch.md`). Same-title documents no longer collide across devices
except at 1 in 65,536; the merge engine treats an id mismatch as a name collision, not a
conflict (§6.4, §12.2).

Changes in 0.16: `@key(value)` annotations removed. All metadata is frontmatter again, so
only documents carry it: `done:` and `cancelled:` are stamped on task documents, inline
tasks record state only, and nothing records creation time. Conflict copies are documents
in `conflicts/` again, with `conflict:` frontmatter. `@` appears only in the query language.

Changes in 0.15: the "no syntax of our own" restriction is lifted (§1). Consequences:
one structural node kind with two spellings, either nestable under either (§3.1);
heading levels unbounded, `§5.3` level overflow gone; any node can be a task, headings
included (§4.5); a document's task state lives on its own title line, edges carry none
(§4.7, §4.9); the app stamps `@created`, `@done`, `@cancelled` as title-line annotations on
any node (§3.2); the id moves from the filename into `id:` frontmatter, filenames are plain
slugs, edges are `![[id]]` (§3.4, §6.4); `|` is allowed in titles; merge conflicts are
inline `@conflict` siblings, `conflicts/` is gone (§12.4). `notes render --plain` lowers
the format to standard Markdown for export (§5.1).

Changes in 0.14: `[[wiki-links]]` between nodes removed from 1.0 — no link syntax, resolution,
rename rewriting, backlinks or tombstones. `[[…]]` in titles and bodies is opaque text,
preserved verbatim and reserved (§4.6, §17). Edges (`![[id-name]]`) and external
`[text](url)` links are unchanged.

Changes in 0.13: capture appends under a `## <today>` child of `Inbox`, created on demand.
Fixed behaviour, not a setting; the day title is ISO `YYYY-MM-DD`.

Changes in 0.12: every path component the app writes is lowercase — file names, directory
names (including `--as`), temp and trash names. Titles keep their case; resolution is
case-insensitive.

Changes in 0.11: no vault settings at all. Capture always goes to the level-1 section titled
`Inbox`, created if missing. `root.md` has no frontmatter in a fresh vault.

Changes in 0.10: `done_precision` and `conflicts` removed. `done:` records a date; conflict
documents live in `conflicts/`. `inbox` is the only vault setting.

Changes in 0.9: `archive` and `:archive-done` removed — done items stay where they are,
a display toggle hides them, and `:clear-done` trashes them when a list is finished.
(An `archive` section as a plain refile target returns later — see §6.5.)
`suggest_bytes` removed — the app never suggests cuts. Three vault settings remain:
`inbox`, `done_precision`, `conflicts`.

Changes in 0.8: `capture` renamed `inbox` and is a plain link target (no date template);
`children: files` removed, so there are no section settings and no derived directories;
`cut`, `rename`, `transliterate` and `id_words` removed — cuts are property-triggered or
manual, filenames follow titles, names keep Unicode, ids are two words. Five vault settings
remain: `inbox`, `archive`, `done_precision`, `conflicts`, `suggest_bytes`.

Changes in 0.7: `dir` and `sort` removed. The only section setting is `children: files`;
its files go in a directory named after the section. New children are always appended.

Changes in 0.6: `config.toml` is gone. Vault settings are `root.md`'s frontmatter; section
settings (`children`) are the section's own frontmatter; saved views are query
blocks. A fresh vault is initialised with a `root.md` that lists every setting at its default,
commented. Only an optional, device-local file for UI preferences remains.

Changes in 0.5: the journal is no longer a concept. Capture appends to a configurable,
date-templated target (default `Inbox/{date}`), and "children of this section are always
files" is a general `[[cut.auto]]` rule. Journal days become ordinary documents; the id-less
filename shape and the date resolution step are gone.

Changes in 0.4: ids are two-word phonemic names in Urbit's `@p` syllable scheme
(`racfer-hattes`, 32 bits) instead of 8-character base32.

Changes in 0.3: document files are named `<id>-<name>.md`, where the id is an 8-character
random base32 string and the name is a slug of the title. Links and edges resolve by id, so
they survive renames and stale rewrites; the name is decoration for humans and other tools.

Changes in 0.2: all inline attribute syntax (`@key(value)`) removed. Properties exist only as
YAML frontmatter of a file, so a node that needs properties becomes a file ("document").
Items can be cut into documents; their checkbox stays on the edge in the parent list.
`@id` removed; the file slug is the identity. Merge conflicts are stored as documents in
`conflicts/`. Task-state syntax `[ ]`, `[x]`, `[-]` is unchanged.

---

## 1. Purpose and principles

A keyboard-driven TUI for a single body of notes stored as Markdown. There is one organizing
primitive — the **subtree** — and one organizing verb — **refile** (move a subtree under a
different parent). Everything else (inbox, tasks, projects, index) is a view over the
same tree.

Principles, in priority order:

1. **The files are the database.** Every fact the app knows is recoverable from the Markdown.
   There are no caches, no configuration, and no per-device state beyond the trash. The
   app never writes a byte it didn't need to change.
2. **Markdown-shaped, with our own syntax where it pays.** Headings, bullets, checkboxes,
   fences and YAML frontmatter are used as they are. On top of them the format owns a small,
   closed set of things a standard renderer will not understand: `![[id]]` embeds, headings
   deeper than six, checkboxes on headings, headings nested under bullets. The set is listed
   in §4.2 and is small enough to lower to standard Markdown later (§17).
3. **Zoom is the unit of reading, editing and splitting.** A single function,
   `render(node, base)`, produces the document you read, the text you edit, and the file
   a block lives in.
3b. **The user reads and edits one document.** The vault is stored as many files, but nothing
   the user reads or types shows a file boundary, embed or id. Every line knows which block
   owns it; the editor routes each change to that block's file, and saves blocks on its own
   as the cursor moves and the typing pauses. Files are how the app keeps its promises, not
   something the user operates (§5.2, §10.6).
4. **Safe beside other editors and beside a phone.** Atomic writes, external-change
   detection, and structural conflict merging are core features, not add-ons.
5. **Legible without the app.** Plain `rg`, Helix and a phone editor must produce something
   sensible from the same files. Standard-Markdown export is a non-goal for 1.0 (§17).

---

## 2. Terminology

| Term | Meaning |
|---|---|
| Vault | A directory containing `root.md` and, optionally, block files. |
| Tree | The single logical outline formed by `root.md` plus all block files stitched in. |
| Node | One thing in the outline: a title line, a body, children. Spelled as a section or an item. |
| Section | A node spelled as an ATX heading `#`, `##`, … (no upper bound). |
| Item | A node spelled as a `- ` bullet. |
| Spelling | Whether a node is written as a heading or a bullet. Presentation only (§3.1). |
| Task | A non-block node with a checkbox after its marker, or a block with a `todo` key. |
| Block | A node with its own identity and file. Only blocks have properties; every edit changes exactly one block. |
| Id | A random four-word phonemic name (`racfer-hattes-dozzod-binwes`) in a block's `id:` frontmatter; its stable identity. |
| Name | The slug of a block's title. With a prefix of the id in front, its filename. Decoration, not identity. |
| Property | A key in a block's YAML frontmatter. |
| Body | Block content belonging to a node before its first child. |
| Zoom | Viewing a node as a standalone block via `render(node, base)`. |
| Splice | Writing an edited zoomed block back into its source span. |
| Make a block | Giving a node an id and its own file, leaving an embed in the parent. |
| Embed | `![[id]]` alone on a line at the node's position in the parent. |
| Span | A byte range in a file. Every node knows its span. |
| Path | A node's address by ancestor titles: `Homelab/NAS/ZFS layout`. |

---

## 3. Data model

### 3.1 Nodes

There is one structural kind of node, with two **spellings**:

```
Root     — implicit; children are the top-level nodes of root.md; body is any preamble text
Section  — spelled as a heading; may hold prose; children of either spelling
Item     — spelled as a bullet; may hold note lines; children of either spelling
```

Rules:

- Any node may be the child of any node. A heading under a bullet is a section whose parent
  is an item; a bullet under a heading is an item whose parent is a section.
- Heading level and indent are **derived from position**, never stored:
  `level(n) = 1 + number of section ancestors of n`, and `indent(n) = 2 × number of item
  ancestors of n`. A section under an item is written at that item's child indent with its
  own level. There is no upper bound on levels.
- Spelling is presentation: `~` toggles it (§10.3) and nothing else changes. Both spellings
  can be tasks, hold bodies, be refiled anywhere, and be made into blocks.
- Sibling order is block order and is meaningful.
- Any node may be a **block** (the root of its own file). Its file starts with the node
  as it is spelled — a heading or a bullet — so the file is the only place its spelling
  lives (§4.9).

### 3.2 Properties

Only blocks have properties. They are the lines of the file's YAML frontmatter. The app
reads frontmatter to find its own five keys and otherwise treats it as text: every line it
did not write is preserved byte for byte, in place, whatever it contains — lists, nested
maps, comments, other tools' keys. There is no typing: a value is the string after the
colon, and only the app's two date keys are checked to be ISO dates, and only when the app
writes them.

- Keys: `[A-Za-z_][A-Za-z0-9_-]*`, case-sensitive.
- Key order is preserved as written; the app writes `id` first and appends new keys at the end.

Giving a property to a node that is not a block **makes it one** (§6.1). This is the only way
a property can come into existence, and it is automatic and permanent: there is no uncut.
There is no other per-node metadata: a node that is not a block has a title, a checkbox,
a body and children, and nothing else.

Reserved keys (the app assigns semantics; users may still read/write them by hand):

| Key | Value | Meaning |
|---|---|---|
| `id` | four `@p` words | The block's identity (§3.4). Present on every managed file; never on `root.md`. |
| `todo` | `open` \| `done` | Makes the block a task and holds its state (§4.5). Absent means not a task. |
| `due` | `YYYY-MM-DD` | Task due date; shown in the outline. |
| `done` | `YYYY-MM-DD` | Written when a task block is checked; removed when unchecked. |
| `conflict` | `<device> <timestamp>` | Marks a "theirs" copy written by the merge engine (§12.4). |

There are no setting keys. Nothing in any frontmatter changes how the app behaves.

New children are always appended after the last existing child; sibling order is manual and
nothing reorders it on insert.

Everything else (`tags`, `priority`, `est`, `waiting`, `since`, `author`, `source`, …) is
user-defined, and the app never looks at it.

### 3.3 Bodies

A body is an ordered list of raw lines, opaque to the tree. The parser only needs to know
enough to find the *next title line*: it tracks fenced code blocks (```` ``` ```` and `~~~`,
any indent) so that `#` or `- ` inside a fence never starts a node. Paragraphs, quotes,
tables, images, HTML and code are all body.

For items, body lines are those indented at least `indent + 2` that are not child title
lines (bullets or headings). For sections, body is everything from the title line to the
first child, at the section's own indent.

### 3.4 Identity and addressing

A block's identity is its **id**: a four-word phonemic name such as
`racfer-hattes-dozzod-binwes`, 64 random bits encoded with Urbit's `@p` syllable tables
(§6.4), generated from OS randomness
when the block is created and stored as the `id` key of its frontmatter. Its filename is
`<prefix>~<name>.md`: the shortest leading run of the id's words that no other file in the
directory uses as its prefix — usually one — then the slug of its title
(`racfer~order-new-switch.md`). The prefix alone identifies the file; the name is for humans. The filename is decoration that any tool may change and the
app will repair (§6.4). The one exception is `root.md`, which has no
id. A non-block node's identity is its **path**.

Embeds reference the id alone (`![[racfer-hattes-dozzod-binwes]]`), so nothing in the vault
depends on a filename and a rename touches only the renamed file.

Resolution order for a command target (the *refile*, *go to* and *make block* prompts, `--to`, …):

1. **Id** — the target is a valid id, or a leading run of its words that matches exactly
   one block (`racfer`, `racfer-hattes`, …), with or without `![[ ]]`.
2. **Path** — segments separated by `/`, matched case-insensitively against titles from the
   root (`Homelab/NAS`). A leading `./` resolves relative to the current zoom root.
3. **Title** — a bare name matching exactly one node's title. Dated inbox days
   (`2026-09-10`) resolve this way; there is nothing special about them.

Ambiguous or missing targets are an error for the command that named them; broken or
duplicate embeds are diagnostics in `notes check` (§15.7).

Renaming a block's title renames its file (§6.4). Nothing else references the filename.

Titles may not contain `/` (the path separator). `[` and `]` are discouraged.

### 3.5 Derived facts

Computed by the index, never stored in files: heading level, indent, path, file, span,
open/done task counts per subtree (`3/7` in the outline), per-file mtime. Per-node
timestamps are not tracked by the app (§12.6).

---

## 4. File format

### 4.1 Vault layout

```
vault/
  root.md              # the tree; with no blocks this is the whole database
  <prefix>~<name>.md   # block files (0..n), e.g. racfer~order-new-switch.md
  assets/              # anything non-Markdown; ignored by the parser
```

The vault is flat: every block sits beside `root.md`. Only `root.md` and `.md` files
whose frontmatter carries a valid `id` (§6.4) are parsed. Everything else — subdirectories,
non-Markdown files, `.md` files without an id — is ignored: never parsed, never written,
never deleted. There is no import: to bring a foreign `.md` file in, paste its text into
a node (or `notes capture < file.md`) and let the app assign ids as it makes blocks. `notes check`
lists ignored `.md` files so nothing is forgotten in the vault.

There is no configuration and no configuration file. The inbox is the section titled
`Inbox` (§7). The app owns no file in the vault besides the
Markdown it is asked to write.

A fresh vault is `root.md` alone, and stays so until a node acquires a property or you make
it a block by hand.

### 4.1.1 A fresh vault

The TUI, when pointed at an empty directory, writes this `root.md`:

````markdown
# Inbox

Captures land here under a heading for the day. Press `s` on this section to give it its own small file for phone capture.
````

`Inbox` is an ordinary section: rename it, move it, delete it. Deleting it only means the
next capture recreates it.

### 4.2 Canonical subset

The app **reads** any CommonMark it can make sense of and **writes** this subset:

- UTF-8, `\n` line endings, file ends with exactly one `\n`, no trailing whitespace.
- Frontmatter, if present, is the first thing in the file: `---`, YAML, `---`, blank line.
  `id` is the first key. The app writes its own keys as `key: value` on one line each and
  leaves every other line as it found it.
- ATX headings only, `#`+ then a space, at the node's indent. No upper bound on level.
  Setext headings are converted on write.
- Bullets are `- ` only. `*` and `+` are accepted on read.
- Nesting is 2 spaces per item ancestor (§3.1). Tabs and 4-space indents are accepted on read.
- Checkboxes are exactly `[ ]` and `[x]`, between the marker and the title, on headings
  or bullets alike. `[X]` is accepted on read; `[-]` is read as `[x]`.
- Exactly one blank line between a node's body and its first child, and between sibling
  sections. Loose/tight lists are preserved as found.

Canonicalization is **lazy**: a node is rewritten in canonical form only when it is touched
(edited, spliced, moved). `notes check --fix` (or *canonicalize* in the palette) rewrites
everything at once.

**What is ours.** A standard Markdown renderer will mishandle exactly these four things:
`![[id]]` embeds, heading levels beyond six, checkboxes on headings, and headings indented
under bullets. Nothing else in a vault is non-standard.

### 4.3 Title lines

```
section  := indent "#"+ " " (checkbox " ")? title
item     := indent "- " (checkbox " ")? title
checkbox := "[" ("_" | "x") "]"                            -- "_" denotes a space
indent   := ("  ")*
title    := any text not starting with whitespace; may not contain "/"
```

There is no attribute syntax. An `@word` or `key:: value` in a title or body is text, and
so is `[[…]]` (reserved, §4.6).
A title line with an empty title is invalid; `notes check` reports it, the TUI won't create it.

### 4.4 Frontmatter

```
frontmatter := "---" NL yaml "---" NL
```

Recognised only at byte 0 of a file. It belongs to the file's block (§4.9). The app scans
it for top-level `key: value` lines, keeps the raw text, and rewrites only the lines for
keys it changes, so comments and any structure it does not understand survive untouched. A
`---` anywhere else in a file is a thematic break and is body text.

Inside the app, frontmatter is storage: the reading pane shows a block's properties as a
dimmed header computed from the index, the property editor (`a`, §10.6) changes them, and
the built-in editor's text (§5.1, `resolve_blocks = true`) contains none.

Frontmatter never appears as text to the user. (An inline `---`-under-the-title spelling
was designed for an external editor and is set aside with it, §17.)

### 4.5 Tasks

Any node can be a task. Where the state is written depends on whether the node is a file:

- A node that is **not** a block is a task iff it has a checkbox after its marker.
  `- [ ]` and `## [ ]` are both open tasks; `[x]` is done. That is all the state it has.
- A **block** is a task iff its frontmatter has `todo:`, whose value is the state:
  `open` or `done`. Its title line in the file carries no checkbox. Checking writes
  `todo: done` and `done: <date>`; reverting sets `todo: open` and removes the date.

There is no cancelled state. A task that will never be done is either checked off or
deleted.

Embeds carry no state. Cutting a checkbox task turns the checkbox into `todo:` (§6.1). A
checkbox on a block's own title line is
accepted on read as `todo` and rewritten on the next touch. A task section has its own
state *and* the derived count of its subtree. See §8.

### 4.6 Links

There are no links between nodes in 1.0. `[[target]]` and `[[target|label]]` are opaque
text wherever they appear: not parsed, not resolved, not rewritten on rename, not reported.
The syntax is **reserved** for a later version; write it by hand if another tool needs it.

External links are ordinary Markdown `[text](url)` and are opened with `xdg-open` / `open`.

### 4.7 Embeds

An embed is where a block is stitched into the tree: `![[id]]` alone on a line, at the
indent the node has at that position.

```
![[dozzod-binwes-talsun-worbec]]
  ![[racfer-hattes-mislup-nodrys]]     -- a child of the item above it
```

The embed names the block by id and nothing else; title, spelling, checkbox or `todo`,
properties, body and children all live in the file. An embed line has no children in the
parent file; child lines indented under an embed are a diagnostic.

`![[` anywhere else is body text. The app does not implement general transclusion.

### 4.8 Query blocks (reserved)

A fenced block with info string `query` is body text in 1.0. The info string is reserved
for a later query language (§17); leave such blocks alone and they will start working.

### 4.9 Block files

A block's file is exactly `render(node, 1, resolve_blocks = false)`:

```
[frontmatter, always beginning with id; todo: if the block is a task]
[the node's title line, as spelled: "# Title" or "- Title"]
[body]
[children]
```

- The root is written as it is spelled in the tree. A block that is a heading is a `#` at column 0; one
  that is a bullet is a `- ` at column 0, with its body and children indented under it exactly
  as they would be in the parent. The file is the only place the spelling is recorded.
- The block's task state is `todo:` in its frontmatter, with a `done:` date beside it
  when done; there is no checkbox in the file. Toggling the node — from the outline,
  or the embed — writes to the file.
- A file with text before the root other than frontmatter, or with more than one node at
  column 0, is a diagnostic; the app treats the file as read-only until fixed.
- `root.md` is the file of the implicit Root node; its frontmatter holds vault-level
  properties (rarely needed) and never an `id`.

### 4.10 Example

`root.md`:

```markdown
# Homelab

Two boxes in the closet, one at Hetzner.

## NAS

![[dozzod-binwes-talsun-worbec]]

## Networking

- [ ] Replace the flaky switch
![[racfer-hattes-mislup-nodrys]]

# Inbox

## 2026-09-10

- Talked to Anya about the venue.
  ### Options
  Warehouse on Ligovsky, or the old bakery. Both need a licence.
- [x] Send the deposit
```

`dozzod~zfs-layout.md`:

```markdown
---
id: dozzod-binwes-talsun-worbec
since: 2024-03
tags: [storage, homelab]   # user-defined; the app preserves it and ignores it
---

# ZFS layout

Mirrored pairs, no raidz. Snapshots hourly via sanoid.

## [ ] Snapshot policy

- hourly, keep 24
- [x] Move scratch to its own dataset
```

`racfer~order-new-switch.md`:

```markdown
---
id: racfer-hattes-mislup-nodrys
todo: open
due: 2026-09-20
---

- Order new switch
  Two options, noted under Networking.
```

Note that "Replace the flaky switch" is a one-line task with no file; "Order new switch"
became a block the moment it got a due date, its checkbox became `todo: open`, and its
file starts with a bullet because that is what it is. "Options" is
a section nested under an item, and "Snapshot policy" is a section that is itself a task.

---

## 5. Projection: zoom, render, splice

### 5.1 `render(node, base, resolve_blocks) → String`

Emits the node and its subtree, in document order, as a standalone Markdown document:

1. If the node is a block and `base == 1` and `resolve_blocks = false`: its frontmatter.
2. The node's title line — checkbox included — at level `base` for a section, or at
   indent 0 for an item.
3. The node's body, verbatim (bodies dedented by the node's original indent).
4. Each descendant, recursively: sections re-levelled by `base − level(node)`; everything
   re-indented by `−indent(node)`.
5. Embeds: with `resolve_blocks = false`, written as embeds. With `resolve_blocks = true`, the
   block's subtree is inlined at the embed's position, its frontmatter omitted, its own
   embeds resolved in turn. The resolved text contains no embed, no id, no frontmatter and no
   file boundary: it is titles, checkboxes, bodies and nesting, nothing else.

Properties:

- Levels are unbounded, so `render` is total.
- For a block, `render(node, 1, false)` is byte-identical to its file.
- For a top-level node of `root.md` that is not a block, `render(node, 1, false)` is
  byte-identical to its span.
- Rendering an item root produces a valid document that is a single list.

`resolve_blocks = false` is the on-disk spelling and is used only to write files.
`resolve_blocks = true` is the user-facing spelling: the reading pane and both editors show
it, and it is the input to `splice`.

### 5.2 Splice

Every node belongs to exactly one block: the nearest ancestor-or-self that is one (the root
counts). A block's text is its own title line, body and children down to, but not into,
the blocks nested under it.

`edit(node)` renders `render(node, 1, true)`, the fully resolved subtree, into an editing
buffer in which **every line carries the id of the block that owns it**. The tag is
invisible and travels with the line: a line typed after a tagged line inherits its tag,
a line pasted somewhere takes the tag of the line above it, a deleted line takes its tag
with it. Nothing about ownership is ever re-derived from the text.

A block is **dirty** when any line it owns changed, or when the position of a block nested
in it moved. `splice(block)` writes one dirty block:

1. Collect the block's owned lines in buffer order, and put each nested block back as an
   embed at the position of that block's title line. The result is exactly an edited
   `render(block, 1, false)`: one file's text, embeds intact, ids never typed.
2. Parse it: title lines, bodies, nesting, embeds. Exactly one root-level node; a `---`
   line is body text.
3. Apply the inverse shift: re-level sections by `+ (level(block) − 1)`; re-indent by
   `+ indent(block)`.
4. Canonicalize the touched nodes (§4.2). Frontmatter is not in the text and is copied
   through unchanged; splice never creates, changes or removes a property.
5. Compare the hash of the source span with the hash recorded when the block was last read.
   If it differs, the file changed underneath: merge two-way against what is on disk
   (§12.4), conflicts and all. Never overwrite silently.
6. Replace the span atomically (§11.1). One file.

Splice runs without being asked (§10.6): when the cursor moves from a dirty block into
another block, after a pause in typing, on leaving the editor, before any outline verb,
before a reload, and on quit. A commit may write several files — one per dirty block — but
each block is written by its own splice, and the law holds per block.

Edge cases follow from the tags, not from rules:

- Renaming a nested block is editing its title line; it is that block's line, so its file
  is rewritten and renamed (§6.4). No second step.
- Deleting a nested block's title line deletes the block: the file goes to trash, and any
  lines it still owned are re-tagged to the enclosing block — they become plain text of the
  parent, which is what deleting a heading does to its content in any editor. This is the
  one way a block's contents survive its death, and its properties do not.
- Moving a nested block's title line (cut and paste in the editor) moves its embed; owned
  lines that are no longer contiguous with the title line are re-tagged to the block they
  now sit in.
- Text moved from one block into another changes owner, and both blocks are dirty.

### 5.3 Levels

Heading levels are unbounded (§3.1), so splice never changes a node's spelling and never
demotes anything. The level-overflow rule of earlier drafts is gone.

### 5.4 One function, three uses

| Use | Call |
|---|---|
| Reading pane | `render(cursor, 1, true)` for reading |
| Editing (`e`) | `render(cursor, 1, true)` with per-line owner tags → `splice` per dirty block |
| Make a block (`s`, or the first property set in `a`) | `render(target, 1, false)` to `<prefix>~<name>.md` |

---

## 6. Blocks and files

### 6.1 Making a block

`make_block(node)`:

1. Generate an id and a name (§6.4). The file is `<prefix>~<name>.md` in the vault root.
2. Write `render(node, 1, resolve_blocks = false)` to the file atomically, with frontmatter
   `id: <id>` and, if the block was made by setting a property, that property.
3. Replace the node's span in its parent file with an embed, `![[id]]`, at the node's
   indent. A checkbox on the node becomes `todo: <state>` in the frontmatter (§4.5).
4. Update the index.

There is no inverse. A block stays a file until it is deleted (`d`, *clear done*, or
another tool). To fold a block's text back into its parent by hand, yank its subtree
from the reading pane, paste it at the embed, and delete the block; the app offers no
single verb for this because it would have to throw the frontmatter away.

A block is made when the first property is set on a node in the property editor (§10.6),
or on `s` / *make block*. There is no other way.

### 6.2 Constraints (enforced)

- Every block is embedded from **exactly one** embed. A second embed for the same id is
  a diagnostic and renders as broken. Cycles are a diagnostic.
- A block file has exactly one node at column 0 (§4.9) and its frontmatter has a valid
  `id`.
- Blocks may be nested: a block's file may itself contain embeds.
- Embed lines have no children in the parent file.
- The vault is flat. The app never creates a directory.
- **Every filename the app writes is lowercase** (§6.4), temp and trash names included.
  Titles inside files keep whatever case you typed; paths resolve case-insensitively
  (§3.4), so `Homelab/NAS` and `homelab/nas` are the same target.

### 6.3 What becomes a block, and when

Only two things make a node a block: giving it a property in `a` (§3.2), or asking (`s`). The
vault is
therefore `root.md` plus exactly the blocks that have properties or that you split by
hand. A note about a book has an author, so it's a file; a section of scratch prose has
nothing, so it isn't; a task that needs a completion date becomes a file. For a phone-friendly
inbox, make `Inbox` a block once; it stays one small file.

### 6.4 Ids, names and filenames

**Id**: 64 random bits rendered as four words of two syllables each, using Urbit's `@p`
tables — 256 prefix syllables (consonant-vowel-consonant with vowels `a i o`) and 256 suffix
syllables (vowels `e u y`) — each word `prefix suffix`, words joined by `-`:
`racfer-hattes-dozzod-binwes`. No `~` sigil and no scrambling step, since the bits are random
rather than sequential. Never derived from content or time. Stored as the `id` frontmatter
key and validated against the syllable tables on read; a file whose `id` does not validate
is ignored, and `notes check` says why.

Collisions: 2⁶⁴ values, so two ids minted independently are never expected to collide; the
index still checks and redraws locally, and the merge engine still reports an id collision
as a diagnostic rather than silently merging two files, but neither path should ever run.
Four words is fixed.

Ids are random and never reused.

**Name**: `slug(title)`: Unicode NFC → lowercase → whitespace runs to `-` → remove every
character not in `\p{L}`, `\p{N}`, `-` → collapse repeated `-` → trim `-` → truncate to
80 bytes at a character boundary → `untitled` if empty. Cyrillic titles keep Cyrillic
names; there is no transliteration.

**Filename**: `<prefix>~<name>.md`, where `<prefix>` is a leading run of the id's words —
`racfer~order-new-switch.md` for id `racfer-hattes-mislup-nodrys`. Prefixes are unique
**on their own**: the title plays no part. The run starts at one word and is the shortest
that no other file in the vault already has as its prefix. If `racfer~notes.md` exists
when a block with id `racfer-wolsun-…` is made — whatever its title — the new file is
`racfer-wolsun~order-new-switch.md`; a third id sharing two words gets three. Existing files
are never lengthened for a newcomer, and a prefix is never shortened. The full id is always
a valid prefix, so the loop terminates. The `~` separates prefix from name unambiguously:
words inside the prefix are joined by `-`, and `~` never occurs in a slug, so a filename
splits without knowing how many words the prefix has. Because the prefix alone is unique, a title change
can never collide with anything, and `racfer~` already tells you which file you are looking
at before you read the rest. Cross-device collisions are found at merge time and resolved
the same way (§12.2). The filename carries no identity: a file is parsed because of its
`id` key, not its name, so `python-basics.md` with an `id:` is a block (and `notes
check` asks for the prefix) while `racfer~order-new-switch.md` without one is ignored.
Safe on APFS, ext4, Android storage and Windows; case-insensitive-safe because lowercase.

Renaming a block's title renames the file to the new name (temp-and-rename). Nothing
else in the vault references the filename, so nothing else is rewritten. A file renamed or
titled by another tool is still found by its id; `notes check` reports filenames whose
prefix is not a leading run of the id or whose name no longer matches the title, and
`notes check --fix` fixes them.

### 6.5 Refile

Refile is always a span operation: delete the subtree's span from the source file and insert
the re-levelled text at the destination (§5). Refiling a block moves only its embed line;
the file does not move. Any node may be refiled under any node;
spelling is preserved. A task block's state travels with its file, not its embed.

**Archive.** Archiving is refile with a fixed destination: the subtree is appended as the
last child of the top-level section titled `Archive` (case-insensitive, like `Inbox`;
created as the last top-level section of `root.md` if missing). Nothing else happens — no
path is recreated; if you want structure inside the archive, make it. `Archive` is an
ordinary section: refile things back out, rename it (the next archive creates a fresh one),
delete it. Nothing in the archive is hidden or dimmed.

---

## 7. Capture

- The inbox is the top-level section titled `Inbox` (case-insensitive). If none exists,
  capture creates `# Inbox` as the last top-level section of `root.md`. If several exist,
  the first in `root.md` order wins and `notes check` reports the rest.
- **Capture** (`c` in the TUI, `notes capture`, a global hotkey via the CLI) appends an item
  under today's day: the child section of `Inbox` titled with today's date, `YYYY-MM-DD`,
  created on demand as the last child. `--task` or a leading `[ ]` makes it a task.
  `--to <target>` captures under any node instead. Capture never creates a block; dates
  and properties are set in the TUI afterwards.
- Days are ordinary sections, appended in order, so the inbox reads oldest to newest. They
  can hold prose, tasks and sub-sections like any section, and `2026-09-11` addresses
  one by title (§3.4). Items placed directly under `Inbox` by hand (or by a phone
  editor appending to the file) are fine; they're simply undated.
- The day title is the only creation timestamp an inline node gets; the app tracks nothing
  finer (§12.6).
- `c` in the TUI lands on today; the inbox itself is the refile queue.

---

## 8. Tasks

### 8.1 States

| Node | open | done |
|---|---|---|
| not a block | `- [ ] Title` / `## [ ] Title` | `[x]` |
| block | `todo: open` | `todo: done` + `done: <date>` |

Any node can be a task. A checkbox task has exactly this much state and nothing else. Its
completion date is not recorded; if you need one, make the task a block so toggling
writes `done:` (§8.2).

### 8.2 Task blocks

A task that needs a date, a property, a completion record, or a body with structure becomes
a block (§6.1). Its checkbox becomes `todo:` in the frontmatter; the embed in the parent
is `![[id]]` and the title line in the file is plain. Toggling from anywhere
(outline or embed) rewrites `todo:` and stamps or removes `done:`.
The reading pane renders a resolved task embed on one line:

```
☐ Order new switch            due 2026-09-20
```

Setting a property on an inline task in `a` is how it becomes a block (§6.1); nothing
about that is visible afterwards except the `▤` marker.

### 8.3 Hierarchy semantics

Indentation is ownership. A nested task is a subtask; indented plain lines are notes on the
task. There is **no roll-up**: completing a parent doesn't complete children and vice
versa. Every node shows derived counts (`open/total`) for its subtree; a task section shows
its own state as well. A task block's sub-nodes live in its file, and the parent list
shows only the embed.

### 8.4 Dates

Dates are ISO, `YYYY-MM-DD`. The property editor refuses anything else for `due` and
`done`; other keys take any text.

### 8.5 Finished tasks

Done items stay where they are: one line each, in the context that gave them
meaning, which is the cheapest history there is. The panes dim them, and `zd` toggles
hiding them for the session.

When a list is genuinely finished, *clear done* (palette) trashes every done item with no
open descendants under `target` (default: current zoom root); task blocks'
files go to trash too. For whole finished subtrees — a completed project, a concluded
meeting series — `za` refiles them under `Archive` (§6.5). The trash (§11.5) and file versioning (§12.6) backstop both.

### 8.6 Recurrence

Out of scope for 1.0. Nothing is reserved for it.

---

## 9. Query language

Not in 1.0. Search is the filter box (§10.5): fuzzy title match plus full-text match over
the whole vault, live, no syntax. A query language with saved views was designed and set
aside (§17, open decision 16); the ```` ```query ```` fence is reserved for it (§4.8).

---

## 10. TUI

### 10.1 Layout

```
┌ Homelab › NAS › ZFS layout ────────────────────────── 3/7 open ┐
│ outline pane (titles only)          │ reading pane (zoomed)   │
│ ▾ Homelab              2/9          │ # ZFS layout             │
│   ▾ NAS                             │ since 2024-03 · storage  │
│     ▸ ZFS layout  ▤  1/2 ◀          │                          │
│     Networking       1/2            │ Mirrored pairs, no raidz…│
│ ▾ Inbox                             │                          │
│   2026-09-10        1/2             │ ## Snapshot policy       │
│                                     │ - hourly, keep 24        │
│                                     │ ☑ Move scratch to its…   │
├─────────────────────────────────────┴──────────────────────────┤
│ :                                      root.md · saved · 12:04 │
└────────────────────────────────────────────────────────────────┘
```

- The **outline pane** shows titles, fold state, task glyphs, derived counts, a block
  marker (`▤`) and, for blocks, `due` dimmed. Never bodies.
- The **reading pane** shows `render(cursor, 1, true)` with light Markdown styling; a
  block's properties appear as a dimmed header under its title, computed from the
  index, never as text. `zr` (session toggle) shows the text unstyled.
- The breadcrumb shows ancestors of the zoom root; the status line shows file, save state,
  pending conflicts, and the last message.
- Split layout, outline on the left at a third of the width (minimum 30 columns); `Tab`
  cycles focus. Below 80 columns the panes stack and `Tab` switches between them.

### 10.2 Modes

`normal` (default), `edit` (the built-in editor over a subtree's Markdown, saving as you go), `filter`
(`/` box), `picker` (fuzzy lists, the palette included), `conflict` (§12.5).
`Esc` always returns to normal.

### 10.3 Outline pane — normal mode

| Key | Action |
|---|---|
| `j` / `k` | next / previous visible node |
| `h` | collapse; if collapsed or leaf, go to parent |
| `l` | expand; if expanded, go to first child |
| `H` / `L` | collapse / expand all under cursor |
| `zd` | toggle hiding done nodes (both panes) |
| `zr` | toggle raw mode: exact source in the reading pane, frontmatter included |
| `za` | archive: refile subtree under `Archive` (§6.5) |
| `-` | go to parent |
| `{` / `}` | previous / next sibling |
| `gg` / `G` | first / last visible node |
| `Ctrl-d` / `Ctrl-u` | half-page down / up |
| `Enter` | zoom: reading pane shows the cursor node; reading pane takes focus |
| `Backspace` | zoom out to parent |
| `>` / `<` | demote / promote: become the last child of the previous sibling / the next sibling of the parent. Spelling unchanged |
| `~` | toggle spelling, section ↔ item (subtree unchanged) |
| `J` / `K` | move node down / up among siblings |
| `n` / `N` | new sibling after cursor / new last child: inserts an empty node and opens it with `e` |
| `e` | edit the subtree's Markdown in the built-in editor; saves as you go (§10.6) |
| `a` | property editor: a form over the node's properties (§10.6); first property on a plain node makes it a block |
| `x` | toggle task open / done (checkbox, or `todo:` plus `done:` on a block) |
| `t` | toggle task-ness: adds or removes the checkbox, or the `todo` key on a block |
| `s` | make the node a block |
| `y` / `d` | yank / delete subtree into the register (delete goes to trash too) |
| `p` / `P` | paste register after / before cursor as sibling |
| `r` | refile: fuzzy-pick a destination; `Ctrl-Enter` = as first child |
| `c` / `C` | capture to the inbox as bullet / as task |
| `/` | filter box (§10.5) |
| `:` | command palette (§10.9); `?` help |
| `u` / `U` | undo / redo |
| `q` | quit (nothing is ever unsaved in normal mode) |

### 10.4 Reading pane — normal mode

| Key | Action |
|---|---|
| `j` / `k`, `Ctrl-d` / `Ctrl-u`, `gg` / `G` | move / scroll |
| `Enter` | on a heading: zoom into it (`x` toggles a task heading); on an embed: follow; on a task item: toggle |
| `Backspace` | zoom out |
| `x` | toggle task under cursor |
| `e` / `a` | edit the node under cursor: its subtree's Markdown, or its properties, as in the outline |
| `o` | open external link under cursor (`xdg-open` / `open`) |
| `[[` / `]]` | previous / next heading |
| `/` | in-block search; `n` / `N` next / previous match |

### 10.5 Filter box

`/` opens a single input over the outline. Typing filters the outline live by fuzzy title
match (nucleo) and, after a 150 ms pause, by full-text match, showing ancestors of hits.
`Enter` on a highlighted hit zooms to it; `Enter` with no hit does nothing. `Esc` clears.
Creating nodes is `n` / `N` (§10.3).

### 10.6 Editing

There is one way to change text: edit the Markdown of the selected subtree. `e` shows
`render(cursor, 1, true)` — the node's title line, body and children, with every
nested block inlined so that no embed, id, frontmatter or file boundary appears.

**Built-in** (`e`): a plain multi-line text box over that text, in the reading pane. There
is nothing to commit and nothing is locked. Each line carries its owning block invisibly
(§5.2); you type anywhere, including across what happens to be a file boundary, and the
app keeps track. Blocks are saved on their own:

- when the cursor moves out of a block that has changed;
- after 750 ms without a keystroke;
- on `Esc` (back to normal mode), on any outline verb, before a reload caused by an
  external change, and on quit.

The status line shows the title of the block the cursor is in, dimmed, and a dot while
something is unsaved — the only two hints that blocks exist. `Ctrl-c` discards changes made
since the last save. Undo inside the editor is the editor's own; each automatic save is one
entry in the session op log (§10.11). The box is not a Markdown editor and stays one.

Properties are not text. `a` opens the **property editor**: a small form listing the
node's keys and values, where you add, change and delete entries. `due` and `done` accept
only ISO dates; `id` is not shown. Setting the first property on a node that is not a
block makes it one (§6.1), silently except for the `▤` marker; deleting the last one leaves
the block as it is. Frontmatter lines the app does not understand (nested structure,
comments) are listed read-only and preserved.

`n` / `N` insert an empty node and put the cursor on it in the editor, so creating and
editing are the same gesture. `x`, `t`, `~`, `>`/`<`, `J`/`K`, `r` and `d` are the
structural verbs; they exist because toggling a checkbox should not require the editor,
and each saves the editor first.

### 10.7 (removed)

Result views went with the query language (§9).

### 10.8 Conflict view

Entered when a `.sync-conflict-*` file is detected or a splice hits a changed span. Shows
each conflict pair (ours in place, the `conflict:` block right after it) side by side; `o` keeps ours,
`t` keeps theirs, `b` keeps both, `e` edits, `n` / `N` next / previous, `Enter` finishes.
Unresolved pairs remain as `conflict:` blocks and stay listed in the status
line until resolved (§12.5).

### 10.9 Command palette

`:` opens a popup listing every action the TUI has, filtered live by fuzzy match (nucleo)
on the action's name and description, each row showing its key binding if it has one.
`Enter` runs the highlighted action; if it needs an argument — a target for *refile* or
*go to*, text for *capture* — a single prompt follows, with the same completion the
key-bound form has. `Esc` closes.

There is no command syntax: nothing is typed except the search and the argument. Every
action reachable by key is in the palette under a readable name (*make block*,
*toggle task*, *archive*, …), and the four with no key live only there: *clear done*,
*canonicalize*, *check*, *merge*. `?` is the same popup filtered to show keys, so help and
palette are one thing.

### 10.10 Markdown styling in the reading pane

Line-based, not a full renderer: headings by level, task glyphs (`☐ ☑`), resolved blocks'
properties dimmed at the end of the line, underlined external links, styled code fences,
`**bold**` / `*em*` / `` `code` `` inline, blockquote bars. Raw text is never hidden;
`zr` shows exact source, frontmatter included.

### 10.11 Undo, redo, saving

Every mutation is an operation with an inverse (span edits, file create/move/delete,
including making blocks). The session op log powers `u` / `U`. Outline verbs write
immediately and atomically; the built-in editor writes each dirty block within 750 ms of the
last keystroke, or sooner when the cursor leaves it (§10.6), so the only unsaved text at any
moment is under a second of typing in one block. Undo after an external change re-checks
span hashes and refuses with a message if the target moved.

---

## 11. Storage, I/O and safety

### 11.1 Writes

- **Atomic**: write to `.<name>.notes-tmp` in the same directory, `fsync`, `rename`.
  Never truncate in place.
- **Span-preserving**: an edit replaces exactly the bytes of the affected node(s) — for a
  property change, exactly the frontmatter lines that changed. Every other byte is copied
  through unchanged. After a write the file is re-parsed.
- **mtime**: files that are not written keep their mtime. Untouched nodes are never
  re-canonicalized, so a no-op session leaves no trace on disk.
- Line endings and a trailing newline are normalized only within the rewritten span.

### 11.2 Watcher and reload

`notify`-based recursive watcher on the vault, 200 ms debounce, ignoring the patterns in
§11.4.

- A changed file that the user is not editing is re-parsed; the cursor is re-attached by
  id, then path, then nearest surviving ancestor.
- A changed file with a built-in edit in progress: the editor saves its dirty blocks first;
  a block whose span hash no longer matches is merged two-way (§5.2 step 5), then the file
  is re-parsed and the buffer re-rendered around the cursor.
- A new `*.sync-conflict-*.md` file starts the merge flow (§12).
- A deleted file that was a block marks its embed broken; nothing is written.
- Editing the same vault in Helix at the same time is a supported workflow.

### 11.3 Index

The index — ids, names, paths, embeds, task table, property table, spans, per-file hashes —
lives in memory and is rebuilt from the vault on every start by parsing every `.md` file.
Nothing is persisted: no cache directory, no shadow copies, no state that could go stale or
disagree with the files. A file is re-indexed whenever the watcher reports it changed.
Startup cost is the parse (§15.5), which is why the parser has a budget.

Nothing under the vault directory is written by the app except Markdown and temp files
during atomic writes.

### 11.4 Ignore patterns

Never parsed, never written, never deleted: dotfiles and dot-directories (`.git`, `.jj`,
`.stfolder`, `.stversions`, `.stignore`, `.obsidian`), `.syncthing.*.tmp`, `*.notes-tmp`,
`*.tmp`, and any file not ending in `.md`. `*.sync-conflict-*.md` is parsed only by the merge
engine.

### 11.5 Trash

Deleted subtrees and resolved `conflict:` blocks are written to
`$XDG_STATE_HOME/notes/trash/<timestamp>-<id-or-name>.md` (device-local, never synced) before
removal. `notes trash list|restore` manages it. The app never deletes user content without
a trash copy.

---

## 12. Sync and conflict merge

### 12.1 Working assumptions about Syncthing

- Sync is per file, block-level, and conflict-unaware: concurrent edits to one file produce a
  winner plus a `name.sync-conflict-<date>-<time>-<device>.md` copy of the loser.
- Therefore: keep concurrently-edited things in different files where possible (a
  `Inbox` as a block; blocks), and make the app the thing that understands conflict files.
- The vault's `.stignore` should contain at minimum: `.git`, `.jj`, `*.notes-tmp`, `*.tmp`.

### 12.2 Detection

The watcher (or `notes merge`, or the startup scan) finds `X.sync-conflict-*.md` next to
`X.md`. Several conflict files for the same `X.md` are merged oldest first.

Before merging, the engine compares the two files' `id` keys. If they differ, this is not a
conflict but a **prefix collision** — two blocks that happened to get the same filename
on different devices — and the conflict file is simply renamed with a longer prefix of its
own id, per §6.4, until its prefix is unique. The same happens, without a sync conflict, when
a reload finds two blocks sharing a prefix. Both embeds resolve by id; nothing is merged
and nothing is lost.

### 12.3 Merge inputs

Ours `O` is `X.md`; theirs `T` is the conflict file — or, for an in-session conflict
(§5.2 step 5), the text the user just edited. There is no stored common ancestor (§11.3),
so every merge is **two-way**: the engine can tell *that* two versions differ, not *which*
side changed. One algorithm, one code path.

### 12.4 Node-level merge

Both versions are parsed into trees (embeds unresolved; each file merges on its own).

**Matching** nodes across versions, recursively from the root:
1. by block id for embeds (exact);
2. else by exact title among unmatched siblings under the matched parent;
3. anything left is an insertion.

A retitled node therefore appears twice after a merge — once per title. Two-way merging
never deletes, so this is the safe failure; delete the copy you don't want.

**Per matched node**, for each of title, checkbox state, body (as text), and — for the
file's block — each frontmatter key independently (`todo` included): `O == T → take O`,
otherwise **conflict**. A conflict pair is raised for every field that differs, even when
only one side touched it; the cost of having no per-device state is paid here, in
resolution clicks, never in lost text.

**Children** are merged as sequences of matched nodes: nodes present on one side only are
insertions, placed relative to their matched neighbours. Nothing is ever treated as
deleted.

**Conflict output**: the `O` version stays in place. The `T` version is written as a new
block `<prefix>~<name>.md` (a fresh id; the name is `O`'s name) with frontmatter
`conflict: "<device> <timestamp>"` (plus, for a block conflict, `T`'s own properties),
and an embed to it is inserted as the next sibling of `O`. The conflict block is
therefore always the node immediately after the one it conflicts with; no other link
between them is stored. No text is ever lost: every node from `O` and `T` appears in the
result exactly once.

The result is written atomically to `X.md` and the conflict file goes to trash. The
algorithm is deterministic and idempotent.

### 12.5 Resolution

The conflict view (§10.8) lists every `conflict:` block, each shown against the sibling
before it, and the status line counts them. Resolving a pair:

- **keep ours** — delete the conflict block and its embed (to trash);
- **keep theirs** — replace ours' title, body and children with the conflict block's,
  and its frontmatter with the conflict block's minus `id` and `conflict`; then delete
  the conflict block and its embed (to trash);
- **keep both** — drop the `conflict` key; the block stays where it is as an ordinary
  sibling.

Unresolved pairs are ordinary blocks; they sync to every device and are visible in any
editor, and can be lived with indefinitely.

### 12.6 History (outside the app)

The app keeps no history: per-node timestamps, "last modified", and completion dates of
inline tasks are simply not recorded anywhere (§3.5, §7, §8.1). If you want history, turn
on staggered file versioning on an always-on Syncthing node — the app neither needs it nor
notices it.

---

## 13. CLI

All commands take `--vault PATH` (default: `$NOTES_VAULT`, else the nearest ancestor of
`$PWD` containing `root.md`, else `~/notes`).

| Command | Purpose |
|---|---|
| `notes` | open the TUI; creates `root.md` if the directory is empty (§4.1.1) |
| `notes capture [TEXT] [--to TARGET] [--task]` | append to the inbox (stdin if no TEXT) |
| `notes check [--fix]` | diagnostics with source spans (§15.7); `--fix` rewrites the vault in canonical form (§4.2) and repairs filenames (§6.4) |
| `notes merge [--dry-run]` | process sync-conflict files non-interactively; list leftovers |
| `notes trash list \| restore ID` | trash management |

That is the whole CLI: the entry points a shell, a hotkey, a cron job on the homelab, or a
recovery session needs. Everything else is a TUI verb.

---

## 14. Settings

There are no vault settings. Everything that would have been one is either fixed behaviour
or content: the inbox is the section titled `Inbox` (§7);
filenames are an id prefix plus the title; names keep Unicode; ids are four words in
frontmatter (§6.4); making blocks
are property-triggered or manual only; `done:` records a date; conflict copies are
blocks marked `conflict:`. Each was a knob nobody needs to agree on with a file. Two vaults that
contain the same Markdown behave identically.

There are no device settings either. The app reads no file outside the vault. What used to
be configuration is now:

- the vault: `--vault PATH`, else `$NOTES_VAULT`, else the nearest ancestor of `$PWD`
  containing `root.md`, else `~/notes`;
- the trash location: `$XDG_STATE_HOME/notes/trash/` (§11.5);
- everything visual: fixed (§10.1) or a session toggle (`zd`, `zr`).

The only thing the app keeps outside the vault is the trash.

---

## 15. Architecture (Rust)

### 15.1 Workspace

```
crates/
  notes-core/    parsing, tree, render/splice, index, store, merge — no TUI deps
  notes-tui/     ratatui application
  notes-cli/     clap binary; depends on both
```

`notes-core` is the stable API a future Android client or Helix extension would use.

### 15.2 Core types (sketch)

```rust
pub struct NodeId(u32);                         // arena index, session-stable
pub enum Kind { Root, Section, Item }           // Section / Item are spellings (§3.1)
pub enum TaskState { Open, Done }

pub struct Node {
    kind: Kind,
    title: String,
    task: Option<TaskState>,                    // any node; from the checkbox, or `todo` for a block
    body: Vec<String>,                          // raw lines, opaque
    children: Vec<NodeId>,
    parent: Option<NodeId>,
    file: FileId,
    span: Span,                                 // bytes in `file`: title line through subtree
    doc: Option<Block>,                      // Some if this node is the root of a file
}

pub struct Block {
    id: Option<Id>,                             // None only for root.md
    name: String,                               // slug of the title at last rename
    path: RelPath,                              // e.g. "racfer~order-new-switch.md"
    props: IndexMap<String, String>,            // top-level `key: value` lines, order-preserving; includes `id`
    frontmatter_raw: String,                    // verbatim, for lossless rewrite
    edge_span: Span,                            // the embed line in the parent file
}

pub struct Span { start: usize, end: usize }

pub struct Tree { nodes: Vec<Node>, root: NodeId, files: Vec<FileState> }
pub struct Cursor { path: Vec<NodeId> }         // zipper-style focus; all outline verbs are cursor ops
```

Levels and indents are computed from ancestry, never stored.

### 15.3 Parser strategy

Hand-written, line-oriented, fence-aware. The structural grammar (§4.3) is regular per line
(headings at any indent and any depth are title lines);
bodies are opaque; frontmatter is a delimited block at byte 0 scanned line by line for
top-level `key: value` pairs — not a YAML parser, since nothing is typed and unknown lines
are never interpreted. This is deliberately not a CommonMark parser: the app needs
exact spans, lossless round-tripping, and a small canonical subset — not a full AST.
`pulldown-cmark` may be used later for inline styling in the reading pane only.

Reading tolerance (setext headings, `*` bullets, tabs, `[X]`, `[-]`) is handled in the lexer and
recorded per node as "non-canonical", which `notes check` reports and touching rewrites.

### 15.4 Dependencies

`ratatui`, `crossterm`, `tui-textarea` (built-in editor), `notify` (watcher), `jiff`
(today's date), `clap`, `indexmap`, `nucleo` (fuzzy
filter), `similar` (sequence alignment and diff3 for merge), `blake3`, `tempfile`,
`unicode-normalization`, `unicode-width`, `directories` (XDG),
`getrandom` (64-bit ids; the `@p` syllable tables are a 512-entry constant in `notes-core`),
`ariadne` (diagnostics), `proptest` + `insta` (tests).

### 15.5 Performance targets

| Operation | Target |
|---|---|
| Parse a 5 MB `root.md` (~20k nodes) | < 50 ms |
| Re-parse after a single-node write | < 10 ms |
| Index build, 20k nodes, 2k blocks | < 200 ms |
| Fuzzy filter keystroke, 20k titles | < 5 ms |
| Filter box, 20k nodes, per keystroke | < 20 ms |
| Frame render | < 8 ms |
| Startup, 5 MB vault, no cache | < 300 ms |

### 15.6 Testing

Property-based, since the whole design rests on a few laws:

- `parse(render(t, 1, false))` equals `t` re-levelled, for all generated trees `t`.
- `render(parse(s), 1, false) == s` for all canonical blocks `s`, frontmatter included.
- `canonicalize` is idempotent.
- `splice(node, render(node, 1, true))` is a no-op on disk, for any node.
- Every splice writes exactly one file; a save writes one splice per dirty block.
- `render(make_block(n), 1, false)` equals `render(n, 1, false)` plus the generated frontmatter,
  and `set key value` on any node yields a block whose file round-trips.
- Merge: `merge(O, O) == O`; `merge(O, T)` contains every node of `O` and `T` exactly
  once; merging is idempotent on its own output.

Plus `insta` snapshot tests for the TUI via `ratatui::backend::TestBackend`, and a corpus
of real-world Markdown (Obsidian, Logseq, FSNotes exports) that must parse without loss.

### 15.7 Diagnostics

`notes check` reports, with `ariadne`-rendered source spans: non-canonical syntax, empty
titles, titles containing `/`, broken, duplicate or cyclic embeds, embeds with children in the
parent file, malformed block files, invalid or duplicate `id` keys, ignored `.md` files,
filenames whose prefix or name no longer match their id or title, frontmatter outside
byte 0, `due` or `done` values that are not ISO dates, and unresolved conflict blocks.
The TUI
surfaces the same diagnostics in the status line for the current file.

---

## 16. Interoperability

The vault is Markdown-shaped but not Markdown; §4.2 lists the four things a standard renderer
gets wrong. What other tools see:

| Tool | What it sees | Notes |
|---|---|---|
| `rg`, `fd`, Helix | plain text | `rg '\[ \]'` and `rg '^todo: open'` together are the open task list; `rg '^done: 2026-09'` the blocks finished this month; `rg '^due:'` the dated ones |
| Any phone editor | plain text | capture by appending a `- ` line to the inbox block's file; the app adds nothing to it |
| Obsidian, Logseq, FSNotes | mostly readable, structurally wrong | frontmatter as properties and `- [ ]` bullets render; `![[id]]` shows as a broken embed, `## [ ]` as a literal heading, `#######` as a paragraph, indented headings as code. Fine for reading a file; do not edit structure there |

---

## 17. Non-goals for 1.0

Encryption; CRDT / operation-log sync; a mobile client (the file format is its contract);
a query language, saved views and `--json` output (the ```` ```query ```` fence is reserved); attachment management beyond ignoring `assets/`; full Markdown
rendering or WYSIWYG; plugins; task recurrence; spaced repetition; whiteboards; multiple
vaults open at once; general transclusion; an external `$EDITOR` handoff (designed:
resolved subtree with frontmatter inline under each block's title, ids included, replaced
wholesale on exit); properties on non-blocks; export or lowering
to standard Markdown (the four non-standard forms of §4.2 are the whole gap); a cancelled
task state; wiki-links between nodes and backlinks
(`[[…]]` is reserved, §4.6); rendering the format correctly in third-party editors.

---

## 18. Milestones

| # | Deliverable | Exit criterion |
|---|---|---|
| M0 | `notes-core`: parser incl. frontmatter, tree, render/splice, canonicalize, ids and names | round-trip laws pass under proptest; `notes check` works |
| M1 | TUI: outline + reading panes, zoom, fold, move, promote/demote, refile, filter box, subtree editing (`e`), undo | daily-drivable on a single `root.md` |
| M2 | Blocks and properties: `s`, property-triggered blocks, task blocks | a task can be given a due date in the editor and its file round-trips |
| M3 | Tasks: toggling on any node, `todo:` and `done:` stamping, dates, clear-done, filter box | replaces a TaskPaper file |
| M4 | Inbox and capture; watcher and reload | safe to edit in Helix and the TUI at once |
| M5 | Sync: conflict detection, two-way node-level merge, `conflict:` blocks, conflict view, `notes merge` | phone + laptop + homelab over Syncthing for two weeks without data loss |
| M6 | CLI polish, diagnostics, docs | 1.0 |

---

## 19. Open decisions

1. **Name.** Binary `notes` is a placeholder.
2. **Dated inbox, ISO titles, fixed.** Capture creates `## YYYY-MM-DD` under `Inbox`. There
   is no format choice and no way to turn it off except `--to`. Days append oldest-first; a
   long-lived inbox is a long section, which is what `s` (make block) and *clear done* are for.
3. **Filenames follow titles, always.** A title change is a file rename (Syncthing handles
   renames by block reuse, so cheap but visible) and nothing else, since the id lives in the
   file. The id prefix stays in front through renames, so it is the one part of a filename
   that never moves.
3b. **Id format.** Four-word `@p` names (64 bits). Two words (32 bits) were enough for
   identity but left the one-word filename prefix with only 16 bits of room to grow; four
   words make the id collision-free in practice and give the prefix three words of slack.
   Chosen for being pronounceable, typeable from memory, and self-validating against the
   syllable tables; nobody is expected to type all four.
4. **Task sections.** Any node can be a task, so a project heading can be checked off. The
   derived count is still shown beside it; there is no roll-up in either direction (§8.3).
4b. **Two spellings of task state.** A checkbox for nodes that have no file, `todo:` for
   nodes that do. One mechanism was considered twice: all-frontmatter would make every task
   a file, all-checkbox puts structure on a title line the frontmatter already describes.
   The split costs one conversion when a block is made and nothing else.
5. **Two task states.** Open and done. Cancelled was removed: it was a third value in every
   table, a second date key, and a key binding, for a distinction between "done" and
   "won't do" that a deleted line or a done line expresses well enough. `[-]` is read as
   done for old vaults.
6. **No un-block.** Earlier drafts had an inverse of making a block that dropped the frontmatter. Removed:
   a verb whose defining feature is discarding data is a footgun, and the manual route
   (yank, paste, delete) makes the loss explicit. Revisit only if vaults fill with
   blocks nobody wanted.
7. **No export.** `notes render` and the `--plain` lowering were removed for 1.0. The
   resolved render is now only what the reading pane reads; a standalone-Markdown exporter
   is a later feature with a known scope (§4.2's four forms).
8. **No fuzzy matching in merge.** A node retitled on one device and edited on another
   comes out of a merge as two nodes. The fuzzy title match that would have paired them
   needed a tunable threshold and could pair the wrong nodes; a duplicate is cheaper to
   fix than a wrong pairing, so the step was removed.
8b. **Two-way merge, no cache.** Shadow copies would have made sync merges three-way and
   resolved most differences automatically; they were the only per-device state and were
   removed with it (§11.3). Sync conflicts now produce a conflict pair for every differing
   field. The mitigation is structural: keep concurrently-edited things in separate files
   (§12.1), so conflicts are small and rare. If they turn out to be neither, a per-device
   base is the thing to bring back.
9. **Text is edited as text, properties as a form.** Earlier drafts had a title editor and
   a block editor beside an external-editor handoff; those collapsed into one subtree edit,
   and the external handoff was then dropped from 1.0 (§17). Properties briefly appeared in
   the text as `---` blocks and were taken out again: frontmatter is a storage format, and
   showing it would surface files. The property editor is the one non-text edit. The
   built-in editor is a text box; if it wants to become a Markdown editor, stop.
10. **Inbox by title.** Capture finds the inbox by the title `Inbox`. Renaming it means the
    next capture creates a new one. A hidden marker property was the alternative and would
    have been the last setting.
10b. **No config file.** Seven presentation keys, each already available from the environment
    or replaceable by a key binding, were not worth a file format, a lookup order, and a
    `notes config` command. If a real per-device need appears (a colour-blind palette, say)
    it can come back as one environment variable.
11. **Windows.** Slugs and atomic writes are Windows-safe by design; nothing else is tested.
12. **No links in 1.0.** Storing links by id, displaying the live title, and rewriting labels
    on rename were all designed and then cut for the first implementation. Embeds already
    prove the id machinery; links can return on top of it without changing the file format,
    which is why `[[…]]` is reserved rather than free.
13. **No per-node timestamps.** Title-line annotations (`@done(…)`, `@created(…)`) were
    tried in 0.15 and removed: one metadata mechanism is worth more than inline completion
    dates. An inline task that needs a date becomes a block; the day section is the
    only creation stamp anything else gets.
13b. **Flat vault.** `--as dir/` was removed; the app never creates a directory and ignores
    any it finds. Hierarchy is embeds, and one place for files keeps prefixes unique
    vault-wide with no per-directory rule.
14. **Prefix collisions.** Two blocks whose ids share a first word are `racfer~notes.md`
    and `racfer-wolsun~order-new-switch.md`, regardless of their titles; the prefix grows only
    as far as needed and never shrinks (§6.4). Uniqueness is judged on the prefix alone, not
    the whole filename, so the prefix is a complete address and titles can change freely.
    The cost is that roughly one file in 256 needs a second word from day one; that was
    judged cheaper than a rule that depends on titles. The full id was rejected as too long
    for a name, the bare slug as too collision-prone across devices, and a numeric `-2` as
    meaningless.
17. **Ownership travels with lines.** Three other models were tried: distributing a
    multi-file edit back by re-matching nodes (needs heuristics and a prompt when they
    fail), showing nested blocks as placeholders (leaks files into the text), and locking
    every block but one (a seam the user has to work around). Tagging each buffer line with
    its block and saving blocks as the cursor leaves them needs no matching, shows no seam,
    and still writes each file from its own text. An external editor cannot carry tags;
    the design for one (frontmatter inline, ids included, subtree replaced on exit) is
    parked in §17.
16. **No query language in 1.0.** Several syntaxes were sketched — boolean with `@key`, a
    search box with `is:` and `key:value`, and "everything is a key" (`todo:open
    due:..today`, text is literal, `-` negates, `..` ranges). The last is the likely
    shape when it returns. Until then the filter box is the only search and the `query`
    fence is reserved.
15. **Third-party editors are readers, not editors.** Once the format owns syntax, Obsidian
    and friends stop being a safe place to restructure a vault. Editing a body paragraph
    there is still fine; moving headings is not. This is the price of §1 principle 2 and is
    accepted.
