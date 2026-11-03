package dev.flowclone.server

import dev.flowclone.data.HebrewModels
import dev.flowclone.data.ModelPages
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** D41: kind + engine → slot, mirroring apps/flow-windows/src/server_library.rs `plan`. */
class ServerSlotsTest {
    private val dirs = ModelDirs(File("/m"), File("/p"), File("/h"), File("/l"))
    private fun plan(m: LibraryModel, local: Boolean = true) = ServerSlots.plan(m, dirs, local)
    private fun reason(m: LibraryModel, local: Boolean = true) = plan(m, local).exceptionOrNull()!!.message!!

    @Test fun customModelsPickTheirSlotByKindAndEngine() {
        assertEquals(Slot.STT_WHISPER, plan(model("w", ModelKind.STT, "whisper", listOf(f("ggml-x.bin"), f("README.md")))).getOrThrow().slot)
        assertEquals(Slot.STT_PARAKEET, plan(model("p", ModelKind.STT, "parakeet", SHERPA)).getOrThrow().slot)
        assertEquals(Slot.HEBREW_ZIPFORMER, plan(model("z", ModelKind.HEBREW, "sherpa_online", SHERPA)).getOrThrow().slot)
        assertEquals(Slot.HEBREW_PARAKEET, plan(model("hp", ModelKind.HEBREW, "parakeet", SHERPA)).getOrThrow().slot)
        assertEquals(Slot.LLM, plan(model("q", ModelKind.LLM, "gguf", listOf(f("q.gguf")))).getOrThrow().slot)
    }

    @Test fun unusableModelsSayWhy() {
        val noEngine = reason(model("x", ModelKind.STT, null, SHERPA))
        assertTrue(noEngine, noEngine.contains("no engine"))
        for (e in ServerSlots.ENGINES) assertTrue(noEngine, noEngine.contains("\"$e\""))
        assertTrue(reason(model("x", ModelKind.STT, "sherpa_online", SHERPA)).contains("only as the Hebrew model"))
        assertTrue(reason(model("x", ModelKind.HEBREW, "whisper", listOf(f("a.bin")))).contains("sherpa_online or parakeet"))
        assertTrue(reason(model("x", ModelKind.LID, "whisper", listOf(f("a.bin")))).contains("language-detection"))
        assertTrue(reason(model("x", ModelKind.VAD, "silero", listOf(f("a.onnx")))).contains("voice-activity"))
        assertTrue(reason(model("x", ModelKind.LLM, "mlx", listOf(f("a.gguf")))).contains("can't load the mlx engine"))
        assertTrue(reason(model("x", null, "whisper", listOf(f("a.bin")))).contains("doesn't know the kind"))
        assertTrue(reason(model("x", ModelKind.STT, "whisper", listOf(f("a.bin"), f("b.bin")))).contains("exactly one .bin"))
        assertTrue(reason(model("x", ModelKind.LLM, "gguf", listOf(f("a.bin")))).contains("exactly one .gguf"))
        assertTrue(reason(model("x", ModelKind.STT, "parakeet", SHERPA.drop(1))).contains("encoder, decoder and joiner"))
        // Folder models need the sherpa files at the top level.
        assertTrue(reason(model("x", ModelKind.STT, "parakeet", SHERPA.map { f("sub/${it.name}") })).contains("encoder"))
        assertTrue(reason(model("w", ModelKind.STT, "whisper", listOf(f("a.bin"))), local = false).contains("no on-device engine"))
    }

    @Test fun customFilesLandInTheirSlotFolders() {
        val w = plan(model("my-w", ModelKind.STT, "whisper", listOf(f("sub/ggml-x.bin"), f("README.md")))).getOrThrow()
        assertEquals(File("/m/my-w-ggml-x.bin"), w.target)
        assertEquals(listOf("sub/ggml-x.bin"), w.fetches.map { it.file.name })
        assertEquals(File("/m/my-w-ggml-x.bin.part"), w.fetches[0].part)
        assertEquals(File("/l/qwen-x.gguf"), plan(model("qwen", ModelKind.LLM, "gguf", listOf(f("qwen-x.gguf")))).getOrThrow().target)
        val h = plan(model("he1", ModelKind.HEBREW, "sherpa_online", SHERPA + f("onnx/extra.bin"))).getOrThrow()
        assertEquals(File("/h/server-he1"), h.target)
        assertEquals(File("/h/server-he1/onnx/extra.bin"), h.fetches.last().dest)
        assertNull(h.catalogue)
        assertEquals(50L, h.bytes)
    }

    /** A tag with just a catalogue id (no engine) is that catalogue model: the phone's default English model and the recommended whisper files. */
    @Test fun aCatalogueIdAloneIsEnough() {
        for ((id, slot) in listOf("parakeet-unified-en-0.6b-int8" to Slot.STT_PARAKEET, "whisper-base-en-q5_1" to Slot.STT_WHISPER)) {
            val e = ServerCatalog.entry(id)!!
            val p = plan(model("srv-$id", ModelKind.STT, null, e.pins, catalogueId = id)).getOrThrow()
            assertEquals(slot, p.slot)
            assertEquals(e, p.catalogue)
        }
        assertEquals(File("/m/ggml-base.en-q5_1.bin"), plan(model("w", ModelKind.STT, null, ServerCatalog.entry("whisper-base-en-q5_1")!!.pins, catalogueId = "whisper-base-en-q5_1")).getOrThrow().target)
    }

    @Test fun catalogueCopiesMustMatchTheBuiltInPins() {
        val e = ServerCatalog.entry("parakeet-tdt-0.6b-v2-int8")!!
        val ok = plan(model("srv-name", ModelKind.STT, "parakeet", e.pins + f("README.md"), catalogueId = e.id)).getOrThrow()
        assertEquals(Slot.STT_PARAKEET, ok.slot)
        assertEquals(File("/p/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"), ok.target)
        assertEquals(e.pins, ok.fetches.map { it.file })
        val swapped = e.pins.mapIndexed { i, p -> if (i == 0) p.copy(sha256 = "0".repeat(64)) else p }
        assertTrue(reason(model("x", ModelKind.STT, "parakeet", swapped, catalogueId = e.id)).contains("differs from the fingerprint"))
        assertTrue(reason(model("x", ModelKind.STT, "parakeet", e.pins.drop(1), catalogueId = e.id)).contains("has no encoder.int8.onnx"))
        // The Hebrew catalogue copy installs where import puts it (kindOf knows it by its pins).
        val he = ServerCatalog.entry("zipformer-he-int8")!!
        assertEquals(File("/h/${HebrewModels.Kind.ZIPFORMER.dirName}"), plan(model("z", ModelKind.HEBREW, "sherpa_online", he.pins, catalogueId = he.id)).getOrThrow().target)
        // An id this build doesn't know is a custom model.
        assertNull(plan(model("w", ModelKind.STT, "whisper", listOf(f("a.bin")), catalogueId = "whisper-huge")).getOrThrow().catalogue)
    }

    @Test fun eachScreenListsItsModels() {
        assertEquals(ModelPages.Screen.WHISPER, ServerSlots.screenOf(model("w", ModelKind.STT, "whisper", listOf(f("a.bin")))))
        assertEquals(ModelPages.Screen.PARAKEET, ServerSlots.screenOf(model("p", ModelKind.STT, "parakeet", SHERPA)))
        assertEquals(ModelPages.Screen.HEBREW, ServerSlots.screenOf(model("h", ModelKind.HEBREW, "parakeet", SHERPA)))
        assertEquals(ModelPages.Screen.HEBREW, ServerSlots.screenOf(model("lid", ModelKind.LID, "whisper", listOf(f("a.bin")))))
        assertEquals(ModelPages.Screen.LLM, ServerSlots.screenOf(model("q", ModelKind.LLM, "gguf", listOf(f("a.gguf")))))
        assertEquals(ModelPages.Screen.PARAKEET, ServerSlots.screenOf(model("x", ModelKind.STT, "whisper", listOf(f("a.bin")), catalogueId = "parakeet-tdt-0.6b-v2-int8")))
    }
}
