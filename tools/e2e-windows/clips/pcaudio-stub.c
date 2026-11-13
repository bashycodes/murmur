/* Stand-in for libpcaudio.so.0 so a user-space (no root) espeak-ng can
 * write WAV files (-w) without PulseAudio/ALSA. Every call fails: the
 * harness never plays sound through espeak-ng. */
#include <stddef.h>
#include <stdint.h>

struct audio_object;

struct audio_object *create_audio_device_object(const char *d, const char *a, const char *n) {
    (void)d; (void)a; (void)n;
    return NULL;
}
int audio_object_open(struct audio_object *o, int f, uint32_t r, uint8_t c) {
    (void)o; (void)f; (void)r; (void)c;
    return -1;
}
void audio_object_close(struct audio_object *o) { (void)o; }
void audio_object_destroy(struct audio_object *o) { (void)o; }
int audio_object_write(struct audio_object *o, const void *d, size_t b) {
    (void)o; (void)d; (void)b;
    return -1;
}
int audio_object_drain(struct audio_object *o) { (void)o; return 0; }
int audio_object_flush(struct audio_object *o) { (void)o; return 0; }
const char *audio_object_strerror(struct audio_object *o, int e) {
    (void)o; (void)e;
    return "no audio (stub)";
}
