package dev.flowclone.data

import dev.flowclone.core.ParakeetDecoding
import dev.flowclone.data.DevSettingsForm.Field
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** §5 Developer settings: validation, flavour defaults/locks, persistence, unlock. */
class DevSettingsTest {
    private fun form(caps: BuildCaps = BuildCaps.ALL, f: DevSettingsForm.Input.() -> DevSettingsForm.Input = { this }) =
        DevSettingsForm.validate(DevSettingsForm.Input.of(DevSettings.defaults(caps)).f(), caps)

    // ---- host / port (twin of flow_router::net::validate_host's table) ----

    /** 2026-10-24: debug-log `route_order` = the engines the router tries, in order. */
    @Test fun routeOrderWireListsTheEnabledRoutesInOrder() {
        val d = DevSettings.defaults(BuildCaps.ALL).copy(allowServer = true, allowClient = true, host = "100.64.0.10")
        assertEquals("server,client", d.routeOrderWire(BuildCaps.ALL))
        assertEquals("client,server", d.copy(routeOrder = listOf(DevSettings.Route.CLIENT, DevSettings.Route.SERVER)).routeOrderWire(BuildCaps.ALL))
        assertEquals("client", d.copy(allowServer = false).routeOrderWire(BuildCaps.ALL))
        assertEquals("", d.copy(allowServer = false, allowClient = false).routeOrderWire(BuildCaps.ALL))
        assertEquals("client", d.routeOrderWire(BuildCaps.LOCAL))
    }

    @Test fun validHosts() {
        for (h in listOf("192.168.1.2", "0.0.0.0", "desktop", "box.tail1.ts.net", "::1", "[fd7a:115c:a1e0::1]", "a-b.example.com", "fe80::1", "::ffff:192.168.1.1", "1::", "2001:db8:0:0:0:0:2:1")) {
            assertTrue(h, HostRules.validate(h).isSuccess)
        }
        assertEquals("::1", HostRules.validate(" [::1] ").getOrThrow())
        assertEquals("100.64.0.10", HostRules.validate(" 100.64.0.10 ").getOrThrow())
    }

    @Test fun invalidHosts() {
        for (h in listOf("", "ws://x", "x/y", "a b", "192.168.01.2", "256.1.1.1", "1.2.3", "host:7313", "-a.com", "a-.com", "a..b", "a_b.com", "[desktop]",
            "1.2.3.4.5", "::1::", "12345::", "fe80::1%wlan0", ":::", "1:2:3:4:5:6:7:8:9", "a".repeat(64), List(5) { "a".repeat(60) }.joinToString(".") + ".com")) {
            assertTrue("'$h' should be invalid", HostRules.validate(h).isFailure)
        }
    }

    @Test fun classificationTable() {
        val t = mapOf(
            "127.0.0.1" to HostRules.AddrClass.LOOPBACK, "::1" to HostRules.AddrClass.LOOPBACK, "[::1]" to HostRules.AddrClass.LOOPBACK,
            "localhost" to HostRules.AddrClass.LOOPBACK, "10.1.2.3" to HostRules.AddrClass.LAN, "172.16.0.1" to HostRules.AddrClass.LAN,
            "172.32.0.1" to HostRules.AddrClass.PUBLIC, "192.168.1.20" to HostRules.AddrClass.LAN, "169.254.1.1" to HostRules.AddrClass.LAN,
            "100.64.0.10" to HostRules.AddrClass.TAILNET, "100.127.255.255" to HostRules.AddrClass.TAILNET, "100.128.0.1" to HostRules.AddrClass.PUBLIC,
            "100.63.0.1" to HostRules.AddrClass.PUBLIC, "fd7a:115c:a1e0::1" to HostRules.AddrClass.TAILNET, "fd00::1" to HostRules.AddrClass.LAN,
            "fe80::1" to HostRules.AddrClass.LAN, "2001:4860:4860::8888" to HostRules.AddrClass.PUBLIC, "::ffff:192.168.1.1" to HostRules.AddrClass.LAN,
            "::ffff:8.8.8.8" to HostRules.AddrClass.PUBLIC, "8.8.8.8" to HostRules.AddrClass.PUBLIC, "box.tail1234.ts.net" to HostRules.AddrClass.TAILNET,
            "desktop.local" to HostRules.AddrClass.LOCAL_NAME, "desktop" to HostRules.AddrClass.LOCAL_NAME, "example.com" to HostRules.AddrClass.PUBLIC,
            "ts.net.example.com" to HostRules.AddrClass.PUBLIC,
        )
        for ((h, want) in t) assertEquals(h, want, HostRules.classify(h))
    }

    @Test fun portMustBe1To65535() {
        for (p in listOf("0", "65536", "-1", "abc", "", "1.5")) assertTrue(p, Field.PORT in form { copy(port = p) }.errors)
        for (p in listOf("1", "7313", "65535", " 443 ")) assertTrue(p, form { copy(port = p) }.canSave)
        assertEquals(443, form { copy(port = " 443 ") }.settings!!.port)
    }

    @Test fun invalidHostBlocksSaveWithInlineError() {
        val r = form { copy(host = "http://10.0.0.2:7313") }
        assertFalse(r.canSave)
        assertNull(r.settings)
        assertEquals("enter the host only (no scheme or path)", r.errors[Field.HOST])
    }

    @Test fun ipv6IsStoredWithoutBrackets() = assertEquals("fd7a:115c:a1e0::5", form { copy(host = "[fd7a:115c:a1e0::5]") }.settings!!.host)

    // ---- TLS / pin / token / timeouts ----

    @Test fun publicHostForcesAndLocksTls() {
        val r = form { copy(host = "flow.example.com", tls = false) }
        assertTrue(r.tlsLocked)
        assertTrue(r.settings!!.tls)
        val tailnet = form { copy(host = "100.64.0.10", tls = false, token = "x".repeat(16)) }
        assertFalse(tailnet.tlsLocked)
        assertFalse(tailnet.settings!!.tls)
        assertTrue(DevSettingsForm.PLAINTEXT in tailnet.warnings)
    }

    @Test fun pinIsEmptyOr64HexColonsStripped() {
        val hex = "3F:2A" + "9c0b7e4d1f6a8b5c2e9d0a7f4b1c6e3d8a5f2b9c0e7d4a1f6b3c8e5d2a9f"
        val ok = form { copy(host = "10.0.0.2", tls = true, pin = hex) }
        assertEquals("3f2a9c0b7e4d1f6a8b5c2e9d0a7f4b1c6e3d8a5f2b9c0e7d4a1f6b3c8e5d2a9f", ok.settings!!.tlsPinSha256)
        assertTrue(Field.PIN in form { copy(tls = true, pin = "abc") }.errors)
        assertTrue(Field.PIN in form { copy(tls = true, pin = "z".repeat(64)) }.errors)
        assertEquals("pin only applies with TLS", "", form { copy(tls = false, pin = hex) }.settings!!.tlsPinSha256)
    }

    @Test fun tokenEmptyOrAtLeast16() {
        assertTrue(form { copy(token = "") }.canSave)
        assertTrue(Field.TOKEN in form { copy(token = "short-token") }.errors)
        assertTrue(form { copy(token = "0123456789abcdef") }.canSave)
    }

    @Test fun timeoutRanges() {
        assertTrue(Field.CONNECT_TIMEOUT in form { copy(connectTimeoutMs = "99") }.errors)
        assertTrue(Field.CONNECT_TIMEOUT in form { copy(connectTimeoutMs = "10001") }.errors)
        assertTrue(Field.COMMIT_TIMEOUT in form { copy(commitTimeoutBaseMs = "499") }.errors)
        assertTrue(Field.COMMIT_TIMEOUT in form { copy(commitTimeoutBaseMs = "60001") }.errors)
        assertTrue(Field.HEALTH_TTL in form { copy(healthTtlS = "4") }.errors)
        assertTrue(Field.HEALTH_TTL in form { copy(healthTtlS = "601") }.errors)
        assertTrue(form { copy(connectTimeoutMs = "100", commitTimeoutBaseMs = "60000", healthTtlS = "600") }.canSave)
    }

    // ---- toggles × flavour ----

    @Test fun flavourDefaults() {
        assertEquals(DevSettings(allowServer = false, allowClient = true), DevSettings.defaults(BuildCaps.LOCAL))
        assertEquals(DevSettings(allowServer = true, allowClient = true), DevSettings.defaults(BuildCaps.ALL))
        assertEquals(DevSettings(allowServer = true, allowClient = false), DevSettings.defaults(BuildCaps.REMOTE))
    }

    @Test fun localBuildCanNeverEnableTheServer() {
        val r = form(BuildCaps.LOCAL) { copy(allowServer = true, host = "100.64.0.10") }
        assertFalse(r.settings!!.allowServer)
        val json = DevSettings(allowServer = true, allowClient = true, host = "x").routerConfigFor(BuildCaps.LOCAL)
        assertTrue(json, json.contains("\"allow_server\":false"))
    }

    @Test fun remoteBuildCanNeverEnableTheClient() {
        assertFalse(form(BuildCaps.REMOTE) { copy(allowClient = true) }.settings!!.allowClient)
        assertFalse(DevSettings(allowServer = true, allowClient = true).effective(BuildCaps.REMOTE).allowClient)
    }

    @Test fun bothOffIsSavableButFlaggedClearly() {
        val r = form { copy(allowServer = false, allowClient = false) }
        assertTrue(r.canSave)
        assertTrue(r.settings!!.bothOff)
        assertTrue(DevSettingsForm.BOTH_OFF in r.warnings)
        // In a local build "server off" is the only option, so client off alone is "both off".
        assertTrue(DevSettingsForm.BOTH_OFF in form(BuildCaps.LOCAL) { copy(allowClient = false) }.warnings)
    }

    @Test fun serverOnWithoutHostWarnsNotConfigured() {
        assertTrue(DevSettingsForm.NO_HOST in form { copy(host = "") }.warnings)
        assertFalse(DevSettingsForm.NO_HOST in form(BuildCaps.LOCAL) { copy(host = "") }.warnings)
    }

    @Test fun toStringNeverLeaksTheToken() {
        val s = DevSettings(allowServer = true, allowClient = true, token = "super-secret-token-123")
        assertFalse(s.toString().contains("super-secret"))
    }

    // ---- persistence ----

    private class MemKV : KeyValue {
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

    /** Reversible stand-in for the Keystore box; proves only ciphertext is stored. */
    private object RotBox : SecretBox {
        override fun seal(plain: String) = "sealed:" + plain.reversed()
        override fun open(sealed: String) = sealed.removePrefix("sealed:").reversed().takeIf { sealed.startsWith("sealed:") }
    }

    @Test fun storeRoundTripsEveryFieldAndSealsTheToken() {
        val kv = MemKV()
        val store = DevStore(kv, BuildCaps.ALL, RotBox)
        assertEquals(DevSettings.defaults(BuildCaps.ALL), store.load())
        val s = DevSettings(
            allowServer = true, allowClient = false, routeOrder = DevSettings.orderFor(DevSettings.Prefer.CLIENT), host = "fd7a:115c:a1e0::5", port = 9000,
            tls = true, tlsPinSha256 = "a".repeat(64), token = "0123456789abcdef-tok", connectTimeoutMs = 1200,
            commitTimeoutBaseMs = 5000, healthTtlS = 60, keepLocalWarm = false, showEngineBadge = true,
        )
        store.save(s)
        assertEquals(s, store.load())
        assertFalse("plain token never stored", kv.m.values.any { it == "0123456789abcdef-tok" })
        assertTrue(kv.m["server.token_sealed"].toString().startsWith("sealed:"))
        store.save(s.copy(token = ""))
        assertFalse("clearing the token removes the ciphertext", kv.contains("server.token_sealed"))
    }

    @Test fun storeDropsTogglesTheFlavourCannotHonour() {
        val kv = MemKV()
        DevStore(kv, BuildCaps.ALL, RotBox).save(DevSettings(allowServer = true, allowClient = true, host = "10.0.0.2"))
        // Same prefs after switching to the local flavour (same applicationId):
        val local = DevStore(kv, BuildCaps.LOCAL, RotBox).load()
        assertFalse(local.allowServer); assertTrue(local.allowClient); assertEquals("10.0.0.2", local.host)
        val remote = DevStore(kv, BuildCaps.REMOTE, RotBox).load()
        assertTrue(remote.allowServer); assertFalse(remote.allowClient)
    }

    /** D68: no override by default (the dictionary decides); an override persists and clears. */
    @Test fun parakeetDecodingOverrideDefaultsUnsetAndPersists() {
        val kv = MemKV()
        val store = DevStore(kv, BuildCaps.LOCAL, RotBox)
        assertNull(store.parakeetDecodingOverride)
        store.parakeetDecodingOverride = ParakeetDecoding.AUTO
        assertEquals(ParakeetDecoding.AUTO, DevStore(kv, BuildCaps.LOCAL, RotBox).parakeetDecodingOverride)
        store.parakeetDecodingOverride = null
        assertNull(DevStore(kv, BuildCaps.LOCAL, RotBox).parakeetDecodingOverride)
    }

    @Test fun debugLogDefaultsOffInEveryFlavourAndPersists() {
        for (caps in listOf(BuildCaps.LOCAL, BuildCaps.ALL, BuildCaps.REMOTE)) {
            val kv = MemKV()
            val store = DevStore(kv, caps, RotBox)
            assertFalse(store.debugLog)
            store.debugLog = true
            assertTrue(DevStore(kv, caps, RotBox).debugLog)
            store.save(DevSettings.defaults(caps))
            assertTrue("a settings save leaves the toggle alone", store.debugLog)
            store.reset()
            assertFalse("reset to defaults turns it off", store.debugLog)
        }
    }

    @Test fun unreadableTokenLoadsAsEmpty() {
        val kv = MemKV().apply { m["server.token_sealed"] = "garbage" }
        assertEquals("", DevStore(kv, BuildCaps.ALL, RotBox).load().token)
    }

    @Test fun resetKeepsUnlockFlag() {
        val kv = MemKV()
        val store = DevStore(kv, BuildCaps.LOCAL, RotBox)
        store.devUnlocked = true
        store.save(DevSettings(allowServer = false, allowClient = false))
        store.reset()
        assertTrue(store.devUnlocked)
        assertEquals(DevSettings.defaults(BuildCaps.LOCAL), store.load())
    }

    @Test fun routeOrderDrivesPreferAndToleratesUnknownRoutes() {
        assertEquals(DevSettings.Prefer.SERVER, DevSettings(true, true).prefer)
        assertEquals(DevSettings.Prefer.CLIENT, DevSettings(true, true, routeOrder = DevSettings.orderFor(DevSettings.Prefer.CLIENT)).prefer)
        assertEquals(listOf(DevSettings.Route.CLIENT, DevSettings.Route.SERVER), DevSettings.parseOrder("cloud_openai,client"))
        assertEquals(DevSettings.DEFAULT_ORDER, DevSettings.parseOrder(null))
    }

    // ---- access ----

    @Test fun visibilityPerFlavour() {
        assertTrue(DevUnlock.visible(BuildCaps.ALL, false))
        assertTrue(DevUnlock.visible(BuildCaps.REMOTE, false))
        assertFalse(DevUnlock.visible(BuildCaps.LOCAL, false))
        assertTrue(DevUnlock.visible(BuildCaps.LOCAL, true))
    }

    @Test fun sevenQuickTapsUnlock() {
        var t = 0L
        val u = DevUnlock({ t })
        repeat(6) { assertFalse(u.tap()); t += 300 }
        assertTrue(u.tap())
        // A long pause restarts the count.
        repeat(5) { u.tap(); t += 300 }
        t += 10_000
        repeat(6) { assertFalse(u.tap()); t += 300 }
        assertTrue(u.tap())
    }

    // ---- D24 connect timeout ----

    @Test fun connectTimeoutDefaultIs2500AndAStored800Migrates() {
        assertEquals(2500L, DevSettings(allowServer = true, allowClient = true).connectTimeoutMs)
        assertEquals(2500L, DevSettings.migrateConnectTimeout(null, migrated = false))
        assertEquals("old default", 2500L, DevSettings.migrateConnectTimeout(800, migrated = false))
        assertEquals("user value", 1200L, DevSettings.migrateConnectTimeout(1200, migrated = false))
        assertEquals("800 chosen after the migration", 800L, DevSettings.migrateConnectTimeout(800, migrated = true))
        val kv = MemKV().apply { m["server.connect_timeout_ms"] = 800L }
        val store = DevStore(kv, BuildCaps.ALL, RotBox)
        assertEquals(2500L, store.load().connectTimeoutMs)
        assertEquals("written back once", 2500L, kv.m["server.connect_timeout_ms"])
        store.save(store.load().copy(connectTimeoutMs = 800))
        assertEquals("a deliberate 800 survives", 800L, store.load().connectTimeoutMs)
    }
}
