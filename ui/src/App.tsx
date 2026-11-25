import { useCallback, useEffect, useState } from 'react';
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

export function App() {
  const [tab, setTabState] = useState<Tab>(readTabFromUrl);
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [memories, setMemories] = useState<MemoryHit[]>([]);
  const [connections, setConnections] = useState<ClientConnection[]>([]);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [daemon, setDaemon] = useState<DaemonInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sseConnected, setSseConnected] = useState(false);
  const [daemonStopped, setDaemonStopped] = useState(false);
  const [checkedOnboardingRedirect, setCheckedOnboardingRedirect] = useState(false);

  const setTab = useCallback((next: Tab, replace = false) => {
    setTabState(next);
    const url = new URL(window.location.href);
    url.pathname = next === 'stream' ? '/' : `/${next}`;
    url.searchParams.delete('tab');
    if (replace) {
      window.history.replaceState({}, '', url);
    } else {
      window.history.pushState({}, '', url);
    }
  }, []);

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
        api.searchMemories({ limit: 40 }),
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
    es.onopen = () => {
      setSseConnected(true);
      setDaemonStopped(false);
    };
    es.onerror = () => setSseConnected(false);

    const onEvent = () => {
      void refresh();
    };
    for (const kind of [
      'memory_created',
      'memory_updated',
      'memory_deleted',
      'usage_recorded',
      'connection_changed',
      'raw_event',
    ]) {
      es.addEventListener(kind, onEvent);
    }
    return () => {
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
            loading={loading}
            sseConnected={sseConnected}
            onRefresh={refresh}
            onNavigate={setTab}
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
