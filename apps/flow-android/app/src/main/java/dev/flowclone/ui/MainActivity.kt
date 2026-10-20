package dev.flowclone.ui

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import dev.flowclone.EngineHolder
import dev.flowclone.FlowApp
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.service.FlowService

/** Onboarding checklist + engine status (10-clone-spec-android §2: mic → overlay → a11y → notifications). */
class MainActivity : Activity() {
    private val app get() = FlowApp.of(this)

    override fun onResume() {
        super.onResume()
        render()
    }

    private fun granted(p: String) = checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    private fun render() {
        page {
            heading("Flow Clone")
            body("On-device voice dictation. This app has no internet permission: audio, transcripts and history never leave the phone.")

            heading("Speech engine")
            when (val lib = app.engine.libraryState()) {
                is NativeFlowCore.LibraryState.Loaded -> body("Native engine: ${lib.version}", OK_COLOR)
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
            button("Speech model…") { startActivity(Intent(this@MainActivity, ModelImportActivity::class.java)) }

            heading("Permissions")
            val mic = granted(Manifest.permission.RECORD_AUDIO)
            body(if (mic) "Microphone: granted" else "Microphone: needed", if (mic) OK_COLOR else BAD_COLOR)
            if (!mic) button("Grant microphone") { requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1) }

            val overlay = Settings.canDrawOverlays(this@MainActivity)
            body(if (overlay) "Display over other apps: allowed" else "Display over other apps: needed for the bubble", if (overlay) OK_COLOR else BAD_COLOR)
            if (!overlay) button("Allow display over other apps") {
                startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
            }

            val a11y = app.accessibility != null
            body(
                if (a11y) "Accessibility service: on" else
                    "Accessibility service: off. Flow Clone uses it to see when the keyboard is open and to type the transcript into the focused field. It never reads or types into password, PIN, number or phone fields, and stays hidden in banking apps.",
                if (a11y) OK_COLOR else BAD_COLOR,
            )
            if (!a11y) button("Open Accessibility settings") { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }

            val notif = granted(Manifest.permission.POST_NOTIFICATIONS)
            body(if (notif) "Notifications: allowed" else "Notifications: optional (status + session limit)", if (notif) OK_COLOR else null)
            if (!notif) button("Allow notifications") { requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 2) }

            heading("Run")
            button("Start / show bubble") { startBubble() }

            heading("More")
            button("Settings") { startActivity(Intent(this@MainActivity, SettingsActivity::class.java)) }
            button("Dictionary & snippets") { startActivity(Intent(this@MainActivity, DictionaryActivity::class.java)) }
            button("History") { startActivity(Intent(this@MainActivity, HistoryActivity::class.java)) }
        }
    }

    private fun startBubble() {
        if (!Settings.canDrawOverlays(this)) return
        runCatching { startForegroundService(Intent(this, FlowService::class.java).setAction(FlowService.ACTION_SHOW)) }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        render()
    }
}
