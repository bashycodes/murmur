package dev.flowclone.debug

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import dev.flowclone.FlowApp
import dev.flowclone.calls.CallRequest
import dev.flowclone.calls.Decoders
import dev.flowclone.calls.To16kMono
import kotlin.concurrent.thread
import kotlin.math.sqrt

/**
 * Debug builds only (src/debug), D46 Calls device check. The host broadcasts
 * `dev.flowclone.debug.DECODE_CALL --es path files/debug/calls-in/x.m4a [--ez transcribe true --es lang en]`
 * (tools/phone/call-decode.sh). The file is decoded with the Calls decoder (MediaCodec for
 * m4a) to 16 kHz mono and the result data says what came out: input format, output samples,
 * length and level. With `transcribe`, the same file then goes through the Calls job (its own
 * engine); the transcript lands on the Calls page, progress in logcat (tag FlowCallDecode).
 * Guarded by android.permission.DUMP in the manifest (shell only).
 */
class CallDecodeReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val path = intent.getStringExtra(DebugWav.EXTRA_PATH)
        val file = path?.let { DebugWav.resolve(context.filesDir, it) }
        if (file == null) { reply("error: no file under files/debug/: $path"); return }
        val transcribe = intent.getBooleanExtra("transcribe", false)
        val lang = intent.getStringExtra("lang") ?: "en"
        val pending = goAsync()
        thread(name = "call-decode-check") {
            val msg = try {
                decode(context, Uri.fromFile(file))
            } catch (e: Exception) {
                "error: ${e.javaClass.simpleName}: ${e.message}"
            }
            var out = msg
            if (transcribe && msg.startsWith("ok")) {
                val app = FlowApp.of(context)
                val started = app.calls.start(CallRequest(Uri.fromFile(file).toString(), file.name, file.lastModified(), lang))
                out += if (started) "; transcription started (see the Calls page)" else "; a transcription is already running"
                if (started) thread(name = "call-decode-watch") {
                    var last: Any? = null
                    while (true) {
                        val s = app.calls.state.value
                        if (s != last) { Log.i(TAG, "job: $s"); last = s }
                        if (!s.busy) break
                        Thread.sleep(500)
                    }
                }
            }
            Log.i(TAG, out)
            runCatching { pending.resultData = out }
            pending.finish()
        }
    }

    private fun decode(ctx: Context, uri: Uri): String {
        Decoders.open(ctx, uri).use { src ->
            val conv = To16kMono()
            var inRate = 0; var inCh = 0; var n = 0L; var sumSq = 0.0
            fun take(p: ShortArray) { n += p.size; for (s in p) sumSq += s.toDouble() * s }
            while (true) {
                val b = src.next() ?: break
                inRate = b.sampleRate; inCh = b.channels
                take(conv.feed(b))
            }
            take(conv.flush())
            val rms = if (n == 0L) 0.0 else sqrt(sumSq / n) / 32768.0
            return "ok: in ${inRate} Hz ${inCh} ch (container ${src.durationMs} ms); out $n samples 16000 Hz mono = ${n / 16} ms; rms %.3f".format(java.util.Locale.ROOT, rms)
        }
    }

    private fun reply(msg: String) {
        Log.i(TAG, msg)
        if (isOrderedBroadcast) resultData = msg
    }

    private companion object {
        const val TAG = "FlowCallDecode"
    }
}
