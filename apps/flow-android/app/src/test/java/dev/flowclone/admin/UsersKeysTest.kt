package dev.flowclone.admin

import dev.flowclone.data.BuildCaps
import dev.flowclone.data.DevSettings
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
            {"id":"k-1","name":"owner","admin":true,"created":1700000000,"last_seen":1700000500,"client":"flow-android 1.0.0",
             "dictations":3,"audio_ms":90000,"downloads":2,"download_bytes":2500000,"current":true},
            {"id":"k-2","name":"old","admin":false,"created":1700000000,"revoked":1700000900,"dictations":0,"audio_ms":0,"current":false}]}}"""
        val keys = (AdminJson.decode(json, AdminJson::keys) as AdminResult.Ok).value
        assertEquals(2, keys.size)
        val o = keys[0]
        assertEquals(KeyView("k-1", "owner", true, 1700000000, null, 1700000500, "flow-android 1.0.0", 3, 90000, 2, 2500000, true), o)
        assertTrue(o.active)
        assertFalse(keys[1].active); assertNull(keys[1].client); assertEquals(0L, keys[1].downloads)
        val meta = UsersKeys.meta(o)
        assertTrue(meta, meta.contains("flow-android 1.0.0") && meta.contains("3 dictations") && meta.contains("1.5 min audio"))
        assertTrue(meta, meta.contains("2 downloads (2.5 MB)"))
        assertTrue(UsersKeys.meta(keys[1]).startsWith("Revoked "))
        assertEquals("admin · this app", UsersKeys.badge(o))
        assertEquals("", UsersKeys.badge(keys[1]))
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
