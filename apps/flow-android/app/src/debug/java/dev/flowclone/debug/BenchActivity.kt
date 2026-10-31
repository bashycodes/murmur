package dev.flowclone.debug

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Debug
import android.os.PowerManager
import android.util.Log
import android.view.WindowManager
import android.widget.ScrollView
import android.widget.TextView
import dev.flowclone.BuildConfig
import dev.flowclone.core.NativeFlowCore
import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Debug builds only (src/debug): the on-device benchmark (tools/probes/android_bench/README.md).
 * Exported for the platform tools' shell and guarded by android.permission.DUMP, like
 * [WavMicReceiver]: `am start -n dev.flowclone/.debug.BenchActivity --es suite all --es run r1`.
 * Reads the bundle from `files/bench/in` (copied in with run-as; `<external files>/bench/in` is
 * also tried, but files the shell puts there are unreadable to the app), writes
 * `<external files>/bench/out/<run>/`. Keeps the screen on while it runs; `--ez stop true` stops
 * after the current item. One run at a time per process.
 */
class BenchActivity : Activity() {
    companion object {
        private const val TAG = "FlowBench"
        @Volatile private var running = false
        @Volatile private var stopRequested = false
    }

    private lateinit var text: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        text = TextView(this).apply { setPadding(32, 48, 32, 32); textSize = 13f; setTextIsSelectable(true) }
        setContentView(ScrollView(this).apply { addView(text) })
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    private fun show(line: String) {
        Log.i(TAG, line)
        runOnUiThread { text.append(line + "\n") }
    }

    private fun handle(intent: Intent) {
        val extras = intent.extras
        if (extras?.getBoolean("stop", false) == true) {
            stopRequested = true
            show("stop requested")
            return
        }
        if (running) return show("a benchmark is already running")
        val args = try {
            @Suppress("DEPRECATION")
            BenchArgs.parse { k -> extras?.get(k)?.toString() }
        } catch (e: IllegalArgumentException) {
            return show("bad arguments: ${e.message}")
        }
        val ext = getExternalFilesDir(null)
        val inDir = args.inDir?.let(::File)
            ?: listOfNotNull(ext?.let { File(it, "bench/in") }, File(filesDir, "bench/in")).firstOrNull { File(it, "manifest.json").isFile }
        if (inDir == null || !File(inDir, "manifest.json").isFile) return show("no bundle: manifest.json missing in ${File(filesDir, "bench/in")} (tools/probes/android_bench/run_on_device.sh push)")
        val run = args.run ?: SimpleDateFormat("yyyyMMdd-HHmmss", Locale.ROOT).format(Date())
        val outDir = File(ext ?: filesDir, "bench/out/$run")
        // Parakeet derives bpe.vocab into TMPDIR (as FlowApp.load does).
        runCatching { android.system.Os.setenv("TMPDIR", cacheDir.path, true) }
        running = true
        stopRequested = false
        show("bench ${BuildConfig.VERSION_NAME} (${BuildConfig.BUILD}) suite=${args.suite} threads=${args.threads}\nin=$inDir\nout=$outDir")
        val power = getSystemService(PowerManager::class.java)
        val device = JSONObject().put("model", Build.MODEL).put("soc", Build.SOC_MODEL).put("sdk", Build.VERSION.SDK_INT)
            .put("cores", Runtime.getRuntime().availableProcessors()).put("app_version", BuildConfig.VERSION_NAME)
            .put("app_build", BuildConfig.BUILD).put("flavor", BuildConfig.FLAVOR)
            .put("native", runCatching { NativeFlowCore.loadLibrary().toString() }.getOrDefault("?"))
        Thread({
            try {
                val meta = BenchRunner(
                    args, filesDir, inDir, outDir, NativeFlowCore,
                    clean = { core, req -> (core as NativeFlowCore).benchClean(req) },
                    device = device, log = ::show, stopped = { stopRequested },
                    extra = {
                        JSONObject().put("pss_kb", Debug.getPss()).put("native_heap_kb", Debug.getNativeHeapAllocatedSize() / 1024)
                            .put("thermal", power?.currentThermalStatus ?: -1)
                    },
                ).run()
                show("finished: ${meta.optString("error", "ok")}\n${File(outDir, "status.json").takeIf { it.isFile }?.readText() ?: ""}")
            } finally {
                running = false
            }
        }, "flow-bench").start()
    }
}
