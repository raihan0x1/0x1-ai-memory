import { useEffect, useId, useMemo, useState } from 'react';
import { api, formatLocalTime, formatUtcTime } from '../api';
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
import { PaginationBar } from '../components/PaginationBar';
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
  const [includeInactive, setIncludeInactive] = useState(() => readParam('inactive') === '1');
  const [filtersOpen, setFiltersOpen] = useState(false);
  const filterPanelId = useId();

  const [hits, setHits] = useState<MemoryHit[]>([]);
  const [total, setTotal] = useState(0);
  const [pageSize, setPageSize] = useState(20);
  const [requestVersion, setRequestVersion] = useState(0);
  const [settledRequest, setSettledRequest] = useState<{ key: string; error: string | null }>();
  const trimmedQuery = query.trim();
  const filters = useMemo(
    () => ({
      query: trimmedQuery,
      project_id: projectId,
      scope,
      memory_type: memType,
      source_client: sourceClient,
      tag: tagFilter,
      file: fileFilter,
      sort: sortMode,
      include_inactive: includeInactive,
    }),
    [
      trimmedQuery,
      projectId,
      scope,
      memType,
      sourceClient,
      tagFilter,
      fileFilter,
      sortMode,
      includeInactive,
    ],
  );
  const filterKey = JSON.stringify([filters, pageSize]);
  const [pageState, setPageState] = useState({ key: filterKey, page: 1 });
  const page = pageState.key === filterKey ? pageState.page : 1;
  const requestKey = JSON.stringify([viewMode, filterKey, page, requestVersion]);
  const loading = settledRequest?.key !== requestKey;
  const error = settledRequest?.key === requestKey ? settledRequest.error : null;

  const [graphData, setGraphData] = useState<MemoryGraphResponse | null>(null);

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
      ['q', trimmedQuery],
      ['project', projectId],
      ['scope', scope],
      ['type', memType],
      ['client', sourceClient],
      ['tag', tagFilter],
      ['file', fileFilter],
      ['sort', sortMode],
      ['inactive', includeInactive ? '1' : ''],
    ];
    for (const [k, v] of entries) {
      if (v) url.searchParams.set(k, v);
      else url.searchParams.delete(k);
    }
    window.history.replaceState({}, '', url);
  }, [
    viewMode,
    groupBy,
    trimmedQuery,
    projectId,
    scope,
    memType,
    sourceClient,
    tagFilter,
    fileFilter,
    sortMode,
    includeInactive,
  ]);

  useEffect(() => {
    const controller = new AbortController();
    const { signal } = controller;
    const timer = window.setTimeout(async () => {
      let loadError: string | null = null;
      try {
        if (viewMode === 'graph') {
          const res = await api.graph({ ...filters, limit: 200 }, signal);
          if (!signal.aborted) setGraphData(res);
          return;
        }

        const offset = (page - 1) * pageSize;
        const res = await api.searchMemories({ ...filters, limit: pageSize, offset }, signal);
        if (signal.aborted) return;
        if (res.total > 0 && offset >= res.total) {
          setPageState({ key: filterKey, page: Math.ceil(res.total / pageSize) });
          return;
        }
        setHits(res.memories);
        setTotal(res.total);
      } catch (e) {
        loadError = e instanceof Error ? e.message : 'Could not load memories';
      } finally {
        if (!signal.aborted) setSettledRequest({ key: requestKey, error: loadError });
      }
    }, 150);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [viewMode, filters, page, pageSize, filterKey, requestKey]);

  function reloadResults() {
    setRequestVersion((version) => version + 1);
  }

  function refreshAll() {
    reloadResults();
    return onChanged();
  }

  async function togglePin(hit: MemoryHit) {
    await api.updateMemory(hit.id, { status: hit.status === 'pinned' ? 'active' : 'pinned' });
    await refreshAll();
  }

  const topTags = useMemo(() => {
    if (loading || error) return [];
    const counts = new Map<string, number>();
    const memories = viewMode === 'graph' ? graphData?.nodes : hits;
    for (const m of memories ?? []) {
      for (const t of m.tags) {
        counts.set(t, (counts.get(t) ?? 0) + 1);
      }
    }
    return Array.from(counts.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 8);
  }, [viewMode, graphData, hits, loading, error]);

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
    trimmedQuery ||
      projectId ||
      scope ||
      memType ||
      sourceClient ||
      tagFilter ||
      fileFilter ||
      sortMode ||
      includeInactive,
  );
  const filterCount = [scope, memType, sourceClient, sortMode, includeInactive, tagFilter, fileFilter]
    .filter(Boolean).length;

  function clearFilters() {
    setQuery('');
    setProjectId('');
    setScope('');
    setMemType('');
    setSourceClient('');
    setTagFilter('');
    setFileFilter('');
    setSortMode('');
    setIncludeInactive(false);
  }

  return (
    <div className="memories-page" data-filters-open={filtersOpen}>
      <div className="page-head">
        <div>
          <h1 className="page-title">Memories &amp; knowledge map</h1>
          <p className="page-subtitle">
            Hybrid FTS5 + sqlite-vec retrieval, multi-axis grouping, and zero-LLM connection graph.
          </p>
        </div>

        <div className="head-actions">
          <fieldset className="seg-group" aria-label="Memories view mode">
            <button
              type="button"
              className="seg-btn"
              aria-pressed={viewMode === 'list'}
              onClick={() => setViewMode('list')}
            >
              <IconList size={14} /> Grouped list
            </button>
            <button
              type="button"
              className="seg-btn"
              aria-pressed={viewMode === 'graph'}
              onClick={() => setViewMode('graph')}
            >
              <IconGraph size={14} /> Graph map
            </button>
          </fieldset>

          <button type="button" className="btn btn-primary" onClick={() => setPreviewOpen(true)}>
            Briefing preview
          </button>
        </div>
      </div>

      <div className="toolbar memories-toolbar">
        <div className="search-field">
          <IconSearch className="search-field-icon" />
          <input
            type="search"
            className="input"
            aria-label="Search memories"
            placeholder="Search memories"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>

        <RimSelect
          ariaLabel="Filter by project"
          value={projectId}
          options={projectOptions}
          onChange={setProjectId}
        />

        <button
          type="button"
          className="btn memories-filter-toggle"
          aria-expanded={filtersOpen}
          aria-controls={filterPanelId}
          onClick={() => setFiltersOpen((open) => !open)}
        >
          {filterCount > 0 ? `Filters (${filterCount})` : 'Filters'}
          <IconChevronDown size={14} />
        </button>

        <div className="memories-extra-filters" id={filterPanelId}>
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

          <RimSelect
            ariaLabel="Sort memories"
            value={sortMode}
            options={SORT_OPTIONS}
            onChange={setSortMode}
          />

          {viewMode === 'list' ? (
            <RimSelect
              ariaLabel="Group memories by"
              value={groupBy}
              options={GROUP_OPTIONS}
              onChange={(v) => setGroupBy(v as GroupBy)}
            />
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
      </div>

      {hasFilters || topTags.length > 0 ? (
        <div className="facet-bar memories-facets" data-active-facet={Boolean(tagFilter || fileFilter)}>
          {tagFilter ? (
            <button
              type="button"
              className="chip chip--active"
              onClick={() => setTagFilter('')}
              title="Clear tag filter"
            >
              <IconTag size={11} />
              <span className="chip-label">tag: {tagFilter}</span>
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
              <span className="chip-label">file: {fileFilter}</span>
              <IconClose size={11} />
            </button>
          ) : null}
          <div className="memories-quick-tags">
            {topTags.length > 0 ? <span className="field-hint">Quick tags:</span> : null}
            {topTags.map(([t, count]) => (
              <button
                key={t}
                type="button"
                className={`chip${tagFilter.toLowerCase() === t.toLowerCase() ? ' chip--active' : ''}`}
                onClick={() =>
                  setTagFilter((prev) => (prev.toLowerCase() === t.toLowerCase() ? '' : t))
                }
              >
                <span className="chip-label">#{t}</span> <span className="measure">({count})</span>
              </button>
            ))}
          </div>
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
          graph={loading || error ? null : graphData}
          loading={loading}
          error={error}
          selectedId={selectedMemoryId}
          onSelectNode={(id) => setSelectedMemoryId(id)}
          onRetry={reloadResults}
        />
      ) : (
        <>
          {error ? (
            <div className="callout danger" style={{ marginBottom: 'var(--space-md)' }}>
              <span role="alert">Search error: {error}</span>
              <button type="button" className="btn btn-danger" onClick={reloadResults}>
                Retry
              </button>
            </div>
          ) : null}

          <section className="panel" aria-label="Memory search results">
            <div className="panel-header">
              <h2 className="panel-title">
                {loading ? 'Searching...' : error ? 'Memories' : null}
                <span role="status">
                  {loading || error ? null : `${total.toLocaleString()} ${total === 1 ? 'memory' : 'memories'}`}
                </span>
              </h2>
              <span className="field-hint memories-results-help">
                Click any row to inspect lineage, links, or edit
              </span>
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
                            >
                              <div className="ledger-summary">
                                <div className="ledger-client">
                                  <span className={`tag ${m.source_client}`}>
                                    {m.source_client}
                                  </span>
                                  <span className="field-hint">{m.memory_type}</span>
                                </div>

                                <div className="ledger-main">
                                  <h3 className="ledger-title">
                                    <button
                                      type="button"
                                      className="ledger-row-trigger"
                                      onClick={() => setSelectedMemoryId(m.id)}
                                    >
                                      {m.title}
                                    </button>
                                  </h3>
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
                                        onClick={() => {
                                          setTagFilter((prev) =>
                                            prev.toLowerCase() === t.toLowerCase() ? '' : t,
                                          );
                                        }}
                                      >
                                        <IconTag size={10} />
                                        <span className="chip-label">{t}</span>
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
                                        onClick={() => {
                                          setFileFilter((prev) =>
                                            prev.toLowerCase() === f.toLowerCase() ? '' : f,
                                          );
                                        }}
                                      >
                                        <IconFile size={10} />
                                        <span className="chip-label">{f}</span>
                                      </button>
                                    ))}
                                    {m.files.length > 2 ? (
                                      <span className="field-hint measure">
                                        +{m.files.length - 2} files
                                      </span>
                                    ) : null}
                                  </div>
                                </div>

                                <div className="ledger-aside">
                                  <time
                                    className="field-hint measure"
                                    dateTime={m.updated_at}
                                    title={formatUtcTime(m.updated_at)}
                                  >
                                    {formatLocalTime(m.updated_at)}
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

            <PaginationBar
              label="Memories pages"
              page={page}
              pageSize={pageSize}
              total={total}
              pageSizeOptions={[10, 20, 50]}
              onPageChange={(nextPage) => setPageState({ key: filterKey, page: nextPage })}
              onPageSizeChange={(nextSize) => {
                setPageSize(nextSize);
              }}
            />
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
