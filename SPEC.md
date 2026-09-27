# SPEC — a tree-shaped plain-text notes and task manager for the terminal

Status: draft 0.40 · 2026-09-26
Working name: not chosen yet. This document uses `notes` as the binary name; rename freely.
Language: Rust · TUI: ratatui · Sync: Syncthing · History: file versioning on one node

---

## 1. Purpose and principles

A mouse-first TUI — every action is a click, a drag or the wheel away, and every one also
has a key — for a single body of notes stored as Markdown. There is one organizing
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
| Node | One thing in the outline: a title line and its children. Spelled as a section or an item. |
| Children | A node's ordered content: text children and child nodes, interleaved (§3.1). |
| Text child | A run of lines among a node's children that is not a node: prose, code, a quote (§3.3). |
| Section | A node spelled as an ATX heading `#`, `##`, … (no upper bound). |
| Item | A node spelled as a `- ` bullet. |
| Spelling | Whether a node is written as a heading or a bullet. Presentation only (§3.1). |
| Task | A node with a checkbox after its marker — block or not. |
| Block | A node with its own identity and file. Only blocks have properties; every edit changes exactly one block. |
| Id | A random four-word phonemic name (`racfer-hattes-dozzod-binwes`) in a block's `id:` frontmatter; its stable identity. |
| Name | The slug of a block's title. With a prefix of the id in front, its filename. Decoration, not identity. |
| Property | A key in a block's YAML frontmatter. |
| Body | A node's leading text children: what comes before its first child node. |
| Zoom | Viewing a node as a standalone block via `render(node, base)`. |
| Splice | Writing an edited zoomed block back into its source span. |
| Make a block | Giving a node an id and its own file, leaving an embed in the parent. |
| Embed | `![[id]]` alone on a line (or as a heading, `## ![[id]]`) at the node's position in the parent. |
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

A node's **children** are an ordered list in which two things interleave: **child nodes**
and **text children** (§3.3), in the order the file has them:

```
Trip                       ← section
├─ text   "Plan below."
├─ item   book flights
├─ item   book hotel
└─ text   "Budget is tight, so check both before paying."
```

Rules:

- Any node may be the child of any node. A heading under a bullet is a section whose parent
  is an item; a bullet under a heading is an item whose parent is a section.
- **Ordering rule.** Every node's children match

  ```
  children := (text | item)* section*
  ```

  Markdown has no way to close a heading: anything written after a section child — text or
  an item at that position — belongs to that section. So once a section child appears, only
  section children follow it. The parser produces nothing else; every verb that places a
  node (§6.5, §10.3, §12.4) keeps the rule by **clamping**: an item goes no later than just
  before its new parent's first section child, and a section no earlier than just after its
  last item child. The *boundary* of a node's children is the position between its last
  item or text child and its first section child.

  **What the rule costs.** A node cannot have items or text after a section child: under
  `# Trip` with a `## Budget` subsection, there is no way to put another bullet or paragraph
  back under Trip after Budget. Everything written there is Budget's. Plain Markdown cannot
  express it either, so the format accepts the limit rather than invent a closing syntax
  no other tool would read. The practical consequences are that `>`, `<`, paste, refile and
  capture sometimes place an item earlier than asked (before the first section), `J` / `K`
  refuse to move an item past a section, and `~` moves the node it respells. The TUI says
  so in the status line whenever the rule, not the user, chose the position; none of these
  is a bug. The workaround is the obvious one:
  put the later items in a section of their own (`## Also`), or keep sections last.
- **Text children are content, not outline.** They are not rows in the outline pane, not
  targets, never blocks or tasks, and are never moved on their own. A verb that moves,
  deletes or refiles a node moves exactly that node's own lines; the text children around it
  stay where they are, with the siblings they were between. They move only with the node
  that contains them.
- Heading level and indent are **derived from position**, never stored:
  `level(n) = 1 + number of section ancestors of n`, and `indent(n) = 2 × number of item
  ancestors of n`. A section under an item is written at that item's child indent with its
  own level. There is no upper bound on levels.
- Spelling is presentation: `~` toggles it (§10.3) and nothing else about the node changes.
  Both spellings can be tasks, hold text, be refiled anywhere, and be made into blocks. The
  one consequence of spelling is position: under the ordering rule a node respelled by `~`
  moves to its parent's boundary, so its siblings keep their parent.
- Sibling order is block order and is meaningful.
- Any node may be a **block** (the root of its own file). Its file starts with the node
  as it is spelled — a heading or a bullet — and that is where its spelling is defined
  (§4.9). The embed repeats it in its form (§4.7) so the parent file parses into the right
  shape on its own; the app keeps the two in step (`~` rewrites both), and where they
  disagree the file wins and `notes check` reports the embed.

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
| `due` | `YYYY-MM-DD` | Task due date; shown in the outline. |
| `done` | `YYYY-MM-DD` | Written when a task block is checked; removed when unchecked. |
| `conflict` | `<device> <timestamp>` | Marks a "theirs" copy written by the merge engine (§12.4). |

There are no setting keys. Nothing in any frontmatter changes how the app behaves.

New children are always appended after the last existing child; sibling order is manual and
nothing reorders it on insert.

Everything else (`tags`, `priority`, `est`, `waiting`, `since`, `author`, `source`, …) is
user-defined, and the app never looks at it.

### 3.3 Text children

A text child is a run of consecutive raw lines, opaque to the tree, between two structural
points: the title line, a child node, or the end of the node. The parser only needs to know
enough to find the *next title line*: it tracks fenced code blocks (```` ``` ```` and `~~~`,
any indent) so that `#` or `- ` inside a fence never starts a node. Paragraphs, quotes,
tables, images, HTML and code are all text.

A text line belongs to the deepest open node whose region it reaches:

- an item's region is lines indented at least `indent + 2`;
- a section's region is lines at or beyond its own indent;
- the root's region is everything.

So a line at column 0 after `- b` under `# Trip` is Trip's text, not b's: it ends the list.
(CommonMark would read an unindented line right after a list item with no blank line between
as a lazy continuation of that item; this format does not. Canonical form therefore always
writes a blank line there (§4.2), and then both agree.)

A blank line belongs to the text run it is in, or — between nodes — to the deepest open node
before it: the blank line after `- a` is `a`'s trailing separator. Blank lines are therefore
never lost or invented when a node is rendered or moved.

The **body** of a node is its leading text: the text children before its first child node.
It is what the reading pane shows under the title before any child, and what "a node with a
body" means elsewhere in this spec.

### 3.4 Identity and addressing

A block's identity is its **id**: a four-word phonemic name such as
`racfer-hattes-dozzod-binwes`, 64 random bits encoded with Urbit's `@p` syllable tables
(§6.4), generated from OS randomness
when the block is created and stored as the `id` key of its frontmatter. Its filename is
`<prefix>~<name>.md`: the shortest leading run of the id's words that no other file in the
directory uses as its prefix — usually one — then the slug of its title
(`racfer~order-new-switch.md`). The prefix alone identifies the file; the name is for humans. The filename is decoration that any tool may change and the
app will repair (§6.4). The one exception is `root.md`, which has no
id. A non-block node has no stored identity. Two things stand in for one, and they are
different on purpose:

- **Typed targets** (below) name a node by titles. Titles are what a person can type, and a
  title that matches twice is an ambiguity error, never a guess.
- **Tracking** — re-attaching the cursor, folds and a verb's result across a re-parse —
  uses the node's **key**: the block whose file holds it (none for `root.md`), then one step
  per level down from that file's root, each step the title *and the node's ordinal among
  same-titled siblings*. Two siblings with one title, or two empty new nodes, have
  different keys. Keys are never shown and never typed.

Verbs never find a node again by its title path: a verb that re-parses finds its result by
key, by id, or by position.

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

Renaming a block's title does not rename its file (§6.4); a stale name is harmless, since
nothing references the filename.

Titles may not contain `/` (the path separator). `[` and `]` are discouraged.

### 3.5 Derived facts

Computed by the index, never stored in files: heading level, indent, path, file, span,
open/done task counts per subtree (`3/7` in the outline; a conflict copy's tasks, §12.4,
are not counted above it), per-file mtime. Per-node timestamps are not tracked by the app
(§12.6).

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
- A blank line before any text child that follows a child node (§3.3), so that CommonMark
  readers agree the text is not part of that node.

Canonicalization is **lazy**: a node is rewritten in canonical form only when it is touched
(edited, spliced, moved). `notes check --fix` (or *canonicalize* in the palette) rewrites
everything at once.

**What is ours.** A standard Markdown renderer will mishandle exactly these four things:
`![[id]]` embeds (bare or as a heading), heading levels beyond six, checkboxes on headings,
and headings indented under bullets. Nothing else in a vault is non-standard.

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

Frontmatter never appears as text to the user.

### 4.5 Tasks

Any node can be a task, and there is one way to write it: a **checkbox after the marker on
its title line**. `- [ ]` and `## [ ]` are both open tasks; `[x]` is done. A block is no
different — its checkbox is on its own title line, in its file:

```markdown
---
id: racfer-hattes-mislup-nodrys
due: 2026-09-20
---

- [ ] Order new switch
```

What a block adds is a completion record: checking it writes `done: <date>` to its
frontmatter, and unchecking removes the date. A plain task has no frontmatter and so no
date; that is the whole difference. Because the state is always on the title line,
`rg '\[ \]'` over the vault is the complete open-task list.

There is no cancelled state. A task that will never be done is either checked off or
deleted.

Embeds carry no state: the checkbox travels with the title line into the block's file when
it is made (§6.1). A task section has its own state *and* the derived count of its subtree.
See §8.

### 4.6 Links

There are no links between nodes in 1.0. `[[target]]` and `[[target|label]]` are opaque
text wherever they appear: not parsed, not resolved, not rewritten on rename, not reported.
The syntax is **reserved** for a later version; write it by hand if another tool needs it.

External links are ordinary Markdown `[text](url)` and are opened with `xdg-open` / `open`.

### 4.7 Embeds

An embed is where a block is stitched into the tree. It has two forms, one per position in
the ordering rule (§3.1):

- **bare**, `![[id]]` alone on a line at the node's indent — an item position;
- **heading**, `## ![[id]]` — a heading line holding nothing but the embed, at the level and
  indent a section has at that position.

```
![[dozzod-binwes-talsun-worbec]]
  ![[racfer-hattes-mislup-nodrys]]     -- a child of the item above it
## ![[lacnum-walbyn-dirlyn-havtyp]]    -- a section-position embed
```

The app writes the form that matches the block's spelling: a block whose root is a section
is embedded with a heading embed, a block whose root is an item with a bare one. The
heading form exists because a bare embed after a section sibling would, like any line
there, belong to that section (§3.1). Either form is read anywhere; an embed whose form
does not match its block's spelling is a diagnostic (§15.7) that `notes check --fix`
rewrites, and it is placed where its form puts it.

The embed names the block by id and nothing else; title, checkbox, properties, text and
children all live in the file. The embed's form repeats the block's spelling so that the
parent file parses into the right shape without opening the block. An embed line has no
children in the parent file; lines indented under a bare embed, or nested under a heading
embed, are a diagnostic.

A heading embed's **level** is read like any heading's: honoured as written, and it
decides the embed's parent the same way. The level its position gives is `1 +` its section
ancestors (§3.1), and that is the level the app writes. A written level deeper than that —
left by another editor, or by moving text around by hand — still parses under the same
parent (a shallower one would parse elsewhere and is simply a different position), so
`notes check` reports it and `check --fix` rewrites it to the positional level; the
structure does not change.

`![[` anywhere else is body text. The app does not implement general transclusion.

### 4.8 Query blocks (reserved)

A fenced block with info string `query` is body text in 1.0. The info string is reserved
for a later query language (§17); leave such blocks alone and they will start working.

### 4.9 Block files

A block's file is exactly `render(node, 1, resolve_blocks = false)`:

```
[frontmatter, always beginning with id; done: if the block is a checked task]
[the node's title line, as spelled: "# Title" or "- Title"]
[its children: text and nodes, in order]
```

- The root is written as it is spelled in the tree. A block that is a heading is a `#` at column 0; one
  that is a bullet is a `- ` at column 0, with its body and children indented under it exactly
  as they would be in the parent. The file is the only place the spelling is recorded.
- The block's task state is the checkbox on its title line, with a `done:` date in the
  frontmatter when it is checked. Toggling the node — from the outline, or the embed —
  writes to the file.
- The first node at column 0 is the block's root. **Anything after it at column 0 — more
  nodes, or text — is adopted as the root's children**, in order, as if it were indented
  under it. That is what a phone editor produces by appending `- new thing` to a block
  spelled as a bullet, and it must never make the block unusable. Adopted lines are
  non-canonical (`notes check` notes them); the next write of the block, or `check --fix`,
  writes them in place under the root (indented under a bullet root, re-levelled under a
  heading root).
- Text before the root other than frontmatter, or an embed before it, is a diagnostic; the
  app treats that file as read-only until fixed, since it cannot tell what the block is.
- `root.md` is the file of the implicit Root node; its frontmatter holds vault-level
  properties (rarely needed) and never an `id`.

### 4.10 Example

`root.md`:

```markdown
# Homelab

Two boxes in the closet, one at Hetzner.

## NAS

### ![[dozzod-binwes-talsun-worbec]]

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
due: 2026-09-20
---

- [ ] Order new switch
  Two options, noted under Networking.
```

Note that "Replace the flaky switch" is a one-line task with no file; "ZFS layout" is a
section block, so its embed is a heading at the level it has under NAS, while "Order new
switch" is an item block and is embedded bare. "Order new switch" became a block the moment
it got a due date; its checkbox moved into the file with its title line, and its file starts
with a bullet because that is what it is. "Options" is
a section nested under an item, and "Snapshot policy" is a section that is itself a task.

---

## 5. Projection: zoom, render, splice

### 5.1 `render(node, base, resolve_blocks) → String`

Emits the node and its subtree, in document order, as a standalone Markdown document:

1. If the node is a block and `base == 1` and `resolve_blocks = false`: its frontmatter.
2. The node's title line — checkbox included — at level `base` for a section, or at
   indent 0 for an item.
3. The node's children in order: each text child verbatim (dedented by the node's original
   indent), each child node recursively — sections re-levelled by `base − level(node)`,
   everything re-indented by `−indent(node)`.
4. Blank lines are text and are reproduced where they are (§3.3); `render` adds none.
5. Embeds: with `resolve_blocks = false`, written as embeds in their form (§4.7). With `resolve_blocks = true`, the
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
   embed at the position of that block's title line — a heading embed if that title line is
   a heading, a bare one if it is a bullet (§4.7). The result is exactly an edited
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
before a reload, and on quit or any other end of the app. A commit may write several
files — one per dirty block — but each block is written by its own splice, and the law
holds per block.

Edge cases follow from the tags, not from rules:

- Renaming a nested block is editing its title line; it is that block's line, so its file
  is rewritten (and keeps its name, §6.4). No second step.
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
demotes anything.

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
3. Replace the node's span in its parent file with an embed at the node's position: a
   heading embed at its level for a section, a bare `![[id]]` at its indent for an item
   (§4.7). The text children around the node stay in the parent. A checkbox on the node
   stays on its title line, now in the block's file (§4.5).
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
- A block file has one root, its first node at column 0; later column-0 lines are adopted
  under it (§4.9). Its frontmatter has a valid
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

**Names are set once.** The name is the slug of the title when the block is made, and a
later title change does not rename the file. Under Syncthing a rename arrives on other
devices as a delete plus a create; a phone that edits the old name meanwhile turns that
into a sync conflict or an orphan, for nothing — the prefix alone identifies the file and
nothing references the filename. A file renamed or titled by another tool is still found by
its id. `notes check` reports filenames whose prefix is not a leading run of the id (a
problem) and names that no longer match the title (stale, harmless); `notes check --fix`
renames both, which is the one moment the app renames a file.

### 6.5 Refile

Refile is always a span operation: delete the subtree's span from the source file and insert
the re-levelled text at the destination (§5). Refiling a block moves only its embed line;
the file does not move. Any node may be refiled under any node;
spelling is preserved. A task block's state travels with its file, not its embed.

The subtree becomes the destination's **last child, clamped** by the ordering rule (§3.1): a
section goes after everything; an item goes after the destination's last item or text child,
just before its first section child. `Ctrl-Enter` (first child) is clamped the same way: a
section as first child lands just after the destination's last item. Text children of the
source's old parent stay behind.

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
  `--to <target>` captures under any node instead. A captured item is clamped like any
  inserted item (§3.1): under a target with section children it lands just before the first
  of them. Capture never creates a block; dates
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
| block | `- [ ] Title` / `## [ ] Title` | `[x]` + `done: <date>` in frontmatter |

Any node can be a task. A checkbox task has exactly this much state and nothing else. Its
completion date is not recorded; if you need one, make the task a block so toggling
writes `done:` (§8.2).

### 8.2 Task blocks

A task that needs a date, a property, a completion record, or a body with structure becomes
a block (§6.1). Its checkbox moves into the block's file with its title line; the embed in
the parent is `![[id]]`. Toggling from anywhere (outline or embed) rewrites that checkbox and
stamps or removes `done:`.
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
hiding them (remembered with the view, §10.1). The selection stays on its node; one
hidden with them gives way to its next shown sibling, else the row above it.

When a list is genuinely finished, *clear done* (palette) trashes every done item with no
open descendants under `target` (default: current zoom root); task blocks'
files go to trash too. For whole finished subtrees — a completed project, a concluded
meeting series — `za` refiles them under `Archive` (§6.5). The trash (§11.5) and file versioning (§12.6) backstop both.

### 8.6 Recurrence

Out of scope for 1.0. Nothing is reserved for it.

---

## 9. Query language

Not in 1.0. Search is the filter box (§10.5): fuzzy title match plus full-text match over
the whole vault, live, no syntax. A query language is future work (§17, §19 decision 16); the
```` ```query ```` fence is reserved for it (§4.8).

---

## 10. TUI

### 10.1 Layout and the pointer

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
 moved “Label the cables”                          ⚠ 1 conflict  root.md  ✓ saved
```

Four regions, each of them live under the pointer:

- The **top bar**: the breadcrumb of the zoom root (`fold` is the vault root; every
  segment is a link that zooms there), and buttons for *Filter*, *Capture*, *Reading
  pane*, *Undo*, *Redo*, *Commands* and *Help*. Buttons shrink to their icons when the bar
  is narrow; a toggle's button looks pressed while it is on.
- The **outline pane**: titles, fold markers (`▸ ▾`), task glyphs (`☐ ☑`), the block
  marker (`▤`), a due date and the open/total count of the tasks below (a leaf shows no
  count). Done tasks are dimmed and struck through; sections are bold. Long titles end in
  `…`. Beside the reading pane, the date and count sit at the pane's right edge and rows
  show titles only. Without it, they follow the title, and after them, dimmed, comes the
  first line of the node's own text, so no prose is out of sight. A conflict copy
  (§12.5) has `⚠`, in the warning colour, for `▤`, and, in place of its text, whose copy
  it is: *other device · PHONE 09-27 10:00*. It starts folded until unfolded or zoomed
  into, a fold the view does not remember.
- The **reading pane**: `render(target, 1, true)`, the zoom root's or else the selected
  node's, with light Markdown styling (§10.9); a block's properties appear as a dimmed
  `⚑` line under its title, never as text, and a conflict copy's title line ends in its
  `⚠` and whose copy it is, as its row does. Its border carries the node's title and the
  buttons *Edit* and `⋯` (the node menu). While editing, the pane is the editor (§10.6) and
  its buttons are *Done* and *Revert*.
- The **status bar**: the last message on the left (at start, a hint at the gestures and
  `?`, cut down to *right-click for actions · ? help* where it doesn't fit); on the right
  the unresolved conflicts (click to resolve; lit while new ones wait, §10.7), *done
  hidden* when `zd` is on (click to show), the file, and the save state. A verb's message
  names the node it acted on, in the menus' words, with the next key where one helps —
  *deleted “Homelab” (12 nodes) · u undoes*, *moved “rack” to “NAS” — placed before the
  sections* when the ordering rule chose the place (§3.1); in the reading pane, whose keys
  are its own (§10.4), the step is the menu of the line's node: *copied “NAS” · m, then
  choose Paste after* — and names no file or id (§1, 3b), but for undo or redo refusing
  (§10.10); where the bar is short, the name is cut
  down before the words around it. Once a message is some 5 s old and a key
  or click has come since, the next step takes its place: the keys of what is on screen —
  *n new · e edit · x done · m menu · / find · ? help* in the outline, *e edit · Enter
  zoom/follow · Tab outline* in the reading pane, *Esc done · Ctrl-S save · Ctrl-Z undo* in
  the editor (*i insert · :q! revert · :wq done* in Vim and Helix, where `Esc` never
  leaves), *n add · Enter change · d delete · Esc close* in the property form, and each
  popup's own — cut down by whole parts where they don't fit, the first and the last kept,
  so the way on (*? help*, *Esc done*, *:wq done*) stays. An error or a refusal
  (*error: …*, *can't …*, *… refused: …*) stays until a key or click that comes once it
  is that old. The greeting is the outline's, and gives way at once to the editor's or a
  popup's keys. While a key sequence is half typed, the bar says what can follow it:
  *z… p pane · w wrap · d hide done · r raw · a archive*, cut down from its end where it
  doesn't fit; with done hidden, *d show done* comes first.

The outline fills the screen. The reading pane is hidden until asked for: `zp` or the top
bar's `◨` (*Reading pane*) shows it, `Tab` shows it and moves focus there, and the editor
(§10.6) always opens in it, hiding it again when done if it was hidden before. Shown, the
layout splits: outline on the left at a third of the width (minimum 30 columns); the border
between the panes is a handle — drag it to resize. Below 80 columns the panes stack.

**The view is remembered** per vault, in `$XDG_STATE_HOME/fold/views/`: whether the
reading pane is shown, wrapping, hidden done tasks, the editor's keymap, the divider's
place, the folds and the zoom. None of it is in the vault; another device keeps its own.
A fold or zoom whose node has gone is dropped.

**Long lines wrap** in the reading pane and the editor. Prose breaks after a space, and a
wrapped list item's continuation rows hang under its text, not under its bullet or
checkbox; a word longer than the pane breaks where it must. Lines inside fenced code break
at the pane's edge instead, one column early, with `↪` marking each break, so code is
never reflowed. Every screen row of a line is that line to the pointer: a click on any
row selects it or places the cursor where it lands. `zw` (*Wrap lines*) turns wrapping
off, and long lines are cut at the edge. The outline never wraps: a long
title ends in `…`.

**The pointer.** Everything a key does, the pointer does too, and the screen shows where:

| Gesture | Where | Does |
|---|---|---|
| click | an outline row | select it (the reading pane follows) |
| click | `▸` / `▾` | fold / unfold |
| click | `☐` / `☑`, in either pane | toggle the task |
| click | a conflict copy's `⚠`, in either pane | the conflict view at its pair (§10.7) |
| click | `⋯` (shown on the selected and the hovered row) | the node menu |
| click | a breadcrumb segment | zoom there |
| click | a link in the reading pane | open it |
| click | a line in the reading pane / the editor | put the cursor there |
| double-click | an outline row | zoom into it |
| double-click | a line in the reading pane | a heading: zoom into it; an embedded block: follow it; anything else: open the editor with the cursor on that line |
| right-click | a row or a line | the node menu for that node |
| drag | an outline row onto another row's **title** | move it **into** that node, as its last child |
| drag | an outline row onto the space **left of** another row's title | move it **before** that node, as its sibling |
| wheel | either pane, the editor, any list | scroll what is under the pointer; the selection stays put |
| drag | the border between the panes | resize them |
| click | outside any popup | close it |

While dragging, the target row is highlighted (*into*) or marked with `▶` and a bar
(*before*), the row being moved is dimmed, the status bar spells out the move, and the pane
scrolls when the pointer reaches its edge. Both drops are clamped by the ordering rule
(§3.1) and say so when they are; a drop into the node's own subtree is refused. One drag is
one undo step.

**No key acts out of sight.** The wheel leaves the selection, and the reading pane's cursor,
where they are, even out of view. A key that acts on one — in the outline
`x t d s J K > < ~ za r p P`, `e a m y` and the folds; in the reading pane `x`, `Enter` and
`e a m o` — first scrolls it back into view, a third of the way down. A key that changes
nothing then acts; one that changes something stops there, and until the next key or click
the status bar says what a second press does: *press d again to delete “Replace fan”*.
Pressed again, with the node in view, it acts. The pointer acts where it points and is
never held back.

**The node menu** (right-click, `⋯`, or `m`) lists every action on a node, each with its key:
*Edit · Zoom in · Properties… | New sibling · New child | Done / reopen · Task on / off ·
Heading ↔ bullet · Make block | Move up · Move down · Indent · Outdent · Move to… · Archive |
Copy · Paste after · Paste before · Delete*, and on either side of a conflict pair, last,
*Resolve conflict…*: the conflict view at that pair. Moving the pointer onto an item
highlights it, and a click runs it; `↑`/`↓` and the wheel move the highlight on, whatever
item the pointer rests on. On a screen too short for it the separators go first, then the
list scrolls with the selection.

**Pickers instead of typing.** Where an action needs a node — *Move to…*, *Go to…* — the
prompt is a list of every node, title first and path dimmed, narrowed as you type (title
prefix, then title, then path) and picked with a click or `↑`/`↓` and `Enter`. A typed id,
path or title (§3.4) still works. *Move to…* leaves out the moving node's own subtree, and
conflict copies, which keeping ours trashes (§12.5); *Go to…* marks a node in one `⚠`.

Every popup — node menu, prompt, properties, filter, commands, help — has its buttons on
its bottom border (*OK*, *Close*, …), a list you can click and scroll, and closes on a click
outside it or `Esc`.

### 10.2 Modes

`normal` (default), `edit` (the built-in editor over a subtree's Markdown, saving as you go), `filter`
(`/` box), `picker` (the command palette), `properties` (the property form), `help`,
`conflict` (§12.5). The node menu and prompts open over any mode. `Esc` — or a click outside
the popup — closes the topmost thing.

### 10.3 Outline pane — normal mode

| Key | Action |
|---|---|
| `j` / `k` | next / previous visible node |
| `h` | collapse; if collapsed or leaf, go to parent |
| `l` | expand; if expanded, go to first child |
| `H` / `L` | collapse / expand all under cursor |
| `zd` | toggle hiding done nodes (both panes) |
| `zr` | toggle raw mode: exact source in the reading pane, frontmatter included |
| `zw` | toggle wrapping of long lines in the reading pane and the editor (§10.1) |
| `zp` | show / hide the reading pane (§10.1) |
| `za` | archive: refile subtree under `Archive` (§6.5) |
| `-` | go to parent |
| `{` / `}` | previous / next sibling |
| `gg` / `G` | first / last visible node |
| `Ctrl-d` / `Ctrl-u` | half-page down / up |
| `Enter` | zoom: reading pane shows the cursor node; reading pane takes focus |
| `Backspace` | zoom out to parent |
| `>` / `<` | demote / promote: become the last child of the previous sibling node / the next sibling of the parent, both clamped by the ordering rule (§3.1): an item promoted out of a section lands just before the parent's first section sibling. Spelling unchanged. `>` is refused under a conflict copy, which keeping ours trashes (§12.5) |
| `~` | toggle spelling, section ↔ item (subtree unchanged). The node moves to its parent's boundary (§3.1): an item respelled as a section becomes the first section child, a section respelled as an item the last item child, so no sibling changes parent |
| `J` / `K` | move node down / up past the next / previous sibling node; text children stay put. A node and its conflict copy (§12.4) move as one, and a node moves past them as one. An item never moves below a section sibling, nor a section above an item: the move is refused with a message |
| `n` / `N` | new sibling after cursor / new last child, spelled like the cursor node / like the last child node (a section if the parent has section children): inserts an empty node and opens it with `e` |
| `e` | edit the subtree's Markdown in the built-in editor; saves as you go (§10.6) |
| `a` | property editor: a form over the node's properties (§10.6); first property on a plain node makes it a block |
| `x` | toggle task open / done: the checkbox, plus `done:` on a block |
| `t` | toggle task-ness: adds or removes the checkbox (and a block's `done:`) |
| `s` | make the node a block |
| `y` / `d` | yank / delete subtree into the register (delete goes to trash too) |
| `p` / `P` | paste register after / before cursor as sibling, clamped by the ordering rule (§3.1) |
| `r` | refile: fuzzy-pick a destination; `Ctrl-Enter` = as first child |
| `c` / `C` | capture to the inbox as bullet / as task |
| `/` | filter box (§10.5) |
| `m` | the node menu (§10.1) |
| `:` | command palette (§10.8); `?` or `F1` help |
| `u` / `U` | undo / redo |
| `q` | quit (nothing is ever unsaved in normal mode) |

A second key that follows the first in no sequence (`zq`) does nothing and says so: *zq
does nothing · after z press p, w, d, r or a*; `Esc` lets the first key go. A few keys
fold has no use for, where one often reaches for them, say what to press instead: `i` (*i
does nothing here: e edits*), `o` (*n adds a node below*), `Delete` (*d deletes*),
`Ctrl-Z` (*u undoes*), `Ctrl-F` (*/ finds*); any other key that does nothing says nothing.

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
| `:` `?` `F1` `m` `c` `C` `u` `U` `z…` | as in the outline; `m` is the menu of the node under the cursor; `i`, `Delete` (*m, then choose Delete*), `Ctrl-Z` and `Ctrl-F` (*/ searches*) say what to press instead |

### 10.5 Filter box

`/`, or *Filter* in the top bar, opens a popup with an input and the hits below it:
fuzzy title match (nucleo) and full-text match over every text child, each hit shown as its
title with its path dimmed, and `⚠` after the title of a hit in a conflict copy (§12.5).
Clicking a hit — or `↑`/`↓` and `Enter` — unfolds its ancestors and selects it. `Esc` or a
click outside closes. Creating nodes is `n` / `N` (§10.3).

### 10.6 Editing

There is one way to change text: edit the Markdown of the selected subtree. `e`, *Edit* on
the reading pane or in the node menu, or a double-click on a line of text shows
`render(cursor, 1, true)` — the node's title line, body and children, with every
nested block inlined so that no embed, id, frontmatter or file boundary appears.

**Built-in** (`e`): a plain multi-line text box over that text, in the reading pane. There
is nothing to commit and nothing is locked. Each line carries its owning block invisibly
(§5.2); you type anywhere, including across what happens to be a file boundary, and the
app keeps track. Blocks are saved on their own:

- when the cursor moves out of a block that has changed;
- after 750 ms without a keystroke;
- on `Esc` or *Done* (back to normal mode), on any outline verb, before a reload caused by
  an external change, and on quit;
- when the app ends any other way: `SIGTERM`, `SIGHUP` (a closed window, a dropped ssh
  session), `SIGINT`, an I/O error, a panic. A second `SIGTERM` or `SIGINT` ends it at
  once, unsaved, as the signal would: the way out while a write waits on a terminal that
  reads nothing. A `SIGHUP` before one does not make it a second.

A save shows in the status bar's save state (§10.1), not as a message, but for `:w` and
`Ctrl-S`, which are asked for and answer *saved*, and one that an outside change forced,
which the news of that change mentions (§11.2).

The pane's border shows the title of the block the cursor is in, *⚠ conflict copy from
PHONE* after it in a conflict copy (§12.5), and a dot while something is unsaved, and lines
of other blocks in the subtree are drawn a shade dimmer — the only hints that blocks
exist. The border's text stays left of `⌨` and the buttons: where that is short, the title
gives way first, to its first letters, then the words after ⚠ (*⚠ copy from PHONE*, then
*⚠*), then *Editing*; the ⚠, the dot and the Vim or Helix mode stay whole.
*Revert* (or `:q!`) discards changes made since the last save.

Text a save cannot take — its block changed on disk under it (§5.2) — stays in the editor,
and is never dropped without a copy unless asked twice: *Revert* first writes the editor's
whole text to the trash as `unsaved-<title>.md` (§11.5) and the status line names the entry,
and the app ending any way but a quit does the same, saying where once the terminal is back:
a panic in the save itself too, whether an autosave, `Esc` or leaving a block ran it.
Where the trash cannot be written, *Revert* drops nothing and says so, and a second *Revert*
on the same text drops it. However it ends, but for that second signal, the app leaves the
terminal as it found it, a panic's message printed after.

**Keymaps.** The editor speaks one of three keymaps, over the same text, cursor, selection,
clipboard and undo:

- **normal** (the default): a conventional editor in the manner of micro. Always typing;
  arrows, `Home`/`End`, `PgUp`/`PgDn`, `Ctrl-←/→` by word; `Shift` with any of them
  selects; typing replaces the selection. `Ctrl-C`/`X`/`V` copy, cut, paste (with nothing
  selected, the current line); `Ctrl-Z`/`Y` undo, redo; `Ctrl-A` select all; `Ctrl-K` cut
  the line; `Ctrl-D` duplicate; `Alt-↑/↓` move lines; `Tab`/`Shift-Tab` indent;
  `Ctrl-F` find, `Ctrl-N`/`Ctrl-P` next, previous; `Ctrl-S` save; `Ctrl-E` a `:` command;
  `Esc` clears the selection, then leaves.
- **vim**: normal, insert, visual and visual-line modes; counts; motions `h j k l w b e W
  B E 0 ^ $ gg G { } % f t F T ; ,`; operators `d c y > <` with any motion, doubled for
  lines, and text objects `iw aw i" a" i' a' i( a( i[ a[ i{ a{ i< a< ip ap`; `x X s S D C Y
  r J ~ p P o O i a I A u Ctrl-R .`; `/ ? n N * #`; `ZZ`, `ZQ`.
- **helix**: selection first — `w b e W B E` select, `x` selects (and extends by) lines,
  `% ; Alt-;`, `f t F T`, `gg ge gh gl gs`, `mi` / `ma` + object, `v` to extend; `d c y p P
  R r ~ > < J` act on the selection; `i a I A o O`; `u U`; `/ ? n N *`.

With wrapping on (§10.1), the normal keymap's `↑`/`↓`/`PgUp`/`PgDn` and Helix's `j`/`k`
move by screen row through a wrapped line, as micro and Helix do; Vim's `j`/`k` move by line,
and `gj`/`gk` by screen row.

All three take `:` commands — `:w` save, `:q` / `:wq` / `:x` done, `:q!` / `:e!` revert,
`:N` go to line — and in all three a click puts the cursor where it lands, a drag
selects, a double-click selects a word, the wheel scrolls, and text pasted into the
terminal is inserted as typed. Copying also sets the system clipboard (OSC 52). Undo
inside the editor is the editor's own; each save is still one entry in the op log (§10.10).
`F1` shows the help, `?` being text here.

The keymap is `$FOLD_KEYS` (`normal`, `vim`, `helix`), or `--keys`; the *Editor keys*
action, or the `⌨` label in the editor's border, switches it, and the choice is remembered
with the view (§10.1) unless `--keys` or `$FOLD_KEYS` chose the keymap for this run. When
editing ends, focus returns to the pane it came from. Undo inside the editor is the
editor's own; each automatic save is one entry in the session op log (§10.10). The box is
not a Markdown editor and stays one.

Properties are not text. `a`, or *Properties…* in the node menu, opens the **property
editor**: a small form listing the
node's keys and values: click a value to change it (the prompt starts with the current
value), `✕` deletes an entry, *Add* adds one. `due` and `done` accept
only ISO dates; `id` is not shown. Setting the first property on a node that is not a
block makes it one (§6.1), silently except for the `▤` marker; deleting the last one leaves
the block as it is. Frontmatter lines the app does not understand (nested structure,
comments) are listed read-only and preserved.

`n` / `N` insert an empty node and put the cursor on it in the editor, so creating and
editing are the same gesture. `x`, `t`, `~`, `>`/`<`, `J`/`K`, `r` and `d` are the
structural verbs; they exist because toggling a checkbox should not require the editor,
and each saves the editor first.

### 10.7 Conflict view

Opened by the status bar's ⚠ count or *Resolve conflicts*; at a pair, by a copy's `⚠` or
*Resolve conflict…* in either side's node menu; and on its own when a `.sync-conflict-*`
file is detected or a splice hits a changed span — on the first new pair in the outline,
and only from an outline at rest: normal mode, nothing open over it, no key, paste or
click for 2 s. A busy user is not interrupted: the mode stays, the editor keeps its text,
the status line names the node — *sync conflict in “NAS”: click ⚠ to resolve*
— and the ⚠ count is lit until the view opens, from a click or on its own once the outline
is at rest. Opened on its own, for its first half second it ignores `o t b e`, meant for
what was there before. Pairs that come in while it is open leave it on the pair it shows.

It shows each conflict pair (ours in place, the `conflict:` block right after it) side by side; `o` keeps ours,
`t` keeps theirs, `b` keeps both, `e` edits, `n` / `N` next / previous, `u` / `U` undo / redo,
`Enter` finishes. Each choice names its node — *kept theirs for “NAS” · u undoes* — and is
one undo step.
The view's top bar carries the same as buttons — *Previous*, *Next*, *Keep ours*, *Keep
theirs*, *Keep both*, *Edit ours*, *Close* — and the two versions sit side by side, this
device's on the left.
Unresolved pairs remain as `conflict:` blocks and stay listed in the status
line until resolved (§12.5).

### 10.8 Command palette

`:`, or *Commands* in the top bar, opens a popup listing every action the TUI has — key,
name, what it does — filtered as you type: names that start with the query first, then
names and descriptions that contain it, then fuzzy matches. A toggle shows its state
beside its name (*Reading pane · on*). A click or `Enter` runs the highlighted action; if
it needs an argument — a node for *Move to…* or *Go to…*, text for *Capture* — its prompt
follows (§10.1). `Esc` or a click outside closes.

There is no command syntax: nothing is typed except the search and the argument. Every
action is in the palette under a readable name, and the four that have no key or button
live only there: *Clear done*, *Canonicalize*, *Merge sync conflicts*, *Resolve
conflicts*. `?` is help: the pointer gestures first, then the keys.

### 10.9 Markdown styling in the reading pane

Line-based, not a full renderer: headings coloured by level, task checkboxes shown as `☐ ☑`
(clickable; done lines dimmed and struck through), links underlined (clickable),
`` `code` `` tinted, `**bold**` and `*em*` styled, quotes in colour.

**Fenced code** sits on a tinted background, its fence dimmed and its info string in the
accent colour, and is **syntax-highlighted** with tree-sitter when the info string's first
word names a known language. The app compiles in every tree-sitter grammar published on
crates.io that builds against its tree-sitter version and has a highlights query — about
140 languages, from Ada to Zig, including every mainstream programming, shell, markup,
config, query and hardware-description language that has such a crate — keyed by the
language's name and its usual fence aliases (`rs`, `py`, `js`, `ts`, `sh`, `c++`, `c#`,
`kt`, `rb`, `hs`, `ex`, `yml`, `makefile`, `dockerfile`, `sql`, …). Where a grammar crate
ships a highlights query without exporting it, the app carries a copy
(`crates/fold-tui/queries/`, with its provenance). `rust,ignore`, `{.python}` and `sh
title=x` work too. Each grammar is loaded on first use; the block is highlighted as a whole
(so a string or comment spanning lines is right) and cached until its text changes. Any
other language, or a block its grammar cannot parse, is shown in the
plain code colour. Highlighting is the reading pane's only: the editor is a plain text box
(§10.6). Raw text is
never hidden: `#`, `-`, `**` and link targets stay on screen, dimmed. `zr` shows exact
source, frontmatter included.

### 10.10 Undo, redo, saving

Every mutation is an operation with an inverse (span edits, file create/move/delete,
including making blocks). The session op log powers `u` / `U`. Outline verbs write
immediately and atomically; the built-in editor writes each dirty block within 750 ms of the
last keystroke, or sooner when the cursor leaves it (§10.6), so the only unsaved text at any
moment is under a second of typing in one block.

An op-log entry records **exactly the files the operation touched**: for each, its text
before and after, where *absent* stands for a file the operation created or deleted (making
a block, trashing one). An operation that changed nothing leaves no entry, so it does not
clear the redo stack. Undo checks every touched file against its *after* text, and redo
against its *before* text; if any differs — another editor or a sync changed it since — it
refuses with a message naming the file, writes nothing, and keeps the entry. An entry is
named in words, by what was done to which node — *mark “rack” done*, *delete “Homelab”*,
*edit “NAS”* for an editor save — and undo and redo say it: *undone: mark “rack” done*,
*undo refused: root.md changed since mark “rack” done; not overwriting*. Files the
operation did not touch are never written by undo, so an external edit to them survives.
A block cut in the editor and not pasted back (§5.2) is trashed in the entry of the save
that wrote its embed out, even when a later save or *Revert* is what deletes it, so one
undo puts back its embed and its file together.

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
§11.4. Only writes count — a file created, written, renamed or removed — never a file
opened or read, as every reload reads them all. A reload runs only when the files differ
from what the app last read or wrote (a file's text changed, a file came or went) or a new
`*.sync-conflict-*.md` file appeared. The app's own writes, seen once they land, reload
nothing.

- A changed file that the user is not editing is re-parsed; the cursor is re-attached by
  id, then key (§3.4), then the deepest step of the key that still exists.
- The status line says what came in, by the top-level nodes it touched:
  `↻ changed outside fold: Inbox (+1 item)`.
- A changed file with a built-in edit in progress: the editor saves its dirty blocks first;
  a block whose span hash no longer matches is merged two-way (§5.2 step 5), then the file
  is re-parsed and the buffer re-rendered around the cursor; a block cut there and not
  pasted back is deleted then (§5.2). A change only to files the buffer does not hold
  leaves the editor as it is, a cut block still to paste.
- A new `*.sync-conflict-*.md` file starts the merge flow (§12). What it merges without a
  pair is said as above, at startup too; a copy that brings nothing in, one it leaves alone
  or one the same as its file, says nothing, nor that the editor saved first for it. A copy
  it leaves alone or fails on is no change by itself, and is tried again with the next
  change.
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
removal; editor text no save could take, to `<timestamp>-unsaved-<title>.md` (§10.6).
`notes trash list|restore` manages it. The app never deletes user content without
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
`X.md`, as the files are then: an `X.md` that came in with its copy is merged into. Several
conflict files for the same `X.md` are merged oldest first.

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

**Per matched node**, for each of title, checkbox state, text (all its text children, with
their positions among its child nodes, as one field), and — for the file's block — each
frontmatter key independently: `O == T → take O`, otherwise **conflict**. A conflict pair is raised for every field that differs, even when
only one side touched it; the cost of having no per-device state is paid here, in
resolution clicks, never in lost text.

**Children** are merged as sequences of matched nodes: nodes present on one side only are
insertions, placed relative to their matched neighbours and clamped by the ordering rule
(§3.1). `O`'s text children are emitted where `O` has them. Nothing is ever treated as
deleted.

**Conflict output**: the `O` version stays in place. The `T` version is written as a new
block `<prefix>~<name>.md` (a fresh id; the name is `O`'s name) with frontmatter
`conflict: "<device> <timestamp>"` (plus, for a block conflict, `T`'s own properties),
and an embed to it is inserted as the next sibling of `O`, in the form matching `O`'s
spelling (§4.7) — for a section, a heading embed after its whole subtree. The conflict block
is therefore always the node immediately after the one it conflicts with; no other link
between them is stored. No text is ever lost: every node from `O` and `T` appears in the
result exactly once.

The result is written atomically to `X.md` and the conflict file goes to trash. The
algorithm is deterministic and idempotent.

### 12.5 Resolution

The conflict view (§10.7) lists every `conflict:` block, in outline order, each shown
against the sibling before it, and the status line counts them. Resolving a pair:

- **keep ours** — delete the conflict block and its embed (to trash);
- **keep theirs** — replace ours' title and children (text and nodes) with the conflict block's,
  and its frontmatter with the conflict block's minus `id` and `conflict`; then delete
  the conflict block and its embed (to trash);
- **keep both** — drop the `conflict` key; the block stays where it is as an ordinary
  sibling.

Unresolved pairs are ordinary blocks; they sync to every device and are visible in any
editor, and can be lived with indefinitely. The app marks each copy `⚠` and folds it,
leaves its tasks out of the counts, and offers it as no *Move to…* destination; `>` puts
nothing under it (§10.1, §10.3). No verb puts a node between a copy and the node it
follows, which the copy would then pair with: `J`/`K` move the two as one, and a node
added, pasted, dropped or outdented there goes after the copy.

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
| `notes [--keys normal\|vim\|helix]` | open the TUI; creates `root.md` if the directory is empty (§4.1.1); `--keys` picks the editor's keymap (§10.6) |
| `notes capture [TEXT] [--to TARGET] [--task]` | append to the inbox (stdin if no TEXT) |
| `notes check [--fix]` | diagnostics with source spans (§15.7); `--fix` rewrites the vault in canonical form (§4.2) and repairs filenames (§6.4) |
| `notes merge [--dry-run]` | process sync-conflict files non-interactively; list leftovers |
| `notes trash list \| restore ID` | trash management |

That is the whole CLI: the entry points a shell, a hotkey, a cron job on the homelab, or a
recovery session needs. Everything else is a TUI verb.

---

## 14. Settings

There are no vault settings. Everything that could be one is either fixed behaviour or
content: the inbox is the section titled `Inbox` (§7);
filenames are an id prefix plus the title; names keep Unicode; ids are four words in
frontmatter (§6.4); making blocks
are property-triggered or manual only; `done:` records a date; conflict copies are
blocks marked `conflict:`. Each was a knob nobody needs to agree on with a file. Two vaults that
contain the same Markdown behave identically.

There are no device settings either. The app reads no file outside the vault. What other
tools keep in configuration is:

- the vault: `--vault PATH`, else `$NOTES_VAULT`, else the nearest ancestor of `$PWD`
  containing `root.md`, else `~/notes`;
- the trash location: `$XDG_STATE_HOME/notes/trash/` (§11.5);
- the editor's keymap: `--keys`, else `$FOLD_KEYS` (`normal`, `vim`, `helix`), else
  `normal` (§10.6);
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
    task: Option<TaskState>,                    // any node, block or not: the checkbox on its title line
    content: Vec<Content>,                      // ordered children: text runs and nodes (§3.1)
    parent: Option<NodeId>,
    file: FileId,
    span: Span,                                 // bytes in `file`: title line through subtree
    doc: Option<Block>,                      // Some if this node is the root of a file
}

pub struct Block {
    id: Option<Id>,                             // None only for root.md
    name: String,                               // slug of the title when made (or at the last `check --fix`)
    path: RelPath,                              // e.g. "racfer~order-new-switch.md"
    props: IndexMap<String, String>,            // top-level `key: value` lines, order-preserving; includes `id`
    frontmatter_raw: String,                    // verbatim, for lossless rewrite
    edge_span: Span,                            // the embed line in the parent file
}

pub enum Content { Text(Span), Node(NodeId) } // `(Text | Item)* Section*` (§3.1)

pub struct Span { start: usize, end: usize }

pub struct Tree { nodes: Vec<Node>, root: NodeId, files: Vec<FileState> }
pub struct Cursor { path: Vec<NodeId> }         // zipper-style focus; all outline verbs are cursor ops
pub enum NodeKey {                              // survives re-parses (§3.4)
    Root,
    Id(Id),
    Path { block: Option<Id>, steps: Vec<(String, usize)> }, // title + ordinal among same-titled siblings
}
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

`ratatui`, `crossterm`, `tui-textarea` (built-in editor), `notify` (watcher),
`tree-sitter` + `tree-sitter-highlight` and the grammar crates of §10.9 (code highlighting), `jiff`
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
- Every verb (`J`/`K`, `>`/`<`, `~`, `p`/`P`, refile, capture, `n`/`N`, make block, merge)
  leaves every node's children satisfying `(text | item)* section*`, keeps every other node's
  parent, and moves no text child it was not asked to move.
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
parent file, embeds whose form (bare or heading) does not match their block's spelling,
heading embeds whose level is not the level of their position, text directly after a child
node with no blank line, column-0 lines adopted by a block's root, malformed block files, invalid or duplicate `id` keys, ignored `.md` files,
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
| `rg`, `fd`, Helix | plain text | `rg '\[ \]'` is the open task list; `rg '^done: 2026-09'` the blocks finished this month; `rg '^due:'` the dated ones |
| Any phone editor | plain text | capture by appending a `- ` line to the inbox block's file, at any indent: a column-0 line is adopted by the block's root (§4.9); the app adds nothing to it |
| Obsidian, Logseq, FSNotes | mostly readable, structurally wrong | frontmatter as properties and `- [ ]` bullets render; `![[id]]` shows as a broken embed (a heading embed as a heading holding one), `## [ ]` as a literal heading, `#######` as a paragraph, indented headings as code. Fine for reading a file; do not edit structure there |

---

## 17. Non-goals for 1.0

Encryption; CRDT / operation-log sync; a mobile client (the file format is its contract);
a query language, saved views and `--json` output (the ```` ```query ```` fence is reserved); attachment management beyond ignoring `assets/`; full Markdown
rendering or WYSIWYG; plugins; task recurrence; spaced repetition; whiteboards; multiple
vaults open at once; general transclusion; an external `$EDITOR` handoff; properties on non-blocks; export or lowering
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
| M3 | Tasks: toggling on any node, `done:` stamping, dates, clear-done, filter box | replaces a TaskPaper file |
| M4 | Inbox and capture; watcher and reload | safe to edit in Helix and the TUI at once |
| M5 | Sync: conflict detection, two-way node-level merge, `conflict:` blocks, conflict view, `notes merge` | phone + laptop + homelab over Syncthing for two weeks without data loss |
| M6 | CLI polish, diagnostics, docs | 1.0 |

---

## 19. Decisions

1. **Name.** Binary `notes` is a placeholder.
2. **Dated inbox, ISO titles, fixed.** Capture creates `## YYYY-MM-DD` under `Inbox`. There
   is no format choice and no way to turn it off except `--to`. Days append oldest-first; a
   long-lived inbox is a long section, which is what `s` (make block) and *clear done* are for.
3. **Filenames are named once.** A title change does not rename the file: renames sync as
   delete-plus-create and race with edits on other devices. The id prefix identifies the
   file; the name is a hint that `notes check --fix` refreshes on request.
4. **Id format.** Four-word `@p` names (64 bits): collision-free in practice, with three words
   of slack for the filename prefix to grow into. Pronounceable, typeable from memory, and
   self-validating against the syllable tables; nobody is expected to type all four.
5. **Task sections.** Any node can be a task, so a project heading can be checked off. The
   derived count is still shown beside it; there is no roll-up in either direction (§8.3).
6. **One spelling of task state.** The checkbox, for every node, so `rg '\[ \]'` finds every
   open task and making a block never converts anything. Frontmatter holds only what a
   checkbox cannot: `done:`.
7. **Two task states.** Open and done. "Won't do" is a deleted line or a done line; a third
   state would cost a value in every table, a second date key and a key binding. `[-]` is
   read as done.
8. **No un-block.** There is no inverse of making a block: it would have to discard the
   frontmatter, and a verb whose defining feature is discarding data is a footgun. The
   manual route (yank, paste, delete) makes the loss explicit.
9. **No export.** The resolved render is what the reading pane shows; a standalone-Markdown
   exporter is future work with a known scope (§4.2's four forms).
10. **No fuzzy matching in merge.** A node retitled on one device and edited on another
    comes out of a merge as two nodes. A fuzzy title match would need a tunable threshold and
    could pair the wrong nodes; a duplicate is cheaper to fix than a wrong pairing.
11. **Two-way merge, no cache.** A per-device base copy would make sync merges three-way,
    but it would be the only per-device state (§11.3). Sync conflicts therefore produce a
    conflict pair for every differing field; the mitigation is structural — keep
    concurrently-edited things in separate files (§12.1), so conflicts are small and rare.
12. **Text is edited as text, properties as a form.** One subtree edit in a text box covers
    titles, bodies and structure. Frontmatter is a storage format, and showing it would
    surface files, so the property editor is the one non-text edit. The built-in editor is a
    text box; if it wants to become a Markdown editor, stop.
13. **Inbox by title.** Capture finds the inbox by the title `Inbox`. Renaming it means the
    next capture creates a new one. A hidden marker property would be the only setting.
14. **No config file.** Presentation choices are fixed or session toggles; a config format, a
    lookup order and a `notes config` command are not worth it. A real per-device need is
    one environment variable: the editor's keymap is `$FOLD_KEYS` (§10.6), and a
    colour-blind palette, say, would be another.
15. **Windows.** Slugs and atomic writes are Windows-safe by design; nothing else is tested.
16. **No links or query language in 1.0.** Embeds prove the id machinery, and links can be
    built on it without changing the file format, which is why `[[…]]` is reserved rather
    than free. The likely query shape is "everything is a key" (`due:..today`, text is
    literal, `-` negates, `..` ranges); until then the filter box is the only search and the
    `query` fence is reserved.
17. **No per-node timestamps.** One metadata mechanism is worth more than inline annotations
    such as `@done(…)`. An inline task that needs a date becomes a block; the day section is
    the only creation stamp anything else gets.
18. **Flat vault.** The app never creates a directory and ignores any it finds. Hierarchy is
    embeds, and one place for files keeps prefixes unique vault-wide with no per-directory
    rule.
19. **Prefix collisions.** Two blocks whose ids share a first word are `racfer~notes.md`
    and `racfer-wolsun~order-new-switch.md`, regardless of their titles; the prefix grows only
    as far as needed and never shrinks (§6.4). Uniqueness is judged on the prefix alone, so
    the prefix is a complete address and titles can change freely. Roughly one file in 256
    needs a second word from day one, which is cheaper than any rule that depends on titles;
    the full id is too long for a name, the bare slug too collision-prone across devices, and
    a numeric `-2` meaningless.
20. **Ownership travels with lines.** Each editor line is tagged with its block, and blocks
    are saved as the cursor leaves them. That needs no re-matching of nodes after an edit,
    shows no seam between files, and still writes each file from its own text. An external
    editor cannot carry the tags, so there is none in 1.0 (§17).
21. **Third-party editors are readers, not editors.** Once the format owns syntax, Obsidian
    and friends are not a safe place to restructure a vault. Editing a paragraph there is
    fine; moving headings is not. This is the price of §1 principle 2 and is accepted.
