export interface Project {
  id: string;
  name: string;
  remote_url: string | null;
  root_paths: string[];
  privacy_mode: 'full' | 'local_only' | 'paused';
  created_at: string;
  updated_at: string;
}

export interface MemoryHit {
  id: string;
  scope: 'project' | 'global';
  project_id: string | null;
  project_name: string | null;
  source_client: string;
  memory_type: string;
  title: string;
  summary: string;
  status: 'active' | 'pinned' | 'superseded' | 'archived';
  files: string[];
  tags: string[];
  updated_at: string;
  score: number;
}

export interface MemoryDetail extends MemoryHit {
  session_id: string | null;
  content: string;
  superseded_by: string | null;
  created_at: string;
}

export interface UsageSummary {
  today_input_tokens: number;
  today_output_tokens: number;
  today_total_tokens: number;
  today_cost_usd: number;
  today_calls: number;
  today_blocked: number;
  total_memories: number;
  total_projects: number;
  pending_raw_events: number;
  daily_token_cap: number;
  daily_cost_cap_usd: number;
  budget_exhausted: boolean;
  active_provider: string;
  active_model: string;
  embedding_status: string;
}

export interface UsageEntry {
  id: string;
  provider: string;
  model: string;
  operation: string;
  input_tokens: number;
  output_tokens: number;
  estimated_cost_usd: number;
  session_id: string | null;
  status: string;
  error_message: string | null;
  created_at: string;
}

export interface ClientConnection {
  client_id: string;
  display_name: string;
  detected: boolean;
  connected: boolean;
  needs_manual_step: boolean;
  manual_step_hint: string | null;
  config_paths: string[];
  config_backup_path: string | null;
  last_heartbeat_at: string | null;
  last_event_at: string | null;
  status_note: string;
}

export interface ConnectionPreview {
  client_id: string;
  display_name: string;
  target_files: string[];
  summary: string;
  diff_preview: string;
  manual_step_hint: string | null;
}

export interface UniversalMcpSnippets {
  streamable_http_url: string;
  json_http: string;
  toml_http: string;
  json_stdio: string;
}

export interface SettingsView {
  provider: string;
  model: string;
  base_url: string;
  api_key_masked: string;
  has_api_key: boolean;
  daily_token_cap: number;
  daily_cost_cap_usd: number;
  session_start_token_budget: number;
  embedding_mode: string;
  embedding_model: string;
  raw_event_retention_days: number;
  t3_watcher_enabled: boolean;
  distill_idle_seconds: number;
  zero_silent_fallback: boolean;
}

export interface ConnectionTestResult {
  ok: boolean;
  provider: string;
  model: string;
  latency_ms: number;
  input_tokens: number;
  output_tokens: number;
  message: string;
}

export interface ContextPreviewResponse {
  briefing: string;
  memory_count: number;
  approx_tokens: number;
  token_budget: number;
}
