package soy.iko.fold

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import soy.iko.fold.ui.FoldTheme
import soy.iko.fold.ui.Root

class MainActivity : ComponentActivity() {
    private val vm: FoldViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) handle(intent)
        setContent {
            FoldTheme { Root(vm) }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    override fun onResume() {
        super.onResume()
        // a vault in shared storage may have been granted access meanwhile,
        // and Syncthing may have synced while fold was away
        vm.retryAccess()
        vm.onResume()
    }

    override fun onStop() {
        super.onStop()
        vm.onStop()
    }

    /** Text shared to fold, or the Capture shortcut: the capture sheet (§7). */
    private fun handle(intent: Intent?) {
        when (intent?.action) {
            Intent.ACTION_SEND -> {
                val text = listOfNotNull(
                    intent.getStringExtra(Intent.EXTRA_SUBJECT),
                    intent.getStringExtra(Intent.EXTRA_TEXT),
                ).joinToString("\n").trim()
                if (text.isNotEmpty()) vm.captureFromOutside(text)
            }
            ACTION_CAPTURE -> vm.captureFromOutside("")
        }
    }

    companion object {
        const val ACTION_CAPTURE = "soy.iko.fold.CAPTURE"
    }
}
