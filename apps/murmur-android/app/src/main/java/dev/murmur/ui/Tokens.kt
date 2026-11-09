package dev.flowclone.ui

import android.content.Context
import android.content.res.Configuration
import android.graphics.Color

/**
 * Design tokens of the Android app, from docs/design/app-design/flow-theme.css:
 * light = the `.lw-android` block (cool-grey Material-style surfaces) plus the design
 * system's badge / route / segment colours; dark = derived from the `.lw-dark` block (the design
 * has no dark Android artboard, so the Android roles take the desktop dark values).
 * [DevPalette] maps its older role names onto these, so every screen follows them.
 */
internal class Tokens private constructor(
    val night: Boolean,
    /** Page background of the main tabs (`--a-bg`). */
    val bg: Int,
    /** Page background of settings pages and the drawer (`--a-bg-settings`). */
    val bgSettings: Int,
    val card: Int,
    val ink: Int,
    val ink2: Int,
    /** Outlined field / button border (`--a-outline`). */
    val outline: Int,
    /** Pressed / selected fill and the progress-bar track (`--a-selected`). */
    val selected: Int,
    val divider: Int,
    /** Filled dark button (`--a-button`) and its text. */
    val button: Int, val onButton: Int,
    /** Peach callout (`--a-peach`) and its text. */
    val peach: Int, val onPeach: Int,
    /** Bubble / sample tint (`--a-bubble`). */
    val bubble: Int,
    /** Switch: off track + border + thumb, on track + thumb (`.a-switch`). */
    val trackOff: Int, val trackOffBorder: Int, val thumbOff: Int, val trackOn: Int, val thumbOn: Int,
    /** FAB (`.a-fab`, vast-950 on light). */
    val fab: Int, val onFab: Int,
    /** Input fill behind an outlined field. */
    val field: Int,
    val danger: Int, val dangerBg: Int, val ok: Int, val warnBg: Int,
    /** `.fl-badge-success` / `.fl-badge-warning` / `.fl-badge-light`. */
    val badgeOkBg: Int, val badgeOkFg: Int, val badgeWarnBg: Int, val badgeWarnFg: Int, val badgeNeutralBg: Int, val badgeNeutralFg: Int,
    /** `.lw-route.is-pc` / `.is-server` / `.is-cloud`. */
    val routePhoneBg: Int, val routePhoneFg: Int, val routeServerBg: Int, val routeServerFg: Int, val routeCloudBg: Int, val routeCloudFg: Int,
    /** `.lw-rank`. */
    val rankBg: Int, val rankFg: Int,
    /** `.lw-seg`: track, selected segment, selected text, other text. */
    val segBg: Int, val segOn: Int, val segOnText: Int, val segText: Int,
    /** Status dots (`--success-500`, `--glow-300`). */
    val dotOk: Int, val dotWarn: Int,
) {
    companion object {
        private fun c(s: String) = Color.parseColor(s)

        fun isNight(ctx: Context) =
            ctx.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES

        fun of(ctx: Context): Tokens = if (isNight(ctx)) DARK else LIGHT

        val LIGHT = Tokens(
            night = false,
            bg = c("#eeeef0"), bgSettings = c("#f5f5f5"), card = c("#ffffff"), ink = c("#1a1a1a"), ink2 = c("#79767d"),
            outline = c("#cac4d0"), selected = c("#dcdcdc"), divider = c("#dae0dc"), button = c("#48454e"), onButton = c("#ffffff"),
            peach = c("#fdd8ac"), onPeach = c("#1a1a1a"), bubble = c("#f8f7ff"),
            trackOff = c("#f3ebfa"), trackOffBorder = c("#e4d8ef"), thumbOff = c("#ffffff"), trackOn = c("#1a1a1a"), thumbOn = c("#ffffff"),
            fab = c("#1a1a1a"), onFab = c("#ffffff"), field = c("#ffffff"),
            danger = c("#dc2626"), dangerBg = c("#fee2e2"), ok = c("#059669"), warnBg = c("#ffedd5"),
            badgeOkBg = c("#d1fae5"), badgeOkFg = c("#065f46"), badgeWarnBg = c("#ffedd5"), badgeWarnFg = c("#9a3412"),
            badgeNeutralBg = c("#f5f4f0"), badgeNeutralFg = c("#71716e"),
            routePhoneBg = c("#ceedea"), routePhoneFg = c("#034f46"), routeServerBg = c("#f7ebff"), routeServerFg = c("#502b66"),
            routeCloudBg = c("#fce5d1"), routeCloudFg = c("#3c2407"),
            rankBg = c("#eeebe3"), rankFg = c("#30302f"),
            segBg = c("#eeebe3"), segOn = c("#ffffff"), segOnText = c("#1a1a1a"), segText = c("#30302f"),
            dotOk = c("#4fbf78"), dotWarn = c("#ffa946"),
        )

        val DARK = Tokens(
            night = true,
            bg = c("#1a1a1a"), bgSettings = c("#1a1a1a"), card = c("#202020"), ink = c("#e9e8e1"), ink2 = c("#9d9c98"),
            outline = c("#5b5b59"), selected = c("#30302f"), divider = c("#30302f"), button = c("#fcfcfb"), onButton = c("#1a1a1a"),
            peach = c("#251403"), onPeach = c("#facda1"), bubble = c("#502b66"),
            trackOff = c("#30302f"), trackOffBorder = c("#5b5b59"), thumbOff = c("#9d9c98"), trackOn = c("#fcfcfb"), thumbOn = c("#1a1a1a"),
            fab = c("#fcfcfb"), onFab = c("#1a1a1a"), field = c("#2a2a29"),
            danger = c("#ff5757"), dangerBg = c("#3b1f1e"), ok = c("#34d399"), warnBg = c("#3a2414"),
            // rgba(52,211,153,.15) / rgba(249,115,22,.15) over the dark card.
            badgeOkBg = c("#24392f"), badgeOkFg = c("#34d399"), badgeWarnBg = c("#3a2a1f"), badgeWarnFg = c("#f97316"),
            badgeNeutralBg = c("#30302f"), badgeNeutralFg = c("#9d9c98"),
            routePhoneBg = c("#034f46"), routePhoneFg = c("#b8e4dc"), routeServerBg = c("#3c2947"), routeServerFg = c("#f0d7ff"),
            routeCloudBg = c("#251403"), routeCloudFg = c("#facda1"),
            rankBg = c("#30302f"), rankFg = c("#deddd7"),
            segBg = c("#30302f"), segOn = c("#1a1a1a"), segOnText = c("#e9e8e1"), segText = c("#deddd7"),
            dotOk = c("#34d399"), dotWarn = c("#f97316"),
        )
    }
}

/** Type scale of the Android artboards (sp) and the weights they use with Figtree. */
internal object TypeScale {
    /** Top-bar title on main tabs (700 22/28, -0.02em). */
    const val TITLE = 22f
    const val TITLE_TRACKING = -0.02f
    /** Top-bar title on settings pages (550 21/28). */
    const val PAGE_TITLE = 21f
    /** Day / group heading (700 20/28). */
    const val HEADING = 20f
    /** Row title and body text (550 or 400 17/24). */
    const val ROW = 17f
    /** Row subtitle / secondary text (400 15/21). */
    const val SUB = 15f
    /** Buttons (550 16-17). */
    const val BUTTON = 16f
    /** Uppercase section label (600 13/18, .06em). */
    const val SECTION = 13f
    const val SECTION_TRACKING = 0.06f
    /** Small meta / captions (400 13-14). */
    const val META = 13f
    /** Badge / route tag (550 12 / 11). */
    const val BADGE = 12f
    const val ROUTE = 11f
    /** Serif stat number (EB Garamond 26/28). */
    const val STAT = 26f

    const val W_REGULAR = 400
    const val W_MEDIUM = 550
    const val W_SEMIBOLD = 600
    const val W_BOLD = 700
}
