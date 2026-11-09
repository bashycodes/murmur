package dev.murmur.core;

// Single-file source launch: `java -Djava.library.path=DIR JniMeeting.java`.
// JNI_CONTRACT.md "Meeting transcription": a long recording through nativeMeeting* on a mock core.
public class JniMeeting {
    static void check(boolean ok, String what) { if (!ok) throw new RuntimeException(what); }

    static final String CORE = "{\"asr_backend\":{\"mock\":{\"text\":\"hello btw\"}},\"asr_path\":\"\",\"threads\":1}";
    static final String SESSION = "{\"languages\":[\"en\"],\"platform\":\"android\",\"raw_mode\":true,"
        + "\"dictionary\":[{\"word\":\"btw\",\"replacement\":\"by the way\",\"isSnippet\":false}]}";

    /** Voiced "speech": a 150 Hz tone with a 4 Hz syllable envelope; zeros when !voiced. */
    static short[] block(int start, int n, boolean voiced) {
        short[] b = new short[n];
        if (!voiced) return b;
        for (int i = 0; i < n; i++) {
            double t = (start + i) / 16000.0;
            double env = 0.55 + 0.45 * Math.sin(2 * Math.PI * 4 * t);
            b[i] = (short) (8000 * env * Math.sin(2 * Math.PI * 150 * t));
        }
        return b;
    }

    public static void main(String[] args) {
        System.loadLibrary("murmur_ffi");
        // 12 s speech, 2 s silence, 12 s speech: two chunks.
        long m = NativeMurmurCore.nativeMeetingOpen(NativeMurmurCore.nativeLoadModels(CORE), SESSION, null);
        check(m != 0, "open: " + NativeMurmurCore.nativeLastError());
        NativeMurmurCore.nativeMeetingSetTotal(m, 26 * 16000);
        StringBuilder segs = new StringBuilder();
        for (int s = 0; s < 26; s++) {
            boolean voiced = s < 12 || s >= 14;
            String r = NativeMurmurCore.nativeMeetingPush(m, block(s * 16000, 16000, voiced), 16000);
            check(r != null, "push: " + NativeMurmurCore.nativeLastError());
            if (!r.equals("[]")) segs.append(r).append('\n');
        }
        String f = NativeMurmurCore.nativeMeetingFinish(m);
        check(f != null, "finish: " + NativeMurmurCore.nativeLastError());
        segs.append(f);
        System.out.println(segs.toString().replace("\n", " "));
        System.out.println(NativeMurmurCore.nativeMeetingProgress(m));
        NativeMurmurCore.nativeMeetingClose(m);
        NativeMurmurCore.nativeMeetingClose(0);
        // Cancel from "another thread" (here: between pushes): the next push fails with kind cancelled.
        long c = NativeMurmurCore.nativeMeetingOpen(NativeMurmurCore.nativeLoadModels(CORE), SESSION, "{}");
        check(NativeMurmurCore.nativeMeetingPush(c, block(0, 16000, true), 16000) != null, "push before cancel");
        NativeMurmurCore.nativeMeetingCancel(c);
        check(NativeMurmurCore.nativeMeetingPush(c, block(0, 16000, true), 16000) == null, "push after cancel");
        System.out.println("KIND " + NativeMurmurCore.nativeLastErrorKind());
        NativeMurmurCore.nativeMeetingClose(c);
        // No engine / bad config: 0 + bad_config; the handle is taken over either way.
        check(NativeMurmurCore.nativeMeetingOpen(0, SESSION, null) == 0, "no engine");
        check(NativeMurmurCore.nativeMeetingOpen(NativeMurmurCore.nativeLoadModels(CORE), "[", null) == 0, "bad session");
        System.out.println("KIND " + NativeMurmurCore.nativeLastErrorKind());
        System.out.println(NativeMurmurCore.nativeMeetingProgress(0));
    }
}

class NativeMurmurCore {
    static native String nativeLastError();
    static native String nativeLastErrorKind();
    static native long nativeLoadModels(String cfg);
    static native long nativeMeetingOpen(long engine, String session, String params);
    static native void nativeMeetingSetTotal(long m, long samples);
    static native String nativeMeetingPush(long m, short[] pcm, int len);
    static native String nativeMeetingFinish(long m);
    static native String nativeMeetingProgress(long m);
    static native void nativeMeetingCancel(long m);
    static native void nativeMeetingClose(long m);
}
