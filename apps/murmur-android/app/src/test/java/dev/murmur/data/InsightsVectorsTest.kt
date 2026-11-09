package dev.flowclone.data

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Kotlin [Insights] mirror against flow-core's own golden vectors
 * (crates/flow-core/fixtures/insights/insights_vectors.json, on the test classpath via
 * sourceSets; the Rust test crates/flow-core/tests/insights_vectors.rs checks the same file),
 * so Android and Windows can't disagree on a number unnoticed.
 */
class InsightsVectorsTest {
    private val fixture by lazy {
        JSONObject(javaClass.classLoader!!.getResource("insights/insights_vectors.json")!!.readText())
    }

    private fun day(s: String) = Insights.parse(s)!!

    @Test fun summaryCasesMatchTheCore() {
        val cases = fixture.getJSONArray("cases")
        assertTrue(cases.length() >= 10)
        for (i in 0 until cases.length()) {
            val c = cases.getJSONObject(i)
            var g = Insights.Aggregates()
            val adds = c.getJSONArray("adds")
            for (j in 0 until adds.length()) {
                val a = adds.getJSONObject(j)
                g = g.add(day(a.getString("date")), a.getLong("words"), a.getLong("ms"), a.getString("app"))
            }
            if (c.optBoolean("reset")) g = g.reset()
            val got = g.summary(day(c.getString("today")), c.getDouble("typing_wpm")).toJson()
            val want = c.getJSONObject("expect")
            assertTrue("case ${c.getString("name")}:\n got $got\nwant $want", approxEq(got, want))
        }
    }

    @Test fun streakVectorsMatchTheCore() {
        val v = fixture.getJSONArray("streaks")
        for (i in 0 until v.length()) {
            val s = v.getJSONObject(i)
            val days = s.getJSONArray("days").let { a -> (0 until a.length()).map { day(a.getString(it)) } }
            assertEquals(s.toString(), s.getInt("current") to s.getInt("longest"), Insights.streaks(days, day(s.getString("today"))))
        }
    }

    @Test fun localDateVectorsMatchTheCore() {
        val v = fixture.getJSONArray("dates")
        for (i in 0 until v.length()) {
            val d = v.getJSONObject(i)
            assertEquals(d.toString(), d.getString("date"), Insights.key(Insights.dayOf(d.getLong("ts_ms"), d.getInt("offset_min"))))
        }
    }

    private fun approxEq(a: Any?, b: Any?): Boolean = when {
        a is Number && b is Number -> Math.abs(a.toDouble() - b.toDouble()) < 1e-9
        a is JSONArray && b is JSONArray -> a.length() == b.length() && (0 until a.length()).all { approxEq(a.get(it), b.get(it)) }
        a is JSONObject && b is JSONObject -> a.length() == b.length() && a.keys().asSequence().all { k -> b.has(k) && approxEq(a.get(k), b.get(k)) }
        else -> a == b
    }
}
