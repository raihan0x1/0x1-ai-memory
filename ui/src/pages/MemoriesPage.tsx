import { useCallback, useEffect, useMemo, useState } from 'react';
import { api, formatUtcTime } from '../api';
import { BriefingPreviewModal } from '../components/BriefingPreviewModal';
import {
  IconChevronDown,
  IconClose,
  IconFile,
  IconGraph,
  IconList,
  IconPin,
  IconSearch,
  IconTag,
} from '../components/Icons';
import { MemoryGraphCanvas } from '../components/MemoryGraphCanvas';
import { MemoryInspectorDrawer } from '../components/MemoryInspectorDrawer';
import { RimSelect } from '../components/RimSelect';
import type { MemoryGraphResponse, MemoryHit, Project } from '../types';

interface MemoriesPageProps {
  projects: Project[];
  onChanged: () => Promise<void>;
}

type ViewMode = 'list' | 'graph';
type GroupBy = 'project' | 'flat' | 'type' | 'session' | 'file' | 'tag';

const SCOPE_OPTIONS = [
  { value: '', label: 'All scopes' },
  { value: 'project', label: 'Project' },
  { value: 'global', label: 'Global' },
];

const TYPE_OPTIONS = [
  { value: '', label: 'All types' },
  { value: 'decision', label: 'decision' },
  { value: 'architecture', label: 'architecture' },
  { value: 'bugfix', label: 'bugfix' },
  { value: 'preference', label: 'preference' },
  { value: 'pattern', label: 'pattern' },
  { value: 'session_summary', label: 'session_summary' },
];

const CLIENT_OPTIONS = [
  { value: '', label: 'All clients' },
  { value: 'claude-code', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  { value: 'antigravity', label: 'Antigravity' },
  { value: 't3-code', label: 'T3 Code' },
  { value: 'mcp', label: 'MCP' },
  { value: 'ui', label: 'Manual UI' },
];

const SORT_OPTIONS = [
  { value: '', label: 'Best match / Recent' },
  { value: 'most_recalled', label: 'Most recalled' },
  { value: 'never_recalled', label: 'Never recalled' },
];

const GROUP_OPTIONS = [
  { value: 'project', label: 'Group: Project' },
  { value: 'type', label: 'Group: Type' },
  { value: 'session', label: 'Group: Session' },
  { value: 'file', label: 'Group: File' },
  { value: 'tag', label: 'Group: Tag' },
  { value: 'flat', label: 'Group: Flat' },
];

function readParam(name: string): string {
  return new URLSearchParams(window.location.search).get(name) ?? '';
}

function readViewMode(): ViewMode {
  return readParam('view') === 'graph' ? 'graph' : 'list';
}

function readGroupBy(): GroupBy {
  const g = readParam('group');
  if (g === 'flat' || g === 'type' || g === 'session' || g === 'file' || g === 'tag') {
    return g;
  }
  return 'project';
}

export function MemoriesPage({ projects, onChanged }: MemoriesPageProps) {
  const [viewMode, setViewMode] = useState<ViewMode>(readViewMode);
  const [groupBy, setGroupBy] = useState<GroupBy>(readGroupBy);

  const [query, setQuery] = useState(() => readParam('q'));
  const [projectId, setProjectId] = useState(() => readParam('project'));
  const [scope, setScope] = useState(() => readParam('scope'));
  const [memType, setMemType] = useState(() => readParam('type'));
  const [sourceClient, setSourceClient] = useState(() => readParam('client'));
  const [tagFilter, setTagFilter] = useState(() => readParam('tag'));
  const [fileFilter, setFileFilter] = useState(() => readParam('file'));
  const [sortMode, setSortMode] = useState(() => readParam('sort'));
  const [includeInactive, setIncludeInactive] = useState(false);

  const [hits, setHits] = useState<MemoryHit[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [graphData, setGraphData] = useState<MemoryGraphResponse | null>(null);
  const [graphLoading, setGraphLoading] = useState(false);
  const [graphError, setGraphError] = useState<string | null>(null);

  const [collapsedGroups, setCollapsedGroups] = useState<Record<string, boolean>>({});
  const [selectedMemoryId, setSelectedMemoryId] = useState<string | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);

  const projectOptions = [
    { value: '', label: 'All projects' },
    ...projects.map((p) => ({ value: p.id, label: p.name, meta: p.privacy_mode })),
  ];

  useEffect(() => {
    const url = new URL(window.location.href);
    const entries: [string, string][] = [
      ['view', viewMode === 'graph' ? 'graph' : ''],
      ['group', groupBy !== 'project' ? groupBy : ''],
      ['q', query.trim()],
      ['project', projectId],
      ['scope', scope],
      ['type', memType],
      ['client', sourceClient],
      ['tag', tagFilter],
      ['file', fileFilter],
      ['sort', sortMode],
    ];
    for (const [k, v] of entries) {
      if (v) url.searchParams.set(k, v);
      else url.searchParams.delete(k);
    }
    window.history.replaceState({}, '', url);
  }, [
    viewMode,
    groupBy,
    query,
    projectId,
    scope,
    memType,
    sourceClient,
    tagFilter,
    fileFilter,
    sortMode,
  ]);

  const runSearch = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await api.searchMemories({
        query: query.trim() || undefined,
        project_id: projectId || undefined,
        scope: scope || undefined,
        memory_type: memType || undefined,
        source_client: sourceClient || undefined,
        tag: tagFilter || undefined,
        file: fileFilter || undefined,
        sort: sortMode || undefined,
        include_inactive: includeInactive,
        limit: 120,
      });
      setHits(res.memories);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Search failed');
    } finally {
      setLoading(false);
    }
  }, [
    query,
    projectId,
    scope,
    memType,
    sourceClient,
    tagFilter,
    fileFilter,
    sortMode,
    includeInactive,
  ]);

  const runGraph = useCallback(async () => {
    setGraphLoading(true);
    setGraphError(null);
    try {
      const res = await api.graph({
        project_id: projectId || undefined,
        scope: scope || undefined,
        memory_type: memType || undefined,
        source_client: sourceClient || undefined,
        tag: tagFilter || undefined,
        file: fileFilter || undefined,
        include_inactive: includeInactive,
        limit: 200,
      });
      setGraphData(res);
    } catch (e) {
      setGraphError(e instanceof Error ? e.message : 'Failed to build memory graph');
    } finally {
      setGraphLoading(false);
    }
  }, [projectId, scope, memType, sourceClient, tagFilter, fileFilter, includeInactive]);

  useEffect(() => {
    const timer = setTimeout(() => {
      void runSearch();
    }, 150);
    return () => clearTimeout(timer);
  }, [runSearch]);

  useEffect(() => {
    if (viewMode === 'graph') {
      void runGraph();
    }
  }, [viewMode, runGraph]);

  async function refreshAll() {
    await Promise.all([
      runSearch(),
      viewMode === 'graph' ? runGraph() : Promise.resolve(),
      onChanged(),
    ]);
  }

  async function togglePin(hit: MemoryHit) {
    await api.updateMemory(hit.id, { status: hit.status === 'pinned' ? 'active' : 'pinned' });
    await refreshAll();
  }

  const topTags = useMemo(() => {
    const counts = new Map<string, number>();
    for (const m of hits) {
      for (const t of m.tags) {
        counts.set(t, (counts.get(t) ?? 0) + 1);
      }
    }
    return Array.from(counts.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 8);
  }, [hits]);

  const groupedHits = useMemo(() => {
    if (groupBy === 'flat') {
      return [{ key: 'flat', label: `${hits.length} memories`, items: hits }];
    }

    const groups = new Map<string, { label: string; items: MemoryHit[] }>();
    const add = (key: string, label: string, m: MemoryHit) => {
      const existing = groups.get(key);
      if (existing) existing.items.push(m);
      else groups.set(key, { label, items: [m] });
    };

    for (const m of hits) {
      if (groupBy === 'project') {
        if (m.scope === 'global') add('__global__', 'Global preferences', m);
        else add(m.project_id ?? 'unassigned', m.project_name ?? 'Workspace project', m);
      } else if (groupBy === 'type') {
        add(m.memory_type, m.memory_type, m);
      } else if (groupBy === 'session') {
        if (m.session_id) {
          add(
            m.session_id,
            `${m.source_client} · ${m.project_name ?? 'global'} · ${m.session_id.slice(0, 12)}`,
            m,
          );
        } else {
          add(`manual_${m.source_client}`, `Manual / MCP (${m.source_client})`, m);
        }
      } else if (groupBy === 'file') {
        if (m.files.length === 0) {
          add('__no_file__', 'Unattached (architecture & preferences)', m);
        } else {
          for (const f of m.files) add(f, f, m);
        }
      } else if (groupBy === 'tag') {
        if (m.tags.length === 0) {
          add('__no_tag__', 'Untagged', m);
        } else {
          for (const t of m.tags) add(t, `#${t}`, m);
        }
      }
    }

    return Array.from(groups.entries())
      .sort((a, b) => b[1].items.length - a[1].items.length)
      .map(([key, g]) => ({ key, ...g }));
  }, [hits, groupBy]);

  const hasFilters = Boolean(
    query || projectId || scope || memType || sourceClient || tagFilter || fileFilter || sortMode,
  );

  function clearFilters() {
    setQuery('');
    setProjectId('');
    setScope('');
    setMemType('');
    setSourceClient('');
    setTagFilter('');
    setFileFilter('');
    setSortMode('');
  }

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-title">Memories &amp; knowledge map</h1>
          <p className="page-subtitle">
            Hybrid FTS5 + sqlite-vec retrieval, multi-axis grouping, and zero-LLM connection graph.
          </p>
        </div>

        <div className="head-actions">
          <div className="seg-group" role="tablist" aria-label="Memories view mode">
            <button
              type="button"
              role="tab"
              className="seg-btn"
              aria-selected={viewMode === 'list'}
              onClick={() => setViewMode('list')}
            >
              <IconList size={14} /> Grouped list
            </button>
            <button
              type="button"
              role="tab"
              className="seg-btn"
              aria-selected={viewMode === 'graph'}
              onClick={() => setViewMode('graph')}
            >
              <IconGraph size={14} /> Graph map
            </button>
          </div>

          <button type="button" className="btn btn-primary" onClick={() => setPreviewOpen(true)}>
            Briefing preview
          </button>
        </div>
      </div>

      <div className="toolbar">
        {viewMode === 'list' ? (
          <div className="search-field">
            <IconSearch className="search-field-icon" />
            <input
              type="search"
              className="input"
              aria-label="Search memories"
              placeholder="Search decisions, bugfixes, files, or concepts"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
        ) : null}

        <RimSelect
          ariaLabel="Filter by project"
          value={projectId}
          options={projectOptions}
          onChange={setProjectId}
        />

        <RimSelect
          ariaLabel="Filter by scope"
          value={scope}
          options={SCOPE_OPTIONS}
          onChange={setScope}
        />

        <RimSelect
          ariaLabel="Filter by type"
          value={memType}
          options={TYPE_OPTIONS}
          onChange={setMemType}
        />

        <RimSelect
          ariaLabel="Filter by AI client"
          value={sourceClient}
          options={CLIENT_OPTIONS}
          onChange={setSourceClient}
        />

        {viewMode === 'list' ? (
          <>
            <RimSelect
              ariaLabel="Sort memories"
              value={sortMode}
              options={SORT_OPTIONS}
              onChange={setSortMode}
            />
            <RimSelect
              ariaLabel="Group memories by"
              value={groupBy}
              options={GROUP_OPTIONS}
              onChange={(v) => setGroupBy(v as GroupBy)}
            />
          </>
        ) : null}

        <label className="rim-check-label">
          <input
            type="checkbox"
            className="rim-check"
            checked={includeInactive}
            onChange={(e) => setIncludeInactive(e.target.checked)}
          />
          Include superseded &amp; archived
        </label>
      </div>

      {tagFilter || fileFilter || topTags.length > 0 ? (
        <div className="facet-bar">
          {tagFilter ? (
            <button
              type="button"
              className="chip chip--active"
              onClick={() => setTagFilter('')}
              title="Clear tag filter"
            >
              <IconTag size={11} />
              tag: {tagFilter}
              <IconClose size={11} />
            </button>
          ) : null}
          {fileFilter ? (
            <button
              type="button"
              className="chip chip--active"
              onClick={() => setFileFilter('')}
              title="Clear file filter"
            >
              <IconFile size={11} />
              file: {fileFilter}
              <IconClose size={11} />
            </button>
          ) : null}
          {topTags.length > 0 ? (
            <>
              <span className="field-hint">Quick tags:</span>
              {topTags.map(([t, count]) => (
                <button
                  key={t}
                  type="button"
                  className={`chip${tagFilter.toLowerCase() === t.toLowerCase() ? ' chip--active' : ''}`}
                  onClick={() =>
                    setTagFilter((prev) => (prev.toLowerCase() === t.toLowerCase() ? '' : t))
                  }
                >
                  #{t} <span className="measure">({count})</span>
                </button>
              ))}
            </>
          ) : null}
          {hasFilters ? (
            <button
              type="button"
              className="btn-subtle"
              style={{ marginLeft: 'auto' }}
              onClick={clearFilters}
            >
              Reset all
            </button>
          ) : null}
        </div>
      ) : null}

      {viewMode === 'graph' ? (
        <MemoryGraphCanvas
          graph={graphData}
          loading={graphLoading}
          error={graphError}
          selectedId={selectedMemoryId}
          onSelectNode={(id) => setSelectedMemoryId(id)}
          onRetry={() => void runGraph()}
        />
      ) : (
        <>
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
              <span className="field-hint">Click any row to inspect lineage, links, or edit</span>
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
                {hasFilters ? (
                  <button type="button" className="btn" onClick={clearFilters}>
                    Clear filters
                  </button>
                ) : null}
              </div>
            ) : (
              <div>
                {groupedHits.map((group) => {
                  const isCollapsed = Boolean(collapsedGroups[group.key]);
                  return (
                    <div key={group.key} className="group-block">
                      {groupBy !== 'flat' ? (
                        <button
                          type="button"
                          className="group-head"
                          aria-expanded={!isCollapsed}
                          onClick={() =>
                            setCollapsedGroups((prev) => ({
                              ...prev,
                              [group.key]: !prev[group.key],
                            }))
                          }
                        >
                          <span className="group-head-left">
                            <IconChevronDown size={13} className="group-chevron" />
                            <span>{group.label}</span>
                          </span>
                          <span className="tag measure">{group.items.length}</span>
                        </button>
                      ) : null}

                      {!isCollapsed ? (
                        <ul className="ledger">
                          {group.items.map((m) => (
                            <li
                              key={`${group.key}-${m.id}`}
                              className="ledger-row ledger-row--clickable"
                              data-selected={selectedMemoryId === m.id}
                              onClick={() => setSelectedMemoryId(m.id)}
                            >
                              <div className="ledger-summary">
                                <div className="ledger-client">
                                  <span className={`tag ${m.source_client}`}>
                                    {m.source_client}
                                  </span>
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
                                    {m.recall_count > 0 ? (
                                      <span className="tag ok measure">
                                        recalled {m.recall_count}x
                                      </span>
                                    ) : null}
                                    {m.tags.map((t) => (
                                      <button
                                        key={t}
                                        type="button"
                                        className={`chip${
                                          tagFilter.toLowerCase() === t.toLowerCase()
                                            ? ' chip--active'
                                            : ''
                                        }`}
                                        onClick={(e) => {
                                          e.stopPropagation();
                                          setTagFilter((prev) =>
                                            prev.toLowerCase() === t.toLowerCase() ? '' : t,
                                          );
                                        }}
                                      >
                                        <IconTag size={10} />
                                        {t}
                                      </button>
                                    ))}
                                    {m.files.slice(0, 2).map((f) => (
                                      <button
                                        key={f}
                                        type="button"
                                        className={`chip chip--file${
                                          fileFilter.toLowerCase() === f.toLowerCase()
                                            ? ' chip--active'
                                            : ''
                                        }`}
                                        onClick={(e) => {
                                          e.stopPropagation();
                                          setFileFilter((prev) =>
                                            prev.toLowerCase() === f.toLowerCase() ? '' : f,
                                          );
                                        }}
                                      >
                                        <IconFile size={10} />
                                        {f}
                                      </button>
                                    ))}
                                    {m.files.length > 2 ? (
                                      <span className="field-hint measure">
                                        +{m.files.length - 2} files
                                      </span>
                                    ) : null}
                                  </div>
                                </div>

                                <div
                                  className="ledger-aside"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                  }}
                                >
                                  <time className="field-hint measure" dateTime={m.updated_at}>
                                    {formatUtcTime(m.updated_at)}
                                  </time>
                                  <div className="tag-row">
                                    <button
                                      type="button"
                                      className="btn-subtle"
                                      onClick={() => setSelectedMemoryId(m.id)}
                                    >
                                      Inspect
                                    </button>
                                    <button
                                      type="button"
                                      className="btn-subtle"
                                      onClick={() => void togglePin(m)}
                                    >
                                      <IconPin size={11} />
                                      {m.status === 'pinned' ? 'Unpin' : 'Pin'}
                                    </button>
                                  </div>
                                </div>
                              </div>
                            </li>
                          ))}
                        </ul>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            )}
          </section>
        </>
      )}

      <MemoryInspectorDrawer
        memoryId={selectedMemoryId}
        onClose={() => setSelectedMemoryId(null)}
        onSelectMemory={(id) => setSelectedMemoryId(id)}
        onFilterTag={(t) => {
          setTagFilter(t);
          setSelectedMemoryId(null);
        }}
        onFilterFile={(f) => {
          setFileFilter(f);
          setSelectedMemoryId(null);
        }}
        onMutated={refreshAll}
      />

      <BriefingPreviewModal
        open={previewOpen}
        initialProjectId={projectId || projects[0]?.id}
        projects={projects}
        onClose={() => setPreviewOpen(false)}
        onChanged={refreshAll}
      />
    </div>
  );
}
