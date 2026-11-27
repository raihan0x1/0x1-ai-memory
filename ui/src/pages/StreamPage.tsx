import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { api, formatUtcClock, formatUtcDayKey, formatUtcDayLabel, formatUtcTime } from '../api';
import {
  IconBolt,
  IconChevronDown,
  IconFile,
  IconPanelRight,
  IconPause,
  IconPin,
  IconPlay,
  IconTag,
} from '../components/Icons';
import { MemoryInspectorDrawer } from '../components/MemoryInspectorDrawer';
import { ModalDialog } from '../components/ModalDialog';
import { PaginationBar } from '../components/PaginationBar';
import { RimSelect } from '../components/RimSelect';
import type {
  LiveStreamItem,
  MemoryHit,
  Project,
  RawEventView,
  SessionSummary,
  UsageSummary,
} from '../types';
import type { Tab } from '../App';

interface StreamPageProps {
  summary: UsageSummary | null;
  projects: Project[];
  liveEvents: LiveStreamItem[];
  sseMemoryTick: number;
  sseSessionTick: number;
  sseTailTick: number;
  loading: boolean;
  sseConnected: boolean;
  onRefresh: () => Promise<void>;
  onNavigate: (tab: Tab, queryParams?: Record<string, string>) => void;
}

const MEM_PAGE_SIZE = 10;
const SESSIONS_PAGE_SIZE = 6;
const TAIL_PAGE_SIZE = 20;

type StreamGroupMode = 'day' | 'session' | 'flat';
type TailFilter = 'all' | 'prompts' | 'tools' | 'turn_end' | 'distill';

const EVENT_LABELS: Record<string, string> = { turn_stop: 'Turn End', distill_result: 'Distill' };

const HOST_LABELS: Record<string, string> = { 't3-code': 'T3 Code', orca: 'Orca', terminal: 'terminal' };

const viaHost = (host: string | null) => (host ? ` via ${HOST_LABELS[host] ?? host}` : '');

const CLIENT_OPTIONS = [
  { value: '', label: 'All AI clients' },
  { value: 'claude-code', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  { value: 'antigravity', label: 'Antigravity' },
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

interface AggregatedTailRow {
  key: string;
  item: LiveStreamItem;
  repeatCount: number;
}

function aggregateTail(items: LiveStreamItem[]): AggregatedTailRow[] {
  const out: AggregatedTailRow[] = [];
  for (const item of items) {
    const canAggregate =
      (item.event_type === 'tool_use' || item.event_type === 'tool_error') &&
      Boolean(item.tool_name);
    const prev = out[out.length - 1];
    if (
      canAggregate &&
      prev &&
      prev.item.session_id === item.session_id &&
      prev.item.event_type === item.event_type &&
      prev.item.tool_name === item.tool_name
    ) {
      prev.repeatCount += 1;
    } else {
      out.push({ key: item.id, item, repeatCount: 1 });
    }
  }
  return out;
}

function matchesTailFilters(
  item: LiveStreamItem,
  projectFilter: string,
  clientFilter: string,
  tailFilter: TailFilter,
): boolean {
  if (projectFilter && item.project_id && item.project_id !== projectFilter) return false;
  if (clientFilter && item.source_client !== 'distill' && item.source_client !== clientFilter) {
    return false;
  }
  if (tailFilter === 'prompts' && item.event_type !== 'prompt') return false;
  if (tailFilter === 'tools' && item.event_type !== 'tool_use' && item.event_type !== 'tool_error') {
    return false;
  }
  if (tailFilter === 'turn_end' && item.event_type !== 'turn_stop') return false;
  if (tailFilter === 'distill' && item.event_type !== 'distill_result') return false;
  return true;
}

function NewItemsBanner({
  visible,
  label,
  buttonText,
  onBack,
}: {
  visible: boolean;
  label: string;
  buttonText: string;
  onBack: () => void;
}) {
  if (!visible) return null;
  return (
    <div className="stream-new-banner" role="status">
      <span>{label}</span>
      <button type="button" className="btn-subtle" onClick={onBack}>
        {buttonText}
      </button>
    </div>
  );
}

export function StreamPage({
  summary,
  projects,
  liveEvents,
  sseMemoryTick,
  sseSessionTick,
  sseTailTick,
  loading,
  sseConnected,
  onRefresh,
  onNavigate,
}: StreamPageProps) {
  const [projectFilter, setProjectFilter] = useState('');
  const [clientFilter, setClientFilter] = useState('');
  const [groupMode, setGroupMode] = useState<StreamGroupMode>('day');
  const [railVisible, setRailVisible] = useState(true);
  const [collapsedGroups, setCollapsedGroups] = useState<Record<string, boolean>>({});
  const [selectedMemoryId, setSelectedMemoryId] = useState<string | null>(null);

  const [memHits, setMemHits] = useState<MemoryHit[]>([]);
  const [memTotal, setMemTotal] = useState(0);
  const [memLoading, setMemLoading] = useState(true);
  const [memError, setMemError] = useState<string | null>(null);

  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionsTotal, setSessionsTotal] = useState(0);
  const [sessionsLoading, setSessionsLoading] = useState(true);
  const [sessionsError, setSessionsError] = useState<string | null>(null);

  const [rawEvents, setRawEvents] = useState<RawEventView[]>([]);
  const [tailTotal, setTailTotal] = useState(0);
  const [tailLoading, setTailLoading] = useState(true);
  const [tailError, setTailError] = useState<string | null>(null);

  const [actionError, setActionError] = useState<string | null>(null);
  const [distillingId, setDistillingId] = useState<string | null>(null);
  const [flushing, setFlushing] = useState(false);
  const [tailFilter, setTailFilter] = useState<TailFilter>('all');

  const memFilterKey = `${projectFilter}:${clientFilter}`;
  const [memPageState, setMemPageState] = useState({ key: memFilterKey, page: 1 });
  const memPage = memPageState.key === memFilterKey ? memPageState.page : 1;
  const [memNewState, setMemNewState] = useState({ key: memFilterKey, value: false });
  const memHasNew = memNewState.key === memFilterKey && memNewState.value;

  const sessFilterKey = `${projectFilter}|${clientFilter}`;
  const [sessPageState, setSessPageState] = useState({ key: sessFilterKey, page: 1 });
  const sessionsPage = sessPageState.key === sessFilterKey ? sessPageState.page : 1;
  const [sessNewState, setSessNewState] = useState({ key: sessFilterKey, value: false });
  const sessionsHasNew = sessNewState.key === sessFilterKey && sessNewState.value;

  const tailFilterKey = `${projectFilter}:${clientFilter}:${tailFilter}`;
  const [tailPageState, setTailPageState] = useState({ key: tailFilterKey, page: 1 });
  const tailPage = tailPageState.key === tailFilterKey ? tailPageState.page : 1;
  const [tailNewState, setTailNewState] = useState({ key: tailFilterKey, value: false });
  const tailHasNew = tailNewState.key === tailFilterKey && tailNewState.value;
  const [pausedState, setPausedState] = useState<{
    key: string;
    rows: AggregatedTailRow[] | null;
  }>({ key: tailFilterKey, rows: null });
  const pausedSnapshot = pausedState.key === tailFilterKey ? pausedState.rows : null;
  const tailPaused = pausedSnapshot !== null;

  const prevMemTickRef = useRef(sseMemoryTick);
  const prevSessTickRef = useRef(sseSessionTick);
  const prevTailTickRef = useRef(sseTailTick);
  const latestEventRef = useRef<LiveStreamItem | undefined>(liveEvents[0]);
  latestEventRef.current = liveEvents[0];

  const [saveOpen, setSaveOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [title, setTitle] = useState('');
  const [summaryText, setSummaryText] = useState('');
  const [content, setContent] = useState('');
  const [memType, setMemType] = useState('decision');
  const [scope, setScope] = useState<'project' | 'global'>('project');
  const [targetProject, setTargetProject] = useState('');

  const loadMemories = useCallback(async () => {
    setMemLoading(true);
    setMemError(null);
    try {
      const res = await api.searchMemories({
        project_id: projectFilter || undefined,
        source_client: clientFilter || undefined,
        limit: MEM_PAGE_SIZE,
        offset: (memPage - 1) * MEM_PAGE_SIZE,
      });
      if (res.total > 0 && (memPage - 1) * MEM_PAGE_SIZE >= res.total) {
        setMemPageState({ key: memFilterKey, page: Math.max(1, Math.ceil(res.total / MEM_PAGE_SIZE)) });
        return;
      }
      setMemHits(res.memories);
      setMemTotal(res.total);
    } catch (err) {
      setMemError(err instanceof Error ? err.message : 'Failed to load memories');
    } finally {
      setMemLoading(false);
    }
  }, [projectFilter, clientFilter, memPage, memFilterKey]);

  const loadSessions = useCallback(async () => {
    setSessionsLoading(true);
    setSessionsError(null);
    try {
      const sRes = await api.sessions({
        project_id: projectFilter || undefined,
        client: clientFilter || undefined,
        limit: SESSIONS_PAGE_SIZE,
        offset: (sessionsPage - 1) * SESSIONS_PAGE_SIZE,
      });
      if (sRes.total > 0 && (sessionsPage - 1) * SESSIONS_PAGE_SIZE >= sRes.total) {
        setSessPageState({
          key: sessFilterKey,
          page: Math.max(1, Math.ceil(sRes.total / SESSIONS_PAGE_SIZE)),
        });
        return;
      }
      setSessions(sRes.sessions);
      setSessionsTotal(sRes.total);
    } catch (err) {
      setSessionsError(err instanceof Error ? err.message : 'Failed to load sessions');
    } finally {
      setSessionsLoading(false);
    }
  }, [projectFilter, clientFilter, sessionsPage, sessFilterKey]);

  const loadTail = useCallback(async () => {
    setTailLoading(true);
    setTailError(null);
    try {
      const rRes = await api.rawEvents({
        project_id: projectFilter || undefined,
        client: clientFilter || undefined,
        kind: tailFilter === 'all' ? undefined : tailFilter,
        limit: TAIL_PAGE_SIZE,
        offset: (tailPage - 1) * TAIL_PAGE_SIZE,
      });
      if (rRes.total > 0 && (tailPage - 1) * TAIL_PAGE_SIZE >= rRes.total) {
        setTailPageState({
          key: tailFilterKey,
          page: Math.max(1, Math.ceil(rRes.total / TAIL_PAGE_SIZE)),
        });
        return;
      }
      setRawEvents(rRes.events);
      setTailTotal(rRes.total);
    } catch (err) {
      setTailError(err instanceof Error ? err.message : 'Failed to load event tail');
    } finally {
      setTailLoading(false);
    }
  }, [projectFilter, clientFilter, tailFilter, tailPage, tailFilterKey]);

  useEffect(() => {
    void loadMemories();
  }, [loadMemories]);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  useEffect(() => {
    void loadTail();
  }, [loadTail]);

  useEffect(() => {
    if (sseMemoryTick === prevMemTickRef.current) return;
    prevMemTickRef.current = sseMemoryTick;
    if (memPage === 1) {
      void loadMemories();
    } else {
      const latest = latestEventRef.current;
      const matchesMemScope =
        !latest ||
        ((!projectFilter || !latest.project_id || latest.project_id === projectFilter) &&
          (!clientFilter || latest.source_client === clientFilter));
      if (matchesMemScope) {
        setMemNewState({ key: memFilterKey, value: true });
      }
    }
  }, [sseMemoryTick, memPage, memFilterKey, projectFilter, clientFilter, loadMemories]);

  useEffect(() => {
    if (sseSessionTick === prevSessTickRef.current) return;
    prevSessTickRef.current = sseSessionTick;
    if (sessionsPage === 1) {
      void loadSessions();
    } else {
      const latest = latestEventRef.current;
      if (!latest || !projectFilter || !latest.project_id || latest.project_id === projectFilter) {
        setSessNewState({ key: sessFilterKey, value: true });
      }
    }
  }, [sseSessionTick, sessionsPage, sessFilterKey, projectFilter, loadSessions]);

  useEffect(() => {
    if (sseTailTick === prevTailTickRef.current) return;
    prevTailTickRef.current = sseTailTick;
    if (tailPage === 1) {
      void loadTail();
    } else {
      const latest = latestEventRef.current;
      if (!latest || matchesTailFilters(latest, projectFilter, clientFilter, tailFilter)) {
        setTailNewState({ key: tailFilterKey, value: true });
      }
    }
  }, [sseTailTick, tailPage, tailFilterKey, projectFilter, clientFilter, tailFilter, loadTail]);

  const tokenPct =
    summary && summary.daily_token_cap > 0
      ? Math.min(100, Math.round((summary.today_total_tokens / summary.daily_token_cap) * 100))
      : 0;
  const costPct =
    summary && summary.daily_cost_cap_usd > 0
      ? Math.min(100, Math.round((summary.today_cost_usd / summary.daily_cost_cap_usd) * 100))
      : 0;
  const budgetPct = Math.max(tokenPct, costPct);
  const hasDailyLimit = summary && (summary.daily_token_cap > 0 || summary.daily_cost_cap_usd > 0);
  const meterClass = budgetPct >= 90 ? 'danger' : budgetPct >= 70 ? 'warn' : '';

  const filtered = memHits;

  const sessionMap = useMemo(() => {
    const map = new Map<string, SessionSummary>();
    for (const s of sessions) map.set(s.id, s);
    return map;
  }, [sessions]);

  const groupedMemories = useMemo(() => {
    if (groupMode === 'flat') {
      return [{ key: 'all', label: `All recent (${filtered.length})`, items: filtered }];
    }
    const groups = new Map<string, { label: string; items: MemoryHit[] }>();
    for (const m of filtered) {
      let key: string;
      let label: string;
      if (groupMode === 'day') {
        key = formatUtcDayKey(m.updated_at);
        label = formatUtcDayLabel(m.updated_at);
      } else {
        key = m.session_id || `manual_${m.source_client}`;
        const sess = m.session_id ? sessionMap.get(m.session_id) : undefined;
        label = sess
          ? `${sess.source_client}${viaHost(sess.host)} · ${sess.project_name ?? 'project'}${sess.branch ? ` (${sess.branch})` : ''} · started ${formatUtcClock(sess.started_at)}`
          : m.session_id
            ? `${m.source_client} · session ${m.session_id.slice(0, 12)}`
            : `Manual / MCP (${m.source_client})`;
      }
      const existing = groups.get(key);
      if (existing) existing.items.push(m);
      else groups.set(key, { label, items: [m] });
    }
    return Array.from(groups.entries()).map(([key, g]) => ({ key, ...g }));
  }, [filtered, groupMode, sessionMap]);

  const liveTail = useMemo(() => {
    const base: LiveStreamItem[] = rawEvents.map((ev) => ({
      id: ev.id,
      kind: 'raw_event',
      timestamp: ev.created_at,
      source_client: ev.source_client,
      project_id: ev.project_id,
      project_name: ev.project_name,
      session_id: ev.session_id,
      event_type: ev.event_type,
      tool_name: ev.tool_name,
      preview: ev.preview,
    }));

    const matchedBase = base.filter((item) =>
      matchesTailFilters(item, projectFilter, clientFilter, tailFilter),
    );
    if (tailPage > 1) {
      return aggregateTail(matchedBase);
    }

    const baseIds = new Set(matchedBase.map((item) => item.id));
    const liveExtras: LiveStreamItem[] = [];
    const seenExtras = new Set<string>();
    for (const item of liveEvents) {
      if (baseIds.has(item.id) || seenExtras.has(item.id)) continue;
      if (!matchesTailFilters(item, projectFilter, clientFilter, tailFilter)) continue;
      seenExtras.add(item.id);
      liveExtras.push(item);
      if (liveExtras.length >= 10) break;
    }
    return aggregateTail([...liveExtras, ...matchedBase]);
  }, [rawEvents, liveEvents, tailPage, projectFilter, clientFilter, tailFilter]);

  const displayedTail = tailPage === 1 && pausedSnapshot ? pausedSnapshot : liveTail;

  const activeProjectObj = projects.find((p) => p.id === projectFilter);
  const projectFilterOptions = [
    { value: '', label: 'All projects' },
    ...projects.map((p) => ({ value: p.id, label: p.name, meta: p.privacy_mode })),
  ];
  const projectTargetOptions = projects.map((p) => ({ value: p.id, label: p.name }));
  const totalRecalls = summary?.total_recalls ?? 0;
  const activeSessionsCount = summary?.active_sessions ?? 0;
  const hasAnyPending =
    (summary?.pending_raw_events ?? 0) > 0 || sessions.some((s) => s.pending_events > 0);
  const isZeroLlm = !summary || summary.active_provider === 'none';

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
      if (memPage === 1) await loadMemories();
      else setMemNewState({ key: memFilterKey, value: true });
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

  async function handleTogglePin(m: MemoryHit) {
    const nextStatus = m.status === 'pinned' ? 'active' : 'pinned';
    await api.updateMemory(m.id, { status: nextStatus });
    setMemHits((prev) => prev.map((hit) => (hit.id === m.id ? { ...hit, status: nextStatus } : hit)));
    await onRefresh();
    if (memPage === 1) await loadMemories();
  }

  async function handleDistillSession(sid: string) {
    setDistillingId(sid);
    setActionError(null);
    try {
      await api.distillSession(sid);
      await Promise.all([onRefresh(), loadSessions(), loadTail()]);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to queue distillation');
    } finally {
      setDistillingId(null);
    }
  }

  async function handleFlushPending() {
    setFlushing(true);
    setActionError(null);
    try {
      await api.distillFlush();
      await Promise.all([onRefresh(), loadSessions(), loadTail()]);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to flush pending events');
    } finally {
      setFlushing(false);
    }
  }

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-title">Live cross-agent stream</h1>
          <p className="page-subtitle">
            Real-time capture and shared memory across Claude Code, Codex and Antigravity.
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
              {summary ? (
                <>
                  {summary.today_total_tokens.toLocaleString()} tok /{' '}
                  {summary.daily_token_cap > 0
                    ? summary.daily_token_cap.toLocaleString()
                    : 'Unlimited'}
                  {' · USD limit '}
                  {summary.daily_cost_cap_usd > 0
                    ? `$${summary.daily_cost_cap_usd.toLocaleString('en-US', { maximumSignificantDigits: 8 })}`
                    : 'Unlimited'}
                </>
              ) : 'Daily limits unavailable'}
            </span>
            {hasDailyLimit ? <div className="meter-row">
              <div
                className="meter-track"
                role="progressbar"
                aria-valuenow={budgetPct}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-label="Daily budget used"
              >
                <div className={`meter-fill ${meterClass}`} style={{ width: `${budgetPct}%` }} />
              </div>
              <span className="meter-pct measure">{budgetPct}%</span>
            </div> : null}
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
            {summary?.total_projects ?? 0} projects · {totalRecalls} agent recalls ·{' '}
            {summary?.pending_raw_events ?? 0} queued
          </span>
        </div>

        <div className="metric-cell">
          <div className="metric-head">
            <span className="metric-label">Stream and vectors</span>
            <div className="metric-value metric-value--signal">
              {sseConnected ? 'Live' : 'Offline'}
            </div>
          </div>
          <span className="metric-sub measure">
            {summary?.embedding_status ?? 'fts5'} · {activeSessionsCount} active sessions
          </span>
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

        <fieldset className="seg-group" aria-label="Group stream memories">
          {(
            [
              ['day', 'By Day'],
              ['session', 'By Session'],
              ['flat', 'Flat'],
            ] as const
          ).map(([mode, label]) => (
            <button
              key={mode}
              type="button"
              className="seg-btn"
              aria-pressed={groupMode === mode}
              onClick={() => setGroupMode(mode)}
            >
              {label}
            </button>
          ))}
        </fieldset>

        {activeProjectObj ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-xs)' }}>
            <span className="field-hint">Privacy</span>
            <RimSelect
              id="proj-privacy"
              ariaLabel="Project privacy mode"
              value={activeProjectObj.privacy_mode}
              options={PRIVACY_OPTIONS}
              onChange={(mode) => void handlePrivacyChange(activeProjectObj.id, mode)}
            />
          </div>
        ) : null}

        <button
          type="button"
          className="btn"
          style={{ marginLeft: 'auto' }}
          aria-pressed={railVisible}
          onClick={() => setRailVisible((v) => !v)}
        >
          <IconPanelRight size={14} />
          {railVisible ? 'Hide pipeline' : 'Show pipeline'}
        </button>
      </div>

      <div className={railVisible ? 'stream-layout' : ''}>
        <section className="panel" aria-label="Recent cross-agent memories">
          <div className="panel-header">
            <h2 className="panel-title">Recent memories ({memTotal})</h2>
            <button type="button" className="btn btn-ghost" onClick={() => onNavigate('memories')}>
              Search &amp; graph
            </button>
          </div>

          <NewItemsBanner
            visible={memHasNew && memPage > 1}
            label={`New memories arrived while reading page ${memPage}.`}
            buttonText="New memories — back to page 1"
            onBack={() => {
              setMemNewState({ key: memFilterKey, value: false });
              setMemPageState({ key: memFilterKey, page: 1 });
            }}
          />

          {memError ? (
            <div className="callout danger" style={{ margin: 'var(--space-sm)' }}>
              <span>{memError}</span>
              <button type="button" className="btn btn-danger" onClick={() => void loadMemories()}>
                Retry
              </button>
            </div>
          ) : null}

          {(loading || memLoading) && filtered.length === 0 ? (
            <div aria-busy="true">
              <span className="visually-hidden">Loading memories</span>
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
                No memories recorded yet. Connect Claude Code, Codex or Antigravity, or
                save a decision directly.
              </p>
              <span className="field-hint measure">
                Local MCP ready at {window.location.origin}/mcp
              </span>
              <div className="head-actions">
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => onNavigate('onboarding')}
                >
                  Open onboarding
                </button>
                <button type="button" className="btn" onClick={() => setSaveOpen(true)}>
                  Save first memory
                </button>
              </div>
            </div>
          ) : (
            <div>
              {groupedMemories.map((group) => {
                const isCollapsed = Boolean(collapsedGroups[group.key]);
                return (
                  <div key={group.key} className="group-block">
                    {groupMode !== 'flat' ? (
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
                            key={m.id}
                            className="ledger-row ledger-row--clickable"
                            data-selected={selectedMemoryId === m.id}
                          >
                            <div className="ledger-summary">
                              <div className="ledger-client">
                                <span className={`tag ${m.source_client}`}>{m.source_client}</span>
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
                                  {m.status === 'pinned' ? (
                                    <span className="tag accent">pinned</span>
                                  ) : null}
                                  {m.scope === 'global' ? (
                                    <span className="tag">global</span>
                                  ) : m.project_name ? (
                                    <span className="tag">{m.project_name}</span>
                                  ) : null}
                                  {m.recall_count > 0 ? (
                                    <span className="tag ok measure">
                                      recalled {m.recall_count}x
                                    </span>
                                  ) : null}
                                  {m.tags.slice(0, 3).map((t) => (
                                    <button
                                      key={t}
                                      type="button"
                                      className="chip"
                                      onClick={() => {
                                        onNavigate('memories', { tag: t });
                                      }}
                                    >
                                      <IconTag size={10} />
                                      {t}
                                    </button>
                                  ))}
                                  {m.tags.length > 3 ? (
                                    <span className="field-hint measure">
                                      +{m.tags.length - 3}
                                    </span>
                                  ) : null}
                                  {m.files.slice(0, 2).map((f) => (
                                    <button
                                      key={f}
                                      type="button"
                                      className="chip chip--file"
                                      onClick={() => {
                                        onNavigate('memories', { file: f });
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

                              <div className="ledger-aside">
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
                                    onClick={() => void handleTogglePin(m)}
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
            label="Recent memories pages"
            page={memPage}
            pageSize={MEM_PAGE_SIZE}
            total={memTotal}
            onPageChange={(nextPage) => {
              setMemNewState({ key: memFilterKey, value: false });
              setMemPageState({ key: memFilterKey, page: nextPage });
            }}
          />
        </section>

        {railVisible ? (
          <aside className="stream-rail" aria-label="Live capture and distillation pipeline">
            {actionError ? (
              <div className="callout danger" role="alert">
                <span>{actionError}</span>
                <button type="button" className="btn-subtle" onClick={() => setActionError(null)}>
                  Dismiss
                </button>
              </div>
            ) : null}

            <section className="panel">
              <div className="panel-header">
                <h2 className="panel-title">Sessions &amp; queue ({sessionsTotal})</h2>
                {hasAnyPending && !isZeroLlm ? (
                  <button
                    type="button"
                    className="btn-subtle"
                    disabled={flushing}
                    onClick={() => void handleFlushPending()}
                  >
                    <IconBolt size={12} />
                    {flushing ? 'Flushing...' : 'Flush pending'}
                  </button>
                ) : null}
              </div>

              <NewItemsBanner
                visible={sessionsHasNew && sessionsPage > 1}
                label={`New activity while on page ${sessionsPage}.`}
                buttonText="New activity — back to page 1"
                onBack={() => {
                  setSessNewState({ key: sessFilterKey, value: false });
                  setSessPageState({ key: sessFilterKey, page: 1 });
                }}
              />

              {sessionsError ? (
                <div className="callout danger" style={{ margin: 'var(--space-sm)' }} role="alert">
                  <span>{sessionsError}</span>
                  <button
                    type="button"
                    className="btn btn-danger"
                    onClick={() => void loadSessions()}
                  >
                    Retry
                  </button>
                </div>
              ) : null}

              {isZeroLlm && (summary?.pending_raw_events ?? 0) > 0 ? (
                <div className="callout warn" style={{ margin: 'var(--space-sm)' }}>
                  <span>
                    Zero-LLM mode active: configure a provider in Engine to auto-distill raw
                    events.
                  </span>
                </div>
              ) : null}

              {sessionsLoading && sessions.length === 0 ? (
                <div className="skeleton-row">
                  <div className="skeleton-bar skeleton-bar--wide" />
                </div>
              ) : sessions.length === 0 ? (
                <div className="empty-state" style={{ paddingBlock: 'var(--space-lg)' }}>
                  <p>No sessions captured yet.</p>
                </div>
              ) : (
                <ul className="session-list">
                  {sessions.map((s) => (
                    <li key={s.id} className="session-row">
                      <div className="session-row-top">
                        <div className="tag-row">
                          <span className={`tag ${s.source_client}`}>{s.source_client}</span>
                          {s.host ? (
                            <span className="field-hint">{viaHost(s.host).trim()}</span>
                          ) : null}
                          <span style={{ fontSize: 'var(--text-sm)', fontWeight: 500 }}>
                            {s.project_name ?? 'workspace'}
                          </span>
                          {s.branch ? <span className="tag measure">{s.branch}</span> : null}
                        </div>
                        <span className={`tag ${s.status === 'active' ? 'ok' : ''}`}>
                          {s.status}
                        </span>
                      </div>

                      <div className="session-row-meta">
                        <span className="field-hint measure">
                          {s.pending_events} queued · {s.memory_count} mem · started{' '}
                          {formatUtcClock(s.started_at)} · last {formatUtcClock(s.last_turn_at)}
                        </span>
                        {s.pending_events > 0 && !isZeroLlm ? (
                          <button
                            type="button"
                            className="btn-subtle"
                            disabled={distillingId === s.id}
                            onClick={() => void handleDistillSession(s.id)}
                          >
                            <IconBolt size={11} />
                            {distillingId === s.id ? 'Queuing...' : 'Distill now'}
                          </button>
                        ) : null}
                      </div>
                    </li>
                  ))}
                </ul>
              )}

              <PaginationBar
                label="Sessions pages"
                page={sessionsPage}
                pageSize={SESSIONS_PAGE_SIZE}
                total={sessionsTotal}
                onPageChange={(nextPage) => {
                  setSessNewState({ key: sessFilterKey, value: false });
                  setSessPageState({ key: sessFilterKey, page: nextPage });
                }}
              />
            </section>

            <section className="panel">
              <div className="panel-header">
                <h2 className="panel-title">Live event tail ({tailTotal})</h2>
                {tailPage === 1 ? (
                  <button
                    type="button"
                    className="btn-subtle"
                    aria-pressed={tailPaused}
                    onClick={() =>
                      setPausedState((prev) => ({
                        key: tailFilterKey,
                        rows: prev.key === tailFilterKey && prev.rows !== null ? null : liveTail,
                      }))
                    }
                  >
                    {tailPaused ? <IconPlay size={12} /> : <IconPause size={12} />}
                    {tailPaused ? 'Resume' : 'Pause'}
                  </button>
                ) : (
                  <span className="field-hint measure">Page {tailPage}</span>
                )}
              </div>

              <NewItemsBanner
                visible={tailHasNew && tailPage > 1}
                label={`New events arrived while on page ${tailPage}.`}
                buttonText="New events — back to page 1"
                onBack={() => {
                  setTailNewState({ key: tailFilterKey, value: false });
                  setPausedState({ key: tailFilterKey, rows: null });
                  setTailPageState({ key: tailFilterKey, page: 1 });
                }}
              />

              {tailError ? (
                <div className="callout danger" style={{ margin: 'var(--space-sm)' }} role="alert">
                  <span>{tailError}</span>
                  <button type="button" className="btn btn-danger" onClick={() => void loadTail()}>
                    Retry
                  </button>
                </div>
              ) : null}

              <div
                className="panel-header"
                style={{
                  background: 'var(--color-paper-3)',
                  borderBottomColor: 'var(--color-rule-2)',
                  paddingBlock: 'var(--space-xs)',
                }}
              >
                <fieldset className="seg-group" aria-label="Filter live event tail">
                  {(
                    [
                      ['all', 'All'],
                      ['prompts', 'Prompts'],
                      ['tools', 'Tools'],
                      ['turn_end', 'Turn End'],
                      ['distill', 'Distill'],
                    ] as const
                  ).map(([kind, label]) => (
                    <button
                      key={kind}
                      type="button"
                      className="seg-btn"
                      aria-pressed={tailFilter === kind}
                      onClick={() => setTailFilter(kind)}
                    >
                      {label}
                    </button>
                  ))}
                </fieldset>
              </div>

              {tailLoading && displayedTail.length === 0 ? (
                <div className="skeleton-row" aria-busy="true">
                  <div className="skeleton-bar skeleton-bar--short" />
                  <div className="skeleton-bar skeleton-bar--wide" />
                </div>
              ) : displayedTail.length === 0 ? (
                <div className="empty-state" style={{ paddingBlock: 'var(--space-lg)' }}>
                  <p>Waiting for hook or MCP activity.</p>
                </div>
              ) : (
                <ul className="event-tail">
                  {displayedTail.map(({ key, item, repeatCount }) => {
                    const isErr =
                      item.event_type === 'tool_error' ||
                      item.status === 'error' ||
                      item.status === 'budget_blocked' ||
                      (item.event_type === 'distill_result' && item.preview?.startsWith('Failed:'));
                    const isGood =
                      item.kind === 'memory_created' ||
                      (item.kind === 'usage_recorded' && item.status === 'ok');
                    return (
                      <li key={key} className="event-row">
                        <div className="event-row-top">
                          <div className="tag-row">
                            <span className={`tag ${item.source_client}`}>
                              {item.source_client}
                            </span>
                            <span
                              className={`tag ${isErr ? 'danger' : isGood ? 'ok' : ''}`}
                            >
                              {item.tool_name ??
                                EVENT_LABELS[item.event_type ?? ''] ??
                                item.event_type}
                              {repeatCount > 1 ? ` x${repeatCount}` : ''}
                            </span>
                          </div>
                          <time className="field-hint measure" dateTime={item.timestamp}>
                            {formatUtcClock(item.timestamp)}
                          </time>
                        </div>
                        <span className="event-preview" title={item.preview}>
                          {item.preview}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              )}

              <PaginationBar
                label="Live event tail pages"
                page={tailPage}
                pageSize={TAIL_PAGE_SIZE}
                total={tailTotal}
                onPageChange={(nextPage) => {
                  setTailNewState({ key: tailFilterKey, value: false });
                  setPausedState({ key: tailFilterKey, rows: null });
                  setTailPageState({ key: tailFilterKey, page: nextPage });
                }}
              />
            </section>
          </aside>
        ) : null}
      </div>

      <MemoryInspectorDrawer
        memoryId={selectedMemoryId}
        onClose={() => setSelectedMemoryId(null)}
        onSelectMemory={(id) => setSelectedMemoryId(id)}
        onFilterTag={(t) => {
          setSelectedMemoryId(null);
          onNavigate('memories', { tag: t });
        }}
        onFilterFile={(f) => {
          setSelectedMemoryId(null);
          onNavigate('memories', { file: f });
        }}
        onMutated={async () => {
          await onRefresh();
          if (memPage === 1) await loadMemories();
        }}
      />

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
