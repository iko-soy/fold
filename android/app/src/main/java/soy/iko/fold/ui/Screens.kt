@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)

package soy.iko.fold.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.CalendarMonth
import androidx.compose.material.icons.filled.CheckBox
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset
import soy.iko.fold.FoldViewModel
import soy.iko.fold.Screen
import soy.iko.fold.core.Hit
import soy.iko.fold.core.Keep
import soy.iko.fold.core.Property
import soy.iko.fold.core.ReadKind
import soy.iko.fold.core.ReadLine
import soy.iko.fold.core.Task

/** A screen's top bar with a way back. */
@Composable
fun BackBar(title: String, onBack: () -> Unit, actions: @Composable () -> Unit = {}) {
    TopAppBar(
        title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
        actions = { actions() },
    )
}

// ------------------------------------------------------------ reading

/**
 * The reading view (§5.4): `render(node, 1, true)` with light styling
 * (§10.9). A title zooms the outline there; its checkbox toggles; a long
 * press is the node's menu.
 */
@Composable
fun ReadingScreen(vm: FoldViewModel, key: String?) {
    val lines = vm.reading
    val title = lines.firstOrNull { it.kind == ReadKind.TITLE }?.title?.takeIf { key != null } ?: "Everything"
    Scaffold(
        topBar = {
            BackBar(title, { vm.pop() }) {
                if (key != null) IconButton(onClick = { vm.edit(key) }) { Icon(Icons.Filled.Edit, "Edit") }
            }
        },
    ) { padding ->
        LazyColumn(
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = padding.calculateTopPadding() + 4.dp, bottom = padding.calculateBottomPadding() + 32.dp),
        ) {
            // fenced code is styled as a run, so lines go in groups
            val groups = group(lines)
            items(groups.size) { i -> ReadGroup(vm, groups[i]) }
        }
    }
}

/** Lines in runs: each title line alone, body lines together. */
private fun group(lines: List<ReadLine>): List<List<ReadLine>> {
    val out = mutableListOf<MutableList<ReadLine>>()
    for (l in lines) {
        val body = l.kind == ReadKind.BODY || l.kind == ReadKind.BLANK
        val last = out.lastOrNull()
        if (body && last != null && last.first().kind != ReadKind.TITLE && last.first().kind != ReadKind.EMBED) {
            last.add(l)
        } else {
            out.add(mutableListOf(l))
        }
    }
    return out
}

@Composable
private fun ReadGroup(vm: FoldViewModel, lines: List<ReadLine>) {
    val c = LocalFoldColors.current
    val first = lines.first()
    when (first.kind) {
        ReadKind.TITLE -> ReadTitle(vm, first)
        ReadKind.EMBED -> Text("▤ ${first.text.trim()} (missing)", color = c.warn, style = MaterialTheme.typography.bodyMedium)
        else -> {
            // a run of text: dedented by the indent of the node it is under
            val indent = lines.filter { it.text.isNotBlank() }.minOfOrNull { it.text.length - it.text.trimStart().length } ?: 0
            val key = first.node
            MarkdownBody(
                lines.map { if (it.text.length >= indent) it.text.substring(indent) else it.text.trimStart() },
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = (indent * 8).dp)
                    .let { m -> if (key != null) m.combinedClickable(onClick = {}, onDoubleClick = { vm.edit(key) }, onLongClick = { vm.openMenu(key) }) else m },
            )
        }
    }
}

@Composable
private fun ReadTitle(vm: FoldViewModel, l: ReadLine) {
    val c = LocalFoldColors.current
    val key = l.node ?: return
    val style = when (l.heading.toInt()) {
        0 -> MaterialTheme.typography.bodyLarge
        1 -> MaterialTheme.typography.headlineSmall
        2 -> MaterialTheme.typography.titleLarge
        3 -> MaterialTheme.typography.titleMedium
        else -> MaterialTheme.typography.titleSmall
    }
    val done = l.task == Task.DONE
    Column(
        Modifier
            .fillMaxWidth()
            .padding(start = (l.indent.toInt() * 8).dp, top = if (l.heading > 0u) 10.dp else 0.dp)
            .combinedClickable(onClick = { vm.zoom(key); vm.pop() }, onLongClick = { vm.openMenu(key) }),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (l.heading == 0u && l.task == Task.NONE) {
                Text("•", color = c.dim, modifier = Modifier.padding(end = 8.dp))
            }
            if (l.task != Task.NONE) {
                Icon(
                    if (done) Icons.Filled.CheckBox else Icons.Filled.CheckBoxOutlineBlank,
                    contentDescription = if (done) "Reopen" else "Done",
                    tint = if (done) c.done else c.accent,
                    modifier = Modifier.padding(end = 6.dp).size(22.dp).clickable { vm.toggleTask(key) },
                )
            }
            Text(
                inline(l.title.ifEmpty { "(untitled)" }, c),
                style = style.copy(
                    fontWeight = if (l.heading > 0u) FontWeight.SemiBold else style.fontWeight,
                    color = if (done) c.done else style.color,
                    textDecoration = if (done) TextDecoration.LineThrough else null,
                ),
            )
            if (l.conflict != null) {
                Icon(Icons.Filled.Warning, "conflict copy", tint = c.warn, modifier = Modifier.padding(start = 6.dp).size(16.dp))
            }
        }
        l.props?.let { Text("⚑ $it", color = c.dim, style = MaterialTheme.typography.bodySmall) }
        l.conflict?.let { Text("other device · $it", color = c.warn, style = MaterialTheme.typography.bodySmall) }
    }
}

/** A node's exact source, frontmatter included (§10.3 `zr`). */
@Composable
fun SourceDialog(text: String, onClose: () -> Unit) {
    val c = LocalFoldColors.current
    AlertDialog(
        onDismissRequest = onClose,
        confirmButton = { TextButton(onClick = onClose) { Text("Close") } },
        title = { Text("Source") },
        text = {
            Box(Modifier.verticalScroll(rememberScrollState()).horizontalScroll(rememberScrollState()).background(c.codeBackground)) {
                Text(text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall, softWrap = false, modifier = Modifier.padding(8.dp))
            }
        },
    )
}

// ------------------------------------------------------------ search

/**
 * The filter box (§10.5), and the *Move to…* picker (§10.1): a list of
 * nodes, title first and path dimmed, narrowed as you type.
 */
@Composable
fun SearchScreen(vm: FoldViewModel, moving: Screen.MoveTo?) {
    val c = LocalFoldColors.current
    val focus = remember { FocusRequester() }
    LaunchedEffect(moving) { focus.requestFocus() }
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = { vm.pop() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
                title = {
                    TextField(
                        value = vm.searchQuery,
                        onValueChange = { vm.search(it) },
                        placeholder = { Text(if (moving != null) "Move “${moving.title}” to…" else "Find titles and text") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                        colors = TextFieldDefaults.colors(
                            focusedContainerColor = Color.Transparent,
                            unfocusedContainerColor = Color.Transparent,
                            focusedIndicatorColor = Color.Transparent,
                            unfocusedIndicatorColor = Color.Transparent,
                        ),
                        modifier = Modifier.fillMaxWidth().focusRequester(focus),
                    )
                },
            )
        },
    ) { padding ->
        LazyColumn(contentPadding = padding) {
            if (moving == null && vm.searchQuery.isNotBlank() && vm.hits.isEmpty()) {
                item { Text("Nothing matches", color = c.dim, modifier = Modifier.padding(24.dp)) }
            }
            items(vm.hits, key = { it.key }) { hit ->
                HitRow(hit) {
                    if (moving != null) vm.moveTo(moving.key, hit.key) else vm.reveal(hit.key, hit.parent)
                }
            }
        }
    }
}

@Composable
private fun HitRow(hit: Hit, onClick: () -> Unit) {
    val c = LocalFoldColors.current
    ListItem(
        modifier = Modifier.clickable(onClick = onClick),
        leadingContent = when (hit.task) {
            Task.OPEN -> { { Icon(Icons.Filled.CheckBoxOutlineBlank, null, tint = c.accent) } }
            Task.DONE -> { { Icon(Icons.Filled.CheckBox, null, tint = c.done) } }
            Task.NONE -> null
        },
        headlineContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(inline(hit.title.ifEmpty { "(untitled)" }, c), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                if (hit.conflict) Text(" ⚠", color = c.warn)
            }
        },
        supportingContent = {
            Column {
                if (hit.path.isNotEmpty()) Text(hit.path, color = c.dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
                hit.excerpt?.let { Text(it, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall) }
            }
        },
    )
}

// ------------------------------------------------------------ properties

/**
 * The property form (§10.6): the node's keys and values. Setting the first
 * on a node that is not a block makes it one (§6.1); `due` and `done` take
 * ISO dates only (§8.4); lines fold does not understand are read-only.
 */
@Composable
fun PropertiesScreen(vm: FoldViewModel, key: String) {
    val c = LocalFoldColors.current
    var editing by remember { mutableStateOf<Pair<String, String>?>(null) }
    var adding by remember { mutableStateOf(false) }
    val node = vm.outline?.rows?.firstOrNull { it.key == key }?.title ?: vm.outline?.zoom?.takeIf { it.key == key }?.title
    Scaffold(
        topBar = { BackBar(node?.let { "Properties · $it" } ?: "Properties", { vm.pop() }) },
        floatingActionButton = {
            androidx.compose.material3.ExtendedFloatingActionButton(
                onClick = { adding = true },
                icon = { Icon(Icons.Filled.Add, null) },
                text = { Text("Add") },
            )
        },
    ) { padding ->
        LazyColumn(contentPadding = padding) {
            if (vm.props.isEmpty()) {
                item {
                    Text(
                        "No properties. Adding one — a due date, say — gives the node its own file.",
                        color = c.dim,
                        modifier = Modifier.padding(24.dp),
                    )
                }
            }
            items(vm.props, key = { it.key }) { p -> PropertyRow(p, { editing = p.key to p.value }) { vm.removeProperty(key, p.key) } }
        }
    }
    editing?.let { (k, v) ->
        PropertyDialog(k, v, fixedKey = true, onDismiss = { editing = null }) { name, value ->
            editing = null
            vm.setProperty(key, name, value)
        }
    }
    if (adding) {
        PropertyDialog("", "", fixedKey = false, onDismiss = { adding = false }) { name, value ->
            adding = false
            vm.setProperty(key, name, value)
        }
    }
}

@Composable
private fun PropertyRow(p: Property, onEdit: () -> Unit, onDelete: () -> Unit) {
    val c = LocalFoldColors.current
    ListItem(
        modifier = if (p.editable) Modifier.clickable(onClick = onEdit) else Modifier,
        headlineContent = { Text(p.value.ifEmpty { "—" }, fontFamily = if (p.editable) null else FontFamily.Monospace) },
        overlineContent = { Text(if (p.editable) p.key else "kept as written", color = c.dim) },
        trailingContent = {
            if (p.editable) IconButton(onClick = onDelete) { Icon(Icons.Filled.Close, "Remove ${p.key}") }
        },
    )
}

@Composable
private fun PropertyDialog(key: String, value: String, fixedKey: Boolean, onDismiss: () -> Unit, onSave: (String, String) -> Unit) {
    var k by remember { mutableStateOf(key) }
    var v by remember { mutableStateOf(value) }
    var picking by remember { mutableStateOf(false) }
    val date = k == "due" || k == "done"
    val valid = k.matches(Regex("[A-Za-z_][A-Za-z0-9_-]*")) && k != "id" &&
        (!date || v.matches(Regex("\\d{4}-\\d{2}-\\d{2}")))
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (fixedKey) key else "New property") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (!fixedKey) {
                    OutlinedTextField(k, { k = it.trim() }, label = { Text("Name") }, singleLine = true, placeholder = { Text("due") })
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        for (s in listOf("due", "tags", "priority")) {
                            androidx.compose.material3.SuggestionChip(onClick = { k = s }, label = { Text(s) })
                        }
                    }
                }
                OutlinedTextField(
                    v,
                    { v = it.replace("\n", " ") },
                    label = { Text(if (date) "YYYY-MM-DD" else "Value") },
                    singleLine = true,
                    isError = date && v.isNotEmpty() && !v.matches(Regex("\\d{4}-\\d{2}-\\d{2}")),
                    trailingIcon = {
                        if (date) IconButton(onClick = { picking = true }) { Icon(Icons.Filled.CalendarMonth, "Pick a date") }
                    },
                )
            }
        },
        confirmButton = { TextButton(onClick = { onSave(k, v) }, enabled = valid) { Text("Save") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
    if (picking) {
        val initial = runCatching { LocalDate.parse(v) }.getOrNull() ?: LocalDate.now()
        val state = rememberDatePickerState(initialSelectedDateMillis = initial.atStartOfDay().toInstant(ZoneOffset.UTC).toEpochMilli())
        DatePickerDialog(
            onDismissRequest = { picking = false },
            confirmButton = {
                TextButton(onClick = {
                    state.selectedDateMillis?.let { v = Instant.ofEpochMilli(it).atZone(ZoneOffset.UTC).toLocalDate().toString() }
                    picking = false
                }) { Text("OK") }
            },
            dismissButton = { TextButton(onClick = { picking = false }) { Text("Cancel") } },
        ) { DatePicker(state) }
    }
}

// ------------------------------------------------------------ conflicts

/**
 * The conflict view (§10.7): each pair, this device's version and the
 * other's, and the choices — keep ours, keep theirs, keep both (§12.5).
 */
@Composable
fun ConflictsScreen(vm: FoldViewModel) {
    val c = LocalFoldColors.current
    val pairs = vm.conflicts
    var at by remember { mutableIntStateOf(0) }
    val i = at.coerceIn(0, (pairs.size - 1).coerceAtLeast(0))
    Scaffold(
        topBar = {
            BackBar(if (pairs.isEmpty()) "Conflicts" else "Conflict ${i + 1} of ${pairs.size}", { vm.pop() }) {
                TextButton(onClick = { at = i - 1 }, enabled = i > 0) { Text("Previous") }
                TextButton(onClick = { at = i + 1 }, enabled = i + 1 < pairs.size) { Text("Next") }
            }
        },
    ) { padding ->
        val pair = pairs.getOrNull(i)
        if (pair == null) {
            Box(Modifier.padding(padding).fillMaxSize(), contentAlignment = Alignment.Center) {
                Text("No sync conflicts", color = c.dim)
            }
            return@Scaffold
        }
        Column(Modifier.padding(padding).fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
            Text("“${pair.title}”", style = MaterialTheme.typography.titleLarge)
            Gap(12.dp)
            Version("This device", pair.oursText, MaterialTheme.colorScheme.primary)
            Gap(12.dp)
            Version("Other device · ${pair.from}", pair.theirsText, c.warn)
            Gap(16.dp)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = { vm.resolve(pair.theirs, Keep.OURS) }, modifier = Modifier.weight(1f)) { Text("Keep ours") }
                FilledTonalButton(onClick = { vm.resolve(pair.theirs, Keep.THEIRS) }, modifier = Modifier.weight(1f)) { Text("Keep theirs") }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { vm.resolve(pair.theirs, Keep.BOTH) }, modifier = Modifier.weight(1f)) { Text("Keep both") }
                OutlinedButton(onClick = { vm.edit(pair.ours) }, modifier = Modifier.weight(1f)) { Text("Edit ours") }
            }
            Gap(8.dp)
            Text(
                "Keep ours trashes the other copy; keep theirs puts its text in place of ours; keep both leaves both as ordinary nodes. Undo takes any choice back.",
                color = c.dim,
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

@Composable
private fun Version(label: String, text: String, accent: Color) {
    val c = LocalFoldColors.current
    Surface(shape = RoundedCornerShape(12.dp), color = c.codeBackground, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(8.dp).background(accent, RoundedCornerShape(50)))
                Spacer(Modifier.width(8.dp))
                Text(label, style = MaterialTheme.typography.labelLarge, color = accent)
            }
            Gap(6.dp)
            Text(text.trimEnd(), fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
        }
    }
}

// ------------------------------------------------------------ check, trash

/** `fold check` (§15.7), and *Canonicalize* (§4.2). */
@Composable
fun ChecksScreen(vm: FoldViewModel) {
    val c = LocalFoldColors.current
    val d = vm.diagnostics
    Scaffold(
        topBar = {
            BackBar("Check the vault", { vm.pop() }) {
                TextButton(onClick = { vm.canonicalize() }) { Text("Canonicalize") }
            }
        },
    ) { padding ->
        LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = padding.calculateTopPadding(), bottom = 32.dp)) {
            when {
                d == null -> item { Text("Checking…", color = c.dim, modifier = Modifier.padding(vertical = 16.dp)) }
                d.isEmpty() -> item { Text("All good: every file is as fold writes it.", modifier = Modifier.padding(vertical = 16.dp)) }
                else -> {
                    item {
                        Text(
                            "Canonicalize rewrites the vault in canonical form and repairs file names, as `fold check --fix` does.",
                            color = c.dim,
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.padding(vertical = 8.dp),
                        )
                    }
                    items(d) { line ->
                        Text(line, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(vertical = 4.dp))
                        HorizontalDivider(color = c.guide)
                    }
                }
            }
        }
    }
}

/** The trash (§11.5): what was deleted on this device, newest first. */
@Composable
fun TrashScreen(vm: FoldViewModel) {
    val c = LocalFoldColors.current
    Scaffold(topBar = { BackBar("Trash", { vm.pop() }) }) { padding ->
        LazyColumn(contentPadding = padding) {
            if (vm.trash.isEmpty()) item { Text("The trash is empty", color = c.dim, modifier = Modifier.padding(24.dp)) }
            itemsIndexed(vm.trash, key = { _, e -> e.name }) { _, e ->
                ListItem(
                    modifier = Modifier.clickable { vm.showTrash(e.name) },
                    headlineContent = { Text(e.name.drop(16).ifEmpty { e.name }, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    supportingContent = {
                        val stamp = e.name.take(15)
                        val shown = if (stamp.length == 15 && stamp[8] == '-') "${stamp.substring(0, 4)}-${stamp.substring(4, 6)}-${stamp.substring(6, 8)} ${stamp.substring(9, 11)}:${stamp.substring(11, 13)}" else ""
                        Text("$shown · ${e.bytes} bytes", color = c.dim)
                    },
                )
            }
        }
    }
    vm.trashShown?.let { (name, text) ->
        AlertDialog(
            onDismissRequest = { vm.showTrash(null) },
            title = { Text(name.drop(16).ifEmpty { name }, maxLines = 2, overflow = TextOverflow.Ellipsis) },
            text = {
                Box(Modifier.height(360.dp).verticalScroll(rememberScrollState()).background(c.codeBackground)) {
                    androidx.compose.foundation.text.selection.SelectionContainer {
                        Text(text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(8.dp))
                    }
                }
            },
            confirmButton = { TextButton(onClick = { vm.restore(name) }) { Text("Restore") } },
            dismissButton = { TextButton(onClick = { vm.showTrash(null) }) { Text("Close") } },
        )
    }
}

@Composable
fun Spaced(h: androidx.compose.ui.unit.Dp) = Spacer(Modifier.height(h))
