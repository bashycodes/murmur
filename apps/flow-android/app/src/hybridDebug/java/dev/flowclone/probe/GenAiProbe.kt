package dev.flowclone.probe

import android.content.Context
import android.os.ParcelFileDescriptor
import com.google.mlkit.genai.common.DownloadStatus
import com.google.mlkit.genai.common.GenAiException
import com.google.mlkit.genai.common.audio.AudioSource
import com.google.mlkit.genai.speechrecognition.SpeechRecognition
import com.google.mlkit.genai.speechrecognition.SpeechRecognizer
import com.google.mlkit.genai.speechrecognition.SpeechRecognizerOptions
import com.google.mlkit.genai.speechrecognition.SpeechRecognizerResponse
import com.google.mlkit.genai.speechrecognition.speechRecognizerOptions
import com.google.mlkit.genai.speechrecognition.speechRecognizerRequest
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.transformWhile
import kotlinx.coroutines.launch
import java.util.Locale
import java.util.concurrent.atomic.AtomicLong

/** What a run reports back, always on the main thread. */
interface RunListener {
    fun onState(s: String)
    fun onPartial(text: String)
    fun onFinal(text: String)
    fun onDone(record: ProbeRecord, line: String)
}

/**
 * ML Kit GenAI Speech Recognition (genai-speech-recognition 1.0.0-alpha1), en-US, advanced
 * (Gemini Nano) or basic mode. One client per mode, kept for the process. Sources: the mic
 * (AudioSource.fromMic) or the probe clip through a pipe (AudioSource.fromPfd, 16 kHz PCM16 mono).
 *
 * alpha1 has no getBaseModelName (the docs mention it for later versions), so the probe says so.
 */
object GenAiProbe {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var client: SpeechRecognizer? = null
    private var clientAdvanced: Boolean? = null
    private var runJob: Job? = null
    private var micEnd: (() -> Unit)? = null

    val running get() = runJob?.isActive == true
    val micRunning get() = running && micEnd != null

    private fun client(advanced: Boolean): SpeechRecognizer {
        client?.takeIf { clientAdvanced == advanced }?.let { return it }
        client?.let { runCatching { it.close() } }
        return SpeechRecognition.getClient(
            speechRecognizerOptions {
                locale = Locale.US
                preferredMode = if (advanced) SpeechRecognizerOptions.Mode.MODE_ADVANCED else SpeechRecognizerOptions.Mode.MODE_BASIC
            },
        ).also { client = it; clientAdvanced = advanced }
    }

    private fun describe(t: Throwable): Pair<Int?, String> = when (t) {
        is GenAiException -> t.errorCode to "${ProbeCodes.genAiError(t.errorCode)}: ${t.message}"
        else -> null to t.toString()
    }

    /** checkStatus() → FeatureStatus name, or the error. */
    fun checkStatus(advanced: Boolean, done: (String) -> Unit) {
        scope.launch {
            val s = try {
                ProbeCodes.featureStatus(client(advanced).checkStatus())
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                "check failed: " + describe(t).second
            }
            ProbeRuntime.lastGenAiStatus = s
            done(s)
        }
    }

    /** download() progress lines until completed or failed. */
    fun download(advanced: Boolean, update: (String) -> Unit): Job = scope.launch {
        var total: Long? = null
        try {
            client(advanced).download().collect { d ->
                when (d) {
                    is DownloadStatus.DownloadStarted -> { total = d.bytesToDownload; update("started: ${ProbeCodes.downloadProgress(0, total)}") }
                    is DownloadStatus.DownloadProgress -> update("downloading: ${ProbeCodes.downloadProgress(d.totalBytesDownloaded, total)}")
                    is DownloadStatus.DownloadCompleted -> update("download completed")
                    is DownloadStatus.DownloadFailed -> update("download failed: " + describe(d.e).second)
                }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (t: Throwable) {
            update("download error: " + describe(t).second)
        }
    }

    /** End of a mic run: the user's Stop. The final text arrives after it. */
    fun stopMic() { micEnd?.invoke() }

    fun cancel() { runJob?.cancel() }

    /**
     * One recognition. [clip] null = the mic. [trigger] is "screen" or "overlay". The run's
     * record is logged (files/probe-english.jsonl + logcat) and handed to [l].
     */
    fun run(ctx: Context, advanced: Boolean, clip: ProbeRuntime.Clip?, trigger: String, l: RunListener) {
        if (running) { l.onState("a GenAI run is already going"); return }
        val app = ctx.applicationContext
        val onTop = ProbeRuntime.probeOnTop
        runJob = scope.launch {
            val c = try { client(advanced) } catch (t: Throwable) {
                l.onState("client failed: $t"); return@launch
            }
            val status = try { ProbeCodes.featureStatus(c.checkStatus()) } catch (e: CancellationException) { throw e } catch (t: Throwable) { "check failed: " + describe(t).second }
            ProbeRuntime.lastGenAiStatus = status
            val t0 = ProbeRuntime.now()
            var timing = ProbeTiming(t0)
            val audioEnd = AtomicLong(0) // 0 = not yet; set from the feeder thread or Stop
            val finals = ArrayList<String>()
            var partials = 0
            var errCode: Int? = null
            var err: String? = null
            var cancelled = false
            var pipe: Array<ParcelFileDescriptor>? = null
            var feeder: ProbeFeeder? = null
            val source: AudioSource = if (clip == null) {
                micEnd = {
                    audioEnd.compareAndSet(0, ProbeRuntime.now())
                    l.onState("stopping…")
                    scope.launch { runCatching { c.stopRecognition() } }
                }
                AudioSource.fromMic()
            } else {
                val p = ParcelFileDescriptor.createPipe().also { pipe = it }
                feeder = ProbeFeeder(clip.pcm, ProbeRuntime.realTimePacing, p[1]) { end ->
                    audioEnd.compareAndSet(0, end)
                    if (ProbeRuntime.stopAtClipEnd) scope.launch { runCatching { c.stopRecognition() } }
                }
                AudioSource.fromPfd(p[0])
            }
            l.onState("GenAI ${ProbeCodes.mode(advanced)} · $status · listening (${clip?.label ?: "mic"})")
            try {
                feeder?.start()
                c.startRecognition(speechRecognizerRequest { audioSource = source })
                    .transformWhile { emit(it); it !is SpeechRecognizerResponse.CompletedResponse && it !is SpeechRecognizerResponse.ErrorResponse }
                    .collect { r ->
                        val now = ProbeRuntime.now()
                        when (r) {
                            is SpeechRecognizerResponse.PartialTextResponse -> {
                                partials++
                                if (timing.firstPartialAt == null) timing = timing.copy(firstPartialAt = now)
                                l.onPartial(r.text)
                            }
                            is SpeechRecognizerResponse.FinalTextResponse -> {
                                finals += r.text
                                timing = timing.copy(lastFinalAt = now)
                                l.onFinal(finals.joinToString(" "))
                            }
                            is SpeechRecognizerResponse.CompletedResponse -> timing = timing.copy(completedAt = now)
                            is SpeechRecognizerResponse.ErrorResponse -> { val d = describe(r.e); errCode = d.first; err = d.second }
                        }
                    }
            } catch (e: CancellationException) {
                cancelled = true
                scope.launch { runCatching { c.stopRecognition() } }
            } catch (t: Throwable) {
                val d = describe(t); errCode = d.first; err = d.second
            } finally {
                feeder?.stop()
                pipe?.forEach { runCatching { it.close() } }
                micEnd = null
            }
            if (clip == null) audioEnd.compareAndSet(0, timing.completedAt ?: ProbeRuntime.now())
            timing = timing.copy(audioEndAt = audioEnd.get().takeIf { it > 0 })
            val text = finals.joinToString(" ").trim()
            val rec = ProbeRecord(
                tsMs = System.currentTimeMillis(), appBuild = ProbeRuntime.build(),
                engine = "genai", mode = ProbeCodes.mode(advanced), source = clip?.source ?: "mic",
                trigger = trigger, probeOnTop = onTop, status = status,
                outcome = ProbeRecord.outcome(text, errCode ?: err?.let { -1 }, cancelled),
                text = text, partials = partials, timing = timing,
                audioMs = clip?.durationMs ?: timing.audioEndAt?.let { it - t0 },
                errorCode = errCode, error = err,
                extras = mapOf(
                    "locale" to "en-US",
                    "real_time" to (clip != null && ProbeRuntime.realTimePacing),
                    "stop_at_clip_end" to (clip != null && ProbeRuntime.stopAtClipEnd),
                    "base_model" to "n/a (alpha1 has no getBaseModelName)",
                ),
            )
            val line = ProbeRuntime.append(app, rec)
            l.onDone(rec, line)
        }
    }
}
