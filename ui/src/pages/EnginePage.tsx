import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { api, formatLocalTime, formatUtcTime } from '../api';
import { ModalDialog } from '../components/ModalDialog';
import { PaginationBar } from '../components/PaginationBar';
import { RimSelect } from '../components/RimSelect';
import type {
  ConnectionTestResult,
  DaemonInfo,
  SettingsView,
  UpdateStatusResponse,
  UsageEntry,
  UsageSummary,
} from '../types';

const PROVIDER_PRESETS: Record<
  string,
  { label: string; meta: string; model: string; baseUrl: string; note: string }
> = {
  none: {
    label: 'Distillation off',
    meta: 'manual',
    model: '',
    baseUrl: '',
    note: 'Save memories manually and search with the embedding mode selected below.',
  },
  openrouter: {
    label: 'OpenRouter',
    meta: 'API',
    model: 'google/gemini-2.5-flash-lite',
    baseUrl: 'https://openrouter.ai/api/v1',
    note: 'Uses the OpenRouter adapter and sends redacted turn activity to the Base URL below.',
  },
  gemini: {
    label: 'Google Gemini API',
    meta: 'API',
    model: 'gemini-2.5-flash-lite',
    baseUrl: 'https://generativelanguage.googleapis.com/v1beta',
    note: 'Uses the Gemini adapter and sends redacted turn activity to the Base URL below.',
  },
  ollama: {
    label: 'Ollama (local LLM)',
    meta: 'local',
    model: 'qwen2.5-coder:7b',
    baseUrl: 'http://127.0.0.1:11434/v1',
    note: 'Use a loopback URL and a locally installed model for on-device distillation.',
  },
  openai_compatible: {
    label: 'OpenAI-compatible endpoint',
    meta: 'API',
    model: 'gpt-4.1-nano',
    baseUrl: 'https://api.openai.com/v1',
    note: 'Redacted turn activity goes to the configured endpoint for distillation.',
  },
  cli_claude: {
    label: 'Claude CLI (claude -p)',
    meta: 'opt-in',
    model: 'haiku',
    baseUrl: '',
    note: 'OPT-IN WARNING: Uses your Claude rolling quota and carries Anthropic consumer OAuth ToS caveats. Protected by OX1_MEMORY_INTERNAL=1 and disabled hooks.',
  },
  cli_codex: {
    label: 'Codex CLI (codex exec)',
    meta: 'opt-in',
    model: '',
    baseUrl: '',
    note: 'OPT-IN WARNING: Uses your Codex or ChatGPT rolling quota. Runs ephemeral read-only with features.hooks=false.',
  },
  cli_agy: {
    label: 'Antigravity CLI (agy -p)',
    meta: 'opt-in',
    model: '',
    baseUrl: '',
    note: 'OPT-IN WARNING: Uses Antigravity quota. Guarded by OX1_MEMORY_INTERNAL=1 recursion prevention.',
  },
};

const PROVIDER_OPTIONS = Object.entries(PROVIDER_PRESETS).map(([value, p]) => ({
  value,
  label: p.label,
  meta: p.meta,
}));

const EMBEDDING_OPTIONS = [
  { value: 'local_onnx', label: 'Local ONNX', meta: '384d' },
  { value: 'fts_only', label: 'Keyword search only', meta: 'FTS5' },
  { value: 'api', label: 'API embeddings', meta: 'API' },
];

const EMBEDDING_HINTS: Record<string, string> = {
  local_onnx: 'Keyword and meaning-based search. The model downloads once and runs on this device.',
  fts_only: 'Matches keywords without downloading or running an embedding model.',
  api: 'Sends global search queries and eligible memory content to the API below. Changing models requires re-embedding existing memory vectors for accurate results.',
};

function effectiveBaseUrl(provider: string, baseUrl: string) {
  const trimmed = baseUrl.trim();
  let end = trimmed.length;
  while (trimmed.endsWith('/', end)) {
    end -= 1;
  }
  return trimmed.slice(0, end) || PROVIDER_PRESETS[provider]?.baseUrl || 'https://api.openai.com/v1';
}

interface EnginePageProps {
  active: boolean;
  summary: UsageSummary | null;
  daemon: DaemonInfo | null;
  daemonLoading?: boolean;
  updateStatus: UpdateStatusResponse | null;
  onCheckUpdates: () => Promise<void>;
  onOpenUpdateModal: () => void;
  onOpenWhatsNew: () => void;
  onSaved: () => Promise<void>;
  onDaemonStopped: () => void;
  onRunOnboarding: () => void;
}

export function EnginePage({
  active,
  summary,
  daemon,
  daemonLoading = false,
  updateStatus,
  onCheckUpdates,
  onOpenUpdateModal,
  onOpenWhatsNew,
  onSaved,
  onDaemonStopped,
  onRunOnboarding,
}: EnginePageProps) {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [usage, setUsage] = useState<UsageEntry[]>([]);
  const [usageTotal, setUsageTotal] = useState(0);
  const [usagePage, setUsagePage] = useState(1);
  const [usagePageSize, setUsagePageSize] = useState(20);
  const [usageLoading, setUsageLoading] = useState(true);
  const [usageError, setUsageError] = useState<string | null>(null);
  const usageVersion = useRef(0);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [provider, setProvider] = useState('none');
  const [model, setModel] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [apiKeyInput, setApiKeyInput] = useState('');
  const [dailyTokenCap, setDailyTokenCap] = useState('250000');
  const [dailyCostCap, setDailyCostCap] = useState('0.5');
  const [sessionBudget, setSessionBudget] = useState('450');
  const [embeddingMode, setEmbeddingMode] = useState('local_onnx');
  const [embeddingModel, setEmbeddingModel] = useState('all-MiniLM-L6-v2');
  const [retentionDays, setRetentionDays] = useState('14');
  const [idleSeconds, setIdleSeconds] = useState('120');

  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [saveNotice, setSaveNotice] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);
  const testVersion = useRef(0);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [autoCheckBusy, setAutoCheckBusy] = useState(false);
  const [daemonNotice, setDaemonNotice] = useState<{ kind: 'ok' | 'danger'; text: string } | null>(
    null,
  );
  const [confirmStopOpen, setConfirmStopOpen] = useState(false);
  const [stopping, setStopping] = useState(false);

  const syncFormFromSettings = useCallback((s: SettingsView) => {
    setProvider(s.provider);
    setModel(s.model);
    setBaseUrl(s.base_url);
    setDailyTokenCap(String(s.daily_token_cap));
    setDailyCostCap(String(s.daily_cost_cap_usd));
    setSessionBudget(String(s.session_start_token_budget));
    setEmbeddingMode(s.embedding_mode);
    setEmbeddingModel(s.embedding_model);
    setRetentionDays(String(s.raw_event_retention_days));
    setIdleSeconds(String(s.distill_idle_seconds));
  }, []);

  const loadSettings = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const s = await api.settings();
      setSettings(s);
      syncFormFromSettings(s);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : 'Failed to load settings');
    } finally {
      setLoading(false);
    }
  }, [syncFormFromSettings]);

  const loadUsage = useCallback(async () => {
    const version = ++usageVersion.current;
    setUsageLoading(true);
    setUsageError(null);
    try {
      const u = await api.usageHistory({
        limit: usagePageSize,
        offset: (usagePage - 1) * usagePageSize,
      });
      if (version !== usageVersion.current) return;
      if (u.total > 0 && (usagePage - 1) * usagePageSize >= u.total) {
        setUsagePage(Math.max(1, Math.ceil(u.total / usagePageSize)));
        return;
      }
      setUsage(u.usage);
      setUsageTotal(u.total);
    } catch (e) {
      if (version === usageVersion.current) {
        setUsageError(e instanceof Error ? e.message : 'Failed to load usage ledger');
      }
    } finally {
      if (version === usageVersion.current) setUsageLoading(false);
    }
  }, [usagePage, usagePageSize]);

  useEffect(() => {
    void loadSettings();
  }, [loadSettings]);

  useEffect(() => {
    if (active) void loadUsage();
  }, [active, loadUsage]);

  function clearConnectionResult() {
    testVersion.current += 1;
    setTestResult(null);
    setError(null);
  }

  function onProviderChange(next: string) {
    if (next === provider) return;
    setProvider(next);
    setApiKeyInput('');
    clearConnectionResult();
    const preset = PROVIDER_PRESETS[next];
    if (preset) {
      setModel(preset.model);
      setBaseUrl(preset.baseUrl);
    }
  }

  const connection = { provider, model, base_url: baseUrl };
  const draft = {
    ...connection,
    daily_token_cap: Number(dailyTokenCap),
    daily_cost_cap_usd: Number(dailyCostCap),
    session_start_token_budget: Number(sessionBudget),
    embedding_mode: embeddingMode,
    embedding_model: embeddingModel,
    raw_event_retention_days: Number(retentionDays),
    distill_idle_seconds: Number(idleSeconds),
  };
  const patch = Object.fromEntries(
    Object.entries(draft).filter(([key, value]) => settings?.[key as keyof SettingsView] !== value),
  );
  const destination = effectiveBaseUrl(provider, baseUrl);
  const destinationChanged = provider !== settings?.provider ||
    destination !== effectiveBaseUrl(provider, settings?.base_url ?? '');
  const hasNewKey = apiKeyInput.trim() !== '';
  if (destinationChanged || patch.model !== undefined || hasNewKey) Object.assign(patch, connection);
  if (hasNewKey) patch.api_key = apiKeyInput.trim();
  const dirty = !!settings && Object.keys(patch).length > 0;

  async function handleSave(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!dirty || saving || testing || autoCheckBusy) return;
    if (draft.daily_token_cap > 0 && draft.daily_token_cap < 1000) {
      const input = e.currentTarget.elements.namedItem('daily_token_cap');
      if (input instanceof HTMLInputElement) {
        input.setCustomValidity('Enter 0 for unlimited or at least 1,000 tokens.');
        input.reportValidity();
      }
      return;
    }
    setSaving(true);
    setError(null);
    setSaveNotice(null);
    try {
      const updated = await api.patchSettings(patch);
      setSettings(updated);
      syncFormFromSettings(updated);
      setApiKeyInput('');
      setTestResult(null);
      setSaveNotice('Settings saved for new work.');
      await onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save settings');
    } finally {
      setSaving(false);
    }
  }

  function discardDraft(form: HTMLFormElement | null) {
    if (!settings) return;
    const tokenInput = form?.elements.namedItem('daily_token_cap');
    if (tokenInput instanceof HTMLInputElement) tokenInput.setCustomValidity('');
    syncFormFromSettings(settings);
    setApiKeyInput('');
    setSaveNotice(null);
    clearConnectionResult();
  }

  async function handleTest() {
    if (!settings || testing || saving || provider === 'none') return;
    const version = ++testVersion.current;
    setTesting(true);
    setError(null);
    setTestResult(null);
    try {
      const res = await api.testEngine({
        ...connection,
        ...(hasNewKey ? { api_key: apiKeyInput.trim() } : {}),
      });
      if (version === testVersion.current) setTestResult(res);
      await Promise.all([loadUsage(), onSaved()]);
    } catch (err) {
      if (version === testVersion.current) {
        setError(err instanceof Error ? err.message : 'Failed to test distillation');
      }
    } finally {
      setTesting(false);
    }
  }

  async function handleAutostartToggle(enabled: boolean) {
    setAutostartBusy(true);
    setDaemonNotice(null);
    try {
      const updated = await api.setAutostart(enabled);
      setDaemonNotice({
        kind: 'ok',
        text: updated.enabled
          ? 'Auto-Start enabled for future logins.'
          : 'Auto-Start disabled. The running Daemon stays active.',
      });
      await onSaved();
    } catch (e) {
      setDaemonNotice({
        kind: 'danger',
        text: e instanceof Error ? e.message : 'Failed to update Auto-Start',
      });
    } finally {
      setAutostartBusy(false);
    }
  }

  async function handleCheckUpdates() {
    setCheckingUpdates(true);
    setDaemonNotice(null);
    try {
      await onCheckUpdates();
    } catch (e) {
      setDaemonNotice({
        kind: 'danger',
        text: e instanceof Error ? e.message : 'Failed to check for updates',
      });
    } finally {
      setCheckingUpdates(false);
    }
  }

  async function handleAutoUpdateToggle(enabled: boolean) {
    setAutoCheckBusy(true);
    setDaemonNotice(null);
    try {
      const updated = await api.patchSettings({ auto_update_check: enabled });
      setSettings(updated);
      setDaemonNotice({
        kind: 'ok',
        text: updated.auto_update_check
          ? 'Automatic 6-hour update checks enabled.'
          : 'Automatic update checks disabled (manual checks still work).',
      });
      await onSaved();
    } catch (e) {
      setDaemonNotice({
        kind: 'danger',
        text: e instanceof Error ? e.message : 'Failed to update check preference',
      });
    } finally {
      setAutoCheckBusy(false);
    }
  }

  async function handleConfirmStop() {
    setStopping(true);
    setDaemonNotice(null);
    try {
      await api.stopDaemon();
      setConfirmStopOpen(false);
      onDaemonStopped();
    } catch (e) {
      setDaemonNotice({
        kind: 'danger',
        text: e instanceof Error ? e.message : 'Failed to stop Daemon',
      });
    } finally {
      setStopping(false);
    }
  }

  const preset = PROVIDER_PRESETS[provider];
  const activePreset = summary ? PROVIDER_PRESETS[summary.active_provider] : undefined;
  const connectionBusy = !settings || saving;
  const keyHint = destinationChanged
    ? 'Enter credentials for the new destination if it requires authentication.'
    : settings?.has_api_key
      ? `Blank keeps the current key (${settings.api_key_masked || 'from environment'}).`
      : 'Enter an API key or an env:VAR reference if the endpoint requires authentication.';

  return (
    <div className="stack">
      <div className="page-head" style={{ marginBottom: 0 }}>
        <h1 className="page-title">Engine &amp; budget</h1>
      </div>

      <section className="panel" aria-labelledby="engine-usage">
        <div className="panel-header">
          <div>
            <h2 className="panel-title" id="engine-usage">Today’s usage</h2>
            <p className="field-hint">Resets at 00:00 UTC</p>
          </div>
          {summary?.budget_exhausted ? <span className="tag warn">Daily limit reached</span> : null}
        </div>
        {summary ? (
          <dl className="engine-usage-grid">
            <div className="metric-cell">
              <dt className="metric-label">Active distillation</dt>
              <dd>
                {activePreset?.label ?? summary.active_provider}
                {summary.active_provider !== 'none' && summary.active_model ? (
                  <div className="metric-sub measure engine-model">{summary.active_model}</div>
                ) : null}
              </dd>
            </div>
            <div className="metric-cell">
              <dt className="metric-label">Tokens used</dt>
              <dd>
                <span className="measure">{summary.today_total_tokens.toLocaleString()}</span>
                <span className="metric-sub"> / {summary.daily_token_cap > 0
                  ? summary.daily_token_cap.toLocaleString()
                  : 'Unlimited'}</span>
              </dd>
            </div>
            <div className="metric-cell">
              <dt className="metric-label">Recorded cost (USD)</dt>
              <dd>
                <span className="measure">${summary.today_cost_usd.toFixed(4)}</span>
                <span className="metric-sub"> / {summary.daily_cost_cap_usd > 0
                  ? `$${summary.daily_cost_cap_usd.toLocaleString('en-US', { maximumSignificantDigits: 8 })}`
                  : 'Unlimited'}</span>
              </dd>
            </div>
          </dl>
        ) : (
          <div className="panel-body field-hint" role="status">
            {daemonLoading ? 'Loading usage…' : 'Usage is currently unavailable.'}
          </div>
        )}
        {summary && summary.today_blocked > 0 ? (
          <p className="engine-usage-note field-hint">
            Blocked requests today: {summary.today_blocked.toLocaleString()}. Skipped distillation
            batches stay in raw activity until they expire and are excluded from recovery.
          </p>
        ) : null}
      </section>

      <form className="panel" aria-labelledby="engine-settings" method="post" onSubmit={handleSave}>
        <div className="panel-header">
          <div>
            <h2 className="panel-title" id="engine-settings">Engine settings</h2>
            {dirty ? <p className="field-hint">Unsaved changes</p> : null}
          </div>
          <div className="head-actions">
            {dirty ? (
              <button
                type="button"
                className="btn"
                disabled={saving}
                onClick={(e) => discardDraft(e.currentTarget.form)}
              >
                Discard changes
              </button>
            ) : null}
            {provider !== 'none' ? (
              <button
                type="button"
                className="btn"
                disabled={testing || connectionBusy}
                aria-busy={testing}
                aria-describedby="eng-test-hint"
                onClick={() => void handleTest()}
              >
                {testing ? 'Testing…' : 'Test distillation'}
              </button>
            ) : null}
            <button
              type="submit"
              className="btn btn-primary"
              disabled={!dirty || saving || testing || autoCheckBusy}
              aria-busy={saving}
            >
              {saving ? 'Saving…' : 'Save settings'}
            </button>
          </div>
        </div>

        <div className="panel-body stack">
          {loading ? <div className="field-hint" role="status">Loading settings…</div> : null}
          {loadError ? (
            <div className="callout danger" role="alert">
              <span>Could not load settings. {loadError}</span>
              <button type="button" className="btn" onClick={() => void loadSettings()}>
                Retry loading
              </button>
            </div>
          ) : null}
          {error ? <div className="callout danger" role="alert">{error}</div> : null}
          {saveNotice && !dirty ? <div className="callout ok" role="status">{saveNotice}</div> : null}
          {testResult ? (
            <div
              className={`callout ${testResult.status === 'budget_blocked' ? 'warn' : testResult.ok ? 'ok' : 'danger'}`}
              role="status"
            >
              <span>{testResult.message}</span>
              <span className="measure">
                {testResult.latency_ms} ms
                {testResult.estimated_cost_usd > 0
                  ? ` · $${testResult.estimated_cost_usd.toPrecision(3)}`
                  : ''}
              </span>
            </div>
          ) : null}

          {settings ? (
            <>
              <fieldset className="engine-fieldset stack-sm" disabled={saving}>
                <legend>Distillation</legend>
                <div className="grid-2">
                  <div className="field">
                    <label htmlFor="eng-provider">Memory LLM provider</label>
                    <RimSelect
                      id="eng-provider"
                      ariaLabel="Memory LLM provider"
                      fullWidth
                      disabled={saving}
                      value={provider}
                      options={PROVIDER_OPTIONS}
                      onChange={onProviderChange}
                    />
                  </div>
                  {provider !== 'none' ? (
                    <div className="field">
                      <label htmlFor="eng-model">Model</label>
                      <input
                        id="eng-model"
                        name="model"
                        className="input input--mono"
                        placeholder={preset?.model || undefined}
                        spellCheck={false}
                        autoCapitalize="none"
                        value={model}
                        onChange={(e) => {
                          setModel(e.target.value);
                          clearConnectionResult();
                        }}
                      />
                    </div>
                  ) : null}
                </div>
                {preset ? <p className="field-hint">{preset.note}</p> : null}
                {provider !== 'none' && !provider.startsWith('cli_') ? (
                  <div className="grid-2">
                    <div className="field">
                      <label htmlFor="eng-url">Base URL</label>
                      <input
                        id="eng-url"
                        name="base_url"
                        type="url"
                        className="input input--mono"
                        placeholder={preset?.baseUrl}
                        spellCheck={false}
                        autoCapitalize="none"
                        value={baseUrl}
                        onChange={(e) => {
                          setBaseUrl(e.target.value);
                          setApiKeyInput('');
                          clearConnectionResult();
                        }}
                      />
                    </div>
                    <div className="field">
                      <label htmlFor="eng-key">API key</label>
                      <input
                        id="eng-key"
                        name="api_key"
                        type="password"
                        className="input input--mono"
                        autoComplete="off"
                        aria-describedby="eng-key-hint"
                        value={apiKeyInput}
                        onChange={(e) => {
                          setApiKeyInput(e.target.value);
                          clearConnectionResult();
                        }}
                      />
                      <span className="field-hint" id="eng-key-hint">{keyHint}</span>
                    </div>
                  </div>
                ) : null}
                {provider !== 'none' ? (
                  <p id="eng-test-hint" className="field-hint">
                    Uses the draft provider, saved daily limits, and a small sample that can use
                    provider credits.
                  </p>
                ) : null}
              </fieldset>

              <fieldset className="engine-fieldset stack-sm" disabled={saving}>
                <legend>Daily budget</legend>
                <div className="grid-2">
                  <div className="field">
                    <label htmlFor="eng-tok">Token limit</label>
                    <input
                      id="eng-tok"
                      name="daily_token_cap"
                      type="number"
                      min={0}
                      step={1}
                      required
                      aria-describedby="eng-tok-hint"
                      className="input input--mono"
                      value={dailyTokenCap}
                      onChange={(e) => {
                        e.currentTarget.setCustomValidity('');
                        setDailyTokenCap(e.target.value);
                      }}
                    />
                    <span className="field-hint" id="eng-tok-hint">0 = Unlimited. Otherwise enter 1,000 or more.</span>
                  </div>
                  <div className="field">
                    <label htmlFor="eng-cost">Cost limit (USD)</label>
                    <input
                      id="eng-cost"
                      name="daily_cost_cap_usd"
                      type="number"
                      min={0}
                      step="any"
                      required
                      aria-describedby="eng-cost-hint"
                      className="input input--mono"
                      value={dailyCostCap}
                      onChange={(e) => setDailyCostCap(e.target.value)}
                    />
                    <span className="field-hint" id="eng-cost-hint">0 = Unlimited. Provider limits still apply.</span>
                  </div>
                </div>
                <p className="field-hint">
                  A reached limit blocks new calls. A call already in progress can exceed the limit.
                </p>
              </fieldset>

              <fieldset className="engine-fieldset stack-sm" disabled={saving}>
                <legend>Search &amp; briefing</legend>
                <div className="grid-2">
                  <div className="field">
                    <label htmlFor="eng-emb">Search mode</label>
                    <RimSelect
                      id="eng-emb"
                      ariaLabel="Search mode"
                      fullWidth
                      disabled={saving}
                      value={embeddingMode}
                      options={EMBEDDING_OPTIONS}
                      onChange={setEmbeddingMode}
                    />
                    <span className="field-hint">{EMBEDDING_HINTS[embeddingMode]}</span>
                    {summary?.embedding_status ? (
                      <span className="field-hint">Active: {summary.embedding_status}</span>
                    ) : null}
                  </div>
                  <div className="field">
                    <label htmlFor="eng-brief">Session briefing (tokens)</label>
                    <input
                      id="eng-brief"
                      name="session_start_token_budget"
                      type="number"
                      min={100}
                      max={2000}
                      step={1}
                      required
                      aria-describedby="eng-brief-hint"
                      className="input input--mono"
                      value={sessionBudget}
                      onChange={(e) => setSessionBudget(e.target.value)}
                    />
                    <span className="field-hint" id="eng-brief-hint">
                      Approximate size of the memory summary sent to an agent at session start.
                      Default 450.
                    </span>
                  </div>
                </div>
                {embeddingMode === 'api' ? (
                  <div className="field">
                    <label htmlFor="eng-emb-model">API embedding model</label>
                    <input
                      id="eng-emb-model"
                      name="embedding_model"
                      className="input input--mono"
                      aria-describedby="eng-emb-model-hint"
                      value={embeddingModel}
                      spellCheck={false}
                      onChange={(e) => setEmbeddingModel(e.target.value)}
                    />
                    <span className="field-hint" id="eng-emb-model-hint">
                      Endpoint <span className="measure engine-model">{destination}</span>.
                      The vector index stores 384 dimensions.
                    </span>
                  </div>
                ) : null}
              </fieldset>

              <details className="engine-advanced">
                <summary>Storage &amp; recovery</summary>
                <fieldset className="engine-fieldset grid-2" aria-label="Storage and recovery" disabled={saving}>
                  <div className="field">
                    <label htmlFor="eng-ret">Raw activity retention (days)</label>
                    <input
                      id="eng-ret"
                      name="raw_event_retention_days"
                      type="number"
                      min={1}
                      max={365}
                      step={1}
                      required
                      aria-describedby="eng-ret-hint"
                      className="input input--mono"
                      value={retentionDays}
                      onChange={(e) => setRetentionDays(e.target.value)}
                    />
                    <span className="field-hint" id="eng-ret-hint">
                      Processed activity expires after this many days. Pending activity can stay
                      twice as long. Saved memories have separate lifetimes.
                    </span>
                  </div>
                  <div className="field">
                    <label htmlFor="eng-idle">Idle time before recovery (seconds)</label>
                    <input
                      id="eng-idle"
                      name="distill_idle_seconds"
                      type="number"
                      min={15}
                      max={3600}
                      step={1}
                      required
                      aria-describedby="eng-idle-hint"
                      className="input input--mono"
                      value={idleSeconds}
                      onChange={(e) => setIdleSeconds(e.target.value)}
                    />
                    <span className="field-hint" id="eng-idle-hint">
                      Makes pending activity eligible for distillation if a session goes quiet
                      without a turn-end signal. The worker checks every 30 seconds.
                    </span>
                  </div>
                </fieldset>
              </details>
            </>
          ) : null}
        </div>
      </form>

      <section className="panel">
        <div className="panel-header">
          <h2 className="panel-title">
            Usage ledger ({usageTotal} {usageTotal === 1 ? 'entry' : 'entries'})
          </h2>
          <span className="field-hint">Distillation, API embeddings, and tests</span>
        </div>
        {usageError ? (
          <div className="callout danger" style={{ margin: 'var(--space-sm)' }}>
            <span>{usageError}</span>
            <button type="button" className="btn btn-danger" onClick={() => void loadUsage()}>
              Retry
            </button>
          </div>
        ) : null}
        <div className="table-wrap">
          {usageLoading && usage.length === 0 ? (
            <div className="skeleton-row">
              <div className="skeleton-bar skeleton-bar--wide" />
            </div>
          ) : usageError && usage.length === 0 ? null : usage.length === 0 ? (
            <div className="empty-state">
              <p>No usage recorded yet.</p>
            </div>
          ) : (
            <table>
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Provider</th>
                  <th>Model</th>
                  <th>Operation</th>
                  <th>Tokens (in / out)</th>
                  <th>Estimated cost</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {usage.map((u) => (
                  <tr key={u.id}>
                    <td className="measure" title={formatUtcTime(u.created_at)}>
                      <time dateTime={u.created_at}>{formatLocalTime(u.created_at)}</time>
                    </td>
                    <td className="measure">{u.provider}</td>
                    <td className="measure">{u.model || 'none'}</td>
                    <td>{u.operation}</td>
                    <td className="measure">
                      {u.input_tokens} / {u.output_tokens}
                    </td>
                    <td className="measure">${u.estimated_cost_usd.toFixed(5)}</td>
                    <td>
                      <span
                        className={`tag ${
                          u.status === 'ok'
                            ? 'ok'
                            : u.status === 'budget_blocked'
                              ? 'warn'
                              : 'danger'
                        }`}
                        title={u.error_message ?? undefined}
                      >
                        {u.status}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
        <PaginationBar
          label="Usage ledger pages"
          page={usagePage}
          pageSize={usagePageSize}
          total={usageTotal}
          pageSizeOptions={[10, 20, 50]}
          onPageChange={setUsagePage}
          onPageSizeChange={(nextSize) => {
            setUsagePageSize(nextSize);
            setUsagePage(1);
          }}
        />
      </section>

      <section className="panel">
        <div className="panel-header">
          <div>
            <h2 className="panel-title">Local Daemon and Auto-Start</h2>
            <p className="field-hint">
              Manage background lifecycle, login Auto-Start, or re-run Onboarding.
            </p>
          </div>
          <div className="head-actions">
            <button
              type="button"
              className="btn"
              disabled={dirty || saving}
              title={dirty ? 'Save or discard your Engine settings first' : undefined}
              onClick={onRunOnboarding}
            >
              Run onboarding
            </button>
            <button
              type="button"
              className="btn btn-danger"
              onClick={() => setConfirmStopOpen(true)}
            >
              Stop daemon
            </button>
          </div>
        </div>

        <div className="panel-body stack">
          {daemonNotice ? (
            <div
              className={`callout ${daemonNotice.kind}`}
              role={daemonNotice.kind === 'danger' ? 'alert' : 'status'}
            >
              {daemonNotice.text}
            </div>
          ) : null}

          {daemonLoading && !daemon ? (
            <div className="skeleton-row" aria-busy="true">
              <div className="skeleton-bar skeleton-bar--short" />
              <div className="skeleton-bar skeleton-bar--wide" />
            </div>
          ) : daemon ? (
            <>
              <div className="daemon-meta-grid">
                <div className="daemon-meta-item">
                  <span className="field-hint">Endpoint and PID</span>
                  <span className="measure">
                    http://127.0.0.1:{daemon.port} · PID {daemon.pid}
                  </span>
                </div>
                <div className="daemon-meta-item">
                  <span className="field-hint">Log file</span>
                  <span className="measure">{daemon.log_path}</span>
                </div>
                <div className="daemon-meta-item">
                  <span className="field-hint">Version &amp; updates</span>
                  <div className="tag-row">
                    <span className="tag measure">v{daemon.version}</span>
                    {updateStatus?.update_available && updateStatus.latest_release ? (
                      <span className="tag accent measure">
                        v{updateStatus.latest_release.version} available
                      </span>
                    ) : updateStatus?.last_checked_at ? (
                      <span className="tag ok">Up to date</span>
                    ) : null}
                  </div>
                </div>
                <div className="daemon-meta-item">
                  <span className="field-hint">Last release check</span>
                  <span
                    className="measure"
                    title={
                      updateStatus?.last_checked_at
                        ? formatUtcTime(updateStatus.last_checked_at)
                        : undefined
                    }
                  >
                    {formatLocalTime(updateStatus?.last_checked_at)}
                  </span>
                </div>
              </div>

              {updateStatus?.check_error ? (
                <div className="callout warn" role="status">
                  <span>Last update check failed: {updateStatus.check_error}</span>
                </div>
              ) : null}

              <div className="head-actions">
                <button
                  type="button"
                  className="btn"
                  disabled={checkingUpdates}
                  aria-busy={checkingUpdates}
                  onClick={() => void handleCheckUpdates()}
                >
                  {checkingUpdates ? 'Checking...' : 'Check for updates'}
                </button>
                {updateStatus?.update_available && updateStatus.latest_release ? (
                  <button type="button" className="btn btn-primary" onClick={onOpenUpdateModal}>
                    Update to v{updateStatus.latest_release.version}
                  </button>
                ) : null}
                <button type="button" className="btn" onClick={onOpenWhatsNew}>
                  What&apos;s new
                </button>
              </div>

              <div className="stack-sm">
                <label className="rim-check-label" htmlFor="eng-autoupdate">
                  <input
                    id="eng-autoupdate"
                    type="checkbox"
                    className="rim-check"
                    checked={settings?.auto_update_check ?? true}
                    disabled={autoCheckBusy || saving || !settings}
                    onChange={(e) => void handleAutoUpdateToggle(e.target.checked)}
                  />
                  <span>Automatically check for updates every 6 hours</span>
                </label>
                {daemon.autostart.supported ? (
                  <label className="rim-check-label" htmlFor="eng-autostart">
                    <input
                      id="eng-autostart"
                      type="checkbox"
                      className="rim-check"
                      checked={daemon.autostart.enabled}
                      disabled={autostartBusy}
                      onChange={(e) => void handleAutostartToggle(e.target.checked)}
                    />
                    <span>Start Daemon automatically on login ({daemon.autostart.platform})</span>
                  </label>
                ) : null}
                <p className="field-hint">{daemon.autostart.note}</p>
                <p className="field-hint measure">
                  Terminal controls: 0x1-memory status · 0x1-memory stop · 0x1-memory logs
                </p>
              </div>
            </>
          ) : (
            <div className="empty-state">
              <p>Daemon metadata is currently unavailable.</p>
            </div>
          )}
        </div>
      </section>

      <ModalDialog
        open={confirmStopOpen}
        title="Stop background Daemon?"
        onClose={() => setConfirmStopOpen(false)}
        footer={
          <>
            <button type="button" className="btn" onClick={() => setConfirmStopOpen(false)}>
              Cancel
            </button>
            <button
              type="button"
              className="btn btn-danger"
              disabled={stopping}
              aria-busy={stopping}
              onClick={() => void handleConfirmStop()}
            >
              {stopping ? 'Stopping...' : 'Stop daemon'}
            </button>
          </>
        }
      >
        <p style={{ fontSize: 'var(--text-sm)', color: 'var(--color-ink-2)' }}>
          This Web UI, local MCP server, and AI client hooks will stop until you run{' '}
          <code className="measure">0x1-memory</code> or{' '}
          <code className="measure">npx 0x1-ai-memory</code> again in your terminal.
        </p>
      </ModalDialog>
    </div>
  );
}
