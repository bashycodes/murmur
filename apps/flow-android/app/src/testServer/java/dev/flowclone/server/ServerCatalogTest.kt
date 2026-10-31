package dev.flowclone.server

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** D41: the Android catalogue ids and pins are the Windows ones (apps/flow-windows/src/model_download.rs). */
class ServerCatalogTest {
    private val windows: String = File(requireNotNull(System.getProperty("flow.windowsCatalog")) { "flow.windowsCatalog not set by build.gradle.kts" }).readText()

    @Test fun everyPinIsTheWindowsPin() {
        for (e in ServerCatalog.ALL) {
            for (p in e.pins) {
                val size = "%,d".format(java.util.Locale.ROOT, p.size).replace(',', '_')
                assertTrue("${e.id}/${p.name}: sha256 not in the Windows catalogue", windows.contains("\"${p.sha256}\""))
                val line = windows.lines().firstOrNull { it.contains("\"${p.sha256}\"") }.orEmpty()
                assertTrue("${e.id}/${p.name}: name/size differ from Windows: $line", line.contains("\"${p.name}\"") && line.contains(size))
            }
        }
    }

    @Test fun everyIdIsAWindowsCatalogueId() {
        val ids = Regex("""id: "([^"]+)"""").findAll(windows).map { it.groupValues[1] }.toSet() +
            Regex("""pub const \w+_ID: &str = "([^"]+)"""").findAll(windows).map { it.groupValues[1] }.toSet()
        for (e in ServerCatalog.ALL) assertTrue("${e.id} is not a Windows catalogue id", e.id in ids)
        assertEquals(ServerCatalog.ALL.size, ServerCatalog.ALL.map { it.id }.toSet().size)
    }
}
