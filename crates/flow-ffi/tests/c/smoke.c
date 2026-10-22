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
    if (!ok) return 11;

    /* D18 cloud symbols (this staticlib is built without `cloud`). */
    char *cc = flow_cloud_capabilities();
    printf("%s\n", cc);
    ok = strstr(cc, "\"cloud\":false") != NULL;
    flow_string_free(cc);
    char *pr = flow_cloud_presets();
    ok = ok && strcmp(pr, "[]") == 0;
    flow_string_free(pr);
    char *probe = flow_probe_cloud(NULL, "{\"id\":\"groq-1\",\"base_url\":\"https://api.groq.com/openai/v1\",\"model\":\"m\"}", "sk-x");
    ok = ok && strstr(probe, "cloud_not_configured") && !strstr(probe, "sk-x");
    flow_string_free(probe);
    FlowHandle *core2 = flow_load_models("{\"asr_backend\":{\"mock\":{\"text\":\"x\"}}}");
    if (flow_set_cloud_key(core2, "groq-1", "sk-x") != -1) return 12; /* core handle */
    FlowHandle *rt2 = flow_open_router(client_only, core2);
    if (flow_set_cloud_key(rt2, "groq-1", "sk-x") != 0) return 13;
    if (flow_set_cloud_key(rt2, "groq-1", NULL) != 0) return 14;
    if (flow_set_cloud_usage(rt2, "{\"month\":\"1999-01\",\"providers\":[]}") != 0) return 15;
    flow_reset_cloud_usage(rt2);
    char *u = flow_cloud_usage(rt2);
    ok = ok && strstr(u, "\"providers\":[]");
    flow_string_free(u);
    flow_unload(rt2);
    return ok ? 0 : 16;
}
