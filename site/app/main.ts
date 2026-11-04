import "@fontsource-variable/geist";
import "./app.css";

/** Phone companion: pairs with the desktop app over Web Push and shows the last pushed snapshot offline. */

interface PhoneWindow {
  name: string;
  pct: number;
  resetsAt: number;
}
interface PhoneAccount {
  id: string;
  label: string;
  capturedAt: number;
  windows: PhoneWindow[];
}
interface Snapshot {
  at: number;
  accounts: PhoneAccount[];
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const now = () => Date.now() / 1000;
const CODE_RE = /^[A-HJ-NP-Z2-9]{10}$/;

// ---------- IndexedDB (same store as sw.js) ----------

function db(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open("cam", 1);
    req.onupgradeneeded = () => req.result.createObjectStore("kv");
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function get<T>(key: string): Promise<T | undefined> {
  const d = await db();
  return new Promise((resolve, reject) => {
    const req = d.transaction("kv").objectStore("kv").get(key);
    req.onsuccess = () => resolve(req.result as T | undefined);
    req.onerror = () => reject(req.error);
  });
}

// ---------- formatting (same wording as the desktop app) ----------

/** "35 min", "1 h 05", "2 j 5 h". */
function duration(secs: number): string {
  const m = Math.ceil(Math.max(0, secs) / 60);
  if (m < 60) return `${m} min`;
  if (m < 24 * 60) return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
  return `${Math.floor(m / 1440)} j ${Math.floor((m % 1440) / 60)} h`;
}

function ago(secs: number): string {
  if (secs < 60) return "à l'instant";
  if (secs < 3600) return `il y a ${Math.floor(secs / 60)} min`;
  if (secs < 86400) return `il y a ${Math.floor(secs / 3600)} h`;
  return `il y a ${Math.floor(secs / 86400)} j`;
}

function level(pct: number): string {
  return pct >= 80 ? "alert" : pct >= 50 ? "warn" : "ok";
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

// ---------- snapshot view ----------

let snapshot: Snapshot | undefined;

function render() {
  const list = $("accounts");
  const t = now();
  list.replaceChildren();
  if (!snapshot || snapshot.accounts.length === 0) {
    $("empty").hidden = false;
    $("updated").textContent = "";
    return;
  }
  $("empty").hidden = true;
  $("updated").textContent = `Dernière réception ${ago(t - snapshot.at)}`;
  for (const a of snapshot.accounts) {
    const card = el("article", "card");
    const head = el("header", "card-head");
    head.append(el("h2", "card-title", a.label));
    const stale = t - a.capturedAt;
    head.append(el("span", stale > 15 * 60 ? "stale" : "fresh", `relevé ${ago(stale)}`));
    card.append(head);
    for (const w of a.windows) {
      const row = el("div", "win");
      const reset = w.resetsAt <= t;
      const top = el("div", "win-top");
      top.append(el("span", "win-name", w.name));
      top.append(el("span", `win-pct ${reset ? "done" : level(w.pct)}`, reset ? "100 % dispo" : `${Math.round(w.pct)} %`));
      const bar = el("div", "bar");
      const fill = el("div", `fill ${reset ? "done" : level(w.pct)}`);
      fill.style.width = `${reset ? 0 : Math.min(100, Math.max(0, w.pct))}%`;
      bar.append(fill);
      const when = el(
        "div",
        "win-reset",
        reset ? "Réinitialisé : de nouveau disponible" : `Réinitialisation dans ${duration(w.resetsAt - t)}`,
      );
      row.append(top, bar, when);
      card.append(row);
    }
    list.append(card);
  }
}

async function reload() {
  try {
    snapshot = await get<Snapshot>("snapshot");
  } catch {
    snapshot = undefined;
  }
  render();
}

// ---------- pairing ----------

const isIos = /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
const standalone = window.matchMedia("(display-mode: standalone)").matches || (navigator as { standalone?: boolean }).standalone === true;
const pushable = "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;

function deviceLabel(): string {
  const ua = navigator.userAgent;
  const os = /Android/.test(ua) ? "Android" : isIos ? (/iPad/.test(ua) ? "iPad" : "iPhone") : /Windows/.test(ua) ? "Windows" : /Mac/.test(ua) ? "Mac" : "Linux";
  const browser = /SamsungBrowser/.test(ua) ? "Samsung Internet" : /Edg\//.test(ua) ? "Edge" : /Firefox|FxiOS/.test(ua) ? "Firefox" : /Chrome|CriOS/.test(ua) ? "Chrome" : /Safari/.test(ua) ? "Safari" : "Navigateur";
  return `${os} · ${browser}`;
}

function b64urlToBytes(s: string): Uint8Array<ArrayBuffer> {
  const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (s.length % 4)) % 4));
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

function same(a: ArrayBuffer | null | undefined, b: Uint8Array): boolean {
  if (!a || a.byteLength !== b.length) return false;
  const x = new Uint8Array(a);
  return x.every((v, i) => v === b[i]);
}

function say(text: string, kind: "info" | "ok" | "error" = "info") {
  const m = $("pair-msg");
  m.textContent = text;
  m.className = `msg ${kind}`;
}

async function pair(code: string) {
  if (!CODE_RE.test(code)) return say("Code invalide : 10 caractères, comme affiché sur le PC.", "error");
  const button = $<HTMLButtonElement>("pair-btn");
  button.disabled = true;
  try {
    say("Connexion…");
    const offer = await fetch(`/api/pair/${code}`, { cache: "no-store" });
    if (offer.status === 404) return say("Code inconnu ou expiré : relancez « Associer un téléphone » sur le PC.", "error");
    if (!offer.ok) return say(`Serveur indisponible (${offer.status}). Réessayez.`, "error");
    const { vapidKey } = (await offer.json()) as { vapidKey: string };
    const key = b64urlToBytes(vapidKey);

    if ((await Notification.requestPermission()) !== "granted") {
      return say("Notifications refusées : autorisez-les dans les réglages du navigateur pour ce site.", "error");
    }
    const reg = await navigator.serviceWorker.ready;
    let sub = await reg.pushManager.getSubscription();
    if (sub && !same(sub.options.applicationServerKey, key)) {
      await sub.unsubscribe();
      sub = null;
    }
    sub ??= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });

    const res = await fetch(`/api/pair/${code}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ subscription: sub.toJSON(), label: deviceLabel() }),
    });
    if (res.status === 409) return say("Ce code a déjà servi : relancez l'association sur le PC.", "error");
    if (!res.ok) return say(`Association refusée (${res.status}).`, "error");
    say("Associé. Une notification de confirmation arrive dans quelques secondes.", "ok");
    history.replaceState(null, "", "/app/");
  } catch (e) {
    say(`Échec : ${e instanceof Error ? e.message : String(e)}`, "error");
  } finally {
    button.disabled = false;
  }
}

function setupPairing() {
  const input = $<HTMLInputElement>("code");
  const fromHash = /^#pair=([A-Za-z0-9]{10})$/.exec(location.hash)?.[1]?.toUpperCase();
  if (fromHash) input.value = fromHash;

  if (isIos && !standalone) {
    $("ios").hidden = false;
    $("pair-form").hidden = true;
    return;
  }
  if (!pushable) {
    say("Ce navigateur ne gère pas les notifications push. Sur iPhone : iOS 16.4 ou plus, depuis l'écran d'accueil.", "error");
    $("pair-form").hidden = true;
    return;
  }
  $("pair-form").addEventListener("submit", (e) => {
    e.preventDefault();
    void pair(input.value.trim().toUpperCase());
  });
  void navigator.serviceWorker.ready
    .then((reg) => reg.pushManager.getSubscription())
    .then((sub) => {
      if (sub && !fromHash) {
        $("pair-title").textContent = "Associer un autre PC";
        $("paired").hidden = false;
      }
    });
}

if ("serviceWorker" in navigator) {
  void navigator.serviceWorker.register("/app/sw.js", { scope: "/app/" });
  navigator.serviceWorker.addEventListener("message", (e: MessageEvent<{ type?: string }>) => {
    if (e.data?.type === "updated") void reload();
  });
}
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") void reload();
});
setupPairing();
void reload();
setInterval(render, 15_000);
