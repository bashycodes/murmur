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
  /** D47: the meeting transcription order (absent from an older backend). */
  meetings?: MeetingOrder;
}

/** D47: one place that transcribes meetings, with its own switch. */
export interface MeetingRoute {
  route: Route;
  enabled: boolean;
}

export interface MeetingOrder {
  routes: MeetingRoute[];
  /** Never edited: dictation's order and switches. */
  follows_dictation: boolean;
}

// ------------------------------------------------- OpenRouter (D28)

export interface KeyState {
  set: boolean;
  masked: string;
}

export interface UsageRow {
  /** `cloud::PURPOSE_*`; `custom` = the D52 custom cleanup server (never in the budget). */
  purpose: "stt" | "cleanup" | "removed" | "custom";
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
  /** D36: the engines that may clean, in order (switched-off ones left out). */
  order?: CleanupRoute[];
  /** D36: the server can clean text (`null`/absent = not known yet or no server). */
  server_llm?: boolean | null;
  /** D36: the server cleanup circuit, when the server is in the order. */
  server_circuit?: Circuit | null;
  consent: boolean;
  blocked?: string | null;
  circuit: Circuit | null;
  model: string;
  timeout_ms: number;
  last: CleanUsage | null;
  month_cost_usd: number | null;
  custom_month_cost_usd?: number | null;
  /** D52: who the cloud cleanup route calls. */
  provider?: CloudProvider;
  /** D52: the custom server's host (absent for OpenRouter). */
  host?: string;
}

/** D52: OpenRouter, or the user's own OpenAI-compatible server. */
export type CloudProvider = "openrouter" | "custom";

/** D52: the custom cleanup server (`murmur/cloud/custom` key, masked). */
export interface CustomCleanupView {
  /** As saved: normalised (`/v1` added when typed without a path). */
  base_url: string;
  host: string;
  model: string;
  timeout_ms: number;
  /** Consent for the current host (a host change voids it). */
  consent: boolean;
  consent_host: string;
  consent_at_ms: number;
  plain_http: boolean;
  key: KeyState;
}

export type Purpose = "stt" | "cleanup";
export type CleanupRoute = "local" | "server" | "cloud";

/** One entry of the D36 cleanup order (all three engines, in order). */
export interface CleanupEngine {
  route: CleanupRoute;
  enabled: boolean;
}

export interface OrView {
  supported: boolean;
  key: KeyState;
  privacy: { zdr: boolean; data_collection: "deny" | "allow" };
  stt: {
    /** D78: the Cloud provider can do speech to text (OpenRouter only). */
    available?: boolean;
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
    /** D36: every engine once, in the order tried, with its switch. */
    engines: CleanupEngine[];
    model: string;
    sort: string;
    timeout_ms: number;
    send_surrounding_text: boolean;
    send_screen_words: boolean;
    send_app_name: boolean;
    status: CleanupStatus;
    /** D52: `allow`, the order and "Also send" are shared; the rest above is OpenRouter's. */
    provider: CloudProvider;
    custom: CustomCleanupView;
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

// ------------------------------------------------- Users & keys (D39)

/** `admin_status`: whether the saved token is an admin key. */
export interface AdminStatus {
  configured: boolean;
  admin: boolean;
  name: string;
  id: string;
  error?: string;
  code?: string;
}

/** murmur_protocol::admin::KeyView. Times are Unix seconds. */
export interface KeyView {
  id: string;
  name: string;
  admin: boolean;
  created: number;
  revoked?: number | null;
  /** Last dictation or model download (absent on older servers). */
  last_used?: number | null;
  /** Last authenticated request of any kind (absent on older servers). */
  last_connected?: number | null;
  /** Older servers' name for last connected; newer servers send both. */
  last_seen?: number | null;
  client?: string | null;
  dictations: number;
  audio_ms: number;
  /** Model downloads from the server (absent on older servers). */
  downloads?: number;
  download_bytes?: number;
  /** The key this app is using. */
  current: boolean;
}

export interface KeyList {
  keys: KeyView[];
}

/** `admin_create_key`: `secret` is shown once, never stored. */
export interface CreatedKey {
  key: KeyView;
  secret: string;
}
