package soy.iko.fold

import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import soy.iko.fold.core.OutlineQuery
import soy.iko.fold.core.Session
import soy.iko.fold.core.Spelling
import soy.iko.fold.core.Task

/**
 * The Kotlin bindings over the Rust core, as the app calls them: the host
 * build of `crates/fold-ffi`, loaded through JNA.
 */
class CoreTest {
    private lateinit var dir: File

    private val all = OutlineQuery(zoom = null, folded = emptyList(), unfolded = emptyList(), hideDone = false)

    @Before
    fun vault() {
        dir = Files.createTempDirectory("fold").toFile()
        dir.resolve("root.md").writeText(ROOT)
        dir.resolve("racfer~order-new-switch.md").writeText(BLOCK)
    }

    @Test
    fun outlineReadsTheSameFilesTheDesktopWrites() {
        val s = Session.open(dir.path)
        val rows = s.outline(all).rows
        assertEquals(
            listOf("Homelab", "NAS", "Replace fan", "Networking", "Order new switch", "Inbox"),
            rows.map { it.title },
        )
        val switch = rows.first { it.title == "Order new switch" }
        assertTrue(switch.block)
        assertEquals("2026-09-20", switch.due)
        assertEquals(Task.OPEN, switch.task)
        assertEquals(Spelling.SECTION, rows.first().spelling)
        assertEquals("Two boxes.", rows.first().preview)
        s.destroy()
    }

    @Test
    fun verbsWriteMarkdownAndUndo() {
        val s = Session.open(dir.path)
        val switch = s.outline(all).rows.first { it.title == "Order new switch" }
        val r = s.toggleTask(switch.key)
        assertTrue(r.message, r.ok)
        val block = dir.resolve("racfer~order-new-switch.md").readText()
        assertTrue(block, block.contains("- [x] Order new switch"))
        assertTrue(block, block.contains("done: "))
        assertTrue(s.undo(null).ok)
        assertEquals(BLOCK, dir.resolve("racfer~order-new-switch.md").readText())
        assertEquals(ROOT, dir.resolve("root.md").readText())

        val captured = s.capture("call the plumber", true)
        assertTrue(captured.message, captured.ok)
        assertTrue(dir.resolve("root.md").readText().contains("- [ ] call the plumber\n"))
        s.destroy()
    }

    @Test
    fun theEditorSavesEachBlockToItsFile() {
        val s = Session.open(dir.path)
        val net = s.outline(all).rows.first { it.title == "Networking" }
        val view = s.editOpen(net.key)
        assertEquals("# Networking\n\n- [ ] Order new switch\n  Two options.", view.text)
        val text = view.text.replace("Two options.", "Two options, both cheap.")
        assertTrue(s.editUpdate(text, null, view.generation).dirty)
        val saved = s.editClose()
        assertTrue(saved.message, saved.ok)
        assertEquals(ROOT, dir.resolve("root.md").readText())
        assertTrue(dir.resolve("racfer~order-new-switch.md").readText().endsWith("  Two options, both cheap.\n"))
        s.destroy()
    }

    @Test
    fun aChangeFromOutsideIsTakenIn() {
        val s = Session.open(dir.path)
        assertFalse(s.refresh(false).changed)
        dir.resolve("root.md").appendText("\n- from the phone\n")
        val r = s.refresh(false)
        assertTrue(r.changed)
        assertNotNull(r.message)
        assertTrue(s.outline(all).rows.any { it.title == "from the phone" })
        s.destroy()
    }

    companion object {
        const val ROOT = "# Homelab\n\nTwo boxes.\n\n## NAS\n\n- [ ] Replace fan\n\n## Networking\n\n![[racfer-hattes-mislup-nodrys]]\n\n# Inbox\n"
        const val BLOCK = "---\nid: racfer-hattes-mislup-nodrys\ndue: 2026-09-20\n---\n\n- [ ] Order new switch\n  Two options.\n"
    }
}
