package dev.murmur.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The shared model-fit vectors (crates/murmur-core/fixtures/model-fit/vectors.json, on the test
 * classpath via sourceSets). murmur-core's tests/model_fit_vectors.rs checks the same file, so
 * [ModelFit] and the Rust table agree (D116).
 */
class ModelFitVectorsTest {
    private val doc = JSONObject(javaClass.classLoader!!.getResource("model-fit/vectors.json")!!.readText())

    private fun machine(v: JSONObject) = ModelFit.Machine(
        totalRam = v.getLong("ram"),
        freeDisk = if (v.isNull("free")) null else v.getLong("free"),
        gpuLlm = v.getBoolean("gpu_llm"),
        gpuWhisper = v.getBoolean("gpu_whisper"),
    )

    @Test fun recommendVectors() {
        val list = doc.getJSONArray("recommend")
        val fails = mutableListOf<String>()
        for (i in 0 until list.length()) {
            val v = list.getJSONObject(i)
            val want = v.getJSONObject("expect")
            val p = ModelFit.recommend(machine(v))
            val wantCleanup = if (want.isNull("cleanup")) null else want.getString("cleanup")
            if (p.speech.key != want.getString("speech") || p.cleanup?.key != wantCleanup || p.hebrew.key != want.getString("hebrew"))
                fails += "$v: got $p"
        }
        assertTrue(fails.joinToString("\n"), fails.isEmpty())
        assertTrue(list.length() > 20)
    }

    @Test fun fitVectors() {
        val list = doc.getJSONArray("fit")
        val fails = mutableListOf<String>()
        for (i in 0 until list.length()) {
            val v = list.getJSONObject(i)
            val want = v.getJSONObject("expect")
            val model = ModelFit.Model.of(v.getString("model")) ?: error("unknown model ${v.getString("model")}")
            val f = ModelFit.fit(model, v.getLong("size"), v.getBoolean("installed"), machine(v))
            val reasons = want.getJSONArray("reasons").let { a -> List(a.length()) { a.getString(it) } }
            if (f.fits != want.getBoolean("fits") || f.reasons != reasons) fails += "$v: got $f"
        }
        assertTrue(fails.joinToString("\n"), fails.isEmpty())
        // Every model of the table has a vector.
        val keys = (0 until list.length()).map { list.getJSONObject(it).getString("model") }.toSet()
        assertEquals(ModelFit.Model.entries.map { it.key }.toSet(), keys)
    }
}
