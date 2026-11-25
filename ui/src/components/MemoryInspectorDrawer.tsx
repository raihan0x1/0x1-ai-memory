import { useEffect, useState } from 'react';
import { api, formatUtcTime } from '../api';
import {
  IconClose,
  IconFile,
  IconGraph,
  IconLink,
  IconPin,
  IconTag,
  IconTimeline,
} from './Icons';
import type { MemoryContextResponse, NeighborLink } from '../types';

interface MemoryInspectorDrawerProps {
  memoryId: string | null;
  onClose: () => void;
  onSelectMemory: (id: string) => void;
  onFilterTag?: (tag: string) => void;
  onFilterFile?: (file: string) => void;
  onMutated: () => Promise<void>;
}

type InspectorTab = 'details' | 'timeline' | 'connections';

const EDGE_KIND_LABELS: Record<string, string> = {
  supersedes: 'Supersedes',
  same_file: 'Shared file',
  same_session: 'Same session',
  shared_tag: 'Shared tag',
  semantic_similar: 'Semantic',
};

const EDGE_COLORS: Record<string, string> = {
  supersedes: '#ff8b3e',
  same_file: '#6fa8ff',
  same_session: '#e8c468',
  semantic_similar: '#5fd39a',
  shared_tag: '#8c877f',
};

function parseCsvList(raw: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const part of raw.split(',')) {
    const trimmed = part.trim();
    if (trimmed && !seen.has(trimmed)) {
      seen.add(trimmed);
      out.push(trimmed);
    }
  }
  return out;
}

function clipToBox(
  fromX: number,
  fromY: number,
  toX: number,
  toY: number,
  halfW: number,
  halfH: number,
): [number, number] {
  const dx = fromX - toX;
  const dy = fromY - toY;
  if (dx === 0 && dy === 0) return [toX, toY];
  const tx = dx !== 0 ? halfW / Math.abs(dx) : Infinity;
  const ty = dy !== 0 ? halfH / Math.abs(dy) : Infinity;
  const t = Math.min(1, tx, ty);
  return [toX + dx * t, toY + dy * t];
}

function LocalNeighborSvg({
  centerTitle,
  neighbors,
  onSelectMemory,
}: {
  centerTitle: string;
  neighbors: NeighborLink[];
  onSelectMemory: (id: string) => void;
}) {
  const shown = neighbors.slice(0, 12);
  const w = 480;
  const h = 250;
  const cx = w / 2;
  const cy = h / 2;
  const rx = 165;
  const ry = 92;

  return (
    <div className="neighbor-svg-box">
      <svg
        viewBox={`0 0 ${w} ${h}`}
        width="100%"
        height="220"
        role="img"
        aria-label="1-hop local memory neighborhood graph"
      >
        <defs>
          <marker
            id="rim-arrow"
            viewBox="0 0 8 8"
            refX="7"
            refY="4"
            markerWidth="6"
            markerHeight="6"
            orient="auto-start-reverse"
          >
            <path d="M 0 1 L 7 4 L 0 7 z" fill="#ff8b3e" />
          </marker>
        </defs>

        {shown.map((link, idx) => {
          const angle = (idx / Math.max(1, shown.length)) * Math.PI * 2 - Math.PI / 2;
          const nx = cx + Math.cos(angle) * rx;
          const ny = cy + Math.sin(angle) * ry;
          const stroke = EDGE_COLORS[link.edge_kind] ?? '#5e5b55';
          const isSup = link.edge_kind === 'supersedes';
          const rawX1 = link.direction === 'incoming' ? nx : cx;
          const rawY1 = link.direction === 'incoming' ? ny : cy;
          const rawX2 = link.direction === 'incoming' ? cx : nx;
          const rawY2 = link.direction === 'incoming' ? cy : ny;
          const [x1, y1] =
            link.direction === 'incoming'
              ? clipToBox(cx, cy, nx, ny, 59, 13)
              : clipToBox(nx, ny, cx, cy, 68, 16);
          const [x2, y2] =
            link.direction === 'incoming'
              ? clipToBox(nx, ny, cx, cy, 70, 18)
              : clipToBox(cx, cy, nx, ny, 61, 15);
          return (
            <g key={`${link.memory.id}-${link.edge_kind}-${idx}`}>
              <line
                x1={rawX1 === rawX2 && rawY1 === rawY2 ? rawX1 : x1}
                y1={rawX1 === rawX2 && rawY1 === rawY2 ? rawY1 : y1}
                x2={rawX1 === rawX2 && rawY1 === rawY2 ? rawX2 : x2}
                y2={rawX1 === rawX2 && rawY1 === rawY2 ? rawY2 : y2}
                stroke={stroke}
                strokeWidth={isSup ? 1.8 : 1.2}
                strokeDasharray={link.edge_kind === 'semantic_similar' ? '4 3' : undefined}
                markerEnd={isSup ? 'url(#rim-arrow)' : undefined}
                opacity={0.85}
              />
            </g>
          );
        })}

        {shown.map((link, idx) => {
          const angle = (idx / Math.max(1, shown.length)) * Math.PI * 2 - Math.PI / 2;
          const nx = cx + Math.cos(angle) * rx;
          const ny = cy + Math.sin(angle) * ry;
          const shortTitle =
            link.memory.title.length > 16
              ? `${link.memory.title.slice(0, 14)}..`
              : link.memory.title;
          const stroke = EDGE_COLORS[link.edge_kind] ?? '#5e5b55';
          return (
            <g
              key={`node-${link.memory.id}-${idx}`}
              style={{ cursor: 'pointer' }}
              onClick={() => onSelectMemory(link.memory.id)}
            >
              <title>{`${link.memory.title} (${EDGE_KIND_LABELS[link.edge_kind] ?? link.edge_kind}: ${link.label})`}</title>
              <rect
                x={nx - 59}
                y={ny - 13}
                width={118}
                height={26}
                fill="#181715"
                stroke={stroke}
                strokeWidth={1}
              />
              <text
                x={nx}
                y={ny + 3.5}
                textAnchor="middle"
                fill="#e8e4dd"
                fontSize="10"
                fontFamily="var(--font-outlier)"
              >
                {shortTitle}
              </text>
            </g>
          );
        })}

        <g>
          <rect
            x={cx - 68}
            y={cy - 16}
            width={136}
            height={32}
            fill="#ff8b3e"
            stroke="#ff8b3e"
            strokeWidth={1}
          />
          <text
            x={cx}
            y={cy + 4}
            textAnchor="middle"
            fill="#14110f"
            fontSize="11"
            fontWeight="600"
            fontFamily="var(--font-body)"
          >
            {centerTitle.length > 17 ? `${centerTitle.slice(0, 15)}..` : centerTitle}
          </text>
        </g>
      </svg>
    </div>
  );
}

export function MemoryInspectorDrawer({
  memoryId,
  onClose,
  onSelectMemory,
  onFilterTag,
  onFilterFile,
  onMutated,
}: MemoryInspectorDrawerProps) {
  const [tab, setTab] = useState<InspectorTab>('details');
  const [ctx, setCtx] = useState<MemoryContextResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [editTitle, setEditTitle] = useState('');
  const [editSummary, setEditSummary] = useState('');
  const [editContent, setEditContent] = useState('');
  const [editTags, setEditTags] = useState('');
  const [editFiles, setEditFiles] = useState('');
  const [saving, setSaving] = useState(false);
  const [saveNotice, setSaveNotice] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    if (!memoryId) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    setSaveNotice(null);
    setConfirmDelete(false);

    api
      .memoryContext(memoryId)
      .then((res) => {
        if (cancelled) return;
        setCtx(res);
        setEditTitle(res.memory.title);
        setEditSummary(res.memory.summary);
        setEditContent(res.memory.content);
        setEditTags(res.memory.tags.join(', '));
        setEditFiles(res.memory.files.join(', '));
      })
      .catch((e) => {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : 'Failed to load memory details');
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [memoryId]);

  useEffect(() => {
    if (!memoryId) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [memoryId, onClose]);

  if (!memoryId) return null;

  const mem = ctx?.memory;

  async function reloadContext() {
    if (!memoryId) return;
    const res = await api.memoryContext(memoryId);
    setCtx(res);
    setEditTitle(res.memory.title);
    setEditSummary(res.memory.summary);
    setEditContent(res.memory.content);
    setEditTags(res.memory.tags.join(', '));
    setEditFiles(res.memory.files.join(', '));
  }

  async function handleSave() {
    if (!mem) return;
    setSaving(true);
    setSaveNotice(null);
    try {
      await api.updateMemory(mem.id, {
        title: editTitle,
        summary: editSummary,
        content: editContent,
        tags: parseCsvList(editTags),
        files: parseCsvList(editFiles),
      });
      await Promise.all([reloadContext(), onMutated()]);
      setSaveNotice('Saved and re-indexed.');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to save changes');
    } finally {
      setSaving(false);
    }
  }

  async function handleStatus(nextStatus: string) {
    if (!mem) return;
    setSaving(true);
    setError(null);
    try {
      await api.updateMemory(mem.id, { status: nextStatus });
      await Promise.all([reloadContext(), onMutated()]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to update memory status');
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete() {
    if (!mem) return;
    setSaving(true);
    setError(null);
    try {
      await api.deleteMemory(mem.id);
      await onMutated();
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to delete memory');
    } finally {
      setSaving(false);
    }
  }

  return (
    <div
      className="inspector-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <aside className="inspector-drawer" aria-label="Memory inspector">
        <div className="inspector-head">
          <div className="inspector-head-top">
            <div className="tag-row">
              {mem ? (
                <>
                  <span className={`tag ${mem.source_client}`}>{mem.source_client}</span>
                  <span className="tag">{mem.memory_type}</span>
                  <span
                    className={`tag ${
                      mem.status === 'pinned'
                        ? 'accent'
                        : mem.status === 'superseded'
                          ? 'warn'
                          : ''
                    }`}
                  >
                    {mem.status}
                  </span>
                  <span className="tag">{mem.project_name ?? 'global'}</span>
                </>
              ) : (
                <span className="field-hint measure">{memoryId}</span>
              )}
            </div>

            <div className="head-actions">
              {mem ? (
                <button
                  type="button"
                  className="btn-subtle"
                  disabled={saving}
                  onClick={() => void handleStatus(mem.status === 'pinned' ? 'active' : 'pinned')}
                >
                  <IconPin size={12} />
                  {mem.status === 'pinned' ? 'Unpin' : 'Pin'}
                </button>
              ) : null}
              <button
                type="button"
                className="btn-subtle"
                onClick={onClose}
                aria-label="Close inspector"
              >
                <IconClose size={12} />
                Close
              </button>
            </div>
          </div>

          {mem ? (
            <div className="tag-row">
              <span className="field-hint measure">{mem.id}</span>
              <span className="field-hint measure">
                Updated {formatUtcTime(mem.updated_at)}
              </span>
              <span className="field-hint measure">
                {mem.recall_count > 0
                  ? `Recalled ${mem.recall_count}x${
                      mem.last_recalled_clients.length > 0
                        ? ` (${mem.last_recalled_clients.join(', ')})`
                        : ''
                    }`
                  : 'Not recalled yet'}
              </span>
            </div>
          ) : null}
        </div>

        <div className="inspector-tabs" role="tablist" aria-label="Inspector sections">
          <button
            type="button"
            role="tab"
            className="inspector-tab"
            aria-selected={tab === 'details'}
            onClick={() => setTab('details')}
          >
            <IconLink size={13} /> Details &amp; Edit
          </button>
          <button
            type="button"
            role="tab"
            className="inspector-tab"
            aria-selected={tab === 'timeline'}
            onClick={() => setTab('timeline')}
          >
            <IconTimeline size={13} /> Timeline &amp; Lineage
            {ctx && ctx.lineage.length > 1 ? ` (${ctx.lineage.length})` : ''}
          </button>
          <button
            type="button"
            role="tab"
            className="inspector-tab"
            aria-selected={tab === 'connections'}
            onClick={() => setTab('connections')}
          >
            <IconGraph size={13} /> Connections
            {ctx ? ` (${ctx.neighbors.length})` : ''}
          </button>
        </div>

        <div className="inspector-body">
          {error ? <div className="callout danger">{error}</div> : null}
          {saveNotice ? <div className="callout ok">{saveNotice}</div> : null}

          {loading ? (
            <div className="stack-sm" aria-busy="true">
              <div className="skeleton-bar skeleton-bar--short" />
              <div className="skeleton-bar skeleton-bar--wide" />
              <div className="skeleton-bar skeleton-bar--wide" />
            </div>
          ) : mem ? (
            <>
              {tab === 'details' ? (
                <div className="stack">
                  <div className="field">
                    <label htmlFor="insp-title">Title</label>
                    <input
                      id="insp-title"
                      className="input"
                      value={editTitle}
                      onChange={(e) => setEditTitle(e.target.value)}
                    />
                  </div>

                  <div className="field">
                    <label htmlFor="insp-summary">Summary (used in SessionStart briefing)</label>
                    <textarea
                      id="insp-summary"
                      className="textarea"
                      rows={3}
                      style={{ minHeight: '72px' }}
                      value={editSummary}
                      onChange={(e) => setEditSummary(e.target.value)}
                    />
                  </div>

                  <div className="field">
                    <label htmlFor="insp-content">Full content (loaded via memory_get)</label>
                    <textarea
                      id="insp-content"
                      className="textarea"
                      rows={7}
                      value={editContent}
                      onChange={(e) => setEditContent(e.target.value)}
                    />
                  </div>

                  {mem.tags.length > 0 ? (
                    <div className="stack-sm" style={{ gap: '4px' }}>
                      <span className="field-hint">Tags (click to filter)</span>
                      <div className="tag-row">
                        {mem.tags.map((t) => (
                          <button
                            key={t}
                            type="button"
                            className="chip"
                            onClick={() => {
                              onFilterTag?.(t);
                            }}
                          >
                            <IconTag size={11} />
                            {t}
                          </button>
                        ))}
                      </div>
                    </div>
                  ) : null}

                  {mem.files.length > 0 ? (
                    <div className="stack-sm" style={{ gap: '4px' }}>
                      <span className="field-hint">Files (click to filter)</span>
                      <div className="tag-row">
                        {mem.files.map((f) => (
                          <button
                            key={f}
                            type="button"
                            className="chip chip--file"
                            onClick={() => {
                              onFilterFile?.(f);
                            }}
                          >
                            <IconFile size={11} />
                            {f}
                          </button>
                        ))}
                      </div>
                    </div>
                  ) : null}

                  <div className="grid-2">
                    <div className="field">
                      <label htmlFor="insp-tags">Edit tags (comma-separated)</label>
                      <input
                        id="insp-tags"
                        className="input input--mono"
                        value={editTags}
                        onChange={(e) => setEditTags(e.target.value)}
                      />
                    </div>
                    <div className="field">
                      <label htmlFor="insp-files">Edit files (comma-separated)</label>
                      <input
                        id="insp-files"
                        className="input input--mono"
                        value={editFiles}
                        onChange={(e) => setEditFiles(e.target.value)}
                      />
                    </div>
                  </div>
                </div>
              ) : null}

              {tab === 'timeline' ? (
                <div className="stack">
                  <section className="stack-sm">
                    <h3 className="panel-title">Supersession lineage</h3>
                    {ctx && ctx.lineage.length > 1 ? (
                      <div className="lineage-chain">
                        {ctx.lineage.map((item, idx) => (
                          <button
                            key={item.id}
                            type="button"
                            className={`lineage-step${item.id === mem.id ? ' lineage-step--current' : ''}`}
                            onClick={() => onSelectMemory(item.id)}
                          >
                            <div style={{ minWidth: 0 }}>
                              <div className="tag-row">
                                <span className="field-hint measure">v{idx + 1}</span>
                                <span className={`tag ${item.source_client}`}>
                                  {item.source_client}
                                </span>
                                <span
                                  className={`tag ${
                                    item.status === 'pinned'
                                      ? 'accent'
                                      : item.status === 'superseded'
                                        ? 'warn'
                                        : 'ok'
                                  }`}
                                >
                                  {item.status}
                                </span>
                              </div>
                              <div
                                style={{
                                  marginTop: '4px',
                                  fontSize: 'var(--text-sm)',
                                  fontWeight: 500,
                                }}
                              >
                                {item.title}
                              </div>
                            </div>
                            <span className="field-hint measure">
                              {formatUtcTime(item.updated_at)}
                            </span>
                          </button>
                        ))}
                      </div>
                    ) : (
                      <p className="field-hint">
                        No prior or superseding versions linked to this memory.
                      </p>
                    )}
                  </section>

                  <section className="stack-sm">
                    <h3 className="panel-title">Chronological project timeline</h3>
                    {ctx && ctx.timeline.length > 0 ? (
                      <div className="lineage-chain">
                        {ctx.timeline.map((item) => (
                          <button
                            key={item.id}
                            type="button"
                            className={`lineage-step${item.id === mem.id ? ' lineage-step--current' : ''}`}
                            onClick={() => onSelectMemory(item.id)}
                          >
                            <div style={{ minWidth: 0 }}>
                              <div className="tag-row">
                                <span className={`tag ${item.source_client}`}>
                                  {item.source_client}
                                </span>
                                <span className="field-hint">{item.memory_type}</span>
                              </div>
                              <div style={{ marginTop: '4px', fontSize: 'var(--text-sm)' }}>
                                {item.title}
                              </div>
                            </div>
                            <span className="field-hint measure">
                              {formatUtcTime(item.updated_at)}
                            </span>
                          </button>
                        ))}
                      </div>
                    ) : (
                      <p className="field-hint">No neighboring memories in timeline.</p>
                    )}
                  </section>
                </div>
              ) : null}

              {tab === 'connections' ? (
                <div className="stack">
                  {ctx && ctx.neighbors.length > 0 ? (
                    <>
                      <LocalNeighborSvg
                        centerTitle={mem.title}
                        neighbors={ctx.neighbors}
                        onSelectMemory={onSelectMemory}
                      />

                      <div className="lineage-chain">
                        {ctx.neighbors.map((link, idx) => (
                          <button
                            key={`${link.memory.id}-${link.edge_kind}-${idx}`}
                            type="button"
                            className="lineage-step"
                            onClick={() => onSelectMemory(link.memory.id)}
                          >
                            <div style={{ minWidth: 0 }}>
                              <div className="tag-row">
                                <span
                                  className="tag"
                                  style={{
                                    borderColor: EDGE_COLORS[link.edge_kind],
                                    color: EDGE_COLORS[link.edge_kind],
                                  }}
                                >
                                  {EDGE_KIND_LABELS[link.edge_kind] ?? link.edge_kind}
                                </span>
                                <span className="field-hint measure">{link.label}</span>
                                <span className={`tag ${link.memory.source_client}`}>
                                  {link.memory.source_client}
                                </span>
                              </div>
                              <div style={{ marginTop: '4px', fontSize: 'var(--text-sm)' }}>
                                {link.memory.title}
                              </div>
                            </div>
                            <span className="field-hint">Inspect</span>
                          </button>
                        ))}
                      </div>
                    </>
                  ) : (
                    <div className="empty-state" style={{ paddingInline: 0 }}>
                      <p>
                        No derived connections yet (shared files, same session, supersession, or
                        vector similarity above 72%).
                      </p>
                    </div>
                  )}
                </div>
              ) : null}
            </>
          ) : null}
        </div>

        {mem ? (
          <div className="inspector-foot">
            <div className="head-actions">
              {tab === 'details' ? (
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={saving}
                  aria-busy={saving}
                  onClick={() => void handleSave()}
                >
                  {saving ? 'Saving...' : 'Save changes'}
                </button>
              ) : null}
              <button
                type="button"
                className="btn"
                disabled={saving}
                onClick={() =>
                  void handleStatus(mem.status === 'superseded' ? 'active' : 'superseded')
                }
              >
                {mem.status === 'superseded' ? 'Restore active' : 'Mark superseded'}
              </button>
              <button
                type="button"
                className="btn"
                disabled={saving}
                onClick={() =>
                  void handleStatus(mem.status === 'archived' ? 'active' : 'archived')
                }
              >
                {mem.status === 'archived' ? 'Unarchive' : 'Archive'}
              </button>
            </div>

            {confirmDelete ? (
              <div className="head-actions">
                <button type="button" className="btn" onClick={() => setConfirmDelete(false)}>
                  Cancel
                </button>
                <button
                  type="button"
                  className="btn btn-danger"
                  disabled={saving}
                  onClick={() => void handleDelete()}
                >
                  Confirm delete
                </button>
              </div>
            ) : (
              <button
                type="button"
                className="btn btn-danger"
                disabled={saving}
                onClick={() => setConfirmDelete(true)}
              >
                Delete
              </button>
            )}
          </div>
        ) : null}
      </aside>
    </div>
  );
}
