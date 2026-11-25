import { useEffect, useState } from 'react';
import { api } from '../api';
import { IconCheck, IconCopy } from './Icons';
import { ModalDialog } from './ModalDialog';
import { RimSelect } from './RimSelect';
import type { ContextPreviewResponse, Project } from '../types';

interface BriefingPreviewModalProps {
  open: boolean;
  initialProjectId?: string;
  projects: Project[];
  onClose: () => void;
  onChanged: () => Promise<void>;
}

export function BriefingPreviewModal({
  open,
  initialProjectId,
  projects,
  onClose,
  onChanged,
}: BriefingPreviewModalProps) {
  const [selectedProjectId, setSelectedProjectId] = useState(
    initialProjectId || projects[0]?.id || '',
  );
  const [preview, setPreview] = useState<ContextPreviewResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (open) {
      setSelectedProjectId((prev) => initialProjectId || prev || projects[0]?.id || '');
      setCopied(false);
    }
  }, [open, initialProjectId, projects]);

  async function loadPreview(pid: string) {
    setLoading(true);
    setError(null);
    try {
      const res = await api.contextPreview(pid || undefined);
      setPreview(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load briefing preview');
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .contextPreview(selectedProjectId || undefined)
      .then((res) => {
        if (!cancelled) setPreview(res);
      })
      .catch((e) => {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : 'Failed to load briefing preview');
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, selectedProjectId]);

  async function handleUnpin(id: string) {
    setError(null);
    try {
      await api.updateMemory(id, { status: 'active' });
      const res = await api.contextPreview(selectedProjectId || undefined);
      setPreview(res);
      await onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to unpin memory');
    }
  }

  async function handleCopy() {
    if (!preview?.briefing) return;
    await navigator.clipboard.writeText(preview.briefing);
    setCopied(true);
    setTimeout(() => setCopied(false), 1800);
  }

  const budget = Math.max(1, preview?.token_budget ?? 450);
  const pinnedTok = preview?.sections.find((s) => s.kind === 'pinned')?.tokens ?? 0;
  const projectTok = preview?.sections.find((s) => s.kind === 'project')?.tokens ?? 0;
  const globalTok = preview?.sections.find((s) => s.kind === 'global')?.tokens ?? 0;
  const approx = preview?.approx_tokens ?? 0;
  const archive = preview?.archive_approx_tokens ?? 0;
  const savedPct =
    archive > approx && archive > 0 ? Math.round(((archive - approx) / archive) * 100) : null;

  const projectOptions = [
    { value: '', label: 'Global only (no project)' },
    ...projects.map((p) => ({ value: p.id, label: p.name, meta: p.privacy_mode })),
  ];

  return (
    <ModalDialog
      open={open}
      title="SessionStart context briefing"
      onClose={onClose}
      footer={
        <>
          <button
            type="button"
            className="btn"
            disabled={!preview?.briefing}
            onClick={() => void handleCopy()}
          >
            {copied ? <IconCheck size={13} /> : <IconCopy size={13} />}
            {copied ? 'Copied' : 'Copy briefing'}
          </button>
          <button type="button" className="btn btn-primary" onClick={onClose}>
            Done
          </button>
        </>
      }
    >
      <div className="toolbar" style={{ marginBottom: 0, justifyContent: 'space-between' }}>
        <RimSelect
          ariaLabel="Preview project briefing"
          value={selectedProjectId}
          options={projectOptions}
          onChange={setSelectedProjectId}
        />
        <span className="field-hint measure">
          {approx} / {budget} tokens ({preview?.memory_count ?? 0} memories)
        </span>
      </div>

      {error ? (
        <div className="callout danger">
          <span>{error}</span>
          <button
            type="button"
            className="btn btn-danger"
            onClick={() => void loadPreview(selectedProjectId)}
          >
            Retry
          </button>
        </div>
      ) : null}

      {savedPct !== null ? (
        <div className="callout ok">
          <span>
            Progressive disclosure: injecting {approx.toLocaleString()} tok instead of{' '}
            {archive.toLocaleString()} tok full archive
          </span>
          <strong className="measure">{savedPct}% saved</strong>
        </div>
      ) : null}

      <div className="stack-sm" style={{ gap: '6px' }}>
        <div className="budget-bar" role="img" aria-label="Token budget breakdown">
          <div
            className="budget-seg--pinned"
            style={{ width: `${Math.min(100, (pinnedTok / budget) * 100)}%` }}
            title={`Pinned: ${pinnedTok} tok`}
          />
          <div
            className="budget-seg--project"
            style={{ width: `${Math.min(100, (projectTok / budget) * 100)}%` }}
            title={`Project recent: ${projectTok} tok`}
          />
          <div
            className="budget-seg--global"
            style={{ width: `${Math.min(100, (globalTok / budget) * 100)}%` }}
            title={`Global: ${globalTok} tok`}
          />
        </div>
        <div className="tag-row">
          <span className="graph-legend-item field-hint measure">
            <span className="graph-swatch" style={{ background: 'var(--color-accent)' }} />
            Pinned ({pinnedTok} tok)
          </span>
          <span className="graph-legend-item field-hint measure">
            <span className="graph-swatch" style={{ background: 'var(--color-accent-2)' }} />
            Project ({projectTok} tok)
          </span>
          <span className="graph-legend-item field-hint measure">
            <span className="graph-swatch" style={{ background: 'var(--color-success)' }} />
            Global ({globalTok} tok)
          </span>
          <span className="field-hint measure" style={{ marginLeft: 'auto' }}>
            {Math.max(0, budget - approx)} tok headroom
          </span>
        </div>
      </div>

      {loading ? (
        <div className="skeleton-bar skeleton-bar--wide" />
      ) : preview && preview.sections.length > 0 ? (
        <div className="panel-sunk">
          {preview.sections.map((sec) => (
            <div key={sec.kind}>
              <div
                className="briefing-item-row"
                style={{ background: 'var(--color-paper-2)', color: 'var(--color-muted)' }}
              >
                <strong>{sec.label}</strong>
                <span className="measure">{sec.tokens} tok</span>
              </div>
              {sec.items.map((item) => (
                <div key={item.id} className="briefing-item-row">
                  <div className="tag-row" style={{ minWidth: 0 }}>
                    <span className={`tag ${item.source_client}`}>{item.source_client}</span>
                    <span style={{ color: item.included ? 'var(--color-ink)' : 'var(--color-muted)' }}>
                      {item.title}
                    </span>
                    {!item.included ? <span className="tag warn">budget trimmed</span> : null}
                  </div>
                  <div className="tag-row" style={{ flexShrink: 0 }}>
                    <span className="field-hint measure">{item.approx_tokens}t</span>
                    {item.status === 'pinned' ? (
                      <button
                        type="button"
                        className="btn-subtle"
                        onClick={() => void handleUnpin(item.id)}
                      >
                        Unpin
                      </button>
                    ) : null}
                  </div>
                </div>
              ))}
            </div>
          ))}
        </div>
      ) : null}

      <div className="panel-sunk ticks" style={{ position: 'relative' }}>
        <pre className="code-block">
          {preview?.briefing ||
            '(Empty: save or distill memories to populate the SessionStart briefing.)'}
        </pre>
      </div>
    </ModalDialog>
  );
}
