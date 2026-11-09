package dev.flowclone.data

/**
 * JVM-test stand-in for `nativeCustomCleanupAddress` (the router's `custom_server_address`; its
 * real rules are tested in flow-router). Good enough for the plain hosts the tests use: http(s),
 * lower-cased host, default port dropped, `/v1` when there is no path, no trailing slash.
 */
object FakeCustomAddress : CustomAddressCheck {
    override fun check(baseUrl: String, model: String): CustomAddress {
        val m = model.trim()
        val modelOk = m.isNotEmpty() && m.toByteArray().size <= 256 && m.none { it.isWhitespace() || it.isISOControl() }
        val t = baseUrl.trim()
        fun bad(why: String) = CustomAddress(modelOk = modelOk, error = "bad_base_url", message = why)
        if (t.isEmpty() || t.any { it.isWhitespace() || it == '\\' || it == '?' || it == '#' || it == '@' }) return bad("base URL must be http(s)://host[:port][/path]")
        val u = runCatching { java.net.URI(t) }.getOrNull() ?: return bad("base URL does not parse")
        val scheme = u.scheme?.lowercase()
        if (scheme != "http" && scheme != "https") return bad("base URL must be http(s)://host[:port][/path]")
        val host = u.host?.lowercase()?.removePrefix("[")?.removeSuffix("]")?.takeIf { it.isNotEmpty() } ?: return bad("base URL has no host")
        val port = u.port.takeIf { it > 0 && it != (if (scheme == "https") 443 else 80) }?.let { ":$it" } ?: ""
        val path = (u.rawPath ?: "").trimEnd('/').ifEmpty { "/v1" }
        val urlHost = if (':' in host) "[$host]" else host
        return CustomAddress(url = "$scheme://$urlHost$port$path", host = host, modelOk = modelOk)
    }

    fun install() { CustomServer.check = this }
}
