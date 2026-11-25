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
  session_id: string | null;
  source_client: string;
  memory_type: string;
  title: string;
  summary: string;
  status: 'active' | 'pinned' | 'superseded' | 'archived';
  files: string[];
  tags: string[];
  superseded_by: string | null;
  recall_count: number;
  last_recalled_at: string | null;
  last_recalled_clients: string[];
  updated_at: string;
  score: number;
}

export interface MemoryDetail extends MemoryHit {
  content: string;
  created_at: string;
}

export interface SessionSummary {
  id: string;
  project_id: string;
  project_name: string | null;
  source_client: string;
  branch: string | null;
  started_at: string;
  ended_at: string | null;
  last_turn_at: string;
  status: 'active' | 'ended';
  pending_events: number;
  total_events: number;
  memory_count: number;
}

export interface RawEventView {
  id: string;
  session_id: string;
  project_id: string;
  project_name: string | null;
  source_client: string;
  event_type: string;
  tool_name: string | null;
  preview: string;
  distilled: boolean;
  created_at: string;
}

export interface LiveStreamItem {
  id: string;
  kind:
    | 'raw_event'
    | 'usage_recorded'
    | 'memory_created'
    | 'memory_updated'
    | 'memory_deleted'
    | 'connection_changed';
  timestamp: string;
  source_client: string;
  project_id: string | null;
  project_name: string | null;
  session_id: string | null;
  event_type: string;
  tool_name: string | null;
  preview: string;
  status?: string;
  tokens?: number;
  cost_usd?: number;
  memory_id?: string;
}

export type GraphEdgeKind =
  | 'supersedes'
  | 'same_file'
  | 'same_session'
  | 'shared_tag'
  | 'semantic_similar';

export interface GraphNode {
  id: string;
  title: string;
  summary: string;
  scope: 'project' | 'global';
  project_id: string | null;
  project_name: string | null;
  source_client: string;
  memory_type: string;
  status: 'active' | 'pinned' | 'superseded' | 'archived';
  files: string[];
  tags: string[];
  session_id: string | null;
  superseded_by: string | null;
  recall_count: number;
  updated_at: string;
  degree: number;
}

export interface GraphEdge {
  source: string;
  target: string;
  kind: GraphEdgeKind;
  weight: number;
  label: string;
}

export interface MemoryGraphResponse {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface NeighborLink {
  memory: MemoryHit;
  edge_kind: GraphEdgeKind;
  weight: number;
  label: string;
  direction: 'outgoing' | 'incoming' | 'undirected';
}

export interface MemoryContextResponse {
  memory: MemoryDetail;
  timeline: MemoryHit[];
  lineage: MemoryHit[];
  neighbors: NeighborLink[];
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
  total_recalls?: number;
  active_sessions?: number;
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
  configured_port: number | null;
  port_mismatch: boolean;
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
  onboarding_completed: boolean;
  auto_update_check: boolean;
  zero_silent_fallback: boolean;
}

export interface ReleaseInfo {
  tag_name: string;
  version: string;
  name: string;
  body: string;
  html_url: string;
  published_at: string | null;
}

export interface SelfUpdateCapability {
  supported: boolean;
  reason: string | null;
  install_kind: 'npx' | 'binary' | 'dev_build' | 'unsupported_platform';
  manual_command: string;
  asset_name: string | null;
}

export interface UpdateRuntimeState {
  in_progress: boolean;
  stage:
    | 'idle'
    | 'downloading'
    | 'verifying'
    | 'extracting'
    | 'smoke_testing'
    | 'replacing'
    | 'restarting'
    | 'error';
  target_version: string | null;
  error: string | null;
}

export interface WhatsNewInfo {
  should_show: boolean;
  version: string;
  previous_version: string | null;
  title: string;
  body: string | null;
  html_url: string;
  compare_url: string | null;
}

export interface UpdateStatusResponse {
  current_version: string;
  auto_check_enabled: boolean;
  last_checked_at: string | null;
  check_error: string | null;
  update_available: boolean;
  dismissed_version: string | null;
  latest_release: ReleaseInfo | null;
  capability: SelfUpdateCapability;
  runtime: UpdateRuntimeState;
  whats_new: WhatsNewInfo;
}

export interface AutoStartStatus {
  supported: boolean;
  enabled: boolean;
  platform: 'macos' | 'linux' | 'windows' | 'unsupported';
  service_path: string | null;
  note: string;
}

export interface DaemonInfo {
  running: boolean;
  pid: number;
  port: number;
  version: string;
  mcp_url: string;
  data_dir: string;
  log_path: string;
  autostart: AutoStartStatus;
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

export interface BriefingItemBreakdown {
  id: string;
  title: string;
  source_client: string;
  memory_type: string;
  status: string;
  approx_tokens: number;
  included: boolean;
}

export interface BriefingSectionBreakdown {
  kind: 'pinned' | 'project' | 'global';
  label: string;
  tokens: number;
  items: BriefingItemBreakdown[];
}

export interface ContextPreviewResponse {
  briefing: string;
  memory_count: number;
  approx_tokens: number;
  token_budget: number;
  archive_approx_tokens: number;
  sections: BriefingSectionBreakdown[];
}
