/* Service worker of the phone companion: shows pushed alerts and keeps the last snapshot for offline countdowns.
 * Payload (JSON, v1): {kind, title?, body?, tag?, sound, at, reset?, accounts?}. Never executes payload content. */
const DB = "cam", STORE = "kv";

function db() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function kv(mode, fn) {
  const d = await db();
  return new Promise((resolve, reject) => {
    const tx = d.transaction(STORE, mode);
    const out = fn(tx.objectStore(STORE));
    tx.oncomplete = () => resolve(out && "result" in out ? out.result : undefined);
    tx.onerror = () => reject(tx.error);
  });
}

const get = (k) => kv("readonly", (s) => s.get(k));
const put = (k, v) => kv("readwrite", (s) => s.put(v, k));

function pct(n) {
  return `${Math.round(n)} %`;
}

/** One line per account: the most used window, for the silent status notification. */
function summary(accounts) {
  return accounts
    .map((a) => {
      const top = [...a.windows].sort((x, y) => y.pct - x.pct)[0];
      return top ? `${a.label} : ${pct(top.pct)} (${top.name})` : a.label;
    })
    .join("\n");
}

async function handle(msg) {
  if (Array.isArray(msg.accounts)) {
    const prev = await get("snapshot");
    if (!prev || prev.at <= msg.at) await put("snapshot", { at: msg.at, accounts: msg.accounts });
  }
  if (msg.kind === "reset" && msg.reset) {
    const done = (await get("resets")) || {};
    done[msg.reset.key] = msg.reset.resetsAt;
    for (const [k, t] of Object.entries(done)) if (t < Date.now() / 1000 - 8 * 86400) delete done[k];
    await put("resets", done);
  }
  for (const c of await self.clients.matchAll({ type: "window" })) c.postMessage({ type: "updated" });

  const status = msg.kind === "status";
  const title = status ? "Quotas Claude" : String(msg.title || "Claude Accounts Manager");
  const body = status ? summary(msg.accounts || []) : String(msg.body || "");
  return self.registration.showNotification(title, {
    body,
    tag: status ? "cam-status" : msg.tag || undefined,
    renotify: !status && Boolean(msg.tag),
    silent: status || !msg.sound,
    icon: "/app/icon-192.png",
    badge: "/app/icon-192.png",
    timestamp: (msg.at || Date.now() / 1000) * 1000,
    data: { url: "/app/" },
  });
}

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

self.addEventListener("push", (event) => {
  let msg = null;
  try {
    msg = event.data ? event.data.json() : null;
  } catch {
    msg = null;
  }
  if (!msg || msg.v !== 1) {
    event.waitUntil(self.registration.showNotification("Claude Accounts Manager", { body: "Nouvelle notification", icon: "/app/icon-192.png" }));
    return;
  }
  event.waitUntil(handle(msg));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    (async () => {
      const wins = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      const open = wins.find((w) => new URL(w.url).pathname.startsWith("/app/"));
      if (open) return open.focus();
      return self.clients.openWindow("/app/");
    })(),
  );
});
