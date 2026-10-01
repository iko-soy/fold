@file:OptIn(ExperimentalMaterial3Api::class)

package soy.iko.fold.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.FormatIndentDecrease
import androidx.compose.material.icons.automirrored.filled.FormatIndentIncrease
import androidx.compose.material.icons.automirrored.filled.FormatListBulleted
import androidx.compose.material.icons.automirrored.filled.Redo
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Tag
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import soy.iko.fold.EditorUi
import soy.iko.fold.FoldViewModel

/**
 * The editor (§10.6): the selected subtree's Markdown, every nested block
 * inlined, in a plain text box. It saves on its own after a pause, and on
 * Done; nothing in it shows a file, an id or frontmatter.
 */
@Composable
fun EditorScreen(vm: FoldViewModel, ed: EditorUi) {
    val c = LocalFoldColors.current
    var more by remember { mutableStateOf(false) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(ed) { focus.requestFocus() }
    Scaffold(
        // the bottom is the keyboard's, or the tool bar's over the navigation bar
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        topBar = {
            TopAppBar(
                navigationIcon = {
                    IconButton(onClick = { vm.editDone() }) { Icon(Icons.Filled.Check, "Done") }
                },
                title = {
                    Column {
                        Text(ed.title.ifEmpty { "(untitled)" }, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            when {
                                ed.failed -> "not saved"
                                ed.dirty -> "editing…"
                                else -> "saved"
                            },
                            style = MaterialTheme.typography.labelSmall,
                            color = if (ed.failed) c.warn else c.dim,
                        )
                    }
                },
                actions = {
                    IconButton(onClick = { vm.editUndo() }, enabled = ed.canUndo) { Icon(Icons.AutoMirrored.Filled.Undo, "Undo") }
                    IconButton(onClick = { vm.editUndo(redo = true) }, enabled = ed.canRedo) { Icon(Icons.AutoMirrored.Filled.Redo, "Redo") }
                    Box {
                        IconButton(onClick = { more = true }) { Icon(Icons.Filled.MoreVert, "More") }
                        DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
                            MenuItem("Save now", { more = false }) { vm.editSave() }
                            MenuItem("Revert", { more = false }) { vm.editRevert() }
                        }
                    }
                },
            )
        },
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize().imePadding()) {
            ed.status?.let {
                Surface(color = c.warn.copy(alpha = 0.14f), modifier = Modifier.fillMaxWidth()) {
                    Text(it, color = c.warn, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
                }
            }
            Box(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState())) {
                BasicTextField(
                    value = ed.value,
                    onValueChange = { vm.editChange(it) },
                    textStyle = MaterialTheme.typography.bodyLarge.copy(
                        fontFamily = FontFamily.Monospace,
                        fontSize = 15.sp,
                        lineHeight = 22.sp,
                        color = MaterialTheme.colorScheme.onSurface,
                    ),
                    cursorBrush = SolidColor(c.accent),
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 12.dp)
                        .focusRequester(focus),
                )
            }
            HorizontalDivider()
            LineTools(ed.value) { vm.editChange(it) }
        }
    }
}

/** Buttons for what a phone keyboard makes slow: indenting, bullets, tasks. */
@Composable
private fun LineTools(value: TextFieldValue, change: (TextFieldValue) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .navigationBarsPadding()
            .horizontalScroll(rememberScrollState()),
    ) {
        IconButton(onClick = { change(indent(value, 1)) }) { Icon(Icons.AutoMirrored.Filled.FormatIndentIncrease, "Indent line") }
        IconButton(onClick = { change(indent(value, -1)) }) { Icon(Icons.AutoMirrored.Filled.FormatIndentDecrease, "Outdent line") }
        IconButton(onClick = { change(bullet(value)) }) { Icon(Icons.AutoMirrored.Filled.FormatListBulleted, "Bullet") }
        IconButton(onClick = { change(checkbox(value)) }) { Icon(Icons.Filled.CheckBoxOutlineBlank, "Checkbox") }
        IconButton(onClick = { change(heading(value)) }) { Icon(Icons.Filled.Tag, "Heading") }
    }
}

/** The current line's bounds in the text. */
private fun lineAt(v: TextFieldValue): IntRange {
    val at = v.selection.start.coerceIn(0, v.text.length)
    val start = v.text.lastIndexOf('\n', at - 1) + 1
    val end = v.text.indexOf('\n', at).let { if (it < 0) v.text.length else it }
    return start until end
}

/** Replace the current line with `f(line)`, keeping the caret in place. */
private fun editLine(v: TextFieldValue, f: (String) -> String): TextFieldValue {
    val r = lineAt(v)
    val line = v.text.substring(r.first, r.last + 1)
    val new = f(line)
    val text = v.text.substring(0, r.first) + new + v.text.substring(r.last + 1)
    val shift = new.length - line.length
    val caret = (v.selection.start + shift).coerceIn(r.first, r.first + new.length)
    return TextFieldValue(text, TextRange(caret))
}

/** Two spaces per level, as the format nests (§4.2). */
private fun indent(v: TextFieldValue, dir: Int) = editLine(v) { line ->
    if (dir > 0) "  $line" else line.removePrefix(if (line.startsWith("  ")) "  " else " ")
}

private val marker = Regex("^( *)(#+ |[-*+] )?(\\[[ xX]] )?")

private fun bullet(v: TextFieldValue) = editLine(v) { line ->
    val m = marker.find(line)!!
    val (sp, mark, box) = m.destructured
    val rest = line.substring(m.value.length)
    // a bullet goes, its checkbox with it; anything else becomes one
    if (mark.isNotEmpty() && !mark.startsWith("#")) "$sp$rest" else "$sp- $box$rest"
}

private fun checkbox(v: TextFieldValue) = editLine(v) { line ->
    val m = marker.find(line)!!
    val (sp, mark, box) = m.destructured
    val rest = line.substring(m.value.length)
    when {
        box.isNotEmpty() -> "$sp$mark$rest"
        mark.isEmpty() -> "$sp- [ ] $rest"
        else -> "$sp$mark[ ] $rest"
    }
}

private fun heading(v: TextFieldValue) = editLine(v) { line ->
    val m = marker.find(line)!!
    val (sp, mark, box) = m.destructured
    val rest = line.substring(m.value.length)
    when {
        mark.startsWith("#") && mark.length < 7 -> "$sp#$mark$box$rest"
        mark.startsWith("#") -> "$sp$box$rest"
        else -> "$sp# $box$rest"
    }
}
