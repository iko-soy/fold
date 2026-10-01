package soy.iko.fold

import android.app.Application
import android.system.Os

class FoldApp : Application() {
    override fun onCreate() {
        super.onCreate()
        // fold keeps its trash under $XDG_STATE_HOME/fold/trash (§11.5): on
        // the phone that is the app's own storage, never the synced vault
        Os.setenv("HOME", filesDir.absolutePath, true)
        Os.setenv("XDG_STATE_HOME", filesDir.resolve("state").absolutePath, true)
    }
}
