package dev.murmur.ui

import android.content.Context
import android.content.res.AssetManager
import android.graphics.Typeface

/**
 * The design's two typefaces (docs/design/app-design: `--font-sans` Figtree, `--font-serif`
 * EB Garamond), shipped as variable-weight fonts in assets/fonts (SIL OFL 1.1; licences in
 * assets/licenses). A weight is picked with the `wght` axis (Typeface.Builder, API 26+; minSdk 33).
 * If a font cannot be loaded the system sans / serif at that weight is used, so text always renders.
 */
internal object Fonts {
    const val SANS_ASSET = "fonts/figtree.ttf"
    const val SERIF_ASSET = "fonts/eb_garamond.ttf"

    private val cache = HashMap<String, Typeface>()

    /** Figtree at [weight] (100..900; the file covers 300..900). */
    fun sans(ctx: Context, weight: Int = TypeScale.W_REGULAR): Typeface = get(ctx.assets, SANS_ASSET, weight, Typeface.SANS_SERIF)

    /** EB Garamond at [weight] (the file covers 400..800). */
    fun serif(ctx: Context, weight: Int = TypeScale.W_REGULAR): Typeface = get(ctx.assets, SERIF_ASSET, weight, Typeface.SERIF)

    /** [path] from [assets] at [weight], or [fallback] at that weight when it cannot be loaded. */
    @Synchronized
    fun get(assets: AssetManager, path: String, weight: Int, fallback: Typeface): Typeface {
        val w = weight.coerceIn(1, 1000)
        return cache.getOrPut("$path@$w") { load(assets, path, w) ?: Typeface.create(fallback, w, false) }
    }

    private fun load(assets: AssetManager, path: String, weight: Int): Typeface? = runCatching {
        // Builder.build() returns null (no fallback family set) when the file is missing or bad.
        if (assets.list(path.substringBeforeLast('/'))?.contains(path.substringAfterLast('/')) != true) return null
        Typeface.Builder(assets, path)
            .setFontVariationSettings("'wght' $weight")
            .setWeight(weight)
            .build()
    }.getOrNull()

    /** Test hook: forget loaded typefaces. */
    @Synchronized
    internal fun clear() = cache.clear()
}
