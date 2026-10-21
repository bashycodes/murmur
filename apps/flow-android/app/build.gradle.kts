import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.Collections
import java.util.zip.ZipEntry
import java.util.zip.ZipFile

plugins {
    id("com.android.application")
}

android {
    namespace = "dev.flowclone"
    compileSdk = 35
    // Same NDK as tools/build-native.sh, so AGP can strip the prebuilt jniLibs.
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "dev.flowclone"
        minSdk = 33
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a") }
    }

    // D17 (docs/hybrid-architecture.md §1.4): one applicationId, three network capabilities.
    // Switching flavour is an in-place update that keeps prefs, dictionary and history.
    //   local  - on-device ASR only; no network permission at all (I3)
    //   hybrid - on-device ASR + self-hosted flow-server; adds only INTERNET
    //   remote - flow-server only; adds only INTERNET; no ASR/LLM native libs or models (I4)
    // jniLibs live in src/<flavour>/jniLibs (tools/build-native.sh --flavor <f>).
    flavorDimensions += "net"
    productFlavors {
        create("local") {
            dimension = "net"
            buildConfigField("boolean", "CAP_LOCAL", "true")
            buildConfigField("boolean", "CAP_REMOTE", "false")
        }
        create("hybrid") {
            dimension = "net"
            buildConfigField("boolean", "CAP_LOCAL", "true")
            buildConfigField("boolean", "CAP_REMOTE", "true")
        }
        create("remote") {
            dimension = "net"
            buildConfigField("boolean", "CAP_LOCAL", "false")
            buildConfigField("boolean", "CAP_REMOTE", "true")
        }
    }
    buildFeatures { buildConfig = true }
    sourceSets {
        // RouterJsonTest parses the router's own fixtures (no copies).
        getByName("test").resources.directories.add("../../../crates/flow-router/fixtures")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
    testImplementation("org.json:json:20240303")
}

// tools/probes/jni_stub/run.sh: link-test NativeFlowCore against a host stub .so.
// tools/probes/host_native/run.sh: load a host build of the real crates/flow-ffi instead.
val jniStubDir = providers.gradleProperty("flow.jniStubDir").orNull
val jniLibDir = providers.gradleProperty("flow.jniLibDir").orNull
tasks.withType<Test>().configureEach {
    if (jniStubDir != null) {
        systemProperty("java.library.path", jniStubDir)
        systemProperty("flow.jniStub", "1")
    } else if (jniLibDir != null) {
        systemProperty("java.library.path", jniLibDir)
        systemProperty("flow.jniReal", "1")
        for (k in listOf("flow.server", "flow.model", "flow.wav", "flow.threads", "flow.reportDir", "flow.llmModel", "flow.llmLib")) {
            providers.gradleProperty(k).orNull?.let { systemProperty(k, it) }
        }
        testLogging { showStandardStreams = true }
    }
}

// ---- Per-flavour privacy / packaging gates (docs/hybrid-architecture.md §1.4) ----
// Each reads the variant's MERGED manifest (so permissions added by any dependency count)
// or its packaged APK. assemble*/test*UnitTest/check depend on them; ManifestPrivacyTest
// and FlavorGatesTest re-check the same facts from the unit-test suite.
val networkPermissions = setOf(
    "android.permission.INTERNET",
    "android.permission.ACCESS_NETWORK_STATE",
    "android.permission.CHANGE_NETWORK_STATE",
    "android.permission.ACCESS_WIFI_STATE",
    "android.permission.CHANGE_WIFI_STATE",
)

abstract class VerifyManifestNetwork : DefaultTask() {
    @get:InputFile abstract val manifest: RegularFileProperty
    @get:OutputFile abstract val report: RegularFileProperty
    /** Network permissions the flavour must declare (empty for `local`). */
    @get:Input abstract val required: SetProperty<String>
    @get:Input abstract val forbidden: SetProperty<String>

    @TaskAction
    fun verify() {
        val xml = manifest.get().asFile.readText()
        val declared = Regex("""<uses-permission(?:-sdk-23)?\s[^>]*android:name="([^"]+)"""")
            .findAll(xml).map { it.groupValues[1] }.toSet()
        val bad = declared intersect forbidden.get()
        val missing = required.get() - declared
        report.get().asFile.apply { parentFile.mkdirs() }
            .writeText("declared=${declared.sorted()}\nforbidden=${bad.sorted()}\nmissing=${missing.sorted()}\n")
        if (bad.isNotEmpty()) throw GradleException(
            "Merged manifest declares network permissions $bad that this flavour must not have (D7/D9/D17 I3).",
        )
        if (missing.isNotEmpty()) throw GradleException("Merged manifest lacks $missing")
        if (Regex("""usesCleartextTraffic="true"""").containsMatchIn(xml)) throw GradleException("usesCleartextTraffic=true in merged manifest")
    }
}

/**
 * Inspects the packaged APK(s). `remote`: no ASR/LLM native libs and a libflow_ffi.so that
 * exports no whisper_/llama_ symbols (I4). `local`: libflow_ffi.so carries no WebSocket/TLS
 * client code (I3, backs up the cargo-tree gate in tools/check-build-matrix.sh).
 */
abstract class VerifyApkNativeLibs : DefaultTask() {
    @get:InputFiles abstract val apkDir: DirectoryProperty
    @get:OutputFile abstract val report: RegularFileProperty
    @get:Input abstract val mode: Property<String>

    /** Defined dynamic symbols of an ELF64 little-endian shared object (enough for arm64 / x86_64). */
    fun elfDynamicSymbols(b: ByteArray): Set<String> {
        val bb = ByteBuffer.wrap(b).order(ByteOrder.LITTLE_ENDIAN)
        if (b.size < 64 || bb.getInt(0) != 0x464c457f || b[4].toInt() != 2) return emptySet()
        val shoff = bb.getLong(0x28).toInt(); val shentsize = bb.getShort(0x3A).toInt(); val shnum = bb.getShort(0x3C).toInt()
        fun sh(i: Int) = shoff + i * shentsize
        val out = HashSet<String>()
        for (i in 0 until shnum) {
            if (bb.getInt(sh(i) + 4) != 11) continue // SHT_DYNSYM
            val off = bb.getLong(sh(i) + 0x18).toInt(); val size = bb.getLong(sh(i) + 0x20).toInt()
            val strOff = bb.getLong(sh(bb.getInt(sh(i) + 0x28)) + 0x18).toInt()
            var p = off
            while (p + 24 <= off + size) {
                val name = bb.getInt(p); val shndx = bb.getShort(p + 6).toInt() and 0xffff
                if (name != 0 && shndx != 0) {
                    var e = strOff + name
                    while (b[e].toInt() != 0) e++
                    out += String(b, strOff + name, e - strOff - name, Charsets.US_ASCII)
                }
                p += 24
            }
        }
        return out
    }

    @TaskAction
    fun verify() {
        val apks = apkDir.get().asFile.walk().filter { it.isFile && it.name.endsWith(".apk") }.toList()
        if (apks.isEmpty()) throw GradleException("no APK in ${apkDir.get().asFile}")
        val lines = mutableListOf<String>()
        val errors = mutableListOf<String>()
        for (apk in apks) {
            val zip = ZipFile(apk)
            val all: List<ZipEntry> = Collections.list(zip.entries())
            val libs = all.filter { it.name.startsWith("lib/") && it.name.endsWith(".so") }
            lines += "${apk.name}: ${libs.map { "${it.name} (${it.size} B)" }}"
            for (e in libs) {
                val n = e.name.substringAfterLast('/')
                val bytes: ByteArray = if (n == "libflow_ffi.so") zip.getInputStream(e).readBytes() else ByteArray(0)
                when (mode.get()) {
                    "noAsr" -> {
                        if (Regex("""^lib(whisper|sherpa|onnxruntime|flow_llm|c\+\+_shared|llama|ggml).*""").matches(n)) errors += "${apk.name}: $n must not ship in the remote flavour"
                        if (n == "libflow_ffi.so") {
                            val asr = elfDynamicSymbols(bytes).filter { it.startsWith("whisper_") || it.startsWith("llama_") || it.startsWith("ggml_") || it.startsWith("SherpaOnnx") }
                            if (asr.isNotEmpty()) errors += "${apk.name}: libflow_ffi.so exports ASR/LLM symbols ${asr.take(5)}"
                        }
                    }
                    "noNetCode" -> if (n == "libflow_ffi.so") {
                        val s = String(bytes, Charsets.ISO_8859_1)
                        for (m in listOf("tungstenite", "rustls", "flow-remote", "flow_remote")) if (s.contains(m)) errors += "${apk.name}: libflow_ffi.so contains '$m' (network code in the local flavour)"
                    }
                }
            }
            zip.close()
        }
        report.get().asFile.apply { parentFile.mkdirs() }.writeText((lines + errors.map { "ERROR $it" }).joinToString("\n") + "\n")
        if (errors.isNotEmpty()) throw GradleException(errors.joinToString("\n"))
    }
}

androidComponents {
    onVariants { variant ->
        val cap = variant.name.replaceFirstChar { it.uppercase() }
        val bt = (variant.buildType ?: "").replaceFirstChar { it.uppercase() }
        val flavor = variant.flavorName ?: ""
        val fl = flavor.replaceFirstChar { it.uppercase() }
        val manifestGate = tasks.register<VerifyManifestNetwork>(
            if (flavor == "local") "verify${cap}NoNetworkPermissions" else "verify$fl${bt}OnlyInternetPermission",
        ) {
            manifest.set(variant.artifacts.get(com.android.build.api.artifact.SingleArtifact.MERGED_MANIFEST))
            report.set(layout.buildDirectory.file("reports/network-permissions/${variant.name}.txt"))
            if (flavor == "local") { required.set(emptySet()); forbidden.set(networkPermissions) }
            else { required.set(setOf("android.permission.INTERNET")); forbidden.set(networkPermissions - "android.permission.INTERNET") }
        }
        val apkGate = when (flavor) {
            "remote" -> tasks.register<VerifyApkNativeLibs>("verifyRemote${bt}NoAsrLibs") { mode.set("noAsr") }
            "local" -> tasks.register<VerifyApkNativeLibs>("verifyLocal${bt}NoNetworkCode") { mode.set("noNetCode") }
            else -> null
        }
        apkGate?.configure {
            apkDir.set(variant.artifacts.get(com.android.build.api.artifact.SingleArtifact.APK))
            report.set(layout.buildDirectory.file("reports/native-libs/${variant.name}.txt"))
        }
        tasks.matching { it.name == "assemble$cap" || it.name == "test${cap}UnitTest" || it.name == "check" }
            .configureEach { dependsOn(manifestGate) }
        tasks.matching { it.name == "assemble$cap" || it.name == "check" }
            .configureEach { apkGate?.let { g -> dependsOn(g) } }
    }
}

// `./gradlew testDebugUnitTest` keeps meaning "every flavour" (the merge-order checks in
// docs/hybrid-architecture.md §6.3 run it); assembleDebug already aggregates.
tasks.register("testDebugUnitTest") {
    group = "verification"
    dependsOn("testLocalDebugUnitTest", "testHybridDebugUnitTest", "testRemoteDebugUnitTest")
}
