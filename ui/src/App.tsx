import { useCallback, useEffect, useRef, useState } from 'react';
import { api, DEFAULT_DAEMON_PORT } from './api';
import { IconClose, IconEngine, IconMemory, IconPlug, IconStream } from './components/Icons';
import { UpdateModal, WhatsNewModal } from './components/UpdateModals';
import { ConnectionsPage } from './pages/ConnectionsPage';
import { EnginePage } from './pages/EnginePage';
import { MemoriesPage } from './pages/MemoriesPage';
import { OnboardingPage } from './pages/OnboardingPage';
import { StreamPage } from './pages/StreamPage';
import type {
  ClientConnection,
  DaemonInfo,
  LiveStreamItem,
  Project,
  SettingsView,
  UpdateStatusResponse,
  UsageSummary,
} from './types';

export type Tab = 'stream' | 'memories' | 'connections' | 'engine' | 'onboarding';

const TAB_TITLES: Record<Tab, string> = {
  stream: 'Live Stream | AI Memory',
  memories: 'Memories | AI Memory',
  connections: 'Connections | AI Memory',
  engine: 'Engine & Budget | AI Memory',
  onboarding: 'Onboarding | AI Memory',
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
  const [engineVisited, setEngineVisited] = useState(tab === 'engine');
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [connections, setConnections] = useState<ClientConnection[]>([]);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [daemon, setDaemon] = useState<DaemonInfo | null>(null);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatusResponse | null>(null);
  const [liveEvents, setLiveEvents] = useState<LiveStreamItem[]>([]);
  const [sseMemoryTick, setSseMemoryTick] = useState(0);
  const [sseSessionTick, setSseSessionTick] = useState(0);
  const [sseTailTick, setSseTailTick] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sseConnected, setSseConnected] = useState(false);
  const [daemonStopped, setDaemonStopped] = useState(false);
  const [checkedOnboardingRedirect, setCheckedOnboardingRedirect] = useState(false);
  const [updateModalOpen, setUpdateModalOpen] = useState(false);
  const [whatsNewOpen, setWhatsNewOpen] = useState(false);
  const [whatsNewAuto, setWhatsNewAuto] = useState(false);

  const sseSeqRef = useRef(0);
  const whatsNewAutoHandledRef = useRef(false);

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
    if (tab === 'engine') setEngineVisited(true);
    if (tab !== 'onboarding') {
      document.title = TAB_TITLES[tab];
    }
  }, [tab]);

  const refresh = useCallback(async () => {
    try {
      const [sum, proj, cfg, dm, conns, upd] = await Promise.all([
        api.summary(),
        api.projects(),
        api.settings(),
        api.daemon(),
        api.connections(),
        api.updateStatus().catch(() => null),
      ]);
      setSummary(sum);
      setProjects(proj.projects);
      setSettings(cfg);
      setDaemon(dm);
      setConnections(conns.connections);
      if (upd) setUpdateStatus(upd);
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
    if (
      tab !== 'onboarding' &&
      !whatsNewAutoHandledRef.current &&
      updateStatus?.whats_new.should_show
    ) {
      whatsNewAutoHandledRef.current = true;
      setWhatsNewAuto(true);
      setWhatsNewOpen(true);
    }
  }, [tab, updateStatus]);

  useEffect(() => {
    void refresh();
    const es = new EventSource('/api/stream');
    const timers = new Map<string, number>();
    let opened = false;
    // Events that arrived while the tab was hidden; one resync replaces them all.
    let stale = false;

    const debounce = (key: string, fn: () => void) => {
      window.clearTimeout(timers.get(key));
      timers.set(key, window.setTimeout(fn, 300));
    };
    const resync = () => {
      if (document.hidden) {
        stale = true;
        return;
      }
      stale = false;
      void refresh();
      setSseMemoryTick((t) => t + 1);
      setSseSessionTick((t) => t + 1);
      setSseTailTick((t) => t + 1);
    };
    const onVisibility = () => {
      if (stale) resync();
    };
    document.addEventListener('visibilitychange', onVisibility);

    es.onopen = () => {
      setSseConnected(true);
      setDaemonStopped(false);
      // A reconnect means events were missed while the stream was down.
      if (opened) resync();
      opened = true;
    };
    es.onerror = () => setSseConnected(false);

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
        let item: LiveStreamItem | null = null;
        if (ev instanceof MessageEvent && typeof ev.data === 'string') {
          sseSeqRef.current += 1;
          item = parseSseItem(kind, ev.data, sseSeqRef.current);
          const live = item;
          if (live) {
            setLiveEvents((prev) => [live, ...prev.filter((p) => p.id !== live.id)].slice(0, 80));
          }
        }
        if (document.hidden) {
          stale = true;
          return;
        }
        const memoryKind =
          kind === 'memory_created' || kind === 'memory_updated' || kind === 'memory_deleted';
        if (memoryKind) setSseMemoryTick((t) => t + 1);
        if (kind === 'raw_event' || kind === 'usage_recorded' || kind === 'memory_created') {
          setSseSessionTick((t) => t + 1);
          setSseTailTick((t) => t + 1);
        }
        if (memoryKind || kind === 'usage_recorded' || item?.event_type === 'distill_result') {
          debounce('summary', () => {
            api.summary().then(setSummary, () => undefined);
          });
        }
        if (kind === 'connection_changed') {
          debounce('connections', () => {
            api.connections().then((c) => setConnections(c.connections), () => undefined);
          });
        }
      });
    }

    es.addEventListener('update_status', (ev) => {
      if (ev instanceof MessageEvent && typeof ev.data === 'string') {
        try {
          const outer = JSON.parse(ev.data) as { payload?: UpdateStatusResponse };
          if (outer.payload) setUpdateStatus(outer.payload);
        } catch {
          // ignore malformed update status event
        }
      }
    });

    return () => {
      for (const t of timers.values()) window.clearTimeout(t);
      document.removeEventListener('visibilitychange', onVisibility);
      es.close();
    };
  }, [refresh]);

  const handleCloseWhatsNew = useCallback(async () => {
    setWhatsNewOpen(false);
    if (updateStatus?.whats_new.should_show) {
      try {
        const next = await api.ackWhatsNew();
        setUpdateStatus(next);
      } catch {
        setUpdateStatus((prev) =>
          prev ? { ...prev, whats_new: { ...prev.whats_new, should_show: false } } : prev,
        );
      }
    }
  }, [updateStatus]);

  const handleOpenWhatsNew = useCallback(async () => {
    setWhatsNewAuto(false);
    setWhatsNewOpen(true);
    if (updateStatus && !updateStatus.whats_new.body) {
      try {
        const next = await api.fetchWhatsNew();
        setUpdateStatus(next);
      } catch {
        // Offline fallback handles missing release notes inside the modal
      }
    }
  }, [updateStatus]);

  const handleCheckUpdates = useCallback(async () => {
    const next = await api.checkUpdates();
    setUpdateStatus(next);
  }, []);

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
  const showUpdatePill = Boolean(
    updateStatus?.update_available &&
      updateStatus.latest_release &&
      updateStatus.dismissed_version !== updateStatus.latest_release.version,
  );

  if (daemonStopped) {
    return (
      <div className="shell">
        <header className="topbar">
          <div className="topbar-inner">
            <span className="brand">
              <span className="brand-mark">0x1</span>
              <span>AI Memory</span>
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
                  AI Memory shut down cleanly. Local hooks fail open so your coding agents are
                  never blocked.
                </p>
              </div>
              <div className="field">
                <span className="field-label">Start again anytime from your terminal</span>
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
              <span>AI Memory</span>
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
            <span>AI Memory</span>
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

          <div className="topbar-meta">
            {showUpdatePill && updateStatus?.latest_release ? (
              <>
                <span className="topbar-update-pill">
                  <button
                    type="button"
                    className="topbar-update-btn"
                    onClick={() => setUpdateModalOpen(true)}
                  >
                    Update v{updateStatus.latest_release.version}
                  </button>
                  <button
                    type="button"
                    className="topbar-update-dismiss"
                    aria-label={`Dismiss update v${updateStatus.latest_release.version}`}
                    onClick={() => {
                      const ver = updateStatus.latest_release?.version;
                      if (!ver) return;
                      void api
                        .dismissUpdate(ver)
                        .then(setUpdateStatus)
                        .catch(() => {});
                    }}
                  >
                    <IconClose size={11} />
                  </button>
                </span>
                <span className="topbar-sep" aria-hidden="true" />
              </>
            ) : null}
            <span className="measure" title="Recorded cost today (UTC)">
              ${(summary?.today_cost_usd ?? 0).toFixed(4)} USD
            </span>
            <span className="topbar-sep" aria-hidden="true" />
            <span className="telemetry-status" role="status">
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
            liveEvents={liveEvents}
            sseMemoryTick={sseMemoryTick}
            sseSessionTick={sseSessionTick}
            sseTailTick={sseTailTick}
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
        {tab === 'engine' || engineVisited ? (
          <div hidden={tab !== 'engine'}>
            <EnginePage
              active={tab === 'engine'}
              summary={summary}
              daemon={daemon}
              daemonLoading={loading}
              updateStatus={updateStatus}
              onCheckUpdates={handleCheckUpdates}
              onOpenUpdateModal={() => setUpdateModalOpen(true)}
              onOpenWhatsNew={() => void handleOpenWhatsNew()}
              onSaved={refresh}
              onDaemonStopped={() => {
                setDaemonStopped(true);
                setSseConnected(false);
                setError(null);
              }}
              onRunOnboarding={() => setTab('onboarding')}
            />
          </div>
        ) : null}
      </main>

      <UpdateModal
        open={updateModalOpen}
        status={updateStatus}
        daemon={daemon}
        onClose={() => setUpdateModalOpen(false)}
        onStatusChange={setUpdateStatus}
      />

      <WhatsNewModal
        open={whatsNewOpen}
        whatsNew={updateStatus?.whats_new ?? null}
        autoTriggered={whatsNewAuto}
        onClose={() => void handleCloseWhatsNew()}
        onRetryFetch={async () => {
          const next = await api.fetchWhatsNew();
          setUpdateStatus(next);
        }}
      />
    </div>
  );
}
