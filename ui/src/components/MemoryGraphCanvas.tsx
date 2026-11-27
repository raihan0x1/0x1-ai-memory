import { useEffect, useMemo, useRef, useState } from 'react';
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from 'd3-force';
import { IconReset, IconZoomIn, IconZoomOut } from './Icons';
import type { GraphEdgeKind, GraphNode, MemoryGraphResponse } from '../types';

interface MemoryGraphCanvasProps {
  graph: MemoryGraphResponse | null;
  loading: boolean;
  error: string | null;
  selectedId: string | null;
  onSelectNode: (id: string) => void;
  onRetry: () => void;
}

interface SimNode extends GraphNode, SimulationNodeDatum {
  x: number;
  y: number;
  vx?: number;
  vy?: number;
}

interface SimLink extends SimulationLinkDatum<SimNode> {
  kind: GraphEdgeKind;
  weight: number;
  label: string;
}

type ColorBy = 'type' | 'client';

const TYPE_COLORS: Record<string, string> = {
  decision: '#ff8b3e',
  architecture: '#6fa8ff',
  bugfix: '#f27474',
  preference: '#d79cff',
  pattern: '#5fd39a',
  session_summary: '#e8c468',
};

const CLIENT_COLORS: Record<string, string> = {
  'claude-code': '#ff8b3e',
  codex: '#5fd39a',
  antigravity: '#6fa8ff',
  mcp: '#e8c468',
  ui: '#b8b2a8',
};

const EDGE_STYLES: Record<
  GraphEdgeKind,
  { label: string; color: string; dashed?: number[]; arrow?: boolean }
> = {
  supersedes: { label: 'Supersedes', color: '#ff8b3e', arrow: true },
  same_file: { label: 'Shared files', color: '#6fa8ff' },
  same_session: { label: 'Same session', color: '#e8c468' },
  semantic_similar: { label: 'Semantic >=72%', color: '#5fd39a', dashed: [5, 4] },
  shared_tag: { label: 'Shared tags', color: '#8c877f', dashed: [2, 3] },
};

const ALL_EDGE_KINDS: GraphEdgeKind[] = [
  'supersedes',
  'same_file',
  'same_session',
  'semantic_similar',
  'shared_tag',
];

function nodeColor(node: GraphNode, colorBy: ColorBy): string {
  if (colorBy === 'client') {
    return CLIENT_COLORS[node.source_client] ?? '#98938b';
  }
  return TYPE_COLORS[node.memory_type] ?? '#98938b';
}

function nodeRadius(node: GraphNode): number {
  return 6 + Math.min(node.degree, 8) * 1.1;
}

export function MemoryGraphCanvas({
  graph,
  loading,
  error,
  selectedId,
  onSelectNode,
  onRetry,
}: MemoryGraphCanvasProps) {
  const stageRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const [colorBy, setColorBy] = useState<ColorBy>('type');
  const [enabledEdges, setEnabledEdges] = useState<Record<GraphEdgeKind, boolean>>({
    supersedes: true,
    same_file: true,
    same_session: true,
    semantic_similar: true,
    shared_tag: false,
  });
  const [showOrphans, setShowOrphans] = useState(true);
  const [hoveredNode, setHoveredNode] = useState<GraphNode | null>(null);
  const [isDragging, setIsDragging] = useState(false);

  const simRef = useRef<Simulation<SimNode, SimLink> | null>(null);
  const nodesRef = useRef<SimNode[]>([]);
  const linksRef = useRef<SimLink[]>([]);
  const transformRef = useRef<{ x: number; y: number; k: number }>({ x: 0, y: 0, k: 1 });
  const dragStateRef = useRef<{
    mode: 'none' | 'pan' | 'node';
    node: SimNode | null;
    startX: number;
    startY: number;
    lastX: number;
    lastY: number;
    moved: boolean;
  }>({
    mode: 'none',
    node: null,
    startX: 0,
    startY: 0,
    lastX: 0,
    lastY: 0,
    moved: false,
  });
  const hoveredIdRef = useRef<string | null>(null);
  const selectedIdRef = useRef<string | null>(selectedId);
  const colorByRef = useRef<ColorBy>(colorBy);
  const drawRef = useRef<() => void>(() => {});

  const edgeCounts = useMemo(() => {
    const counts: Record<GraphEdgeKind, number> = {
      supersedes: 0,
      same_file: 0,
      same_session: 0,
      semantic_similar: 0,
      shared_tag: 0,
    };
    for (const e of graph?.edges ?? []) {
      counts[e.kind] = (counts[e.kind] ?? 0) + 1;
    }
    return counts;
  }, [graph]);

  const filteredData = useMemo(() => {
    if (!graph) return { nodes: [] as SimNode[], links: [] as SimLink[] };
    const activeLinks = graph.edges.filter((e) => enabledEdges[e.kind]);
    const degreeMap = new Map<string, number>();
    for (const e of activeLinks) {
      degreeMap.set(e.source, (degreeMap.get(e.source) ?? 0) + 1);
      degreeMap.set(e.target, (degreeMap.get(e.target) ?? 0) + 1);
    }
    const visibleNodes = graph.nodes.filter(
      (n) => showOrphans || (degreeMap.get(n.id) ?? 0) > 0,
    );
    const visibleSet = new Set(visibleNodes.map((n) => n.id));
    const simNodes: SimNode[] = visibleNodes.map((n, i) => {
      const angle = (i / Math.max(1, visibleNodes.length)) * Math.PI * 2;
      const rad = 60 + (i % 5) * 28;
      return {
        ...n,
        degree: degreeMap.get(n.id) ?? 0,
        x: Math.cos(angle) * rad,
        y: Math.sin(angle) * rad,
      };
    });
    const simLinks: SimLink[] = activeLinks
      .filter((e) => visibleSet.has(e.source) && visibleSet.has(e.target))
      .map((e) => ({ ...e }));

    return { nodes: simNodes, links: simLinks };
  }, [graph, enabledEdges, showOrphans]);

  useEffect(() => {
    selectedIdRef.current = selectedId;
    colorByRef.current = colorBy;
    drawRef.current();
  }, [selectedId, colorBy]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const stage = stageRef.current;
    if (!canvas || !stage) return;

    const rect = stage.getBoundingClientRect();
    const width = Math.max(320, rect.width);
    const height = Math.max(360, rect.height);
    transformRef.current = { x: width / 2, y: height / 2, k: 1 };

    nodesRef.current = filteredData.nodes;
    linksRef.current = filteredData.links;

    const draw = () => {
      const c = canvasRef.current;
      if (!c) return;
      const ctx = c.getContext('2d');
      if (!ctx) return;

      const dpr = window.devicePixelRatio || 1;
      const cssW = c.clientWidth || width;
      const cssH = c.clientHeight || height;
      if (c.width !== Math.round(cssW * dpr) || c.height !== Math.round(cssH * dpr)) {
        c.width = Math.round(cssW * dpr);
        c.height = Math.round(cssH * dpr);
      }

      ctx.save();
      ctx.scale(dpr, dpr);
      ctx.clearRect(0, 0, cssW, cssH);

      const t = transformRef.current;

      // Subtle architectural dot grid
      const gridStep = 28 * t.k;
      if (gridStep >= 10) {
        ctx.fillStyle = 'rgba(152, 147, 139, 0.14)';
        const offX = ((t.x % gridStep) + gridStep) % gridStep;
        const offY = ((t.y % gridStep) + gridStep) % gridStep;
        for (let gx = offX; gx < cssW; gx += gridStep) {
          for (let gy = offY; gy < cssH; gy += gridStep) {
            ctx.fillRect(gx, gy, 1, 1);
          }
        }
      }

      ctx.translate(t.x, t.y);
      ctx.scale(t.k, t.k);

      const activeSelectedId = selectedIdRef.current;
      const activeColorBy = colorByRef.current;
      const focalId = hoveredIdRef.current ?? activeSelectedId;
      const neighborSet = new Set<string>();
      if (focalId) {
        neighborSet.add(focalId);
        for (const l of linksRef.current) {
          const s = typeof l.source === 'object' ? l.source.id : String(l.source);
          const tg = typeof l.target === 'object' ? l.target.id : String(l.target);
          if (s === focalId) neighborSet.add(tg);
          if (tg === focalId) neighborSet.add(s);
        }
      }

      // Draw edges
      for (const l of linksRef.current) {
        const src = l.source as SimNode;
        const dst = l.target as SimNode;
        if (src.x == null || dst.x == null) continue;
        const sId = src.id;
        const tId = dst.id;
        const touchingFocal = focalId ? sId === focalId || tId === focalId : true;
        const style = EDGE_STYLES[l.kind];

        ctx.save();
        ctx.globalAlpha = focalId ? (touchingFocal ? 0.9 : 0.14) : 0.62;
        ctx.strokeStyle = style.color;
        ctx.lineWidth = (l.kind === 'supersedes' ? 1.8 : 1.15) / Math.max(0.7, t.k * 0.85);
        if (style.dashed) {
          ctx.setLineDash(style.dashed);
        }
        ctx.beginPath();
        ctx.moveTo(src.x, src.y);
        ctx.lineTo(dst.x, dst.y);
        ctx.stroke();

        if (style.arrow) {
          const dx = dst.x - src.x;
          const dy = dst.y - src.y;
          const len = Math.hypot(dx, dy);
          if (len > 12) {
            const rad = nodeRadius(dst) + 3;
            const ax = dst.x - (dx / len) * rad;
            const ay = dst.y - (dy / len) * rad;
            const ang = Math.atan2(dy, dx);
            ctx.setLineDash([]);
            ctx.fillStyle = style.color;
            ctx.beginPath();
            ctx.moveTo(ax, ay);
            ctx.lineTo(ax - 7 * Math.cos(ang - 0.45), ay - 7 * Math.sin(ang - 0.45));
            ctx.lineTo(ax - 7 * Math.cos(ang + 0.45), ay - 7 * Math.sin(ang + 0.45));
            ctx.closePath();
            ctx.fill();
          }
        }
        ctx.restore();
      }

      // Draw nodes
      const showAllLabels = nodesRef.current.length <= 28 || t.k >= 1.15;
      for (const n of nodesRef.current) {
        const r = nodeRadius(n);
        const col = nodeColor(n, activeColorBy);
        const isSelected = n.id === activeSelectedId;
        const isHovered = n.id === hoveredIdRef.current;
        const inFocus = focalId ? neighborSet.has(n.id) : true;
        const isMutedStatus = n.status === 'superseded' || n.status === 'archived';

        ctx.save();
        ctx.globalAlpha = inFocus ? (isMutedStatus ? 0.65 : 1) : 0.2;

        // Pinned outer square ring
        if (n.status === 'pinned') {
          ctx.strokeStyle = '#ff8b3e';
          ctx.lineWidth = 1.4;
          ctx.strokeRect(n.x - r - 3.5, n.y - r - 3.5, (r + 3.5) * 2, (r + 3.5) * 2);
        }

        ctx.beginPath();
        ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
        if (isMutedStatus) {
          ctx.fillStyle = '#141311';
          ctx.fill();
          ctx.strokeStyle = col;
          ctx.lineWidth = 1.6;
          ctx.stroke();
        } else {
          ctx.fillStyle = col;
          ctx.fill();
        }

        if (isSelected || isHovered) {
          ctx.strokeStyle = '#f5f2eb';
          ctx.lineWidth = 2;
          ctx.beginPath();
          ctx.arc(n.x, n.y, r + 2.5, 0, Math.PI * 2);
          ctx.stroke();
        }

        if (isSelected || isHovered || (inFocus && (showAllLabels || n.degree >= 3))) {
          const label = n.title.length > 28 ? `${n.title.slice(0, 27)}...` : n.title;
          ctx.font = '11px "Switzer", ui-sans-serif, system-ui, sans-serif';
          ctx.textAlign = 'center';
          ctx.fillStyle = isSelected || isHovered ? '#f5f2eb' : '#ccc6bc';
          ctx.fillText(label, n.x, n.y + r + 13);
        }
        ctx.restore();
      }

      ctx.restore();
    };

    drawRef.current = draw;

    simRef.current?.stop();
    const sim = forceSimulation<SimNode>(filteredData.nodes)
      .force(
        'link',
        forceLink<SimNode, SimLink>(filteredData.links)
          .id((d) => d.id)
          .distance((l) => (l.kind === 'supersedes' ? 78 : l.kind === 'same_file' ? 92 : 118))
          .strength((l) => Math.min(0.85, l.weight * 0.65)),
      )
      .force('charge', forceManyBody().strength(-210).distanceMax(420))
      .force(
        'collide',
        forceCollide<SimNode>((d) => nodeRadius(d) + 16),
      )
      .force('x', forceX(0).strength(0.06))
      .force('y', forceY(0).strength(0.06))
      .force('center', forceCenter(0, 0));

    sim.stop();
    sim.tick(120);
    sim.on('tick', draw);
    simRef.current = sim;
    draw();

    const ro = new ResizeObserver(() => draw());
    ro.observe(stage);

    return () => {
      sim.stop();
      ro.disconnect();
    };
  }, [filteredData]);

  function screenToWorld(clientX: number, clientY: number): { wx: number; wy: number } {
    const canvas = canvasRef.current;
    if (!canvas) return { wx: 0, wy: 0 };
    const rect = canvas.getBoundingClientRect();
    const sx = clientX - rect.left;
    const sy = clientY - rect.top;
    const t = transformRef.current;
    return { wx: (sx - t.x) / t.k, wy: (sy - t.y) / t.k };
  }

  function findNodeAt(wx: number, wy: number): SimNode | null {
    for (let i = nodesRef.current.length - 1; i >= 0; i -= 1) {
      const n = nodesRef.current[i];
      const r = nodeRadius(n) + 5;
      if (Math.hypot(wx - n.x, wy - n.y) <= r) {
        return n;
      }
    }
    return null;
  }

  function zoomBy(factor: number, clientX?: number, clientY?: number) {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const sx = clientX !== undefined ? clientX - rect.left : rect.width / 2;
    const sy = clientY !== undefined ? clientY - rect.top : rect.height / 2;
    const t = transformRef.current;
    const nextK = Math.max(0.25, Math.min(3.5, t.k * factor));
    const ratio = nextK / t.k;
    transformRef.current = {
      x: sx - (sx - t.x) * ratio,
      y: sy - (sy - t.y) * ratio,
      k: nextK,
    };
    drawRef.current();
  }

  function resetView() {
    const stage = stageRef.current;
    if (!stage) return;
    const rect = stage.getBoundingClientRect();
    transformRef.current = { x: rect.width / 2, y: rect.height / 2, k: 1 };
    drawRef.current();
  }

  const legendEntries =
    colorBy === 'client'
      ? [
          ['claude-code', CLIENT_COLORS['claude-code']],
          ['codex', CLIENT_COLORS.codex],
          ['antigravity', CLIENT_COLORS.antigravity],
        ]
      : [
          ['decision', TYPE_COLORS.decision],
          ['architecture', TYPE_COLORS.architecture],
          ['bugfix', TYPE_COLORS.bugfix],
          ['pattern', TYPE_COLORS.pattern],
          ['preference', TYPE_COLORS.preference],
          ['session_summary', TYPE_COLORS.session_summary],
        ];

  const total = graph?.total ?? graph?.nodes.length ?? 0;
  const nodeCount = filteredData.nodes.length;
  const edgeCount = filteredData.links.length;

  return (
    <section className="panel ticks" aria-label="Memory knowledge graph">
      <div className="panel-header">
        <div className="head-actions">
          <fieldset className="seg-group" aria-label="Color nodes by">
            <button
              type="button"
              className="seg-btn"
              aria-pressed={colorBy === 'type'}
              onClick={() => setColorBy('type')}
            >
              Color: Type
            </button>
            <button
              type="button"
              className="seg-btn"
              aria-pressed={colorBy === 'client'}
              onClick={() => setColorBy('client')}
            >
              Color: Client
            </button>
          </fieldset>

          <label className="rim-check-label">
            <input
              type="checkbox"
              className="rim-check"
              checked={showOrphans}
              onChange={(e) => setShowOrphans(e.target.checked)}
            />
            Show unconnected
          </label>
        </div>

        <div className="head-actions">
          <span className="field-hint measure" role="status">
            {graph && !loading && !error
              ? `Showing ${nodeCount.toLocaleString()} of ${total.toLocaleString()} ${total === 1 ? 'result' : 'results'} · ${edgeCount.toLocaleString()} ${edgeCount === 1 ? 'edge' : 'edges'}`
              : null}
          </span>
          <button
            type="button"
            className="btn-subtle"
            onClick={() => zoomBy(1.22)}
            aria-label="Zoom in graph"
          >
            <IconZoomIn size={13} />
          </button>
          <button
            type="button"
            className="btn-subtle"
            onClick={() => zoomBy(0.82)}
            aria-label="Zoom out graph"
          >
            <IconZoomOut size={13} />
          </button>
          <button type="button" className="btn-subtle" onClick={resetView}>
            <IconReset size={12} /> Reset
          </button>
        </div>
      </div>

      <div
        className="panel-header"
        style={{ background: 'var(--color-paper-3)', borderBottomColor: 'var(--color-rule-2)' }}
      >
        <div className="tag-row">
          <span className="field-hint">Edges:</span>
          {ALL_EDGE_KINDS.map((kind) => {
            const st = EDGE_STYLES[kind];
            const active = enabledEdges[kind];
            return (
              <button
                key={kind}
                type="button"
                className="btn-subtle"
                aria-pressed={active}
                onClick={() =>
                  setEnabledEdges((prev) => ({
                    ...prev,
                    [kind]: !prev[kind],
                  }))
                }
              >
                <span className="graph-swatch" style={{ background: st.color }} />
                {st.label} ({edgeCounts[kind]})
              </button>
            );
          })}
        </div>
      </div>

      {error ? (
        <div className="panel-body">
          <div className="callout danger">
            <span role="alert">Graph error: {error}</span>
            <button type="button" className="btn btn-danger" onClick={onRetry}>
              Retry
            </button>
          </div>
        </div>
      ) : loading ? (
        <div className="panel-body" aria-busy="true">
          <div className="skeleton-row">
            <div className="skeleton-bar skeleton-bar--short" />
            <div className="skeleton-bar skeleton-bar--wide" />
          </div>
        </div>
      ) : filteredData.nodes.length === 0 ? (
        <div className="empty-state">
          <p>
            {graph?.nodes.length
              ? 'No connected memories with these edge settings. Turn on Show unconnected to see the loaded memories.'
              : 'No memories match the current query or filters.'}
          </p>
        </div>
      ) : (
        <div ref={stageRef} className="graph-stage">
          <canvas
            ref={canvasRef}
            className={`graph-canvas${isDragging ? ' graph-canvas--dragging' : ''}`}
            onWheel={(e) => {
              e.preventDefault();
              zoomBy(e.deltaY < 0 ? 1.12 : 0.89, e.clientX, e.clientY);
            }}
            onPointerDown={(e) => {
              const { wx, wy } = screenToWorld(e.clientX, e.clientY);
              const hit = findNodeAt(wx, wy);
              e.currentTarget.setPointerCapture(e.pointerId);
              if (hit) {
                hit.fx = hit.x;
                hit.fy = hit.y;
                simRef.current?.alphaTarget(0.25).restart();
                dragStateRef.current = {
                  mode: 'node',
                  node: hit,
                  startX: e.clientX,
                  startY: e.clientY,
                  lastX: e.clientX,
                  lastY: e.clientY,
                  moved: false,
                };
              } else {
                dragStateRef.current = {
                  mode: 'pan',
                  node: null,
                  startX: e.clientX,
                  startY: e.clientY,
                  lastX: e.clientX,
                  lastY: e.clientY,
                  moved: false,
                };
              }
              setIsDragging(true);
            }}
            onPointerMove={(e) => {
              const ds = dragStateRef.current;
              if (ds.mode === 'node' && ds.node) {
                if (Math.hypot(e.clientX - ds.startX, e.clientY - ds.startY) > 4) {
                  ds.moved = true;
                }
                const { wx, wy } = screenToWorld(e.clientX, e.clientY);
                ds.node.fx = wx;
                ds.node.fy = wy;
                drawRef.current();
                return;
              }
              if (ds.mode === 'pan') {
                const dx = e.clientX - ds.lastX;
                const dy = e.clientY - ds.lastY;
                ds.lastX = e.clientX;
                ds.lastY = e.clientY;
                if (Math.hypot(e.clientX - ds.startX, e.clientY - ds.startY) > 4) {
                  ds.moved = true;
                }
                transformRef.current = {
                  ...transformRef.current,
                  x: transformRef.current.x + dx,
                  y: transformRef.current.y + dy,
                };
                drawRef.current();
                return;
              }
              const { wx, wy } = screenToWorld(e.clientX, e.clientY);
              const hit = findNodeAt(wx, wy);
              const nextHover = hit?.id ?? null;
              if (nextHover !== hoveredIdRef.current) {
                hoveredIdRef.current = nextHover;
                setHoveredNode(hit);
                drawRef.current();
              }
            }}
            onPointerUp={(e) => {
              const ds = dragStateRef.current;
              if (ds.mode === 'node' && ds.node) {
                ds.node.fx = null;
                ds.node.fy = null;
                simRef.current?.alphaTarget(0);
                if (!ds.moved) {
                  onSelectNode(ds.node.id);
                }
              }
              dragStateRef.current = {
                mode: 'none',
                node: null,
                startX: 0,
                startY: 0,
                lastX: 0,
                lastY: 0,
                moved: false,
              };
              setIsDragging(false);
              if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                e.currentTarget.releasePointerCapture(e.pointerId);
              }
            }}
          />

          <div className="graph-legend" aria-hidden="true">
            {legendEntries.map(([name, color]) => (
              <span key={name} className="graph-legend-item">
                <span className="graph-swatch" style={{ background: color }} />
                {name}
              </span>
            ))}
          </div>

          {hoveredNode ? (
            <div className="graph-tooltip">
              <div className="tag-row">
                <span className={`tag ${hoveredNode.source_client}`}>
                  {hoveredNode.source_client}
                </span>
                <span className="field-hint">{hoveredNode.memory_type}</span>
                {hoveredNode.recall_count > 0 ? (
                  <span className="field-hint measure">{hoveredNode.recall_count}x recalled</span>
                ) : null}
              </div>
              <strong style={{ fontSize: 'var(--text-sm)' }}>{hoveredNode.title}</strong>
              <span className="field-hint">{hoveredNode.summary}</span>
            </div>
          ) : null}
        </div>
      )}
    </section>
  );
}
