import { useEffect, useState, type FormEvent } from 'react';
import { api, formatUtcTime } from '../api';
import type { ConnectionTestResult, SettingsView, UsageEntry } from '../types';

const PROVIDER_PRESETS: Record<
  string,
  { label: string; model: string; baseUrl: string; note: string }
> = {
  none: {
    label: 'Zero-LLM safe mode ($0)',
    model: '',
    baseUrl: '',
    note: 'No background LLM calls. Hybrid FTS5 and vector search plus explicit memory_save run at $0 cost.',
  },
  openrouter: {
    label: 'OpenRouter (recommended cheap API)',
    model: 'google/gemini-2.5-flash-lite',
    baseUrl: 'https://openrouter.ai/api/v1',
    note: 'Decoupled distillation via OpenRouter with reasoning disabled and exact cost tracking. Supports env:OPENROUTER_API_KEY.',
  },
  gemini: {
    label: 'Google Gemini API',
    model: 'gemini-2.5-flash-lite',
    baseUrl: 'https://generativelanguage.googleapis.com/v1beta',
    note: 'Direct JSON distillation via Gemini API. Supports env:GEMINI_API_KEY.',
  },
  ollama: {
    label: 'Ollama (local LLM)',
    model: 'qwen2.5-coder:7b',
    baseUrl: 'http://127.0.0.1:11434/v1',
    note: 'Zero external traffic and $0 API cost using your local Ollama server.',
  },
  openai_compatible: {
    label: 'OpenAI-compatible endpoint',
    model: 'gpt-4.1-nano',
    baseUrl: 'https://api.openai.com/v1',
    note: 'Works with OpenAI, LM Studio, DeepSeek, Groq, vLLM, or any /v1/chat/completions server.',
  },
  cli_claude: {
    label: 'Opt-in: Claude CLI (claude -p)',
    model: 'haiku',
    baseUrl: '',
    note: 'OPT-IN WARNING: Uses your Claude rolling quota and carries Anthropic consumer OAuth ToS caveats. Protected by OX1_MEMORY_INTERNAL=1 and disabled hooks.',
  },
  cli_codex: {
    label: 'Opt-in: Codex CLI (codex exec)',
    model: '',
    baseUrl: '',
    note: 'OPT-IN WARNING: Uses your Codex or ChatGPT rolling quota. Runs ephemeral read-only with features.hooks=false.',
  },
  cli_agy: {
    label: 'Opt-in: Antigravity CLI (agy -p)',
    model: '',
    baseUrl: '',
    note: 'OPT-IN WARNING: Uses Antigravity quota. Guarded by OX1_MEMORY_INTERNAL=1 recursion prevention.',
  },
};

interface EnginePageProps {
  onSaved: () => Promise<void>;
}

export function EnginePage({ onSaved }: EnginePageProps) {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [usage, setUsage] = useState<UsageEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [provider, setProvider] = useState('none');
  const [model, setModel] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [apiKeyInput, setApiKeyInput] = useState('');
  const [dailyTokenCap, setDailyTokenCap] = useState(250000);
  const [dailyCostCap, setDailyCostCap] = useState(0.5);
  const [sessionBudget, setSessionBudget] = useState(450);
  const [embeddingMode, setEmbeddingMode] = useState('local_onnx');
  const [retentionDays, setRetentionDays] = useState(14);
  const [idleSeconds, setIdleSeconds] = useState(120);

  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [saveNotice, setSaveNotice] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const [s, u] = await Promise.all([api.settings(), api.usageHistory()]);
      setSettings(s);
      setUsage(u.usage);
      setProvider(s.provider);
      setModel(s.model);
      setBaseUrl(s.base_url);
      setDailyTokenCap(s.daily_token_cap);
      setDailyCostCap(s.daily_cost_cap_usd);
      setSessionBudget(s.session_start_token_budget);
      setEmbeddingMode(s.embedding_mode);
      setRetentionDays(s.raw_event_retention_days);
      setIdleSeconds(s.distill_idle_seconds);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load settings');
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  function onProviderChange(next: string) {
    setProvider(next);
    const preset = PROVIDER_PRESETS[next];
    if (preset) {
      setModel(preset.model);
      setBaseUrl(preset.baseUrl);
    }
  }

  async function handleSave(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaveNotice(null);
    try {
      const patch: Record<string, unknown> = {
        provider,
        model,
        base_url: baseUrl,
        daily_token_cap: Number(dailyTokenCap),
        daily_cost_cap_usd: Number(dailyCostCap),
        session_start_token_budget: Number(sessionBudget),
        embedding_mode: embeddingMode,
        raw_event_retention_days: Number(retentionDays),
        distill_idle_seconds: Number(idleSeconds),
      };
      if (apiKeyInput.trim() !== '') {
        patch.api_key = apiKeyInput.trim();
      }
      const updated = await api.patchSettings(patch);
      setSettings(updated);
      setApiKeyInput('');
      setSaveNotice('Settings saved.');
      await onSaved();
    } finally {
      setSaving(false);
    }
  }

  async function handleTest() {
    setTesting(true);
    setTestResult(null);
    try {
      const res = await api.testEngine();
      setTestResult(res);
      await Promise.all([load(), onSaved()]);
    } finally {
      setTesting(false);
    }
  }

  const preset = PROVIDER_PRESETS[provider];

  return (
    <div className="stack">
      <div className="page-head" style={{ marginBottom: 0 }}>
        <div>
          <h1 className="page-title">Memory engine and budget guard</h1>
          <p className="page-subtitle">
            Keep coding agents focused on coding. Distill turns using a cheap decoupled API or local
            model with zero silent fallback.
          </p>
        </div>
        <span className="tag ok">Zero silent fallback enforced</span>
      </div>

      {error ? (
        <div className="callout danger">
          <span>Settings error: {error}</span>
          <button type="button" className="btn btn-danger" onClick={() => void load()}>
            Retry
          </button>
        </div>
      ) : null}

      <form className="panel" onSubmit={handleSave}>
        <div className="panel-header">
          <h2 className="panel-title">Distillation provider and hard caps</h2>
          <div className="head-actions">
            <button
              type="button"
              className="btn"
              disabled={testing || loading}
              aria-busy={testing}
              onClick={() => void handleTest()}
            >
              {testing ? 'Testing...' : 'Test connection'}
            </button>
            <button
              type="submit"
              className="btn btn-primary"
              disabled={saving || loading}
              aria-busy={saving}
            >
              {saving ? 'Saving...' : 'Save settings'}
            </button>
          </div>
        </div>

        <div className="panel-body stack">
          {saveNotice ? <div className="callout ok">{saveNotice}</div> : null}
          {testResult ? (
            <div className={`callout ${testResult.ok ? 'ok' : 'danger'}`}>
              <span>{testResult.message}</span>
              <span className="measure">{testResult.latency_ms} ms</span>
            </div>
          ) : null}

          <div className="grid-2">
            <div className="field">
              <label htmlFor="eng-provider">Memory LLM provider</label>
              <select
                id="eng-provider"
                className="select"
                value={provider}
                onChange={(e) => onProviderChange(e.target.value)}
              >
                {Object.entries(PROVIDER_PRESETS).map(([k, v]) => (
                  <option key={k} value={k}>
                    {v.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="field">
              <label htmlFor="eng-model">Model</label>
              <input
                id="eng-model"
                className="input input--mono"
                placeholder="google/gemini-2.5-flash-lite"
                value={model}
                onChange={(e) => setModel(e.target.value)}
              />
            </div>
          </div>

          {preset ? (
            <div className={`callout ${provider.startsWith('cli_') ? 'warn' : ''}`}>
              {preset.note}
            </div>
          ) : null}

          {provider !== 'none' && !provider.startsWith('cli_') ? (
            <div className="grid-2">
              <div className="field">
                <label htmlFor="eng-url">Base URL</label>
                <input
                  id="eng-url"
                  className="input input--mono"
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                />
              </div>
              <div className="field">
                <label htmlFor="eng-key">
                  API key{' '}
                  {settings?.has_api_key
                    ? `(configured: ${settings.api_key_masked || 'from env'})`
                    : '(or env:OPENROUTER_API_KEY)'}
                </label>
                <input
                  id="eng-key"
                  type="password"
                  className="input input--mono"
                  autoComplete="off"
                  placeholder="Leave blank to keep existing, or enter key / env:VAR_NAME"
                  value={apiKeyInput}
                  onChange={(e) => setApiKeyInput(e.target.value)}
                />
              </div>
            </div>
          ) : null}

          <div className="grid-2">
            <div className="field">
              <label htmlFor="eng-tok">Hard daily token cap</label>
              <span className="field-hint">Blocks distillation automatically once reached</span>
              <input
                id="eng-tok"
                type="number"
                min={1000}
                step={10000}
                required
                className="input input--mono"
                value={dailyTokenCap}
                onChange={(e) => setDailyTokenCap(Number(e.target.value))}
              />
            </div>
            <div className="field">
              <label htmlFor="eng-cost">Hard daily cost cap (USD)</label>
              <span className="field-hint">Recommended: $0.25 to $1.00 per day</span>
              <input
                id="eng-cost"
                type="number"
                min={0}
                step={0.05}
                required
                className="input input--mono"
                value={dailyCostCap}
                onChange={(e) => setDailyCostCap(Number(e.target.value))}
              />
            </div>
          </div>

          <div className="grid-2">
            <div className="field">
              <label htmlFor="eng-emb">Embeddings mode (1 runtime, no Python)</label>
              <select
                id="eng-emb"
                className="select"
                value={embeddingMode}
                onChange={(e) => setEmbeddingMode(e.target.value)}
              >
                <option value="local_onnx">Local ONNX (fastembed 384d, free and offline)</option>
                <option value="fts_only">FTS5 BM25 only (zero model download)</option>
                <option value="api">API embeddings (uses configured base URL)</option>
              </select>
            </div>
            <div className="field">
              <label htmlFor="eng-brief">SessionStart briefing budget (tokens)</label>
              <span className="field-hint">Default 450 tokens for lean progressive disclosure</span>
              <input
                id="eng-brief"
                type="number"
                min={100}
                max={2000}
                required
                className="input input--mono"
                value={sessionBudget}
                onChange={(e) => setSessionBudget(Number(e.target.value))}
              />
            </div>
          </div>

          <div className="grid-2">
            <div className="field">
              <label htmlFor="eng-ret">Raw event retention (days)</label>
              <input
                id="eng-ret"
                type="number"
                min={1}
                max={365}
                required
                className="input input--mono"
                value={retentionDays}
                onChange={(e) => setRetentionDays(Number(e.target.value))}
              />
            </div>
            <div className="field">
              <label htmlFor="eng-idle">Crash-recovery idle sweep (seconds)</label>
              <input
                id="eng-idle"
                type="number"
                min={15}
                max={3600}
                required
                className="input input--mono"
                value={idleSeconds}
                onChange={(e) => setIdleSeconds(Number(e.target.value))}
              />
            </div>
          </div>
        </div>
      </form>

      <section className="panel">
        <div className="panel-header">
          <h2 className="panel-title">Usage ledger (last 50 calls)</h2>
          <span className="field-hint">Every background LLM call is logged with UTC time</span>
        </div>
        <div className="table-wrap">
          {loading ? (
            <div className="skeleton-row">
              <div className="skeleton-bar skeleton-bar--wide" />
            </div>
          ) : usage.length === 0 ? (
            <div className="empty-state">
              <p>No background LLM calls recorded yet ($0.0000 USD spent).</p>
            </div>
          ) : (
            <table>
              <thead>
                <tr>
                  <th>Time (UTC)</th>
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
                    <td className="measure">{formatUtcTime(u.created_at)}</td>
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
      </section>
    </div>
  );
}
