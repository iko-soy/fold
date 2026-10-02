package soy.iko.fold

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import java.util.concurrent.Executors
import kotlinx.coroutines.Job
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import soy.iko.fold.core.ConflictPair
import soy.iko.fold.core.Hit
import soy.iko.fold.core.Keep
import soy.iko.fold.core.NodeInfo
import soy.iko.fold.core.OpResult
import soy.iko.fold.core.Outline
import soy.iko.fold.core.OutlineQuery
import soy.iko.fold.core.Property
import soy.iko.fold.core.ReadLine
import soy.iko.fold.core.Session
import soy.iko.fold.core.TrashEntry
import soy.iko.fold.core.UndoMark

/** What is on screen above the outline. */
sealed interface Screen {
    data object Search : Screen

    data class MoveTo(val key: String, val title: String) : Screen

    /** The reading view of a node, or of the whole vault. */
    data class Reading(val key: String?) : Screen

    data object Editor : Screen

    data class Properties(val key: String) : Screen

    data object Conflicts : Screen

    data object Trash : Screen

    data object Checks : Screen

    data object Vaults : Screen
}

/**
 * A line for the snackbar, and whether it offers to undo what it says: then
 * `mark` names that change, so the offer never undoes a later one.
 */
data class Message(val text: String, val action: Action? = null, val mark: UndoMark? = null) {
    enum class Action { Undo, Resolve }
}

/** What the input sheet asks for. */
data class Input(
    val kind: Kind,
    /** The node it adds beside or under. */
    val key: String? = null,
    val text: String = "",
    val task: Boolean = false,
) {
    enum class Kind { Capture, Child, Sibling }
}

/** The editor (§10.6): the text field's state, and what saving says. */
class EditorUi(val node: String, val title: String, text: String, generation: UInt) {
    var value by mutableStateOf(TextFieldValue(text))

    /** The core's name for the text the field holds (`Session.editUpdate`). */
    var generation = generation
    var dirty by mutableStateOf(false)
    var canUndo by mutableStateOf(false)
    var canRedo by mutableStateOf(false)

    /** *saved*, or why not. */
    var status by mutableStateOf<String?>(null)
    var failed by mutableStateOf(false)
}

class FoldViewModel(app: Application) : AndroidViewModel(app) {
    private val prefs = Prefs(app)

    /** Every call into the core runs here, one at a time, in order. */
    private val executor = Executors.newSingleThreadExecutor { Thread(it, "fold-core") }
    private val worker = executor.asCoroutineDispatcher()

    private var session: Session? = null
    private var watcher: Watcher? = null

    // ------------------------------------------------------------ state

    var vault by mutableStateOf<String?>(null)
        private set
    var vaultError by mutableStateOf<String?>(null)
        private set

    /** The vault that waits for storage access before it can open. */
    var waitingForAccess by mutableStateOf<String?>(null)
        private set
    var recent by mutableStateOf(prefs.recent)
        private set

    var view by mutableStateOf(ViewState())
        private set
    var outline by mutableStateOf<Outline?>(null)
        private set
    var showPreview by mutableStateOf(prefs.showPreview)
        private set

    /** A row to scroll to and flash: what a verb made or moved. */
    var highlight by mutableStateOf<String?>(null)
        private set

    val screens = mutableStateListOf<Screen>()
    var menu by mutableStateOf<NodeInfo?>(null)
        private set
    var input by mutableStateOf<Input?>(null)
        private set
    var editor by mutableStateOf<EditorUi?>(null)
        private set

    var searchQuery by mutableStateOf("")
        private set
    var hits by mutableStateOf<List<Hit>>(emptyList())
        private set
    var reading by mutableStateOf<List<ReadLine>>(emptyList())
        private set
    var props by mutableStateOf<List<Property>>(emptyList())
        private set
    var conflicts by mutableStateOf<List<ConflictPair>>(emptyList())
        private set
    var trash by mutableStateOf<List<TrashEntry>>(emptyList())
        private set
    var trashShown by mutableStateOf<Pair<String, String>?>(null)
        private set
    var diagnostics by mutableStateOf<List<String>?>(null)
        private set
    var source by mutableStateOf<String?>(null)
        private set

    private val _messages = MutableSharedFlow<Message>(extraBufferCapacity = 16)
    val messages: SharedFlow<Message> = _messages

    private var autosave: Job? = null
    private var refreshing: Job? = null
    private var searching: Job? = null

    /** Text shared to fold before its vault was open. */
    private var pendingCapture: String? = null

    init {
        prefs.vault?.let { open(it) }
    }

    override fun onCleared() {
        watcher?.stop()
        val s = session
        session = null
        // after whatever is queued for it, as `closeVault`
        if (s != null) executor.execute { runCatching { s.editKeep(true) }; s.destroy() }
        executor.shutdown()
    }

    private fun say(text: String, action: Message.Action? = null) {
        _messages.tryEmit(Message(text, action))
    }

    /** Offer to undo what a verb just did: the op-log entry it made, named with it. */
    private fun sayUndoable(r: OpResult) {
        _messages.tryEmit(Message(r.message, if (r.undo != null) Message.Action.Undo else null, r.undo))
    }

    private suspend fun <T> core(f: (Session) -> T): T? {
        val s = session ?: return null
        return withContext(worker) { f(s) }
    }

    // ------------------------------------------------------------ vaults

    /** Open the vault at `path`, after the storage access it needs. */
    fun open(path: String) {
        val app = getApplication<Application>()
        if (!Storage.hasAccess(app, path)) {
            waitingForAccess = path
            return
        }
        waitingForAccess = null
        viewModelScope.launch {
            closeVault()
            val opened = withContext(worker) { runCatching { Session.open(path) } }
            opened.onFailure {
                vaultError = it.message ?: it.toString()
                vault = null
            }
            val s = opened.getOrNull() ?: return@launch
            session = s
            vault = path
            vaultError = null
            prefs.vault = path
            prefs.recent = (listOf(path) + prefs.recent.filter { it != path })
            recent = prefs.recent
            view = prefs.view(path)
            screens.clear()
            watcher = Watcher(File(path)) { refreshSoon() }.also { it.start() }
            // the startup scan: sync-conflict copies already there merge now
            refresh(force = true)
            pendingCapture?.let {
                pendingCapture = null
                input = Input(Input.Kind.Capture, text = it)
            }
        }
    }

    /** The access fold asked for may have been granted meanwhile. */
    fun retryAccess() {
        waitingForAccess?.let { open(it) }
    }

    fun usePrivateVault() = open(Storage.privateVault(getApplication()).absolutePath)

    private suspend fun closeVault() {
        watcher?.stop()
        watcher = null
        refreshing?.cancel()
        searching?.cancel()
        autosave?.cancel()
        val s = session ?: return
        // no call starts on it from here; those already queued run first,
        // then it saves, letting go of a block cut and not pasted back
        // (§5.2), and goes
        session = null
        editor = null
        outline = null
        withContext(worker) {
            val said = s.editKeep(true)
            s.destroy()
            said
        }?.let { say(it) }
    }

    fun forgetVault() {
        viewModelScope.launch {
            closeVault()
            prefs.vault = null
            vault = null
            screens.clear()
        }
    }

    // ------------------------------------------------------------ outline

    private fun query() = OutlineQuery(
        zoom = view.zoom,
        folded = view.folded.toList(),
        unfolded = view.unfolded.toList(),
        hideDone = view.hideDone,
    )

    /** Ask the core for the outline again, and keep the view in step. */
    private suspend fun reload() {
        val o = core { it.outline(query()) } ?: return
        outline = o
        // a zoom whose node went away fell back to its deepest ancestor
        val zoom = o.zoom?.key
        if (zoom != view.zoom) applyView(view.copy(zoom = zoom))
        reloadOpenScreens()
    }

    private suspend fun reloadOpenScreens() {
        for (screen in screens) {
            when (screen) {
                is Screen.Reading -> reading = core { it.reading(screen.key) } ?: emptyList()
                is Screen.Properties -> props = core { it.properties(screen.key) } ?: emptyList()
                Screen.Conflicts -> conflicts = core { it.conflicts() } ?: emptyList()
                Screen.Search -> if (searchQuery.isNotBlank()) hits = core { it.search(searchQuery) } ?: emptyList()
                else -> {}
            }
        }
    }

    private fun applyView(v: ViewState) {
        view = v
        vault?.let { prefs.saveView(it, v) }
    }

    fun refreshOutline() = viewModelScope.launch { reload() }

    fun zoom(key: String?) {
        applyView(view.copy(zoom = key))
        highlight = null
        viewModelScope.launch { reload() }
    }

    /** Back from a zoom: to its parent in the outline (§10.3 Backspace). */
    fun zoomOut(): Boolean {
        val crumbs = outline?.crumbs ?: return false
        if (view.zoom == null) return false
        zoom(crumbs.getOrNull(crumbs.size - 2)?.key)
        return true
    }

    fun toggleFold(key: String, conflictCopy: Boolean) {
        applyView(
            if (conflictCopy) {
                view.copy(unfolded = view.unfolded.toggled(key))
            } else {
                view.copy(folded = view.folded.toggled(key))
            },
        )
        viewModelScope.launch { reload() }
    }

    fun toggleHideDone() {
        applyView(view.copy(hideDone = !view.hideDone))
        say(if (view.hideDone) "done hidden" else "done shown")
        viewModelScope.launch { reload() }
    }

    fun togglePreview() {
        showPreview = !showPreview
        prefs.showPreview = showPreview
    }

    fun highlightShown() {
        highlight = null
    }

    /** Show a node: zoom to its parent, so it is a row on screen, and flash it. */
    fun reveal(key: String, parent: String?) {
        applyView(view.copy(zoom = parent))
        screens.removeAll { it is Screen.Search || it is Screen.Reading }
        viewModelScope.launch {
            reload()
            highlight = key
        }
    }

    private fun Set<String>.toggled(k: String) = if (k in this) this - k else this + k

    // ------------------------------------------------------------ verbs

    /**
     * Run a verb on `subject`. A zoom on the node follows it where it moved
     * (§10.3); the snackbar says what was done, with Undo.
     */
    private fun act(subject: String?, undoable: Boolean = true, op: (Session) -> OpResult) {
        viewModelScope.launch {
            val r = core(op) ?: return@launch
            settle(r)
            if (r.ok && subject != null && view.zoom == subject && r.node != null) {
                applyView(view.copy(zoom = r.node))
            }
            reload()
            if (r.ok) r.node?.let { highlight = it }
            if (r.ok && undoable) sayUndoable(r) else say(r.message)
        }
    }

    /**
     * A verb run while the editor is open saved it first and re-rendered
     * it over what the verb wrote (§10.6): the text field takes the new
     * text, or the editor closes where its node is gone.
     */
    private fun settle(r: OpResult) {
        val ed = editor ?: return
        if (r.editorClosed) {
            editor = null
            screens.remove(Screen.Editor)
            say("the node being edited is gone")
            return
        }
        // a new generation comes with new text, as in `refresh`
        r.editorText?.let { text ->
            val sel = ed.value.selection
            ed.value = TextFieldValue(text, TextRange(sel.start.coerceAtMost(text.length), sel.end.coerceAtMost(text.length)))
            ed.generation = r.editorGeneration
        }
    }

    fun openMenu(key: String) {
        viewModelScope.launch { menu = core { it.node(key) } }
    }

    fun closeMenu() {
        menu = null
    }

    fun toggleTask(key: String) = act(key) { it.toggleTask(key) }

    fun toggleTaskness(key: String) = act(key) { it.toggleTaskness(key) }

    fun toggleSpelling(key: String) = act(key) { it.toggleSpelling(key) }

    fun makeBlock(key: String) = act(key) { it.makeBlock(key) }

    fun moveSibling(key: String, down: Boolean) = act(key) { it.moveSibling(key, down) }

    fun indent(key: String) = act(key) { it.indent(key) }

    fun outdent(key: String) = act(key) { it.outdent(key) }

    fun archive(key: String) = act(key) { it.archive(key) }

    fun delete(key: String) = act(key) { it.delete(key) }

    fun copy(key: String) = act(key, undoable = false) { it.copy(key) }

    fun paste(key: String, after: Boolean) = act(null) { it.paste(key, after) }

    fun clearDone() = act(null) { it.clearDone(view.zoom) }

    fun canonicalize() = act(null) { it.canonicalize() }.also { checks() }

    /** Undo the last change; from a message, only the change it named. */
    fun undo(mark: UndoMark? = null) = act(null, undoable = false) { it.undo(mark) }

    fun redo() = act(null, undoable = false) { it.redo() }

    // ------------------------------------------------------------ input

    fun capture(text: String = "") {
        input = Input(Input.Kind.Capture, text = text)
    }

    /** Text shared to fold, or the Capture shortcut: the capture sheet. */
    fun captureFromOutside(text: String) {
        if (session == null) pendingCapture = text else capture(text)
    }

    fun addChild(key: String?) {
        input = Input(Input.Kind.Child, key = key)
    }

    fun addSibling(key: String) {
        input = Input(Input.Kind.Sibling, key = key)
    }

    fun dismissInput() {
        input = null
    }

    fun submitInput(text: String, task: Boolean) {
        val req = input ?: return
        input = null
        when (req.kind) {
            Input.Kind.Capture -> act(null) { it.capture(text, task) }
            Input.Kind.Child -> act(null) { it.addNode(req.key, text, task, true) }
            Input.Kind.Sibling -> act(null) { it.addNode(req.key, text, task, false) }
        }
    }

    // ------------------------------------------------------------ screens

    fun push(screen: Screen) {
        screens.remove(screen)
        screens.add(screen)
        viewModelScope.launch {
            when (screen) {
                is Screen.Reading -> {
                    source = null
                    reading = core { it.reading(screen.key) } ?: emptyList()
                }
                is Screen.Properties -> props = core { it.properties(screen.key) } ?: emptyList()
                Screen.Conflicts -> conflicts = core { it.conflicts() } ?: emptyList()
                Screen.Trash -> {
                    trashShown = null
                    trash = core { it.trash() } ?: emptyList()
                }
                Screen.Checks -> checks()
                is Screen.MoveTo -> hits = core { it.targets("", screen.key) } ?: emptyList()
                Screen.Search -> {
                    searchQuery = ""
                    hits = emptyList()
                }
                else -> {}
            }
        }
    }

    /** Back: the top screen goes; false when only the outline is left. */
    fun pop(): Boolean {
        val top = screens.lastOrNull() ?: return false
        if (top == Screen.Editor) {
            editDone()
            return true
        }
        screens.removeAt(screens.lastIndex)
        return true
    }

    fun search(q: String) {
        searchQuery = q
        val moving = (screens.lastOrNull() as? Screen.MoveTo)?.key
        searching?.cancel()
        searching = viewModelScope.launch {
            delay(80)
            hits = if (moving != null) {
                core { it.targets(q, moving) } ?: emptyList()
            } else {
                core { it.search(q) } ?: emptyList()
            }
        }
    }

    fun moveTo(key: String, dest: String) {
        screens.removeAll { it is Screen.MoveTo }
        act(key) { it.moveTo(key, dest) }
    }

    fun showSource(key: String) {
        viewModelScope.launch { source = core { it.source(key) } }
    }

    fun hideSource() {
        source = null
    }

    fun setProperty(key: String, name: String, value: String) {
        viewModelScope.launch {
            val r = core { it.setProperty(key, name, value) } ?: return@launch
            settle(r)
            // a first property makes a block (§6.1): the form follows it
            if (r.ok && r.node != null && r.node != key) {
                val i = screens.indexOf(Screen.Properties(key))
                if (i >= 0) screens[i] = Screen.Properties(r.node!!)
                if (view.zoom == key) applyView(view.copy(zoom = r.node))
            }
            reload()
            if (r.ok) sayUndoable(r) else say(r.message)
        }
    }

    fun removeProperty(key: String, name: String) = act(key) { it.removeProperty(key, name) }

    fun resolve(theirs: String, keep: Keep) = act(null) { it.resolve(theirs, keep) }

    fun checks() {
        viewModelScope.launch {
            diagnostics = null
            diagnostics = core { it.diagnostics() } ?: emptyList()
        }
    }

    fun showTrash(name: String?) {
        if (name == null) {
            trashShown = null
            return
        }
        viewModelScope.launch {
            trashShown = core { it.trashText(name) }?.let { name to it }
        }
    }

    fun restore(name: String) {
        viewModelScope.launch {
            val r = core { it.restore(name) } ?: return@launch
            settle(r)
            trash = core { it.trash() } ?: emptyList()
            trashShown = null
            reload()
            say(r.message)
        }
    }

    // ------------------------------------------------------------ files

    /** A write in the vault directory: look at the files once it settles. */
    private fun refreshSoon() {
        refreshing?.cancel()
        refreshing = viewModelScope.launch {
            delay(250)
            refresh(force = false)
        }
    }

    /** The app is back in front: something may have synced meanwhile. */
    fun onResume() {
        if (session != null) viewModelScope.launch { refresh(force = false) }
    }

    /** The app went to the background, and may never come back (§10.6). */
    fun onStop() {
        val s = session ?: return
        autosave?.cancel()
        viewModelScope.launch {
            // a block cut and not pasted back stays in transit: the user may
            // be off copying something, and back to paste it
            withContext(worker) { s.editKeep(false) }?.let { say(it) }
        }
    }

    private suspend fun refresh(force: Boolean) {
        val r = core { it.refresh(force) } ?: return
        r.editorText?.let { text ->
            editor?.let { ed ->
                val sel = ed.value.selection
                ed.value = TextFieldValue(text, TextRange(sel.start.coerceAtMost(text.length), sel.end.coerceAtMost(text.length)))
                ed.generation = r.editorGeneration
            }
        }
        if (r.editorClosed) {
            editor = null
            screens.remove(Screen.Editor)
            say("the node being edited is gone")
        }
        if (r.changed || outline == null) reload()
        r.message?.let { say(it, if (r.raised > 0u) Message.Action.Resolve else null) }
    }

    // ------------------------------------------------------------ editor

    fun edit(key: String) {
        viewModelScope.launch {
            val v = withContext(worker) { session?.let { s -> runCatching { s.editOpen(key) } } } ?: return@launch
            v.onFailure { say(it.message ?: "can't edit") }
            val opened = v.getOrNull() ?: return@launch
            editor = EditorUi(opened.node, opened.title, opened.text, opened.generation)
            menu = null
            push(Screen.Editor)
        }
    }

    fun editChange(input: TextFieldValue) {
        val ed = editor ?: return
        val value = withoutCarriageReturns(input)
        val changed = value.text != ed.value.text
        ed.value = value
        if (!changed) return
        val text = value.text
        val cursor = text.codePointCount(0, value.selection.end.coerceIn(0, text.length)).toUInt()
        val generation = ed.generation
        viewModelScope.launch {
            val st = core { it.editUpdate(text, cursor, generation) } ?: return@launch
            ed.dirty = st.dirty
            ed.canUndo = st.canUndo
            ed.canRedo = st.canRedo
        }
        // saves on its own after a pause in typing (§10.6)
        autosave?.cancel()
        autosave = viewModelScope.launch {
            delay(750)
            editSave()
        }
    }

    fun editSave() {
        val ed = editor ?: return
        viewModelScope.launch {
            val saved = core { it.editSave() } ?: return@launch
            ed.dirty = saved.dirty
            ed.failed = !saved.ok
            ed.status = if (saved.ok) null else saved.message
        }
    }

    /** Done: save and leave the editor; a refused save keeps it open. */
    fun editDone() {
        val ed = editor ?: return
        autosave?.cancel()
        viewModelScope.launch {
            val saved = core { it.editClose() } ?: return@launch
            if (!saved.ok) {
                ed.failed = true
                ed.status = saved.message
                say("${saved.message} — still editing; Revert drops the changes")
                return@launch
            }
            // its title, and so its key, may be what was edited
            if (view.zoom == ed.node && saved.node != null) applyView(view.copy(zoom = saved.node))
            editor = null
            screens.remove(Screen.Editor)
            reload()
            saved.node?.let { highlight = it }
        }
    }

    fun editRevert() {
        autosave?.cancel()
        viewModelScope.launch {
            val r = core { it.editRevert() } ?: return@launch
            if (r.ok) {
                editor = null
                screens.remove(Screen.Editor)
                reload()
            }
            say(r.message)
        }
    }

    fun editUndo(redo: Boolean = false) {
        val ed = editor ?: return
        viewModelScope.launch {
            val back = core { if (redo) it.editRedo() else it.editUndo() } ?: return@launch
            val text = back.text
            val at = ed.value.selection.end.coerceAtMost(text.length)
            ed.value = TextFieldValue(text, TextRange(at))
            ed.generation = back.generation
            val st = core { it.editUpdate(text, null, back.generation) } ?: return@launch
            ed.dirty = st.dirty
            ed.canUndo = st.canUndo
            ed.canRedo = st.canRedo
            autosave?.cancel()
            autosave = viewModelScope.launch {
                delay(750)
                editSave()
            }
        }
    }
}

/** Text pasted from elsewhere may end its lines in `\r\n`; the files end them in `\n` (§4.2). */
internal fun withoutCarriageReturns(v: TextFieldValue): TextFieldValue {
    if ('\r' !in v.text) return v
    val text = v.text.replace("\r\n", "\n").replace('\r', '\n')
    // a lone \r became \n; a \r\n lost its \r
    fun mapped(i: Int): Int {
        val head = v.text.substring(0, i.coerceIn(0, v.text.length))
        return head.replace("\r\n", "\n").replace('\r', '\n').length
    }
    return TextFieldValue(text, TextRange(mapped(v.selection.start), mapped(v.selection.end)))
}
