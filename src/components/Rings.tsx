import { motion, useReducedMotion } from "motion/react";

/** Usage level thresholds (percent). Below `warn` = comfortable, from `alert` = nearly exhausted. */
export const LEVELS = { warn: 50, alert: 80 };

/** Traffic-light colour for a usage percentage; neutral grey when unknown. */
export function levelColor(pct: number | null | undefined): string {
  if (pct === null || pct === undefined) return "var(--text-3)";
  if (pct >= LEVELS.alert) return "#FF453A";
  if (pct >= LEVELS.warn) return "#FF9F0A";
  return "#30D158";
}

interface RingsProps {
  /** Outer ring: current 5-hour window, 0–100 (null = unknown). */
  fiveHour: number | null;
  /** Inner ring: rolling week, 0–100. */
  weekly: number | null;
  size?: number;
  stroke?: number;
  label?: string;
}

function Ring({ r, stroke, pct }: { r: number; stroke: number; pct: number | null }) {
  const reduce = useReducedMotion();
  const c = 2 * Math.PI * r;
  const value = Math.max(0, Math.min(100, pct ?? 0)) / 100;
  const color = levelColor(pct);
  return (
    <>
      <circle r={r} fill="none" strokeWidth={stroke} style={{ stroke: "var(--fill)" }} />
      <motion.circle
        r={r}
        fill="none"
        strokeWidth={stroke}
        strokeLinecap="round"
        strokeDasharray={c}
        initial={{ strokeDashoffset: c }}
        animate={{ strokeDashoffset: c * (1 - value) }}
        transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 60, damping: 18 }}
        transform="rotate(-90)"
        style={{ stroke: color, opacity: pct === null ? 0 : 1, transition: "stroke 300ms ease" }}
      />
    </>
  );
}

export function Rings({ fiveHour, weekly, size = 160, stroke = 16, label }: RingsProps) {
  const outer = size / 2 - stroke / 2;
  const inner = outer - stroke - 3;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`${-size / 2} ${-size / 2} ${size} ${size}`}
      role="img"
      aria-label={label ?? `Session 5 h : ${fiveHour ?? "inconnu"} %, 7 jours : ${weekly ?? "inconnu"} %`}
    >
      <Ring r={outer} stroke={stroke} pct={fiveHour} />
      <Ring r={inner} stroke={stroke} pct={weekly} />
    </svg>
  );
}
