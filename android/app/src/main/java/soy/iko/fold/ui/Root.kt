package soy.iko.fold.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Snackbar
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import soy.iko.fold.FoldViewModel
import soy.iko.fold.Message
import soy.iko.fold.Screen

/** The app: the vault chooser until a vault is open, then the outline and what is open over it. */
@Composable
fun Root(vm: FoldViewModel) {
    val snackbar = remember { SnackbarHostState() }
    // the outline's place, kept while another screen is open
    val outlineList = rememberLazyListState()
    LaunchedEffect(vm) {
        vm.messages.collect { m ->
            snackbar.currentSnackbarData?.dismiss()
            val result = snackbar.showSnackbar(
                message = m.text,
                actionLabel = when (m.action) {
                    Message.Action.Undo -> "Undo"
                    Message.Action.Resolve -> "Resolve"
                    null -> null
                },
                duration = if (m.action != null) SnackbarDuration.Long else SnackbarDuration.Short,
            )
            if (result == SnackbarResult.ActionPerformed) {
                when (m.action) {
                    Message.Action.Undo -> vm.undo()
                    Message.Action.Resolve -> vm.push(Screen.Conflicts)
                    null -> {}
                }
            }
        }
    }
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Box(Modifier.fillMaxSize()) {
            if (vm.vault == null) {
                VaultScreen(vm)
            } else {
                // back: the top screen first, then the zoom, one level at a time
                BackHandler(enabled = vm.screens.isNotEmpty() || vm.view.zoom != null) {
                    if (!vm.pop()) vm.zoomOut()
                }
                // one screen at a time: a screen over the outline would let
                // a touch on its empty space through to the rows beneath
                when (val top = vm.screens.lastOrNull()) {
                    null -> OutlineScreen(vm, outlineList)
                    Screen.Search -> SearchScreen(vm, null)
                    is Screen.MoveTo -> SearchScreen(vm, top)
                    is Screen.Reading -> ReadingScreen(vm, top.key)
                    Screen.Editor -> vm.editor?.let { EditorScreen(vm, it) }
                    is Screen.Properties -> PropertiesScreen(vm, top.key)
                    Screen.Conflicts -> ConflictsScreen(vm)
                    Screen.Trash -> TrashScreen(vm)
                    Screen.Checks -> ChecksScreen(vm)
                    Screen.Vaults -> VaultScreen(vm) { vm.pop() }
                }
            }
            vm.menu?.let { NodeMenu(vm, it) }
            vm.input?.let { InputSheet(vm, it) }
            vm.source?.let { SourceDialog(it) { vm.hideSource() } }
            SnackbarHost(
                snackbar,
                modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = 72.dp),
            ) { Snackbar(it) }
        }
    }
}
