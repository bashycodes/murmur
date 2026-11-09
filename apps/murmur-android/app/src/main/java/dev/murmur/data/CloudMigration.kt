package dev.murmur.data

import org.json.JSONArray
import org.json.JSONObject

/**
 * D28 one-time migration of the stored D18 cloud settings (schema 1: up to 8 providers) to the
 * one OpenRouter config (schema 2). The rules are the router's (`RouterConfig::migrate_d28`,
 * reached through `nativeMigrateCloud`, fixtures `crates/murmur-router/fixtures/migration/`); this
 * applies the report to what only the shell owns:
 *  1. keys: `murmur_cloud_<key_from>` → `murmur_cloud_openrouter`, every `delete_keys` vault destroyed;
 *  2. this month's usage: the kept entry renamed, removed providers folded into `removed`
 *     (the month's total, and so the budget, is unchanged; `CloudUsage::migrate_d28`);
 *  3. the schema-2 config persisted (which marks the migration done);
 *  4. the notice kept for a one-time banner ([DevStore.cloudNotice]).
 * Keys first: a crash half-way re-runs the whole thing next start and every step is idempotent.
 */
object CloudMigration {
    data class Report(
        val migrated: Boolean,
        val keyFrom: String? = null,
        val keyTo: String? = null,
        val deleteKeys: List<String> = emptyList(),
        /** Ids of the removed providers (their usage counters fold into [CloudUsage.REMOVED_ID]). */
        val removedIds: List<String> = emptyList(),
        val removedNames: List<String> = emptyList(),
        val cloudDisabled: Boolean = false,
        val modelFrom: String? = null,
        val modelTo: String? = null,
        val notice: String? = null,
    )

    fun decodeReport(o: JSONObject): Report {
        fun str(k: String) = if (!o.has(k) || o.isNull(k)) null else o.optString(k).ifEmpty { null }
        val del = o.optJSONArray("delete_keys") ?: JSONArray()
        val rem = o.optJSONArray("removed") ?: JSONArray()
        val removed = (0 until rem.length()).mapNotNull { rem.optJSONObject(it) }
        return Report(
            migrated = o.optBoolean("migrated"),
            keyFrom = str("key_from"), keyTo = str("key_to"),
            deleteKeys = (0 until del.length()).map { del.optString(it) }.filter { it.isNotEmpty() },
            removedIds = removed.map { it.optString("id") },
            removedNames = removed.map { r -> r.optString("name").ifEmpty { r.optString("id") } },
            cloudDisabled = o.optBoolean("cloud_disabled"),
            modelFrom = str("model_from"), modelTo = str("model_to"),
            notice = str("notice"),
        )
    }

    /** `nativeMigrateCloud`'s answer → (config, report); null when malformed. */
    fun parse(out: String?): Pair<JSONObject, Report>? = runCatching {
        val o = JSONObject(out ?: return null)
        val cfg = o.optJSONObject("config") ?: return null
        cfg to decodeReport(o.optJSONObject("report") ?: return null)
    }.getOrNull()

    /** What `nativeMigrateCloud` gets: the stored settings as `RouterConfig` JSON with the raw legacy `cloud`. */
    fun devJson(stored: DevSettings, legacy: JSONObject): String =
        stored.copy(cloud = CloudSettings()).toRouterConfigJson().apply {
            put("allow_cloud", stored.allowCloud)
            put("order", JSONArray(stored.routeOrder.map { it.wire }))
            put("cloud", legacy)
            remove("allow_cloud_cleanup"); remove("cleanup_order"); remove("cleanup_off")
        }.toString()

    /** The migrated config into the stored settings; only the cloud part changes. */
    fun apply(stored: DevSettings, config: JSONObject, r: Report): DevSettings {
        val m = DevSettings.fromRouterConfigJson(config.toString())
        return stored.copy(
            cloud = m.cloud,
            // "If another provider was active, cloud STT is switched off rather than silently moved."
            allowCloud = if (r.cloudDisabled) false else config.optBoolean("allow_cloud", stored.allowCloud),
            routeOrder = if (r.cloudDisabled) stored.routeOrder - DevSettings.Route.CLOUD else stored.routeOrder,
            allowCloudCleanup = m.allowCloudCleanup, cleanupOrder = m.cleanupOrder, cleanupOff = m.cleanupOff,
        )
    }

    /** Kotlin twin of `CloudUsage::migrate_d28`. Null in, null out. */
    fun migrateUsage(json: String?, r: Report): String? {
        if (json == null || !r.migrated) return json
        val u = CloudUsage.decode(json)
        var gone = CloudUsage.ProviderUsage(CloudUsage.REMOVED_ID)
        var anyGone = false
        val kept = ArrayList<CloudUsage.ProviderUsage>()
        for (p in u.providers) when {
            r.keyFrom != null && p.providerId == r.keyFrom -> kept += p.copy(providerId = OpenRouter.ID)
            p.providerId in r.removedIds -> {
                anyGone = true
                gone = gone.copy(
                    requests = gone.requests + p.requests, billedAudioMs = gone.billedAudioMs + p.billedAudioMs,
                    costUsd = gone.costUsd + p.costUsd, costEstimated = gone.costEstimated || p.costEstimated,
                )
            }
            else -> kept += p
        }
        if (anyGone) {
            val i = kept.indexOfFirst { it.providerId == CloudUsage.REMOVED_ID }
            if (i < 0) kept += gone
            else kept[i] = kept[i].let {
                it.copy(
                    requests = it.requests + gone.requests, billedAudioMs = it.billedAudioMs + gone.billedAudioMs,
                    costUsd = it.costUsd + gone.costUsd, costEstimated = it.costEstimated || gone.costEstimated,
                )
            }
        }
        return u.copy(providers = kept).toJson().toString()
    }

    /**
     * Runs the migration when the stored cloud settings are still schema 1. [native] is
     * `nativeMigrateCloud` (pure, no network); null from it (library missing, bad JSON) leaves
     * everything as it was, to retry next start. Returns the report, or null when nothing ran.
     */
    fun run(store: DevStore, keys: CloudKeyStore, native: (String) -> String?): Report? {
        val legacy = store.legacyCloudJson() ?: return null
        val stored = store.load()
        val (config, r) = parse(native(devJson(stored, legacy))) ?: return null
        r.keyFrom?.let { keys.move(it, r.keyTo ?: OpenRouter.ID) }
        for (id in r.deleteKeys) if (id != OpenRouter.ID) keys.remove(id)
        store.cloudUsageJson?.let { store.cloudUsageJson = migrateUsage(it, r) }
        val next = apply(stored, config, r)
        store.saveCloud(
            allowCloud = next.allowCloud, cloud = next.cloud, routeOrder = next.routeOrder,
            allowCloudCleanup = next.allowCloudCleanup, cleanupOrder = next.cleanupOrder, cleanupOff = next.cleanupOff,
        )
        r.notice?.let { store.cloudNotice = it }
        return r
    }
}
