import { invoke } from "@tauri-apps/api/core";

export type Plan = "pro" | "max5x" | "max20x" | "other";

export interface Price {
  amount: number;
  currency: string;
}

export interface Account {
  id: string;
  label: string;
  email: string;
  plan: Plan;
  price: Price | null;
  renewalDate: string | null;
  color: string;
  configDir: string | null;
  desktopUserDataDir: string | null;
  fiveHourTokenCap: number | null;
  weeklyTokenCap: number | null;
  createdAt: string;
  updatedAt: string;
}

export interface AccountInput {
  id?: string;
  label: string;
  email: string;
  plan: Plan;
  price: Price | null;
  renewalDate: string | null;
  color: string;
  isDefaultProfile: boolean;
  fiveHourTokenCap: number | null;
  weeklyTokenCap: number | null;
}

export interface UsageWindow {
  tokens: number;
  cap: number | null;
  pct: number | null;
  resetsAt: string | null;
}

export interface UsageSummary {
  accountId: string;
  fiveHour: UsageWindow;
  weekly: UsageWindow;
  capSource: "official" | "manual" | "peak" | "none";
  /** When Claude Code last reported the official quotas. */
  officialAt: string | null;
  /** Official limits scoped to a model or surface (e.g. Fable's weekly allowance). */
  scoped: { label: string; pct: number; resetsAt: string }[];
  /** Last 7 days by model family ("opus", "fable"…; "" = unknown), largest first. */
  weeklyByModel: { model: string; tokens: number }[];
  lastActivity: string | null;
  hasTranscripts: boolean;
}

export interface Recommendation {
  accountId: string | null;
  reason: string;
  scores: { accountId: string; score: number; excluded: boolean }[];
}

export interface ProfileState {
  configDir: string;
  isDefault: boolean;
  loggedIn: boolean;
  actions: string[];
  warnings: string[];
}

export interface PersistentState {
  current: string | null;
  accountId: string | null;
}

export interface CliInfo {
  path: string;
  version: string | null;
  source: string;
}

export interface AppError {
  code: string;
  message: string;
}

export const errorMessage = (e: unknown) =>
  typeof e === "object" && e && "message" in e ? String((e as AppError).message) : String(e);

export interface Identity {
  email: string | null;
  displayName: string | null;
  plan: Plan | null;
  subscriptionCreatedAt: string | null;
  nextRenewal: string | null;
}

/** Public monthly list prices (USD), used only to prefill an empty price. */
export const PLAN_LIST_PRICE: Partial<Record<Plan, number>> = { pro: 20, max5x: 100, max20x: 200 };

export const api = {
  detectAccountInfo: (id?: string) => invoke<Identity>("detect_account_info", { id: id ?? null }),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  upsertAccount: (input: AccountInput) => invoke<Account>("upsert_account", { input }),
  deleteAccount: (id: string) => invoke<void>("delete_account", { id }),
  detectCli: () => invoke<CliInfo>("detect_cli"),
  profileStatus: (id: string) => invoke<ProfileState>("profile_status", { id }),
  ensureCliProfile: (id: string) => invoke<ProfileState>("ensure_cli_profile", { id }),
  launchCli: (id: string) => invoke<void>("launch_cli", { id }),
  launchDesktop: (id: string) => invoke<void>("launch_desktop", { id }),
  getPersistentProfile: () => invoke<PersistentState>("get_persistent_profile"),
  setPersistentProfile: (id: string) => invoke<PersistentState>("set_persistent_profile", { id }),
  getUsage: () => invoke<UsageSummary[]>("get_usage"),
  recommend: () => invoke<Recommendation>("recommend"),
  /** Reads the official quotas of every logged-in account now (about 15 s). */
  refreshQuotas: () => invoke<RefreshReport>("refresh_quotas"),
  testNotification: () => invoke<void>("test_notification"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
};

/** Usage alerts: once per threshold and per window, plus a pace warning. */
export interface AlertSettings {
  enabled: boolean;
  thresholds: number[];
  pace: boolean;
  /** Silent below this level, with sound from it. */
  soundFrom: number;
}

/** Only `alerts` is edited here; the other fields are sent back unchanged. */
export interface Settings {
  alerts: AlertSettings;
  [key: string]: unknown;
}

export interface RefreshReport {
  refreshed: number;
  /** "<account> : <reason>" per account that could not be read. */
  failed: string[];
}

export const PLAN_LABEL: Record<Plan, string> = {
  pro: "Pro",
  max5x: "Max 5×",
  max20x: "Max 20×",
  other: "Autre",
};

export const ACCOUNT_COLORS = ["#0A84FF", "#30D158", "#FF9F0A", "#BF5AF2", "#FF375F", "#64D2FF"];
