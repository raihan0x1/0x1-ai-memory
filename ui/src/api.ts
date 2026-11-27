import type {
  AutoStartStatus,
  ClientConnection,
  ConnectionPreview,
  ConnectionTestResult,
  ContextPreviewResponse,
  DaemonInfo,
  MemoryContextResponse,
  MemoryDetail,
  MemoryGraphResponse,
  MemoryHit,
  Project,
  RawEventView,
  SessionSummary,
  SettingsView,
  UniversalMcpSnippets,
  UpdateStatusResponse,
  UsageEntry,
  UsageSummary,
} from './types';

export const DEFAULT_DAEMON_PORT = 10101;

function resolveSameOriginApiUrl(path: string): string {
  if ((!path.startsWith('/api/') && path !== '/health') || path.startsWith('//')) {
    throw new Error(`Refusing non-API request path: ${path}`);
  }
  const url = new URL(path, window.location.origin);
  if (url.origin !== window.location.origin) {
    throw new Error(`Refusing cross-origin request: ${url.origin}`);
  }
  return `${url.pathname}${url.search}`;
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const target = resolveSameOriginApiUrl(path);
  const res = await fetch(target, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers ?? {}),
    },
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  return res.json() as Promise<T>;
}

function buildQuery(params: Record<string, string | number | boolean | undefined>): string {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== '') qs.set(k, String(v));
  }
  const s = qs.toString();
  return s ? `?${s}` : '';
}

export const api = {
  summary: () => req<UsageSummary>('/api/summary'),
  projects: () => req<{ projects: Project[] }>('/api/projects'),
  setProjectPrivacy: (id: string, privacy_mode: string) =>
    req<{ updated: boolean }>(`/api/projects/${encodeURIComponent(id)}/privacy`, {
      method: 'PATCH',
      body: JSON.stringify({ privacy_mode }),
    }),
  sessions: (
    params: { project_id?: string; client?: string; limit?: number; offset?: number } = {},
    signal?: AbortSignal,
  ) =>
    req<{ sessions: SessionSummary[]; total: number; limit: number; offset: number }>(
      `/api/sessions${buildQuery(params)}`,
      { signal },
    ),
  distillSession: (id: string) =>
    req<{ queued: boolean; session_id: string }>(
      `/api/sessions/${encodeURIComponent(id)}/distill`,
      { method: 'POST' },
    ),
  distillFlush: () =>
    req<{ queued_sessions: number; session_ids: string[] }>('/api/distill/flush', {
      method: 'POST',
    }),
  rawEvents: (
    params: { project_id?: string; client?: string; kind?: string; limit?: number; before?: string } = {},
    signal?: AbortSignal,
  ) =>
    req<{ events: RawEventView[]; total: number; limit: number; next_cursor: string | null }>(
      `/api/raw-events${buildQuery(params)}`,
      { signal },
    ),
  searchMemories: (params: Record<string, string | number | boolean | undefined>, signal?: AbortSignal) =>
    req<{ count: number; total: number; limit: number; offset: number; memories: MemoryHit[] }>(
      `/api/memories${buildQuery(params)}`,
      { signal },
    ),
  getMemory: (id: string) => req<{ memory: MemoryDetail }>(`/api/memories/${encodeURIComponent(id)}`),
  memoryContext: (id: string) =>
    req<MemoryContextResponse>(`/api/memories/${encodeURIComponent(id)}/context`),
  graph: (params: Record<string, string | number | boolean | undefined>, signal?: AbortSignal) =>
    req<MemoryGraphResponse>(`/api/graph${buildQuery(params)}`, { signal }),
  createMemory: (body: {
    scope: string;
    project_id?: string | null;
    memory_type: string;
    title: string;
    summary: string;
    content: string;
    tags?: string[];
    files?: string[];
    status?: string;
  }) =>
    req<{ memory: MemoryDetail }>('/api/memories', {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  updateMemory: (
    id: string,
    patch: Partial<{
      status: string;
      title: string;
      summary: string;
      content: string;
      tags: string[];
      files: string[];
      superseded_by: string;
    }>,
  ) =>
    req<{ memory: MemoryDetail }>(`/api/memories/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(patch),
    }),
  deleteMemory: (id: string) =>
    req<{ deleted: boolean }>(`/api/memories/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  contextPreview: (projectId?: string) => {
    const qs = projectId ? `?project_id=${encodeURIComponent(projectId)}` : '';
    return req<ContextPreviewResponse>(`/api/context-preview${qs}`);
  },
  usageHistory: (params: { limit?: number; offset?: number } = {}) =>
    req<{ usage: UsageEntry[]; total: number; limit: number; offset: number }>(
      `/api/usage${buildQuery(params)}`,
    ),
  settings: () => req<SettingsView>('/api/settings'),
  patchSettings: (patch: Record<string, unknown>) =>
    req<SettingsView>('/api/settings', {
      method: 'PATCH',
      body: JSON.stringify(patch),
    }),
  testEngine: (draft?: Record<string, unknown>) =>
    req<ConnectionTestResult>('/api/engine/test', {
      method: 'POST',
      ...(draft ? { body: JSON.stringify(draft) } : {}),
    }),
  connections: () => req<{ connections: ClientConnection[] }>('/api/connections'),
  universalSnippets: () => req<UniversalMcpSnippets>('/api/connections/universal'),
  previewConnect: (client: string) =>
    req<ConnectionPreview>(`/api/connections/${encodeURIComponent(client)}/preview`),
  connectClient: (client: string) =>
    req<{ connection: ClientConnection }>(`/api/connections/${encodeURIComponent(client)}/connect`, {
      method: 'POST',
    }),
  disconnectClient: (client: string) =>
    req<{ connection: ClientConnection }>(
      `/api/connections/${encodeURIComponent(client)}/disconnect`,
      { method: 'POST' },
    ),
  pingClient: (client: string) =>
    req<{
      ok: boolean;
      mcp_ok: boolean;
      config_connected: boolean;
      mcp_url: string;
      connection: ClientConnection | null;
    }>(`/api/connections/${encodeURIComponent(client)}/ping`, { method: 'POST' }),
  daemon: () => req<DaemonInfo>('/api/daemon'),
  stopDaemon: () => req<{ stopping: boolean }>('/api/daemon/stop', { method: 'POST' }),
  setAutostart: (enabled: boolean) =>
    req<AutoStartStatus>('/api/daemon/autostart', {
      method: 'POST',
      body: JSON.stringify({ enabled }),
    }),
  health: () => req<{ ok: boolean; service: string; version: string }>('/health'),
  updateStatus: () => req<UpdateStatusResponse>('/api/updates/status'),
  checkUpdates: () => req<UpdateStatusResponse>('/api/updates/check', { method: 'POST' }),
  applyUpdate: () => req<UpdateStatusResponse>('/api/updates/apply', { method: 'POST' }),
  dismissUpdate: (version: string) =>
    req<UpdateStatusResponse>('/api/updates/dismiss', {
      method: 'POST',
      body: JSON.stringify({ version }),
    }),
  ackWhatsNew: () => req<UpdateStatusResponse>('/api/updates/ack-whats-new', { method: 'POST' }),
  fetchWhatsNew: () => req<UpdateStatusResponse>('/api/updates/fetch-whats-new', { method: 'POST' }),
};

function parseUtcDate(iso: string): Date | null {
  const trimmed = iso.trim();
  const hasZone = /([zZ]|[+-]\d{2}:?\d{2})$/.test(trimmed);
  const normalized = hasZone ? trimmed : `${trimmed.replace(' ', 'T')}Z`;
  const d = new Date(normalized);
  return Number.isNaN(d.getTime()) ? null : d;
}

/** Exact UTC timestamp for tooltip details alongside local timestamps. */
export function formatUtcTime(iso: string | null | undefined): string {
  if (!iso) return 'none';
  const d = parseUtcDate(iso);
  if (!d) return iso;
  const yyyy = d.getUTCFullYear();
  const mm = String(d.getUTCMonth() + 1).padStart(2, '0');
  const dd = String(d.getUTCDate()).padStart(2, '0');
  const hh = String(d.getUTCHours()).padStart(2, '0');
  const mi = String(d.getUTCMinutes()).padStart(2, '0');
  const ss = String(d.getUTCSeconds()).padStart(2, '0');
  return `${yyyy}-${mm}-${dd} ${hh}:${mi}:${ss} UTC`;
}

function localDayKey(d: Date): string {
  const yyyy = d.getFullYear();
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  return `${yyyy}-${mm}-${dd}`;
}

/** Local timestamp (`YYYY-MM-DD HH:MM:SS`). */
export function formatLocalTime(iso: string | null | undefined): string {
  if (!iso) return 'none';
  const d = parseUtcDate(iso);
  if (!d) return iso;
  const hh = String(d.getHours()).padStart(2, '0');
  const mi = String(d.getMinutes()).padStart(2, '0');
  const ss = String(d.getSeconds()).padStart(2, '0');
  return `${localDayKey(d)} ${hh}:${mi}:${ss}`;
}

/** Short local `HH:MM:SS` clock for compact live stream tails and session rows. */
export function formatLocalClock(iso: string | null | undefined): string {
  if (!iso) return '--:--:--';
  const d = parseUtcDate(iso);
  if (!d) return iso;
  const hh = String(d.getHours()).padStart(2, '0');
  const mi = String(d.getMinutes()).padStart(2, '0');
  const ss = String(d.getSeconds()).padStart(2, '0');
  return `${hh}:${mi}:${ss}`;
}

/** Local calendar day label (`Today · YYYY-MM-DD`, `Yesterday · YYYY-MM-DD`, or `YYYY-MM-DD`). */
export function formatLocalDayLabel(iso: string): string {
  const d = parseUtcDate(iso);
  if (!d) return 'Unknown date';
  const key = localDayKey(d);
  const now = new Date();
  const todayKey = localDayKey(now);
  const yest = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  const yestKey = localDayKey(yest);
  if (key === todayKey) return `Today · ${key}`;
  if (key === yestKey) return `Yesterday · ${key}`;
  return key;
}

export function formatLocalDayKey(iso: string): string {
  const d = parseUtcDate(iso);
  return d ? localDayKey(d) : 'unknown';
}
