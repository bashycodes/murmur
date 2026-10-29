// Backend view shapes shared by several pages (Developer / engines and
// OpenRouter, D28). Page-local shapes stay in their page module.

export type Route = "server" | "client" | "cloud";

export interface DevForm {
  allow_server: boolean;
  allow_client: boolean;
  route_order: Route[];
  host: string;
  port: number;
  tls: boolean;
  tls_pin_sha256: string;
  token: string;
  token_saved?: boolean;
  connect_timeout_ms: number;
  commit_timeout_base_ms: number;
  health_ttl_s: number;
  keep_local_warm: boolean;
  show_engine_badge: boolean;
  dev_unlocked: boolean;
  decode_while_speaking: boolean;
  parakeet_decoding: string;
}

export interface FieldError {
  field: string;
  message: string;
}

export interface DevCheck {
  errors: FieldError[];
  warnings: string[];
  tls_forced: boolean;
}

export interface Health {
  ok: boolean;
  error: string | null;
  message: string | null;
  server: string | null;
  rtt_ms: { min: number; median: number; max: number } | null;
  server_version: string | null;
  server_core: string | null;
  postprocess_match: boolean | null;
  proto: number | null;
  asr_backend: string | null;
  asr_model: string | null;
  llm: boolean | null;
  workers: number | null;
  busy: number | null;
  provider?: string;
  model?: string;
  transcript?: string;
  latency_ms?: number;
  cost_usd?: number;
}

export interface LastDictation {
  engine: string | null;
  fallback: string | null;
  server: string | null;
  rtt_ms: number | null;
  queue_ms: number | null;
  asr_ms: number;
  total_ms: number;
  /** D22: release -> result on this PC (network included); null = not measured. */
  client_ms: number | null;
  wait_ms: number;
  segments: number;
  pre_asr_ms: number;
  status: string;
  empty_reason: string | null;
  /** D27: "AI cleaned" / "AI not needed (already clean)" / …; null = LLM off. */
  llm: string | null;
  /** D28: which engine cleaned the text (`local` / `cloud` / `server`), its model and cost. */
  llm_engine?: string | null;
  llm_model?: string | null;
  llm_cost_usd?: number | null;
  /** D28: e.g. "AI cleaned · OpenRouter · meta-llama/… · $0.0007"; shown instead of `llm`. */
  llm_row?: string | null;
  /** D30: what produced the text when AI cleanup was tried (`cloud` / `local` / `rules`) and why the cloud answer was not used. */
  cleanup_route?: string | null;
  cloud_fallback?: string | null;
  at_ms: number;
  row: string | null;
  notice: string | null;
}

export interface Circuit {
  open: boolean;
  reason: string | null;
  retry_in_ms: number | null;
  until_config_change: boolean;
}

export interface RouteView {
  last: LastDictation | null;
  circuit: Circuit;
  health: Health | null;
  cloud_circuit: Circuit | null;
}

export interface DevView {
  form: DevForm;
  caps: { local: boolean; remote: boolean; cloud: boolean };
  capabilities: { local: boolean; remote: boolean; asr_engines: string[]; llm: boolean; proto: number; core_version: string };
  visible: boolean;
  /** D26: `0.2.0`, `abc1234` / `abc1234-dirty` / `unknown`, `0.2.0+abc1234`. */
  version: string;
  build: string;
  full: string;
  check: DevCheck;
  route: RouteView;
}

// ------------------------------------------------- OpenRouter (D28)

export interface KeyState {
  set: boolean;
  masked: string;
}

export interface UsageRow {
  purpose: string;
  label: string;
  requests: number;
  minutes: number;
  cost: string | number;
}

export interface UsageView {
  month: string;
  speech: string | number;
  cleanup: string | number;
  total: string | number;
  budget: number | null;
  budget_reached: boolean;
  rows: UsageRow[];
}

export type CatalogSource = "Live" | "Cache" | "Baked";

export interface CatalogStatus {
  source: CatalogSource;
  fetched_at_ms: number;
  snapshot_date: string | null;
  models: number;
  stt: number;
  cleanup: number;
  error?: string;
}

export type SttPath = "transcriptions" | "chat_audio";

export interface ModelRow {
  id: string;
  name: string;
  context_length: number;
  input: string[];
  output: string[];
  stt_path: SttPath | null;
  zdr: boolean;
  free: boolean;
  allowed_by_account: boolean | null;
  recommended: boolean;
  price_label: string;
  badges: string[];
}

export interface CleanUsage {
  engine: string | null;
  model: string | null;
  cost_usd: number | null;
  ttft_ms: number | null;
  latency_ms: number | null;
}

export interface CleanupStatus {
  supported: boolean;
  enabled: boolean;
  consent: boolean;
  blocked?: string | null;
  circuit: Circuit | null;
  model: string;
  timeout_ms: number;
  last: CleanUsage | null;
  month_cost_usd: number | null;
}

export type Purpose = "stt" | "cleanup";
export type CleanupRoute = "cloud" | "local";

export interface OrView {
  supported: boolean;
  key: KeyState;
  privacy: { zdr: boolean; data_collection: "deny" | "allow" };
  stt: {
    allow: boolean;
    consent: boolean;
    consent_at_ms: number;
    model: string;
    stt_path: "auto" | SttPath;
    language: string;
    send_dictionary: boolean;
  };
  cleanup: {
    allow: boolean;
    consent: boolean;
    consent_at_ms: number;
    order: CleanupRoute[];
    model: string;
    sort: string;
    timeout_ms: number;
    send_surrounding_text: boolean;
    send_screen_words: boolean;
    send_app_name: boolean;
    status: CleanupStatus;
  };
  budget: number | null;
  usage: UsageView;
  catalog: CatalogStatus;
  notice: string | null;
  warnings: string[];
  /** The saved route order (Developer → Engines). */
  route_order?: Route[];
  stt_model_info: ModelRow | null;
  cleanup_model_info: ModelRow | null;
}
