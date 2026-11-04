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

const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
  list_accounts: () => wait(1200).then(() => accounts),
  test_notification: () => undefined,
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
