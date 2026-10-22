package dev.flowclone.core;

// Single-file source launch: `java -Djava.library.path=DIR JniSmoke.java`.
// Mirrors apps/flow-android/JNI_CONTRACT.md: static natives on dev.flowclone.core.NativeFlowCore.
public class JniSmoke {
    static void check(boolean ok, String what) { if (!ok) throw new RuntimeException(what); }

    public static void main(String[] args) {
        System.loadLibrary("flow_ffi");
        System.out.println(NativeFlowCore.nativeVersion());
        check(NativeFlowCore.nativeLastError() == null, "fresh thread has no error");
        long bad = NativeFlowCore.nativeLoadModels("{\"asr_backend\":\"whisper\",\"asr_path\":\"/nonexistent.bin\",\"threads\":1}");
        check(bad == 0, "missing model must fail");
        check(NativeFlowCore.nativeLastError() != null, "load failure sets last error");
        // Payloads shaped exactly like CoreJson.kt sends them.
        long h = NativeFlowCore.nativeLoadModels(
            "{\"asr_backend\":{\"mock\":{\"text\":\"hello btw\"}},\"asr_path\":\"\",\"llm_path\":null,\"threads\":2}");
        check(h != 0, "load failed: " + NativeFlowCore.nativeLastError());
        boolean ok = NativeFlowCore.nativeStartSession(h,
            "{\"languages\":[\"en\"],\"dictionary\":[{\"word\":\"btw\",\"replacement\":\"by the way\",\"isSnippet\":false}],"
            + "\"styles\":{\"personal\":\"formal\",\"work\":\"formal\",\"email\":\"formal\",\"other\":\"formal\"},"
            + "\"cleanup_level\":\"none\",\"raw_mode\":false,\"max_samples\":4800000,\"platform\":\"android\","
            + "\"partial_interval_ms\":null,\"llm_timeout_ms\":1500}");
        check(ok, "start failed: " + NativeFlowCore.nativeLastError());
        short[] buf = new short[640];
        for (int c = 0; c < 25; c++) {
            for (int i = 0; i < buf.length; i++) buf[i] = (short) (Math.sin((c * 640 + i) * 0.05) * 8000);
            check(NativeFlowCore.nativePushAudio(h, buf, buf.length), "push failed");
        }
        // Over-long length is clamped, not an ArrayIndexOutOfBoundsException.
        check(NativeFlowCore.nativePushAudio(h, new short[0], 999), "clamped push");
        NativeFlowCore.nativeSetContext(h,
            "{\"app_id\":\"com.whatsapp\",\"url_host\":\"\",\"app_category\":\"personal_chat\",\"before\":\"\",\"selected\":\"\",\"after\":\"\",\"screen_words\":[]}");
        NativeFlowCore.nativeSetContext(h, "not json"); // ignored
        System.out.println(NativeFlowCore.nativePartial(h));
        System.out.println(NativeFlowCore.nativeCommit(h));
        System.out.println(NativeFlowCore.nativeLearnEdit(h, "ask whisper now", "ask Wispr now"));
        check(!NativeFlowCore.nativeStartSession(h, "["), "bad json accepted");
        System.out.println("ERR " + NativeFlowCore.nativeLastError());
        NativeFlowCore.nativeCancel(h);
        NativeFlowCore.nativeCancel(h);
        check(!NativeFlowCore.nativePushAudio(h, buf, buf.length), "push after cancel must be false");
        System.out.println(NativeFlowCore.nativeCommit(0));
        System.out.println(NativeFlowCore.nativeLearnEdit(0, "a", "b"));
        NativeFlowCore.nativeUnload(h);
        NativeFlowCore.nativeUnload(0);

        // ---- D17 router: the shell's real flow (load local, hand it to the router, session on the router) ----
        // args[0] (optional): "host:port" of a running flow-server (mock ASR) to route to.
        String server = args.length > 0 ? args[0] : null;
        long local = NativeFlowCore.nativeLoadModels("{\"asr_backend\":{\"mock\":{\"text\":\"hello btw\"}},\"asr_path\":\"\",\"threads\":1}");
        String host = server == null ? "" : server.substring(0, server.lastIndexOf(':'));
        String port = server == null ? "7313" : server.substring(server.lastIndexOf(':') + 1);
        String cfg = "{\"allow_server\":" + (server != null) + ",\"allow_client\":true,\"prefer\":\"server\",\"keep_local_warm\":true,"
            + "\"server\":{\"host\":\"" + host + "\",\"port\":" + port + ",\"tls\":false,\"tls_pin_sha256\":null,"
            + "\"token\":\"" + (args.length > 1 ? args[1] : "") + "\",\"connect_timeout_ms\":800,\"commit_timeout_base_ms\":3000,\"health_ttl_s\":30}}";
        long r = NativeFlowCore.nativeOpenRouter(cfg, local);
        check(r != 0, "open_router: " + NativeFlowCore.nativeLastError());
        String rsess = "{\"languages\":[\"en\"],\"platform\":\"android\",\"dictionary\":[{\"word\":\"btw\",\"replacement\":\"by the way\",\"isSnippet\":false}]";
        // Hands-free session: endpoint_silence_ms only in hands-free mode.
        String info = NativeFlowCore.nativeStartSessionInfo(r, rsess + ",\"endpoint_silence_ms\":2000}");
        check(info != null, "router start: " + NativeFlowCore.nativeLastError());
        System.out.println(info);
        for (int c = 0; c < 25; c++) {
            for (int i = 0; i < buf.length; i++) buf[i] = (short) (Math.sin((c * 640 + i) * 0.05) * 8000);
            NativeFlowCore.nativePushAudio(r, buf, buf.length);
        }
        System.out.println(NativeFlowCore.nativePoll(r));
        System.out.println(NativeFlowCore.nativePoll(0));
        String badInfo = NativeFlowCore.nativeStartSessionInfo(r, "[");
        System.out.println("INFO " + badInfo + " KIND " + NativeFlowCore.nativeLastErrorKind());
        // Push-to-talk: plain start (no endpointing), as before.
        check(NativeFlowCore.nativeStartSession(r, rsess + "}"), "router start: " + NativeFlowCore.nativeLastError());
        for (int c = 0; c < 25; c++) {
            for (int i = 0; i < buf.length; i++) buf[i] = (short) (Math.sin((c * 640 + i) * 0.05) * 8000);
            NativeFlowCore.nativePushAudio(r, buf, buf.length);
        }
        System.out.println(NativeFlowCore.nativeCommit(r));
        System.out.println(NativeFlowCore.nativeRouteStatus(r));
        System.out.println(NativeFlowCore.nativeLearnEdit(r, "ask whisper now", "ask Wispr now"));
        check(NativeFlowCore.nativeUpdateRouter(r, cfg.replace("\"prefer\":\"server\"", "\"prefer\":\"client\"")), "update: " + NativeFlowCore.nativeLastError());
        check(!NativeFlowCore.nativeUpdateRouter(r, "{\"server\":{\"port\":0}}"), "port 0 must be bad_config");
        System.out.println("KIND " + NativeFlowCore.nativeLastErrorKind());
        NativeFlowCore.nativeResetCircuit(r);
        NativeFlowCore.nativeUnload(r);
        check(NativeFlowCore.nativeOpenRouter("{\"allow_server\":false,\"allow_client\":false}", 0) == 0, "both off must refuse");
        System.out.println("KIND " + NativeFlowCore.nativeLastErrorKind());
        System.out.println(NativeFlowCore.nativeCapabilities());
        // Nothing listens on port 9 of loopback: a fast connect_failed (or "no server support" without `remote`).
        System.out.println(NativeFlowCore.nativeProbeServer("{\"host\":\"127.0.0.1\",\"port\":9,\"connect_timeout_ms\":300}"));
        if (server != null) {
            String probe = NativeFlowCore.nativeProbeServer(cfg.substring(cfg.indexOf("\"server\":") + 9, cfg.length() - 1));
            System.out.println(probe);
        }
    }
}

class NativeFlowCore {
    static native String nativeVersion();
    static native String nativeLastError();
    static native long nativeLoadModels(String cfg);
    static native void nativeUnload(long h);
    static native boolean nativeStartSession(long h, String cfg);
    static native boolean nativePushAudio(long h, short[] pcm, int len);
    static native void nativeSetContext(long h, String ctx);
    static native String nativePartial(long h);
    static native String nativeCommit(long h);
    static native void nativeCancel(long h);
    static native String nativeLearnEdit(long h, String pasted, String edited);
    static native String nativeLastErrorKind();
    static native String nativeCapabilities();
    static native long nativeOpenRouter(String cfg, long local);
    static native boolean nativeUpdateRouter(long h, String cfg);
    static native String nativeProbeServer(String cfg);
    static native String nativeProbeRouter(long h);
    static native String nativeRouteStatus(long h);
    static native void nativeResetCircuit(long h);
    static native String nativeStartSessionInfo(long h, String cfg);
    static native String nativePoll(long h);
}
