package dev.murmur.debug

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import dev.murmur.service.MurmurService

/**
 * Debug builds only (src/debug): "WAV as microphone". The host broadcasts
 * `dev.murmur.debug.PLAY_WAV --es path files/debug/wav-in/x.wav [--ez fast true] [--ez tap true]`
 * (tools/phone/play-wav.sh); this hands it to the running MurmurService, which runs one normal
 * dictation on that audio. Guarded by android.permission.DUMP in the manifest (shell only).
 * The result data says what happened, so `am broadcast` prints it.
 */
class WavMicReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val path = intent.getStringExtra(DebugWav.EXTRA_PATH)
        val file = path?.let { DebugWav.resolve(context.filesDir, it) }
        if (file == null) {
            reply("error: no WAV file under files/debug/: $path")
            return
        }
        val forward = Intent(context, MurmurService::class.java).setAction(DebugWav.ACTION)
            .putExtra(DebugWav.EXTRA_PATH, path)
            .putExtra(DebugWav.EXTRA_FAST, intent.getBooleanExtra(DebugWav.EXTRA_FAST, false))
            .putExtra(DebugWav.EXTRA_TAP, intent.getBooleanExtra(DebugWav.EXTRA_TAP, false))
        if (!MurmurService.running) {
            reply("error: Murmur is not running (open the app so the bubble shows)")
            return
        }
        try {
            context.startService(forward)
            reply("ok: ${file.name} queued")
        } catch (e: Exception) { // background start refused, SecurityException
            reply("error: could not reach MurmurService: ${e.message}")
        }
    }

    private fun reply(msg: String) {
        Log.i("MurmurDebugWav", msg)
        if (isOrderedBroadcast) resultData = msg
    }
}
