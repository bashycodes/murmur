package dev.flowclone.service

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.app.KeyguardManager
import android.content.BroadcastReceiver
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioManager
import android.media.ToneGenerator
import android.os.Build
import android.os.IBinder
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
import dev.flowclone.bubble.BubblePresence
import dev.flowclone.bubble.PreviewContent
import dev.flowclone.bubble.ShakeDetector
import dev.flowclone.bubble.BubbleUi
import dev.flowclone.bubble.BubbleView
import dev.flowclone.core.CommitResult
import dev.flowclone.core.FieldContext
import dev.flowclone.data.HistoryRow
import dev.flowclone.health.AccessibilityHealth
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
        const val ACTION_UNSNOOZE = "dev.flowclone.action.UNSNOOZE"
        const val ACTION_STOP = "dev.flowclone.action.STOP"
        /** Tap on the "Dictation not inserted" notification: copy that text (2026-10-24). */
        const val ACTION_COPY_UNSENT = "dev.flowclone.action.COPY_UNSENT"
        private const val CHANNEL_SERVICE = "flow_service"
        private const val CHANNEL_LIMIT = "flow_session_limit"
        const val CHANNEL_STATUS = "flow_status"
        private const val NOTIF_ID = 1001
        private const val NOTIF_LIMIT_ID = 3001
        private const val NOTIF_SNOOZE_ID = 789
        private const val NOTIF_UNSENT_ID = 3002
        private const val HEALTH_TICK_MS = 15_000L

        /** For the a11y-side watchdog (same process). */
        @Volatile var running = false; private set
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
    /** The newest dictation that did not go in, until copied (2026-10-24: never lose a dictation). */
    @Volatile private var unsentText: String? = null
    private var copyJob: Job? = null
    private var tone: ToneGenerator? = null
    private var micPromoted = false
    private lateinit var presence: BubblePresence
    private val repair = MutableStateFlow(false)
    private var health = AccessibilityHealth.Status.OK
    private val healthGate = AccessibilityHealth.ChangeGate(health)
    private val shake = ShakeDetector()
    private var shakeRegistered = false
    /** D21: insertion status + steps of the current dictation (all its text runs), for the debug log. */
    private val insertSteps = java.util.Collections.synchronizedList(ArrayList<String>())

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannels()
        if (!promote(withMic = false)) { stopSelf(); return }
        running = true
        tone = runCatching { ToneGenerator(AudioManager.STREAM_NOTIFICATION, 60) }.getOrNull()
        controller = DictationController(
            scope = scope, audio = MicRecorder(), host = Host(),
            core = { app.engine.current() }, commitDispatcher = Dispatchers.Default,
            cap = app.settings.load().sessionCap(), now = SystemClock::elapsedRealtime,
            // Review C2: start_session can connect to the server (TCP + TLS + hello): never on Main.
            startDispatcher = Dispatchers.IO,
        )
        presence = BubblePresence(scope) { app.settings.load().shrinkBubble }
        overlay = BubbleOverlay(this, app.settings.bubblePosition, BubbleListener(), ::snoozeDrop)
        app.engine.ensureLoaded()
        scope.launch { app.fieldFocus.collect { controller.onFieldFocusChanged(it != null) } }
        scope.launch { controller.state.collect { presence.onState(it) } }
        scope.launch {
            combine(controller.state, controller.level, showCopy, presence.presence, repair) { s, l, c, p, r ->
                BubbleUi(s, c && s == DictationState.Ready, l, minimized = p.minimized, repair = r, visible = presence.visible(s))
            }.collect { if (::overlay.isInitialized) overlay.render(it) }
        }
        scope.launch {
            combine(controller.state, controller.partial, controller.hint, presence.presence, controller.route) { s, p, h, _, r ->
                // D18 C7: "C" whenever the last dictation went to a cloud provider, even with the badge off.
                val badge = dev.flowclone.core.DevText.bubbleBadge(app.lastRoute.value?.result?.route, app.dev.load().showEngineBadge)
                // Live route ("via phone | server | provider"), relabelled on failover.
                val route = dev.flowclone.bubble.RouteLabel.of(r) { id -> app.dev.load().cloud.provider(id)?.displayName }
                PreviewContent.line(s, p, h, app.settings.load().livePreview, badge, route)
            }.collect { overlay.renderPreview(it) }
        }
        scope.launch {
            while (true) { checkHealth(); updateShakeListener(); delay(HEALTH_TICK_MS) }
        }
        registerReceiver(screenReceiver, IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_ON); addAction(Intent.ACTION_SCREEN_OFF); addAction(Intent.ACTION_USER_PRESENT)
        }, RECEIVER_NOT_EXPORTED)
        updateShakeListener()
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
        when (intent?.action) {
            ACTION_STOP -> {
                app.settings.serviceStoppedByUser = true
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_RELOAD_ENGINE -> { controller.cap = app.settings.load().sessionCap(); app.engine.reload(); updateShakeListener() }
            ACTION_UNSNOOZE -> { presence.unsnooze(); cancelSnoozeNotification() }
            ACTION_COPY_UNSENT -> {
                unsentText?.let { copySensitive(it); toast(R.string.toast_copied) }
                unsentText = null
                runCatching { getSystemService(NotificationManager::class.java).cancel(NOTIF_UNSENT_ID) }
                return START_STICKY
            }
            else -> app.settings.serviceStoppedByUser = false
        }
        overlay.attach()
        checkHealth()
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
        running = false
        runCatching { unregisterReceiver(screenReceiver) }
        unregisterShake()
        if (::controller.isInitialized) controller.cancel()
        if (::overlay.isInitialized) overlay.detach()
        tone?.release()
        scope.cancel()
        super.onDestroy()
    }

    // ---- foreground ----

    private fun hasMic() = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    /** FGS types: `specialUse` exists from API 34; on 33 the host runs untyped and adds `microphone` when recording. */
    private fun promote(withMic: Boolean): Boolean {
        val special = if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0
        val want = if (withMic && hasMic()) special or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else special
        fun start(type: Int) =
            if (type == 0) startForeground(NOTIF_ID, serviceNotification()) else startForeground(NOTIF_ID, serviceNotification(), type)
        return try {
            start(want)
            micPromoted = want and ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE != 0
            true
        } catch (e: Exception) { // ForegroundServiceStartNotAllowedException / SecurityException
            Log.w(TAG, "startForeground($want) refused", e)
            if (want != special) runCatching { start(special) }.isSuccess else false
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
        nm.createNotificationChannel(NotificationChannel(CHANNEL_STATUS, getString(R.string.notif_channel_status), NotificationManager.IMPORTANCE_DEFAULT))
    }

    private fun openAppIntent() = PendingIntent.getActivity(
        this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun serviceNotification(): Notification = Notification.Builder(this, CHANNEL_SERVICE)
        .setContentTitle(getString(R.string.notif_title))
        .setContentText(
            getString(
                when (health) {
                    AccessibilityHealth.Status.GHOST -> R.string.notif_text_ghost
                    AccessibilityHealth.Status.DISABLED -> R.string.notif_text_a11y_off
                    else -> R.string.notif_text
                },
            ),
        )
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

    /** Tap-to-copy notification for a dictation that did not go in; text only on tap (lock screen). */
    private fun unsentNotification(text: String, outcome: String) {
        unsentText = text
        val n = Notification.Builder(this, CHANNEL_STATUS)
            .setContentTitle(dev.flowclone.session.NotInsertedNotice.TITLE)
            .setContentText(dev.flowclone.session.NotInsertedNotice.text(outcome, text))
            .setSmallIcon(R.drawable.ic_flow)
            .setAutoCancel(true)
            .setContentIntent(
                PendingIntent.getService(
                    this, 2, Intent(this, FlowService::class.java).setAction(ACTION_COPY_UNSENT),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
            .build()
        runCatching { getSystemService(NotificationManager::class.java).notify(NOTIF_UNSENT_ID, n) }
    }

    // ---- health, snooze, shake ----

    /** 15 s tick: ghost / disabled accessibility → Repair bubble + notification text. */
    private fun checkHealth() {
        val h = app.accessibilityHealth()
        if (!healthGate.changed(h)) return // unchanged: no notification re-post, no bubble relayout
        health = h
        val needsRepair = h == AccessibilityHealth.Status.GHOST || h == AccessibilityHealth.Status.DISABLED
        presence.repair = needsRepair
        repair.value = needsRepair
        Log.i(TAG, "accessibility health: $h")
        app.debugLog.event("a11y_health", h.name)
        runCatching { getSystemService(NotificationManager::class.java).notify(NOTIF_ID, serviceNotification()) }
    }

    private fun snoozeDrop(): Boolean {
        if (!presence.snooze()) return false
        haptic(VibrationEffect.EFFECT_CLICK)
        toast(R.string.toast_snoozed)
        val n = Notification.Builder(this, CHANNEL_STATUS)
            .setContentTitle(getString(R.string.notif_snooze_title))
            .setContentText(getString(R.string.notif_snooze_text))
            .setSmallIcon(R.drawable.ic_flow)
            .setAutoCancel(true)
            .setTimeoutAfter(BubblePresence.SNOOZE_MS)
            .setContentIntent(
                PendingIntent.getService(
                    this, 1, Intent(this, FlowService::class.java).setAction(ACTION_UNSNOOZE),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
            .build()
        runCatching { getSystemService(NotificationManager::class.java).notify(NOTIF_SNOOZE_ID, n) }
        return true
    }

    private fun cancelSnoozeNotification() = runCatching { getSystemService(NotificationManager::class.java).cancel(NOTIF_SNOOZE_ID) }

    private val shakeListener = object : SensorEventListener {
        override fun onSensorChanged(e: SensorEvent) {
            if (!shake.onSample(e.values[0], e.values[1], e.values[2], SystemClock.elapsedRealtime())) return
            val locked = getSystemService(KeyguardManager::class.java)?.isKeyguardLocked == true
            when (presence.onShake(locked)) {
                BubblePresence.ShakeOutcome.UNSNOOZED -> {
                    haptic(VibrationEffect.EFFECT_CLICK); toast(R.string.toast_back); cancelSnoozeNotification()
                }
                BubblePresence.ShakeOutcome.REVEALED -> haptic(VibrationEffect.EFFECT_TICK)
                BubblePresence.ShakeOutcome.IGNORED -> Unit
            }
        }
        override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit
    }

    /** Listen only while the screen is on and the setting allows it (battery). */
    private fun updateShakeListener() {
        val interactive = getSystemService(android.os.PowerManager::class.java)?.isInteractive != false
        if (interactive && app.settings.load().shakeToReveal) registerShake() else unregisterShake()
    }

    private fun registerShake() {
        if (shakeRegistered) return
        val sm = getSystemService(SensorManager::class.java) ?: return
        val acc = sm.getDefaultSensor(Sensor.TYPE_ACCELEROMETER) ?: return
        shakeRegistered = sm.registerListener(shakeListener, acc, SensorManager.SENSOR_DELAY_UI)
    }

    private fun unregisterShake() {
        if (!shakeRegistered) return
        getSystemService(SensorManager::class.java)?.unregisterListener(shakeListener)
        shakeRegistered = false
    }

    /** Screen on/off gate the shake sensor; unlock re-checks the overlay and accessibility health (07 §6 Recovery). */
    private val screenReceiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            when (i.action) {
                Intent.ACTION_SCREEN_OFF -> unregisterShake()
                Intent.ACTION_SCREEN_ON -> updateShakeListener()
                Intent.ACTION_USER_PRESENT -> {
                    updateShakeListener()
                    if (::overlay.isInitialized) overlay.ensureAttached()
                    checkHealth()
                }
            }
        }
    }

    // ---- feedback ----

    private fun toast(res: Int) = Toast.makeText(this, res, Toast.LENGTH_SHORT).show()
    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_LONG).show()

    private fun haptic(effect: Int) {
        runCatching { getSystemService(Vibrator::class.java)?.vibrate(VibrationEffect.createPredefined(effect)) }
    }

    /** S16: sensitive clip, taken off the clipboard again after a minute (or once the text is inserted). */
    private fun copySensitive(text: String) = app.clipboard.copy(text)

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
                FeedbackEvent.NoSpeech -> Unit
                FeedbackEvent.TargetChanged -> toast(getString(R.string.toast_target_changed))
                FeedbackEvent.TargetProtected -> toast(getString(R.string.toast_target_protected))
                FeedbackEvent.MicLost -> toast(R.string.toast_mic_lost)
            }
        }

        override suspend fun insert(text: String): Boolean {
            lastText = text
            copyJob?.cancel()
            copyJob = scope.launch { delay(COPY_DELAY_MS); showCopy.value = true }
            val bridge = app.accessibility
            val ok = if (bridge != null) bridge.insertTextAndVerify(text) else { copySensitive(text); false }
            runCatching { insertSteps += if (bridge != null) bridge.lastInsertSteps else listOf("NO_ACCESSIBILITY_SERVICE") }
            if (ok) app.clipboard.onInserted() // S16: a failed earlier text leaves the clipboard now
            copyJob?.cancel()
            if (showCopy.value || !ok) {
                showCopy.value = true
                copyJob = scope.launch { delay(COPY_AUTOHIDE_MS); showCopy.value = false }
            }
            return ok
        }

        override suspend fun sendKey(key: dev.flowclone.core.PasteKey): Boolean = app.accessibility?.sendKey(key) ?: false

        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?) {
            app.lastRoute.value = dev.flowclone.data.LastDictation.of(result, System.currentTimeMillis())
            // D18: persist this month's cloud counters; say so when the cloud failed and another engine took over.
            if (result.route?.engine == "cloud") scope.launch(Dispatchers.IO) { runCatching { app.engine.persistCloudUsage() } }
            dev.flowclone.data.CloudText.fallbackMessage(result.route, app.dev.load().effective(app.caps).cloudFirst)?.let { m ->
                scope.launch(Dispatchers.Main) { toast(m) }
            }
            val retention = app.settings.load().retention
            val t = result.timings
            val row = HistoryRow(
                0, System.currentTimeMillis(), appId, result.status.name, result.asrText, result.historyText, inserted, t.audioMs,
                fallbackReason = result.fallbackReason, asrMs = t.asrMs, llmMs = t.llmMs, totalMs = t.totalMs,
            )
            Log.i(TAG, dev.flowclone.debug.CommitLine.of(result))
            scope.launch(Dispatchers.IO) { runCatching { app.history.add(row, retention) } }
        }

        override fun hasMicPermission() = hasMic()

        override fun clientFallbackNote(): String? = app.engine.clientNote()

        override fun onNotInserted(text: String, outcome: String) {
            scope.launch(Dispatchers.Main) { unsentNotification(text, outcome) }
        }

        /** D21: redact protected targets, attach the insert steps, enqueue. Never blocks, never throws. */
        override fun onDictationTrace(trace: dev.flowclone.debug.DictationTrace) {
            try {
                if (!app.debugLog.isEnabled()) return
                val steps = synchronized(insertSteps) { insertSteps.toList() }
                val ready = app.engine.state.value as? dev.flowclone.EngineHolder.State.Ready
                val (engine, model) = dev.flowclone.debug.DebugEngine.of(
                    trace.route, ready?.mock == true, ready?.asr ?: ready?.onDemandAsr, app.dev.serverAsrModel, trace.model,
                )
                val t = dev.flowclone.debug.DebugRedact.apply(
                    trace.copy(
                        insert = trace.insert.copy(steps = steps), engine = engine, model = model,
                        routeOrder = runCatching { app.dev.load().routeOrderWire(app.caps) }.getOrNull(),
                    ),
                    protectedField = app.accessibility?.startFieldProtected == true,
                    protectedApp = dev.flowclone.debug.DebugRedact.isProtectedApp(trace.target, app.settings.load().userDenylist),
                )
                app.debugLog.dictation(t)
            } catch (_: Throwable) {
                // logging never affects dictation
            }
        }

        override fun keepDictationAudio(): Boolean = app.dev.keepDebugAudio

        /** Never for protected targets; the WAV is written on the debug-audio thread. Never throws. */
        override fun onDictationAudio(trace: dev.flowclone.debug.DictationTrace, pcm: ShortArray): String? = try {
            val keep = app.dev.keepDebugAudio && dev.flowclone.debug.DebugRedact.keepsAudio(
                trace,
                protectedField = app.accessibility?.startFieldProtected == true,
                protectedApp = dev.flowclone.debug.DebugRedact.isProtectedApp(trace.target, app.settings.load().userDenylist),
            )
            if (!keep) null else {
                app.debugAudioWriter.execute { runCatching { app.debugAudio.save(trace.tsMs, pcm) }.onFailure { Log.w(TAG, "debug audio: ${it.message}") } }
                dev.flowclone.debug.DebugAudioStore.nameFor(trace.tsMs)
            }
        } catch (_: Throwable) { null }

        // "Writing style → Apps" overrides the built-in bucket of the target app (D27).
        override fun captureContext(): FieldContext? =
            app.accessibility?.captureFieldContext()?.let { app.settings.load().withAppBucket(it) }

        override fun sessionConfig() = app.settings.load().let { it.toSessionConfig(app.dictionary.all(), language = app.effectiveLanguage(it)) }

        override fun endpointSilenceMs(): Int? = app.settings.load().handsFreeEndpointMs()

        override fun insertTarget(explicit: Boolean) =
            app.accessibility?.insertTarget(explicit) ?: dev.flowclone.session.InsertTarget.OK

        override fun onRecordingActive(active: Boolean) {
            if (active) insertSteps.clear()
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
                BubbleButton.COPY -> {
                    // FailedToPaste (target changed / paste failed): copy that text, not the previous one.
                    val failed = (s as? DictationState.Error)?.kind as? DictationState.ErrorKind.FailedToPaste
                    (failed?.text ?: lastText)?.let { copySensitive(it); toast(R.string.toast_copied); showCopy.value = false }
                }
                BubbleButton.DISMISS -> {
                    if (s is DictationState.Error && s.kind is DictationState.ErrorKind.EngineUnavailable)
                        startActivity(Intent(this@FlowService, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    controller.dismissError()
                }
                BubbleButton.MAIN -> if (repair.value && (s == DictationState.Ready || s == DictationState.Idle)) {
                    startActivity(Intent(this@FlowService, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                } else when (s) {
                    DictationState.Ready, DictationState.Idle -> {
                        presence.touch()
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
            if (repair.value && controller.state.value.let { it == DictationState.Ready || it == DictationState.Idle }) return
            presence.touch()
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
