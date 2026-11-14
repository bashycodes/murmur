package dev.murmur.data

import dev.murmur.calls.CallsView
import dev.murmur.core.HealthReport
import dev.murmur.core.ServerModelName
import dev.murmur.core.ServerModelNames

/*
 * D98: how the models a server names read on its engine rows (Speech to text, AI cleanup, the
 * Settings hub and the drawer's Engines card). An older server names none, so every row reads
 * as before.
 */

/** The speech models, the default then each per-language one by its language's name: "Parakeet · Hebrew: Zipformer"; null = none. */
fun ServerModelNames.speechLabel(): String? =
    (listOfNotNull(speech?.name) + languages.map { (code, m) -> "${CallsView.languageName(code)}: ${m.name}" })
        .takeIf { it.isNotEmpty() }?.joinToString(" · ")

/** The cleanup model's name; null = none. */
fun ServerModelNames.cleanupLabel(): String? = cleanup?.name

/** "[name] · [model]", or [name] alone when there is no model. A separator, not brackets: titles carry their own. */
fun withModel(name: String, model: String?): String = if (model == null) name else "$name · $model"

/**
 * The models a connected probe ([this]) names, with the router's cleanup model ([cleanupStatusModel],
 * `cleanup_status.server_llm_model`, kept from the last hello without a probe) ahead of the probe's.
 */
fun HealthReport?.serverModelNames(cleanupStatusModel: String? = null): ServerModelNames {
    val m = this?.takeIf { it.ok }?.models ?: ServerModelNames()
    return if (cleanupStatusModel == null) m else m.copy(cleanup = ServerModelName(title = cleanupStatusModel))
}
