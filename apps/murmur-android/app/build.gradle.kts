import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.Collections
import java.util.zip.ZipEntry
import java.util.zip.ZipFile

plugins {
    id("com.android.application")
}

// ---- Product version (D26, docs/versioning.md) ----
// The repo's VERSION file is the only source; versionName/versionCode are derived here and
// tools/check-version.sh fails on a literal. The formula is duplicated in
// dev.flowclone.data.AppVersion (app code cannot be called from here); AppVersionTest pins the two.
val flowVersionFile: File = rootProject.file("../../VERSION")
val flowVersion: String = providers.fileContents(rootProject.layout.projectDirectory.file("../../VERSION")).asText.get().trim()
fun flowVersionCode(v: String): Int {
    val m = Regex("""(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)""").matchEntire(v)
        ?: throw GradleException("VERSION '$v' is not MAJOR.MINOR.PATCH (docs/versioning.md)")
    val (major, minor, patch) = m.groupValues.drop(1).map { it.toInt() }
    if (major > 209_999 || minor > 99 || patch > 99) throw GradleException("VERSION '$v' does not fit a versionCode")
    return major * 10_000 + minor * 100 + patch
}
// Build metadata: the commit and whether the tree had uncommitted changes (tracked files only).
// FLOW_GIT_SHA / FLOW_GIT_DIRTY override (a build from a tarball without .git shows "unknown").
fun flowGit(vararg args: String): String? = runCatching {
    providers.exec {
        commandLine("git", *args)
        workingDir = rootDir
        isIgnoreExitValue = true
    }.standardOutput.asText.get().trim()
}.getOrNull()?.takeIf { it.isNotEmpty() }
val flowGitSha: String = System.getenv("FLOW_GIT_SHA") ?: flowGit("rev-parse", "--short=7", "HEAD") ?: ""
val flowGitDirty: Boolean = System.getenv("FLOW_GIT_DIRTY")?.let { it == "1" || it == "true" }
    ?: (flowGit("status", "--porcelain", "--untracked-files=no") != null)
val flowBuild: String = when {
    flowGitSha.isEmpty() -> "unknown"
    flowGitDirty -> "$flowGitSha-dirty"
    else -> flowGitSha
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
        versionCode = flowVersionCode(flowVersion)
        versionName = flowVersion
        buildConfigField("String", "GIT_SHA", "\"$flowGitSha\"")
        buildConfigField("boolean", "GIT_DIRTY", "$flowGitDirty")
        buildConfigField("String", "BUILD", "\"$flowBuild\"")
        ndk { abiFilters += listOf("arm64-v8a") }
    }

    // D17 (docs/hybrid-architecture.md §1.4): one applicationId, three network capabilities.
    // Switching flavour is an in-place update that keeps prefs, dictionary and history.
    //   local  - on-device ASR only; no network permission at all (I3)
    //   hybrid - on-device ASR + self-hosted flow-server; adds only INTERNET
    //   remote - flow-server only; adds only INTERNET; no ASR/LLM native libs or models (I4)
    // D18: CAP_CLOUD = the native library carries the opt-in third-party STT client (hybrid,
    // remote; never local: C2). No new permission: it uses the INTERNET hybrid/remote already have.
    // jniLibs live in src/<flavour>/jniLibs (tools/build-native.sh --flavor <f>).
    flavorDimensions += "net"
    productFlavors {
        create("local") {
            dimension = "net"
            buildConfigField("boolean", "CAP_LOCAL", "true")
            buildConfigField("boolean", "CAP_REMOTE", "false")
            buildConfigField("boolean", "CAP_CLOUD", "false")
        }
        create("hybrid") {
            dimension = "net"
            buildConfigField("boolean", "CAP_LOCAL", "true")
            buildConfigField("boolean", "CAP_REMOTE", "true")
            buildConfigField("boolean", "CAP_CLOUD", "true")
        }
        create("remote") {
            dimension = "net"
            buildConfigField("boolean", "CAP_LOCAL", "false")
            buildConfigField("boolean", "CAP_REMOTE", "true")
            buildConfigField("boolean", "CAP_CLOUD", "true")
        }
    }
    buildFeatures { buildConfig = true }
    sourceSets {
        // RouterJsonTest parses the router's own fixtures (no copies).
        getByName("test").resources.directories.add("../../../crates/flow-router/fixtures")
        // AppVersionTest: the version-rule vectors tools/versioning/test_versioning.py also checks.
        getByName("test").resources.directories.add("../../../tools/versioning/fixtures")
        // InsightsVectorsTest: flow-core's Insights vectors (insights/insights_vectors.json).
        getByName("test").resources.directories.add("../../../crates/flow-core/fixtures")
        // D41: the model-library client (server downloads) exists only where there is a server:
        // src/server in hybrid + remote, a no-op stand-in (src/noserver) in local, so the local
        // APK carries no library code at all (verifyLocal*NoNetworkCode checks the dex too).
        for ((set, dir) in listOf("hybrid" to "src/server/java", "remote" to "src/server/java", "local" to "src/noserver/java",
            "testHybrid" to "src/testServer/java", "testRemote" to "src/testServer/java")) {
            getByName(set).java.directories.add(dir)
            getByName(set).kotlin.directories.add(dir)
        }
    }

    // Release signing without secrets in git: the keystore and its passwords come from Gradle
    // properties or environment variables (e.g. ~/.gradle/gradle.properties, or CI secrets):
    //   flow.release.storeFile / FLOW_RELEASE_STORE_FILE, flow.release.storePassword / FLOW_RELEASE_STORE_PASSWORD,
    //   flow.release.keyAlias / FLOW_RELEASE_KEY_ALIAS, flow.release.keyPassword / FLOW_RELEASE_KEY_PASSWORD.
    // Without them `assemble<Flavour>Release` still builds, unsigned (INSTALL.md "Release build").
    fun releaseProp(name: String, env: String): String? =
        providers.gradleProperty("flow.release.$name").orNull ?: System.getenv(env)
    val releaseStore = releaseProp("storeFile", "FLOW_RELEASE_STORE_FILE")?.let { file(it) }?.takeIf { it.isFile }
    if (releaseStore != null) {
        signingConfigs {
            create("release") {
                storeFile = releaseStore
                storePassword = releaseProp("storePassword", "FLOW_RELEASE_STORE_PASSWORD")
                keyAlias = releaseProp("keyAlias", "FLOW_RELEASE_KEY_ALIAS")
                keyPassword = releaseProp("keyPassword", "FLOW_RELEASE_KEY_PASSWORD")
            }
        }
    }

    // D56: CI signs debug builds with the owner's fixed debug key, decoded from a repo secret to a
    // file named by flow.debug.storeFile / FLOW_DEBUG_STORE_FILE (standard AGP debug keystore:
    // password "android", alias "androiddebugkey"). Unset or missing file: AGP's default
    // ~/.android/debug.keystore, as before (local builds).
    val debugStore = (providers.gradleProperty("flow.debug.storeFile").orNull ?: System.getenv("FLOW_DEBUG_STORE_FILE"))
        ?.let { file(it) }?.takeIf { it.isFile }
    if (debugStore != null) {
        signingConfigs.getByName("debug") {
            storeFile = debugStore
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (releaseStore != null) signingConfig = signingConfigs.getByName("release")
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

// AGP does not pre-create flavour+build-type configurations; it picks this one up by name.
configurations.create("hybridDebugImplementation")

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    // English 1 spike (docs/findings/2026-11-02-pixel-offline-dictation.md, PQ-a): ML Kit GenAI speech
    // (Gemini Nano) for the debug-only probe screen in src/hybridDebug. hybridDebug only, so no
    // release APK and no local/remote build carries it (its telemetry deps would add network code).
    // alpha1, not alpha2: alpha2 is built with Kotlin 2.4 metadata, which our Kotlin 2.2 cannot read.
    "hybridDebugImplementation"("com.google.mlkit:genai-speech-recognition:1.0.0-alpha1")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
    testImplementation("org.json:json:20240303")
    // SecretFieldTest: real View / AccessibilityNodeInfo behaviour (the a11y tree must never hold a secret).
    testImplementation("org.robolectric:robolectric:4.14.1")
}

// tools/probes/jni_stub/run.sh: link-test NativeFlowCore against a host stub .so.
// tools/probes/host_native/run.sh: load a host build of the real crates/flow-ffi instead.
val jniStubDir = providers.gradleProperty("flow.jniStubDir").orNull
val jniLibDir = providers.gradleProperty("flow.jniLibDir").orNull
tasks.withType<Test>().configureEach {
    systemProperty("flow.versionFile", flowVersionFile.absolutePath)
    // ServerCatalogTest: the Android pins equal the Windows catalogue's (one set of hashes), the ids and engines flow-protocol's.
    systemProperty("flow.windowsCatalog", rootProject.file("../flow-windows/src/model_download.rs").absolutePath)
    systemProperty("flow.protocolLibrary", rootProject.file("../../crates/flow-protocol/src/library.rs").absolutePath)
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

// D80: the repo's COPYRIGHT, NOTICE and THIRD-PARTY-LICENSES ship in every APK as
// assets/licenses/{COPYRIGHT,NOTICE,THIRD-PARTY-LICENSES}.txt
// (next to the fonts' OFL texts), copied from the repo root at build time so there is one copy.
abstract class LegalAssets : DefaultTask() {
    @get:InputFiles abstract val sources: ConfigurableFileCollection
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun copy() {
        val dir = outputDir.get().asFile.resolve("licenses")
        dir.deleteRecursively()
        dir.mkdirs()
        for (f in sources.files) {
            if (!f.isFile) throw GradleException("$f not found (D80: every APK carries it)")
            f.copyTo(dir.resolve("${f.name}.txt"), overwrite = true)
        }
    }
}
val legalAssets = tasks.register<LegalAssets>("legalAssets") {
    sources.from(file("../../../COPYRIGHT"), file("../../../NOTICE"), file("../../../THIRD-PARTY-LICENSES"))
    outputDir.set(layout.buildDirectory.dir("generated/legal-assets"))
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
 * Release variants: the debug "WAV as microphone" receiver, the BenchActivity (src/debug) and the
 * EnglishProbeActivity (src/hybridDebug) must
 * not be in the merged manifest, so nothing outside the app can make a release build dictate. NoDebugWavReceiverTest
 * re-checks it (and that the class is absent) from the release unit-test suite.
 */
abstract class VerifyNoDebugReceiver : DefaultTask() {
    @get:InputFile abstract val manifest: RegularFileProperty
    @get:OutputFile abstract val report: RegularFileProperty

    @TaskAction
    fun verify() {
        val xml = manifest.get().asFile.readText()
        val found = listOf("WavMicReceiver", "dev.flowclone.debug.PLAY_WAV", "BenchActivity", "CallDecodeReceiver", "EnglishProbeActivity").filter { xml.contains(it) }
        report.get().asFile.apply { parentFile.mkdirs() }.writeText("found=$found\n")
        if (found.isNotEmpty()) throw GradleException("Release manifest contains a debug-only component ($found)")
    }
}

/**
 * Inspects the packaged APK(s). `remote`: no ASR/LLM native libs and a libflow_ffi.so that
 * exports no whisper_/llama_ symbols (I4). `local`: libflow_ffi.so carries no WebSocket/TLS
 * client code (I3) and no cloud STT client (D18 C2: reqwest / webpki / flow-cloud); backs up the
 * cargo-tree gate in tools/check-build-matrix.sh.
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
                        for (m in listOf("tungstenite", "rustls", "flow-remote", "flow_remote", "reqwest", "webpki", "flow-cloud", "Java_dev_flowclone_server_")) if (s.contains(m)) errors += "${apk.name}: libflow_ffi.so contains '$m' (network / cloud STT / model-library code in the local flavour, D17 I3 / D18 C2 / D41)"
                    }
                }
            }
            // D41: the local flavour has no server-library classes (src/server is hybrid / remote only).
            // Its one dev.flowclone.server class is src/noserver's no-op ServerModels stand-in (main
            // code imports that name in every flavour), so any other class in the package fails.
            if (mode.get() == "noNetCode") for (e in all.filter { it.name.matches(Regex("""classes\d*\.dex""")) }) {
                val s = String(zip.getInputStream(e).readBytes(), Charsets.ISO_8859_1)
                val server = Regex("""Ldev/flowclone/server/[^;\u0000]*;""").findAll(s).map { it.value }
                    .filter { it != "Ldev/flowclone/server/ServerModels;" }.toSortedSet()
                if (server.isNotEmpty()) errors += "${apk.name}: ${e.name} contains dev.flowclone.server classes ${server.take(5)} (model library in the local flavour, D41)"
            }
            zip.close()
        }
        report.get().asFile.apply { parentFile.mkdirs() }.writeText((lines + errors.map { "ERROR $it" }).joinToString("\n") + "\n")
        if (errors.isNotEmpty()) throw GradleException(errors.joinToString("\n"))
    }
}

/**
 * Copies flow-core's `data/app-groups.json` (D81) into a generated Java-resource dir, so that
 * file alone ships (PolicyLists reads it); later files under crates/flow-core/data do not.
 */
abstract class CopyAppGroups : DefaultTask() {
    @get:InputFile abstract val source: RegularFileProperty
    @get:OutputDirectory abstract val outDir: DirectoryProperty

    @TaskAction
    fun copy() {
        val d = outDir.get().asFile
        d.deleteRecursively()
        source.get().asFile.copyTo(File(d.apply { mkdirs() }, "app-groups.json"))
    }
}

/** Fails when a packaged APK lacks `app-groups.json` (PolicyLists would fall back to hiding the bubble everywhere). */
abstract class VerifyApkAppGroups : DefaultTask() {
    @get:InputFiles abstract val apkDir: DirectoryProperty
    @get:OutputFile abstract val report: RegularFileProperty

    @TaskAction
    fun verify() {
        val apks = apkDir.get().asFile.walk().filter { it.isFile && it.name.endsWith(".apk") }.toList()
        if (apks.isEmpty()) throw GradleException("no APK in ${apkDir.get().asFile}")
        val missing = apks.filter { apk -> ZipFile(apk).use { it.getEntry("app-groups.json") == null } }.map { it.name }
        report.get().asFile.apply { parentFile.mkdirs() }
            .writeText(apks.joinToString("\n") { "${it.name}: app-groups.json ${if (it.name in missing) "MISSING" else "ok"}" } + "\n")
        if (missing.isNotEmpty()) throw GradleException("app-groups.json missing from $missing (D81)")
    }
}

androidComponents {
    // AGP 9 creates unit tests only for the tested build type (debug). Release keeps its own
    // suite (src/testRelease) so it can prove the debug-only "WAV as microphone" receiver is absent.
    beforeVariants(selector().withBuildType("release")) { v ->
        (v as? com.android.build.api.variant.HasHostTestsBuilder)
            ?.hostTests?.get(com.android.build.api.variant.HostTestBuilder.UNIT_TEST_TYPE)?.enable = true
    }
    onVariants { variant ->
        variant.sources.assets?.addGeneratedSourceDirectory(legalAssets, LegalAssets::outputDir)
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
        // D81: PolicyLists reads app-groups.json, the app groups flow-core and the Windows app use.
        val appGroups = tasks.register<CopyAppGroups>("copy${cap}AppGroups") {
            source.set(rootProject.layout.projectDirectory.file("../../crates/flow-core/data/app-groups.json"))
        }
        variant.sources.resources?.addGeneratedSourceDirectory(appGroups, CopyAppGroups::outDir)
        val appGroupsGate = tasks.register<VerifyApkAppGroups>("verify${cap}AppGroups") {
            apkDir.set(variant.artifacts.get(com.android.build.api.artifact.SingleArtifact.APK))
            report.set(layout.buildDirectory.file("reports/app-groups/${variant.name}.txt"))
        }
        tasks.matching { it.name == "assemble$cap" || it.name == "check" }
            .configureEach { dependsOn(appGroupsGate) }
        apkGate?.configure {
            apkDir.set(variant.artifacts.get(com.android.build.api.artifact.SingleArtifact.APK))
            report.set(layout.buildDirectory.file("reports/native-libs/${variant.name}.txt"))
        }
        tasks.matching { it.name == "assemble$cap" || it.name == "test${cap}UnitTest" || it.name == "check" }
            .configureEach { dependsOn(manifestGate) }
        tasks.matching { it.name == "assemble$cap" || it.name == "check" }
            .configureEach { apkGate?.let { g -> dependsOn(g) } }
        if (variant.buildType == "release") {
            val noDebugReceiver = tasks.register<VerifyNoDebugReceiver>("verify${cap}NoDebugWavReceiver") {
                manifest.set(variant.artifacts.get(com.android.build.api.artifact.SingleArtifact.MERGED_MANIFEST))
                report.set(layout.buildDirectory.file("reports/debug-receiver/${variant.name}.txt"))
            }
            tasks.matching { it.name == "assemble$cap" || it.name == "test${cap}UnitTest" || it.name == "check" }
                .configureEach { dependsOn(noDebugReceiver) }
        }
    }
}

// `./gradlew testDebugUnitTest` keeps meaning "every flavour" (the merge-order checks in
// docs/hybrid-architecture.md §6.3 run it); assembleDebug already aggregates.
tasks.register("testDebugUnitTest") {
    group = "verification"
    dependsOn("testLocalDebugUnitTest", "testHybridDebugUnitTest", "testRemoteDebugUnitTest")
}
