import { useCallback, useEffect, useRef, useState } from 'react';
import { api, DEFAULT_DAEMON_PORT } from './api';
import { IconEngine, IconMemory, IconPlug, IconStream } from './components/Icons';
import { ConnectionsPage } from './pages/ConnectionsPage';
import { EnginePage } from './pages/EnginePage';
import { MemoriesPage } from './pages/MemoriesPage';
import { OnboardingPage } from './pages/OnboardingPage';
import { StreamPage } from './pages/StreamPage';
import type {
  ClientConnection,
  DaemonInfo,
  LiveStreamItem,
  MemoryHit,
  Project,
  SettingsView,
  UsageSummary,
} from './types';

export type Tab = 'stream' | 'memories' | 'connections' | 'engine' | 'onboarding';

const TAB_TITLES: Record<Tab, string> = {
  stream: 'Live Stream | 0x1-ai-memory',
  memories: 'Memories | 0x1-ai-memory',
  connections: 'Connections | 0x1-ai-memory',
  engine: 'Engine & Budget | 0x1-ai-memory',
  onboarding: 'Onboarding | 0x1-ai-memory',
};

const ROUTE_TABS = new Set<Tab>(['memories', 'connections', 'engine', 'onboarding']);

function readTabFromUrl(): Tab {
  const path = window.location.pathname.replace(/^\/+|\/+$/g, '') as Tab;
  if (ROUTE_TABS.has(path)) return path;
  const param = new URLSearchParams(window.location.search).get('tab') as Tab | null;
  if (param && ROUTE_TABS.has(param)) return param;
  return 'stream';
}

function summarizeSsePayload(
  eventType: string,
  toolName: string | null,
  payloadJson?: string,
): string {
  if (!payloadJson) return toolName ?? eventType;
  try {
    const p = JSON.parse(payloadJson) as Record<string, unknown>;
    if (eventType === 'prompt' && typeof p.prompt === 'string') {
      return p.prompt.replace(/\s+/g, ' ').slice(0, 120);
    }
    if (eventType === 'turn_stop' && typeof p.assistant === 'string') {
      return p.assistant.replace(/\s+/g, ' ').slice(0, 120);
    }
    const input = p.input as Record<string, unknown> | undefined;
    const detail =
      input?.CommandLine ?? input?.command ?? input?.Path ?? input?.file_path ?? input?.TargetFile;
    if (typeof detail === 'string' && detail.trim()) {
      return `${toolName ?? 'tool'}: ${detail.replace(/\s+/g, ' ').slice(0, 100)}`;
    }
  } catch {
    // ignore malformed payload preview
  }
  return toolName ?? eventType;
}

function parseSseItem(kind: LiveStreamItem['kind'], raw: string, seq: number): LiveStreamItem | null {
  try {
    const outer = JSON.parse(raw) as {
      timestamp?: string;
      payload?: Record<string, unknown>;
    };
    const ts = outer.timestamp ?? new Date().toISOString();
    const p = outer.payload ?? {};

    if (kind === 'raw_event') {
      const evType = String(p.event_type ?? 'raw_event');
      const toolName = typeof p.tool_name === 'string' ? p.tool_name : null;
      return {
        id: typeof p.id === 'string' ? p.id : `sse_${seq}`,
        kind,
        timestamp: typeof p.created_at === 'string' ? p.created_at : ts,
        source_client: String(p.source_client ?? 'mcp'),
        project_id: typeof p.project_id === 'string' ? p.project_id : null,
        project_name: null,
        session_id: typeof p.session_id === 'string' ? p.session_id : null,
        event_type: evType,
        tool_name: toolName,
        preview: summarizeSsePayload(
          evType,
          toolName,
          typeof p.payload_json === 'string' ? p.payload_json : undefined,
        ),
      };
    }

    if (kind === 'usage_recorded') {
      const status = String(p.status ?? 'ok');
      const inTok = Number(p.input_tokens ?? 0);
      const outTok = Number(p.output_tokens ?? 0);
      const cost = Number(p.cost_usd ?? 0);
      const errMsg = typeof p.error_message === 'string' ? p.error_message : null;
      return {
        id: `sse_usg_${seq}`,
        kind,
        timestamp: ts,
        source_client: 'distill',
        project_id: null,
        project_name: null,
        session_id: typeof p.session_id === 'string' ? p.session_id : null,
        event_type: `distill_${status}`,
        tool_name: null,
        status,
        tokens: inTok + outTok,
        cost_usd: cost,
        preview:
          status === 'ok'
            ? `Distilled turn (${(inTok + outTok).toLocaleString()} tok · $${cost.toFixed(4)})`
            : status === 'budget_blocked'
              ? 'Blocked by daily budget cap (zero silent fallback)'
              : `Distill error: ${errMsg ?? 'provider failure'}`,
      };
    }

    if (kind === 'memory_created' || kind === 'memory_updated' || kind === 'memory_deleted') {
      return {
        id: `sse_mem_${seq}`,
        kind,
        timestamp: ts,
        source_client: String(p.source_client ?? 'ui'),
        project_id: typeof p.project_id === 'string' ? p.project_id : null,
        project_name: typeof p.project_name === 'string' ? p.project_name : null,
        session_id: typeof p.session_id === 'string' ? p.session_id : null,
        event_type: kind,
        tool_name: null,
        memory_id: typeof p.id === 'string' ? p.id : undefined,
        preview:
          kind === 'memory_deleted'
            ? `Deleted memory ${String(p.id ?? '')}`
            : `${kind === 'memory_created' ? 'Created' : 'Updated'}: ${String(p.title ?? p.id ?? '')}`,
      };
    }

    return null;
  } catch {
    return null;
  }
}

export function App() {
  const [tab, setTabState] = useState<Tab>(readTabFromUrl);
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [memories, setMemories] = useState<MemoryHit[]>([]);
  const [connections, setConnections] = useState<ClientConnection[]>([]);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [daemon, setDaemon] = useState<DaemonInfo | null>(null);
  const [liveEvents, setLiveEvents] = useState<LiveStreamItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sseConnected, setSseConnected] = useState(false);
  const [daemonStopped, setDaemonStopped] = useState(false);
  const [checkedOnboardingRedirect, setCheckedOnboardingRedirect] = useState(false);

  const sseSeqRef = useRef(0);

  const setTab = useCallback(
    (next: Tab, replace = false, queryParams?: Record<string, string>) => {
      setTabState(next);
      const url = new URL(window.location.href);
      url.pathname = next === 'stream' ? '/' : `/${next}`;
      url.search = '';
      if (queryParams) {
        for (const [k, v] of Object.entries(queryParams)) {
          if (v) url.searchParams.set(k, v);
        }
      }
      if (replace) {
        window.history.replaceState({}, '', url);
      } else {
        window.history.pushState({}, '', url);
      }
    },
    [],
  );

  useEffect(() => {
    const onPop = () => setTabState(readTabFromUrl());
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, []);

  useEffect(() => {
    if (tab !== 'onboarding') {
      document.title = TAB_TITLES[tab];
    }
  }, [tab]);

  const refresh = useCallback(async () => {
    try {
      const [sum, proj, mem, cfg, dm, conns] = await Promise.all([
        api.summary(),
        api.projects(),
        api.searchMemories({ limit: 80 }),
        api.settings(),
        api.daemon(),
        api.connections(),
      ]);
      setSummary(sum);
      setProjects(proj.projects);
      setMemories(mem.memories);
      setSettings(cfg);
      setDaemon(dm);
      setConnections(conns.connections);
      setError(null);
      setDaemonStopped(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load daemon data');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!checkedOnboardingRedirect && settings) {
      setCheckedOnboardingRedirect(true);
      if (!settings.onboarding_completed && tab !== 'onboarding') {
        setTab('onboarding', true);
      }
    }
  }, [checkedOnboardingRedirect, settings, tab, setTab]);

  useEffect(() => {
    void refresh();
    const es = new EventSource('/api/stream');
    let refreshTimer: number | null = null;

    es.onopen = () => {
      setSseConnected(true);
      setDaemonStopped(false);
    };
    es.onerror = () => setSseConnected(false);

    const scheduleRefresh = () => {
      if (refreshTimer !== null) window.clearTimeout(refreshTimer);
      refreshTimer = window.setTimeout(() => {
        refreshTimer = null;
        void refresh();
      }, 250);
    };

    const kinds: LiveStreamItem['kind'][] = [
      'memory_created',
      'memory_updated',
      'memory_deleted',
      'usage_recorded',
      'connection_changed',
      'raw_event',
    ];
    for (const kind of kinds) {
      es.addEventListener(kind, (ev) => {
        if (ev instanceof MessageEvent && typeof ev.data === 'string') {
          sseSeqRef.current += 1;
          const item = parseSseItem(kind, ev.data, sseSeqRef.current);
          if (item) {
            setLiveEvents((prev) => [item, ...prev.filter((p) => p.id !== item.id)].slice(0, 80));
          }
        }
        scheduleRefresh();
      });
    }

    return () => {
      if (refreshTimer !== null) window.clearTimeout(refreshTimer);
      es.close();
    };
  }, [refresh]);

  const completeOnboarding = useCallback(async () => {
    try {
      const updated = await api.patchSettings({ onboarding_completed: true });
      setSettings(updated);
    } catch {
      // Even if saving fails transiently, let the user proceed to the dashboard.
    }
    setTab('stream');
    await refresh();
  }, [refresh, setTab]);

  const mismatchedClients = connections.filter((c) => c.port_mismatch);

  if (daemonStopped) {
    return (
      <div className="shell">
        <header className="topbar">
          <div className="topbar-inner">
            <span className="brand">
              <span className="brand-mark">0x1</span>
              <span>0x1-ai-memory</span>
            </span>
            <div className="topbar-meta" role="status">
              <span className="telemetry-status">
                <span className="signal-square signal-square--offline" aria-hidden="true" />
                <span>Daemon stopped</span>
              </span>
            </div>
          </div>
        </header>

        <main className="main">
          <section className="panel ticks stopped-panel">
            <div className="panel-body stack">
              <div>
                <h1 className="page-title">Daemon stopped</h1>
                <p className="page-subtitle">
                  0x1-ai-memory shut down cleanly. Local hooks fail open so your coding agents are
                  never blocked.
                </p>
              </div>
              <div className="field">
                <label>Start again anytime from your terminal</label>
                <pre className="code-block">
                  {'0x1-memory\n# or\nnpx 0x1-ai-memory'}
                </pre>
              </div>
              <div className="head-actions">
                <button type="button" className="btn btn-primary" onClick={() => void refresh()}>
                  Check connection
                </button>
              </div>
            </div>
          </section>
        </main>
      </div>
    );
  }

  if (tab === 'onboarding') {
    return (
      <div className="shell">
        <header className="topbar">
          <div className="topbar-inner">
            <span className="brand">
              <span className="brand-mark">0x1</span>
              <span>0x1-ai-memory</span>
            </span>

            <div className="topbar-meta">
              <span className="measure">:{daemon?.port ?? DEFAULT_DAEMON_PORT}</span>
              <span className="topbar-sep" aria-hidden="true" />
              <button
                type="button"
                className="btn btn-ghost"
                onClick={() => void completeOnboarding()}
              >
                Skip to dashboard
              </button>
            </div>
          </div>
        </header>

        <main className="onboarding-main">
          {error ? (
            <div className="callout danger" style={{ marginBottom: 'var(--space-md)' }}>
              <span>Daemon unreachable: {error}</span>
              <button type="button" className="btn btn-danger" onClick={() => void refresh()}>
                Retry
              </button>
            </div>
          ) : null}

          <OnboardingPage
            daemon={daemon}
            daemonLoading={loading}
            settings={settings}
            onRefresh={refresh}
            onComplete={completeOnboarding}
          />
        </main>
      </div>
    );
  }

  return (
    <div className="shell">
      <header className="topbar">
        <div className="topbar-inner">
          <a
            href="/"
            className="brand"
            onClick={(e) => {
              e.preventDefault();
              setTab('stream');
            }}
          >
            <span className="brand-mark">0x1</span>
            <span>0x1-ai-memory</span>
          </a>

          <nav className="nav" aria-label="Primary">
            <button
              type="button"
              className="nav-btn"
              aria-current={tab === 'stream' ? 'page' : undefined}
              onClick={() => setTab('stream')}
            >
              <IconStream /> Live stream
            </button>
            <button
              type="button"
              className="nav-btn"
              aria-current={tab === 'memories' ? 'page' : undefined}
              onClick={() => setTab('memories')}
            >
              <IconMemory /> Memories
            </button>
            <button
              type="button"
              className="nav-btn"
              aria-current={tab === 'connections' ? 'page' : undefined}
              onClick={() => setTab('connections')}
            >
              <IconPlug /> Connections
            </button>
            <button
              type="button"
              className="nav-btn"
              aria-current={tab === 'engine' ? 'page' : undefined}
              onClick={() => setTab('engine')}
            >
              <IconEngine /> Engine &amp; budget
            </button>
          </nav>

          <div className="topbar-meta" role="status">
            <span className="measure">${(summary?.today_cost_usd ?? 0).toFixed(4)} USD</span>
            <span className="topbar-sep" aria-hidden="true" />
            <span className="telemetry-status">
              <span
                className={`signal-square${sseConnected ? '' : ' signal-square--offline'}`}
                aria-hidden="true"
              />
              <span className={sseConnected ? 'telemetry-live' : 'telemetry-offline'}>
                {sseConnected ? 'SSE live' : 'SSE offline'}
              </span>
            </span>
          </div>
        </div>
      </header>

      <main className="main">
        {error ? (
          <div className="callout danger" style={{ marginBottom: 'var(--space-md)' }}>
            <span>Daemon unreachable: {error}</span>
            <button type="button" className="btn btn-danger" onClick={() => void refresh()}>
              Retry
            </button>
          </div>
        ) : null}

        {mismatchedClients.length > 0 && tab !== 'connections' ? (
          <div className="callout warn" style={{ marginBottom: 'var(--space-md)' }} role="status">
            <span>
              Port mismatch detected on {mismatchedClients.map((c) => c.display_name).join(', ')} —
              reconnect to update configs to :{daemon?.port ?? DEFAULT_DAEMON_PORT}.
            </span>
            <button type="button" className="btn" onClick={() => setTab('connections')}>
              Reconnect in Connections
            </button>
          </div>
        ) : null}

        {tab === 'stream' ? (
          <StreamPage
            summary={summary}
            projects={projects}
            memories={memories}
            liveEvents={liveEvents}
            loading={loading}
            sseConnected={sseConnected}
            onRefresh={refresh}
            onNavigate={(next, params) => setTab(next, false, params)}
          />
        ) : null}
        {tab === 'memories' ? (
          <MemoriesPage projects={projects} onChanged={refresh} />
        ) : null}
        {tab === 'connections' ? (
          <ConnectionsPage
            activePort={daemon?.port ?? DEFAULT_DAEMON_PORT}
            onRunOnboarding={() => setTab('onboarding')}
          />
        ) : null}
        {tab === 'engine' ? (
          <EnginePage
            daemon={daemon}
            daemonLoading={loading}
            onSaved={refresh}
            onDaemonStopped={() => {
              setDaemonStopped(true);
              setSseConnected(false);
              setError(null);
            }}
            onRunOnboarding={() => setTab('onboarding')}
          />
        ) : null}
      </main>
    </div>
  );
}
