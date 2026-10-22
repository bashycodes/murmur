package dev.flowclone.data

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Review S2: the Kotlin base-URL rules must give the router's answers. The vectors are
 * `crates/flow-ffi/tests/fixtures/url_parse_vectors.json` (= `crates/flow-router/tests/url_parse.rs`),
 * which `crates/flow-ffi/tests/url_parse_fixture.rs` checks against the Rust parser.
 */
class UrlParseFixtureTest {
    private val f: JSONObject = JSONObject(
        requireNotNull(javaClass.classLoader!!.getResource("url_parse_vectors.json")) { "fixture not on the test classpath" }.readText(),
    )

    private fun strings(a: JSONArray) = (0 until a.length()).map { a.getString(it) }
    private fun provider(url: String) = CloudProvider(id = "p-1", baseUrl = url, model = "m")

    @Test fun rejectedUrlsAreRejected() {
        val bad = strings(f.getJSONArray("rejected"))
        assertTrue(bad.size >= 17)
        for (u in bad) {
            assertNull(u, CloudRules.parseBaseUrl(u).url)
            assertNotNull(u, CloudRules.parseBaseUrl(u).error)
            assertNull(u, CloudRules.host(u))
            assertNotNull(u, CloudRules.validate(provider(u)))
            assertEquals(u, "", provider(u).host)
        }
    }

    @Test fun hostsComeFromTheParser() {
        val a = f.getJSONArray("hosts")
        for (i in 0 until a.length()) {
            val c = a.getJSONObject(i)
            val u = c.getString("url")
            val b = requireNotNull(CloudRules.parseBaseUrl(u).url) { "$u: ${CloudRules.parseBaseUrl(u).error}" }
            assertEquals(u, c.getBoolean("https"), b.https)
            assertEquals(u, c.getString("host"), b.host)
            assertEquals(u, c.getString("host"), provider(u).host)
        }
    }

    @Test fun cleartextRuleUsesTheParsedHost() {
        for (u in strings(f.getJSONArray("cleartext_refused"))) assertNotNull(u, CloudRules.validate(provider(u)))
        for (u in strings(f.getJSONArray("cleartext_ok"))) assertNull(u, CloudRules.validate(provider(u)))
    }

    @Test fun consentUsesTheParsedHost() {
        val a = f.getJSONArray("consent")
        for (i in 0 until a.length()) {
            val c = a.getJSONObject(i)
            val p = provider(c.getString("base_url")).copy(consentHost = c.getString("consent_host"), consentAtMs = 1)
            assertEquals(p.baseUrl, c.getBoolean("ok"), p.consentOk)
        }
    }

    /** Beyond the shared vectors: WHATWG forms the Kotlin port must also get right. */
    @Test fun moreParserForms() {
        fun host(u: String) = CloudRules.parseBaseUrl(u).url?.host
        assertEquals("10.0.0.1", host("http://012.0.0.1/v1"))          // leading-0 octal
        assertEquals("10.1.0.2", host("http://10.1.2/v1"))             // last part fills 2 bytes
        assertEquals("127.0.0.1", host("http://127.1/"))
        assertNull(host("http://256.0.0.1/"))                          // overflow
        assertNull(host("http://1.2.3.4.5/"))
        assertNull(host("http://0x100000000/"))
        assertEquals("::ffff:10.0.0.1", host("http://[::ffff:a00:1]/"))
        assertEquals("fd7a:115c:a1e0::1", host("http://[FD7A:115C:A1E0:0:0:0:0:1]/"))
        assertEquals("1:0:2::", host("http://[1:0:2:0:0:0:0:0]/"))
        assertEquals("1::2:0:3", host("http://[1:0:0:0:0:2:0:3]/"))
        assertEquals("xn--bcher-kva.example", host("https://bücher.example/"))
        assertNull(host("https://exa%zzmple.com/"))
        assertNull(host("https://exa<mple.com/"))
        assertNull(host("https://example.com:99999/"))
        assertEquals(8000, CloudRules.parseBaseUrl("http://[::1]:8000/v1").url?.port)
        assertNull("default port is not reported", CloudRules.parseBaseUrl("https://api.groq.com:443/v1").url?.port)
    }
}
