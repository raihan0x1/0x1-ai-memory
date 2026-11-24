import { useEffect, useState } from 'react';
import { api, formatUtcTime } from '../api';
import { ModalDialog } from '../components/ModalDialog';
import type { ClientConnection, ConnectionPreview, UniversalMcpSnippets } from '../types';

export function ConnectionsPage() {
  const [connections, setConnections] = useState<ClientConnection[]>([]);
  const [universal, setUniversal] = useState<UniversalMcpSnippets | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<ConnectionPreview | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [snippetTab, setSnippetTab] = useState<'http' | 'toml' | 'stdio'>('http');

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const [cRes, uRes] = await Promise.all([api.connections(), api.universalSnippets()]);
      setConnections(cRes.connections);
      setUniversal(uRes);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to scan local clients');
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function openPreview(clientId: string) {
    setFeedback(null);
    const data = await api.previewConnect(clientId);
    setPreview(data);
  }

  async function confirmConnect() {
    if (!preview) return;
    setBusyId(preview.client_id);
    try {
      const { connection } = await api.connectClient(preview.client_id);
      setPreview(null);
      setFeedback(
        `Connected ${connection.display_name}. ${connection.manual_step_hint ?? 'Ready to share memory.'}`,
      );
      await load();
    } catch (e) {
      setFeedback(`Connect failed: ${String(e)}`);
    } finally {
      setBusyId(null);
    }
  }

  async function handleDisconnect(clientId: string) {
    setBusyId(clientId);
    setFeedback(null);
    try {
      const { connection } = await api.disconnectClient(clientId);
      setFeedback(
        `Disconnected ${connection.display_name}. Config backup kept in ~/.0x1-memory/backups.`,
      );
      await load();
    } finally {
      setBusyId(null);
    }
  }

  async function handlePing(clientId: string) {
    setBusyId(clientId);
    try {
      const res = await api.pingClient(clientId);
      setFeedback(`Verified MCP endpoint at ${res.mcp_url} for ${clientId}.`);
      await load();
    } finally {
      setBusyId(null);
    }
  }

  async function copyText(text: string) {
    await navigator.clipboard.writeText(text);
    setFeedback('Copied MCP config snippet to clipboard.');
  }

  const activeSnippet =
    snippetTab === 'http'
      ? (universal?.json_http ?? '')
      : snippetTab === 'toml'
        ? (universal?.toml_http ?? '')
        : (universal?.json_stdio ?? '');

  return (
    <div className="stack">
      <div className="page-head" style={{ marginBottom: 0 }}>
        <div>
          <h1 className="page-title">AI client connections</h1>
          <p className="page-subtitle">
            Auto-detected local clients. Preview exact config diffs and timestamped backups before
            connecting.
          </p>
        </div>
        <button type="button" className="btn" onClick={() => void load()}>
          Re-scan machine
        </button>
      </div>

      {error ? (
        <div className="callout danger">
          <span>Client scan error: {error}</span>
          <button type="button" className="btn btn-danger" onClick={() => void load()}>
            Retry
          </button>
        </div>
      ) : null}

      {feedback ? <div className="callout ok">{feedback}</div> : null}

      {loading ? (
        <div className="grid-2" aria-busy="true">
          {[0, 1, 2, 3].map((i) => (
            <div key={i} className="panel">
              <div className="skeleton-row">
                <div className="skeleton-bar skeleton-bar--short" />
                <div className="skeleton-bar skeleton-bar--wide" />
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="grid-2">
          {connections.map((c) => (
            <section key={c.client_id} className="panel">
              <div className="panel-header">
                <div className="tag-row">
                  <h2 className="panel-title">{c.display_name}</h2>
                  <span className={`tag ${c.client_id}`}>{c.client_id}</span>
                </div>
                <span className={`tag ${c.connected ? 'ok' : c.detected ? 'warn' : ''}`}>
                  {c.connected ? 'Connected' : c.detected ? 'Detected' : 'Not found'}
                </span>
              </div>

              <div className="panel-body stack-sm">
                <p style={{ fontSize: 'var(--text-sm)', color: 'var(--color-ink-2)' }}>
                  {c.status_note}
                </p>

                {c.manual_step_hint ? (
                  <div className="callout warn">{c.manual_step_hint}</div>
                ) : null}

                <div className="stack-sm" style={{ gap: 'var(--space-2xs)' }}>
                  <span className="field-hint measure">
                    Heartbeat: {formatUtcTime(c.last_heartbeat_at)}
                  </span>
                  <span className="field-hint measure">
                    Last event: {formatUtcTime(c.last_event_at)}
                  </span>
                </div>

                <div className="head-actions" style={{ marginTop: 'var(--space-xs)' }}>
                  {!c.connected ? (
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={busyId === c.client_id}
                      aria-busy={busyId === c.client_id}
                      onClick={() => void openPreview(c.client_id)}
                    >
                      Preview and connect
                    </button>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="btn"
                        disabled={busyId === c.client_id}
                        onClick={() => void openPreview(c.client_id)}
                      >
                        Re-sync
                      </button>
                      <button
                        type="button"
                        className="btn"
                        disabled={busyId === c.client_id}
                        onClick={() => void handlePing(c.client_id)}
                      >
                        Test ping
                      </button>
                      <button
                        type="button"
                        className="btn btn-danger"
                        disabled={busyId === c.client_id}
                        onClick={() => void handleDisconnect(c.client_id)}
                      >
                        Disconnect
                      </button>
                    </>
                  )}
                </div>
              </div>
            </section>
          ))}
        </div>
      )}

      <section className="panel ticks">
        <div className="panel-header">
          <div>
            <h2 className="panel-title">Universal MCP (any other AI or IDE)</h2>
            <p className="field-hint">
              Connect Cursor, Windsurf, OpenCode, Cline, or custom agents to{' '}
              <span className="measure">{universal?.streamable_http_url}</span>
            </p>
          </div>
          <div className="head-actions">
            <button
              type="button"
              className={`btn ${snippetTab === 'http' ? 'btn-primary' : ''}`}
              onClick={() => setSnippetTab('http')}
            >
              HTTP JSON
            </button>
            <button
              type="button"
              className={`btn ${snippetTab === 'toml' ? 'btn-primary' : ''}`}
              onClick={() => setSnippetTab('toml')}
            >
              TOML
            </button>
            <button
              type="button"
              className={`btn ${snippetTab === 'stdio' ? 'btn-primary' : ''}`}
              onClick={() => setSnippetTab('stdio')}
            >
              Stdio fallback
            </button>
            <button type="button" className="btn" onClick={() => void copyText(activeSnippet)}>
              Copy
            </button>
          </div>
        </div>
        <div className="panel-body">
          <pre className="code-block">{activeSnippet}</pre>
        </div>
      </section>

      <ModalDialog
        open={Boolean(preview)}
        title={preview ? `Connect ${preview.display_name}` : 'Connect client'}
        onClose={() => setPreview(null)}
        footer={
          <>
            <button type="button" className="btn" onClick={() => setPreview(null)}>
              Cancel
            </button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busyId !== null}
              aria-busy={busyId !== null}
              onClick={() => void confirmConnect()}
            >
              {busyId ? 'Applying...' : 'Backup and connect'}
            </button>
          </>
        }
      >
        {preview ? (
          <div className="stack">
            <p>{preview.summary}</p>
            <div className="field">
              <label>Planned changes (automatic backup saved to ~/.0x1-memory/backups/)</label>
              <pre className="code-block">{preview.diff_preview}</pre>
            </div>
            {preview.manual_step_hint ? (
              <div className="callout warn">{preview.manual_step_hint}</div>
            ) : null}
          </div>
        ) : null}
      </ModalDialog>
    </div>
  );
}
