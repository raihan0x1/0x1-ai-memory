import { useState, type FormEvent } from 'react';
import { api, formatUtcTime } from '../api';
import { ModalDialog } from '../components/ModalDialog';
import { RimSelect } from '../components/RimSelect';
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

const CLIENT_OPTIONS = [
  { value: '', label: 'All AI clients' },
  { value: 'claude-code', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  { value: 'antigravity', label: 'Antigravity' },
  { value: 't3-code', label: 'T3 Code' },
  { value: 'mcp', label: 'MCP' },
  { value: 'ui', label: 'Manual UI' },
];

const PRIVACY_OPTIONS = [
  { value: 'full', label: 'Full', meta: 'distill' },
  { value: 'local_only', label: 'Local-only', meta: 'no API' },
  { value: 'paused', label: 'Paused', meta: 'off' },
];

const SCOPE_OPTIONS = [
  { value: 'project', label: 'Project (shared across all AI clients)' },
  { value: 'global', label: 'Global (personal preference across repos)' },
];

const TYPE_OPTIONS = [
  { value: 'decision', label: 'decision' },
  { value: 'architecture', label: 'architecture' },
  { value: 'bugfix', label: 'bugfix' },
  { value: 'preference', label: 'preference' },
  { value: 'pattern', label: 'pattern' },
  { value: 'session_summary', label: 'session_summary' },
];

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
  const projectFilterOptions = [
    { value: '', label: 'All projects' },
    ...projects.map((p) => ({ value: p.id, label: p.name, meta: p.privacy_mode })),
  ];
  const projectTargetOptions = projects.map((p) => ({ value: p.id, label: p.name }));

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

  const isZeroLlm = !summary || summary.active_provider === 'none';

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

      <section className="metrics-strip ticks" aria-label="Live telemetry and budget meter">
        <div className="metric-cell">
          <div className="metric-head">
            <span className="metric-label">Daily spend</span>
            <div className="metric-value">
              ${(summary?.today_cost_usd ?? 0).toFixed(4)}
              <span className="metric-unit">USD</span>
            </div>
          </div>
          <div>
            <span className="metric-sub measure">
              {(summary?.today_total_tokens ?? 0).toLocaleString()} /{' '}
              {(summary?.daily_token_cap ?? 250000).toLocaleString()} tok (cap $
              {(summary?.daily_cost_cap_usd ?? 0.5).toFixed(2)})
            </span>
            <div className="meter-row">
              <div className="meter-track" aria-label={`Daily budget used: ${budgetPct}%`}>
                <div className={`meter-fill ${meterClass}`} style={{ width: `${budgetPct}%` }} />
              </div>
              <span className="meter-pct measure">{budgetPct}%</span>
            </div>
          </div>
        </div>

        <div className="metric-cell">
          <div className="metric-head">
            <span className="metric-label">Memory engine</span>
            <div className={`metric-value${isZeroLlm ? ' metric-value--muted' : ''}`}>
              {summary?.active_provider ?? 'none'}
            </div>
          </div>
          <span className="metric-sub">
            {isZeroLlm
              ? 'Zero-LLM safe mode ($0)'
              : `${summary?.active_model} (zero silent fallback)`}
          </span>
        </div>

        <div className="metric-cell">
          <div className="metric-head">
            <span className="metric-label">Active memories</span>
            <div className="metric-value">{summary?.total_memories ?? 0}</div>
          </div>
          <span className="metric-sub measure">
            {summary?.total_projects ?? 0} projects, {summary?.pending_raw_events ?? 0} queued raw
            events
          </span>
        </div>

        <div className="metric-cell">
          <div className="metric-head">
            <span className="metric-label">Stream and vectors</span>
            <div className="metric-value metric-value--signal">
              {sseConnected ? 'Live' : 'Offline'}
            </div>
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
        <RimSelect
          ariaLabel="Filter by project"
          value={projectFilter}
          options={projectFilterOptions}
          onChange={setProjectFilter}
        />

        <RimSelect
          ariaLabel="Filter by AI client"
          value={clientFilter}
          options={CLIENT_OPTIONS}
          onChange={setClientFilter}
        />

        {activeProjectObj ? (
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 'var(--space-xs)',
              marginLeft: 'auto',
            }}
          >
            <span className="field-hint">Project privacy</span>
            <RimSelect
              id="proj-privacy"
              ariaLabel="Project privacy mode"
              value={activeProjectObj.privacy_mode}
              options={PRIVACY_OPTIONS}
              onChange={(mode) => void handlePrivacyChange(activeProjectObj.id, mode)}
            />
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
            <span className="field-hint measure">
              Local MCP ready at {window.location.origin}/mcp
            </span>
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
              <RimSelect
                id="sm-scope"
                ariaLabel="Memory scope"
                fullWidth
                value={scope}
                options={SCOPE_OPTIONS}
                onChange={(v) => setScope(v as 'project' | 'global')}
              />
            </div>
            <div className="field">
              <label htmlFor="sm-type">Type</label>
              <RimSelect
                id="sm-type"
                ariaLabel="Memory type"
                fullWidth
                value={memType}
                options={TYPE_OPTIONS}
                onChange={setMemType}
              />
            </div>
          </div>

          {scope === 'project' && projectTargetOptions.length > 0 ? (
            <div className="field">
              <label htmlFor="sm-project">Project</label>
              <RimSelect
                id="sm-project"
                ariaLabel="Target project"
                fullWidth
                value={targetProject || projectFilter || projects[0]?.id || ''}
                options={projectTargetOptions}
                onChange={setTargetProject}
              />
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
