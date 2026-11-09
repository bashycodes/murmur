package dev.flowclone.core;

// Single-file source launch (crates/flow-ffi/tests/jni_cloud.rs):
//   java -Djava.library.path=DIR JniCloud.java <key> <cacheDir>
// The shell's D28 OpenRouter flow through the real JNI exports + native router,
// offline (no request reaches openrouter.ai: no consent / catalogue from the baked
// snapshot; the network paths are covered by flow-contract-tests with the mock).
// Every line printed is checked by jni_cloud.rs (which also asserts the key never appears).
public class JniCloud {
    static void check(boolean ok, String what) { if (!ok) throw new RuntimeException(what); }

    // A D18 (schema 1) config: OpenRouter kept, Groq active and removed.
    static final String LEGACY = "{\"allow_server\":false,\"allow_client\":true,\"allow_cloud\":true,"
        + "\"order\":[\"cloud\",\"client\"],\"cloud\":{\"active\":\"groq-1\",\"providers\":["
        + "{\"id\":\"openrouter-1\",\"preset\":\"openrouter\",\"adapter\":\"openrouter\",\"base_url\":\"https://openrouter.ai/api/v1\","
        + "\"model\":\"openai/gpt-transcribe\",\"zero_retention\":true,\"consent_host\":\"openrouter.ai\",\"consent_at_ms\":1},"
        + "{\"id\":\"groq-1\",\"preset\":\"groq\",\"adapter\":\"openai_compat\",\"label\":\"Groq\",\"base_url\":\"https://api.groq.com/openai/v1\",\"model\":\"m\"}]}}";

    static final String SESSION = "{\"languages\":[\"en\"],\"platform\":\"android\",\"llm_skip_clean\":false,"
        + "\"dictionary\":[{\"word\":\"btw\",\"replacement\":\"by the way\",\"isSnippet\":false}]}";

    static String dictate(long r) {
        check(NativeFlowCore.nativeStartSession(r, SESSION), "start: " + NativeFlowCore.nativeLastError());
        short[] buf = new short[1600];
        for (int c = 0; c < 15; c++) {
            for (int i = 0; i < buf.length; i++) buf[i] = (short) (Math.sin((c * 1600 + i) * 0.05) * 8000);
            NativeFlowCore.nativePushAudio(r, buf, buf.length);
        }
        return NativeFlowCore.nativeCommit(r);
    }

    public static void main(String[] args) {
        String key = args[0], cache = args[1];
        System.loadLibrary("flow_ffi");
        System.out.println(NativeFlowCore.nativeCloudCapabilities());                       // 1 caps
        System.out.println(NativeFlowCore.nativeCloudPresets());                            // 2 one preset
        System.out.println(NativeFlowCore.nativeMigrateCloud(LEGACY));                      // 3 migration
        check(NativeFlowCore.nativeMigrateCloud("[") == null, "bad json");
        System.out.println("KIND " + NativeFlowCore.nativeLastErrorKind());                 // 4

        NativeFlowCore.nativeSetCloudCacheDir(cache);
        System.out.println(NativeFlowCore.nativeCloudCatalog(0, "stt", "whisper", "{}"));    // 5 picker rows
        System.out.println(NativeFlowCore.nativeCloudCatalog(0, "cleanup", "llama", "{}")); // 6
        System.out.println(NativeFlowCore.nativeCloudCatalog(0, "bogus", "", "{}"));        // 7 []

        long local = NativeFlowCore.nativeLoadModels("{\"asr_backend\":{\"mock\":{\"text\":\"local words btw\"}},\"asr_path\":\"\",\"threads\":1}");
        long r = NativeFlowCore.nativeOpenRouter(LEGACY, local);
        check(r != 0, "open_router: " + NativeFlowCore.nativeLastError());
        System.out.println(NativeFlowCore.nativeCloudNotice(r));                            // 8 notice
        NativeFlowCore.nativeAckCloudNotice(r);
        System.out.println(NativeFlowCore.nativeCloudNotice(r) == null ? "NOTICE null" : "NOTICE again"); // 9
        // The removed provider's key is ignored; the legacy OpenRouter id maps to "openrouter".
        check(NativeFlowCore.nativeSetCloudKey(r, "groq-1", key), "ignored key ok");
        System.out.println(NativeFlowCore.nativeCloudKeyInfo(r));                           // 10 no key
        check(NativeFlowCore.nativeSetCloudKey(r, "openrouter-1", key), "alias key");
        System.out.println(dictate(r));                                                     // 11 cloud off after migration -> local

        // Cleanup: consent missing -> blocked, dictation cleaned on-device/deterministic.
        check(NativeFlowCore.nativeSetCleanup(r, "{\"allow_cloud_cleanup\":true,\"cleanup_order\":[\"cloud\",\"local\"]}"), "set cleanup: " + NativeFlowCore.nativeLastError());
        System.out.println(NativeFlowCore.nativeCleanupStatus(r));                          // 12 blocked
        check(!NativeFlowCore.nativeSetCleanup(r, "{\"cleanup\":{\"timeout_ms\":100}}"), "timeout range");
        System.out.println("KIND " + NativeFlowCore.nativeLastErrorKind());                 // 13
        check(NativeFlowCore.nativeSetCloudModel(r, "cleanup", "meta-llama/llama-3.1-8b-instruct"), "model");
        check(!NativeFlowCore.nativeSetCloudModel(r, "cleanup", "Not A Model"), "bad model");
        System.out.println("KIND " + NativeFlowCore.nativeLastErrorKind());                 // 14
        System.out.println(NativeFlowCore.nativeCleanupStatus(r));                          // 15 model
        System.out.println(dictate(r));                                                     // 16 no cloud request
        System.out.println(NativeFlowCore.nativeRouteStatus(r));                            // 17 cleanup_circuit
        NativeFlowCore.nativeUnload(r);
    }
}

class NativeFlowCore {
    static native String nativeLastError();
    static native String nativeLastErrorKind();
    static native long nativeLoadModels(String cfg);
    static native void nativeUnload(long h);
    static native boolean nativeStartSession(long h, String cfg);
    static native boolean nativePushAudio(long h, short[] pcm, int len);
    static native String nativeCommit(long h);
    static native long nativeOpenRouter(String cfg, long local);
    static native String nativeRouteStatus(long h);
    static native boolean nativeSetCloudKey(long h, String providerId, String key);
    static native String nativeCloudCapabilities();
    static native String nativeCloudPresets();
    static native String nativeMigrateCloud(String devJson);
    static native boolean nativeSetCleanup(long h, String cleanupJson);
    static native String nativeCleanupStatus(long h);
    static native String nativeCloudNotice(long h);
    static native void nativeAckCloudNotice(long h);
    static native boolean nativeSetCloudModel(long h, String use, String model);
    static native void nativeSetCloudCacheDir(String dir);
    static native String nativeCloudCatalog(long h, String use, String query, String filtersJson);
    static native String nativeRefreshCloudCatalog(long h, boolean force);
    static native String nativeCloudKeyInfo(long h);
}
