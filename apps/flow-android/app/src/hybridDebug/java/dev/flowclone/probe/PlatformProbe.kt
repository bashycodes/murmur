package dev.flowclone.probe

import android.content.Context
import android.content.Intent
import android.media.AudioFormat
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.speech.RecognitionListener
import android.speech.RecognitionSupport
import android.speech.RecognitionSupportCallback
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer

/**
 * The platform on-device recognizer (`SpeechRecognizer.createOnDeviceSpeechRecognizer`, API 31+;
 * support check, biasing, formatting and app-supplied audio API 33+, our minSdk). Main thread
 * only, as the platform requires. Sources: the mic (the recognizer service records) or the probe
 * clip through `EXTRA_AUDIO_SOURCE` (a pipe of 16 kHz PCM16 mono).
 */
object PlatformProbe {
    const val LANG = "en-US"
    private const val WATCHDOG_MS = 30_000L
    private val main = Handler(Looper.getMainLooper())
    private var rec: SpeechRecognizer? = null
    private var finish: ((String?) -> Unit)? = null
    private var micStop: (() -> Unit)? = null

    val running get() = rec != null
    val micRunning get() = rec != null && micStop != null

    fun available(ctx: Context): Boolean = SpeechRecognizer.isOnDeviceRecognitionAvailable(ctx)

    private fun intent(terms: List<String>, formatting: Boolean, pfd: ParcelFileDescriptor?) =
        Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            putExtra(RecognizerIntent.EXTRA_LANGUAGE, LANG)
            putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
            val t = terms.map { it.trim() }.filter { it.isNotEmpty() }
            if (t.isNotEmpty()) putStringArrayListExtra(RecognizerIntent.EXTRA_BIASING_STRINGS, ArrayList(t))
            if (formatting) putExtra(RecognizerIntent.EXTRA_ENABLE_FORMATTING, RecognizerIntent.FORMATTING_OPTIMIZE_QUALITY)
            if (pfd != null) {
                putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE, pfd)
                putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
                putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_CHANNEL_COUNT, 1)
                putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_SAMPLING_RATE, ProbePcm.RATE)
            }
        }

    /** `checkRecognitionSupport(en-US)`: one word for en-US plus the raw lists. */
    fun checkSupport(ctx: Context, done: (String) -> Unit) {
        if (!available(ctx)) { done("isOnDeviceRecognitionAvailable = false"); return }
        val r = try { SpeechRecognizer.createOnDeviceSpeechRecognizer(ctx) } catch (t: Throwable) { done("create failed: $t"); return }
        r.checkRecognitionSupport(intent(emptyList(), false, null), ctx.mainExecutor, object : RecognitionSupportCallback {
            override fun onSupportResult(s: RecognitionSupport) {
                val word = ProbeCodes.platformSupport(
                    LANG, s.installedOnDeviceLanguages, s.pendingOnDeviceLanguages, s.supportedOnDeviceLanguages, s.onlineLanguages,
                )
                ProbeRuntime.lastPlatformSupport = word
                done(
                    "en-US: $word\ninstalled=${s.installedOnDeviceLanguages}\npending=${s.pendingOnDeviceLanguages}\n" +
                        "supported=${s.supportedOnDeviceLanguages.size} langs\nonline=${s.onlineLanguages.size} langs",
                )
                r.destroy()
            }

            override fun onError(error: Int) {
                done("support check error: ${ProbeCodes.platformError(error)}")
                r.destroy()
            }
        })
    }

    /** `triggerModelDownload(en-US)` (the service may show its own UI or do nothing). */
    fun triggerDownload(ctx: Context): String {
        if (!available(ctx)) return "isOnDeviceRecognitionAvailable = false"
        return try {
            val r = SpeechRecognizer.createOnDeviceSpeechRecognizer(ctx)
            r.triggerModelDownload(intent(emptyList(), false, null))
            main.postDelayed({ r.destroy() }, 5_000)
            "triggerModelDownload(en-US) sent; re-check support"
        } catch (t: Throwable) {
            "triggerModelDownload failed: $t"
        }
    }

    fun stopMic() { micStop?.invoke() }

    fun cancel() { finish?.invoke("cancelled") }

    fun run(ctx: Context, clip: ProbeRuntime.Clip?, terms: List<String>, formatting: Boolean, trigger: String, l: RunListener) {
        if (running) { l.onState("a platform run is already going"); return }
        val app = ctx.applicationContext
        val onTop = ProbeRuntime.probeOnTop
        if (!available(app)) {
            val rec = record(clip, trigger, onTop, "", 0, ProbeTiming(ProbeRuntime.now()), null, "isOnDeviceRecognitionAvailable = false", terms, formatting, false)
            l.onDone(rec, ProbeRuntime.append(app, rec)); return
        }
        val r = try { SpeechRecognizer.createOnDeviceSpeechRecognizer(app) } catch (t: Throwable) {
            l.onState("create failed: $t"); return
        }
        rec = r
        val t0 = ProbeRuntime.now()
        var timing = ProbeTiming(t0)
        var audioEndAt = 0L // set on the main thread only
        var partials = 0
        var text = ""
        var done = false
        var pipe: Array<ParcelFileDescriptor>? = null
        var feeder: ProbeFeeder? = null
        val watchdog = Runnable { finish?.invoke("watchdog: no result ${WATCHDOG_MS / 1000} s after the end of audio") }

        fun end(errCode: Int?, err: String?) {
            if (done) return
            done = true
            main.removeCallbacks(watchdog)
            feeder?.stop()
            pipe?.forEach { runCatching { it.close() } }
            runCatching { r.destroy() }
            rec = null; finish = null; micStop = null
            if (audioEndAt == 0L) audioEndAt = timing.completedAt ?: ProbeRuntime.now()
            timing = timing.copy(audioEndAt = audioEndAt)
            val record = record(clip, trigger, onTop, text, partials, timing, errCode, err, terms, formatting, err == "cancelled")
            l.onDone(record, ProbeRuntime.append(app, record))
        }
        finish = { why -> runCatching { r.cancel() }; end(null, why) }

        r.setRecognitionListener(object : RecognitionListener {
            override fun onReadyForSpeech(params: Bundle?) { l.onState("platform · ready (${clip?.label ?: "mic"})") }
            override fun onBeginningOfSpeech() { l.onState("platform · speech") }
            override fun onRmsChanged(rmsdB: Float) {}
            override fun onBufferReceived(buffer: ByteArray?) {}
            override fun onEndOfSpeech() {
                if (clip == null && audioEndAt == 0L) audioEndAt = ProbeRuntime.now()
                l.onState("platform · end of speech")
            }
            override fun onError(error: Int) = end(error, ProbeCodes.platformError(error))
            override fun onResults(results: Bundle?) {
                val now = ProbeRuntime.now()
                text = results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull().orEmpty()
                timing = timing.copy(lastFinalAt = now, completedAt = now)
                l.onFinal(text)
                end(null, null)
            }
            override fun onPartialResults(partialResults: Bundle?) {
                val p = partialResults?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull() ?: return
                partials++
                if (timing.firstPartialAt == null) timing = timing.copy(firstPartialAt = ProbeRuntime.now())
                l.onPartial(p)
            }
            override fun onEvent(eventType: Int, params: Bundle?) {}
        })

        val pfd: ParcelFileDescriptor? = if (clip != null) {
            val p = ParcelFileDescriptor.createPipe().also { pipe = it }
            feeder = ProbeFeeder(clip.pcm, ProbeRuntime.realTimePacing, p[1]) { endAt ->
                main.post {
                    if (audioEndAt == 0L) audioEndAt = endAt
                    if (!done) main.postDelayed(watchdog, WATCHDOG_MS)
                }
            }
            p[0]
        } else {
            micStop = {
                if (audioEndAt == 0L) audioEndAt = ProbeRuntime.now()
                l.onState("platform · stopping…")
                r.stopListening()
            }
            null
        }
        try {
            r.startListening(intent(terms, formatting, pfd))
            feeder?.start()
        } catch (t: Throwable) {
            end(null, "startListening failed: $t")
        }
    }

    private fun record(
        clip: ProbeRuntime.Clip?, trigger: String, onTop: Boolean, text: String, partials: Int, timing: ProbeTiming,
        errCode: Int?, err: String?, terms: List<String>, formatting: Boolean, cancelled: Boolean,
    ) = ProbeRecord(
        tsMs = System.currentTimeMillis(), appBuild = ProbeRuntime.build(),
        engine = "platform", mode = null, source = clip?.source ?: "mic", trigger = trigger, probeOnTop = onTop,
        status = ProbeRuntime.lastPlatformSupport,
        outcome = ProbeRecord.outcome(text, errCode ?: err?.let { -1 }, cancelled),
        text = text.trim(), partials = partials, timing = timing,
        audioMs = clip?.durationMs ?: timing.audioEndAt?.let { it - timing.startAt },
        errorCode = errCode, error = err,
        extras = mapOf(
            "locale" to LANG,
            "biasing" to terms.map { it.trim() }.filter { it.isNotEmpty() },
            "formatting" to formatting,
            "real_time" to (clip != null && ProbeRuntime.realTimePacing),
        ),
    )
}
