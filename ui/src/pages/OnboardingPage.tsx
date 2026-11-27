import { useCallback, useEffect, useRef, useState } from 'react';
import { api, DEFAULT_DAEMON_PORT } from '../api';
import { ModalDialog } from '../components/ModalDialog';
import type {
  ClientConnection,
  ConnectionPreview,
  ConnectionTestResult,
  DaemonInfo,
  SettingsView,
  UniversalMcpSnippets,
} from '../types';

type StepIndex = 0 | 1 | 2 | 3;

const STEPS: { index: StepIndex; num: string; label: string; title: string }[] = [
  { index: 0, num: '01', label: 'Welcome', title: 'Welcome to AI Memory' },
  { index: 1, num: '02', label: 'AI Clients', title: 'Connect your AI coding clients' },
  { index: 2, num: '03', label: 'Memory Engine', title: 'Choose your memory distillation engine' },
  { index: 3, num: '04', label: 'Ready', title: 'Your local memory Daemon is ready' },
];

const ENGINE_CARDS: {
  id: string;
  title: string;
  badge: string;
  model: string;
  baseUrl: string;
  desc: string;
  needsKey: boolean;
}[] = [
  {
    id: 'none',
    title: 'Distillation off',
    badge: 'Default',
    model: '',
    baseUrl: '',
    desc: 'Save memories manually. Search uses the embedding mode configured in Engine settings.',
    needsKey: false,
  },
  {
    id: 'openrouter',
    title: 'OpenRouter API',
    badge: 'Recommended API',
    model: 'google/gemini-2.5-flash-lite',
    baseUrl: 'https://openrouter.ai/api/v1',
    desc: 'Automatic per-turn distillation with a separate model and configurable daily limits.',
    needsKey: true,
  },
  {
    id: 'gemini',
    title: 'Google Gemini API',
    badge: 'Direct API',
    model: 'gemini-2.5-flash-lite',
    baseUrl: 'https://generativelanguage.googleapis.com/v1beta',
    desc: 'Direct JSON distillation via Gemini API. Supports env:GEMINI_API_KEY or a stored key.',
    needsKey: true,
  },
  {
    id: 'ollama',
    title: 'Ollama (local LLM)',
    badge: '$0 · offline',
    model: 'qwen2.5-coder:7b',
    baseUrl: 'http://127.0.0.1:11434/v1',
    desc: 'Runs on-device with a locally installed model on 127.0.0.1:11434.',
    needsKey: false,
  },
];

interface OnboardingPageProps {
  daemon: DaemonInfo | null;
  daemonLoading?: boolean;
  settings: SettingsView | null;
  onRefresh: () => Promise<void>;
  onComplete: () => Promise<void>;
}

export function OnboardingPage({
  daemon,
  daemonLoading = false,
  settings,
  onRefresh,
  onComplete,
}: OnboardingPageProps) {
  const [step, setStep] = useState<StepIndex>(0);
  const [maxVisited, setMaxVisited] = useState<StepIndex>(0);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const didMountRef = useRef(false);

  // Step 2: Clients state
  const [connections, setConnections] = useState<ClientConnection[]>([]);
  const [universal, setUniversal] = useState<UniversalMcpSnippets | null>(null);
  const [clientsLoading, setClientsLoading] = useState(true);
  const [clientsError, setClientsError] = useState<string | null>(null);
  const [clientFeedback, setClientFeedback] = useState<string | null>(null);
  const [previews, setPreviews] = useState<ConnectionPreview[]>([]);
  const [applyingClients, setApplyingClients] = useState(false);
  const [snippetTab, setSnippetTab] = useState<'http' | 'toml' | 'stdio'>('http');

  // Step 3: Engine state (preserved across Back/Next navigation)
  const [provider, setProvider] = useState(settings?.provider ?? 'none');
  const [model, setModel] = useState(settings?.model ?? '');
  const [baseUrl, setBaseUrl] = useState(settings?.base_url ?? '');
  const [apiKeyInput, setApiKeyInput] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [dailyCostCap, setDailyCostCap] = useState(String(settings?.daily_cost_cap_usd ?? 0.5));
  const [engineInitialized, setEngineInitialized] = useState(false);
  const [engineDirty, setEngineDirty] = useState(false);
  const [savingEngine, setSavingEngine] = useState(false);
  const [testingEngine, setTestingEngine] = useState(false);
  const [engineFeedback, setEngineFeedback] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);

  // Step 4: Auto-Start & Finish state
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [autostartNotice, setAutostartNotice] = useState<{
    kind: 'ok' | 'danger';
    text: string;
  } | null>(null);
  const [finishing, setFinishing] = useState(false);

  useEffect(() => {
    if (settings && !engineInitialized) {
      // Preserve custom/advanced providers (e.g. openai_compatible, cli_*) when re-running Onboarding.
      setProvider(settings.provider);
      setModel(settings.model);
      setBaseUrl(settings.base_url);
      setDailyCostCap(String(settings.daily_cost_cap_usd));
      setEngineInitialized(true);
    }
  }, [settings, engineInitialized]);

  const loadClients = useCallback(async () => {
    setClientsLoading(true);
    setClientsError(null);
    try {
      const [cRes, uRes] = await Promise.all([api.connections(), api.universalSnippets()]);
      setConnections(cRes.connections);
      setUniversal(uRes);
    } catch (e) {
      setClientsError(e instanceof Error ? e.message : 'Failed to scan local clients');
    } finally {
      setClientsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadClients();
  }, [loadClients]);

  useEffect(() => {
    const current = STEPS[step];
    document.title = `Onboarding (${current.num}): ${current.label} | AI Memory`;
    if (didMountRef.current) {
      headingRef.current?.focus();
    } else {
      didMountRef.current = true;
    }
  }, [step]);

  function goToStep(next: StepIndex) {
    setStep(next);
    if (next > maxVisited) {
      setMaxVisited(next);
    }
  }

  async function handleStepClick(next: StepIndex) {
    if (step === 2 && next === 3 && engineDirty) {
      const saved = await saveEngineConfig();
      if (!saved) return;
    }
    goToStep(next);
  }

  async function openSinglePreview(clientId: string) {
    setClientFeedback(null);
    try {
      const data = await api.previewConnect(clientId);
      setPreviews([data]);
    } catch (e) {
      setClientsError(e instanceof Error ? e.message : 'Failed to load diff preview');
    }
  }

  async function openConnectAllPreview() {
    const targets = connections.filter((c) => c.detected && !c.connected);
    if (targets.length === 0) return;
    setClientFeedback(null);
    try {
      const loaded = await Promise.all(targets.map((c) => api.previewConnect(c.client_id)));
      setPreviews(loaded);
    } catch (e) {
      setClientsError(e instanceof Error ? e.message : 'Failed to load combined diff preview');
    }
  }

  async function confirmApplyPreviews() {
    if (previews.length === 0) return;
    setApplyingClients(true);
    setClientsError(null);
    try {
      const connectedNames: string[] = [];
      for (const item of previews) {
        const { connection } = await api.connectClient(item.client_id);
        connectedNames.push(connection.display_name);
      }
      setPreviews([]);
      setClientFeedback(
        `Connected ${connectedNames.join(', ')}. Automatic backups saved to ~/.0x1-memory/backups/.`,
      );
      await Promise.all([loadClients(), onRefresh()]);
    } catch (e) {
      setClientsError(e instanceof Error ? e.message : 'Failed to apply client config');
    } finally {
      setApplyingClients(false);
    }
  }

  function selectEngineCard(cardId: string) {
    if (cardId === provider) return;
    setProvider(cardId);
    setApiKeyInput('');
    setShowKey(false);
    setEngineDirty(true);
    setTestResult(null);
    setEngineFeedback(null);
    const card = ENGINE_CARDS.find((c) => c.id === cardId);
    if (card) {
      setModel(card.model);
      setBaseUrl(card.baseUrl);
    }
  }

  async function saveEngineConfig(): Promise<boolean> {
    if (testingEngine || savingEngine) return false;
    if (!engineDirty) {
      return true;
    }
    setSavingEngine(true);
    setEngineFeedback(null);
    try {
      const patch: Record<string, unknown> = {
        provider,
        model,
        base_url: baseUrl,
        daily_cost_cap_usd: Number(dailyCostCap),
      };
      if (apiKeyInput.trim() !== '') {
        patch.api_key = apiKeyInput.trim();
      }
      await api.patchSettings(patch);
      setApiKeyInput('');
      setEngineDirty(false);
      await onRefresh();
      return true;
    } catch (e) {
      setEngineFeedback(e instanceof Error ? e.message : 'Failed to save engine settings');
      return false;
    } finally {
      setSavingEngine(false);
    }
  }

  async function handleTestEngine() {
    setTestingEngine(true);
    setEngineFeedback(null);
    setTestResult(null);
    try {
      const draft: Record<string, unknown> = {
        provider,
        model,
        base_url: baseUrl,
      };
      if (apiKeyInput.trim() !== '') {
        draft.api_key = apiKeyInput.trim();
      }
      const res = await api.testEngine(draft);
      setTestResult(res);
    } catch (e) {
      setEngineFeedback(e instanceof Error ? e.message : 'Failed to test connection');
    } finally {
      setTestingEngine(false);
    }
  }

  async function handleContinueFromEngine() {
    const saved = await saveEngineConfig();
    if (saved) {
      goToStep(3);
    }
  }

  async function handleToggleAutostart(nextEnabled: boolean) {
    setAutostartBusy(true);
    setAutostartNotice(null);
    try {
      const updated = await api.setAutostart(nextEnabled);
      setAutostartNotice({
        kind: 'ok',
        text: updated.enabled
          ? 'Auto-Start enabled for future logins.'
          : 'Auto-Start disabled. Your current Daemon stays running.',
      });
      await onRefresh();
    } catch (e) {
      setAutostartNotice({
        kind: 'danger',
        text: e instanceof Error ? e.message : 'Could not update Auto-Start',
      });
    } finally {
      setAutostartBusy(false);
    }
  }

  async function handleFinish() {
    setFinishing(true);
    try {
      await onComplete();
    } finally {
      setFinishing(false);
    }
  }

  const detectedUnconnected = connections.filter((c) => c.detected && !c.connected);
  const connectedCount = connections.filter((c) => c.connected).length;
  const activePort = daemon?.port ?? DEFAULT_DAEMON_PORT;
  const activeCard = ENGINE_CARDS.find((c) => c.id === provider);
  const isCustomProvider = !activeCard;
  const activeSnippet =
    snippetTab === 'http'
      ? (universal?.json_http ?? '')
      : snippetTab === 'toml'
        ? (universal?.toml_http ?? '')
        : (universal?.json_stdio ?? '');

  return (
    <div className="stack">
      <nav aria-label="Onboarding progress">
        <ol className="onboarding-steps">
          {STEPS.map((s) => {
            const isCurrent = step === s.index;
            const isDone = s.index < step || s.index <= maxVisited;
            return (
              <li key={s.index} className="onboarding-step-item">
                <button
                  type="button"
                  className="onboarding-step-btn"
                  aria-current={isCurrent ? 'step' : undefined}
                  data-done={s.index < step ? 'true' : undefined}
                  onClick={() => void handleStepClick(s.index)}
                >
                  <span className="onboarding-step-num">
                    {s.index < step ? `${s.num} · Done` : s.num}
                  </span>
                  <span className="onboarding-step-label">{s.label}</span>
                  {!isCurrent && isDone && s.index > step ? (
                    <span className="visually-hidden">(visited)</span>
                  ) : null}
                </button>
              </li>
            );
          })}
        </ol>
      </nav>

      {step === 0 ? (
        <section className="panel ticks">
          <div className="panel-body stack">
            <div>
              <h1 ref={headingRef} tabIndex={-1} className="page-title">
                {STEPS[0].title}
              </h1>
              <p className="page-subtitle">
                One local persistent memory store shared across Claude Code, Codex and
                Antigravity — running quietly in the background without burning coding quotas.
              </p>
            </div>

            <div className="metrics-strip" style={{ marginBottom: 0 }}>
              <div className="metric-cell">
                <span className="metric-label">Background Daemon</span>
                <span className="metric-value metric-value--signal">:{activePort}</span>
                <span className="metric-sub measure">
                  {daemon ? `PID ${daemon.pid} · running` : '127.0.0.1 only'}
                </span>
              </div>
              <div className="metric-cell">
                <span className="metric-label">Local storage</span>
                <span className="metric-value">SQLite</span>
                <span className="metric-sub measure">{daemon?.data_dir ?? '~/.0x1-memory'}</span>
              </div>
              <div className="metric-cell">
                <span className="metric-label">Default LLM spend</span>
                <span className="metric-value">$0.00</span>
                <span className="metric-sub">Zero silent fallback</span>
              </div>
              <div className="metric-cell">
                <span className="metric-label">Detected clients</span>
                <span className="metric-value">
                  {clientsLoading ? '...' : connections.filter((c) => c.detected).length}
                </span>
                <span className="metric-sub">Ready to inspect next</span>
              </div>
            </div>

            <div className="callout">
              <span>
                You stay in control: every client config edit previews exact diffs and saves a
                timestamped backup first.
              </span>
            </div>

            <div className="onboarding-foot">
              <button type="button" className="btn btn-ghost" onClick={() => void handleFinish()}>
                Skip to dashboard
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => goToStep(1)}
              >
                Start setup
              </button>
            </div>
          </div>
        </section>
      ) : null}

      {step === 1 ? (
        <section className="panel ticks">
          <div className="panel-body stack">
            <div className="page-head" style={{ marginBottom: 0 }}>
              <div>
                <h1 ref={headingRef} tabIndex={-1} className="page-title">
                  {STEPS[1].title}
                </h1>
                <p className="page-subtitle">
                  Connect any detected agent below, or copy the Universal MCP snippet for another
                  IDE. Every change previews diffs and writes backups first.
                </p>
              </div>
              <div className="head-actions">
                <button type="button" className="btn" onClick={() => void loadClients()}>
                  Re-scan
                </button>
                {detectedUnconnected.length > 0 ? (
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={() => void openConnectAllPreview()}
                  >
                    Connect all detected ({detectedUnconnected.length})
                  </button>
                ) : null}
              </div>
            </div>

            {clientsError ? (
              <div className="callout danger" role="alert">
                <span>{clientsError}</span>
                <button type="button" className="btn btn-danger" onClick={() => void loadClients()}>
                  Retry
                </button>
              </div>
            ) : null}
            {clientFeedback ? (
              <div className="callout ok" role="status">
                {clientFeedback}
              </div>
            ) : null}

            {clientsLoading ? (
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
            ) : connections.length === 0 ? (
              <div className="empty-state">
                <p>No local AI clients returned. Use the Universal MCP snippet below or retry scan.</p>
              </div>
            ) : (
              <div className="grid-2">
                {connections.map((c) => (
                  <div key={c.client_id} className="panel">
                    <div className="panel-header">
                      <div className="tag-row">
                        <h2 className="panel-title">{c.display_name}</h2>
                      </div>
                      <span
                        className={`tag ${
                          c.connected
                            ? 'ok'
                            : c.port_mismatch
                              ? 'danger'
                              : c.detected
                                ? 'warn'
                                : ''
                        }`}
                      >
                        {c.connected
                          ? 'Connected'
                          : c.port_mismatch
                            ? `Port mismatch (:${c.configured_port ?? '?'})`
                            : c.detected
                              ? 'Detected'
                              : 'Not found'}
                      </span>
                    </div>
                    <div className="panel-body stack-sm">
                      <p style={{ fontSize: 'var(--text-sm)', color: 'var(--color-ink-2)' }}>
                        {c.status_note}
                      </p>
                      {c.manual_step_hint ? (
                        <div className="callout warn">{c.manual_step_hint}</div>
                      ) : null}
                      <div>
                        {!c.connected ? (
                          <button
                            type="button"
                            className={`btn ${c.detected || c.port_mismatch ? 'btn-primary' : ''}`}
                            onClick={() => void openSinglePreview(c.client_id)}
                          >
                            {c.port_mismatch
                              ? `Reconnect to :${activePort}`
                              : 'Preview and connect'}
                          </button>
                        ) : (
                          <span className="field-hint">Wired to :{activePort}</span>
                        )}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}

            <details className="panel">
              <summary
                className="panel-header"
                style={{ cursor: 'pointer', listStyle: 'none' }}
              >
                <span className="panel-title">Universal MCP snippet (Cursor, Windsurf, Cline, Zed)</span>
                <span className="field-hint measure">{universal?.streamable_http_url}</span>
              </summary>
              <div className="panel-body stack-sm">
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
                  <button
                    type="button"
                    className="btn"
                    onClick={() => {
                      void navigator.clipboard.writeText(activeSnippet);
                      setClientFeedback('Copied Universal MCP snippet to clipboard.');
                    }}
                  >
                    Copy
                  </button>
                </div>
                <pre className="code-block">{activeSnippet}</pre>
              </div>
            </details>

            <div className="onboarding-foot">
              <button type="button" className="btn" onClick={() => goToStep(0)}>
                Back
              </button>
              <button type="button" className="btn btn-primary" onClick={() => goToStep(2)}>
                {connectedCount > 0 ? 'Continue' : 'Skip for now'}
              </button>
            </div>
          </div>
        </section>
      ) : null}

      {step === 2 ? (
        <section className="panel ticks">
          <form
            className="panel-body stack"
            onSubmit={(e) => {
              e.preventDefault();
              void handleContinueFromEngine();
            }}
          >
            <div>
              <h1 ref={headingRef} tabIndex={-1} className="page-title">
                {STEPS[2].title}
              </h1>
              <p className="page-subtitle">
                Choose an API or a local Ollama model to save memories automatically after each
                turn. You can also save memories manually.
              </p>
            </div>

            {engineFeedback ? (
              <div className="callout danger" role="alert">
                {engineFeedback}
              </div>
            ) : null}
            {testResult ? (
              <div className={`callout ${testResult.ok ? 'ok' : 'danger'}`} role="status">
                <span>{testResult.message}</span>
                <span className="measure">
                  {testResult.latency_ms} ms
                  {testResult.estimated_cost_usd > 0
                    ? ` · $${testResult.estimated_cost_usd.toPrecision(3)}`
                    : ''}
                </span>
              </div>
            ) : null}

            {isCustomProvider ? (
              <div className="callout">
                <span>
                  Currently using advanced provider <code className="measure">{provider}</code>.
                  Choose a preset below to replace it, or continue to keep your current setup.
                </span>
              </div>
            ) : null}

            <fieldset className="onboarding-fieldset" disabled={testingEngine || savingEngine}>
              <legend className="onboarding-legend">Distillation mode</legend>
              <div className="radio-grid">
                {ENGINE_CARDS.map((card) => (
                  <label key={card.id} className="radio-card" htmlFor={`onb-eng-${card.id}`}>
                    <input
                      id={`onb-eng-${card.id}`}
                      type="radio"
                      name="onboarding-provider"
                      value={card.id}
                      checked={provider === card.id}
                      onChange={() => selectEngineCard(card.id)}
                    />
                    <span className="radio-card-body">
                      <span className="radio-card-head">
                        <span className="radio-card-title">{card.title}</span>
                        <span className="tag">{card.badge}</span>
                      </span>
                      <span className="field-hint">{card.desc}</span>
                    </span>
                  </label>
                ))}
              </div>
            </fieldset>

            {provider !== 'none' && !isCustomProvider ? (
              <div className="stack">
                <div className="grid-2">
                  <div className="field">
                    <label htmlFor="onb-model">Model</label>
                    <input
                      id="onb-model"
                      name="model"
                      className="input input--mono"
                      autoCorrect="off"
                      autoCapitalize="none"
                      spellCheck={false}
                      value={model}
                      disabled={testingEngine || savingEngine}
                      onChange={(e) => {
                        setModel(e.target.value);
                        setTestResult(null);
                        setEngineDirty(true);
                      }}
                    />
                  </div>
                  <div className="field">
                    <label htmlFor="onb-cost-cap">Daily cost limit (USD)</label>
                    <input
                      id="onb-cost-cap"
                      name="daily_cost_cap_usd"
                      type="number"
                      min={0}
                      step="any"
                      required
                      aria-describedby="onb-cost-hint"
                      className="input input--mono"
                      value={dailyCostCap}
                      onChange={(e) => {
                        setDailyCostCap(e.target.value);
                        setEngineDirty(true);
                      }}
                    />
                    <span id="onb-cost-hint" className="field-hint">0 = Unlimited</span>
                  </div>
                </div>

                {activeCard?.needsKey ? (
                  <div className="field">
                    <label htmlFor="onb-api-key">
                      API key{' '}
                      {settings?.has_api_key && settings.provider === provider && settings.base_url === baseUrl
                        ? `(configured: ${settings.api_key_masked || 'from env'})`
                        : '(or env:OPENROUTER_API_KEY / env:GEMINI_API_KEY)'}
                    </label>
                    <div className="password-row">
                      <input
                        id="onb-api-key"
                        name="api_key"
                        type={showKey ? 'text' : 'password'}
                        className="input input--mono"
                        autoComplete="off"
                        autoCorrect="off"
                        autoCapitalize="none"
                        spellCheck={false}
                        aria-describedby="onb-key-hint"
                        placeholder="Paste key or env:VAR_NAME (optional now)"
                        value={apiKeyInput}
                        disabled={testingEngine || savingEngine}
                        onChange={(e) => {
                          setApiKeyInput(e.target.value);
                          setTestResult(null);
                          setEngineDirty(true);
                        }}
                      />
                      <button
                        type="button"
                        className="btn"
                        aria-pressed={showKey}
                        onClick={() => setShowKey((prev) => !prev)}
                      >
                        {showKey ? 'Hide' : 'Show'}
                      </button>
                    </div>
                    <span id="onb-key-hint" className="field-hint">
                      Stored locally in ~/.0x1-memory/memory.db. Never falls back to coding-agent
                      subscriptions if missing.
                    </span>
                  </div>
                ) : null}

                <div>
                  <button
                    type="button"
                    className="btn"
                    disabled={testingEngine || savingEngine}
                    aria-busy={testingEngine}
                    onClick={() => void handleTestEngine()}
                  >
                    {testingEngine ? 'Testing...' : 'Test distillation'}
                  </button>
                  <p className="field-hint">
                    Uses saved daily limits and a small sample that can use provider credits.
                  </p>
                </div>
              </div>
            ) : null}

            <div className="onboarding-foot">
              <button type="button" className="btn" onClick={() => goToStep(1)}>
                Back
              </button>
              <button
                type="submit"
                className="btn btn-primary"
                disabled={savingEngine || testingEngine}
                aria-busy={savingEngine}
              >
                {savingEngine ? 'Saving...' : 'Save and continue'}
              </button>
            </div>
          </form>
        </section>
      ) : null}

      {step === 3 ? (
        <section className="panel ticks">
          <div className="panel-body stack">
            <div>
              <h1 ref={headingRef} tabIndex={-1} className="page-title">
                {STEPS[3].title}
              </h1>
              <p className="page-subtitle">
                You can close this tab anytime — the Daemon stays running quietly in the background
                so hooks and MCP stay live.
              </p>
            </div>

            <div className="metrics-strip" style={{ marginBottom: 0 }}>
              <div className="metric-cell">
                <span className="metric-label">Daemon endpoint</span>
                <span className="metric-value metric-value--signal">:{activePort}</span>
                <span className="metric-sub measure">127.0.0.1:{activePort}</span>
              </div>
              <div className="metric-cell">
                <span className="metric-label">Connected clients</span>
                <span className="metric-value">{connectedCount}</span>
                <span className="metric-sub">Manage anytime in Connections</span>
              </div>
              <div className="metric-cell">
                <span className="metric-label">Memory engine</span>
                <span className="metric-value">{provider}</span>
                <span className="metric-sub measure">
                  Cap ${Number(dailyCostCap || 0).toFixed(2)}/day
                </span>
              </div>
              <div className="metric-cell">
                <span className="metric-label">Auto-Start</span>
                <span className="metric-value">
                  {daemon?.autostart.enabled ? 'ON' : daemon?.autostart.supported ? 'OFF' : 'Manual'}
                </span>
                <span className="metric-sub">{daemon?.autostart.platform ?? 'local'}</span>
              </div>
            </div>

            <div className="panel">
              <div className="panel-header">
                <h2 className="panel-title">Start Daemon automatically on login (optional)</h2>
                <span className="tag">{daemon?.autostart.supported ? 'Opt-in' : 'Manual'}</span>
              </div>
              <div className="panel-body stack-sm">
                {daemonLoading && !daemon ? (
                  <div className="skeleton-row" aria-busy="true">
                    <div className="skeleton-bar skeleton-bar--short" />
                    <div className="skeleton-bar skeleton-bar--wide" />
                  </div>
                ) : daemon ? (
                  <>
                    {daemon.autostart.supported ? (
                      <label className="rim-check-label" htmlFor="onb-autostart">
                        <input
                          id="onb-autostart"
                          type="checkbox"
                          className="rim-check"
                          checked={Boolean(daemon.autostart.enabled)}
                          disabled={autostartBusy}
                          onChange={(e) => void handleToggleAutostart(e.target.checked)}
                        />
                        <span>Enable Auto-Start after reboot ({daemon.autostart.platform})</span>
                      </label>
                    ) : null}
                    <p className="field-hint">{daemon.autostart.note}</p>
                  </>
                ) : (
                  <p className="field-hint">Auto-Start status is currently unavailable.</p>
                )}
                {autostartNotice ? (
                  <div
                    className={`callout ${autostartNotice.kind}`}
                    role={autostartNotice.kind === 'danger' ? 'alert' : 'status'}
                  >
                    {autostartNotice.text}
                  </div>
                ) : null}
              </div>
            </div>

            <div className="panel">
              <div className="panel-header">
                <h2 className="panel-title">Useful terminal commands</h2>
              </div>
              <div className="panel-body">
                <pre className="code-block">
                  {[
                    '0x1-memory status     # check Daemon health, Auto-Start, clients & spend',
                    '0x1-memory stop       # stop the background Daemon cleanly',
                    '0x1-memory logs       # inspect ~/.0x1-memory/daemon.log',
                    '0x1-memory            # start Daemon (or reopen this Web UI)',
                  ].join('\n')}
                </pre>
              </div>
            </div>

            <div className="onboarding-foot">
              <button type="button" className="btn" onClick={() => goToStep(2)}>
                Back
              </button>
              <button
                type="button"
                className="btn btn-primary"
                disabled={finishing}
                aria-busy={finishing}
                onClick={() => void handleFinish()}
              >
                {finishing ? 'Opening...' : 'Enter Dashboard'}
              </button>
            </div>
          </div>
        </section>
      ) : null}

      <ModalDialog
        open={previews.length > 0}
        title={
          previews.length === 1
            ? `Connect ${previews[0].display_name}`
            : `Connect ${previews.length} detected clients`
        }
        onClose={() => setPreviews([])}
        footer={
          <>
            <button type="button" className="btn" onClick={() => setPreviews([])}>
              Cancel
            </button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={applyingClients}
              aria-busy={applyingClients}
              onClick={() => void confirmApplyPreviews()}
            >
              {applyingClients
                ? 'Applying...'
                : previews.length === 1
                  ? 'Backup and connect'
                  : `Backup and connect all (${previews.length})`}
            </button>
          </>
        }
      >
        <div className="stack">
          <p className="field-hint">
            Timestamped backups are saved automatically to ~/.0x1-memory/backups/ before any edit.
          </p>
          {previews.map((item) => (
            <div key={item.client_id} className="stack-sm">
              <h3 className="panel-title">{item.display_name}</h3>
              <p style={{ fontSize: 'var(--text-sm)', color: 'var(--color-ink-2)' }}>
                {item.summary}
              </p>
              <pre className="code-block">{item.diff_preview}</pre>
              {item.manual_step_hint ? (
                <div className="callout warn">{item.manual_step_hint}</div>
              ) : null}
            </div>
          ))}
        </div>
      </ModalDialog>
    </div>
  );
}
