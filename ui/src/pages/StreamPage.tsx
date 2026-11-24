import { useState, type FormEvent } from 'react';
import { api, formatUtcTime } from '../api';
import { ModalDialog } from '../components/ModalDialog';
import type { MemoryHit, Project, UsageSummary } from '../types';
import type { Tab } from '../App';

interface StreamPageProps {
  summary: UsageSummary | null;
  projects: Project[];
  memories: MemoryHit[];
  loading: boolean;
  sseConnected: boolean;
  onRefresh: () => Promise<void>;
  onNavigate: (tab: Tab) => void;
}

export function StreamPage({
  summary,
  projects,
  memories,
  loading,
  sseConnected,
  onRefresh,
  onNavigate,
}: StreamPageProps) {
  const [projectFilter, setProjectFilter] = useState('');
  const [clientFilter, setClientFilter] = useState('');
  const [saveOpen, setSaveOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const [title, setTitle] = useState('');
  const [summaryText, setSummaryText] = useState('');
  const [content, setContent] = useState('');
  const [memType, setMemType] = useState('decision');
  const [scope, setScope] = useState<'project' | 'global'>('project');
  const [targetProject, setTargetProject] = useState('');

  const tokenPct =
    summary && summary.daily_token_cap > 0
      ? Math.min(100, Math.round((summary.today_total_tokens / summary.daily_token_cap) * 100))
      : 0;
  const costPct =
    summary && summary.daily_cost_cap_usd > 0
      ? Math.min(100, Math.round((summary.today_cost_usd / summary.daily_cost_cap_usd) * 100))
      : 0;
  const budgetPct = Math.max(tokenPct, costPct);
  const meterClass = budgetPct >= 90 ? 'danger' : budgetPct >= 70 ? 'warn' : '';

  const filtered = memories.filter((m) => {
    if (projectFilter && m.project_id !== projectFilter && m.scope !== 'global') return false;
    if (clientFilter && m.source_client !== clientFilter) return false;
    return true;
  });

  const activeProjectObj = projects.find((p) => p.id === projectFilter);

  async function handleSave(e: FormEvent) {
    e.preventDefault();
    if (!title.trim() || !summaryText.trim() || !content.trim()) return;
    setSaving(true);
    setSaveError(null);
    try {
      await api.createMemory({
        scope,
        project_id:
          scope === 'project' ? targetProject || projectFilter || projects[0]?.id || null : null,
        memory_type: memType,
        title: title.trim(),
        summary: summaryText.trim(),
        content: content.trim(),
      });
      setTitle('');
      setSummaryText('');
      setContent('');
      setSaveOpen(false);
      await onRefresh();
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : 'Failed to save memory');
    } finally {
      setSaving(false);
    }
  }

  async function handlePrivacyChange(projectId: string, mode: string) {
    await api.setProjectPrivacy(projectId, mode);
    await onRefresh();
  }

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-title">Live cross-agent stream</h1>
          <p className="page-subtitle">
            Unified local memory shared by Claude Code, Codex, Antigravity, and T3 Code.
          </p>
        </div>
        <div className="head-actions">
          <button type="button" className="btn" onClick={() => onNavigate('connections')}>
            Connect AI clients
          </button>
          <button type="button" className="btn btn-primary" onClick={() => setSaveOpen(true)}>
            Save memory
          </button>
        </div>
      </div>

      {/* Signature Rim telemetry strip with corner ticks and tabular measurements */}
      <section className="metrics-strip ticks" aria-label="Live telemetry and budget meter">
        <div className="metric-cell">
          <span className="metric-label">Daily spend</span>
          <div className="metric-value">
            ${(summary?.today_cost_usd ?? 0).toFixed(4)}
            <span className="metric-unit">USD</span>
          </div>
          <span className="metric-sub measure">
            {(summary?.today_total_tokens ?? 0).toLocaleString()} /{' '}
            {(summary?.daily_token_cap ?? 250000).toLocaleString()} tok (cap $
            {(summary?.daily_cost_cap_usd ?? 0.5).toFixed(2)})
          </span>
          <div className="meter-track" aria-label={`Daily budget used: ${budgetPct}%`}>
            <div className={`meter-fill ${meterClass}`} style={{ width: `${budgetPct}%` }} />
          </div>
        </div>

        <div className="metric-cell">
          <span className="metric-label">Memory engine</span>
          <div className="metric-value">{summary?.active_provider ?? 'none'}</div>
          <span className="metric-sub">
            {summary?.active_provider === 'none'
              ? 'Zero-LLM safe mode ($0)'
              : `${summary?.active_model} (zero silent fallback)`}
          </span>
        </div>

        <div className="metric-cell">
          <span className="metric-label">Active memories</span>
          <div className="metric-value">{summary?.total_memories ?? 0}</div>
          <span className="metric-sub measure">
            {summary?.total_projects ?? 0} projects, {summary?.pending_raw_events ?? 0} raw events
            queued
          </span>
        </div>

        <div className="metric-cell">
          <span className="metric-label">Stream and vectors</span>
          <div className="metric-value metric-value--signal">
            {sseConnected ? 'Live' : 'Offline'}
          </div>
          <span className="metric-sub">{summary?.embedding_status ?? 'fts5'}</span>
        </div>
      </section>

      {summary?.budget_exhausted ? (
        <div className="callout danger" style={{ marginBottom: 'var(--space-md)' }}>
          <span>
            Daily budget cap reached ({summary.today_blocked} distillation calls blocked, zero
            silent fallback to coding subscriptions).
          </span>
          <button type="button" className="btn" onClick={() => onNavigate('engine')}>
            Adjust budget
          </button>
        </div>
      ) : null}

      <div className="toolbar">
        <select
          className="select"
          aria-label="Filter by project"
          value={projectFilter}
          onChange={(e) => setProjectFilter(e.target.value)}
        >
          <option value="">All projects</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} ({p.privacy_mode})
            </option>
          ))}
        </select>

        <select
          className="select"
          aria-label="Filter by AI client"
          value={clientFilter}
          onChange={(e) => setClientFilter(e.target.value)}
        >
          <option value="">All AI clients</option>
          <option value="claude-code">Claude Code</option>
          <option value="codex">Codex</option>
          <option value="antigravity">Antigravity</option>
          <option value="t3-code">T3 Code</option>
          <option value="mcp">MCP</option>
          <option value="ui">Manual UI</option>
        </select>

        {activeProjectObj ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-xs)', marginLeft: 'auto' }}>
            <label htmlFor="proj-privacy" className="field-hint">
              Project privacy
            </label>
            <select
              id="proj-privacy"
              className="select"
              value={activeProjectObj.privacy_mode}
              onChange={(e) => handlePrivacyChange(activeProjectObj.id, e.target.value)}
            >
              <option value="full">Full (distill with active engine)</option>
              <option value="local_only">Local-only (never call external API)</option>
              <option value="paused">Paused (stop capturing)</option>
            </select>
          </div>
        ) : null}
      </div>

      <section className="panel" aria-label="Recent cross-agent memories">
        <div className="panel-header">
          <h2 className="panel-title">Recent memories ({filtered.length})</h2>
          <button type="button" className="btn btn-ghost" onClick={() => onNavigate('memories')}>
            Search all
          </button>
        </div>

        {loading ? (
          <div aria-busy="true" aria-label="Loading memories">
            {[0, 1, 2].map((i) => (
              <div key={i} className="skeleton-row">
                <div className="skeleton-bar skeleton-bar--short" />
                <div className="skeleton-bar skeleton-bar--wide" />
              </div>
            ))}
          </div>
        ) : filtered.length === 0 ? (
          <div className="empty-state">
            <p>
              No memories recorded yet. Connect Claude Code, Codex, Antigravity, or T3 Code, or save
              a decision directly.
            </p>
            <div className="head-actions">
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => onNavigate('connections')}
              >
                Open setup wizard
              </button>
              <button type="button" className="btn" onClick={() => setSaveOpen(true)}>
                Save first memory
              </button>
            </div>
          </div>
        ) : (
          <ul className="ledger">
            {filtered.map((m) => (
              <li key={m.id} className="ledger-row">
                <div className="ledger-summary">
                  <div className="ledger-client">
                    <span className={`tag ${m.source_client}`}>{m.source_client}</span>
                    <span className="field-hint">{m.memory_type}</span>
                  </div>
                  <div className="ledger-main">
                    <h3 className="ledger-title">{m.title}</h3>
                    <p className="ledger-desc">{m.summary}</p>
                    <div className="tag-row">
                      {m.status === 'pinned' ? <span className="tag accent">pinned</span> : null}
                      {m.scope === 'global' ? (
                        <span className="tag">global</span>
                      ) : m.project_name ? (
                        <span className="tag">{m.project_name}</span>
                      ) : null}
                    </div>
                  </div>
                  <div className="ledger-aside">
                    <time className="field-hint measure" dateTime={m.updated_at}>
                      {formatUtcTime(m.updated_at)}
                    </time>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <ModalDialog
        open={saveOpen}
        title="Save shared memory"
        onClose={() => setSaveOpen(false)}
        footer={
          <>
            <button type="button" className="btn" onClick={() => setSaveOpen(false)}>
              Cancel
            </button>
            <button
              type="submit"
              form="save-memory-form"
              className="btn btn-primary"
              disabled={saving}
              aria-busy={saving}
            >
              {saving ? 'Saving...' : 'Save memory'}
            </button>
          </>
        }
      >
        <form id="save-memory-form" className="stack" onSubmit={handleSave}>
          {saveError ? <div className="callout danger">{saveError}</div> : null}
          <div className="grid-2">
            <div className="field">
              <label htmlFor="sm-scope">Scope</label>
              <select
                id="sm-scope"
                className="select"
                value={scope}
                onChange={(e) => setScope(e.target.value as 'project' | 'global')}
              >
                <option value="project">Project (shared across all AI clients)</option>
                <option value="global">Global (personal preference across repos)</option>
              </select>
            </div>
            <div className="field">
              <label htmlFor="sm-type">Type</label>
              <select
                id="sm-type"
                className="select"
                value={memType}
                onChange={(e) => setMemType(e.target.value)}
              >
                <option value="decision">decision</option>
                <option value="architecture">architecture</option>
                <option value="bugfix">bugfix</option>
                <option value="preference">preference</option>
                <option value="pattern">pattern</option>
                <option value="session_summary">session_summary</option>
              </select>
            </div>
          </div>

          {scope === 'project' && projects.length > 0 ? (
            <div className="field">
              <label htmlFor="sm-project">Project</label>
              <select
                id="sm-project"
                className="select"
                value={targetProject || projectFilter || projects[0]?.id || ''}
                onChange={(e) => setTargetProject(e.target.value)}
              >
                {projects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </div>
          ) : null}

          <div className="field">
            <label htmlFor="sm-title">Title</label>
            <input
              id="sm-title"
              className="input"
              required
              placeholder="Auth tokens use httpOnly cookies"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
            />
          </div>
          <div className="field">
            <label htmlFor="sm-summary">One-line summary (shown in SessionStart briefing)</label>
            <input
              id="sm-summary"
              className="input"
              required
              placeholder="Short durable takeaway for progressive disclosure"
              value={summaryText}
              onChange={(e) => setSummaryText(e.target.value)}
            />
          </div>
          <div className="field">
            <label htmlFor="sm-content">Full details (loaded on demand via memory_get)</label>
            <textarea
              id="sm-content"
              className="textarea"
              rows={5}
              required
              placeholder="Rationale, files touched, edge cases"
              value={content}
              onChange={(e) => setContent(e.target.value)}
            />
          </div>
        </form>
      </ModalDialog>
    </div>
  );
}
