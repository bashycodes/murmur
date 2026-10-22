package dev.flowclone.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Review S1 / S4: a stored key never travels to a host or adapter it was not saved for. */
class CloudKeyPolicyTest {
    private val groq = CloudProvider(id = "groq-1", preset = "groq", baseUrl = "https://api.groq.com/openai/v1", model = "whisper-large-v3-turbo")

    @Test fun bindingIsHostAndAdapter() {
        assertFalse(CloudKeyPolicy.bindingChanged(groq, groq.copy(model = "other", baseUrl = "https://API.groq.com/v2")))
        assertTrue(CloudKeyPolicy.bindingChanged(groq, groq.copy(baseUrl = "https://evil.example/v1")))
        assertTrue(CloudKeyPolicy.bindingChanged(groq, groq.copy(adapter = "deepgram")))
        assertFalse("new provider: nothing stored yet", CloudKeyPolicy.bindingChanged(null, groq))
    }

    @Test fun testProviderOnAnEditedFormNeedsTheTypedKey() {
        val edited = groq.copy(baseUrl = "https://evil.example/v1")
        assertEquals(CloudKeyPolicy.TestKey.Use("typed"), CloudKeyPolicy.testKey(groq, edited, " typed ", "gsk_stored"))
        val m = CloudKeyPolicy.testKey(groq, edited, "", "gsk_stored")
        assertTrue(m.toString(), m is CloudKeyPolicy.TestKey.Missing && "stored key is only sent to the saved host" in m.message)
        assertEquals(CloudKeyPolicy.TestKey.Use("gsk_stored"), CloudKeyPolicy.testKey(groq, groq.copy(model = "m2"), null, "gsk_stored"))
        assertTrue(CloudKeyPolicy.testKey(null, groq, null, "gsk_stored") is CloudKeyPolicy.TestKey.Missing)
    }

    @Test fun saveWithAChangedHostDeletesTheKeyAndAsksAgain() {
        val moved = groq.copy(baseUrl = "https://other.example/v1")
        assertEquals(CloudKeyPolicy.SaveKeys(deleteStored = true, set = null, askAgain = true), CloudKeyPolicy.onSave(groq, moved, "", hadStored = true))
        assertEquals(CloudKeyPolicy.SaveKeys(deleteStored = true, set = "k2", askAgain = false), CloudKeyPolicy.onSave(groq, moved, "k2", hadStored = true))
        assertEquals(CloudKeyPolicy.SaveKeys(deleteStored = false, set = null, askAgain = false), CloudKeyPolicy.onSave(groq, groq.copy(model = "m"), "  ", hadStored = true))
        assertEquals(CloudKeyPolicy.SaveKeys(deleteStored = false, set = "k", askAgain = false), CloudKeyPolicy.onSave(null, groq, "k", hadStored = false))
    }
}
