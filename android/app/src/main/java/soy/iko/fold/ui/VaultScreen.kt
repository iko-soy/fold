@file:OptIn(ExperimentalMaterial3Api::class)

package soy.iko.fold.ui

import android.Manifest
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import soy.iko.fold.FoldViewModel
import soy.iko.fold.Storage

/**
 * Choosing a vault: a folder Syncthing keeps in step with the desktop, or
 * one in the app's own storage. fold needs "all files access" for a folder
 * in shared storage, as any app that edits files in place there does.
 */
@Composable
fun VaultScreen(vm: FoldViewModel, onBack: (() -> Unit)? = null) {
    val context = LocalContext.current
    val c = LocalFoldColors.current
    var typed by remember { mutableStateOf("") }
    var pickError by remember { mutableStateOf<String?>(null) }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri: Uri? ->
        if (uri == null) return@rememberLauncherForActivityResult
        val path = Storage.pathOfTree(uri)
        if (path == null) {
            pickError = "That folder is not on this phone's storage; pick a local folder, such as the one Syncthing syncs."
        } else {
            pickError = null
            vm.open(path)
        }
    }
    // Android 10 and older grant storage with a runtime permission
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { vm.retryAccess() }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(if (onBack == null) "fold" else "Vaults") },
                navigationIcon = {
                    if (onBack != null) TextButton(onClick = onBack) { Text("Back") }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier.padding(padding).fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                "One outline of notes and tasks, stored as plain Markdown. Open the folder your fold vault syncs to — the same root.md and block files the desktop app uses.",
                style = MaterialTheme.typography.bodyLarge,
            )
            vm.waitingForAccess?.let { path ->
                Surface(shape = RoundedCornerShape(12.dp), color = c.warn.copy(alpha = 0.14f)) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("fold needs access to ${Storage.shown(path)}", style = MaterialTheme.typography.titleSmall)
                        Text(
                            "It reads and writes the vault's files in place, as Syncthing does, so it needs \"all files access\". It touches nothing outside the vault but its own trash.",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                        Button(onClick = {
                            if (Storage.accessFromSettings) {
                                context.startActivity(Storage.accessSettings(context))
                            } else {
                                permission.launch(Manifest.permission.WRITE_EXTERNAL_STORAGE)
                            }
                        }) { Text("Grant access") }
                        TextButton(onClick = { vm.retryAccess() }) { Text("I've granted it — open the vault") }
                    }
                }
            }
            vm.vaultError?.let { Text("Could not open the vault: $it", color = MaterialTheme.colorScheme.error) }
            pickError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            Button(onClick = { pick.launch(null) }, modifier = Modifier.fillMaxWidth()) {
                Icon(Icons.Filled.FolderOpen, null)
                Text("  Choose the vault folder")
            }
            OutlinedButton(onClick = { vm.usePrivateVault() }, modifier = Modifier.fillMaxWidth()) {
                Icon(Icons.Filled.PhoneAndroid, null)
                Text("  Keep a vault on this phone only")
            }
            OutlinedTextField(
                value = typed,
                onValueChange = { typed = it },
                label = { Text("Or type its path") },
                placeholder = { Text("/storage/emulated/0/Sync/fold") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            if (typed.isNotBlank()) {
                Button(onClick = { vm.open(typed.trim()) }) { Text("Open") }
            }
            val recent = vm.recent
            if (recent.isNotEmpty()) {
                Text("Recent", style = MaterialTheme.typography.titleSmall, color = c.dim)
                for (path in recent) {
                    ListItem(
                        modifier = Modifier.clickable { vm.open(path) },
                        leadingContent = { Icon(Icons.Filled.Folder, null) },
                        headlineContent = { Text(Storage.shown(path)) },
                        supportingContent = { if (path == vm.vault) Text("open now", color = c.accent) },
                    )
                }
            }
            Text(
                "An empty folder becomes a new vault with an Inbox. Point Syncthing's .stignore at *.fold-tmp and *.tmp so half-written files never sync.",
                style = MaterialTheme.typography.bodySmall,
                color = c.dim,
            )
        }
    }
}
