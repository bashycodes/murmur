package dev.flowclone.core;

// Single-file source launch (crates/flow-ffi/tests/jni_cloud.rs):
//   java -Djava.library.path=DIR JniCloud.java <okBaseUrl> <authFailBaseUrl> <key>
// The shell's D18 cloud flow through the real JNI exports + native router, against
// flow-cloud's in-process mock provider (OpenAI-compatible wire format, Groq preset).
// Every line printed is checked by jni_cloud.rs (which also asserts the key never appears).
public class JniCloud {
    static void check(boolean ok, String what) { if (!ok) throw new RuntimeException(what); }

    static String provider(String base) {
        return "{\"id\":\"groq-1\",\"preset\":\"groq\",\"adapter\":\"openai_compat\",\"dialect\":\"\",\"label\":\"Groq\","
            + "\"base_url\":\"" + base + "\",\"model\":\"whisper-large-v3-turbo\",\"language\":\"\","
            + "\"send_dictionary\":true,\"send_screen_words\":false,\"zero_retention\":false,\"format_override\":\"\","
            + "\"timeout_base_ms\":6000,\"price_per_hour_usd\":null,\"consent_host\":\"127.0.0.1\",\"consent_at_ms\":1}";
    }

    static String routerConfig(String base) {
        return "{\"allow_server\":false,\"allow_client\":true,\"prefer\":\"server\",\"keep_local_warm\":true,"
            + "\"allow_cloud\":true,\"order\":[\"cloud\",\"client\"],"
            + "\"cloud\":{\"active\":\"groq-1\",\"providers\":[" + provider(base) + "],\"monthly_budget_usd\":null}}";
    }

    static final String SESSION = "{\"languages\":[\"en\"],\"platform\":\"android\","
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
        String ok = args[0], bad = args[1], key = args[2];
        System.loadLibrary("flow_ffi");
        System.out.println(NativeFlowCore.nativeCloudCapabilities());                   // 1 caps
        System.out.println(NativeFlowCore.nativeCloudPresets());                        // 2 presets
        // Test provider before any router exists: h = 0, key passed; then without a key.
        System.out.println(NativeFlowCore.nativeProbeCloud(0, provider(ok), key));     // 3 probe ok
        System.out.println(NativeFlowCore.nativeProbeCloud(0, provider(ok), null));    // 4 no key

        long local = NativeFlowCore.nativeLoadModels("{\"asr_backend\":{\"mock\":{\"text\":\"local words btw\"}},\"asr_path\":\"\",\"threads\":1}");
        check(!NativeFlowCore.nativeSetCloudKey(local, "groq-1", key), "core handle must refuse a key");
        System.out.println("KIND " + NativeFlowCore.nativeLastErrorKind());             // 5
        long r = NativeFlowCore.nativeOpenRouter(routerConfig(ok), local);
        check(r != 0, "open_router: " + NativeFlowCore.nativeLastError());
        check(NativeFlowCore.nativeSetCloudUsage(r, "{\"month\":\"1999-01\",\"providers\":[{\"provider_id\":\"groq-1\",\"requests\":9}]}"), "set usage");

        System.out.println(dictate(r));                                                 // 6 no key -> local
        check(NativeFlowCore.nativeSetCloudKey(r, "groq-1", key), "set key: " + NativeFlowCore.nativeLastError());
        System.out.println(dictate(r));                                                 // 7 cloud
        System.out.println(NativeFlowCore.nativeCloudUsage(r));                         // 8 usage
        System.out.println(NativeFlowCore.nativeRouteStatus(r));                        // 9 status
        // Saved key + unsaved form (key null = stored key).
        System.out.println(NativeFlowCore.nativeProbeCloud(r, provider(ok), null));    // 10 probe stored key

        // Provider now rejects the key: auth_failed, the same audio replays into the device engine.
        check(NativeFlowCore.nativeUpdateRouter(r, routerConfig(bad)), "update: " + NativeFlowCore.nativeLastError());
        System.out.println(dictate(r));                                                 // 11 fallback
        System.out.println(NativeFlowCore.nativeRouteStatus(r));                        // 12 cloud circuit open
        System.out.println(NativeFlowCore.nativeProbeCloud(r, provider(bad), null));   // 13 probe auth_failed

        // Forget the key: cloud skipped with cloud_not_configured.
        check(NativeFlowCore.nativeUpdateRouter(r, routerConfig(ok)), "update back");
        check(NativeFlowCore.nativeSetCloudKey(r, "groq-1", null), "forget key");
        System.out.println(dictate(r));                                                 // 14 no key again
        NativeFlowCore.nativeResetCloudUsage(r);
        System.out.println(NativeFlowCore.nativeCloudUsage(r));                         // 15 reset
        check(!NativeFlowCore.nativeSetCloudUsage(r, "not json"), "bad usage json must fail");
        System.out.println("KIND " + NativeFlowCore.nativeLastErrorKind());             // 16
        NativeFlowCore.nativeUnload(r);

        // Cloud-only config is an engine in a cloud build.
        String cloudOnly = routerConfig(ok).replace("\"allow_client\":true", "\"allow_client\":false").replace("[\"cloud\",\"client\"]", "[\"cloud\"]");
        long c = NativeFlowCore.nativeOpenRouter(cloudOnly, 0);
        check(c != 0, "cloud-only open: " + NativeFlowCore.nativeLastError());
        NativeFlowCore.nativeSetCloudKey(c, "groq-1", key);
        System.out.println(dictate(c));                                                 // 17 cloud only
        NativeFlowCore.nativeUnload(c);
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
    static native boolean nativeUpdateRouter(long h, String cfg);
    static native String nativeRouteStatus(long h);
    static native boolean nativeSetCloudKey(long h, String providerId, String key);
    static native String nativeProbeCloud(long h, String providerJson, String key);
    static native String nativeCloudUsage(long h);
    static native boolean nativeSetCloudUsage(long h, String usageJson);
    static native void nativeResetCloudUsage(long h);
    static native String nativeCloudCapabilities();
    static native String nativeCloudPresets();
}
