import type { KeyboardEvent, ReactNode } from "react";
import type { MonthTemperature, Wind } from "../../../api/client";
import { DIRECTIONS, MONTHS, PARTS, type PartId, PRINCIPAL, bearingOf, directionName } from "./climate";

const INK = "#1f1e1b";
const MUTED = "#6b6860";
const RULE = "#d9d6ce";
const ACCENT = "#2f5d50";
const WIND = "#3d6fb0";
const WARM = "#a33a2c";

/** An SVG shape that behaves like a toggle button for mouse and keyboard. */
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

// --- Wind rose: the wind table, drawn exactly ---

interface RoseProps {
  winds: Wind[];
  selected: string | null;
  onSelect: (direction: string) => void;
}

export function WindRose({ winds, selected, onSelect }: RoseProps) {
  const size = 360, centre = size / 2, radius = 130;
  const top = Math.max(...winds.map((wind) => wind.share), 1);
  const scale = Math.ceil(top / 10) * 10;
  const rings = [scale / 2, scale];
  // Petals are narrower when the data uses sixteen points rather than eight.
  const half = (winds.some((wind) => DIRECTIONS.indexOf(wind.direction) % 2 === 1) ? 22.5 : 45) * 0.38;
  const at = (bearing: number, r: number) => {
    const angle = (bearing * Math.PI) / 180;
    return `${(centre + r * Math.sin(angle)).toFixed(1)},${(centre - r * Math.cos(angle)).toFixed(1)}`;
  };
  const prevailing = winds.reduce((best, wind) => (wind.share > best.share ? wind : best), winds[0]);

  return (
    <svg viewBox={`0 0 ${size} ${size}`} role="group" aria-label="Wind rose" className="w-full max-w-md" fontSize="11" fontFamily="inherit">
      {rings.map((value) => (
        <g key={value}>
          <circle cx={centre} cy={centre} r={(value / scale) * radius} fill="none" stroke={RULE} />
          <text x={centre + 4} y={centre - (value / scale) * radius - 3} fill={MUTED}>
            {value}%
          </text>
        </g>
      ))}
      {PRINCIPAL.map((direction) => {
        const [x, y] = at(bearingOf(direction), radius + 18).split(",").map(Number);
        return (
          <text key={direction} x={x} y={y + 4} textAnchor="middle" fill={MUTED} fontWeight={direction.length === 1 ? 600 : 400}>
            {direction}
          </text>
        );
      })}
      {winds.map((wind) => {
        const bearing = bearingOf(wind.direction);
        const r = Math.max((wind.share / scale) * radius, 3);
        const chosen = selected === wind.direction;
        return (
          <Pick
            key={wind.direction}
            label={`Wind from the ${directionName(wind.direction)}, ${wind.share}% of the time`}
            selected={chosen}
            onPick={() => onSelect(wind.direction)}
          >
            <path
              d={`M${centre},${centre} L${at(bearing - half, r)} A${r},${r} 0 0 1 ${at(bearing + half, r)} Z`}
              fill={wind === prevailing ? WIND : "#9db6d6"}
              stroke={chosen ? INK : "#fff"}
              strokeWidth={chosen ? 2.5 : 1}
            />
          </Pick>
        );
      })}
      <text x={8} y={size - 8} fill={WIND}>
        Prevailing: {prevailing.direction} · {prevailing.share}%
      </text>
    </svg>
  );
}

// --- Temperature range against a comfort band ---

interface ComfortProps {
  months: MonthTemperature[];
  low: number;
  high: number;
  selected: number | null;
  onSelect: (month: number) => void;
}

export function ComfortChart({ months, low, high, selected, onSelect }: ComfortProps) {
  const width = 560, height = 280, left = 40, right = 548, top = 16, bottom = 236;
  const floor = Math.floor((Math.min(low, ...months.map((item) => item.min)) - 2) / 5) * 5;
  const ceiling = Math.ceil((Math.max(high, ...months.map((item) => item.max)) + 2) / 5) * 5;
  const y = (value: number) => bottom - ((value - floor) / (ceiling - floor)) * (bottom - top);
  const slot = (right - left) / 12;
  const ticks = Array.from({ length: (ceiling - floor) / 5 + 1 }, (_, index) => floor + index * 5);
  const byMonth = new Map(months.map((item) => [item.month, item]));

  return (
    <svg viewBox={`0 0 ${width} ${height}`} role="group" aria-label="Monthly temperature range against the comfort band" className="w-full max-w-2xl" fontSize="11" fontFamily="inherit">
      {ticks.map((value) => (
        <g key={value}>
          <line x1={left} x2={right} y1={y(value)} y2={y(value)} stroke={RULE} strokeWidth={value === floor ? 1.5 : 0.6} />
          <text x={left - 6} y={y(value) + 4} textAnchor="end" fill={MUTED}>
            {value}°
          </text>
        </g>
      ))}
      {high > low && <rect x={left} y={y(high)} width={right - left} height={y(low) - y(high)} fill="#e4ece8" />}
      {MONTHS.map((name, index) => {
        const month = byMonth.get(index + 1);
        const x = left + index * slot + slot * 0.22;
        const chosen = selected === index + 1;
        return (
          <g key={name}>
            <text x={left + index * slot + slot / 2} y={bottom + 16} textAnchor="middle" fill={month ? INK : RULE}>
              {name}
            </text>
            {month && (
              <Pick label={`${name}: ${month.min} to ${month.max} degrees Celsius`} selected={chosen} onPick={() => onSelect(month.month)}>
                <rect x={left + index * slot} y={top} width={slot} height={bottom - top} fill="transparent" />
                <rect
                  x={x}
                  y={y(month.max)}
                  width={slot * 0.56}
                  height={Math.max(y(month.min) - y(month.max), 2)}
                  rx={2}
                  fill={month.min > high ? WARM : month.max < low ? WIND : "#c98a4b"}
                  stroke={chosen ? INK : "none"}
                  strokeWidth={2.5}
                />
              </Pick>
            )}
          </g>
        );
      })}
      <text x={left} y={height - 6} fill={MUTED}>
        Bars run from the mean minimum to the mean maximum. Shaded: comfort band {low}–{high} °C, an assumption.
      </text>
    </svg>
  );
}

// --- A room in section, with the prevailing wind moving through it ---

interface SectionProps {
  wind: Wind;
  selected: PartId | null;
  onSelect: (part: PartId) => void;
}

export function RoomSection({ wind, selected, onSelect }: SectionProps) {
  const stroke = (part: PartId) => (selected === part ? ACCENT : INK);
  const weight = (part: PartId) => (selected === part ? 5 : 2.5);
  const part = (id: PartId, children: ReactNode) => (
    <Pick label={PARTS[id].name} selected={selected === id} onPick={() => onSelect(id)}>
      {children}
    </Pick>
  );

  return (
    <svg viewBox="0 0 560 320" role="group" aria-label="Section through a room showing air movement" className="w-full max-w-2xl" fontSize="12" fontFamily="inherit">
      <defs>
        <marker id="air" markerWidth="9" markerHeight="9" refX="7" refY="4.5" orient="auto">
          <path d="M0,0 L9,4.5 L0,9z" fill={WIND} />
        </marker>
        <marker id="warm" markerWidth="9" markerHeight="9" refX="7" refY="4.5" orient="auto">
          <path d="M0,0 L9,4.5 L0,9z" fill={WARM} />
        </marker>
      </defs>
      <line x1="20" y1="260" x2="540" y2="260" stroke={INK} strokeWidth="2" />

      {part("floor", <rect x="170" y="250" width="240" height="10" fill={selected === "floor" ? ACCENT : "#d8d3c6"} />)}

      {/* Windward wall with a low, wide opening */}
      {part(
        "inlet",
        <>
          <rect x="150" y="130" width="40" height="130" fill="transparent" />
          <path d="M170,260 V215 M170,165 V140" stroke={stroke("inlet")} strokeWidth={weight("inlet")} fill="none" />
          <path d="M162,215 H178 M162,165 H178" stroke={stroke("inlet")} strokeWidth="2" />
        </>,
      )}
      {/* Leeward wall with a higher opening */}
      {part(
        "outlet",
        <>
          <rect x="390" y="130" width="40" height="130" fill="transparent" />
          <path d="M410,260 V190 M410,150 V140" stroke={stroke("outlet")} strokeWidth={weight("outlet")} fill="none" />
          <path d="M402,190 H418 M402,150 H418" stroke={stroke("outlet")} strokeWidth="2" />
        </>,
      )}
      {part(
        "roof",
        <>
          <path d="M130,150 L282,70 M298,70 L450,150" stroke={stroke("roof")} strokeWidth={weight("roof") + 1} fill="none" strokeLinecap="round" />
          <path d="M130,150 L282,70 L298,70 L450,150" stroke="transparent" strokeWidth="18" fill="none" />
        </>,
      )}
      {part(
        "vent",
        <>
          <rect x="270" y="46" width="40" height="30" fill="transparent" />
          <path d="M276,62 H304" stroke={stroke("vent")} strokeWidth={weight("vent")} strokeLinecap="round" />
        </>,
      )}

      <g fill="none" stroke={WIND} strokeWidth="2.5" markerEnd="url(#air)">
        <path d="M30,190 H150" />
        <path d="M185,190 C250,190 320,172 396,170" />
        <path d="M424,170 H530" />
      </g>
      <path d="M290,200 C290,150 290,110 290,82" fill="none" stroke={WARM} strokeWidth="2.5" markerEnd="url(#warm)" strokeDasharray="6 5" />

      <text x="28" y="178" fill={WIND}>
        {wind.direction} wind · {wind.share}% of the time
      </text>
      <text x="474" y="158" fill={WIND}>
        out
      </text>
      <text x="300" y="38" fill={WARM}>
        warm air rises out
      </text>
      <text x="170" y="290" fill={MUTED}>
        Inlet faces the {directionName(wind.direction)}. Not to scale.
      </text>
    </svg>
  );
}
