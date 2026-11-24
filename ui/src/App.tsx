import { useCallback, useEffect, useState } from 'react';
import { api } from './api';
import { IconEngine, IconMemory, IconPlug, IconStream } from './components/Icons';
import { ConnectionsPage } from './pages/ConnectionsPage';
import { EnginePage } from './pages/EnginePage';
import { MemoriesPage } from './pages/MemoriesPage';
import { StreamPage } from './pages/StreamPage';
import type { MemoryHit, Project, UsageSummary } from './types';

export type Tab = 'stream' | 'memories' | 'connections' | 'engine';

function readTabFromUrl(): Tab {
  const param = new URLSearchParams(window.location.search).get('tab');
  if (param === 'memories' || param === 'connections' || param === 'engine') return param;
  return 'stream';
}

export function App() {
  const [tab, setTabState] = useState<Tab>(readTabFromUrl);
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [memories, setMemories] = useState<MemoryHit[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sseConnected, setSseConnected] = useState(false);

  const setTab = useCallback((next: Tab) => {
    setTabState(next);
    const url = new URL(window.location.href);
    url.searchParams.set('tab', next);
    window.history.pushState({}, '', url);
  }, []);

  useEffect(() => {
    const onPop = () => setTabState(readTabFromUrl());
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const [sum, proj, mem] = await Promise.all([
        api.summary(),
        api.projects(),
        api.searchMemories({ limit: 40 }),
      ]);
      setSummary(sum);
      setProjects(proj.projects);
      setMemories(mem.memories);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load daemon data');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const es = new EventSource('/api/stream');
    es.onopen = () => setSseConnected(true);
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

  return (
    <div className="shell">
      <header className="topbar">
        <div className="topbar-inner">
          <a
            href="?tab=stream"
            className="brand"
            onClick={(e) => {
              e.preventDefault();
              setTab('stream');
            }}
          >
            <span className="brand-mark">0x1</span>
            <span>0x1-ai-memory</span>
          </a>

          <nav className="nav" aria-label="Primary navigation">
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
        {tab === 'connections' ? <ConnectionsPage /> : null}
        {tab === 'engine' ? <EnginePage onSaved={refresh} /> : null}
      </main>
    </div>
  );
}
