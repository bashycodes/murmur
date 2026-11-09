package dev.murmur.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LoadTimingsLogTest {
    @Test fun theLineCarriesTheCoreTimingsAndWhatExplainsThem() {
        val core = """{"total_ms":5400,"models":[{"kind":"asr","language":"he","backend":"sherpa_online","ms":5300,""" +
            """"phases":[{"name":"sessions","ms":5250}]},{"kind":"llm","backend":"llama","ms":0,"reused":true}]}"""
        val o = JSONObject(LoadTimingsLog.line(core, LoadSet.Language("he"), 4, "0-7"))
        assertEquals(5400, o.getInt("total_ms"))
        assertEquals(true, o.getJSONArray("models").getJSONObject(1).getBoolean("reused"))
        assertEquals("Language(code=he)", o.getString("load_set"))
        assertEquals(4, o.getInt("threads"))
        assertEquals("0-7", o.getString("cpus"))
        assertEquals(false, JSONObject(LoadTimingsLog.line(core, LoadSet.Default, 2, null)).has("cpus"))
    }

    @Test fun aLineFromABadPayloadStillLogs() {
        val o = JSONObject(LoadTimingsLog.line("not json", LoadSet.Both, 1, null))
        assertEquals("not json", o.getString("timings"))
    }

    @Test fun cpusComeFromTheStatusFile() {
        val status = "Name:\tDefaultDispatch\nCpus_allowed:\tff\nCpus_allowed_list:\t0-3\nMems_allowed:\t1\n"
        assertEquals("0-3", LoadTimingsLog.cpusAllowed(status))
        assertNull(LoadTimingsLog.cpusAllowed("Name:\tx\n"))
    }
}
