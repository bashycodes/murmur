package com.flowclone.core;

// Single-file source launch: `java -Djava.library.path=DIR JniSmoke.java`.
public class JniSmoke {
    public static void main(String[] args) {
        System.loadLibrary("flow_ffi");
        long h = NativeFlowCore.loadModels("{\"asr_backend\":{\"mock\":{\"text\":\"hello btw\"}}}");
        if (h == 0) throw new RuntimeException("load failed: " + NativeFlowCore.lastError());
        boolean ok = NativeFlowCore.startSession(h,
            "{\"platform\":\"android\",\"dictionary\":[{\"word\":\"btw\",\"replacement\":\"by the way\",\"isSnippet\":false}]}");
        if (!ok) throw new RuntimeException("start failed");
        short[] buf = new short[640];
        for (int c = 0; c < 25; c++) {
            for (int i = 0; i < buf.length; i++) buf[i] = (short) (Math.sin((c * 640 + i) * 0.05) * 8000);
            if (!NativeFlowCore.pushAudio(h, buf, buf.length)) throw new RuntimeException("push failed");
        }
        NativeFlowCore.setContext(h, "{\"app_id\":\"com.whatsapp\"}");
        System.out.println(NativeFlowCore.commit(h));
        System.out.println(NativeFlowCore.learnEdit(h, "ask whisper now", "ask Wispr now"));
        if (NativeFlowCore.startSession(h, "[")) throw new RuntimeException("bad json accepted");
        System.out.println("ERR " + NativeFlowCore.lastError());
        NativeFlowCore.unload(h);
    }
}

class NativeFlowCore {
    static native long loadModels(String cfg);
    static native void unload(long h);
    static native boolean startSession(long h, String cfg);
    static native boolean pushAudio(long h, short[] pcm, int len);
    static native boolean setContext(long h, String ctx);
    static native String partial(long h);
    static native String commit(long h);
    static native void cancel(long h);
    static native String learnEdit(long h, String pasted, String edited);
    static native String lastError();
}
