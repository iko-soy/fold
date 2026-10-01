@file:OptIn(ExperimentalMaterial3Api::class)

package soy.iko.fold.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.DriveFileMove
import androidx.compose.material.icons.automirrored.filled.FormatIndentDecrease
import androidx.compose.material.icons.automirrored.filled.FormatIndentIncrease
import androidx.compose.material.icons.automirrored.filled.MenuBook
import androidx.compose.material.icons.automirrored.filled.Notes
import androidx.compose.material.icons.filled.AddBox
import androidx.compose.material.icons.filled.Archive
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.CheckBox
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.Code
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.SubdirectoryArrowRight
import androidx.compose.material.icons.filled.Title
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.ZoomIn
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import soy.iko.fold.FoldViewModel
import soy.iko.fold.Input
import soy.iko.fold.Screen
import soy.iko.fold.core.NodeInfo
import soy.iko.fold.core.Spelling
import soy.iko.fold.core.Task

/**
 * The node menu (§10.1): every action on a node, in the TUI's order —
 * Edit · Zoom in · Properties… | New sibling · New child | Done / reopen ·
 * Task on / off · Heading ↔ bullet · Make block | Move up · Move down ·
 * Indent · Outdent · Move to… · Archive | Copy · Paste after · Paste before
 * · Delete, and Resolve conflict… on either side of a pair.
 */
@Composable
fun NodeMenu(vm: FoldViewModel, node: NodeInfo) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = false)
    val c = LocalFoldColors.current
    val key = node.key
    val copied = vm.outline?.copied
    fun run(action: () -> Unit) = {
        vm.closeMenu()
        action()
    }
    ModalBottomSheet(onDismissRequest = { vm.closeMenu() }, sheetState = state) {
        Column(Modifier.verticalScroll(rememberScrollState()).navigationBarsPadding()) {
            Column(Modifier.padding(horizontal = 24.dp).padding(bottom = 8.dp)) {
                Text(
                    node.title.ifEmpty { "(untitled)" },
                    style = MaterialTheme.typography.titleLarge,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                val path = node.path.dropLast(1).joinToString(" › ") { it.title }
                if (path.isNotEmpty()) Text(path, color = c.dim, style = MaterialTheme.typography.bodySmall)
                if (node.conflict != null) {
                    Text("⚠ conflict copy · other device · ${node.conflict}", color = c.warn, style = MaterialTheme.typography.bodySmall)
                }
            }
            HorizontalDivider()
            Item(Icons.Filled.Edit, "Edit", run { vm.edit(key) })
            Item(Icons.Filled.ZoomIn, "Zoom in", run { vm.zoom(key) })
            Item(Icons.AutoMirrored.Filled.MenuBook, "Read", run { vm.push(Screen.Reading(key)) })
            Item(Icons.Filled.Tune, "Properties…", run { vm.push(Screen.Properties(key)) })
            HorizontalDivider()
            Item(Icons.Filled.AddBox, "New sibling", run { vm.addSibling(key) })
            Item(Icons.Filled.SubdirectoryArrowRight, "New child", run { vm.addChild(key) })
            HorizontalDivider()
            when (node.task) {
                Task.OPEN -> Item(Icons.Filled.CheckBox, "Done", run { vm.toggleTask(key) })
                Task.DONE -> Item(Icons.Filled.CheckBoxOutlineBlank, "Reopen", run { vm.toggleTask(key) })
                Task.NONE -> {}
            }
            Item(
                Icons.Filled.CheckBoxOutlineBlank,
                if (node.task == Task.NONE) "Task on" else "Task off",
                run { vm.toggleTaskness(key) },
            )
            Item(
                if (node.spelling == Spelling.SECTION) Icons.AutoMirrored.Filled.Notes else Icons.Filled.Title,
                if (node.spelling == Spelling.SECTION) "Make it a bullet" else "Make it a heading",
                run { vm.toggleSpelling(key) },
            )
            if (!node.block) Item(Icons.Filled.Description, "Give it its own file", run { vm.makeBlock(key) })
            HorizontalDivider()
            Item(Icons.Filled.ArrowUpward, "Move up", run { vm.moveSibling(key, down = false) })
            Item(Icons.Filled.ArrowDownward, "Move down", run { vm.moveSibling(key, down = true) })
            Item(Icons.AutoMirrored.Filled.FormatIndentIncrease, "Indent", run { vm.indent(key) })
            Item(Icons.AutoMirrored.Filled.FormatIndentDecrease, "Outdent", run { vm.outdent(key) })
            Item(Icons.AutoMirrored.Filled.DriveFileMove, "Move to…", run { vm.push(Screen.MoveTo(key, node.title)) })
            Item(Icons.Filled.Archive, "Archive", run { vm.archive(key) })
            HorizontalDivider()
            Item(Icons.Filled.ContentCopy, "Copy", run { vm.copy(key) })
            if (copied != null) {
                Item(Icons.Filled.ContentPaste, "Paste $copied after", run { vm.paste(key, after = true) })
                Item(Icons.Filled.ContentPaste, "Paste $copied before", run { vm.paste(key, after = false) })
            }
            Item(Icons.Filled.Code, "Source", run { vm.showSource(key) })
            Item(Icons.Filled.Delete, "Delete", run { vm.delete(key) }, tint = MaterialTheme.colorScheme.error)
            if (node.paired) {
                HorizontalDivider()
                Item(Icons.Filled.Warning, "Resolve conflict…", run { vm.push(Screen.Conflicts) }, tint = c.warn)
            }
        }
    }
}

@Composable
private fun Item(icon: ImageVector, label: String, onClick: () -> Unit, tint: androidx.compose.ui.graphics.Color? = null) {
    ListItem(
        headlineContent = { Text(label, color = tint ?: MaterialTheme.colorScheme.onSurface) },
        leadingContent = { Icon(icon, null, tint = tint ?: MaterialTheme.colorScheme.onSurfaceVariant) },
        colors = ListItemDefaults.colors(containerColor = androidx.compose.ui.graphics.Color.Transparent),
        modifier = Modifier.clickable(onClick = onClick),
    )
}

/**
 * Capture (§7), or a new node beside or under another (§10.3 `n` / `N`):
 * a title, and whether it is a task. Capture takes more lines: the first is
 * the title, the rest its text.
 */
@Composable
fun InputSheet(vm: FoldViewModel, req: Input) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var value by remember(req) { mutableStateOf(TextFieldValue(req.text, TextRange(req.text.length))) }
    var task by rememberSaveable(req) { mutableStateOf(req.task) }
    val focus = remember { FocusRequester() }
    val capture = req.kind == Input.Kind.Capture
    val submit = {
        if (value.text.isNotBlank()) vm.submitInput(value.text, task)
    }
    LaunchedEffect(req) { focus.requestFocus() }
    ModalBottomSheet(onDismissRequest = { vm.dismissInput() }, sheetState = state) {
        Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 16.dp).imePadding()) {
            Text(
                when (req.kind) {
                    Input.Kind.Capture -> "Capture to the inbox"
                    Input.Kind.Child -> "New child"
                    Input.Kind.Sibling -> "New sibling"
                },
                style = MaterialTheme.typography.titleMedium,
            )
            OutlinedTextField(
                value = value,
                onValueChange = { value = if (capture) it else it.copy(text = it.text.replace("\n", " ")) },
                placeholder = { Text(if (capture) "What's on your mind?" else "Title") },
                singleLine = !capture,
                maxLines = if (capture) 8 else 1,
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.Sentences,
                    imeAction = if (capture) ImeAction.Default else ImeAction.Done,
                ),
                keyboardActions = KeyboardActions(onDone = { submit() }),
                modifier = Modifier.fillMaxWidth().padding(top = 12.dp).focusRequester(focus),
            )
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 4.dp)) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.weight(1f).clickable { task = !task },
                ) {
                    Checkbox(checked = task, onCheckedChange = { task = it })
                    Text("Task")
                }
                TextButton(onClick = { vm.dismissInput() }) { Text("Cancel") }
                Spacer(Modifier.width(4.dp))
                TextButton(onClick = submit, enabled = value.text.isNotBlank()) {
                    Text(if (capture) "Capture" else "Add")
                }
            }
        }
    }
}
