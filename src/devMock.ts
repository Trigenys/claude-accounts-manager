// Browser-only preview (`pnpm dev` outside Tauri): fakes the backend with sample data.
const now = Date.now();
const iso = (ms: number) => new Date(ms).toISOString();
const acc = (id: string, label: string, plan: string, color: string, amount: number, configDir: string | null) => ({
  id,
  label,
  email: `${id}@exemple.com`,
  plan,
  price: { amount, currency: "EUR" },
  renewalDate: "2026-11-18",
  color,
  configDir,
  desktopUserDataDir: null,
  fiveHourTokenCap: null,
  weeklyTokenCap: null,
  createdAt: iso(now),
  updatedAt: iso(now),
});
const accounts = [
  acc("perso", "Perso", "max5x", "#0A84FF", 100, null),
  acc("studio", "Studio", "max20x", "#BF5AF2", 200, "C:/profiles/studio/cli"),
  acc("ecole", "École", "pro", "#30D158", 18, "C:/profiles/ecole/cli"),
  acc("client", "Client", "pro", "#FF9F0A", 18, "C:/profiles/client/cli"),
];
const pcts: Record<string, [number, number]> = { perso: [78, 52], studio: [34, 61], ecole: [12, 20], client: [96, 40] };
const usage = accounts.map((a) => ({
  accountId: a.id,
  fiveHour: { tokens: pcts[a.id][0] * 4000, cap: 400000, pct: pcts[a.id][0], resetsAt: iso(now + 2 * 3600e3) },
  weekly: { tokens: pcts[a.id][1] * 30000, cap: 3e6, pct: pcts[a.id][1], resetsAt: iso(now + 5 * 86400e3) },
  capSource: "official",
  officialAt: iso(now - 60e3),
  scoped: a.plan === "max5x" ? [{ label: "Fable", pct: 23, resetsAt: iso(now + 4 * 86400e3) }] : [],
  weeklyByModel: [
    { model: "opus", tokens: pcts[a.id][1] * 20000 },
    { model: "fable", tokens: pcts[a.id][1] * 10000 },
  ],
  lastActivity: iso(now - 20 * 60e3),
  hasTranscripts: true,
}));

let settings = { cliPathOverride: null, recommend: {}, alerts: { enabled: true, thresholds: [50, 75, 90], pace: true, soundFrom: 90 } };

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

// Phones: pairing answers on the third poll; a phone labelled "Ancien" behaves as an expired subscription.
let phones = [{ id: "p1", label: "Android · Chrome", enabled: true, pairedAt: Date.now() / 1000 - 86400, service: "fcm.googleapis.com" }];
let polls = 0;
const qr = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21" width="200" height="200"><rect width="21" height="21" fill="#fff"/><path d="M1 1h7v7H1zM13 1h7v7h-7zM1 13h7v7H1zM10 10h2v2h-2z" fill="#000"/></svg>`;

const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
  list_accounts: () => wait(1200).then(() => accounts),
  test_notification: () => undefined,
  phone_pair_start: () => {
    polls = 0;
    return wait(400).then(() => ({ code: "ABCD2345EF", url: "https://ai-accounts.trigenys.com/app/#pair=ABCD2345EF", qrSvg: qr, expiresAt: Date.now() / 1000 + 600 }));
  },
  phone_pair_poll: () => {
    if (++polls < 3) return null;
    const d = { id: `p${Date.now()}`, label: "iPhone · Safari", enabled: true, pairedAt: Date.now() / 1000, service: "web.push.apple.com" };
    phones = [...phones, d];
    return d;
  },
  phone_list: () => phones,
  phone_set_enabled: (a) => (phones = phones.map((p) => (p.id === a.id ? { ...p, enabled: a.enabled as boolean } : p))),
  phone_revoke: (a) => wait(300).then(() => (phones = phones.filter((p) => p.id !== a.id))),
  phone_test: (a) => {
    const p = phones.find((x) => x.id === a.id);
    if (p?.label === "Ancien") {
      phones = phones.filter((x) => x.id !== a.id);
      throw { code: "phones", message: "Ce téléphone n'est plus abonné : il a été retiré." };
    }
    return wait(500).then(() => undefined);
  },
  get_settings: () => settings,
  save_settings: (a) => (settings = a.settings as typeof settings),
  refresh_quotas: () => wait(2000).then(() => ({ refreshed: 3, failed: [] })),
  get_usage: () => usage,
  recommend: () => ({ accountId: "ecole", reason: "École est le moins entamé (score 15/100).", scores: [] }),
  get_persistent_profile: () => ({ current: null, accountId: "perso" }),
  detect_account_info: () => ({ email: "perso@exemple.com", displayName: "Perso", plan: "max5x", subscriptionCreatedAt: "2026-10-31T18:43:22Z", nextRenewal: "2026-11-30" }),
  profile_status: (a) => ({ configDir: "C:/Users/demo/.claude", isDefault: a.id === "perso", loggedIn: a.id !== "client", actions: [], warnings: [] }),
};

export function installDevMock() {
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
  if (w.__TAURI_INTERNALS__) return;
  w.__TAURI_INTERNALS__ = {
    invoke: async (cmd: string, args: Record<string, unknown>) => {
      const h = handlers[cmd];
      if (!h) throw { code: "mock", message: `Aperçu navigateur : « ${cmd} » indisponible` };
      return h(args ?? {});
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback: () => 0,
  };
}
