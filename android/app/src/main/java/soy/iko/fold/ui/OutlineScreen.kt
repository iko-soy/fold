@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class, ExperimentalLayoutApi::class)

package soy.iko.fold.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.MenuBook
import androidx.compose.material.icons.automirrored.filled.Redo
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.CheckBox
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AssistChip
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import soy.iko.fold.FoldViewModel
import soy.iko.fold.Screen
import soy.iko.fold.Storage
import soy.iko.fold.core.Header
import soy.iko.fold.core.Row as OutlineRow
import soy.iko.fold.core.Spelling
import soy.iko.fold.core.Task

private val INDENT: Dp = 20.dp

@Composable
fun OutlineScreen(vm: FoldViewModel, list: LazyListState) {
    val outline = vm.outline
    val zoom = outline?.zoom
    val scroll = TopAppBarDefaults.enterAlwaysScrollBehavior()
    val rows = outline?.rows ?: emptyList()
    // scroll to what a verb made or moved, and let its flash fade
    val highlight = vm.highlight
    LaunchedEffect(highlight, rows) {
        val key = highlight ?: return@LaunchedEffect
        val i = rows.indexOfFirst { it.key == key }
        if (i >= 0) {
            val before = 2 + (if (zoom != null) 1 else 0)
            val visible = list.layoutInfo.visibleItemsInfo.any { it.key == key }
            if (!visible) list.animateScrollToItem((i + before - 2).coerceAtLeast(0))
        }
        delay(1600)
        vm.highlightShown()
    }
    Scaffold(
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        topBar = { OutlineTopBar(vm, zoom, scroll) },
        floatingActionButton = {
            if (zoom == null) {
                ExtendedFloatingActionButton(
                    onClick = { vm.capture() },
                    icon = { Icon(Icons.Filled.Add, null) },
                    text = { Text("Capture") },
                )
            } else {
                ExtendedFloatingActionButton(
                    onClick = { vm.addChild(zoom.key) },
                    icon = { Icon(Icons.Filled.Add, null) },
                    text = { Text("Add") },
                )
            }
        },
    ) { padding ->
        LazyColumn(
            state = list,
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(top = padding.calculateTopPadding(), bottom = padding.calculateBottomPadding() + 96.dp),
        ) {
            item(key = "crumbs") {
                if (zoom != null) Crumbs(vm)
            }
            item(key = "conflicts") {
                val n = outline?.conflicts?.toInt() ?: 0
                if (n > 0) ConflictBanner(n) { vm.push(Screen.Conflicts) }
            }
            if (zoom != null) {
                item(key = "header:" + zoom.key) { ZoomHeader(vm, zoom) }
            }
            items(rows, key = { it.key }) { row ->
                OutlineRowView(
                    row = row,
                    highlighted = row.key == highlight,
                    preview = vm.showPreview,
                    onOpen = { vm.zoom(row.key) },
                    onMenu = { vm.openMenu(row.key) },
                    onFold = { vm.toggleFold(row.key, row.conflict != null) },
                    onCheck = { vm.toggleTask(row.key) },
                    modifier = Modifier.animateItem(),
                )
            }
            if (outline != null && rows.isEmpty()) {
                item(key = "empty") { Empty(zoom != null) }
            }
        }
    }
}

@Composable
private fun OutlineTopBar(vm: FoldViewModel, zoom: Header?, scroll: androidx.compose.material3.TopAppBarScrollBehavior) {
    var more by remember { mutableStateOf(false) }
    val outline = vm.outline
    TopAppBar(
        scrollBehavior = scroll,
        title = {
            Text(
                zoom?.title?.ifEmpty { "(untitled)" } ?: vm.vault?.let { java.io.File(it).name } ?: "fold",
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        navigationIcon = {
            if (zoom != null) {
                IconButton(onClick = { vm.zoomOut() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Zoom out") }
            }
        },
        actions = {
            IconButton(onClick = { vm.push(Screen.Search) }) { Icon(Icons.Filled.Search, "Find") }
            IconButton(onClick = { vm.undo() }, enabled = outline?.undo != null) {
                Icon(Icons.AutoMirrored.Filled.Undo, "Undo")
            }
            IconButton(onClick = { vm.redo() }, enabled = outline?.redo != null) {
                Icon(Icons.AutoMirrored.Filled.Redo, "Redo")
            }
            Box {
                IconButton(onClick = { more = true }) { Icon(Icons.Filled.MoreVert, "More") }
                DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
                    val close = { more = false }
                    MenuItem("Read as a document", close) { vm.push(Screen.Reading(zoom?.key)) }
                    MenuItem("Hide done", close, on = vm.view.hideDone) { vm.toggleHideDone() }
                    MenuItem("Text under titles", close, on = vm.showPreview) { vm.togglePreview() }
                    MenuItem("Clear done" + (zoom?.let { " here" } ?: ""), close) { vm.clearDone() }
                    HorizontalDivider()
                    val n = outline?.conflicts?.toInt() ?: 0
                    MenuItem(if (n > 0) "Resolve conflicts ($n)" else "Resolve conflicts", close) { vm.push(Screen.Conflicts) }
                    MenuItem("Check the vault", close) { vm.push(Screen.Checks) }
                    MenuItem("Trash", close) { vm.push(Screen.Trash) }
                    MenuItem("Vaults", close) { vm.push(Screen.Vaults) }
                }
            }
        },
    )
}

/** A menu entry; a toggle shows its state beside its name (§10.8). */
@Composable
fun MenuItem(label: String, close: () -> Unit, on: Boolean? = null, run: () -> Unit) {
    DropdownMenuItem(
        text = { Text(if (on == null) label else "$label · ${if (on) "on" else "off"}") },
        onClick = {
            close()
            run()
        },
    )
}

/** The breadcrumb of the zoom (§10.1): every segment zooms there. */
@Composable
private fun Crumbs(vm: FoldViewModel) {
    val crumbs = vm.outline?.crumbs ?: return
    val c = LocalFoldColors.current
    FlowRow(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 2.dp),
        verticalArrangement = Arrangement.Center,
    ) {
        Crumb(vm.vault?.let { Storage.shown(it).substringAfterLast('/') } ?: "fold") { vm.zoom(null) }
        for (crumb in crumbs.dropLast(1)) {
            Text("›", color = c.dim, modifier = Modifier.padding(horizontal = 2.dp, vertical = 6.dp))
            Crumb(crumb.title.ifEmpty { "(untitled)" }) { vm.zoom(crumb.key) }
        }
    }
}

@Composable
private fun Crumb(title: String, onClick: () -> Unit) {
    Text(
        title,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = Modifier
            .clickable(onClick = onClick)
            .padding(horizontal = 4.dp, vertical = 6.dp),
    )
}

@Composable
private fun ConflictBanner(n: Int, onClick: () -> Unit) {
    val c = LocalFoldColors.current
    Surface(
        color = c.warn.copy(alpha = 0.14f),
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp).clickable(onClick = onClick),
    ) {
        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Warning, null, tint = c.warn)
            Spacer(Modifier.width(12.dp))
            Text(
                if (n == 1) "1 sync conflict to resolve" else "$n sync conflicts to resolve",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.weight(1f),
            )
            Text("Resolve", color = c.warn, style = MaterialTheme.typography.labelLarge)
        }
    }
}

/** The zoomed node above its children: title, properties, its body. */
@Composable
private fun ZoomHeader(vm: FoldViewModel, h: Header) {
    val c = LocalFoldColors.current
    Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (h.task != Task.NONE) {
                TaskBox(h.task, size = 28.dp) { vm.toggleTask(h.key) }
                Spacer(Modifier.width(8.dp))
            }
            Text(
                inline(h.title.ifEmpty { "(untitled)" }, c),
                style = MaterialTheme.typography.headlineSmall.let {
                    if (h.task == Task.DONE) it.copy(color = c.done, textDecoration = TextDecoration.LineThrough) else it
                },
                modifier = Modifier.weight(1f).combinedClickable(onClick = { vm.edit(h.key) }, onLongClick = { vm.openMenu(h.key) }),
            )
            if (h.total > 0u) {
                Text("${h.open}/${h.total}", color = c.dim, style = MaterialTheme.typography.labelLarge)
            }
        }
        if (h.conflict != null) {
            Text("⚠ conflict copy · other device · ${h.conflict}", color = c.warn, style = MaterialTheme.typography.bodySmall)
        }
        val shown = h.props.filter { it.editable && it.key != "conflict" }
        if (shown.isNotEmpty() || h.block) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), modifier = Modifier.padding(top = 4.dp)) {
                for (p in shown) {
                    AssistChip(
                        onClick = { vm.push(Screen.Properties(h.key)) },
                        label = { Text("${p.key} ${p.value}", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    )
                }
                if (shown.isEmpty()) {
                    AssistChip(onClick = { vm.push(Screen.Properties(h.key)) }, label = { Text("Properties") }, leadingIcon = { Icon(Icons.Filled.Tune, null) })
                }
            }
        }
        if (h.body.isNotEmpty()) {
            MarkdownBody(
                h.body,
                modifier = Modifier
                    .padding(top = 8.dp)
                    .fillMaxWidth()
                    .clickable { vm.edit(h.key) },
                style = MaterialTheme.typography.bodyLarge,
            )
        }
        Row(Modifier.padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            TextButton(onClick = { vm.edit(h.key) }) {
                Icon(Icons.Filled.Edit, null, Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Edit")
            }
            TextButton(onClick = { vm.push(Screen.Reading(h.key)) }) {
                Icon(Icons.AutoMirrored.Filled.MenuBook, null, Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Read")
            }
            TextButton(onClick = { vm.openMenu(h.key) }) { Text("More…") }
        }
        HorizontalDivider(color = c.guide)
    }
}

@Composable
private fun Empty(zoomed: Boolean) {
    val c = LocalFoldColors.current
    Box(Modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) {
        Text(
            if (zoomed) "Nothing under it yet — Add makes a child" else "Nothing here yet — Capture adds to the inbox",
            color = c.dim,
            style = MaterialTheme.typography.bodyMedium,
        )
    }
}

/** A task's checkbox, ☐ or ☑ (§10.1), a touch target of its own. */
@Composable
fun TaskBox(task: Task, size: Dp = 24.dp, onClick: () -> Unit) {
    val c = LocalFoldColors.current
    IconButton(onClick = onClick, modifier = Modifier.size(size + 16.dp)) {
        Icon(
            if (task == Task.DONE) Icons.Filled.CheckBox else Icons.Filled.CheckBoxOutlineBlank,
            contentDescription = if (task == Task.DONE) "Reopen" else "Done",
            tint = if (task == Task.DONE) c.done else c.accent,
            modifier = Modifier.size(size),
        )
    }
}

/** One row of the outline (§10.1). */
@Composable
fun OutlineRowView(
    row: OutlineRow,
    highlighted: Boolean,
    preview: Boolean,
    onOpen: () -> Unit,
    onMenu: () -> Unit,
    onFold: () -> Unit,
    onCheck: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val c = LocalFoldColors.current
    val density = LocalDensity.current
    val bg by animateColorAsState(if (highlighted) c.highlight else Color.Transparent, label = "highlight")
    val depth = row.depth.toInt()
    Row(
        modifier = modifier
            .fillMaxWidth()
            .background(bg)
            .combinedClickable(onClick = onOpen, onLongClick = onMenu, onLongClickLabel = "Actions")
            .drawBehind {
                // a guide line down each level of nesting
                val step = with(density) { INDENT.toPx() }
                val start = with(density) { 8.dp.toPx() + 12.dp.toPx() }
                for (d in 0 until depth) {
                    val x = start + d * step
                    drawLine(c.guide, Offset(x, 0f), Offset(x, size.height), strokeWidth = with(density) { 1.dp.toPx() })
                }
            }
            .padding(start = 8.dp + INDENT * depth, end = 8.dp)
            .heightIn(min = 48.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // the fold marker, or a bullet for a leaf
        Box(Modifier.size(24.dp).let { if (row.hasChildren) it.clickable(onClick = onFold) else it }, contentAlignment = Alignment.Center) {
            if (row.hasChildren) {
                Icon(
                    if (row.folded) Icons.Filled.ChevronRight else Icons.Filled.ExpandMore,
                    contentDescription = if (row.folded) "Unfold" else "Fold",
                    tint = c.dim,
                )
            } else {
                Box(Modifier.size(5.dp).background(c.dim, RoundedCornerShape(50)))
            }
        }
        if (row.task != Task.NONE) {
            TaskBox(row.task, size = 22.dp, onClick = onCheck)
        } else {
            Spacer(Modifier.width(6.dp))
        }
        Column(Modifier.weight(1f).padding(vertical = 6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                val done = row.task == Task.DONE
                val title = buildAnnotatedString {
                    append(inline(row.title.ifEmpty { "(untitled)" }, c))
                }
                Text(
                    title,
                    style = MaterialTheme.typography.bodyLarge.copy(
                        fontWeight = if (row.spelling == Spelling.SECTION) FontWeight.SemiBold else FontWeight.Normal,
                        color = if (done) c.done else MaterialTheme.colorScheme.onSurface,
                        textDecoration = if (done) TextDecoration.LineThrough else null,
                    ),
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (row.conflict != null) {
                    Icon(Icons.Filled.Warning, "conflict copy", tint = c.warn, modifier = Modifier.padding(start = 4.dp).size(16.dp))
                } else if (row.block) {
                    Icon(Icons.Filled.Description, "own file", tint = c.dim, modifier = Modifier.padding(start = 4.dp).size(14.dp))
                }
                if (row.broken) {
                    Text(" missing", color = c.warn, style = MaterialTheme.typography.labelSmall)
                }
            }
            val second = when {
                row.conflict != null -> "other device · ${row.conflict}"
                preview -> row.preview
                else -> ""
            }
            if (second.isNotEmpty()) {
                Text(
                    second,
                    style = MaterialTheme.typography.bodySmall.copy(
                        color = if (row.conflict != null) c.warn else c.dim,
                        fontStyle = if (row.conflict != null) FontStyle.Normal else FontStyle.Italic,
                    ),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Meta(row)
    }
}

/** The due date and the open/total count of the tasks below (§10.1). */
@Composable
private fun Meta(row: OutlineRow) {
    val c = LocalFoldColors.current
    val due = row.due
    if (due == null && row.total == 0u) return
    Column(horizontalAlignment = Alignment.End, modifier = Modifier.padding(start = 8.dp)) {
        if (due != null) {
            val overdue = row.task == Task.OPEN && due < java.time.LocalDate.now().toString()
            Text(
                buildAnnotatedString {
                    withStyle(SpanStyle(color = if (overdue) c.warn else c.accent)) { append("⏲ ") }
                    append(if (due.length == 10 && due.startsWith(java.time.LocalDate.now().year.toString())) due.substring(5) else due)
                },
                style = MaterialTheme.typography.labelMedium,
                color = if (overdue) c.warn else c.dim,
                modifier = Modifier.semantics { contentDescription = "due $due" },
            )
        }
        if (row.total > 0u) {
            Text("${row.open}/${row.total}", style = MaterialTheme.typography.labelMedium, color = c.dim)
        }
    }
}

@Composable
fun Gap(h: Dp) = Spacer(Modifier.height(h))
