#include <stdio.h>
#include <string.h>
#include <math.h>
#include "flow_core.h"

int main(void) {
    FlowHandle *h = flow_load_models("{\"asr_backend\":{\"mock\":{\"text\":\"hello from c\"}}}");
    if (!h) { fprintf(stderr, "load: %s\n", flow_last_error()); return 1; }
    if (flow_start_session(h, "{}") != 0) return 2;
    int16_t buf[640];
    for (int c = 0; c < 25; c++) {
        for (int i = 0; i < 640; i++) buf[i] = (int16_t)(sin((c * 640 + i) * 0.05) * 8000);
        if (flow_push_audio(h, buf, 640) != 0) return 3;
    }
    char *r = flow_commit(h);
    printf("%s\n", r);
    int ok = strstr(r, "\"final_text\":\"hello from c \"") != NULL;
    flow_string_free(r);
    flow_unload(h);
    return ok ? 0 : 4;
}
