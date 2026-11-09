/* murmur-core C ABI. All configs/results are UTF-8 JSON (see docs/core-architecture.md).
 * int returns: 0 ok, -1 error (murmur_last_error), 1 ignored (no active session).
 * Strings returned by murmur_* are caller-owned: free with murmur_string_free.
 *
 * Threading (review C2). BLOCKING, never call on a UI thread: murmur_load_models,
 * murmur_unload (joins the worker, waits for a running commit), murmur_start_session /
 * murmur_start_session_info (server route: TCP+TLS+WS handshake up to connect_timeout_ms),
 * murmur_commit, murmur_probe_server, murmur_probe_router, murmur_probe_cloud.
 * NON-BLOCKING: everything else, notably murmur_push_audio (a mid-session failover connects on
 * a helper thread), murmur_poll, murmur_partial, murmur_set_context, murmur_cancel, murmur_update_router,
 * murmur_set_cloud_key*, murmur_route_status.
 * murmur_cancel with no open session aborts a murmur_commit running on another thread (it returns
 * ERROR "cancelled" at once, no fallback); murmur_start_session* never aborts a running commit. */
#ifndef MURMUR_CORE_H
#define MURMUR_CORE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

typedef struct MurmurHandle MurmurHandle;

MurmurHandle *murmur_load_models(const char *model_config_json); /* NULL on error */
void murmur_unload(MurmurHandle *h);
int murmur_start_session(MurmurHandle *h, const char *session_config_json);
int murmur_push_audio(MurmurHandle *h, const int16_t *pcm16_16k_mono, size_t len); /* non-blocking */
int murmur_set_context(MurmurHandle *h, const char *field_context_json);
char *murmur_partial(MurmurHandle *h);                                   /* NULL if none */
char *murmur_commit(MurmurHandle *h);                                    /* CommitResult JSON; blocks */
void murmur_cancel(MurmurHandle *h);
char *murmur_learn_edit(MurmurHandle *h, const char *pasted, const char *edited); /* JSON array */
const char *murmur_last_error(void);
/* "" | "bad_config" | "model_not_found" | "llm_lib_not_found" | "load" | "error"
 * | router: "no_engine_enabled" | "local_not_ready" | "server_not_configured" | "server_unavailable" */
const char *murmur_last_error_kind(void);
void murmur_string_free(char *s);

/* D17 hybrid router (docs/hybrid-architecture.md §6.2). A router handle works
 * with every session function above; murmur_unload frees it (and its local engine). */
char *murmur_capabilities(void);                                       /* Capabilities JSON */
/* Takes ownership of `local` (a murmur_load_models handle, or NULL) whatever the outcome.
 * NULL on error: kind "no_engine_enabled" / "bad_config". */
MurmurHandle *murmur_open_router(const char *router_config_json, MurmurHandle *local);
int murmur_update_router(MurmurHandle *h, const char *router_config_json); /* next session */
char *murmur_probe_server(const char *server_config_json);             /* HealthReport JSON; blocks */
char *murmur_probe_router(MurmurHandle *h);                              /* probe saved server; blocks */
char *murmur_route_status(MurmurHandle *h);                              /* RouteStatus JSON */
void murmur_reset_circuit(MurmurHandle *h);

/* D18 cloud STT (docs/cloud-stt.md). Libraries built without feature `cloud`
 * keep these symbols: they report "not in this build". Keys only ever enter
 * here (never in router JSON); the library never logs or returns them. */
int murmur_set_cloud_key(MurmurHandle *h, const char *provider_id, const char *key_or_null); /* NULL/"" = forget; -1 for a core handle */
/* Key bound to this CloudProviderConfig's base-URL host + adapter; a later config changing either drops it. */
int murmur_set_cloud_key_for(MurmurHandle *h, const char *provider_json, const char *key_or_null);
/* key_or_null NULL = the stored key, used only if provider_json has the host + adapter it was saved for */
char *murmur_probe_cloud(MurmurHandle *h, const char *provider_json, const char *key_or_null); /* HealthReport JSON; blocks; h may be NULL (key required) */
char *murmur_cloud_usage(MurmurHandle *h);                               /* CloudUsage JSON */
int murmur_set_cloud_usage(MurmurHandle *h, const char *usage_json);     /* restore persisted counters */
void murmur_reset_cloud_usage(MurmurHandle *h);
char *murmur_cloud_capabilities(void);                                 /* CloudCapabilities JSON */
char *murmur_cloud_presets(void);                                      /* D28: the single OpenRouter CloudProviderConfig ([] without cloud) */

/* D28 OpenRouter-only cloud (docs/proposals/openrouter-only-cloud.md). */
char *murmur_migrate_cloud(const char *router_config_json);  /* {"config","report"}; NULL + bad_config */
int murmur_set_cleanup(MurmurHandle *h, const char *cleanup_json); /* {"allow_cloud_cleanup"?,"cleanup_order"?,"cleanup_off"?,"cleanup"?}; entries "local", "server" (D36), "cloud" */
char *murmur_cleanup_status(MurmurHandle *h);                   /* CleanupStatus JSON */
/* Command Mode: selected NULL = generate at the cursor. BLOCKS (LLM). {"ok":true,"text"} |
 * {"ok":false,"kind":"no_llm"|"selection_not_allowed"|"no_instruction"|"selection_too_long"|
 * "timeout"|"empty"|"backend","error":pill text} */
char *murmur_command(MurmurHandle *h, const char *instruction, const char *selected);
char *murmur_cloud_notice(MurmurHandle *h);                     /* one-time migration notice or NULL */
void murmur_ack_cloud_notice(MurmurHandle *h);
int murmur_set_cloud_model(MurmurHandle *h, const char *use_, const char *model); /* use_ "stt"|"cleanup" */
void murmur_set_cloud_cache_dir(const char *dir);             /* <dir>/openrouter-catalog.json; no network */
char *murmur_cloud_catalog(const char *use_, const char *query, const char *filters_json); /* picker rows JSON; no network */
char *murmur_refresh_cloud_catalog(MurmurHandle *h, int force); /* BLOCKS <= 10 s; status JSON (+ "error") */
char *murmur_cloud_key_info(MurmurHandle *h);                   /* BLOCKS; KeyInfo JSON or {"error":code} */

/* Live route + hands-free endpointing (2026-10-22, additive). Both work on core and router
 * handles; only a router reports "route" ({"engine":"local"|"remote"|"cloud","provider"?,
 * "model"?,"server"?,"fallback"?}). */
/* murmur_start_session that returns StartInfo JSON {"queued":bool,"route"?:{…}};
 * NULL on error (same murmur_last_error / murmur_last_error_kind as murmur_start_session). */
char *murmur_start_session_info(MurmurHandle *h, const char *session_config_json);
/* Non-blocking RouterPoll JSON {"partial":str|null,"endpoint_reached":bool,"speech_ms":n,
 * "trailing_silence_ms":n,"route"?:{…}}: call on the partial timer. "route" is present while a
 * session is live and changes when a mid-session failure switches route. NULL for a NULL handle. */
char *murmur_poll(MurmurHandle *h);

#ifdef __cplusplus
}
#endif
#endif
