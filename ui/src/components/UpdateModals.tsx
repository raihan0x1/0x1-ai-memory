import { useEffect, useRef, useState, type ReactNode } from 'react';
import { api } from '../api';
import type { DaemonInfo, UpdateStatusResponse, WhatsNewInfo } from '../types';
import { IconCheck, IconCopy } from './Icons';
import { ModalDialog } from './ModalDialog';

function renderInlineMarkdown(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const tokenRe = /(`[^`]+`|\[[^\]]+\]\(https:\/\/[^\s)]+\)|\*\*[^*]+\*\*|https:\/\/[^\s)]+)/g;
  let last = 0;
  let match: RegExpExecArray | null = tokenRe.exec(text);
  let idx = 0;
  while (match !== null) {
    if (match.index > last) {
      out.push(text.slice(last, match.index));
    }
    const token = match[0];
    idx += 1;
    if (token.startsWith('`') && token.endsWith('`')) {
      out.push(
        <code key={`c-${idx}`} className="measure">
          {token.slice(1, -1)}
        </code>,
      );
    } else if (token.startsWith('**') && token.endsWith('**')) {
      out.push(<strong key={`b-${idx}`}>{token.slice(2, -2)}</strong>);
    } else if (token.startsWith('[')) {
      const sep = token.indexOf('](');
      const label = token.slice(1, sep);
      const href = token.slice(sep + 2, -1);
      if (/^https:\/\//i.test(href)) {
        out.push(
          <a key={`l-${idx}`} href={href} target="_blank" rel="noopener noreferrer" className="release-notes-link">
            {label}
          </a>,
        );
      } else {
        out.push(label);
      }
    } else if (/^https:\/\//i.test(token)) {
      out.push(
        <a key={`u-${idx}`} href={token} target="_blank" rel="noopener noreferrer" className="release-notes-link">
          {token}
        </a>,
      );
    } else {
      out.push(token);
    }
    last = match.index + token.length;
    match = tokenRe.exec(text);
  }
  if (last < text.length) {
    out.push(text.slice(last));
  }
  return out;
}

export function ReleaseNotesView({ body }: { body: string }) {
  const lines = body.replace(/\r\n/g, '\n').split('\n');
  const blocks: ReactNode[] = [];
  const seenKeys = new Map<string, number>();
  let bullets: string[] = [];

  const nextKey = (prefix: string, content: string) => {
    const rawKey = `${prefix}:${content.slice(0, 32)}`;
    const n = (seenKeys.get(rawKey) ?? 0) + 1;
    seenKeys.set(rawKey, n);
    return `${rawKey}#${n}`;
  };

  const flushBullets = (hint: string) => {
    if (bullets.length === 0) return;
    const items = bullets;
    bullets = [];
    blocks.push(
      <ul key={nextKey('ul', hint)} className="release-notes-list">
        {items.map((item) => (
          <li key={nextKey('li', item)}>{renderInlineMarkdown(item)}</li>
        ))}
      </ul>,
    );
  };

  for (const raw of lines) {
    const trimmed = raw.trim();
    if (!trimmed) {
      flushBullets('gap');
      continue;
    }
    if (trimmed.startsWith('### ') || trimmed.startsWith('## ') || trimmed.startsWith('# ')) {
      flushBullets(trimmed);
      const text = trimmed.replace(/^#{1,3}\s+/, '');
      blocks.push(
        <h3 key={nextKey('h', text)} className="release-notes-heading">
          {renderInlineMarkdown(text)}
        </h3>,
      );
      continue;
    }
    if (trimmed.startsWith('- ') || trimmed.startsWith('* ')) {
      bullets.push(trimmed.slice(2));
      continue;
    }
    flushBullets(trimmed);
    blocks.push(
      <p key={nextKey('p', trimmed)} className="release-notes-para">
        {renderInlineMarkdown(trimmed)}
      </p>,
    );
  }
  flushBullets('end');

  return <div className="release-notes">{blocks}</div>;
}

function CopyCommandBlock({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      // Clipboard API unavailable in restricted contexts
    }
  }

  return (
    <div className="copy-cmd-box">
      <code className="measure copy-cmd-text">{command}</code>
      <button type="button" className="btn-subtle" onClick={() => void handleCopy()}>
        {copied ? <IconCheck size={12} /> : <IconCopy size={12} />}
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  );
}

function safeHttpsUrl(raw?: string | null): string | null {
  if (!raw) return null;
  const trimmed = raw.trim();
  return /^https:\/\//i.test(trimmed) ? trimmed : null;
}

const PROGRESS_STEPS: { id: string; label: string; minStep: number }[] = [
  { id: 'downloading', label: 'Download release archive & checksums', minStep: 1 },
  { id: 'verifying', label: 'Verify SHA-256 signature', minStep: 2 },
  { id: 'smoke_testing', label: 'Extract & smoke-test new binary', minStep: 3 },
  { id: 'replacing', label: 'Swap binary & cache release notes', minStep: 4 },
  { id: 'restarting', label: 'Restart Daemon & reload Web UI', minStep: 5 },
];

function stageToStep(stage: string, reconnecting: boolean): number {
  if (reconnecting || stage === 'restarting') return 5;
  switch (stage) {
    case 'downloading':
      return 1;
    case 'verifying':
      return 2;
    case 'extracting':
    case 'smoke_testing':
      return 3;
    case 'replacing':
      return 4;
    default:
      return 0;
  }
}

interface UpdateModalProps {
  open: boolean;
  status: UpdateStatusResponse | null;
  daemon: DaemonInfo | null;
  onClose: () => void;
  onStatusChange: (next: UpdateStatusResponse) => void;
}

export function UpdateModal({
  open,
  status,
  daemon,
  onClose,
  onStatusChange,
}: UpdateModalProps) {
  const [starting, setStarting] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);
  const [reconnectTimedOut, setReconnectTimedOut] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const statusRef = useRef(status);
  statusRef.current = status;
  const lastProgressStageRef = useRef<string>('idle');

  const runtime = status?.runtime;
  const activeStage = runtime?.stage ?? 'idle';
  if (activeStage !== 'idle' && activeStage !== 'error') {
    lastProgressStageRef.current = activeStage;
  }
  const isUpdating = Boolean(open && (starting || runtime?.in_progress || reconnecting));
  const targetVersion = runtime?.target_version ?? status?.latest_release?.version ?? null;

  useEffect(() => {
    if (!open) {
      setStarting(false);
      setReconnecting(false);
      setReconnectTimedOut(false);
      setLocalError(null);
      lastProgressStageRef.current = 'idle';
    }
  }, [open]);

  // React immediately to SSE-pushed stage changes without waiting for the next interval tick
  useEffect(() => {
    if (!open) return;
    if (runtime?.stage === 'restarting') {
      setReconnecting(true);
    } else if (runtime?.stage === 'error') {
      setStarting(false);
    }
  }, [open, runtime?.stage]);

  // Poll /api/updates/status while the Daemon is staging/verifying/replacing
  useEffect(() => {
    if (!open || (!starting && !runtime?.in_progress) || reconnecting) return;

    const timer = window.setInterval(async () => {
      try {
        const next = await api.updateStatus();
        onStatusChange(next);
        if (next.runtime.stage === 'restarting') {
          setReconnecting(true);
        } else if (next.runtime.stage === 'error') {
          setStarting(false);
        } else if (!next.runtime.in_progress && next.runtime.stage === 'idle') {
          setStarting(false);
        }
      } catch (err) {
        const stage = statusRef.current?.runtime.stage;
        if (stage === 'replacing' || stage === 'restarting') {
          setReconnecting(true);
        } else {
          setLocalError(err instanceof Error ? err.message : 'Lost connection while checking update status');
          setStarting(false);
        }
      }
    }, 450);

    return () => window.clearInterval(timer);
  }, [open, starting, runtime?.in_progress, reconnecting, onStatusChange]);

  // Poll /health until the restarted Daemon responds with the updated version, then reload UI
  useEffect(() => {
    if (!open || !reconnecting || reconnectTimedOut) return;

    const startedAt = Date.now();
    const expected = targetVersion?.replace(/^v/, '');
    const previous = statusRef.current?.current_version.replace(/^v/, '');

    const timer = window.setInterval(async () => {
      if (Date.now() - startedAt > 20_000) {
        window.clearInterval(timer);
        setReconnectTimedOut(true);
        setReconnecting(false);
        setStarting(false);
        return;
      }
      try {
        const health = await api.health();
        const ver = health.version.replace(/^v/, '');
        if (health.ok && ((expected && ver === expected) || (previous && ver !== previous))) {
          window.clearInterval(timer);
          window.location.reload();
        }
      } catch {
        // Daemon still restarting & rebinding port
      }
    }, 500);

    return () => window.clearInterval(timer);
  }, [open, reconnecting, reconnectTimedOut, targetVersion]);

  async function handleStartUpdate() {
    setStarting(true);
    setLocalError(null);
    setReconnectTimedOut(false);
    setReconnecting(false);
    try {
      const next = await api.applyUpdate();
      onStatusChange(next);
      if (next.runtime.stage === 'restarting') {
        setReconnecting(true);
      }
    } catch (err) {
      setLocalError(err instanceof Error ? err.message : 'Failed to start update');
      setStarting(false);
    }
  }

  if (!status?.latest_release) return null;

  const release = status.latest_release;
  const releaseHref = safeHttpsUrl(release.html_url);
  const errMessage = localError ?? (activeStage === 'error' ? runtime?.error : null);
  const failedPostSwap = lastProgressStageRef.current === 'restarting';
  const currentStep = stageToStep(activeStage, reconnecting);
  const lockClose = isUpdating && !reconnectTimedOut && !errMessage;

  return (
    <ModalDialog
      open={open}
      title={`Update Daemon to v${release.version}`}
      preventClose={lockClose}
      onClose={onClose}
      footer={
        lockClose ? undefined : (
          <>
            <button type="button" className="btn" onClick={onClose}>
              Close
            </button>
            {releaseHref ? (
              <a
                className="btn"
                href={releaseHref}
                target="_blank"
                rel="noopener noreferrer"
              >
                View on GitHub
              </a>
            ) : null}
            {reconnectTimedOut ? (
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => {
                  setReconnectTimedOut(false);
                  setReconnecting(true);
                }}
              >
                Retry reconnect
              </button>
            ) : status.capability.supported ? (
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void handleStartUpdate()}
              >
                {errMessage ? 'Retry update' : 'Update & restart Daemon'}
              </button>
            ) : null}
          </>
        )
      }
    >
      <div className="stack">
        <div className="tag-row">
          <span className="tag measure">Current v{status.current_version}</span>
          <span className="tag accent measure">Latest v{release.version}</span>
          <span className="tag measure">{status.capability.install_kind}</span>
        </div>

        {reconnectTimedOut ? (
          <div className="callout danger" role="alert">
            <div className="stack-sm">
              <strong>Daemon did not respond within 20 seconds after restart.</strong>
              <span>
                Check the Daemon log at <code className="measure">{daemon?.log_path ?? '~/.0x1-memory/daemon.log'}</code>{' '}
                or run the restart command below:
              </span>
              <CopyCommandBlock command="0x1-memory restart" />
            </div>
          </div>
        ) : null}

        {errMessage ? (
          <div className="callout danger" role="alert">
            <div className="stack-sm">
              <strong>Update failed: {errMessage}</strong>
              <span>
                {failedPostSwap
                  ? `The binary swap ran, but restarting the Daemon hit an error. Check ${daemon?.log_path ?? '~/.0x1-memory/daemon.log'} or run:`
                  : 'Your running Daemon binary was not modified. You can retry or update manually:'}
              </span>
              <CopyCommandBlock
                command={failedPostSwap ? '0x1-memory restart' : status.capability.manual_command}
              />
            </div>
          </div>
        ) : null}

        {isUpdating ? (
          <div className="stack-sm">
            <progress
              className="update-progress"
              max={PROGRESS_STEPS.length}
              value={currentStep}
              aria-label="Daemon update progress"
            />
            <ol className="update-steps">
              {PROGRESS_STEPS.map((step) => {
                const state =
                  currentStep > step.minStep
                    ? 'done'
                    : currentStep === step.minStep
                      ? 'active'
                      : 'pending';
                return (
                  <li key={step.id} className={`update-step update-step--${state}`}>
                    <span className="update-step-dot" aria-hidden="true" />
                    <span>{step.label}</span>
                  </li>
                );
              })}
            </ol>
            <p className="field-hint">
              The Web UI will reload automatically once the new Daemon answers on port {daemon?.port ?? 10101}.
            </p>
          </div>
        ) : !status.capability.supported ? (
          <div className="callout warn">
            <div className="stack-sm">
              <strong>1-click self-update is unavailable for this install.</strong>
              <span>{status.capability.reason}</span>
              <CopyCommandBlock command={status.capability.manual_command} />
            </div>
          </div>
        ) : null}

        {release.body.trim() ? (
          <div className="stack-sm">
            <span className="field-label">Release notes ({release.tag_name})</span>
            <ReleaseNotesView body={release.body} />
          </div>
        ) : null}
      </div>
    </ModalDialog>
  );
}

interface WhatsNewModalProps {
  open: boolean;
  whatsNew: WhatsNewInfo | null;
  autoTriggered: boolean;
  onClose: () => void;
  onRetryFetch: () => Promise<void>;
}

export function WhatsNewModal({
  open,
  whatsNew,
  autoTriggered,
  onClose,
  onRetryFetch,
}: WhatsNewModalProps) {
  const [retrying, setRetrying] = useState(false);
  const [retryError, setRetryError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) setRetryError(null);
  }, [open]);

  if (!whatsNew) return null;

  const changelogHref = safeHttpsUrl(whatsNew.compare_url ?? whatsNew.html_url);

  async function handleRetry() {
    setRetrying(true);
    setRetryError(null);
    try {
      await onRetryFetch();
    } catch (err) {
      setRetryError(err instanceof Error ? err.message : 'Failed to load release notes');
    } finally {
      setRetrying(false);
    }
  }

  return (
    <ModalDialog
      open={open}
      title={
        autoTriggered
          ? `Updated to v${whatsNew.version}`
          : `What's new in v${whatsNew.version}`
      }
      onClose={onClose}
      footer={
        <>
          {changelogHref ? (
            <a
              className="btn"
              href={changelogHref}
              target="_blank"
              rel="noopener noreferrer"
            >
              Full changelog on GitHub
            </a>
          ) : null}
          <button type="button" className="btn btn-primary" onClick={onClose}>
            Got it
          </button>
        </>
      }
    >
      <div className="stack">
        {whatsNew.previous_version ? (
          <div className="tag-row">
            <span className="tag ok measure">
              v{whatsNew.previous_version} → v{whatsNew.version}
            </span>
          </div>
        ) : (
          <div className="tag-row">
            <span className="tag ok measure">Running v{whatsNew.version}</span>
          </div>
        )}

        {whatsNew.body ? (
          <ReleaseNotesView body={whatsNew.body} />
        ) : (
          <div className="stack-sm">
            <p className="field-hint">
              0x1-ai-memory Daemon is running v{whatsNew.version}. Release notes are not cached locally yet
              (for example after an offline CLI update).
            </p>
            {retryError ? (
              <div className="callout danger" role="alert">
                <span>Could not load release notes: {retryError}</span>
              </div>
            ) : null}
            <div>
              <button
                type="button"
                className="btn"
                disabled={retrying}
                aria-busy={retrying}
                onClick={() => void handleRetry()}
              >
                {retrying ? 'Loading release notes...' : 'Retry loading release notes'}
              </button>
            </div>
          </div>
        )}
      </div>
    </ModalDialog>
  );
}
