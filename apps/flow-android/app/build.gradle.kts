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
val jniStubDir = providers.gradleProperty("flow.jniStubDir").orNull
tasks.withType<Test>().configureEach {
    if (jniStubDir != null) {
        systemProperty("java.library.path", jniStubDir)
        systemProperty("flow.jniStub", "1")
    }
}

// ---- Privacy gate: the APK must never be able to reach the network. ----
// Reads each variant's MERGED manifest (so permissions added by any dependency count)
// and fails the build if a network permission survives. assemble*/test*UnitTest/check
// depend on it; ManifestPrivacyTest re-checks the same file from the unit-test suite.
abstract class VerifyNoNetworkPermissions : DefaultTask() {
    @get:InputFile abstract val manifest: RegularFileProperty
    @get:OutputFile abstract val report: RegularFileProperty

    @TaskAction
    fun verify() {
        val forbidden = setOf(
            "android.permission.INTERNET",
            "android.permission.ACCESS_NETWORK_STATE",
            "android.permission.CHANGE_NETWORK_STATE",
            "android.permission.ACCESS_WIFI_STATE",
            "android.permission.CHANGE_WIFI_STATE",
        )
        val xml = manifest.get().asFile.readText()
        val declared = Regex("""<uses-permission(?:-sdk-23)?\s[^>]*android:name="([^"]+)"""")
            .findAll(xml).map { it.groupValues[1] }.toSet()
        val bad = declared intersect forbidden
        report.get().asFile.apply { parentFile.mkdirs() }
            .writeText("declared=${declared.sorted()}\nforbidden=${bad.sorted()}\n")
        if (bad.isNotEmpty()) throw GradleException(
            "Merged manifest declares network permissions $bad - the clone must have no network access (DECISIONS D7/D9).",
        )
        if (Regex("""usesCleartextTraffic="true"""").containsMatchIn(xml)) throw GradleException("usesCleartextTraffic=true in merged manifest")
    }
}

androidComponents {
    onVariants { variant ->
        val cap = variant.name.replaceFirstChar { it.uppercase() }
        val verify = tasks.register<VerifyNoNetworkPermissions>("verify${cap}NoNetworkPermissions") {
            manifest.set(variant.artifacts.get(com.android.build.api.artifact.SingleArtifact.MERGED_MANIFEST))
            report.set(layout.buildDirectory.file("reports/network-permissions/${variant.name}.txt"))
        }
        tasks.matching { it.name == "assemble$cap" || it.name == "test${cap}UnitTest" || it.name == "check" }
            .configureEach { dependsOn(verify) }
    }
}
