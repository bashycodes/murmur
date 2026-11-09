package dev.murmur.admin

import dev.murmur.data.BuildCaps
import dev.murmur.data.DevSettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D39 Users & keys rules: the natives' envelope, visibility, revoke guard, texts, no secret leaks. */
class UsersKeysTest {
    private val secret = "SECRET-users-keys-0123456789abcdefghijklmno"

    @Test fun decodesTheEnvelope() {
        val me = AdminJson.decode("""{"ok":true,"value":{"id":"k-1","name":"owner","admin":true}}""", AdminJson::whoami)
        assertEquals(AdminResult.Ok(WhoAmI("k-1", "owner", true)), me)
        val err = AdminJson.decode("""{"ok":false,"code":"forbidden","message":"admin key required"}""", AdminJson::whoami)
        assertEquals(AdminResult.Err("forbidden", "admin key required"), err)
        assertEquals(AdminCode.BAD_RESPONSE, (AdminJson.decode("nope", AdminJson::whoami) as AdminResult.Err).code)
        assertEquals(AdminCode.BAD_RESPONSE, (AdminJson.decode("""{"ok":false}""", AdminJson::whoami) as AdminResult.Err).code)
    }

    @Test fun decodesKeysWithUsageAndDownloads() {
        val json = """{"ok":true,"value":{"keys":[
            {"id":"k-1","name":"owner","admin":true,"created":1700000000,"last_used":1700000400,"last_connected":1700000500,"last_seen":1700000500,"client":"murmur-android 1.0.0",
             "dictations":3,"audio_ms":90000,"downloads":2,"download_bytes":2500000,"current":true},
            {"id":"k-2","name":"old","admin":false,"created":1700000000,"revoked":1700000900,"dictations":0,"audio_ms":0,"current":false}]}}"""
        val keys = (AdminJson.decode(json, AdminJson::keys) as AdminResult.Ok).value
        assertEquals(2, keys.size)
        val o = keys[0]
        assertEquals(KeyView("k-1", "owner", true, 1700000000, null, lastUsed = 1700000400, lastConnected = 1700000500,
            client = "murmur-android 1.0.0", dictations = 3, audioMs = 90000, downloads = 2, downloadBytes = 2500000, current = true), o)
        assertTrue(o.active)
        assertFalse(keys[1].active); assertNull(keys[1].client); assertEquals(0L, keys[1].downloads)
        val meta = UsersKeys.meta(o)
        assertTrue(meta, meta.contains("murmur-android 1.0.0") && meta.contains("3 dictations") && meta.contains("1.5 min audio"))
        assertTrue(meta, meta.contains("2 downloads (2.5 MB)"))
        assertTrue(UsersKeys.meta(keys[1]).startsWith("Revoked "))
        assertEquals("admin · this app", UsersKeys.badge(o))
        assertEquals("", UsersKeys.badge(keys[1]))
    }

    @Test fun lastConnectedFallsBackToAnOlderServersLastSeen() {
        val old = AdminJson.key(org.json.JSONObject("""{"id":"k-1","last_seen":5}"""))
        assertEquals(null to 5L, old.lastUsed to old.lastConnected)
        val new = AdminJson.key(org.json.JSONObject("""{"id":"k-1","last_used":3,"last_connected":7,"last_seen":6}"""))
        assertEquals(3L to 7L, new.lastUsed to new.lastConnected)
        val none = AdminJson.key(org.json.JSONObject("""{"id":"k-1","last_used":null,"last_connected":null}"""))
        assertEquals(null to null, none.lastUsed to none.lastConnected)
    }

    @Test fun lastUsedIsUnknownForAKeyWithUsageButNoTime() {
        assertEquals("unknown", UsersKeys.lastUsed(KeyView("k", "phone", dictations = 2)))
        assertEquals("unknown", UsersKeys.lastUsed(KeyView("k", "phone", downloads = 1)))
        assertEquals("never", UsersKeys.lastUsed(KeyView("k", "phone", lastConnected = 9)))
        val used = UsersKeys.lastUsed(KeyView("k", "phone", dictations = 2, lastUsed = 1700000400))
        assertFalse(used, used == "unknown" || used == "never")
        val meta = UsersKeys.meta(KeyView("k", "phone", dictations = 2, lastConnected = 1700000500))
        assertTrue(meta, meta.contains("Last used unknown · last connected ") && !meta.contains("last connected never"))
        assertTrue(UsersKeys.meta(KeyView("k", "phone")).contains("Last used never · last connected never"))
    }

    @Test fun createdKeyNeverPrintsItsSecret() {
        val json = """{"ok":true,"value":{"key":{"id":"k-3","name":"phone","admin":false,"created":1},"secret":"$secret"}}"""
        val c = (AdminJson.decode(json, AdminJson::created) as AdminResult.Ok).value
        assertEquals(secret, c.secret)
        assertEquals("phone", c.key.name)
        assertFalse(c.toString().contains(secret))
        assertFalse(AdminResult.Ok(c).toString().contains(secret))
    }

    @Test fun entryOnlyForAnAdminKeyInANetworkBuild() {
        assertTrue(UsersKeys.showEntry(AdminResult.Ok(WhoAmI("k", "o", true))))
        assertFalse(UsersKeys.showEntry(AdminResult.Ok(WhoAmI("k", "phone", false))))
        assertFalse(UsersKeys.showEntry(AdminResult.Err(AdminCode.FORBIDDEN)))
        assertFalse(UsersKeys.showEntry(AdminResult.Err(AdminCode.CONNECT_FAILED)))
        assertFalse(UsersKeys.showEntry(null))
        val s = DevSettings(allowServer = true, allowClient = true, host = "100.64.0.10", token = "t".repeat(32))
        assertTrue(UsersKeys.available(BuildCaps(local = true, remote = true), s))
        assertFalse("local flavour", UsersKeys.available(BuildCaps(local = true, remote = false), s))
        assertFalse("no token", UsersKeys.available(BuildCaps(local = true, remote = true), s.copy(token = "")))
        assertFalse("no host", UsersKeys.available(BuildCaps(local = false, remote = true), s.copy(host = "")))
        assertTrue(UsersKeys.clientFor(BuildCaps(local = false, remote = true), s) is NativeAdminApi)
        assertNull("local flavour: no client, no page", UsersKeys.clientFor(BuildCaps(local = true, remote = false), s))
        assertTrue(UsersKeys.mustLeave(AdminCode.FORBIDDEN))
        assertTrue(UsersKeys.mustLeave(AdminCode.UNAUTHORIZED))
        assertFalse(UsersKeys.mustLeave(AdminCode.CONNECT_FAILED))
    }

    @Test fun lastAdminCannotBeRevokedAndRevokedSortLast() {
        val owner = KeyView("k-1", "owner", admin = true)
        val phone = KeyView("k-2", "phone")
        val gone = KeyView("k-0", "old-admin", admin = true, revoked = 5)
        val keys = listOf(gone, owner, phone)
        assertFalse(UsersKeys.canRevoke(owner, keys))
        assertTrue(UsersKeys.canRevoke(phone, keys))
        assertFalse(UsersKeys.canRevoke(gone, keys))
        assertTrue(UsersKeys.canRevoke(owner, keys + KeyView("k-4", "laptop", admin = true)))
        assertEquals(listOf("owner", "phone", "old-admin"), UsersKeys.sorted(keys).map { it.name })
    }

    @Test fun texts() {
        assertEquals("This is the last admin key: create another admin key first.", UsersKeys.errorText(AdminCode.LAST_ADMIN, "x"))
        assertEquals("Key name \"a b\": bad", UsersKeys.errorText(AdminCode.BAD_REQUEST, "key name \"a b\": bad"))
        assertEquals("Weird", UsersKeys.errorText("weird", ""))
        assertNotNull(UsersKeys.nameError("  "))
        assertNotNull(UsersKeys.nameError("x".repeat(65)))
        assertNull(UsersKeys.nameError("phone"))
        assertTrue(UsersKeys.revokeMessage(KeyView("k", "me", current = true)).contains("This is the key this app uses"))
        assertFalse(UsersKeys.revokeMessage(KeyView("k", "phone")).contains("this app uses"))
    }
}
