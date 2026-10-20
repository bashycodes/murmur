package dev.flowclone.service

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.ComponentCallbacks2
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioManager
import android.media.ToneGenerator
import android.os.IBinder
import android.os.PersistableBundle
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.provider.Settings
import android.util.Log
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.bubble.BubbleButton
import dev.flowclone.bubble.BubbleOverlay
import dev.flowclone.bubble.BubbleUi
import dev.flowclone.bubble.BubbleView
import dev.flowclone.core.CommitResult
import dev.flowclone.core.FieldContext
import dev.flowclone.data.HistoryRow
import dev.flowclone.session.DictationController
import dev.flowclone.session.DictationState
import dev.flowclone.session.FeedbackEvent
import dev.flowclone.ui.MainActivity
import dev.flowclone.audio.MicRecorder
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/**
 * Foreground service hosting the bubble, recorder and warm engine (07 §6).
 * Starts as `specialUse`; re-promotes to `microphone|specialUse` on the bubble touch that
 * starts recording (an overlay touch is a while-in-use exemption on Android 14+).
 */
class FlowService : Service() {
    companion object {
        const val ACTION_SHOW = "dev.flowclone.action.SHOW"
        const val ACTION_RELOAD_ENGINE = "dev.flowclone.action.RELOAD_ENGINE"
        private const val CHANNEL_SERVICE = "flow_service"
        private const val CHANNEL_LIMIT = "flow_session_limit"
        private const val NOTIF_ID = 1001
        private const val NOTIF_LIMIT_ID = 3001
        private const val COPY_DELAY_MS = 2000L
        private const val COPY_AUTOHIDE_MS = 5000L
        private const val TAG = "FlowService"
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val app get() = FlowApp.of(this)
    private lateinit var controller: DictationController
    private lateinit var overlay: BubbleOverlay
    private val showCopy = MutableStateFlow(false)
    @Volatile private var lastText: String? = null
    private var copyJob: Job? = null
    private var tone: ToneGenerator? = null
    private var micPromoted = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannels()
        if (!promote(withMic = false)) { stopSelf(); return }
        tone = runCatching { ToneGenerator(AudioManager.STREAM_NOTIFICATION, 60) }.getOrNull()
        controller = DictationController(
            scope = scope, audio = MicRecorder(), host = Host(),
            core = { app.engine.current() }, commitDispatcher = Dispatchers.Default,
            cap = app.settings.load().sessionCap(), now = SystemClock::elapsedRealtime,
        )
        overlay = BubbleOverlay(this, app.settings.bubblePosition, BubbleListener())
        app.engine.ensureLoaded()
        scope.launch { app.fieldFocus.collect { controller.onFieldFocusChanged(it != null) } }
        scope.launch {
            combine(controller.state, controller.level, showCopy) { s, l, c -> BubbleUi(s, c && s == DictationState.Ready, l) }
                .collect { if (::overlay.isInitialized) overlay.render(it) }
        }
        scope.launch {
            controller.state.collect { s ->
                if (s is DictationState.Error && s.kind == DictationState.ErrorKind.FailedToTranscribe) toast(R.string.toast_transcribe_failed)
                if (s is DictationState.Error && s.kind is DictationState.ErrorKind.EngineUnavailable) toast(s.kind.message)
                if (s is DictationState.Error && s.kind == DictationState.ErrorKind.MicUnavailable) toast(R.string.toast_mic_unavailable)
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!Settings.canDrawOverlays(this)) {
            toast(R.string.toast_no_overlay)
            stopSelf()
            return START_NOT_STICKY
        }
        if (!::controller.isInitialized) return START_NOT_STICKY
        overlay.attach()
        when (intent?.action) {
            ACTION_RELOAD_ENGINE -> { controller.cap = app.settings.load().sessionCap(); app.engine.reload() }
            else -> Unit
        }
        return START_STICKY
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        @Suppress("DEPRECATION")
        val low = level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW && level != ComponentCallbacks2.TRIM_MEMORY_UI_HIDDEN
        if (low && ::controller.isInitialized && controller.state.value !is DictationState.Recording &&
            controller.state.value !is DictationState.Processing
        ) app.engine.unload()
    }

    override fun onDestroy() {
        if (::controller.isInitialized) controller.cancel()
        if (::overlay.isInitialized) overlay.detach()
        tone?.release()
        scope.cancel()
        super.onDestroy()
    }

    // ---- foreground ----

    private fun hasMic() = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private fun promote(withMic: Boolean): Boolean {
        val special = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
        val want = if (withMic && hasMic()) special or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else special
        return try {
            startForeground(NOTIF_ID, serviceNotification(), want)
            micPromoted = want and ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE != 0
            true
        } catch (e: Exception) { // ForegroundServiceStartNotAllowedException / SecurityException
            Log.w(TAG, "startForeground($want) refused", e)
            if (want != special) runCatching { startForeground(NOTIF_ID, serviceNotification(), special) }.isSuccess else false
        }
    }

    private fun createChannels() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_SERVICE, getString(R.string.notif_channel_service), NotificationManager.IMPORTANCE_LOW).apply {
                description = getString(R.string.notif_channel_service_desc); setShowBadge(false)
            },
        )
        nm.createNotificationChannel(NotificationChannel(CHANNEL_LIMIT, getString(R.string.notif_channel_limit), NotificationManager.IMPORTANCE_HIGH))
    }

    private fun openAppIntent() = PendingIntent.getActivity(
        this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun serviceNotification(): Notification = Notification.Builder(this, CHANNEL_SERVICE)
        .setContentTitle(getString(R.string.notif_title))
        .setContentText(getString(R.string.notif_text))
        .setSmallIcon(R.drawable.ic_flow)
        .setOngoing(true)
        .setContentIntent(openAppIntent())
        .build()

    private fun limitNotification() {
        val n = Notification.Builder(this, CHANNEL_LIMIT)
            .setContentTitle(getString(R.string.notif_limit_title))
            .setContentText(getString(R.string.notif_limit_text, app.settings.load().capMinutes))
            .setSmallIcon(R.drawable.ic_flow)
            .setAutoCancel(true)
            .setContentIntent(openAppIntent())
            .build()
        runCatching { getSystemService(NotificationManager::class.java).notify(NOTIF_LIMIT_ID, n) }
    }

    // ---- feedback ----

    private fun toast(res: Int) = Toast.makeText(this, res, Toast.LENGTH_SHORT).show()
    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_LONG).show()

    private fun haptic(effect: Int) {
        runCatching { getSystemService(Vibrator::class.java)?.vibrate(VibrationEffect.createPredefined(effect)) }
    }

    private fun copySensitive(text: String) {
        val cm = getSystemService(ClipboardManager::class.java) ?: return
        val clip = ClipData.newPlainText("Flow Clone", text).apply {
            description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
        }
        cm.setPrimaryClip(clip)
    }

    private inner class Host : DictationController.Host {
        override fun feedback(e: FeedbackEvent) {
            when (e) {
                FeedbackEvent.StartSound -> tone?.startTone(ToneGenerator.TONE_PROP_BEEP, 80)
                FeedbackEvent.StopSound -> tone?.startTone(ToneGenerator.TONE_PROP_ACK, 80)
                FeedbackEvent.CancelHaptic -> haptic(VibrationEffect.EFFECT_DOUBLE_CLICK)
                FeedbackEvent.CapWarning -> { haptic(VibrationEffect.EFFECT_HEAVY_CLICK); toast(R.string.toast_cap_warning) }
                FeedbackEvent.CapReached -> { toast(R.string.toast_cap_reached); limitNotification() }
                FeedbackEvent.NeedMicPermission -> {
                    toast(R.string.toast_need_mic)
                    startActivity(Intent(this@FlowService, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                }
                FeedbackEvent.PasteRetryFailed -> toast(R.string.toast_paste_failed)
            }
        }

        override suspend fun insert(text: String): Boolean {
            lastText = text
            copyJob?.cancel()
            copyJob = scope.launch { delay(COPY_DELAY_MS); showCopy.value = true }
            val bridge = app.accessibility
            val ok = if (bridge != null) bridge.insertTextAndVerify(text) else { copySensitive(text); false }
            copyJob?.cancel()
            if (showCopy.value || !ok) {
                showCopy.value = true
                copyJob = scope.launch { delay(COPY_AUTOHIDE_MS); showCopy.value = false }
            }
            return ok
        }

        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?) {
            val retention = app.settings.load().retention
            val row = HistoryRow(0, System.currentTimeMillis(), appId, result.status.name, result.asrText, result.finalText, inserted, result.timings.audioMs)
            scope.launch(Dispatchers.IO) { runCatching { app.history.add(row, retention) } }
        }

        override fun hasMicPermission() = hasMic()

        override fun captureContext(): FieldContext? = app.accessibility?.captureFieldContext()

        override fun sessionConfig() = app.settings.load().toSessionConfig(app.dictionary.all())

        override fun onRecordingActive(active: Boolean) {
            if (active && !micPromoted) promote(withMic = true)
            app.accessibility?.onDictationActive(active)
        }
    }

    /** On the touch that starts recording: promote FGS to microphone, pick up settings changes. */
    private fun beforeStart() {
        if (!micPromoted) promote(withMic = true)
        controller.cap = app.settings.load().sessionCap()
    }

    private inner class BubbleListener : BubbleView.Listener {
        override fun onTap(button: BubbleButton) {
            val s = controller.state.value
            when (button) {
                BubbleButton.CANCEL -> controller.cancel()
                BubbleButton.DONE -> { haptic(VibrationEffect.EFFECT_CLICK); controller.release() }
                BubbleButton.PASTE -> controller.pasteRetry()
                BubbleButton.COPY -> lastText?.let { copySensitive(it); toast(R.string.toast_copied); showCopy.value = false }
                BubbleButton.DISMISS -> {
                    if (s is DictationState.Error && s.kind is DictationState.ErrorKind.EngineUnavailable)
                        startActivity(Intent(this@FlowService, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    controller.dismissError()
                }
                BubbleButton.MAIN -> when (s) {
                    DictationState.Ready, DictationState.Idle -> {
                        haptic(VibrationEffect.EFFECT_CLICK)
                        beforeStart()
                        controller.toggle(hold = false)
                    }
                    is DictationState.Error -> controller.dismissError()
                    else -> Unit
                }
            }
        }

        override fun onHoldStart() {
            haptic(VibrationEffect.EFFECT_HEAVY_CLICK)
            if (controller.state.value is DictationState.Ready || controller.state.value is DictationState.Error) beforeStart()
            controller.toggle(hold = true)
        }

        override fun onHoldEnd() {
            haptic(VibrationEffect.EFFECT_CLICK)
            controller.release()
        }

        override fun onDragStart() = haptic(VibrationEffect.EFFECT_TICK)
        override fun onDragBy(dx: Float, dy: Float) = Unit
        override fun onDragEnd() = Unit
    }
}
