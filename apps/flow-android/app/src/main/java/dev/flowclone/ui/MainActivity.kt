package dev.flowclone.ui

import android.Manifest
import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import android.widget.Toast
import dev.flowclone.EngineHolder
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.data.DictionaryPages
import dev.flowclone.data.EnginePlan
import dev.flowclone.data.SettingsPages
import dev.flowclone.data.SettingsSummary
import dev.flowclone.data.SetupChecklist
import dev.flowclone.data.SetupChecklist.Item
import dev.flowclone.health.OemGuidance
import dev.flowclone.service.FlowService
import java.io.File

/**
 * Main screen in the Settings look (ui/DevUi): a problem banner while a setup step is missing,
 * STATUS (native engine, loaded model, next dictation language → Language, AI cleanup → AI
 * cleanup, version), SETUP (the five permissions in Wispr's order, 03 §Onboarding / 10 §2, each
 * showing its state or the action that fixes it, [SetupChecklist]; the OEM autostart guidance as
 * a collapsible row), RUN (start / stop the bubble) and MORE (Settings, Dictionary & snippets,
 * Notes, Quick note, Insights, History, speech model, Developer settings). While a required step is missing SETUP comes first.
 */
class MainActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private lateinit var ui: DevUi
    private val main = Handler(Looper.getMainLooper())
    private var shown: String? = null
    private var lastSetup: SetupChecklist.State? = null
    /** The OEM steps stay open across the 2 s re-renders. */
    private var oemOpen = false

    /** Re-render only when something visible changed (a rebuild keeps the scroll position). */
    private val refresh = object : Runnable {
        override fun run() {
            val sig = signature()
            if (sig != shown) { shown = sig; render() }
            main.postDelayed(this, 2_000)
        }
    }

    private fun signature() = listOf(
        app.engine.state.value, app.accessibilityHealth(), granted(Manifest.permission.RECORD_AUDIO),
        Settings.canDrawOverlays(this), getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(packageName),
        granted(Manifest.permission.POST_NOTIFICATIONS),
    ).joinToString("|") { if (it is EngineHolder.State.Ready) "ready:${it.description}:${it.llm}" else it.toString() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
    }

    override fun onResume() {
        super.onResume()
        shown = null
        refresh.run() // engine loading / a11y connecting change while we look
    }

    override fun onPause() {
        main.removeCallbacks(refresh)
        super.onPause()
    }

    private fun granted(p: String) = checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    private fun setupState() = SetupChecklist.State(
        mic = granted(Manifest.permission.RECORD_AUDIO),
        overlay = Settings.canDrawOverlays(this),
        a11y = app.accessibilityHealth(),
        batteryExempt = getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(packageName) == true,
        notifications = granted(Manifest.permission.POST_NOTIFICATIONS),
        keepDebugAudio = app.dev.keepDebugAudio,
    )

    private fun fileName(path: String?) = path?.let { File(it).name }

    private fun render(): Unit = with(ui) {
        val setup = setupState()
        rebuild {
            note(
                when (app.caps.flavor) {
                    "local" -> "On-device voice dictation. This app has no internet permission: audio, transcripts and history never leave the phone."
                    "hybrid" -> "Voice dictation on this phone or on your own flow-server (your LAN or a VPN; Developer settings). Voice never goes to a third party; history stays on the phone."
                    else -> "Voice dictation through your own flow-server (your LAN or a VPN; Developer settings). Voice never goes to a third party; history stays on the phone."
                },
            )
            SetupChecklist.banner(setup)?.let { b -> banner().show(b.text, if (b.blocking) Tone.BAD else Tone.WARN) }
            val first = SetupChecklist.setupFirst(setup)
            if (first) setupSection(setup)
            statusSection()
            if (!first) setupSection(setup)
            runSection()
            moreSection()
        }
        ui.announce(SetupChecklist.newlyDone(lastSetup, setup).joinToString(". "))
        lastSetup = setup
    }

    // ---- STATUS ----

    private fun android.widget.LinearLayout.statusSection(): Unit = with(ui) {
        sectionLabel("STATUS")
        card {
            when (val lib = app.engine.libraryState()) {
                is NativeFlowCore.LibraryState.Loaded -> {
                    infoRow("Native engine", lib.version).setTextColor(p.ok)
                    // BuildConfig.CAP_* must equal nativeCapabilities() (D17 §1.4).
                    NativeFlowCore.capabilities()?.let { c ->
                        if (c.local != app.caps.local || c.remote != app.caps.remote) note(
                            "Native library flavour mismatch: this is the ${app.caps.flavor} APK but libflow_ffi.so was built with " +
                                "local=${c.local} remote=${c.remote} (tools/build-native.sh --flavor ${app.caps.flavor}).",
                            p.danger, 14f,
                        )
                    }
                }
                is NativeFlowCore.LibraryState.Unavailable -> infoRow(
                    "Native engine",
                    "Unavailable. ${lib.message}\n\nThis build was packaged without libflow_ffi.so " +
                        "(see apps/flow-android/README.md). Dictation needs it, or turn on the mock engine in Settings to test the shell.",
                ).setTextColor(p.danger)
            }
            val st = app.engine.state.value
            infoRow(
                "Speech engine",
                when (st) {
                    is EngineHolder.State.Ready -> "Loaded: ${st.description}"
                    EngineHolder.State.Loading -> "Loading model…"
                    is EngineHolder.State.Failed -> st.message
                    EngineHolder.State.NotLoaded -> "Not loaded"
                },
            ).setTextColor(if (st is EngineHolder.State.Ready) p.ok else if (st is EngineHolder.State.Failed) p.danger else p.text)
            // 2026-10-23: the language the next dictation really uses (unset follows the phone).
            val langSetting = app.settings.load().language
            link(R.drawable.ic_set_language, "Language", dev.flowclone.data.LanguageDefault.label(langSetting, app.effectiveLanguage()), SettingsPages.Page.LANGUAGE)
            if (st is EngineHolder.State.Ready) {
                val (text, bad) = when (val l = st.llm) {
                    EnginePlan.LlmStatus.Off -> (if (app.settings.load().spokenCleanup) "Off (formatting rules still run)" else "Off") to false
                    EnginePlan.LlmStatus.NoModel -> "On, but no GGUF model imported" to true
                    is EnginePlan.LlmStatus.Loaded -> l.name to false
                    is EnginePlan.LlmStatus.Failed -> "Failed to load, using formatted transcripts: ${l.message}" to true
                }
                link(R.drawable.ic_set_sparkle, "AI cleanup", text, SettingsPages.Page.CLEANUP, bad)
            }
            infoRow("Version", dev.flowclone.data.AppVersion.current.label(app.caps.flavor))
        }
    }

    /** Status line → its Settings page (only pages this build has); one 48 dp+ row for TalkBack. */
    private fun android.widget.LinearLayout.link(icon: Int, title: String, value: String, page: SettingsPages.Page, bad: Boolean = false): Unit = with(ui) {
        if (!SettingsPages.visible(page, app.caps)) { infoRow(title, value); return@with }
        val row = navRow(icon, title, value) { SettingsActivity.open(this@MainActivity, page) }
        if (bad) row.subtitle.setTextColor(p.danger)
        row.view.contentDescription = "$title: $value. Opens ${SettingsPages.TITLES.getValue(page)} settings"
    }

    // ---- SETUP ----

    private fun android.widget.LinearLayout.setupSection(s: SetupChecklist.State): Unit = with(ui) {
        sectionLabel("SETUP")
        card {
            cardHeader("Permissions", SetupChecklist.progress(s))
            for (r in SetupChecklist.rows(s)) {
                val tone = when {
                    r.ok -> Tone.OK
                    r.action == null -> Tone.NEUTRAL
                    r.need == SetupChecklist.Need.REQUIRED -> Tone.BAD
                    r.need == SetupChecklist.Need.RECOMMENDED -> Tone.WARN
                    else -> Tone.NEUTRAL
                }
                stateRow(ICONS.getValue(r.item), r.title, r.subtitle, r.state, tone, r.action) { act(r.item) }
            }
            val oem = OemGuidance.forManufacturer(Build.MANUFACTURER)
            val ex = expandRow(R.drawable.ic_scr_phone, "Keep it running: ${oem.title}", "Phone-specific steps so Android doesn't stop Murmur", oemOpen) {
                note(oem.steps, p.text, 14f)
                oem.extraWarning?.let { note(it, p.danger, 14f) }
                if (oem.intents.isNotEmpty()) chipRow(chip("Open phone settings") { openOem(oem) })
            }
            ex.header.setOnClickListener { ex.expanded = !ex.expanded; oemOpen = ex.expanded }
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

    // ---- RUN / MORE ----

    private fun android.widget.LinearLayout.runSection(): Unit = with(ui) {
        sectionLabel("RUN")
        card {
            navRow(R.drawable.ic_scr_play, "Start / show bubble", "Shows the dictation bubble over other apps", chevron = false) { startBubble() }
            navRow(R.drawable.ic_scr_stop, "Stop bubble", "Hides it until you start it again", chevron = false) {
                runCatching { startService(Intent(this@MainActivity, FlowService::class.java).setAction(FlowService.ACTION_STOP)) }
                main.postDelayed({ render() }, 300)
            }
        }
    }

    private fun android.widget.LinearLayout.moreSection(): Unit = with(ui) {
        sectionLabel("MORE")
        card {
            navRow(R.drawable.ic_scr_settings, "Settings", "Language, writing style, timing, bubble, history") {
                startActivity(Intent(this@MainActivity, SettingsActivity::class.java))
            }
            navRow(R.drawable.ic_scr_snippet, "Notes", "Saved notes to dictate into, search and share") {
                startActivity(Intent(this@MainActivity, NotesActivity::class.java))
            }
            navRow(R.drawable.ic_scr_file, "Quick note", "One note to dictate into, edit and copy out") {
                startActivity(NoteEditorActivity.scratchpad(this@MainActivity))
            }
            navRow(R.drawable.ic_scr_chart, "Insights", "Words, speaking speed, streaks and top apps") {
                startActivity(Intent(this@MainActivity, InsightsActivity::class.java))
            }
            val dict = app.dictionary.all()
            navRow(R.drawable.ic_scr_book, "Dictionary & snippets", DictionaryPages.mainSubtitle(dict)) { startActivity(Intent(this@MainActivity, DictionaryActivity::class.java)) }
            navRow(R.drawable.ic_set_history, "History", SettingsSummary.history(app.settings.load().retention)) {
                startActivity(Intent(this@MainActivity, HistoryActivity::class.java))
            }
            if (app.caps.local) navRow(R.drawable.ic_set_mic, "Speech model", fileName(app.settings.modelPath) ?: "None imported") {
                startActivity(Intent(this@MainActivity, ModelImportActivity::class.java))
            }
            if (app.dev.visible) navRow(R.drawable.ic_set_code, "Developer settings", "Engines, server") {
                startActivity(Intent(this@MainActivity, DeveloperActivity::class.java))
            }
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

    private fun startBubble() {
        if (!Settings.canDrawOverlays(this)) {
            Toast.makeText(this, "Allow Display over other apps first (Setup).", Toast.LENGTH_LONG).show()
            return
        }
        runCatching { startForegroundService(Intent(this, FlowService::class.java).setAction(FlowService.ACTION_SHOW)) }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        shown = signature()
        render()
    }

    private companion object {
        val ICONS = mapOf(
            Item.MIC to R.drawable.ic_set_mic, Item.OVERLAY to R.drawable.ic_scr_overlay, Item.ACCESSIBILITY to R.drawable.ic_scr_a11y,
            Item.BATTERY to R.drawable.ic_scr_battery, Item.NOTIFICATIONS to R.drawable.ic_scr_bell,
        )
    }
}
