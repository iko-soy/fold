package soy.iko.fold

import android.os.Build
import android.os.FileObserver
import java.io.File

/**
 * Tells when a file in the vault was written by someone else — Syncthing,
 * a text editor (§11.2). The vault is flat, so watching its directory is
 * enough. Only writes count; fold's own temp files and dotfiles are noise.
 */
class Watcher(dir: File, private val onChange: () -> Unit) {
    private val mask = FileObserver.CLOSE_WRITE or FileObserver.MOVED_TO or FileObserver.MOVED_FROM or
        FileObserver.DELETE or FileObserver.CREATE or FileObserver.MODIFY

    private val observer: FileObserver =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            object : FileObserver(dir, mask) {
                override fun onEvent(event: Int, path: String?) = seen(path)
            }
        } else {
            @Suppress("DEPRECATION")
            object : FileObserver(dir.absolutePath, mask) {
                override fun onEvent(event: Int, path: String?) = seen(path)
            }
        }

    private fun seen(name: String?) {
        if (name == null || name.startsWith('.') || name.endsWith(".tmp")) return
        if (!name.endsWith(".md")) return
        onChange()
    }

    fun start() = observer.startWatching()

    fun stop() = observer.stopWatching()
}
