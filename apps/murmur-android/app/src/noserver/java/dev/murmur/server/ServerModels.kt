package dev.murmur.server

import dev.murmur.ui.ServerModelsHost
import dev.murmur.ui.ServerModelsSection

/**
 * The local flavour's stand-in for src/server's `ServerModels` (D41): no server, no downloads,
 * no network code (D9). Models arrive only by Import.
 */
internal object ServerModels {
    const val AVAILABLE = false

    @Suppress("UNUSED_PARAMETER")
    fun section(host: ServerModelsHost): ServerModelsSection? = null
}
