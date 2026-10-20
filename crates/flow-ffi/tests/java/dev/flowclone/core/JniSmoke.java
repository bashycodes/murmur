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
}
