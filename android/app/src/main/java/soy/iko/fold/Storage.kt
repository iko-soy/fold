package soy.iko.fold

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import java.io.File

/**
 * Where vaults live on a phone, and the access fold needs to them.
 *
 * fold reads and writes a vault as a directory of plain files, exactly as on
 * a desktop (atomic renames, a directory listing, sync-conflict copies next
 * to their files), so it works on a real path: a folder in shared storage
 * that Syncthing also syncs, with "all files access", or a folder in the
 * app's own storage that needs no permission.
 */
object Storage {
    /** The app's own folder: always writable, never synced by others. */
    fun privateVault(context: Context): File = context.filesDir.resolve("vault")

    /** Whether `path` lies in shared storage, where fold needs a permission. */
    fun isShared(context: Context, path: String): Boolean {
        val own = listOfNotNull(context.filesDir, context.getExternalFilesDir(null)?.parentFile)
        return own.none { path.startsWith(it.absolutePath) }
    }

    fun hasAccess(context: Context, path: String): Boolean {
        if (!isShared(context, path)) return true
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Environment.isExternalStorageManager()
        } else {
            context.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) ==
                PackageManager.PERMISSION_GRANTED
        }
    }

    /** Whether access is granted from the system's settings (Android 11+). */
    val accessFromSettings: Boolean get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R

    /** The settings page that grants all files access to fold. */
    fun accessSettings(context: Context): Intent =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Intent(
                Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                Uri.parse("package:${context.packageName}"),
            )
        } else {
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))
        }

    /**
     * The path of a folder picked with the system's folder picker: the
     * primary storage (`primary:Sync/fold`) or a removable volume
     * (`1A2B-3C4D:fold`). Null for anything that is not a local folder, such
     * as a cloud provider.
     */
    fun pathOfTree(uri: Uri): String? {
        if (uri.authority != "com.android.externalstorage.documents") return null
        val id = runCatching { DocumentsContract.getTreeDocumentId(uri) }.getOrNull() ?: return null
        val volume = id.substringBefore(':')
        val rest = id.substringAfter(':', "")
        val base = if (volume.equals("primary", ignoreCase = true)) {
            Environment.getExternalStorageDirectory().absolutePath
        } else {
            "/storage/$volume"
        }
        return if (rest.isEmpty()) base else "$base/$rest"
    }

    /** A short way to show a vault's path. */
    fun shown(path: String): String {
        val home = Environment.getExternalStorageDirectory().absolutePath
        return when {
            path.startsWith("$home/") -> path.removePrefix("$home/")
            else -> path
        }
    }
}
