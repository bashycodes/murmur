package dev.murmur.ui

import android.Manifest
import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.Toast
import dev.murmur.EngineHolder
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.core.NativeMurmurCore
import dev.murmur.data.EnginePlan
import dev.murmur.data.SettingsPages
import dev.murmur.data.SetupChecklist
import dev.murmur.data.SetupChecklist.Item
import dev.murmur.data.ShellPages
import dev.murmur.health.OemGuidance

/**
 * Permissions (drawer → Permissions; AndroidSetup.dc.html): "3 of 5 done" progress, the problem
 * callout, the first missing required step in its own card with a filled button, then one card
 * per step with its state badge or the action that fixes it ([SetupChecklist], onboarding order),
 * the phone-specific "Keep it running" steps, and STATUS (engine, language, cleanup, version: the
 * old main screen's card, which the device checklist in INSTALL.md reads). Re-checks every 2 s while open (the user comes
 * back from system screens) and announces what became done.
 */
class PermissionsActivity : Activity() {
    private lateinit var ui: DevUi
    private val app get() = MurmurApp.of(this)
    private val main = Handler(Looper.getMainLooper())
    private var shown: String? = null
    private var lastSetup: SetupChecklist.State? = null
    /** The OEM steps stay open across re-renders. */
    private var oemOpen = false

    private val refresh = object : Runnable {
        override fun run() {
            val s = setupState(this@PermissionsActivity)
            val sig = "$s|${app.engine.state.value}"
            if (sig != shown) { shown = sig; render(s) }
            main.postDelayed(this, 2_000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
    }

    override fun onResume() {
        super.onResume()
        shown = null
        refresh.run()
    }

    override fun onPause() {
        main.removeCallbacks(refresh)
        super.onPause()
    }

    private fun render(s: SetupChecklist.State): Unit = with(ui) {
        val rows = SetupChecklist.rows(s)
        val featured = ShellPages.featured(rows)
        rebuild(t.bgSettings) {
            topBar("Permissions", DevUi.TopNav.BACK, { finish() })
            progress(ShellPages.doneFraction(rows), SetupChecklist.progress(s))
            SetupChecklist.banner(s)?.let { b ->
                val (lead, text) = ShellPages.callout(b.text)
                callout(text, lead.ifEmpty { null })
            }
            if (featured != null) featuredCard(featured)
            for (r in rows) if (r !== featured) card { stateRow(null, r.title, r.subtitle, r.state, tone(r), r.action) { act(r.item) } }
            val oem = OemGuidance.forManufacturer(Build.MANUFACTURER)
            card {
                val ex = expandRow(R.drawable.ic_scr_phone, "Keep it running: ${oem.title}", "Phone-specific steps so Android doesn't stop Murmur", oemOpen) {
                    note(oem.steps, p.text, 14f)
                    oem.extraWarning?.let { note(it, p.danger, 14f) }
                    if (oem.intents.isNotEmpty()) chipRow(chip("Open phone settings") { openOem(oem) })
                }
                ex.header.setOnClickListener { ex.expanded = !ex.expanded; oemOpen = ex.expanded }
            }
            statusSection()
        }
        announce(SetupChecklist.newlyDone(lastSetup, s).joinToString(". "))
        lastSetup = s
    }

    /**
     * STATUS (from the old main screen; INSTALL.md's device checklist reads it): native library,
     * speech engine, language, AI cleanup, version. Lines link to their Settings page.
     */
    private fun LinearLayout.statusSection(): Unit = with(ui) {
        sectionLabel("STATUS")
        card {
            when (val lib = app.engine.libraryState()) {
                is NativeMurmurCore.LibraryState.Loaded -> infoRow("Native engine", lib.version).setTextColor(p.ok)
                is NativeMurmurCore.LibraryState.Unavailable -> infoRow(
                    "Native engine",
                    "Unavailable. ${lib.message}\n\nThis build was packaged without libmurmur_ffi.so " +
                        "(see apps/murmur-android/README.md). Dictation needs it, or turn on the mock engine in Settings to test the shell.",
                ).setTextColor(p.danger)
            }
            val st = app.engine.state.value
            infoRow(
                "Speech engine",
                when (st) {
                    is EngineHolder.State.Ready -> "Loaded: ${st.description}"
                    EngineHolder.State.Loading -> app.engine.loadingLabel ?: "Loading model…"
                    is EngineHolder.State.Failed -> st.message
                    EngineHolder.State.NotLoaded -> "Not loaded"
                },
            ).setTextColor(if (st is EngineHolder.State.Ready) p.ok else if (st is EngineHolder.State.Failed) p.danger else p.text)
            val settings = app.settings.load()
            link("Language", app.resolveLanguage(settings).let { dev.murmur.data.LanguageDefault.label(settings.language, it.language, it.source) }, SettingsPages.Page.LANGUAGE)
            if (st is EngineHolder.State.Ready) {
                val (text, bad) = when (val l = st.llm) {
                    EnginePlan.LlmStatus.Off -> (if (settings.spokenCleanup) "Off (formatting rules still run)" else "Off") to false
                    EnginePlan.LlmStatus.NoModel -> "On, but no GGUF model imported" to true
                    is EnginePlan.LlmStatus.Loaded -> l.name to false
                    is EnginePlan.LlmStatus.Failed -> "Failed to load, using formatted transcripts: ${l.message}" to true
                }
                link("AI cleanup", text, SettingsPages.Page.CLEANUP, bad)
            }
            infoRow("Version", dev.murmur.data.AppVersion.current.label(app.caps.flavor))
        }
        note(
            when (app.caps.flavor) {
                "local" -> "On-device voice dictation. This app has no internet permission: audio, transcripts and history never leave the phone."
                "hybrid" -> "Voice dictation on this phone or on your own murmur-server (your LAN or a VPN; Developer settings). Voice never goes to a third party; history stays on the phone."
                else -> "Voice dictation through your own murmur-server (your LAN or a VPN; Developer settings). Voice never goes to a third party; history stays on the phone."
            },
        )
    }

    /** Status line → its Settings page (only pages this build has); one 48 dp+ row for TalkBack. */
    private fun LinearLayout.link(title: String, value: String, page: SettingsPages.Page, bad: Boolean = false): Unit = with(ui) {
        if (!SettingsPages.visible(page, app.caps)) { infoRow(title, value); return@with }
        val row = navRow(null, title, value) { SettingsActivity.open(this@PermissionsActivity, page) }
        if (bad) row.subtitle.setTextColor(p.danger)
        row.view.contentDescription = "$title: $value. Opens ${SettingsPages.TITLES.getValue(page)} settings"
    }

    private fun tone(r: SetupChecklist.Row) = when {
        r.ok -> Tone.OK
        r.action == null -> Tone.NEUTRAL
        r.need == SetupChecklist.Need.REQUIRED -> Tone.BAD
        r.need == SetupChecklist.Need.RECOMMENDED -> Tone.WARN
        else -> Tone.NEUTRAL
    }

    /** The step that blocks dictation first: title, explanation, its state badge and a full-width filled button. */
    private fun LinearLayout.featuredCard(r: SetupChecklist.Row): Unit = with(ui) {
        card {
            val col = LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(dp(20), dp(18), dp(20), dp(18))
            }
            val texts = LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(tv(r.title, TypeScale.ROW, p.text, TypeScale.W_MEDIUM))
                addView(tv(r.subtitle, TypeScale.SUB, p.muted).apply { setPadding(0, dp(4), 0, 0) })
            }
            val head = LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.TOP
                addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
                addView(badge(r.state, DevUi.BadgeTone.WARNING))
                isFocusable = true
                contentDescription = "${r.title}, ${r.state}. ${r.subtitle}"
                for (i in 0 until childCount) getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            }
            col.addView(head)
            val btn = filledButton(r.action ?: "") { act(r.item) }.apply { contentDescription = "${r.action} ${r.title}" }
            col.addView(btn, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(48)).apply { topMargin = dp(12) })
            addView(col)
        }
    }

    private fun act(item: Item) {
        when (item) {
            Item.MIC -> requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1)
            Item.OVERLAY -> startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
            Item.ACCESSIBILITY -> startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            Item.BATTERY -> requestBatteryExemption()
            Item.NOTIFICATIONS -> requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 2)
        }
    }

    private fun requestBatteryExemption() {
        val direct = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:$packageName"))
        if (runCatching { startActivity(direct) }.isFailure) {
            runCatching { startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) }
        }
    }

    /** Vendor screens move between OS versions: try each, then fall back to the app details page. */
    private fun openOem(g: OemGuidance) {
        for (c in g.intents) {
            val i = Intent().setComponent(ComponentName(c.packageName, c.className)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (runCatching { startActivity(i) }.isSuccess) return
        }
        Toast.makeText(this, "Couldn't open that screen on this phone; opening Murmur's app settings.", Toast.LENGTH_LONG).show()
        runCatching { startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))) }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        shown = null
        refresh.run()
    }

    companion object {
        fun intent(c: Context) = Intent(c, PermissionsActivity::class.java)

        private fun granted(c: Context, p: String) = c.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

        /** What the phone allows right now (Permissions page, drawer badge, Home notice). */
        internal fun setupState(c: Context): SetupChecklist.State {
            val app = MurmurApp.of(c)
            return SetupChecklist.State(
                mic = granted(c, Manifest.permission.RECORD_AUDIO),
                overlay = Settings.canDrawOverlays(c),
                a11y = app.accessibilityHealth(),
                batteryExempt = c.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(c.packageName) == true,
                notifications = granted(c, Manifest.permission.POST_NOTIFICATIONS),
                keepDebugAudio = app.dev.keepDebugAudio,
            )
        }
    }
}
