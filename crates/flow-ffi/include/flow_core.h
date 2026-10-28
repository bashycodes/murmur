/* flow-core C ABI. All configs/results are UTF-8 JSON (see docs/core-architecture.md).
 * int returns: 0 ok, -1 error (flow_last_error), 1 ignored (no active session).
 * Strings returned by flow_* are caller-owned: free with flow_string_free.
 *
 * Threading (review C2). BLOCKING, never call on a UI thread: flow_load_models,
 * flow_unload (joins the worker, waits for a running commit), flow_start_session /
 * flow_start_session_info (server route: TCP+TLS+WS handshake up to connect_timeout_ms),
 * flow_commit, flow_probe_server, flow_probe_router, flow_probe_cloud.
 * NON-BLOCKING: everything else, notably flow_push_audio (a mid-session failover connects on
 * a helper thread), flow_poll, flow_partial, flow_set_context, flow_cancel, flow_update_router,
 * flow_set_cloud_key*, flow_route_status.
 * flow_cancel with no open session aborts a flow_commit running on another thread (it returns
 * ERROR "cancelled" at once, no fallback); flow_start_session* never aborts a running commit. */
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
/* Key bound to this CloudProviderConfig's base-URL host + adapter; a later config changing either drops it. */
int flow_set_cloud_key_for(FlowHandle *h, const char *provider_json, const char *key_or_null);
/* key_or_null NULL = the stored key, used only if provider_json has the host + adapter it was saved for */
char *flow_probe_cloud(FlowHandle *h, const char *provider_json, const char *key_or_null); /* HealthReport JSON; blocks; h may be NULL (key required) */
char *flow_cloud_usage(FlowHandle *h);                               /* CloudUsage JSON */
int flow_set_cloud_usage(FlowHandle *h, const char *usage_json);     /* restore persisted counters */
void flow_reset_cloud_usage(FlowHandle *h);
char *flow_cloud_capabilities(void);                                 /* CloudCapabilities JSON */
char *flow_cloud_presets(void);                                      /* D28: the single OpenRouter CloudProviderConfig ([] without cloud) */

/* D28 OpenRouter-only cloud (docs/proposals/openrouter-only-cloud.md). */
char *flow_migrate_cloud(const char *router_config_json);  /* {"config","report"}; NULL + bad_config */
int flow_set_cleanup(FlowHandle *h, const char *cleanup_json); /* {"allow_cloud_cleanup"?,"cleanup_order"?,"cleanup"?} */
char *flow_cleanup_status(FlowHandle *h);                   /* CleanupStatus JSON */
/* Command Mode: selected NULL = generate at the cursor. BLOCKS (LLM). {"ok":true,"text"} |
 * {"ok":false,"kind":"no_llm"|"selection_not_allowed"|"no_instruction"|"selection_too_long"|
 * "timeout"|"empty"|"backend","error":pill text} */
char *flow_command(FlowHandle *h, const char *instruction, const char *selected);
char *flow_cloud_notice(FlowHandle *h);                     /* one-time migration notice or NULL */
void flow_ack_cloud_notice(FlowHandle *h);
int flow_set_cloud_model(FlowHandle *h, const char *use_, const char *model); /* use_ "stt"|"cleanup" */
void flow_set_cloud_cache_dir(const char *dir);             /* <dir>/openrouter-catalog.json; no network */
char *flow_cloud_catalog(const char *use_, const char *query, const char *filters_json); /* picker rows JSON; no network */
char *flow_refresh_cloud_catalog(FlowHandle *h, int force); /* BLOCKS <= 10 s; status JSON (+ "error") */
char *flow_cloud_key_info(FlowHandle *h);                   /* BLOCKS; KeyInfo JSON or {"error":code} */

/* Live route + hands-free endpointing (2026-10-22, additive). Both work on core and router
 * handles; only a router reports "route" ({"engine":"local"|"remote"|"cloud","provider"?,
 * "model"?,"server"?,"fallback"?}). */
/* flow_start_session that returns StartInfo JSON {"queued":bool,"route"?:{…}};
 * NULL on error (same flow_last_error / flow_last_error_kind as flow_start_session). */
char *flow_start_session_info(FlowHandle *h, const char *session_config_json);
/* Non-blocking RouterPoll JSON {"partial":str|null,"endpoint_reached":bool,"speech_ms":n,
 * "trailing_silence_ms":n,"route"?:{…}}: call on the partial timer. "route" is present while a
 * session is live and changes when a mid-session failure switches route. NULL for a NULL handle. */
char *flow_poll(FlowHandle *h);

#ifdef __cplusplus
}
#endif
#endif
