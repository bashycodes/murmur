/* flow-core C ABI. All configs/results are UTF-8 JSON (see docs/core-architecture.md).
 * int returns: 0 ok, -1 error (flow_last_error), 1 ignored (no active session).
 * Strings returned by flow_* are caller-owned: free with flow_string_free. */
#ifndef FLOW_CORE_H
#define FLOW_CORE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

typedef struct FlowHandle FlowHandle;

FlowHandle *flow_load_models(const char *model_config_json); /* NULL on error */
void flow_unload(FlowHandle *h);
int flow_start_session(FlowHandle *h, const char *session_config_json);
int flow_push_audio(FlowHandle *h, const int16_t *pcm16_16k_mono, size_t len); /* non-blocking */
int flow_set_context(FlowHandle *h, const char *field_context_json);
char *flow_partial(FlowHandle *h);                                   /* NULL if none */
char *flow_commit(FlowHandle *h);                                    /* CommitResult JSON; blocks */
void flow_cancel(FlowHandle *h);
char *flow_learn_edit(FlowHandle *h, const char *pasted, const char *edited); /* JSON array */
const char *flow_last_error(void);
/* "" | "bad_config" | "model_not_found" | "llm_lib_not_found" | "load" | "error"
 * | router: "no_engine_enabled" | "local_not_ready" | "server_not_configured" | "server_unavailable" */
const char *flow_last_error_kind(void);
void flow_string_free(char *s);

/* D17 hybrid router (docs/hybrid-architecture.md §6.2). A router handle works
 * with every session function above; flow_unload frees it (and its local engine). */
char *flow_capabilities(void);                                       /* Capabilities JSON */
/* Takes ownership of `local` (a flow_load_models handle, or NULL) whatever the outcome.
 * NULL on error: kind "no_engine_enabled" / "bad_config". */
FlowHandle *flow_open_router(const char *router_config_json, FlowHandle *local);
int flow_update_router(FlowHandle *h, const char *router_config_json); /* next session */
char *flow_probe_server(const char *server_config_json);             /* HealthReport JSON; blocks */
char *flow_probe_router(FlowHandle *h);                              /* probe saved server; blocks */
char *flow_route_status(FlowHandle *h);                              /* RouteStatus JSON */
void flow_reset_circuit(FlowHandle *h);

/* D18 cloud STT (docs/cloud-stt.md). Libraries built without feature `cloud`
 * keep these symbols: they report "not in this build". Keys only ever enter
 * here (never in router JSON); the library never logs or returns them. */
int flow_set_cloud_key(FlowHandle *h, const char *provider_id, const char *key_or_null); /* NULL/"" = forget; -1 for a core handle */
char *flow_probe_cloud(FlowHandle *h, const char *provider_json, const char *key_or_null); /* HealthReport JSON; blocks; h may be NULL (key required) */
char *flow_cloud_usage(FlowHandle *h);                               /* CloudUsage JSON */
int flow_set_cloud_usage(FlowHandle *h, const char *usage_json);     /* restore persisted counters */
void flow_reset_cloud_usage(FlowHandle *h);
char *flow_cloud_capabilities(void);                                 /* CloudCapabilities JSON */
char *flow_cloud_presets(void);                                      /* CloudProviderConfig[] JSON ([] without cloud) */

#ifdef __cplusplus
}
#endif
#endif
