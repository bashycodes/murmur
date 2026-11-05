package dev.flowclone.service

import dev.flowclone.learn.Anchors
import dev.flowclone.learn.EditLearning
import dev.flowclone.learn.EditWatch
import dev.flowclone.learn.FieldRead
import dev.flowclone.learn.runWatch
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
import dev.flowclone.bubble.ErrorTap
import dev.flowclone.core.CommitResult
import dev.flowclone.core.FieldContext
import dev.flowclone.data.HistoryRow
import dev.flowclone.health.AccessibilityHealth
import dev.flowclone.session.DictationController
import dev.flowclone.session.DictationState
import dev.flowclone.session.FeedbackEvent
import dev.flowclone.ui.MainActivity
import dev.flowclone.ui.PermissionsActivity
import dev.flowclone.audio.AudioSwitch
import dev.flowclone.audio.MicRecorder
import dev.flowclone.audio.WavAudioSource
import dev.flowclone.debug.DebugWav
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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
        /**
         * History → "Insert with the bubble": arm the bubble's ✓ insert with history row
         * [EXTRA_HISTORY_ID] (the text is read here from the store, never carried in the intent).
         */
        const val ACTION_OFFER_INSERT = "dev.flowclone.action.OFFER_INSERT"
        const val EXTRA_HISTORY_ID = "history_id"
        /**
         * D35 History → "Retry with current settings": transcribe row [EXTRA_HISTORY_ID]'s kept
         * recording again with the settings active now. The result becomes the row's newest
         * version; nothing is inserted.
         */
        const val ACTION_RETRY = "dev.flowclone.action.RETRY"
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
    /** The mic, or (debug builds, one dictation) a WAV armed by [playDebugWav]. */
    private val audio = AudioSwitch(MicRecorder())
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
    /** Learn from my edits: the one running field watch, and its "a new dictation started" flag. */
    private var learnJob: Job? = null
    @Volatile private var learnStop: java.util.concurrent.atomic.AtomicBoolean? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannels()
        if (!promote(withMic = false)) { stopSelf(); return }
        running = true
        tone = runCatching { ToneGenerator(AudioManager.STREAM_NOTIFICATION, 60) }.getOrNull()
        controller = DictationController(
            scope = scope, audio = audio, host = Host(),
            core = { app.engine.current() }, commitDispatcher = Dispatchers.Default,
            cap = app.settings.load().sessionCap(), now = SystemClock::elapsedRealtime,
            // Review C2: start_session can connect to the server (TCP + TLS + hello): never on Main.
            startDispatcher = Dispatchers.IO,
            // Load 2 (D55): record while the phone's speech model loads, transcribe it there once loaded.
            loadWait = { app.engine.loadWait() },
        )
        presence = BubblePresence(scope) { app.settings.load().shrinkBubble }
        overlay = BubbleOverlay(this, app.settings.bubblePosition, BubbleListener(), ::snoozeDrop)
        app.engine.ensureLoaded()
        scope.launch { app.fieldFocus.collect { controller.onFieldFocusChanged(it != null) } }
        scope.launch {
            controller.state.collect {
                presence.onState(it)
                app.dictationBusy.value = it is DictationState.Recording || it is DictationState.Processing
            }
        }
        scope.launch {
            // "Bubble size" and "Shrink to a dot" ride along with repair, so either applies to the live bubble at once.
            // Command Mode's ✎ rides along too: shown per the "Command button" setting and the field's selection.
            val command = combine(app.settings.commandButton, app.fieldSelection) { mode, sel -> mode.shows(sel) }
            val repairAndLook = combine(repair, app.settings.bubbleScale, app.settings.shrinkToDot, command) { r, scale, dot, cmd -> Look(r, scale, dot, cmd) }
            combine(controller.state, controller.level, showCopy, presence.presence, repairAndLook) { s, l, c, p, look ->
                BubbleUi(
                    s, c && s == DictationState.Ready, l, minimized = p.minimized, repair = look.repair, visible = presence.visible(s),
                    scale = look.scale, dot = look.dot, showCommand = look.command && s == DictationState.Ready,
                )
            }.collect { if (::overlay.isInitialized) overlay.render(it) }
        }
        scope.launch {
            combine(controller.state, controller.partial, controller.hint, presence.presence, controller.route) { s, p, h, _, r ->
                // D18 C7: "C" whenever the last dictation went to a cloud provider, even with the badge off.
                val badge = dev.flowclone.core.DevText.bubbleBadge(app.lastRoute.value?.result?.route, app.dev.load().showEngineBadge)
                // Live route ("via phone | server | provider"), relabelled on failover.
                val route = dev.flowclone.bubble.RouteLabel.of(r) { id -> if (id == dev.flowclone.data.OpenRouter.ID) dev.flowclone.data.OpenRouter.NAME else null }
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
        // A "still loading" bubble goes back to normal (or shows the real error) once the engine settles.
        scope.launch {
            app.engine.state.collect { e ->
                when (e) {
                    is dev.flowclone.EngineHolder.State.Ready -> controller.engineSettled(failure = null)
                    is dev.flowclone.EngineHolder.State.Failed -> controller.engineSettled(failure = e.message)
                    else -> Unit
                }
            }
        }
        scope.launch {
            controller.state.collect { s ->
                if (s is DictationState.Error && s.kind == DictationState.ErrorKind.FailedToTranscribe) toast(R.string.toast_transcribe_failed)
                if (s is DictationState.Error && s.kind is DictationState.ErrorKind.EngineUnavailable) toast(s.kind.message)
                if (s is DictationState.Error && s.kind is DictationState.ErrorKind.CommandFailed) toast(s.kind.message)
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
            ACTION_OFFER_INSERT -> {
                val id = intent?.getLongExtra(EXTRA_HISTORY_ID, -1) ?: -1
                scope.launch {
                    val text = withContext(Dispatchers.IO) {
                        runCatching { app.history.recent().firstOrNull { it.id == id } }.getOrNull()
                    }?.let(dev.flowclone.data.HistoryPages::copyText)
                    val armed = text != null && controller.offerInsert(text)
                    toast(if (armed) R.string.toast_offer_insert else R.string.toast_offer_insert_busy)
                }
                return START_STICKY
            }
            ACTION_RETRY -> {
                retry(intent?.getLongExtra(EXTRA_HISTORY_ID, -1) ?: -1)
                return START_STICKY
            }
            DebugWav.ACTION -> {
                // Compile-time false in release: the branch is dead code there and no receiver sends it.
                if (dev.flowclone.BuildConfig.DEBUG) playDebugWav(intent)
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
        app.dictationBusy.value = false
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
                    haptic(VibrationEffect.EFFECT_CLICK); cancelSnoozeNotification()
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

    // ---- learn from my edits ----

    /**
     * After an insert, re-read the same field (off Main, once a second, up to 45 s) and turn the
     * user's corrections into Dictionary suggestions. Nothing is added to the dictionary here.
     * A new dictation ends the watch (its last observation still counts, as on Windows);
     * turning the setting off ends it without learning.
     */
    private fun watchForEdits(pasted: String, ctx: FieldContext?, sentKey: Boolean) {
        learnJob?.cancel()
        learnStop?.set(true)
        if (ctx == null || !EditLearning.shouldWatch(app.settings.load().learnFromEdits, pasted, sentKey, true)) return
        val reader = app.accessibility?.editReader() ?: return
        val stop = java.util.concurrent.atomic.AtomicBoolean(false)
        learnStop = stop
        learnJob = scope.launch(Dispatchers.Default) {
            val watch = EditWatch(pasted, Anchors.of(ctx), SystemClock.elapsedRealtime())
            val outcome = runWatch(
                { if (stop.get()) FieldRead.FocusChanged else reader.read() }, watch, SystemClock::elapsedRealtime, { delay(it) },
                stillWanted = { app.settings.load().learnFromEdits },
            )
            val core = (app.engine.state.value as? dev.flowclone.EngineHolder.State.Ready)?.core
            var added: List<dev.flowclone.learn.Suggestion> = emptyList()
            if (core != null && outcome.edited != null) {
                app.suggestions.update { cur ->
                    val r = EditLearning.learn(pasted, outcome, cur, app.dictionary.all(), ctx.appId, System.currentTimeMillis(), core::learnEdit)
                    added = r?.added.orEmpty()
                    r?.next ?: cur
                }
            }
            // Reason and counts only: never the text.
            app.debugLog.event("learn_edit", "end=${outcome.reason.name.lowercase()} edited=${outcome.edited != null} suggested=${added.size}")
            if (added.isNotEmpty()) withContext(Dispatchers.Main) {
                toast(getString(R.string.toast_dictionary_suggestion, added.joinToString(", ") { it.word }))
            }
        }
    }

    // ---- feedback ----

    /**
     * D35: History's "Retry with current settings" for row [id]: its recording through the full
     * pipeline with today's settings (route, engine, language, dictionary, style, AI cleanup),
     * styled for the original app. The text becomes the row's newest version; never inserted.
     */
    private fun retry(id: Long) {
        if (id in app.retrying.value) return
        app.retrying.value = app.retrying.value + id
        scope.launch {
            val msg = try {
                val loaded = withContext(Dispatchers.IO) { runCatching { app.history.row(id) to app.history.audio.load(id) }.getOrNull() }
                val row = loaded?.first
                val pcm = loaded?.second
                if (row == null || pcm == null || row.isCommand) dev.flowclone.data.RetryLogic.GONE
                else {
                    val s = app.settings.load()
                    val bucket = row.appCategory ?: dev.flowclone.policy.AppCategorizer.categorize(row.appId, null).coreName
                    val ctx = s.withAppBucket(FieldContext(appId = row.appId.orEmpty(), appCategory = bucket))
                    when (val out = controller.retry(pcm, ctx.takeIf { row.appId != null })) {
                        is dev.flowclone.session.RetryOutcome.Failed -> out.message
                        is dev.flowclone.session.RetryOutcome.Done -> withContext(Dispatchers.IO) {
                            val r = out.result
                            val applied = runCatching { app.history.applyRetry(id, r) }.getOrNull()
                            if (applied == null) dev.flowclone.data.RetryLogic.GONE
                            else {
                                val (old, _) = applied
                                runCatching {
                                    app.insights.retried(
                                        old.createdAt, dev.flowclone.data.RetryLogic.countedWords(old.status, old.finalText),
                                        dev.flowclone.data.RetryLogic.countedWords(r.status.name, r.historyText), old.audioMs, old.appId,
                                    )
                                }
                                dev.flowclone.data.RetryLogic.DONE
                            }
                        }
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "retry: ${e.message}")
                "Retry failed: ${e.message}"
            } finally {
                app.retrying.value = app.retrying.value - id
            }
            app.historyRevision.value = app.historyRevision.value + 1
            toast(msg)
        }
    }

    private fun toast(res: Int) = Toast.makeText(this, res, Toast.LENGTH_SHORT).show()
    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_LONG).show()

    private fun haptic(effect: Int) {
        runCatching { getSystemService(Vibrator::class.java)?.vibrate(VibrationEffect.createPredefined(effect)) }
    }

    /** S16: sensitive clip, taken off the clipboard again after a minute (or once the text is inserted). */
    private fun copySensitive(text: String) = app.clipboard.copy(text)

    private inner class Host : DictationController.Host {
        override fun feedback(e: FeedbackEvent) {
            FeedbackToast.of(e)?.let { t -> if (t.long) toast(getString(t.res)) else toast(t.res) }
            when (e) {
                FeedbackEvent.StartSound -> tone?.startTone(ToneGenerator.TONE_PROP_BEEP, 80)
                FeedbackEvent.StopSound -> tone?.startTone(ToneGenerator.TONE_PROP_ACK, 80)
                FeedbackEvent.CancelHaptic -> haptic(VibrationEffect.EFFECT_DOUBLE_CLICK)
                FeedbackEvent.CapWarning -> haptic(VibrationEffect.EFFECT_HEAVY_CLICK)
                FeedbackEvent.CapReached -> limitNotification()
                FeedbackEvent.NeedMicPermission ->
                    startActivity(PermissionsActivity.intent(this@FlowService).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                FeedbackEvent.PasteRetryFailed, FeedbackEvent.NoSpeech, FeedbackEvent.TargetChanged,
                FeedbackEvent.TargetProtected, FeedbackEvent.MicLost -> Unit
                // Command Mode: the error tone (the reason is the bubble's toast); the field was not touched.
                FeedbackEvent.CommandFailed -> tone?.startTone(ToneGenerator.TONE_PROP_NACK, 200)
            }
        }

        override fun engineLoading(message: String) = toast(message)

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

        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?) =
            recordHistory(result, inserted, appId, outcome, null)

        override fun keepRetryAudio(): Boolean = app.settings.load().let { it.retryAudioKeep > 0 && it.retention.stores }

        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?, recording: dev.flowclone.session.Recording?) {
            app.lastRoute.value = dev.flowclone.data.LastDictation.of(result, System.currentTimeMillis())
            // D18 / D28: persist this month's cloud counters (speech or cleanup). A cloud failover shows in the live route label.
            if (result.route?.engine == "cloud" || result.route?.llmEngine == "cloud") scope.launch(Dispatchers.IO) { runCatching { app.engine.persistCloudUsage() } }
            val settings = app.settings.load()
            val retention = settings.retention
            val t = result.timings
            val row = HistoryRow(
                0, System.currentTimeMillis(), appId, result.status.name, result.asrText, result.historyText, inserted, t.audioMs,
                fallbackReason = result.fallbackReason, asrMs = t.asrMs, llmMs = t.llmMs, totalMs = t.totalMs,
                insertOutcome = outcome, route = result.route?.engine, appCategory = recording?.appCategory,
            )
            // D35: never from a password / sensitive field, a banking or denylisted app, or a protected target.
            val keep = recording?.takeIf {
                dev.flowclone.data.RetryLogic.keepsRecording(
                    settings.retryAudioKeep, retention, command = false, outcome = outcome,
                    protectedField = app.accessibility?.startFieldProtected == true,
                    protectedApp = dev.flowclone.debug.DebugRedact.isProtectedApp(appId, settings.userDenylist),
                )
            }
            Log.i(TAG, dev.flowclone.debug.CommitLine.of(result))
            scope.launch(Dispatchers.IO) {
                // Insights first, in the same coroutine: its one-time backfill from History must
                // not already see this row, or it would be counted twice. Kept whatever the retention.
                if (result.status != dev.flowclone.core.CommitStatus.ERROR) {
                    runCatching { app.insights.record(row.createdAt, result.historyText, t.audioMs, appId) }
                        .onFailure { Log.w(TAG, "insights: ${it.message}") }
                }
                runCatching {
                    val id = app.history.add(row, retention)
                    if (id != null && keep != null) app.history.recordAudio(id, keep.pcm, settings.retryAudioKeep)
                }.onFailure { Log.w(TAG, "history: ${it.message}") }
            }
        }

        override fun hasMicPermission() = hasMic()

        override fun captureCommand(): dev.flowclone.command.CommandTarget =
            app.accessibility?.captureCommand()
                ?: dev.flowclone.command.CommandTarget.Refused(dev.flowclone.command.CommandTarget.NO_FIELD_MESSAGE)

        override suspend fun applyCommand(target: dev.flowclone.command.CommandTarget.Field, text: String): Boolean {
            val bridge = app.accessibility ?: return false
            val ok = bridge.applyCommand(target, text)
            runCatching { insertSteps += bridge.lastInsertSteps }
            return ok
        }

        /** History only (instruction, original, result): a command is not dictated text, so Insights skips it. */
        override fun recordCommand(record: dev.flowclone.command.CommandRecord) {
            val retention = app.settings.load().retention
            val row = HistoryRow.ofCommand(record, System.currentTimeMillis())
            scope.launch(Dispatchers.IO) { runCatching { app.history.add(row, retention) } }
        }

        override fun clientFallbackNote(): String? = app.engine.clientNote()

        override fun onNotInserted(text: String, outcome: String) {
            scope.launch(Dispatchers.Main) { unsentNotification(text, outcome) }
        }

        /** Debug `llm_off`: a dictation with a transcript but no AI cleanup step says why (not Command Mode). */
        private fun llmOffReason(trace: dev.flowclone.debug.DictationTrace, ready: dev.flowclone.EngineHolder.State.Ready?): String? {
            if (trace.llm != null || trace.raw == null || trace.mode == "command") return null
            val s = app.settings.load()
            val d = app.dev.load()
            return dev.flowclone.data.LlmOutcome.offReason(
                phoneOn = dev.flowclone.data.CleanupEngines.on(d, s, dev.flowclone.data.DevSettings.CleanupRoute.LOCAL),
                phone = ready?.llm,
                offPhone = d.effective(app.caps).offPhoneCleanup,
            )
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
                        languageSource = trace.languageSource ?: languageSource.takeIf { trace.language != null },
                        llmOff = trace.llmOff ?: llmOffReason(trace, ready),
                        llmProvider = trace.llmProvider ?: trace.llmEngine?.takeIf { it == "cloud" }
                            ?.let { runCatching { app.dev.load().cloud.cleanupProvider.wire }.getOrNull() },
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

        override fun sessionConfig() = app.settings.load().let {
            // D28 / D36: an engine off the phone (OpenRouter, your server) may clean even with the
            // on-device AI off; the router re-checks consent, key, circuit, budget and the server's LLM per call.
            val cloudCleanup = app.dev.load().effective(app.caps).offPhoneCleanup
            // D37: read the keyboard now, at dictation start; the source goes on the debug line.
            val lang = app.resolveLanguage(it)
            languageSource = lang.source.wire
            it.toSessionConfig(app.dictionary.all(), language = lang.language, cloudCleanup = cloudCleanup)
        }

        override fun endpointSilenceMs(): Int? = app.settings.load().handsFreeEndpointMs()

        override fun insertTarget(explicit: Boolean) =
            app.accessibility?.insertTarget(explicit) ?: dev.flowclone.session.InsertTarget.OK

        override fun onInserted(pasted: String, context: FieldContext?, sentKey: Boolean) =
            watchForEdits(pasted, context, sentKey)

        override fun onRecordingActive(active: Boolean) {
            if (active) { insertSteps.clear(); learnStop?.set(true) }
            if (active && !micPromoted) promote(withMic = true)
            app.accessibility?.onDictationActive(active)
        }
    }

    /**
     * Debug "WAV as microphone" (debug builds; src/debug WavMicReceiver → [DebugWav.ACTION]): one
     * normal dictation (route order, cleanup, insert guard, insert, debug log with source=wav) on
     * the WAV's audio instead of the mic. The end of the WAV is the release.
     */
    private fun playDebugWav(intent: Intent) {
        val path = intent.getStringExtra(DebugWav.EXTRA_PATH)
        val fast = intent.getBooleanExtra(DebugWav.EXTRA_FAST, false)
        val hold = !intent.getBooleanExtra(DebugWav.EXTRA_TAP, false)
        fun refuse(why: String) {
            Log.w(TAG, "debug wav: $why")
            app.debugLog.event("debug_wav", why)
            toast("WAV dictation: $why")
        }
        scope.launch {
            val file = path?.let { DebugWav.resolve(filesDir, it) }
                ?: return@launch refuse("no WAV file under files/debug/: $path")
            val pcm = withContext(Dispatchers.IO) { runCatching { dev.flowclone.debug.Wav.decode(file.readBytes()) } }
                .getOrElse { return@launch refuse("${file.name}: ${it.message}") }
            DebugWav.refusal(controller.state.value, app.accessibility?.insertTarget(explicit = true))
                ?.let { return@launch refuse(it) }
            val src = WavAudioSource(pcm, realTime = !fast, onEnd = { scope.launch { controller.release() } })
            audio.arm(src)
            beforeStart()
            controller.toggle(hold)
            if (audio.disarm()) refuse("did not start (${controller.state.value::class.simpleName})")
            else Log.i(TAG, "debug wav: ${file.name}, ${pcm.size * 1000L / dev.flowclone.session.SAMPLE_RATE} ms, ${if (fast) "fast" else "real time"}")
        }
    }

    /** On the touch that starts recording: promote FGS to microphone, pick up settings changes. */
    private fun beforeStart() {
        if (!micPromoted) promote(withMic = true)
        controller.cap = app.settings.load().sessionCap()
        // D37: the keyboard may have switched to (or from) Hebrew: load the model it needs.
        app.engine.ensureLanguage()
    }

    /** D37: where the last session's language came from (keyboard / phone / fixed), for the debug line. */
    @Volatile private var languageSource: String? = null

    /** Bubble look settings + Command Mode's ✎ (one flow, so the bubble combine stays at five inputs). */
    private data class Look(val repair: Boolean, val scale: Float, val dot: Boolean, val command: Boolean)

    private inner class BubbleListener : BubbleView.Listener {
        override fun onTap(button: BubbleButton) {
            val s = controller.state.value
            when (button) {
                BubbleButton.COMMAND -> {
                    // Command Mode: tap ✎, say the instruction; it ends on silence, ✓ or a second tap.
                    presence.touch()
                    haptic(VibrationEffect.EFFECT_CLICK)
                    beforeStart()
                    controller.toggleCommand()
                }
                BubbleButton.CANCEL -> controller.cancel()
                BubbleButton.DONE -> { haptic(VibrationEffect.EFFECT_CLICK); controller.release() }
                BubbleButton.PASTE -> controller.pasteRetry()
                BubbleButton.COPY -> {
                    // FailedToPaste (target changed / paste failed): copy that text, not the previous one.
                    val failed = (s as? DictationState.Error)?.kind as? DictationState.ErrorKind.FailedToPaste
                    (failed?.text ?: lastText)?.let { copySensitive(it); showCopy.value = false }
                }
                BubbleButton.DISMISS -> when ((s as? DictationState.Error)?.let { ErrorTap.of(it.kind) } ?: ErrorTap.Action.DISMISS) {
                    ErrorTap.Action.RECHECK -> recheck()
                    ErrorTap.Action.OPEN_APP -> {
                        startActivity(Intent(this@FlowService, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                        controller.dismissError()
                    }
                    ErrorTap.Action.DISMISS -> controller.dismissError()
                }
                BubbleButton.MAIN -> if (repair.value && (s == DictationState.Ready || s == DictationState.Idle)) {
                    // The repair alert (Accessibility stopped): straight to where it is fixed.
                    startActivity(PermissionsActivity.intent(this@FlowService).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                } else when (s) {
                    DictationState.Ready, DictationState.Idle -> {
                        presence.touch()
                        haptic(VibrationEffect.EFFECT_CLICK)
                        beforeStart()
                        controller.toggle(hold = false)
                    }
                    is DictationState.Error -> if (ErrorTap.of(s.kind) == ErrorTap.Action.RECHECK) recheck() else controller.dismissError()
                    else -> Unit
                }
            }
        }

        /** "Still loading" bubble tapped: check the engine again (ready → dictate, else say it is still loading). */
        private fun recheck() {
            presence.touch()
            haptic(VibrationEffect.EFFECT_CLICK)
            beforeStart()
            controller.toggle(hold = false)
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
