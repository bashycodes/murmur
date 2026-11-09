package dev.murmur.server

import dev.murmur.core.AsrEngine
import dev.murmur.data.HebrewModels
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** D41: install = verified files in their slot, resumable, nothing kept on cancel / mismatch. */
class ServerInstallerTest {
    private val enc = ByteArray(200) { (it * 7).toByte() }
    private val tok = "a 0\nb 1\n".toByteArray()
    private val files = mapOf("encoder.onnx" to enc, "decoder.onnx" to byteArrayOf(1, 2), "joiner.onnx" to byteArrayOf(3), "tokens.txt" to tok)
    private val listed = files.map { (n, b) -> LibraryFile(n, b.size.toLong(), sha(b)) }

    @Test fun anInterruptedDownloadResumesWhereItStopped() {
        val (root, dirs) = tempDirs()
        try {
            val m = model("he1", ModelKind.HEBREW, "parakeet", listed, title = "Hebrew one")
            val plan = ServerSlots.plan(m, dirs, local = true).getOrThrow()
            val t = FakeTransport(files).apply { cutAfter = 120 }
            var last = 0L
            val first = ServerInstaller(t).install(m, plan, { d, _ -> last = d }, { false })
            assertEquals(ServerInstaller.Outcome.Failed(LibraryFailure("network", "encoder.onnx stopped"), resumable = true), first)
            assertEquals(120L, File(plan.target, "encoder.onnx.part").length())
            assertEquals(120L, plan.bytesOnDisk)
            assertFalse(plan.installed)
            assertEquals("Resume", ServerModelPages.action(plan, inUse = false))
            assertEquals(null, HebrewModels.installed(plan.target))

            val second = ServerInstaller(t).install(m, plan, { d, _ -> last = d }, { false })
            assertTrue(second is ServerInstaller.Outcome.Done)
            assertEquals("encoder.onnx" to 120L, t.requests[1])
            assertEquals(plan.bytes, last)
            assertTrue(plan.installed)
            assertTrue(enc.contentEquals(File(plan.target, "encoder.onnx").readBytes()))
            assertFalse(File(plan.target, "encoder.onnx.part").exists())
            // The custom Hebrew model is usable by the engine: its engine file names it.
            assertEquals(HebrewModels.Installed(AsrEngine.PARAKEET, "Hebrew one"), HebrewModels.installed(plan.target))
            // Use again (re-verify): no bytes.
            val n = t.requests.size
            assertTrue(ServerInstaller(t).install(m, plan, { _, _ -> }, { false }) is ServerInstaller.Outcome.Done)
            assertEquals(n, t.requests.size)
        } finally { root.deleteRecursively() }
    }

    @Test fun aMismatchOrACancelKeepsNothing() {
        val (root, dirs) = tempDirs()
        try {
            val bad = listed.map { if (it.name == "tokens.txt") it.copy(sha256 = "0".repeat(64)) else it }
            val m = model("p1", ModelKind.STT, "parakeet", bad)
            val plan = ServerSlots.plan(m, dirs, local = true).getOrThrow()
            val r = ServerInstaller(FakeTransport(files)).install(m, plan, { _, _ -> }, { false })
            assertEquals("mismatch", (r as ServerInstaller.Outcome.Failed).failure.kind)
            assertFalse(r.resumable)
            assertFalse(File(plan.target, "tokens.txt.part").exists())
            assertFalse(File(plan.target, "tokens.txt").exists())

            val m2 = model("p2", ModelKind.STT, "parakeet", listed)
            val plan2 = ServerSlots.plan(m2, dirs, local = true).getOrThrow()
            var seen = 0L
            val c = ServerInstaller(FakeTransport(files)).install(m2, plan2, { d, _ -> seen = d }, { seen >= 50 })
            assertEquals(ServerInstaller.Outcome.Cancelled, c)
            assertFalse(File(plan2.target, "encoder.onnx.part").exists())
            assertEquals(0L, plan2.bytesOnDisk)
        } finally { root.deleteRecursively() }
    }

    @Test fun aSingleFileModelIsJustItsFile() {
        val (root, dirs) = tempDirs()
        try {
            val gguf = ByteArray(64) { it.toByte() }
            val m = model("my-llm", ModelKind.LLM, "gguf", listOf(LibraryFile("q.gguf", 64, sha(gguf)), LibraryFile("README.md", 3, sha("abc".toByteArray()))))
            val plan = ServerSlots.plan(m, dirs, local = true).getOrThrow()
            val t = FakeTransport(mapOf("q.gguf" to gguf))
            assertTrue(ServerInstaller(t).install(m, plan, { _, _ -> }, { false }) is ServerInstaller.Outcome.Done)
            assertEquals(listOf("my-llm-q.gguf"), dirs.llm.list()!!.toList())
            assertEquals(listOf("q.gguf"), t.requests.map { it.first })
        } finally { root.deleteRecursively() }
    }
}
