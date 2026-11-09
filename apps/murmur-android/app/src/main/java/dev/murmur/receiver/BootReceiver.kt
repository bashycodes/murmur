package dev.murmur.receiver

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.provider.Settings
import android.util.Log
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.health.A11ySetting
import dev.murmur.service.MurmurAccessibilityService
import dev.murmur.service.MurmurService
import dev.murmur.ui.MainActivity

/**
 * Boot / update / unlock recovery:
 * accessibility enabled → start MurmurService (the system rebinds the a11y service itself,
 * which also starts it; doing both covers OEMs that delay the rebind); otherwise post a
 * "turn Murmur back on" notification (id 901). MurmurService starts as
 * `specialUse`, which Android 14/15 still allow from BOOT_COMPLETED (microphone isn't).
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.action ?: return
        if (action !in ACTIONS) return
        val app = MurmurApp.of(context)
        // D49: JobScheduler forgets the call-recording triggers across a reboot; arm them again.
        // (App start does it too; this covers a boot where nothing else starts the app.)
        dev.murmur.calls.AutoCalls.arm(context)
        if (app.settings.serviceStoppedByUser || !Settings.canDrawOverlays(context)) return
        val enabled = A11ySetting.isEnabled(
            Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES),
            context.packageName, MurmurAccessibilityService::class.java.name,
        )
        Log.i("MurmurBoot", "$action: accessibility enabled=$enabled")
        if (enabled) {
            runCatching { context.startForegroundService(Intent(context, MurmurService::class.java).setAction(MurmurService.ACTION_SHOW)) }
                .onFailure { Log.w("MurmurBoot", "could not start MurmurService", it) }
        } else {
            notifyReEnable(context)
        }
    }

    private fun notifyReEnable(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        nm.createNotificationChannel(
            NotificationChannel(MurmurService.CHANNEL_STATUS, context.getString(R.string.notif_channel_status), NotificationManager.IMPORTANCE_DEFAULT),
        )
        val n = Notification.Builder(context, MurmurService.CHANNEL_STATUS)
            .setContentTitle(context.getString(R.string.notif_boot_title))
            .setContentText(context.getString(R.string.notif_boot_text))
            .setStyle(Notification.BigTextStyle().bigText(context.getString(R.string.notif_boot_text)))
            .setSmallIcon(R.drawable.ic_murmur)
            .setAutoCancel(true)
            .setContentIntent(
                PendingIntent.getActivity(
                    context, 2, Intent(context, MainActivity::class.java),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
            .build()
        runCatching { nm.notify(NOTIF_REENABLE_ID, n) }
    }

    companion object {
        const val NOTIF_REENABLE_ID = 901
        val ACTIONS = setOf(Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED, Intent.ACTION_USER_UNLOCKED)
    }
}
