package dev.flowclone.server;

// tests/jni_library.rs: the D41 library natives through a real JVM, as the
// Android shell's ServerLibraryNative (Kotlin) declares them.
// Args: <server config JSON> <model id> <file JSON> <dest> <part>.
// Prints: the list result, then the download result, then "PROGRESS <last>",
// then a cancelled download's result.
public final class ServerLibraryNative {
    public interface Listener {
        void onProgress(long bytesOnDisk);
        boolean isCancelled();
    }

    static native String nativeList(String server);
    static native String nativeDownload(String server, String modelId, String file, String dest, String part, Object listener);

    public static void main(String[] a) {
        System.loadLibrary("flow_ffi");
        System.out.println(nativeList(a[0]));
        final long[] last = {-1};
        Listener l = new Listener() {
            public void onProgress(long n) { last[0] = n; }
            public boolean isCancelled() { return false; }
        };
        System.out.println(nativeDownload(a[0], a[1], a[2], a[3], a[4], l));
        System.out.println("PROGRESS " + last[0]);
        Listener stop = new Listener() {
            public void onProgress(long n) { throw new RuntimeException("listener bug"); }
            public boolean isCancelled() { return true; }
        };
        System.out.println(nativeDownload(a[0], a[1], a[2], a[3] + ".2", a[4] + ".2", stop));
    }
}
