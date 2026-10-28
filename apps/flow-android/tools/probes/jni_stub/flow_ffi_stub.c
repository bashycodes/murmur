/*
 * Host-JVM stand-in for libflow_ffi.so that implements every symbol in
 * JNI_CONTRACT.md with trivial behaviour. Built by tools/probes/jni_stub/run.sh
 * and loaded by NativeFlowCoreStubTest to prove the Kotlin `external` declarations
 * (static natives, names, descriptors) link against the documented C symbols.
 * Not shipped; the real library comes from crates/flow-ffi.
 */
#include <jni.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <stdint.h>

typedef struct { int active; long samples; int signal; } Core;
static __thread char last_error[256];

#define FN(name) Java_dev_flowclone_core_NativeFlowCore_##name

JNIEXPORT jstring JNICALL FN(nativeVersion)(JNIEnv *env, jclass c) { return (*env)->NewStringUTF(env, "flow-ffi-stub 0"); }

JNIEXPORT jstring JNICALL FN(nativeLastError)(JNIEnv *env, jclass c) {
    return last_error[0] ? (*env)->NewStringUTF(env, last_error) : NULL;
}

JNIEXPORT jlong JNICALL FN(nativeLoadModels)(JNIEnv *env, jclass c, jstring cfg) {
    const char *s = (*env)->GetStringUTFChars(env, cfg, NULL);
    int fail = strstr(s, "\"asr_path\":\"/fail\"") != NULL;
    (*env)->ReleaseStringUTFChars(env, cfg, s);
    if (fail) { snprintf(last_error, sizeof last_error, "model load failed: /fail"); return 0; }
    return (jlong)(intptr_t)calloc(1, sizeof(Core));
}

JNIEXPORT void JNICALL FN(nativeUnload)(JNIEnv *env, jclass c, jlong h) { free((void *)(intptr_t)h); }

JNIEXPORT jboolean JNICALL FN(nativeStartSession)(JNIEnv *env, jclass c, jlong h, jstring cfg) {
    Core *k = (Core *)(intptr_t)h; k->active = 1; k->samples = 0; k->signal = 0; return JNI_TRUE;
}

JNIEXPORT jboolean JNICALL FN(nativePushAudio)(JNIEnv *env, jclass c, jlong h, jshortArray pcm, jint len) {
    Core *k = (Core *)(intptr_t)h;
    if (!k->active) return JNI_FALSE;
    jshort buf[4096]; jint n = len > 4096 ? 4096 : len;
    (*env)->GetShortArrayRegion(env, pcm, 0, n, buf);
    for (jint i = 0; i < n; i++) if (buf[i] > 184 || buf[i] < -184) k->signal = 1;
    k->samples += len; return JNI_TRUE;
}

JNIEXPORT void JNICALL FN(nativeSetContext)(JNIEnv *env, jclass c, jlong h, jstring ctx) {}

JNIEXPORT jstring JNICALL FN(nativePartial)(JNIEnv *env, jclass c, jlong h) { return NULL; }

/* 2026-10-22: live route + hands-free endpointing. */
JNIEXPORT jstring JNICALL FN(nativeStartSessionInfo)(JNIEnv *env, jclass c, jlong h, jstring cfg) {
    Core *k = (Core *)(intptr_t)h; k->active = 1; k->samples = 0; k->signal = 0;
    return (*env)->NewStringUTF(env, "{\"queued\":false,\"route\":{\"engine\":\"local\"}}");
}

JNIEXPORT jstring JNICALL FN(nativePoll)(JNIEnv *env, jclass c, jlong h) {
    Core *k = (Core *)(intptr_t)h;
    return (*env)->NewStringUTF(env, k && k->active
        ? "{\"partial\":null,\"endpoint_reached\":false,\"speech_ms\":0,\"trailing_silence_ms\":0,\"route\":{\"engine\":\"local\"}}"
        : "{\"partial\":null,\"endpoint_reached\":false,\"speech_ms\":0,\"trailing_silence_ms\":0}");
}

JNIEXPORT jstring JNICALL FN(nativeCommit)(JNIEnv *env, jclass c, jlong h) {
    Core *k = (Core *)(intptr_t)h; char out[1024];
    k->active = 0;
    snprintf(out, sizeof out,
        "{\"status\":\"%s\",\"asr_text\":\"stub\",\"final_text\":\"%s\",\"language\":\"en\","
        "\"timings\":{\"audio_ms\":%ld,\"asr_ms\":0,\"llm_ms\":0,\"postprocess_ms\":0,\"total_ms\":0,\"dropped_samples\":0},"
        "\"press_enter\":false,\"fallback_reason\":null,\"truncated\":false,\"error\":null}",
        k->signal ? "RAW_TRANSCRIPT" : "EMPTY", k->signal ? "Stub text." : "", k->samples / 16);
    return (*env)->NewStringUTF(env, out);
}

JNIEXPORT void JNICALL FN(nativeCancel)(JNIEnv *env, jclass c, jlong h) { ((Core *)(intptr_t)h)->active = 0; }

JNIEXPORT jstring JNICALL FN(nativeLearnEdit)(JNIEnv *env, jclass c, jlong h, jstring a, jstring b) {
    return (*env)->NewStringUTF(env, "[{\"original\":\"wisper\",\"corrected\":\"Wispr\"}]");
}

/* 2026-10-28: Command Mode. No AI in the stub, unless the instruction is "upper". */
JNIEXPORT jstring JNICALL FN(nativeCommand)(JNIEnv *env, jclass c, jlong h, jstring instruction, jstring selected) {
    const char *i = (*env)->GetStringUTFChars(env, instruction, NULL);
    int upper = strcmp(i, "upper") == 0;
    (*env)->ReleaseStringUTFChars(env, instruction, i);
    return (*env)->NewStringUTF(env, upper && selected
        ? "{\"ok\":true,\"text\":\"STUB\"}"
        : "{\"ok\":false,\"kind\":\"no_llm\",\"error\":\"no AI in the stub\"}");
}
