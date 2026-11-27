import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import {
  api,
  formatLocalClock,
  formatLocalDayKey,
  formatLocalDayLabel,
  formatLocalTime,
  formatUtcTime,
} from '../api';
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
  ClientConnection,
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
  connections: ClientConnection[];
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
const TICK_DEBOUNCE_MS = 300;

function takeOver(ref: { current: AbortController | null }): AbortController {
  ref.current?.abort();
  const ctl = new AbortController();
  ref.current = ctl;
  return ctl;
}

type StreamGroupMode = 'day' | 'session' | 'flat';
type TailFilter = 'signal' | 'all' | 'tools' | 'turn_end' | 'distill' | 'recall';

const EVENT_LABELS: Record<string, string> = {
  prompt: 'Prompt',
  turn_stop: 'Turn End',
  tool_error: 'Tool Error',
  tool_use: 'Tool',
  distill_result: 'Distill',
  recall: 'Recall',
};

const HOST_LABELS: Record<string, string> = { 't3-code': 'T3 Code', orca: 'Orca', terminal: 'terminal' };

const viaHost = (host: string | null) => (host ? ` via ${HOST_LABELS[host] ?? host}` : '');

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
  item: RawEventView;
  repeatCount: number;
}

function aggregateTail(items: RawEventView[]): AggregatedTailRow[] {
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
  connections,
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
  const [tailNextCursor, setTailNextCursor] = useState<string | null>(null);
  const [tailTotal, setTailTotal] = useState(0);
  const [tailLoading, setTailLoading] = useState(true);
  const [tailLoadingMore, setTailLoadingMore] = useState(false);
  const [tailError, setTailError] = useState<string | null>(null);
  const [tailOlderLoaded, setTailOlderLoaded] = useState(false);

  const [actionNotice, setActionNotice] = useState<{ kind: 'ok' | 'danger'; text: string } | null>(
    null,
  );
  const [pendingPauseProject, setPendingPauseProject] = useState<Project | null>(null);
  const [privacyBusy, setPrivacyBusy] = useState(false);
  const cancelPauseRef = useRef<HTMLButtonElement | null>(null);
  const [distillingId, setDistillingId] = useState<string | null>(null);
  const [flushing, setFlushing] = useState(false);
  const [tailFilter, setTailFilter] = useState<TailFilter>('signal');

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
  const [tailNewState, setTailNewState] = useState({ key: tailFilterKey, value: false });
  const tailHasNew = tailNewState.key === tailFilterKey && tailNewState.value;
  const [tailPausedState, setTailPausedState] = useState({ key: tailFilterKey, paused: false });
  const tailPaused = tailPausedState.key === tailFilterKey && tailPausedState.paused;

  const prevMemTickRef = useRef(sseMemoryTick);
  const prevSessTickRef = useRef(sseSessionTick);
  const prevTailTickRef = useRef(sseTailTick);
  const memAbortRef = useRef<AbortController | null>(null);
  const sessAbortRef = useRef<AbortController | null>(null);
  const tailAbortRef = useRef<AbortController | null>(null);

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
    const ctl = takeOver(memAbortRef);
    setMemLoading(true);
    setMemError(null);
    try {
      const res = await api.searchMemories({
        project_id: projectFilter || undefined,
        source_client: clientFilter || undefined,
        sort: 'recent',
        limit: MEM_PAGE_SIZE,
        offset: (memPage - 1) * MEM_PAGE_SIZE,
      }, ctl.signal);
      if (res.total > 0 && (memPage - 1) * MEM_PAGE_SIZE >= res.total) {
        setMemPageState({ key: memFilterKey, page: Math.max(1, Math.ceil(res.total / MEM_PAGE_SIZE)) });
        return;
      }
      setMemHits(res.memories);
      setMemTotal(res.total);
    } catch (err) {
      if (ctl.signal.aborted) return;
      setMemError(err instanceof Error ? err.message : 'Failed to load memories');
    } finally {
      if (!ctl.signal.aborted) setMemLoading(false);
    }
  }, [projectFilter, clientFilter, memPage, memFilterKey]);

  const loadSessions = useCallback(async () => {
    const ctl = takeOver(sessAbortRef);
    setSessionsLoading(true);
    setSessionsError(null);
    try {
      const sRes = await api.sessions({
        project_id: projectFilter || undefined,
        client: clientFilter || undefined,
        limit: SESSIONS_PAGE_SIZE,
        offset: (sessionsPage - 1) * SESSIONS_PAGE_SIZE,
      }, ctl.signal);
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
      if (ctl.signal.aborted) return;
      setSessionsError(err instanceof Error ? err.message : 'Failed to load sessions');
    } finally {
      if (!ctl.signal.aborted) setSessionsLoading(false);
    }
  }, [projectFilter, clientFilter, sessionsPage, sessFilterKey]);

  const loadTail = useCallback(async () => {
    const ctl = takeOver(tailAbortRef);
    setTailLoading(true);
    setTailLoadingMore(false);
    setTailError(null);
    try {
      const rRes = await api.rawEvents({
        project_id: projectFilter || undefined,
        client: clientFilter || undefined,
        kind: tailFilter,
        limit: TAIL_PAGE_SIZE,
      }, ctl.signal);
      setRawEvents(rRes.events);
      setTailNextCursor(rRes.next_cursor);
      setTailTotal(rRes.total);
      setTailOlderLoaded(false);
      setTailNewState({ key: tailFilterKey, value: false });
    } catch (err) {
      if (ctl.signal.aborted) return;
      setTailError(err instanceof Error ? err.message : 'Failed to load event tail');
    } finally {
      if (!ctl.signal.aborted) setTailLoading(false);
    }
  }, [projectFilter, clientFilter, tailFilter, tailFilterKey]);

  const loadOlderTail = useCallback(async () => {
    if (!tailNextCursor || tailLoadingMore) return;
    const ctl = takeOver(tailAbortRef);
    setTailLoadingMore(true);
    setTailError(null);
    try {
      const rRes = await api.rawEvents({
        project_id: projectFilter || undefined,
        client: clientFilter || undefined,
        kind: tailFilter,
        limit: TAIL_PAGE_SIZE,
        before: tailNextCursor,
      }, ctl.signal);
      setRawEvents((prev) => {
        const seen = new Set(prev.map((ev) => ev.id));
        const older = rRes.events.filter((ev) => !seen.has(ev.id));
        return [...prev, ...older];
      });
      setTailNextCursor(rRes.next_cursor);
      setTailTotal(rRes.total);
      setTailOlderLoaded(true);
    } catch (err) {
      if (ctl.signal.aborted) return;
      setTailError(err instanceof Error ? err.message : 'Failed to load older events');
    } finally {
      if (!ctl.signal.aborted) setTailLoadingMore(false);
    }
  }, [tailNextCursor, tailLoadingMore, projectFilter, clientFilter, tailFilter]);

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
      const t = window.setTimeout(() => void loadMemories(), TICK_DEBOUNCE_MS);
      return () => window.clearTimeout(t);
    }
    setMemNewState({ key: memFilterKey, value: true });
  }, [sseMemoryTick, memPage, memFilterKey, loadMemories]);

  useEffect(() => {
    if (sseSessionTick === prevSessTickRef.current) return;
    prevSessTickRef.current = sseSessionTick;
    if (sessionsPage === 1) {
      const t = window.setTimeout(() => void loadSessions(), TICK_DEBOUNCE_MS);
      return () => window.clearTimeout(t);
    }
    setSessNewState({ key: sessFilterKey, value: true });
  }, [sseSessionTick, sessionsPage, sessFilterKey, loadSessions]);

  useEffect(() => {
    if (sseTailTick === prevTailTickRef.current) return;
    // Leave the ref behind while paused so Resume catches up with one fetch.
    if (tailPaused) return;
    if (tailOlderLoaded) {
      prevTailTickRef.current = sseTailTick;
      setTailNewState({ key: tailFilterKey, value: true });
      return;
    }
    // Mark the tick seen only when the fetch fires, so a Pause inside the window still catches up.
    const t = window.setTimeout(() => {
      prevTailTickRef.current = sseTailTick;
      void loadTail();
    }, TICK_DEBOUNCE_MS);
    return () => window.clearTimeout(t);
  }, [sseTailTick, tailPaused, tailOlderLoaded, tailFilterKey, loadTail]);

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
        key = formatLocalDayKey(m.updated_at);
        label = formatLocalDayLabel(m.updated_at);
      } else {
        key = m.session_id || `manual_${m.source_client}`;
        const sess = m.session_id ? sessionMap.get(m.session_id) : undefined;
        label = sess
          ? `${sess.source_client}${viaHost(sess.host)} · ${sess.project_name ?? 'project'}${sess.branch ? ` (${sess.branch})` : ''} · started ${formatLocalClock(sess.started_at)}`
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

  const displayedTail = useMemo(() => aggregateTail(rawEvents), [rawEvents]);

  const activeProjectObj = projects.find((p) => p.id === projectFilter);
  const projectFilterOptions = [
    { value: '', label: 'All projects' },
    ...projects.map((p) => ({ value: p.id, label: p.name, meta: p.privacy_mode })),
  ];
  const clientFilterOptions = useMemo(() => {
    const active = connections.filter(
      (c) => c.connected || c.detected || Boolean(c.last_event_at || c.last_heartbeat_at),
    );
    const list = active.length > 0 ? active : connections;
    return [
      { value: '', label: 'All AI clients' },
      ...list.map((c) => ({ value: c.client_id, label: c.display_name })),
    ];
  }, [connections]);
  const hasStreamFilter = Boolean(projectFilter || clientFilter);
  const hasAnyMemories = (summary?.total_memories ?? 0) > 0;
  const clearStreamFilters = () => {
    setProjectFilter('');
    setClientFilter('');
  };
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

  useEffect(() => {
    if (actionNotice?.kind !== 'ok') return;
    const t = window.setTimeout(() => setActionNotice(null), 4000);
    return () => window.clearTimeout(t);
  }, [actionNotice]);

  useEffect(() => {
    if (pendingPauseProject) {
      cancelPauseRef.current?.focus();
    }
  }, [pendingPauseProject]);

  async function applyPrivacyChange(project: Project, mode: string) {
    setPrivacyBusy(true);
    setActionNotice(null);
    try {
      await api.setProjectPrivacy(project.id, mode);
      await onRefresh();
      const label = PRIVACY_OPTIONS.find((o) => o.value === mode)?.label ?? mode;
      setActionNotice({ kind: 'ok', text: `Privacy for ${project.name} set to ${label}.` });
      if (mode === 'paused') setPendingPauseProject(null);
    } catch (err) {
      setActionNotice({
        kind: 'danger',
        text: err instanceof Error ? err.message : 'Failed to update project privacy',
      });
    } finally {
      setPrivacyBusy(false);
    }
  }

  function handlePrivacySelect(project: Project, mode: string) {
    if (mode === project.privacy_mode) return;
    if (mode === 'paused') {
      setPendingPauseProject(project);
      return;
    }
    void applyPrivacyChange(project, mode);
  }

  async function handleTogglePin(m: MemoryHit) {
    const nextStatus = m.status === 'pinned' ? 'active' : 'pinned';
    setActionNotice(null);
    try {
      await api.updateMemory(m.id, { status: nextStatus });
      setMemHits((prev) =>
        prev.map((hit) => (hit.id === m.id ? { ...hit, status: nextStatus } : hit)),
      );
      await onRefresh();
      if (memPage === 1) await loadMemories();
      setActionNotice({
        kind: 'ok',
        text: `${nextStatus === 'pinned' ? 'Pinned' : 'Unpinned'} "${m.title}".`,
      });
    } catch (err) {
      setActionNotice({
        kind: 'danger',
        text: err instanceof Error ? err.message : 'Failed to update pin state',
      });
    }
  }

  async function handleDistillSession(sid: string) {
    setDistillingId(sid);
    setActionNotice(null);
    try {
      await api.distillSession(sid);
      await Promise.all([onRefresh(), loadSessions(), loadTail()]);
    } catch (err) {
      setActionNotice({
        kind: 'danger',
        text: err instanceof Error ? err.message : 'Failed to queue distillation',
      });
    } finally {
      setDistillingId(null);
    }
  }

  async function handleFlushPending() {
    setFlushing(true);
    setActionNotice(null);
    try {
      await api.distillFlush();
      await Promise.all([onRefresh(), loadSessions(), loadTail()]);
    } catch (err) {
      setActionNotice({
        kind: 'danger',
        text: err instanceof Error ? err.message : 'Failed to flush pending events',
      });
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
          options={clientFilterOptions}
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
              onChange={(mode) => handlePrivacySelect(activeProjectObj, mode)}
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

      {actionNotice ? (
        <div
          className={`callout ${actionNotice.kind}`}
          role={actionNotice.kind === 'danger' ? 'alert' : 'status'}
          style={{ marginBottom: 'var(--space-md)' }}
        >
          <span>{actionNotice.text}</span>
          <button type="button" className="btn-subtle" onClick={() => setActionNotice(null)}>
            Dismiss
          </button>
        </div>
      ) : null}

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
            hasStreamFilter || hasAnyMemories ? (
              <div className="empty-state">
                <p>No results for this filter.</p>
                <div className="head-actions">
                  <button type="button" className="btn" onClick={clearStreamFilters}>
                    Clear filters
                  </button>
                </div>
              </div>
            ) : (
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
            )
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
                  <p>{hasStreamFilter ? 'No results for this filter.' : 'No sessions captured yet.'}</p>
                  {hasStreamFilter ? (
                    <button type="button" className="btn" onClick={clearStreamFilters}>
                      Clear filters
                    </button>
                  ) : null}
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
                          <time dateTime={s.started_at} title={formatUtcTime(s.started_at)}>
                            {formatLocalClock(s.started_at)}
                          </time>{' '}
                          · last{' '}
                          <time dateTime={s.last_turn_at} title={formatUtcTime(s.last_turn_at)}>
                            {formatLocalClock(s.last_turn_at)}
                          </time>
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
                <button
                  type="button"
                  className="btn-subtle"
                  aria-pressed={tailPaused}
                  onClick={() =>
                    setTailPausedState((prev) => ({
                      key: tailFilterKey,
                      paused: !(prev.key === tailFilterKey && prev.paused),
                    }))
                  }
                >
                  {tailPaused ? <IconPlay size={12} /> : <IconPause size={12} />}
                  {tailPaused ? 'Resume' : 'Pause'}
                </button>
              </div>

              <NewItemsBanner
                visible={tailHasNew && tailOlderLoaded}
                label="New events arrived while reading older history."
                buttonText="Show latest"
                onBack={() => void loadTail()}
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
                <fieldset className="seg-group seg-group--tail" aria-label="Filter live event tail">
                  {(
                    [
                      ['signal', 'Signal'],
                      ['all', 'All'],
                      ['tools', 'Tools'],
                      ['turn_end', 'Turn End'],
                      ['distill', 'Distill'],
                      ['recall', 'Recall'],
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
                  <p>
                    {hasStreamFilter || tailFilter !== 'signal'
                      ? 'No results for this filter.'
                      : 'Waiting for hook or MCP activity.'}
                  </p>
                  {hasStreamFilter || tailFilter !== 'signal' ? (
                    <button
                      type="button"
                      className="btn"
                      onClick={() => {
                        clearStreamFilters();
                        setTailFilter('signal');
                      }}
                    >
                      Clear filters
                    </button>
                  ) : null}
                </div>
              ) : (
                <ul className="event-tail">
                  {displayedTail.map(({ key, item, repeatCount }) => {
                    const isErr =
                      item.event_type === 'tool_error' ||
                      (item.event_type === 'distill_result' && item.preview.startsWith('Failed:'));
                    const isGood =
                      item.event_type === 'recall' ||
                      (item.event_type === 'distill_result' && !item.preview.startsWith('Failed:'));
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
                                EVENT_LABELS[item.event_type] ??
                                item.event_type}
                              {repeatCount > 1 ? ` x${repeatCount}` : ''}
                            </span>
                          </div>
                          <time
                            className="field-hint measure"
                            dateTime={item.created_at}
                            title={formatUtcTime(item.created_at)}
                          >
                            {formatLocalClock(item.created_at)}
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

              {tailTotal > 0 ? (
                <div className="pagination-bar">
                  <span className="pagination-meta measure">
                    Showing {rawEvents.length.toLocaleString()} of {tailTotal.toLocaleString()}
                  </span>
                  {tailNextCursor ? (
                    <button
                      type="button"
                      className="btn-subtle page-btn"
                      disabled={tailLoadingMore}
                      onClick={() => void loadOlderTail()}
                    >
                      {tailLoadingMore ? 'Loading...' : 'Load older'}
                    </button>
                  ) : null}
                </div>
              ) : null}
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

      <ModalDialog
        open={Boolean(pendingPauseProject)}
        title="Pause capture for this project?"
        onClose={() => {
          if (!privacyBusy) setPendingPauseProject(null);
        }}
        preventClose={privacyBusy}
        footer={
          <>
            <button
              ref={cancelPauseRef}
              type="button"
              className="btn"
              disabled={privacyBusy}
              onClick={() => setPendingPauseProject(null)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="btn btn-danger"
              disabled={privacyBusy}
              aria-busy={privacyBusy}
              onClick={() => {
                if (pendingPauseProject) void applyPrivacyChange(pendingPauseProject, 'paused');
              }}
            >
              {privacyBusy ? 'Pausing...' : 'Pause project'}
            </button>
          </>
        }
      >
        <p>
          Stopping capture for <strong>{pendingPauseProject?.name}</strong> ignores new prompts,
          tool activity, distillation, and Handoff for this project until you switch privacy back
          to Full or Local-only.
        </p>
      </ModalDialog>
    </div>
  );
}
