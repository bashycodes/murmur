package dev.flowclone.probe

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.text.InputType
import android.view.WindowManager
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import java.io.File
import java.util.Locale
import kotlin.concurrent.thread

/**
 * English 1 spike: the probe screen (Developer settings → DIAGNOSTICS → English speech probe,
 * hybrid debug builds only). Compares ML Kit GenAI speech (Gemini Nano, advanced / basic) with
 * the platform on-device recognizer, from the mic or a synthetic TTS / picked WAV clip, and opens
 * the overlay test. Every run appends one JSON line to files/probe-english.jsonl and logcat
 * (tag EnglishProbe). It never changes how normal dictation is routed.
 */
class EnglishProbeActivity : Activity() {
    private lateinit var col: LinearLayout
    private val dp get() = resources.displayMetrics.density
    private var tts: TextToSpeech? = null

    private lateinit var clipInfo: TextView
    private lateinit var lastLine: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = "English speech probe"
        col = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            val p = (16 * dp).toInt(); setPadding(p, p, p, p)
        }
        setContentView(ScrollView(this).apply { addView(col); fitsSystemWindows = true })
        build()
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), REQ_MIC)
        }
    }

    override fun onResume() { super.onResume(); ProbeRuntime.probeOnTop = true; showClip() }
    override fun onPause() { ProbeRuntime.probeOnTop = false; super.onPause() }
    override fun onDestroy() { tts?.shutdown(); tts = null; super.onDestroy() }

    // ---- tiny view kit ----

    private fun header(t: String) = TextView(this).apply {
        text = t; textSize = 13f; setPadding(0, (18 * dp).toInt(), 0, (4 * dp).toInt())
        setTypeface(typeface, android.graphics.Typeface.BOLD)
    }.also { col.addView(it) }

    private fun text(t: String = "", size: Float = 14f) = TextView(this).apply {
        text = t; textSize = size; setTextIsSelectable(true)
    }.also { col.addView(it) }

    private fun button(label: String, onTap: (Button) -> Unit) = Button(this).apply {
        text = label; isAllCaps = false; setOnClickListener { onTap(this) }
    }.also { col.addView(it) }

    private fun check(label: String, on: Boolean, onChange: (Boolean) -> Unit) = CheckBox(this).apply {
        text = label; isChecked = on; setOnCheckedChangeListener { _, v -> onChange(v) }
    }.also { col.addView(it) }

    private fun edit(value: String, hint: String) = EditText(this).apply {
        setText(value); this.hint = hint; inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
    }.also { col.addView(it) }

    /** Output block of one engine: state, partial, final, latency / error. */
    private inner class Out {
        val state = text("", 13f)
        val partial = text("", 13f).apply { alpha = 0.7f }
        val final = text("", 16f)
        val stats = text("", 13f)

        fun listener(b: Button?, label: String) = object : RunListener {
            override fun onState(s: String) { state.text = s }
            override fun onPartial(text: String) { partial.text = "partial: $text" }
            override fun onFinal(text: String) { final.text = text }
            override fun onDone(record: ProbeRecord, line: String) {
                b?.text = label
                state.text = "${record.engine} ${record.mode ?: ""} · ${record.source} · ${record.outcome}"
                final.text = record.text.ifEmpty { "(no text)" }
                stats.text = buildString {
                    append("latency (end of audio → final): ${record.timing.latencyMs?.let { "$it ms" } ?: "—"}")
                    append("\ncompleted: ${record.timing.completedMs?.let { "$it ms" } ?: "—"} · first partial: ${record.timing.firstPartialMs?.let { "$it ms" } ?: "—"} · partials: ${record.partials}")
                    record.error?.let { append("\nERROR ${record.errorCode ?: ""} $it") }
                }
                lastLine.text = line
            }
        }
    }

    @SuppressLint("SetTextI18n")
    private fun build() {
        text("Throwaway spike (English 1). Runs log to files/${ProbeRuntime.LOG_FILE} and logcat tag ${ProbeRuntime.TAG}. " +
            "Normal dictation is unchanged.", 13f)

        // ---------------- clip ----------------
        header("SYNTHETIC CLIP (for the \"clip\" buttons)")
        val sentence = edit(DEFAULT_SENTENCE, "Sentence to synthesize")
        button("Make clip (TTS)") { makeTtsClip(sentence.text.toString()) }
        button("Pick WAV file…") {
            startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("audio/*"), REQ_PICK)
        }
        clipInfo = text("", 13f)
        check("Real-time pacing (feed the clip at speaking speed)", ProbeRuntime.realTimePacing) { ProbeRuntime.realTimePacing = it }
        check("GenAI: call stopRecognition at clip end", ProbeRuntime.stopAtClipEnd) { ProbeRuntime.stopAtClipEnd = it }

        // ---------------- GenAI ----------------
        header("ML KIT GENAI SPEECH (Gemini Nano) · en-US")
        check("Advanced mode (off = basic)", ProbeRuntime.genAiAdvanced) { ProbeRuntime.genAiAdvanced = it }
        val gStatus = text("Status: not checked\nBase model: n/a (alpha1 has no getBaseModelName)", 13f)
        button("Check status") {
            gStatus.text = "checking…"
            GenAiProbe.checkStatus(ProbeRuntime.genAiAdvanced) { gStatus.text = "Status (${ProbeCodes.mode(ProbeRuntime.genAiAdvanced)}): $it\nBase model: n/a (alpha1 has no getBaseModelName)" }
        }
        val gDl = text("", 13f)
        button("Download model") { gDl.text = "requesting download…"; GenAiProbe.download(ProbeRuntime.genAiAdvanced) { gDl.text = it } }
        val gOut = Out()
        button("GenAI: dictate (mic)") { b ->
            if (GenAiProbe.micRunning) GenAiProbe.stopMic()
            else { b.text = "GenAI: stop"; gOut.final.text = ""; gOut.partial.text = ""; GenAiProbe.run(this, ProbeRuntime.genAiAdvanced, null, "screen", gOut.listener(b, "GenAI: dictate (mic)")) }
        }
        button("GenAI: transcribe clip") {
            val c = ProbeRuntime.clip ?: run { gOut.state.text = "No clip yet: make or pick one above"; return@button }
            gOut.final.text = ""; gOut.partial.text = ""
            GenAiProbe.run(this, ProbeRuntime.genAiAdvanced, c, "screen", gOut.listener(null, ""))
        }
        button("GenAI: cancel run") { GenAiProbe.cancel() }

        // ---------------- platform ----------------
        header("PLATFORM ON-DEVICE RECOGNIZER · en-US")
        val pStatus = text("isOnDeviceRecognitionAvailable: ${PlatformProbe.available(this)}", 13f)
        button("Check support (en-US)") { pStatus.text = "checking…"; PlatformProbe.checkSupport(this) { pStatus.text = it } }
        button("Download en-US (triggerModelDownload)") { pStatus.text = PlatformProbe.triggerDownload(this) }
        text("Biasing strings (EXTRA_BIASING_STRINGS):", 13f)
        val terms = ProbeRuntime.biasing.map { edit(it, "synthetic term") }
        fun syncTerms() { ProbeRuntime.biasing = terms.map { it.text.toString() } }
        check("EXTRA_ENABLE_FORMATTING (quality)", ProbeRuntime.formatting) { ProbeRuntime.formatting = it }
        val pOut = Out()
        button("Platform: dictate (mic)") { b ->
            syncTerms()
            if (PlatformProbe.micRunning) PlatformProbe.stopMic()
            else { b.text = "Platform: stop"; pOut.final.text = ""; pOut.partial.text = ""; PlatformProbe.run(this, null, ProbeRuntime.biasing, ProbeRuntime.formatting, "screen", pOut.listener(b, "Platform: dictate (mic)")) }
        }
        button("Platform: transcribe clip") {
            syncTerms()
            val c = ProbeRuntime.clip ?: run { pOut.state.text = "No clip yet: make or pick one above"; return@button }
            pOut.final.text = ""; pOut.partial.text = ""
            PlatformProbe.run(this, c, ProbeRuntime.biasing, ProbeRuntime.formatting, "screen", pOut.listener(null, ""))
        }
        button("Platform: cancel run") { PlatformProbe.cancel() }

        // ---------------- folder batch ----------------
        header("FOLDER BATCH (parity scorecard)")
        text("Every WAV in <external files>/${ProbeBatchPlan.DIR}/<set>/ (any PCM16 WAV; 16 kHz mono needs no resampling) " +
            "through the ticked engines, one at a time, at the pacing above. GenAI uses the mode above; platform runs " +
            "a segmented session, no biasing, no formatting. One JSON line per file and engine in " +
            "${ProbeBatchPlan.DIR}/${ProbeBatchPlan.RESULTS}/batch-<UTC start>.jsonl. Keep this screen on top.", 13f)
        val bFolder = text(ProbeBatch.describe(this), 12f)
        val bSets = edit("", "Sets (comma list; blank = all)")
        var bGenAi = true
        var bPlatform = true
        check("Batch: GenAI (Gemini Nano)", bGenAi) { bGenAi = it }
        check("Batch: platform recognizer", bPlatform) { bPlatform = it }
        val bState = text("", 13f)
        button("Run folder batch") {
            bFolder.text = ProbeBatch.describe(this)
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            ProbeBatch.run(
                this, bSets.text.toString(), ProbeBatch.Engines(bGenAi, ProbeRuntime.genAiAdvanced, bPlatform),
                progress = { bState.text = it },
                done = { bState.text = it; window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) },
            )
        }
        button("Stop batch") { ProbeBatch.stop() }

        // ---------------- overlay ----------------
        header("OVERLAY TEST (another app on top)")
        text("Shows a small panel over other apps (same window type as the bubble). Open another app, then tap " +
            "G mic / G clip (GenAI) or P mic / P clip (platform). Mic buttons turn into ■: tap again to stop.", 13f)
        val ovState = text("", 13f)
        button("Show / hide overlay") {
            syncTerms()
            when {
                ProbeOverlay.shown -> { ProbeOverlay.hide(this); ovState.text = "overlay hidden" }
                !ProbeOverlay.canShow(this) -> {
                    ovState.text = "Allow \"Display over other apps\", then come back"
                    startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
                }
                else -> { ProbeOverlay.show(this); ovState.text = if (ProbeOverlay.shown) "overlay shown" else "overlay refused (see logcat)" }
            }
        }

        // ---------------- log ----------------
        header("LOG")
        text("files/${ProbeRuntime.LOG_FILE}", 13f)
        lastLine = text("", 11f)
        button("Clear log") { ProbeRuntime.logFile(this).delete(); lastLine.text = "log cleared" }
    }

    @SuppressLint("SetTextI18n")
    private fun showClip() {
        if (!::clipInfo.isInitialized) return
        clipInfo.text = ProbeRuntime.clip?.let { "Clip: ${it.label} · ${it.durationMs} ms" } ?: "Clip: none"
    }

    private fun makeTtsClip(sentence: String) {
        clipInfo.text = "synthesizing…"
        val out = File(cacheDir, "probe-tts.wav")
        val engine = tts
        val speak: (TextToSpeech) -> Unit = { t ->
            t.language = Locale.US
            t.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                override fun onStart(utteranceId: String?) {}
                override fun onDone(utteranceId: String?) {
                    val r = runCatching { ProbePcm.to16kMono(ProbePcm.readWav(out.readBytes())) }
                    runOnUiThread {
                        r.onSuccess { pcm -> ProbeRuntime.clip = ProbeRuntime.Clip(pcm, "clip_tts", "TTS \"${sentence.take(40)}\""); showClip() }
                            .onFailure { clipInfo.text = "TTS clip unreadable: ${it.message}" }
                    }
                }
                @Deprecated("Deprecated in Java")
                override fun onError(utteranceId: String?) { runOnUiThread { clipInfo.text = "TTS error" } }
                override fun onError(utteranceId: String?, errorCode: Int) { runOnUiThread { clipInfo.text = "TTS error $errorCode" } }
            })
            val rc = t.synthesizeToFile(sentence, Bundle(), out, "probe")
            if (rc != TextToSpeech.SUCCESS) clipInfo.text = "synthesizeToFile failed ($rc)"
        }
        if (engine != null) { speak(engine); return }
        var created: TextToSpeech? = null
        created = TextToSpeech(this) { status ->
            if (status == TextToSpeech.SUCCESS) created?.let { tts = it; speak(it) }
            else clipInfo.text = "TTS init failed ($status)"
        }
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQ_PICK || resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        clipInfo.text = "reading…"
        thread(name = "probe-pick") {
            val r = runCatching {
                val bytes = contentResolver.openInputStream(uri)!!.use { it.readBytes() }
                ProbePcm.to16kMono(ProbePcm.readWav(bytes))
            }
            runOnUiThread {
                r.onSuccess { ProbeRuntime.clip = ProbeRuntime.Clip(it, "clip_file", "picked WAV"); showClip() }
                    .onFailure { clipInfo.text = "Not usable: ${it.message} (needs a PCM16 WAV)" }
            }
        }
    }

    companion object {
        private const val REQ_MIC = 1
        private const val REQ_PICK = 2
        /** Synthetic test sentence (tech terms the biasing list names). */
        const val DEFAULT_SENTENCE = "Please run pytest on the new UUID4 helper, then ask Kubectl for the pod logs before Friday."
    }
}

