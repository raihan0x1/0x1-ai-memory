import { useEffect, useState } from 'react';
import { api, formatUtcTime } from '../api';
import { ModalDialog } from '../components/ModalDialog';
import type { ContextPreviewResponse, MemoryDetail, MemoryHit, Project } from '../types';

interface MemoriesPageProps {
  projects: Project[];
  onChanged: () => Promise<void>;
}

function readParam(name: string): string {
  return new URLSearchParams(window.location.search).get(name) ?? '';
}

export function MemoriesPage({ projects, onChanged }: MemoriesPageProps) {
  const [query, setQuery] = useState(() => readParam('q'));
  const [projectId, setProjectId] = useState(() => readParam('project'));
  const [memType, setMemType] = useState(() => readParam('type'));
  const [sourceClient, setSourceClient] = useState(() => readParam('client'));
  const [includeInactive, setIncludeInactive] = useState(false);

  const [hits, setHits] = useState<MemoryHit[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Inline disclosure & editing (modal is last resort in Hallmark app screens)
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [expandedDetail, setExpandedDetail] = useState<MemoryDetail | null>(null);
  const [editTitle, setEditTitle] = useState('');
  const [editSummary, setEditSummary] = useState('');
  const [editContent, setEditContent] = useState('');
  const [savingEdit, setSavingEdit] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const [previewOpen, setPreviewOpen] = useState(false);
  const [preview, setPreview] = useState<ContextPreviewResponse | null>(null);

  // Sync filter state to URL
  useEffect(() => {
    const url = new URL(window.location.href);
    const entries: [string, string][] = [
      ['q', query.trim()],
      ['project', projectId],
      ['type', memType],
      ['client', sourceClient],
    ];
    for (const [k, v] of entries) {
      if (v) url.searchParams.set(k, v);
      else url.searchParams.delete(k);
    }
    window.history.replaceState({}, '', url);
  }, [query, projectId, memType, sourceClient]);

  async function runSearch() {
    setLoading(true);
    setError(null);
    try {
      const res = await api.searchMemories({
        query: query.trim() || undefined,
        project_id: projectId || undefined,
        memory_type: memType || undefined,
        source_client: sourceClient || undefined,
        include_inactive: includeInactive,
        limit: 50,
      });
      setHits(res.memories);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Search failed');
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    const timer = setTimeout(() => {
      void runSearch();
    }, 150);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, projectId, memType, sourceClient, includeInactive]);

  async function toggleExpand(id: string) {
    setConfirmDeleteId(null);
    if (expandedId === id) {
      setExpandedId(null);
      setExpandedDetail(null);
      return;
    }
    setExpandedId(id);
    setExpandedDetail(null);
    const { memory } = await api.getMemory(id);
    setExpandedDetail(memory);
    setEditTitle(memory.title);
    setEditSummary(memory.summary);
    setEditContent(memory.content);
  }

  async function saveInlineEdit() {
    if (!expandedDetail) return;
    setSavingEdit(true);
    try {
      const { memory } = await api.updateMemory(expandedDetail.id, {
        title: editTitle,
        summary: editSummary,
        content: editContent,
      });
      setExpandedDetail(memory);
      await Promise.all([runSearch(), onChanged()]);
    } finally {
      setSavingEdit(false);
    }
  }

  async function toggleStatus(hit: MemoryHit, nextStatus: string) {
    await api.updateMemory(hit.id, { status: nextStatus });
    if (expandedDetail?.id === hit.id) {
      setExpandedDetail({ ...expandedDetail, status: nextStatus as MemoryDetail['status'] });
    }
    await Promise.all([runSearch(), onChanged()]);
  }

  async function removeMemory(id: string) {
    await api.deleteMemory(id);
    setConfirmDeleteId(null);
    if (expandedId === id) {
      setExpandedId(null);
      setExpandedDetail(null);
    }
    await Promise.all([runSearch(), onChanged()]);
  }

  async function openContextPreview() {
    const data = await api.contextPreview(projectId || projects[0]?.id);
    setPreview(data);
    setPreviewOpen(true);
  }

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-title">Memories and hybrid search</h1>
          <p className="page-subtitle">
            FTS5 BM25 and sqlite-vec cosine search ranked with Reciprocal Rank Fusion. Click any row
            to inspect or edit inline.
          </p>
        </div>
        <button type="button" className="btn btn-primary" onClick={openContextPreview}>
          Briefing preview
        </button>
      </div>

      <div className="toolbar">
        <input
          type="search"
          className="input"
          aria-label="Search memories"
          style={{ flex: '1 1 260px' }}
          placeholder="Search decisions, bugfixes, files, or concepts"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <select
          className="select"
          aria-label="Filter by project"
          value={projectId}
          onChange={(e) => setProjectId(e.target.value)}
        >
          <option value="">All projects</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <select
          className="select"
          aria-label="Filter by type"
          value={memType}
          onChange={(e) => setMemType(e.target.value)}
        >
          <option value="">All types</option>
          <option value="decision">decision</option>
          <option value="architecture">architecture</option>
          <option value="bugfix">bugfix</option>
          <option value="preference">preference</option>
          <option value="pattern">pattern</option>
          <option value="session_summary">session_summary</option>
        </select>
        <select
          className="select"
          aria-label="Filter by AI client"
          value={sourceClient}
          onChange={(e) => setSourceClient(e.target.value)}
        >
          <option value="">All clients</option>
          <option value="claude-code">Claude Code</option>
          <option value="codex">Codex</option>
          <option value="antigravity">Antigravity</option>
          <option value="t3-code">T3 Code</option>
        </select>
        <label style={{ display: 'inline-flex', alignItems: 'center', gap: 'var(--space-xs)', fontSize: 'var(--text-xs)' }}>
          <input
            type="checkbox"
            checked={includeInactive}
            onChange={(e) => setIncludeInactive(e.target.checked)}
          />
          Include superseded and archived
        </label>
      </div>

      {error ? (
        <div className="callout danger" style={{ marginBottom: 'var(--space-md)' }}>
          <span>Search error: {error}</span>
          <button type="button" className="btn btn-danger" onClick={() => void runSearch()}>
            Retry
          </button>
        </div>
      ) : null}

      <section className="panel" aria-label="Memory search results">
        <div className="panel-header">
          <h2 className="panel-title">
            {loading ? 'Searching...' : `${hits.length} memories`}
          </h2>
          <span className="field-hint">Inline editor with instant re-indexing</span>
        </div>

        {loading ? (
          <div aria-busy="true">
            {[0, 1, 2, 3].map((i) => (
              <div key={i} className="skeleton-row">
                <div className="skeleton-bar skeleton-bar--short" />
                <div className="skeleton-bar skeleton-bar--wide" />
              </div>
            ))}
          </div>
        ) : hits.length === 0 ? (
          <div className="empty-state">
            <p>No memories match the current query or filter.</p>
            {query || projectId || memType || sourceClient ? (
              <button
                type="button"
                className="btn"
                onClick={() => {
                  setQuery('');
                  setProjectId('');
                  setMemType('');
                  setSourceClient('');
                }}
              >
                Clear filters
              </button>
            ) : null}
          </div>
        ) : (
          <ul className="ledger">
            {hits.map((m) => {
              const isOpen = expandedId === m.id;
              return (
                <li key={m.id} className="ledger-row" data-expanded={isOpen}>
                  <div className="ledger-summary">
                    <div className="ledger-client">
                      <span className={`tag ${m.source_client}`}>{m.source_client}</span>
                      <span className="field-hint">{m.memory_type}</span>
                    </div>

                    <div className="ledger-main">
                      <h3 className="ledger-title">{m.title}</h3>
                      <p className="ledger-desc">{m.summary}</p>
                      <div className="tag-row">
                        <span
                          className={`tag ${
                            m.status === 'pinned'
                              ? 'accent'
                              : m.status === 'superseded'
                                ? 'warn'
                                : ''
                          }`}
                        >
                          {m.status}
                        </span>
                        {m.project_name ? (
                          <span className="tag">{m.project_name}</span>
                        ) : (
                          <span className="tag">global</span>
                        )}
                        <span className="field-hint measure">{m.id}</span>
                      </div>
                    </div>

                    <div className="ledger-aside">
                      <time className="field-hint measure" dateTime={m.updated_at}>
                        {formatUtcTime(m.updated_at)}
                      </time>
                      <div className="tag-row">
                        <button
                          type="button"
                          className="btn btn-ghost"
                          aria-expanded={isOpen}
                          onClick={() => void toggleExpand(m.id)}
                        >
                          {isOpen ? 'Collapse' : 'Inspect'}
                        </button>
                        <button
                          type="button"
                          className="btn btn-ghost"
                          onClick={() =>
                            void toggleStatus(m, m.status === 'pinned' ? 'active' : 'pinned')
                          }
                        >
                          {m.status === 'pinned' ? 'Unpin' : 'Pin'}
                        </button>
                      </div>
                    </div>
                  </div>

                  {isOpen ? (
                    <div className="ledger-detail">
                      {!expandedDetail ? (
                        <div className="skeleton-bar skeleton-bar--wide" />
                      ) : (
                        <>
                          <div className="grid-2">
                            <div className="field">
                              <label htmlFor={`ed-t-${m.id}`}>Title</label>
                              <input
                                id={`ed-t-${m.id}`}
                                className="input"
                                value={editTitle}
                                onChange={(e) => setEditTitle(e.target.value)}
                              />
                            </div>
                            <div className="field">
                              <label htmlFor={`ed-s-${m.id}`}>Summary</label>
                              <input
                                id={`ed-s-${m.id}`}
                                className="input"
                                value={editSummary}
                                onChange={(e) => setEditSummary(e.target.value)}
                              />
                            </div>
                          </div>

                          <div className="field">
                            <label htmlFor={`ed-c-${m.id}`}>Full content</label>
                            <textarea
                              id={`ed-c-${m.id}`}
                              className="textarea"
                              rows={6}
                              value={editContent}
                              onChange={(e) => setEditContent(e.target.value)}
                            />
                          </div>

                          {expandedDetail.files.length > 0 ? (
                            <div className="field-hint measure">
                              Files: {expandedDetail.files.join(', ')}
                            </div>
                          ) : null}

                          <div className="head-actions" style={{ justifyContent: 'space-between' }}>
                            <div className="head-actions">
                              <button
                                type="button"
                                className="btn btn-primary"
                                disabled={savingEdit}
                                aria-busy={savingEdit}
                                onClick={() => void saveInlineEdit()}
                              >
                                {savingEdit ? 'Saving...' : 'Save changes'}
                              </button>
                              <button
                                type="button"
                                className="btn"
                                onClick={() =>
                                  void toggleStatus(
                                    m,
                                    m.status === 'superseded' ? 'active' : 'superseded',
                                  )
                                }
                              >
                                {m.status === 'superseded' ? 'Restore active' : 'Mark superseded'}
                              </button>
                            </div>

                            {confirmDeleteId === m.id ? (
                              <div className="head-actions">
                                <span className="field-hint">Confirm permanent delete?</span>
                                <button
                                  type="button"
                                  className="btn"
                                  onClick={() => setConfirmDeleteId(null)}
                                >
                                  Cancel
                                </button>
                                <button
                                  type="button"
                                  className="btn btn-danger"
                                  onClick={() => void removeMemory(m.id)}
                                >
                                  Confirm delete
                                </button>
                              </div>
                            ) : (
                              <button
                                type="button"
                                className="btn btn-danger"
                                onClick={() => setConfirmDeleteId(m.id)}
                              >
                                Delete
                              </button>
                            )}
                          </div>
                        </>
                      )}
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <ModalDialog
        open={previewOpen}
        title="SessionStart context briefing"
        onClose={() => setPreviewOpen(false)}
        footer={
          <button type="button" className="btn btn-primary" onClick={() => setPreviewOpen(false)}>
            Done
          </button>
        }
      >
        <p className="field-hint">
          Exact payload injected when Claude Code, Codex, or Antigravity starts a session:{' '}
          <span className="measure">
            {preview?.approx_tokens ?? 0} / {preview?.token_budget ?? 450} tokens (
            {preview?.memory_count ?? 0} memories)
          </span>
        </p>
        <div className="panel-sunk ticks" style={{ position: 'relative' }}>
          <pre className="code-block">
            {preview?.briefing ||
              '(Empty: save or distill memories to populate the SessionStart briefing.)'}
          </pre>
        </div>
      </ModalDialog>
    </div>
  );
}
