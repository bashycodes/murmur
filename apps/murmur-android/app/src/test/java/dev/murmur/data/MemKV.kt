package dev.flowclone.data

/** In-memory [KeyValue] (SharedPreferences twin) for the JVM tests. */
internal class MemKV : KeyValue {
    val m = HashMap<String, Any?>()
    override fun getString(k: String) = m[k] as String?
    override fun getBoolean(k: String, d: Boolean) = m[k] as Boolean? ?: d
    override fun getLong(k: String, d: Long) = m[k] as Long? ?: d
    override fun contains(k: String) = k in m
    override fun edit(block: KeyValue.Editor.() -> Unit) = object : KeyValue.Editor {
        override fun putString(k: String, v: String?) { if (v == null) m.remove(k) else m[k] = v }
        override fun putBoolean(k: String, v: Boolean) { m[k] = v }
        override fun putLong(k: String, v: Long) { m[k] = v }
        override fun remove(k: String) { m.remove(k) }
        override fun clear() = m.clear()
    }.block()
}
