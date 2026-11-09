package dev.murmur.data

import dev.murmur.BuildConfig

/**
 * This APK's build identity (D26, docs/versioning.md): the product version from the repo's
 * `VERSION` file, its versionCode, and the git commit it was built from.
 *   build = `abc1234` / `abc1234-dirty` / `unknown`; full = `0.2.0+abc1234.dirty` (SemVer build metadata).
 */
data class AppBuild(val version: String, val code: Int, val build: String, val full: String) {
    /** Settings footer: "Version 0.2.0 (hybrid) · abc1234". */
    fun label(flavor: String) = "Version $version ($flavor) · $build"

    /** Developer → Build row: "0.2.0 (200) · build abc1234". */
    fun detail() = "$version ($code) · build $build"
}

/** The version rules; tools/versioning/versioning.py is the reference (shared vectors). */
object AppVersion {
    private val semver = Regex("""(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)""")

    /** MAJOR.MINOR.PATCH; MINOR, PATCH ≤ 99 and MAJOR ≤ 209999 keep versionCode under Android's 2.1e9. */
    fun parse(v: String): Triple<Int, Int, Int> {
        val m = semver.matchEntire(v) ?: throw IllegalArgumentException("not MAJOR.MINOR.PATCH: $v")
        val (a, b, c) = m.destructured
        val (major, minor, patch) = listOf(a, b, c).map { it.toIntOrNull() ?: throw IllegalArgumentException("too large: $v") }
        require(major <= 209_999 && minor <= 99 && patch <= 99) { "out of range for a versionCode: $v" }
        return Triple(major, minor, patch)
    }

    /** versionCode = MAJOR·10000 + MINOR·100 + PATCH (the same formula as app/build.gradle.kts). */
    fun code(v: String): Int = parse(v).let { (a, b, c) -> a * 10_000 + b * 100 + c }

    fun describe(version: String, sha: String, dirty: Boolean): Pair<String, String> = when {
        sha.isEmpty() -> "unknown" to "$version+unknown"
        dirty -> "$sha-dirty" to "$version+$sha.dirty"
        else -> sha to "$version+$sha"
    }

    val current: AppBuild by lazy {
        val (build, full) = describe(BuildConfig.VERSION_NAME, BuildConfig.GIT_SHA, BuildConfig.GIT_DIRTY)
        AppBuild(BuildConfig.VERSION_NAME, BuildConfig.VERSION_CODE, build, full)
    }
}
