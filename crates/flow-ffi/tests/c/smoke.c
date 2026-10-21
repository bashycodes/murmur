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
    if (!ok) return 4;

    /* D17 router, client only: same result, plus route.engine = local. */
    const char *client_only = "{\"allow_server\":false,\"allow_client\":true,\"prefer\":\"client\"}";
    FlowHandle *core = flow_load_models("{\"asr_backend\":{\"mock\":{\"text\":\"hello from c\"}}}");
    FlowHandle *rt = flow_open_router(client_only, core); /* owns core now */
    if (!rt) { fprintf(stderr, "open_router: %s\n", flow_last_error()); return 5; }
    if (flow_start_session(rt, "{}") != 0) { fprintf(stderr, "router start: %s (%s)\n", flow_last_error(), flow_last_error_kind()); return 6; }
    for (int c = 0; c < 25; c++) {
        for (int i = 0; i < 640; i++) buf[i] = (int16_t)(sin((c * 640 + i) * 0.05) * 8000);
        flow_push_audio(rt, buf, 640);
    }
    r = flow_commit(rt);
    printf("%s\n", r);
    ok = strstr(r, "\"final_text\":\"hello from c \"") && strstr(r, "\"engine\":\"local\"");
    flow_string_free(r);
    char *st = flow_route_status(rt);
    ok = ok && strstr(st, "\"circuit\"");
    flow_string_free(st);
    flow_reset_circuit(rt);
    if (flow_update_router(rt, "{\"allow_server\":false,\"allow_client\":true,\"server\":{\"port\":0}}") != -1) return 7;
    flow_unload(rt);
    if (!ok) return 8;

    /* Both routes off: refused up front. */
    if (flow_open_router("{\"allow_server\":false,\"allow_client\":false}", NULL)) return 9;
    if (strcmp(flow_last_error_kind(), "no_engine_enabled") != 0) return 10;
    char *caps = flow_capabilities();
    printf("%s\n", caps);
    ok = strstr(caps, "\"proto\":1") != NULL;
    flow_string_free(caps);
    return ok ? 0 : 11;
}
