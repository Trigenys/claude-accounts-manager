import "@fontsource-variable/geist";
import "./style.css";

const SVG_NS = "http://www.w3.org/2000/svg";
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Same thresholds as the app (src/components/Rings.tsx levelColor). */
function levelColor(pct: number): string {
  if (pct >= 80) return "var(--ring-alert)";
  if (pct >= 50) return "var(--ring-warn)";
  return "var(--ring-ok)";
}

type Ring = { arc: SVGCircleElement; circumference: number };

function makeRing(svg: SVGSVGElement, radius: number, stroke: number): Ring {
  const track = document.createElementNS(SVG_NS, "circle");
  track.setAttribute("r", String(radius));
  track.setAttribute("class", "ring-track");
  track.setAttribute("stroke-width", String(stroke));
  const arc = document.createElementNS(SVG_NS, "circle");
  arc.setAttribute("r", String(radius));
  arc.setAttribute("class", "ring-arc");
  arc.setAttribute("stroke-width", String(stroke));
  const circumference = 2 * Math.PI * radius;
  arc.style.strokeDasharray = `${circumference}`;
  arc.style.strokeDashoffset = `${circumference}`;
  svg.append(track, arc);
  return { arc, circumference };
}

function setRing(ring: Ring, pct: number): void {
  const clamped = Math.max(0, Math.min(100, pct));
  ring.arc.style.strokeDashoffset = `${ring.circumference * (1 - clamped / 100)}`;
  ring.arc.style.stroke = levelColor(clamped);
}

/** Outer ring: 5 h session; inner ring: 7 days (outer radius − stroke − gap, as in the app). */
function buildPair(svg: SVGSVGElement): [Ring, Ring] {
  const stroke = 18;
  const outer = makeRing(svg, 100 - stroke / 2, stroke);
  const inner = makeRing(svg, 100 - stroke / 2 - stroke - 6, stroke);
  return [outer, inner];
}

// Mini rings in the bento: drawn when they scroll into view.
const miniPairs = [...document.querySelectorAll<SVGSVGElement>(".mini-rings svg")].map((svg) => ({
  svg,
  rings: buildPair(svg),
}));

// Interactive demo.
const demoSvg = document.querySelector<SVGSVGElement>("#demo-rings");
const range = document.querySelector<HTMLInputElement>("#demo-range");
const pctLabel = document.querySelector<HTMLElement>("#demo-pct");
const toastTitle = document.querySelector<HTMLElement>("#demo-toast-title");
const toastBody = document.querySelector<HTMLElement>("#demo-toast-body");
const toast = document.querySelector<HTMLElement>("#demo-toast");
const ringTitle = document.querySelector<SVGTitleElement>("#demo-ring-label");

const ELAPSED_MIN = 160; // 2 h 40 into the 5 h window
const RESET_MIN = 140; // reset in 2 h 20
const WEEK = 40;

/** Duration format used by the app: `m min`, `H h MM`. */
function fmt(min: number): string {
  const m = Math.max(1, Math.round(min));
  if (m < 60) return `${m} min`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
}

function crossedThreshold(pct: number): number | null {
  if (pct >= 90) return 90;
  if (pct >= 75) return 75;
  if (pct >= 50) return 50;
  return null;
}

if (demoSvg && range && pctLabel && toastTitle && toastBody && toast) {
  const [outer, inner] = buildPair(demoSvg);
  const render = (): void => {
    const pct = Number(range.value);
    setRing(outer, pct);
    setRing(inner, WEEK);
    pctLabel.textContent = `${pct} %`;
    if (ringTitle) ringTitle.textContent = `Compte Client : session 5 h à ${pct} %, 7 jours à ${WEEK} %`;

    const threshold = crossedThreshold(pct);
    const rate = pct / ELAPSED_MIN; // % per minute
    const toExhaust = rate > 0 ? (100 - pct) / rate : Infinity;
    const paceAlert = toExhaust < RESET_MIN && pct > 0;

    toast.dataset.level = threshold === null && !paceAlert ? "calm" : pct >= 80 ? "alert" : "warn";
    if (threshold === null && !paceAlert) {
      toastTitle.textContent = "Aucune alerte";
      toastBody.textContent = `Session à ${pct} %, rythme tenable jusqu'à la réinitialisation dans ${fmt(RESET_MIN)}.`;
      return;
    }
    toastTitle.textContent =
      threshold !== null ? `Client · Session de 5 h à ${pct} %` : "Client · Session de 5 h : rythme trop rapide";
    const pace = paceAlert ? ` Au rythme actuel, épuisé dans ~${fmt(toExhaust)}.` : "";
    toastBody.textContent = `Réinitialisation dans ${fmt(RESET_MIN)}.${pace} Basculez sur École (12 %).`;
  };
  range.addEventListener("input", render);
  // First paint after the ring scrolls into view, so the draw-in is visible.
  const io = new IntersectionObserver(
    (entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        render();
        io.disconnect();
      }
    },
    { threshold: 0.3 },
  );
  if (reduceMotion) render();
  else io.observe(demoSvg);
}

// Scroll reveals (CSS does the motion; reduced motion shows everything at once).
const revealables = document.querySelectorAll<HTMLElement>(".reveal");
if (reduceMotion || !("IntersectionObserver" in window)) {
  revealables.forEach((el) => el.classList.add("is-in"));
  miniPairs.forEach(({ svg, rings }) => drawMini(svg, rings));
} else {
  const io = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const el = entry.target as HTMLElement;
        el.classList.add("is-in");
        el.querySelectorAll<SVGSVGElement>(".mini-rings svg").forEach((svg) => {
          const pair = miniPairs.find((p) => p.svg === svg);
          if (pair) drawMini(pair.svg, pair.rings);
        });
        io.unobserve(el);
      }
    },
    { threshold: 0.15, rootMargin: "0px 0px -6% 0px" },
  );
  revealables.forEach((el) => io.observe(el));
}

function drawMini(svg: SVGSVGElement, [outer, inner]: [Ring, Ring]): void {
  setRing(outer, Number(svg.dataset.five ?? 0));
  setRing(inner, Number(svg.dataset.week ?? 0));
}
