import type {
  AutoStartStatus,
  ClientConnection,
  ConnectionPreview,
  ConnectionTestResult,
  ContextPreviewResponse,
  DaemonInfo,
  MemoryDetail,
  MemoryHit,
  Project,
  SettingsView,
  UniversalMcpSnippets,
  UsageEntry,
  UsageSummary,
} from './types';

export const DEFAULT_DAEMON_PORT = 10101;

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
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

export const api = {
  summary: () => req<UsageSummary>('/api/summary'),
  projects: () => req<{ projects: Project[] }>('/api/projects'),
  setProjectPrivacy: (id: string, privacy_mode: string) =>
    req<{ updated: boolean }>(`/api/projects/${encodeURIComponent(id)}/privacy`, {
      method: 'PATCH',
      body: JSON.stringify({ privacy_mode }),
    }),
  searchMemories: (params: Record<string, string | number | boolean | undefined>) => {
    const qs = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== '') qs.set(k, String(v));
    }
    return req<{ count: number; memories: MemoryHit[] }>(`/api/memories?${qs.toString()}`);
  },
  getMemory: (id: string) => req<{ memory: MemoryDetail }>(`/api/memories/${encodeURIComponent(id)}`),
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
  usageHistory: () => req<{ usage: UsageEntry[] }>('/api/usage'),
  settings: () => req<SettingsView>('/api/settings'),
  patchSettings: (patch: Record<string, unknown>) =>
    req<SettingsView>('/api/settings', {
      method: 'PATCH',
      body: JSON.stringify(patch),
    }),
  testEngine: () => req<ConnectionTestResult>('/api/engine/test', { method: 'POST' }),
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
};

/** Exact UTC timestamp (Rim rule: never a bare time without zone). */
export function formatUtcTime(iso: string | null | undefined): string {
  if (!iso) return 'none';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const yyyy = d.getUTCFullYear();
  const mm = String(d.getUTCMonth() + 1).padStart(2, '0');
  const dd = String(d.getUTCDate()).padStart(2, '0');
  const hh = String(d.getUTCHours()).padStart(2, '0');
  const mi = String(d.getUTCMinutes()).padStart(2, '0');
  const ss = String(d.getUTCSeconds()).padStart(2, '0');
  return `${yyyy}-${mm}-${dd} ${hh}:${mi}:${ss} UTC`;
}
