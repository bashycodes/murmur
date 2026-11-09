package dev.flowclone.server

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** D41: the confirm dialog shows the size (the user may be on mobile data) and how files are checked. */
class ServerModelPagesTest {
    private val dirs = ModelDirs(File("/nonexistent/m"), File("/nonexistent/p"), File("/nonexistent/h"), File("/nonexistent/l"))

    @Test fun confirmShowsSizeServerAndCheck() {
        val e = ServerCatalog.entry("whisper-base")!!
        val m = model("whisper-base", ModelKind.STT, "whisper", e.pins, catalogueId = e.id, title = "Whisper base")
        val p = ServerSlots.plan(m, dirs, local = true).getOrThrow()
        val msg = ServerModelPages.confirmMessage(m, p, "10.0.0.2")
        assertTrue(msg, msg.startsWith("147 MB from your server (10.0.0.2)"))
        assertTrue(msg, msg.contains("mobile data"))
        assertTrue(msg, msg.contains("built into this app"))
        assertEquals("Download Whisper base?", ServerModelPages.confirmTitle(m))
        val c = model("c", ModelKind.STT, "whisper", listOf(f("c.bin", 2_500_000_000)))
        val cm = ServerModelPages.confirmMessage(c, ServerSlots.plan(c, dirs, local = true).getOrThrow(), "h")
        assertTrue(cm, cm.startsWith("2.5 GB") && cm.contains("your server lists"))
    }

    @Test fun rowWords() {
        val m = model("lid", ModelKind.LID, "whisper", listOf(f("a.bin", 32_000_000)))
        val planned = ServerSlots.plan(m, dirs, local = true)
        assertTrue(ServerModelPages.subtitle(m, planned).contains("Can't be used"))
        assertNull(ServerModelPages.action(null, false))
        assertEquals("Not usable", ServerModelPages.state(null, false, false))
        val w = model("w", ModelKind.STT, "whisper", listOf(f("a.bin")))
        val wp = ServerSlots.plan(w, dirs, local = true).getOrThrow()
        assertEquals("Download", ServerModelPages.action(wp, false))
        assertEquals("Downloading x… 1 / 4 MB (25 %)", ServerModelPages.progress("x", 1_000_000, 4_000_000))
        assertTrue(ServerModelPages.failed(LibraryFailure("network", "gone"), resumable = true).contains("tap Resume"))
        assertTrue(ServerModelPages.failed(LibraryFailure("mismatch", "differs"), resumable = false).startsWith("Download failed"))
    }
}
