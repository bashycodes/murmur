package dev.murmur.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** JVM stand-in for the Android Keystore: one AES-256 key per alias, created on first use. */
class FakeVaultKeys : VaultKeys {
    val keys = LinkedHashMap<String, SecretKey>()
    override fun key(alias: String): SecretKey = keys.getOrPut(alias) { KeyGenerator.getInstance("AES").apply { init(256) }.generateKey() }
    override fun delete(alias: String) { keys.remove(alias) }
}

/** [SecretVault] = the AES-GCM sealing used for the server token and every cloud API key (C3). */
class SecretVaultTest {
    private val keys = FakeVaultKeys()
    private val secret = "sk-LEAKCHECK-vault-0123456789"

    @Test fun sealsAndOpens() {
        val v = SecretVault("murmur_cloud_groq-1", keys)
        val sealed = v.seal(secret)
        assertFalse(sealed.contains("LEAKCHECK"))
        assertTrue("iv:ciphertext, base64", Regex("^[A-Za-z0-9+/=]+:[A-Za-z0-9+/=]+$").matches(sealed))
        assertEquals(secret, v.open(sealed))
        assertNotEquals("fresh IV every time", sealed, v.seal(secret))
    }

    @Test fun eachProviderHasItsOwnKey() {
        val a = SecretVault(SecretVault.cloudAlias("groq-1"), keys)
        val b = SecretVault(SecretVault.cloudAlias("openrouter-1"), keys)
        assertNull("another provider's vault cannot open it", b.open(a.seal(secret)))
        assertEquals(setOf("murmur_cloud_groq-1", "murmur_cloud_openrouter-1"), keys.keys.keys)
    }

    @Test fun tamperedOrDestroyedReadsAsNull() {
        val v = SecretVault("murmur_cloud_x", keys)
        val sealed = v.seal(secret)
        val ct = sealed.substringAfter(':')
        val flipped = sealed.substringBefore(':') + ":" + (if (ct[0] == 'A') 'B' else 'A') + ct.substring(1)
        assertNull(v.open(flipped))
        assertNull(v.open("garbage"))
        v.destroy()
        assertNull("key gone (provider deleted / restore to a new device)", v.open(sealed))
    }

    @Test fun tokenFormatIsUnchanged() {
        // The pre-generalisation TokenVault wrote android.util.Base64 NO_WRAP = standard alphabet, padded.
        val v = SecretVault("murmur_dev_token", keys)
        val sealed = v.seal("dev-token-0123456789abcdef")
        val iv = java.util.Base64.getDecoder().decode(sealed.substringBefore(':'))
        assertEquals("GCM IV", 12, iv.size)
        assertEquals("dev-token-0123456789abcdef", v.open(sealed))
    }
}
