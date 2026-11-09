#include <stdio.h>
#include <string.h>
#include <math.h>
#include "murmur_core.h"

int main(void) {
    MurmurHandle *h = murmur_load_models("{\"asr_backend\":{\"mock\":{\"text\":\"hello from c\"}}}");
    if (!h) { fprintf(stderr, "load: %s\n", murmur_last_error()); return 1; }
    if (murmur_start_session(h, "{}") != 0) return 2;
    int16_t buf[640];
    for (int c = 0; c < 25; c++) {
        for (int i = 0; i < 640; i++) buf[i] = (int16_t)(sin((c * 640 + i) * 0.05) * 8000);
        if (murmur_push_audio(h, buf, 640) != 0) return 3;
    }
    char *r = murmur_commit(h);
    printf("%s\n", r);
    int ok = strstr(r, "\"final_text\":\"hello from c \"") != NULL;
    murmur_string_free(r);
    murmur_unload(h);
    if (!ok) return 4;

    /* D17 router, client only: same result, plus route.engine = local. */
    const char *client_only = "{\"allow_server\":false,\"allow_client\":true,\"prefer\":\"client\"}";
    MurmurHandle *core = murmur_load_models("{\"asr_backend\":{\"mock\":{\"text\":\"hello from c\"}}}");
    MurmurHandle *rt = murmur_open_router(client_only, core); /* owns core now */
    if (!rt) { fprintf(stderr, "open_router: %s\n", murmur_last_error()); return 5; }
    char *si = murmur_start_session_info(rt, "{\"endpoint_silence_ms\":2000}");
    if (!si) { fprintf(stderr, "router start: %s (%s)\n", murmur_last_error(), murmur_last_error_kind()); return 6; }
    printf("%s\n", si);
    ok = strstr(si, "\"route\":{\"engine\":\"local\"}") != NULL;
    murmur_string_free(si);
    if (!ok) return 17;
    for (int c = 0; c < 25; c++) {
        for (int i = 0; i < 640; i++) buf[i] = (int16_t)(sin((c * 640 + i) * 0.05) * 8000);
        murmur_push_audio(rt, buf, 640);
    }
    char *pl = murmur_poll(rt);
    printf("%s\n", pl);
    ok = strstr(pl, "\"route\":{\"engine\":\"local\"}") && strstr(pl, "\"endpoint_reached\":");
    murmur_string_free(pl);
    if (!ok) return 18;
    if (murmur_poll(NULL) != NULL) return 19;
    if (murmur_start_session_info(rt, "[") != NULL || strcmp(murmur_last_error_kind(), "bad_config") != 0) return 20;
    if (murmur_start_session(rt, "{}") != 0) return 21;
    for (int c = 0; c < 25; c++) {
        for (int i = 0; i < 640; i++) buf[i] = (int16_t)(sin((c * 640 + i) * 0.05) * 8000);
        murmur_push_audio(rt, buf, 640);
    }
    r = murmur_commit(rt);
    printf("%s\n", r);
    ok = strstr(r, "\"final_text\":\"hello from c \"") && strstr(r, "\"engine\":\"local\"");
    murmur_string_free(r);
    char *st = murmur_route_status(rt);
    ok = ok && strstr(st, "\"circuit\"");
    murmur_string_free(st);
    murmur_reset_circuit(rt);
    if (murmur_update_router(rt, "{\"allow_server\":false,\"allow_client\":true,\"server\":{\"port\":0}}") != -1) return 7;
    murmur_unload(rt);
    if (!ok) return 8;

    /* Both routes off: refused up front. */
    if (murmur_open_router("{\"allow_server\":false,\"allow_client\":false}", NULL)) return 9;
    if (strcmp(murmur_last_error_kind(), "no_engine_enabled") != 0) return 10;
    char *caps = murmur_capabilities();
    printf("%s\n", caps);
    ok = strstr(caps, "\"proto\":1") != NULL;
    murmur_string_free(caps);
    if (!ok) return 11;

    /* D18 cloud symbols (this staticlib is built without `cloud`). */
    char *cc = murmur_cloud_capabilities();
    printf("%s\n", cc);
    ok = strstr(cc, "\"cloud\":false") != NULL;
    murmur_string_free(cc);
    char *pr = murmur_cloud_presets();
    ok = ok && strcmp(pr, "[]") == 0;
    murmur_string_free(pr);
    char *probe = murmur_probe_cloud(NULL, "{\"id\":\"groq-1\",\"base_url\":\"https://api.groq.com/openai/v1\",\"model\":\"m\"}", "sk-x");
    ok = ok && strstr(probe, "cloud_not_configured") && !strstr(probe, "sk-x");
    murmur_string_free(probe);
    MurmurHandle *core2 = murmur_load_models("{\"asr_backend\":{\"mock\":{\"text\":\"x\"}}}");
    if (murmur_set_cloud_key(core2, "groq-1", "sk-x") != -1) return 12; /* core handle */
    MurmurHandle *rt2 = murmur_open_router(client_only, core2);
    if (murmur_set_cloud_key(rt2, "groq-1", "sk-x") != 0) return 13;
    if (murmur_set_cloud_key(rt2, "groq-1", NULL) != 0) return 14;
    const char *gp = "{\"id\":\"groq-1\",\"adapter\":\"openai_compat\",\"base_url\":\"https://api.groq.com/openai/v1\",\"model\":\"m\"}";
    if (murmur_set_cloud_key_for(rt2, gp, "sk-x") != 0) return 22;
    if (murmur_set_cloud_key_for(rt2, "[", "sk-x") != -1 || strcmp(murmur_last_error_kind(), "bad_config") != 0) return 23;
    if (murmur_set_cloud_key_for(rt2, gp, NULL) != 0) return 24;
    if (murmur_set_cloud_usage(rt2, "{\"month\":\"1999-01\",\"providers\":[]}") != 0) return 15;
    murmur_reset_cloud_usage(rt2);
    char *u = murmur_cloud_usage(rt2);
    ok = ok && strstr(u, "\"providers\":[]");
    murmur_string_free(u);
    /* D28: migration, cleanup, model selection, notice. */
    char *mig = murmur_migrate_cloud("{\"allow_cloud\":true,\"order\":[\"cloud\"],\"cloud\":{\"active\":\"groq-1\",\"providers\":[{\"id\":\"groq-1\",\"preset\":\"groq\",\"adapter\":\"openai_compat\",\"base_url\":\"https://api.groq.com/openai/v1\",\"model\":\"m\"}]}}");
    ok = ok && mig && strstr(mig, "\"migrated\":true") && strstr(mig, "\"delete_keys\":[\"groq-1\"]");
    murmur_string_free(mig);
    if (murmur_migrate_cloud("[") != NULL || strcmp(murmur_last_error_kind(), "bad_config") != 0) return 25;
    if (murmur_set_cloud_model(rt2, "cleanup", "meta-llama/llama-3.1-8b-instruct") != 0) return 26;
    if (murmur_set_cloud_model(rt2, "cleanup", "Bad Id") != -1) return 27;
    if (murmur_set_cleanup(rt2, "{\"allow_cloud_cleanup\":true,\"cleanup_order\":[\"cloud\",\"local\"]}") != 0) return 28;
    char *cs = murmur_cleanup_status(rt2);
    ok = ok && strstr(cs, "\"supported\":false") && strstr(cs, "llama-3.1-8b");
    murmur_string_free(cs);
    if (murmur_cloud_notice(rt2) != NULL) return 29;
    murmur_ack_cloud_notice(rt2);
    murmur_unload(rt2);
    return ok ? 0 : 16;
}
