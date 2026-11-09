package dev.flowclone.server

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D41: flow-ffi's library JSON (JNI_CONTRACT.md "Model library") as the app reads it. */
class ServerLibraryJsonTest {
    @Test fun parsesTheList() {
        val r = LibraryJson.parseList(
            """{"ok":true,"models":[
              {"id":"whisper-base","title":"Whisper base","kind":"stt","catalogue_id":"whisper-base","engine":"whisper","files":[{"name":"ggml-base.bin","size":147951465,"sha256":"${"a".repeat(64)}"}]},
              {"id":"future","title":"Future","kind":"diarizer","files":[{"name":"x.bin","size":1,"sha256":"${"b".repeat(64)}"}]},
              {"id":"empty","title":"No files","kind":"stt","files":[]}
            ]}""",
        ) as ListResult.Ok
        assertEquals(2, r.models.size)
        val m = r.models[0]
        assertEquals(ModelKind.STT, m.kind)
        assertEquals("whisper-base", m.catalogueId)
        assertEquals("whisper", m.engine)
        assertNull(m.language)
        assertEquals(LibraryFile("ggml-base.bin", 147951465, "a".repeat(64)), m.files[0])
        assertNull("unknown kind kept, can't be used", r.models[1].kind)
        assertEquals("diarizer", r.models[1].kindWire)
    }

    @Test fun failuresKeepTheirKind() {
        val r = LibraryJson.parseList("""{"ok":false,"error":"auth","message":"the server refused the key (HTTP 401)"}""") as ListResult.Failed
        assertEquals("auth", r.failure.kind)
        assertTrue(LibraryJson.parseList("garbage") is ListResult.Failed)
        assertTrue(LibraryJson.parseList(null) is ListResult.Failed)
        assertNull(LibraryJson.parseDownload("""{"ok":true}"""))
        assertEquals("mismatch", LibraryJson.parseDownload("""{"ok":false,"error":"mismatch","message":"m"}""")!!.kind)
        assertEquals("protocol", LibraryJson.parseDownload("")!!.kind)
    }

    @Test fun fileJsonIsWhatTheNativeSideReads() {
        val o = org.json.JSONObject(LibraryFile("a/b.onnx", 5, "c".repeat(64)).toJson())
        assertEquals("a/b.onnx", o.getString("name"))
        assertEquals(5L, o.getLong("size"))
        assertEquals("c".repeat(64), o.getString("sha256"))
    }
}
