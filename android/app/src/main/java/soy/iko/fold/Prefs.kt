package soy.iko.fold

import android.content.Context
import android.util.Base64

/**
 * What fold remembers on this device (§10.1): which vault is open, and per
 * vault the view — folds, zoom, hidden done tasks. None of it is in the
 * vault; another device keeps its own.
 */
class Prefs(context: Context) {
    private val prefs = context.getSharedPreferences("fold", Context.MODE_PRIVATE)

    var vault: String?
        get() = prefs.getString("vault", null)
        set(v) = prefs.edit().apply { if (v == null) remove("vault") else putString("vault", v) }.apply()

    /** Vaults opened before, most recent first. */
    var recent: List<String>
        get() = prefs.getString("recent", null)?.split('\n')?.filter { it.isNotEmpty() } ?: emptyList()
        set(v) = prefs.edit().putString("recent", v.take(5).joinToString("\n")).apply()

    var showPreview: Boolean
        get() = prefs.getBoolean("preview", true)
        set(v) = prefs.edit().putBoolean("preview", v).apply()

    fun view(vault: String): ViewState {
        val p = "view:$vault:"
        return ViewState(
            zoom = prefs.getString(p + "zoom", null)?.let(::unpack),
            folded = prefs.getStringSet(p + "folded", emptySet())!!.map(::unpack).toSet(),
            hideDone = prefs.getBoolean(p + "hideDone", false),
        )
    }

    fun saveView(vault: String, v: ViewState) {
        val p = "view:$vault:"
        prefs.edit()
            .putString(p + "zoom", v.zoom?.let(::pack))
            .putStringSet(p + "folded", v.folded.map(::pack).toSet())
            .putBoolean(p + "hideDone", v.hideDone)
            .apply()
    }

    // node keys hold control characters, which the preferences' XML cannot
    private fun pack(key: String) = Base64.encodeToString(key.toByteArray(), Base64.NO_WRAP or Base64.URL_SAFE)

    private fun unpack(s: String) = String(Base64.decode(s, Base64.NO_WRAP or Base64.URL_SAFE))
}

data class ViewState(
    val zoom: String? = null,
    val folded: Set<String> = emptySet(),
    val hideDone: Boolean = false,
    /** Conflict copies unfolded this run: a copy starts folded (§10.1). */
    val unfolded: Set<String> = emptySet(),
)
