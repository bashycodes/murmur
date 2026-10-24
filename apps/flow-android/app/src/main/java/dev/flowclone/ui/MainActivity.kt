package dev.flowclone.ui

import android.Manifest
import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import android.widget.Toast
import dev.flowclone.EngineHolder
import dev.flowclone.FlowApp
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.data.EnginePlan
import dev.flowclone.data.SettingsPages
import dev.flowclone.health.AccessibilityHealth
import dev.flowclone.health.OemGuidance
import dev.flowclone.service.FlowService

/**
 * Onboarding checklist + engine status, in Wispr's order (03 §Onboarding, 10 §2):
 * mic → overlay → accessibility (prominent disclosure, ghost check) → battery → notifications,
 * then the OEM autostart guidance that keeps the service alive on aggressive skins.
 */
class MainActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private val main = Handler(Looper.getMainLooper())
    private var shown: String? = null
    /** Re-render only when something visible changed (a rebuild resets the scroll position). */
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

    private fun render() {
        page {
            heading("Flow Clone")
            body(
                when (app.caps.flavor) {
                    "local" -> "On-device voice dictation. This app has no internet permission: audio, transcripts and history never leave the phone."
                    "hybrid" -> "Voice dictation on this phone or on your own flow-server (LAN / Tailscale; Developer settings). Voice never goes to a third party; history stays on the phone."
                    else -> "Voice dictation through your own flow-server (LAN / Tailscale; Developer settings). Voice never goes to a third party; history stays on the phone."
                },
            )

            heading("Speech engine")
            when (val lib = app.engine.libraryState()) {
                is NativeFlowCore.LibraryState.Loaded -> {
                    body("Native engine: ${lib.version}", OK_COLOR)
                    // BuildConfig.CAP_* must equal nativeCapabilities() (D17 §1.4).
                    NativeFlowCore.capabilities()?.let { c ->
                        if (c.local != app.caps.local || c.remote != app.caps.remote) body(
                            "Native library flavour mismatch: this is the ${app.caps.flavor} APK but libflow_ffi.so was built with " +
                                "local=${c.local} remote=${c.remote} (tools/build-native.sh --flavor ${app.caps.flavor}).",
                            BAD_COLOR,
                        )
                    }
                }
                is NativeFlowCore.LibraryState.Unavailable -> body(
                    "Native engine unavailable. ${lib.message}\n\nThis build was packaged without libflow_ffi.so " +
                        "(see apps/flow-android/README.md). Dictation needs it, or turn on the mock engine in Settings to test the shell.",
                    BAD_COLOR,
                )
            }
            val st = app.engine.state.value
            body(
                when (st) {
                    is EngineHolder.State.Ready -> "Loaded: ${st.description}"
                    EngineHolder.State.Loading -> "Loading model…"
                    is EngineHolder.State.Failed -> st.message
                    EngineHolder.State.NotLoaded -> "Not loaded"
                },
                if (st is EngineHolder.State.Ready) OK_COLOR else null,
            )
            // 2026-10-23: the language the next dictation really uses (unset follows the phone).
            val langSetting = app.settings.load().language
            body("Language: ${dev.flowclone.data.LanguageDefault.label(langSetting, app.effectiveLanguage())}").linkTo(SettingsPages.Page.LANGUAGE)
            if (st is EngineHolder.State.Ready) when (val l = st.llm) {
                EnginePlan.LlmStatus.Off -> body("AI cleanup: off")
                EnginePlan.LlmStatus.NoModel -> body("AI cleanup: on, but no GGUF model imported", BAD_COLOR)
                is EnginePlan.LlmStatus.Loaded -> body("AI cleanup: ${l.name}", OK_COLOR)
                is EnginePlan.LlmStatus.Failed -> body("AI cleanup failed to load, using plain transcripts: ${l.message}", BAD_COLOR)
            }.linkTo(SettingsPages.Page.CLEANUP)
            if (app.caps.local) button("Speech model…") { startActivity(Intent(this@MainActivity, ModelImportActivity::class.java)) }
            if (app.dev.visible) button("Developer settings (engines, server)") { startActivity(Intent(this@MainActivity, DeveloperActivity::class.java)) }

            heading("1. Microphone")
            val mic = granted(Manifest.permission.RECORD_AUDIO)
            body(if (mic) "Granted" else "Needed to hear you. Audio is processed on the phone and never stored" +
                if (app.dev.keepDebugAudio) " (except: Developer → Keep last dictation audio is on)." else ".", if (mic) OK_COLOR else BAD_COLOR)
            if (!mic) button("Grant microphone") { requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1) }

            heading("2. Display over other apps")
            val overlay = Settings.canDrawOverlays(this@MainActivity)
            body(if (overlay) "Allowed" else "Needed for the dictation bubble.", if (overlay) OK_COLOR else BAD_COLOR)
            if (!overlay) button("Allow display over other apps") {
                startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
            }

            heading("3. Accessibility service")
            when (app.accessibilityHealth()) {
                AccessibilityHealth.Status.OK -> body("On", OK_COLOR)
                AccessibilityHealth.Status.STARTING -> body("Turned on, connecting…")
                AccessibilityHealth.Status.DISABLED -> {
                    body(
                        "Off. Flow Clone uses Accessibility to notice when your keyboard is open, show the bubble, read up to 500 " +
                            "characters around the cursor so the transcript fits, and type the transcript into the focused field. " +
                            "It never reads or types into password, PIN, number or phone fields, and stays hidden in banking apps. " +
                            "Nothing it sees leaves the phone.\n\nIf the switch is greyed out: Settings → Apps → Flow Clone → ⋮ → Allow restricted settings.",
                        BAD_COLOR,
                    )
                    button("Open Accessibility settings") { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
                }
                AccessibilityHealth.Status.GHOST -> {
                    body(
                        "Switched on but not responding. Android or the phone's battery manager stopped it without turning " +
                            "the switch off. Turn Flow Clone OFF and then ON again in Accessibility settings. Then do step 4 and the " +
                            "phone-specific step below so it stays alive.",
                        BAD_COLOR,
                    )
                    button("Open Accessibility settings") { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
                }
            }

            heading("4. Battery optimisation")
            val pm = getSystemService(PowerManager::class.java)
            val exempt = pm?.isIgnoringBatteryOptimizations(packageName) == true
            body(
                if (exempt) "Unrestricted" else
                    "Recommended. Without it Android may stop Flow Clone in the background, and the bubble disappears until you reopen the app.",
                if (exempt) OK_COLOR else BAD_COLOR,
            )
            if (!exempt) button("Allow running in the background") { requestBatteryExemption() }

            heading("5. Notifications")
            val notif = granted(Manifest.permission.POST_NOTIFICATIONS)
            body(if (notif) "Allowed" else "Optional: status, session limit, and the alert when Accessibility needs fixing.", if (notif) OK_COLOR else null)
            if (!notif) button("Allow notifications") { requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 2) }

            val oem = OemGuidance.forManufacturer(Build.MANUFACTURER)
            heading("Keep it running: ${oem.title}")
            body(oem.steps)
            oem.extraWarning?.let { body(it, BAD_COLOR) }
            if (oem.intents.isNotEmpty()) button("Open phone settings") { openOem(oem) }

            heading("Run")
            button("Start / show bubble") { startBubble() }
            button("Stop bubble") {
                runCatching { startService(Intent(this@MainActivity, FlowService::class.java).setAction(FlowService.ACTION_STOP)) }
                main.postDelayed({ render() }, 300)
            }

            heading("More")
            button("Settings") { startActivity(Intent(this@MainActivity, SettingsActivity::class.java)) }
            button("Dictionary & snippets") { startActivity(Intent(this@MainActivity, DictionaryActivity::class.java)) }
            button("History") { startActivity(Intent(this@MainActivity, HistoryActivity::class.java)) }
        }
    }

    /** Status line → its Settings page (only pages this build has); one 48 dp target for TalkBack. */
    private fun android.widget.TextView.linkTo(p: SettingsPages.Page) {
        if (!SettingsPages.visible(p, app.caps)) return
        val label = text.toString()
        text = "$label  ›"
        contentDescription = "$label. Opens ${SettingsPages.TITLES.getValue(p)} settings"
        minHeight = dp(48)
        gravity = android.view.Gravity.CENTER_VERTICAL
        isClickable = true; isFocusable = true
        setOnClickListener { SettingsActivity.open(this@MainActivity, p) }
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
        Toast.makeText(this, "Couldn't open that screen on this phone; opening Flow Clone's app settings.", Toast.LENGTH_LONG).show()
        runCatching { startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))) }
    }

    private fun startBubble() {
        if (!Settings.canDrawOverlays(this)) {
            Toast.makeText(this, "Allow display over other apps first (step 2).", Toast.LENGTH_LONG).show()
            return
        }
        runCatching { startForegroundService(Intent(this, FlowService::class.java).setAction(FlowService.ACTION_SHOW)) }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        render()
    }
}
