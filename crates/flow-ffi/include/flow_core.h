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
void flow_string_free(char *s);

#ifdef __cplusplus
}
#endif
#endif
