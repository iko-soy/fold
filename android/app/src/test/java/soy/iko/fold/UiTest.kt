package soy.iko.fold

import android.app.Application
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.filter
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.test.core.app.ApplicationProvider
import com.github.takahirom.roborazzi.ExperimentalRoborazziApi
import com.github.takahirom.roborazzi.captureScreenRoboImage
import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import soy.iko.fold.ui.FoldThemeFixed
import soy.iko.fold.ui.Root

/**
 * The app on a sample vault, driven as a person would, with a screenshot of
 * each screen (`./gradlew recordRoborazziDebug` writes them under
 * app/build/outputs/roborazzi).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-xxhdpi")
class UiTest {
    @get:Rule val compose = createComposeRule()

    private val app: Application get() = ApplicationProvider.getApplicationContext()

    private fun vault(): File {
        // each test has a files directory of its own
        val dir = File(app.filesDir, "fold")
        dir.mkdirs()
        Sample.write(dir)
        return dir
    }

    private fun start(dark: Boolean = false, dir: File = vault()): FoldViewModel {
        val vm = FoldViewModel(app)
        vm.open(dir.path)
        compose.setContent { FoldThemeFixed(dark) { Root(vm) } }
        await { vm.outline != null }
        compose.waitForIdle()
        return vm
    }

    /**
     * Wait for what the view model does off the main thread: its results
     * come back as posts to the main looper, which Robolectric runs only
     * when it is idled (Compose's waitUntil sleeps without idling it).
     */
    private fun await(timeout: Long = 10_000, what: () -> Boolean) {
        val end = System.currentTimeMillis() + timeout
        while (!what()) {
            check(System.currentTimeMillis() < end) { "timed out" }
            compose.waitForIdle()
            Thread.sleep(10)
        }
        compose.waitForIdle()
    }

    @OptIn(ExperimentalRoborazziApi::class)
    private fun shot(name: String) = captureScreenRoboImage("build/outputs/roborazzi/$name.png")

    @Test
    fun outline() {
        start()
        compose.onNodeWithText("Homelab").assertExists()
        compose.onNodeWithText("Order new switch").assertExists()
        shot("1-outline")
    }

    @Test
    fun outlineDark() {
        start(dark = true)
        shot("2-outline-dark")
    }

    @Test
    fun zoomAndCheckOff() {
        val vm = start()
        compose.onNodeWithText("Networking").performClick()
        await(5_000) { vm.outline?.zoom?.title == "Networking" }
        compose.waitForIdle()
        shot("3-zoomed")
        // checking a task off writes its checkbox in root.md
        val root = File(vm.vault!!, "root.md")
        val key = vm.outline!!.rows.first { it.title == "Replace the flaky switch" }.key
        vm.toggleTask(key)
        await(5_000) { root.readText().contains("- [x] Replace the flaky switch") }
    }

    @Test
    fun nodeMenu() {
        val vm = start()
        compose.onNodeWithText("Replace the flaky switch").performTouchInput { longClick() }
        await(5_000) { vm.menu != null }
        compose.waitForIdle()
        shot("4-node-menu")
    }

    @Test
    fun editor() {
        val vm = start()
        val key = vm.outline!!.rows.first { it.title == "Networking" }.key
        vm.edit(key)
        await(5_000) { vm.editor != null }
        compose.waitForIdle()
        shot("5-editor")
        assertTrue(vm.editor!!.value.text.startsWith("# Networking"))
    }

    @Test
    fun reading() {
        val vm = start()
        val key = vm.outline!!.rows.first { it.title == "Homelab" }.key
        vm.push(Screen.Reading(key))
        await(5_000) { vm.reading.isNotEmpty() }
        compose.waitForIdle()
        shot("6-reading")
    }

    @Test
    fun capture() {
        val vm = start()
        compose.onNodeWithText("Capture", useUnmergedTree = true).performClick()
        await(5_000) { vm.input != null }
        compose.waitForIdle()
        shot("7-capture")
    }

    @Test
    fun conflicts() {
        val dir = vault()
        File(dir, "root.sync-conflict-20260930-181500-PHONE.md").writeText(
            File(dir, "root.md").readText().replace("Two boxes in the closet, one at Hetzner.", "Two boxes in the closet, one at Hetzner, one at mum's."),
        )
        val vm = start(dir = dir)
        await(5_000) { (vm.outline?.conflicts ?: 0u) > 0u }
        compose.waitForIdle()
        shot("8-outline-conflict")
        vm.push(Screen.Conflicts)
        await(5_000) { vm.conflicts.isNotEmpty() }
        compose.waitForIdle()
        shot("9-conflicts")
    }

    @Test
    fun properties() {
        val vm = start()
        val key = vm.outline!!.rows.first { it.title == "Order new switch" }.key
        vm.push(Screen.Properties(key))
        await(5_000) { vm.props.isNotEmpty() }
        compose.waitForIdle()
        shot("10-properties")
    }

    @Test
    fun searchAndReveal() {
        val vm = start()
        compose.onNodeWithContentDescription("Find").performClick()
        await { vm.screens.lastOrNull() == Screen.Search }
        compose.onNode(hasSetTextAction()).performTextInput("plumb")
        await { vm.hits.any { it.title == "Call the plumber" } }
        shot("11-search")
        compose.onNodeWithText("Call the plumber").performClick()
        // shown where it is: the outline zoomed to its day, the row flashing
        await { vm.outline?.zoom?.title == "2026-09-30" }
        assertTrue(vm.outline!!.rows.any { it.title == "Call the plumber" })
    }

    @Test
    fun moveTo() {
        val vm = start()
        val root = File(vm.vault!!, "root.md")
        compose.onNodeWithText("Replace the flaky switch").performTouchInput { longClick() }
        await { vm.menu != null }
        compose.onNodeWithText("Move to…").performScrollTo().performClick()
        await { vm.screens.lastOrNull() is Screen.MoveTo }
        compose.onNode(hasSetTextAction()).performTextInput("Inbox")
        await { vm.hits.firstOrNull()?.title == "Inbox" }
        shot("12-move-to")
        // the hit itself, first; under it, a hit whose path is Inbox
        compose.onAllNodesWithText("Inbox").filter(!hasSetTextAction()).onFirst().performClick()
        // under Inbox now, before its day sections (§3.1), and out of Networking
        await { root.readText().indexOf("- [ ] Replace the flaky switch") > root.readText().indexOf("# Inbox") }
        assertTrue(root.readText().contains("## Networking\n\n![[racfer-hattes-mislup-nodrys]]"))
        await { vm.screens.isEmpty() }
    }

    @Test
    fun typeAndSave() {
        val vm = start()
        val key = vm.outline!!.rows.first { it.title == "Call the plumber" }.key
        vm.edit(key)
        await { vm.editor != null }
        compose.onNode(hasSetTextAction()).performTextInput("Today: ")
        compose.onNodeWithContentDescription("Done").performClick()
        await { vm.editor == null }
        // typed at the caret, the start of the node's text: before its marker
        val root = File(vm.vault!!, "root.md").readText()
        assertTrue(root, root.contains("Today: - [ ] Call the plumber\n"))
    }

    @Test
    fun noVaultYet() {
        val vm = FoldViewModel(app)
        compose.setContent { FoldThemeFixed { Root(vm) } }
        compose.waitForIdle()
        shot("0-choose-vault")
    }
}

/** A vault as the desktop app writes one (SPEC.md §4.10). */
object Sample {
    fun write(dir: File) {
        File(dir, "root.md").writeText(
            """
            |# Homelab
            |
            |Two boxes in the closet, one at Hetzner.
            |
            |## NAS
            |
            |### ![[dozzod-binwes-talsun-worbec]]
            |
            |## Networking
            |
            |- [ ] Replace the flaky switch
            |![[racfer-hattes-mislup-nodrys]]
            |- [x] Label the cables
            |
            |# Reading list
            |
            |- **The Mythical Man-Month** — Brooks
            |- *A Pattern Language*, Alexander et al.
            |  Read the bits on [entrances](https://en.wikipedia.org/wiki/A_Pattern_Language).
            |
            |# Inbox
            |
            |## 2026-09-30
            |
            |- Talked to Anya about the venue.
            |  ### Options
            |  Warehouse on Ligovsky, or the old bakery. Both need a licence.
            |- [x] Send the deposit
            |- [ ] Call the plumber
            |""".trimMargin(),
        )
        File(dir, "dozzod~zfs-layout.md").writeText(
            """
            |---
            |id: dozzod-binwes-talsun-worbec
            |since: 2024-03
            |tags: [storage, homelab]   # user-defined; the app preserves it and ignores it
            |---
            |
            |# ZFS layout
            |
            |Mirrored pairs, no raidz. Snapshots hourly via `sanoid`.
            |
            |## [ ] Snapshot policy
            |
            |- hourly, keep 24
            |- [x] Move scratch to its own dataset
            |
            |```sh
            |zfs list -t snapshot
            |```
            |""".trimMargin(),
        )
        File(dir, "racfer~order-new-switch.md").writeText(
            """
            |---
            |id: racfer-hattes-mislup-nodrys
            |due: 2026-10-04
            |---
            |
            |- [ ] Order new switch
            |  Two options, noted under Networking.
            |""".trimMargin(),
        )
    }
}
