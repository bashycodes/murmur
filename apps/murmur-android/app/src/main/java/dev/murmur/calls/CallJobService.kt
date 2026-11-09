package dev.murmur.calls

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import android.widget.Toast
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.ui.CallsActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * D46: the foreground service a call transcription runs under. Its own small service (not
 * MurmurService, which needs the overlay permission and hosts dictation): it starts the job in
 * [CallJobs], shows its progress with a Cancel action, holds a partial wake lock so the
 * transcription keeps going with the screen off, and stops itself when the job ends.
 * D49: [ACTION_DRAIN] runs the auto-transcribe queue the same way, one recording after another
 * (each after any manual job); the finish notification comes from [notifyFinished] (MurmurApp's
 * job hook), so a job run inside [CallRecordingJob] gets one too.
 */
class CallJobService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val app get() = MurmurApp.of(this)
    private var watch: Job? = null
    private var wake: PowerManager.WakeLock? = null
    private var lastPercent = -2

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        createChannel()
        when (intent?.action) {
            ACTION_CANCEL -> {
                app.calls.cancel()
                if (!app.calls.state.value.busy) stopSelf()
                return START_NOT_STICKY
            }
            ACTION_DRAIN -> {
                if (!promote(app.calls.current?.name ?: "new call recordings")) { stopSelf(); return START_NOT_STICKY }
                if (!app.calls.state.value.busy && !app.calls.startNext()) { stopForeground(STOP_FOREGROUND_REMOVE); stopSelf(); return START_NOT_STICKY }
                watchJob()
            }
            ACTION_START -> {
                val req = intent.request()
                if (!promote(req?.name ?: "")) { stopSelf(); return START_NOT_STICKY }
                if (req == null || !app.calls.start(req)) {
                    if (req != null) Toast.makeText(this, "Another recording is being transcribed", Toast.LENGTH_SHORT).show()
                    if (!app.calls.state.value.busy) stopSelf()
                }
                watchJob()
            }
            else -> if (!app.calls.state.value.busy) stopSelf()
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        // The system or the owner killed us mid-job: stop the job rather than run unguarded.
        if (app.calls.state.value.busy) app.calls.cancel(CallStop.SYSTEM)
        releaseWake()
        scope.cancel()
        super.onDestroy()
    }

    private fun watchJob() {
        if (watch?.isActive == true) return
        watch = scope.launch {
            app.calls.state.collect { s ->
                when (s) {
                    is CallJobState.Running -> { acquireWake(); update(s) }
                    is CallJobState.Idle -> Unit
                    // Next in the auto queue (D49), else done.
                    else -> if (!app.calls.startNext()) { releaseWake(); stopForeground(STOP_FOREGROUND_REMOVE); stopSelf() }
                }
            }
        }
    }

    private fun promote(name: String): Boolean = try {
        val n = progressNotification(CallJobState.Running(name, loading = true, fraction = null))
        if (Build.VERSION.SDK_INT >= 34) startForeground(NOTIF_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        else startForeground(NOTIF_ID, n)
        true
    } catch (e: Exception) {
        Log.w(TAG, "startForeground refused", e)
        false
    }

    private fun update(s: CallJobState.Running) {
        val pct = if (s.loading) -1 else ((s.fraction ?: 0f) * 100).toInt()
        if (pct == lastPercent) return
        lastPercent = pct
        runCatching { nm().notify(NOTIF_ID, progressNotification(s)) }
    }

    private fun progressNotification(s: CallJobState.Running): Notification =
        Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_murmur)
            .setContentTitle("Transcribing a call recording")
            .setContentText(CallsView.progressLabel(s.name, s.fraction, s.loading))
            .setProgress(100, ((s.fraction ?: 0f) * 100).toInt(), s.loading || s.fraction == null)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(openPage(null))
            .addAction(
                Notification.Action.Builder(
                    null, "Cancel",
                    PendingIntent.getService(this, 1, Intent(this, CallJobService::class.java).setAction(ACTION_CANCEL), PendingIntent.FLAG_IMMUTABLE),
                ).build(),
            )
            .build()

    private fun openPage(id: String?) = openPage(this, id)

    private fun acquireWake() {
        if (wake?.isHeld == true) return
        wake = getSystemService(PowerManager::class.java)?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "murmur:call-transcribe")?.apply {
            setReferenceCounted(false)
            acquire(WAKE_TIMEOUT_MS)
        }
    }

    private fun releaseWake() {
        runCatching { wake?.takeIf { it.isHeld }?.release() }
        wake = null
    }

    private fun nm() = getSystemService(NotificationManager::class.java)

    private fun createChannel() = createChannel(this)

    companion object {
        const val ACTION_START = "dev.murmur.action.CALL_TRANSCRIBE"
        const val ACTION_CANCEL = "dev.murmur.action.CALL_CANCEL"
        const val ACTION_DRAIN = "dev.murmur.action.CALL_DRAIN"
        const val EXTRA_URI = "uri"
        const val EXTRA_NAME = "name"
        const val EXTRA_RECORDED_AT = "recorded_at"
        const val EXTRA_LANGUAGE = "language"
        private const val CHANNEL = "murmur_calls"
        private const val NOTIF_ID = 4001
        private const val NOTIF_DONE_ID = 4002
        /** A safety net only: the lock is released when the job ends. */
        private const val WAKE_TIMEOUT_MS = 6 * 60 * 60 * 1000L
        private const val TAG = "CallJobService"

        fun startIntent(ctx: Context, req: CallRequest): Intent = Intent(ctx, CallJobService::class.java).setAction(ACTION_START)
            .putExtra(EXTRA_URI, req.uri).putExtra(EXTRA_NAME, req.name)
            .putExtra(EXTRA_RECORDED_AT, req.recordedAt).putExtra(EXTRA_LANGUAGE, req.language)
            // The read grant from the picker travels with the intent to the service.
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)

        fun drainIntent(ctx: Context): Intent = Intent(ctx, CallJobService::class.java).setAction(ACTION_DRAIN)

        /**
         * A job ended: "transcript ready" (opens it) or the failure. Cancelled jobs say nothing
         * (the owner asked, or the job only stepped aside and is queued again).
         */
        fun notifyFinished(ctx: Context, req: CallRequest, s: CallJobState) {
            val (title, id) = when (s) {
                is CallJobState.Done -> "Call transcript ready" to s.id
                is CallJobState.Failed -> "Couldn't transcribe the recording: ${s.message}" to null
                else -> return
            }
            createChannel(ctx)
            val n = Notification.Builder(ctx, CHANNEL)
                .setSmallIcon(R.drawable.ic_murmur)
                .setContentTitle(title)
                .setContentText(req.name)
                .setAutoCancel(true)
                .setContentIntent(openPage(ctx, id))
                .build()
            runCatching { ctx.getSystemService(NotificationManager::class.java).notify(NOTIF_DONE_ID, n) }
        }

        private fun openPage(ctx: Context, id: String?) = PendingIntent.getActivity(
            ctx, if (id == null) 0 else 1,
            Intent(ctx, CallsActivity::class.java).apply { id?.let { putExtra(CallsActivity.EXTRA_ID, it) } },
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        private fun createChannel(ctx: Context) {
            runCatching {
                ctx.getSystemService(NotificationManager::class.java).createNotificationChannel(
                    NotificationChannel(CHANNEL, "Call transcription", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) },
                )
            }
        }

        fun cancelIntent(ctx: Context): Intent = Intent(ctx, CallJobService::class.java).setAction(ACTION_CANCEL)

        fun Intent.request(): CallRequest? {
            val uri = getStringExtra(EXTRA_URI) ?: return null
            return CallRequest(
                uri, getStringExtra(EXTRA_NAME) ?: "recording", getLongExtra(EXTRA_RECORDED_AT, 0),
                getStringExtra(EXTRA_LANGUAGE) ?: "en",
            )
        }
    }
}
