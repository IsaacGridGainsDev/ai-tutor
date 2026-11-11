import { type KeyboardEvent, type ReactNode, useMemo } from "react";

const INK = "#1f1e1b";
const MUTED = "#6b6860";
const RULE = "#d9d6ce";
const ACCENT = "#2f5d50";
const SOFT = "#e4ece8";

function Pick({ label, selected, onPick, children }: { label: string; selected: boolean; onPick: () => void; children: ReactNode }) {
  function onKey(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onPick();
    }
  }
  return (
    <g role="button" tabIndex={0} aria-label={label} aria-pressed={selected} onClick={onPick} onKeyDown={onKey} className="cursor-pointer outline-none focus-visible:opacity-70">
      {children}
    </g>
  );
}

// --- A map of things and the links between them ---

export interface GraphNode {
  id: string;
  label: string;
  /** Relative size, 0 to 1. */
  weight?: number;
}

export interface GraphEdge {
  source: string;
  target: string;
  label?: string;
  weight?: number;
}

const WIDTH = 680, HEIGHT = 480, MARGIN = 56;

/** Places linked nodes near each other and pushes the rest apart. The same input always gives the same picture. */
export function layOutGraph(nodes: GraphNode[], edges: GraphEdge[]) {
  const count = nodes.length;
  const index = new Map(nodes.map((node, position) => [node.id, position]));
  const points = nodes.map((_, position) => {
    const angle = (2 * Math.PI * position) / count;
    return { x: WIDTH / 2 + (WIDTH / 3) * Math.cos(angle), y: HEIGHT / 2 + (HEIGHT / 3) * Math.sin(angle) };
  });
  const ideal = Math.sqrt(((WIDTH - 2 * MARGIN) * (HEIGHT - 2 * MARGIN)) / Math.max(count, 1)) * 0.75;
  const links = edges.flatMap((edge) => {
    const a = index.get(edge.source), b = index.get(edge.target);
    return a === undefined || b === undefined ? [] : [[a, b] as const];
  });
  for (let step = 0; step < 300; step++) {
    const heat = (WIDTH / 10) * (1 - step / 300);
    const move = points.map(() => ({ x: 0, y: 0 }));
    for (let a = 0; a < count; a++) {
      for (let b = a + 1; b < count; b++) {
        const dx = points[a].x - points[b].x, dy = points[a].y - points[b].y;
        const distance = Math.max(Math.hypot(dx, dy), 0.01);
        const push = (ideal * ideal) / distance;
        move[a].x += (dx / distance) * push;
        move[a].y += (dy / distance) * push;
        move[b].x -= (dx / distance) * push;
        move[b].y -= (dy / distance) * push;
      }
    }
    for (const [a, b] of links) {
      const dx = points[a].x - points[b].x, dy = points[a].y - points[b].y;
      const distance = Math.max(Math.hypot(dx, dy), 0.01);
      const pull = (distance * distance) / ideal;
      move[a].x -= (dx / distance) * pull;
      move[a].y -= (dy / distance) * pull;
      move[b].x += (dx / distance) * pull;
      move[b].y += (dy / distance) * pull;
    }
    points.forEach((point, position) => {
      // A gentle pull to the centre keeps unlinked nodes in view.
      move[position].x += (WIDTH / 2 - point.x) * 0.06;
      move[position].y += (HEIGHT / 2 - point.y) * 0.06;
      const size = Math.max(Math.hypot(move[position].x, move[position].y), 0.01);
      point.x = Math.min(WIDTH - MARGIN, Math.max(MARGIN, point.x + (move[position].x / size) * Math.min(size, heat)));
      point.y = Math.min(HEIGHT - MARGIN, Math.max(MARGIN * 0.6, point.y + (move[position].y / size) * Math.min(size, heat)));
    });
  }
  return points;
}

interface GraphProps {
  label: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** Draw arrowheads from source to target. */
  directed?: boolean;
  selected: string | null;
  onSelect: (id: string) => void;
}

export function Graph({ label, nodes, edges, directed, selected, onSelect }: GraphProps) {
  const points = useMemo(() => layOutGraph(nodes, edges), [nodes, edges]);
  const at = new Map(nodes.map((node, position) => [node.id, points[position]]));
  const near = new Set(edges.flatMap((edge) => (edge.source === selected ? [edge.target] : edge.target === selected ? [edge.source] : [])));
  const heaviest = Math.max(...edges.map((edge) => edge.weight ?? 1), 1);
  const short = (text: string) => (text.length > 24 ? `${text.slice(0, 23)}…` : text);
  const radius = (node: GraphNode) => 7 + 9 * (node.weight ?? 0.4);

  return (
    <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} role="group" aria-label={label} className="w-full" fontSize="11.5" fontFamily="inherit">
      <defs>
        <marker id="link-end" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
          <path d="M0,0 L8,4 L0,8z" fill={MUTED} />
        </marker>
      </defs>
      {edges.map((edge, position) => {
        const a = at.get(edge.source), b = at.get(edge.target);
        if (!a || !b) return null;
        const touches = edge.source === selected || edge.target === selected;
        // Stop short of the target so an arrowhead is not hidden under its circle.
        const length = Math.max(Math.hypot(b.x - a.x, b.y - a.y), 1), gap = directed ? 18 : 0;
        const end = { x: b.x - ((b.x - a.x) / length) * gap, y: b.y - ((b.y - a.y) / length) * gap };
        return (
          <g key={position} opacity={selected && !touches ? 0.25 : 1}>
            <line x1={a.x} y1={a.y} x2={end.x} y2={end.y} stroke={touches ? ACCENT : "#b9b5ab"} strokeWidth={1 + 2.5 * ((edge.weight ?? 1) / heaviest)} markerEnd={directed ? "url(#link-end)" : undefined} />
            {edge.label && (
              <text x={(a.x + b.x) / 2} y={(a.y + b.y) / 2 - 4} textAnchor="middle" fill={MUTED} fontSize="10.5">
                {short(edge.label)}
              </text>
            )}
          </g>
        );
      })}
      {nodes.map((node) => {
        const point = at.get(node.id)!;
        const chosen = node.id === selected;
        return (
          <Pick key={node.id} label={node.label} selected={chosen} onPick={() => onSelect(node.id)}>
            <g opacity={selected && !chosen && !near.has(node.id) ? 0.35 : 1}>
              <circle cx={point.x} cy={point.y} r={radius(node) + 9} fill="transparent" />
              <circle cx={point.x} cy={point.y} r={radius(node)} fill={chosen ? ACCENT : SOFT} stroke={chosen ? INK : ACCENT} strokeWidth={chosen ? 2.5 : 1.5} />
              <text x={point.x} y={point.y + radius(node) + 13} textAnchor="middle" fill={INK} fontWeight={chosen ? 600 : 400} stroke="#f7f6f3" strokeWidth="3" paintOrder="stroke">
                {short(node.label)}
              </text>
            </g>
          </Pick>
        );
      })}
    </svg>
  );
}

// --- Steps in order ---

interface StepsProps {
  label: string;
  steps: { id: string; label: string; link?: string }[];
  selected: string | null;
  onSelect: (id: string) => void;
}

export function StepFlow({ label, steps, selected, onSelect }: StepsProps) {
  return (
    <ol aria-label={label} className="max-w-xl">
      {steps.map((step, position) => (
        <li key={step.id}>
          {position > 0 && (
            <div aria-hidden className="ml-[1.35rem] flex h-7 items-center gap-2 border-l-2 border-accent/40 pl-4 text-xs text-muted">
              {step.link}
            </div>
          )}
          <button
            type="button"
            aria-pressed={selected === step.id}
            onClick={() => onSelect(step.id)}
            className={`flex w-full items-center gap-3 rounded-md border px-3 py-2.5 text-left text-sm ${
              selected === step.id ? "border-accent bg-accent-soft" : "border-rule bg-surface hover:border-accent"
            }`}
          >
            <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-accent text-xs text-white">{position + 1}</span>
            <span>{step.label}</span>
          </button>
        </li>
      ))}
    </ol>
  );
}

// --- Figures the text states ---

interface SeriesProps {
  label: string;
  items: { label: string; value: number }[];
  unit: string;
  selected: number | null;
  onSelect: (position: number) => void;
}

const amount = (value: number, unit: string) => `${value.toLocaleString("en-GB")}${unit}`;

export function BarChart({ label, items, unit, selected, onSelect }: SeriesProps) {
  const row = 34, left = 170, right = 600, top = 8;
  const most = Math.max(...items.map((item) => item.value), 1);
  return (
    <svg viewBox={`0 0 680 ${top * 2 + items.length * row}`} role="group" aria-label={label} className="w-full max-w-2xl" fontSize="12" fontFamily="inherit">
      <line x1={left} y1={top} x2={left} y2={top + items.length * row} stroke={RULE} />
      {items.map((item, position) => {
        const y = top + position * row;
        const chosen = selected === position;
        return (
          <Pick key={item.label} label={`${item.label}: ${amount(item.value, unit)}`} selected={chosen} onPick={() => onSelect(position)}>
            <rect x={0} y={y} width={680} height={row} fill="transparent" />
            <text x={left - 10} y={y + row / 2 + 4} textAnchor="end" fill={INK} fontWeight={chosen ? 600 : 400}>
              {item.label.length > 24 ? `${item.label.slice(0, 23)}…` : item.label}
            </text>
            <rect x={left} y={y + 7} width={Math.max(((right - left) * item.value) / most, 2)} height={row - 14} rx={2} fill={chosen ? ACCENT : "#8fb3a6"} stroke={chosen ? INK : "none"} strokeWidth={2} />
            <text x={left + Math.max(((right - left) * item.value) / most, 2) + 8} y={y + row / 2 + 4} fill={MUTED}>
              {amount(item.value, unit)}
            </text>
          </Pick>
        );
      })}
    </svg>
  );
}

export function LineChart({ label, items, unit, selected, onSelect }: SeriesProps) {
  const left = 64, right = 640, top = 24, bottom = 250;
  const values = items.map((item) => item.value);
  const low = Math.min(...values), high = Math.max(...values);
  const pad = (high - low || 1) * 0.15;
  const xs = items.map((item) => Number(item.label));
  const numeric = xs.every((value) => Number.isFinite(value)) && xs[xs.length - 1] > xs[0];
  const x = (position: number) =>
    left + (numeric ? (xs[position] - xs[0]) / (xs[xs.length - 1] - xs[0]) : position / Math.max(items.length - 1, 1)) * (right - left);
  const y = (value: number) => bottom - ((value - (low - pad)) / (high - low + 2 * pad)) * (bottom - top);
  return (
    <svg viewBox="0 0 680 300" role="group" aria-label={label} className="w-full max-w-2xl" fontSize="12" fontFamily="inherit">
      {[low, high].map((value) => (
        <g key={value}>
          <line x1={left} x2={right} y1={y(value)} y2={y(value)} stroke={RULE} strokeDasharray="3 4" />
          <text x={left - 8} y={y(value) + 4} textAnchor="end" fill={MUTED}>{amount(value, unit)}</text>
        </g>
      ))}
      <line x1={left} x2={right} y1={bottom} y2={bottom} stroke={RULE} strokeWidth="1.5" />
      <polyline points={items.map((item, position) => `${x(position)},${y(item.value)}`).join(" ")} fill="none" stroke={ACCENT} strokeWidth="2" />
      {items.map((item, position) => (
        <Pick key={item.label} label={`${item.label}: ${amount(item.value, unit)}`} selected={selected === position} onPick={() => onSelect(position)}>
          <rect x={x(position) - 22} y={top - 10} width={44} height={bottom - top + 34} fill="transparent" />
          <circle cx={x(position)} cy={y(item.value)} r={selected === position ? 6.5 : 4.5} fill={selected === position ? INK : ACCENT} />
          <text x={x(position)} y={bottom + 18} textAnchor="middle" fill={INK}>{item.label}</text>
        </Pick>
      ))}
      <text x={left} y={292} fill={MUTED}>The vertical scale does not start at zero.</text>
    </svg>
  );
}
