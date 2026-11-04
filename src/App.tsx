import { type ReactNode, useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Plus, Square, X, Terminal, AppWindow, Pencil, LayoutGrid, Check, KeyRound, Info, RefreshCw } from "lucide-react";
import { Account, AccountInput, api, errorMessage, PLAN_LABEL, UsageSummary } from "./api";
import { levelColor, Rings } from "./components/Rings";
import { AccountSheet } from "./components/AccountSheet";
import { Splash } from "./components/Splash";
import { AlertsPopover } from "./components/AlertsPopover";
import { daysUntil, longDate, money, relativeTime, tokens } from "./format";

type View = { kind: "overview" } | { kind: "account"; id: string };

function TitleBar(props: { onRefresh: () => void; refreshing: boolean; onToast: (msg: string) => void }) {
  const { onRefresh, refreshing } = props;
  const win = getCurrentWindow();
  return (
    <div className="titlebar" data-tauri-drag-region>
      <AlertsPopover onToast={props.onToast} />
      <button
        className="refresh-btn"
        onClick={onRefresh}
        disabled={refreshing}
        title="Relever maintenant les quotas officiels de chaque compte connecté"
      >
        <RefreshCw size={13} className={refreshing ? "spin" : undefined} />
        <span>{refreshing ? "Actualisation…" : "Actualiser"}</span>
      </button>
      <div className="window-controls">
        <button aria-label="Réduire" onClick={() => win.minimize()}>
          <Minus size={14} />
        </button>
        <button aria-label="Agrandir" onClick={() => win.toggleMaximize()}>
          <Square size={11} />
        </button>
        <button aria-label="Fermer" title="Fermer (reste actif près de l'horloge)" className="close" onClick={() => win.close()}>
          <X size={15} />
        </button>
      </div>
    </div>
  );
}

function useToast() {
  const [toast, setToast] = useState<{ text: string; error?: boolean } | null>(null);
  const show = (text: string, error = false) => {
    setToast({ text, error });
    window.setTimeout(() => setToast(null), 3200);
  };
  return { toast, show };
}

export default function App() {
  const qc = useQueryClient();
  const accounts = useQuery({ queryKey: ["accounts"], queryFn: api.listAccounts });
  const usage = useQuery({ queryKey: ["usage"], queryFn: api.getUsage, refetchInterval: 30_000 });
  const rec = useQuery({ queryKey: ["recommend"], queryFn: api.recommend, refetchInterval: 30_000 });
  const persistent = useQuery({ queryKey: ["persistent"], queryFn: api.getPersistentProfile });
  const [view, setView] = useState<View>({ kind: "overview" });
  const [sheet, setSheet] = useState<{ account: Account | null } | null>(null);
  const { toast, show } = useToast();

  const list = accounts.data ?? [];
  const usageOf = (id: string) => usage.data?.find((u) => u.accountId === id);
  const refresh = () => qc.invalidateQueries();

  const save = useMutation({
    mutationFn: api.upsertAccount,
    onSuccess: (a) => {
      setSheet(null);
      setView({ kind: "account", id: a.id });
      refresh();
    },
    onError: (e) => show(errorMessage(e), true),
  });

  const run = async (fn: () => Promise<unknown>, ok: string) => {
    try {
      await fn();
      show(ok);
      refresh();
    } catch (e) {
      show(errorMessage(e), true);
    }
  };

  const quotas = useMutation({
    mutationFn: api.refreshQuotas,
    onSuccess: (r) => {
      refresh();
      if (r.failed.length) show(`Échec du relevé — ${r.failed.join(" ; ")}`, true);
      else if (r.refreshed === 0) show("Aucun compte connecté à relever");
      else show(`Quotas à jour (${r.refreshed} compte${r.refreshed > 1 ? "s" : ""})`);
    },
    onError: (e) => show(errorMessage(e), true),
  });

  // The splash stays at least a beat so it never just flashes.
  const [minShown, setMinShown] = useState(false);
  useEffect(() => {
    const t = window.setTimeout(() => setMinShown(true), 900);
    return () => window.clearTimeout(t);
  }, []);
  const booting = accounts.isLoading || !minShown;

  const current = view.kind === "account" ? list.find((a) => a.id === view.id) : undefined;

  return (
    <div className="shell">
      <AnimatePresence>{booting && <Splash key="splash" />}</AnimatePresence>
      <aside className="sidebar">
        <div className="sidebar-drag" data-tauri-drag-region />
        <nav aria-label="Comptes">
          <button className={`nav-item ${view.kind === "overview" ? "active" : ""}`} onClick={() => setView({ kind: "overview" })}>
            <LayoutGrid size={16} className="nav-icon" />
            <span>Aperçu</span>
          </button>
          <p className="nav-heading">Comptes</p>
          {list.map((a) => {
            const u = usageOf(a.id);
            return (
              <button
                key={a.id}
                className={`nav-item ${view.kind === "account" && view.id === a.id ? "active" : ""}`}
                onClick={() => setView({ kind: "account", id: a.id })}
              >
                <Rings fiveHour={u?.fiveHour.pct ?? null} weekly={u?.weekly.pct ?? null} size={20} stroke={3} />
                <span className="nav-label">{a.label}</span>
                {rec.data?.accountId === a.id && <span className="badge-dot" aria-label="Recommandé" />}
              </button>
            );
          })}
        </nav>
        <button className="nav-item add" onClick={() => setSheet({ account: null })}>
          <Plus size={16} className="nav-icon" />
          <span>Ajouter un compte</span>
        </button>
      </aside>

      <main className="content">
        <TitleBar
          onRefresh={() => quotas.mutate()}
          refreshing={quotas.isPending}
          onToast={(m) => show(m)}
        />
        <div className="scroll">
          <AnimatePresence mode="wait">
            <motion.div
              key={view.kind === "overview" ? "overview" : view.id}
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.18 }}
            >
              {view.kind === "overview" || !current ? (
                <Overview
                  accounts={list}
                  usageOf={usageOf}
                  recommendedId={rec.data?.accountId ?? null}
                  reason={rec.data?.reason}
                  loading={accounts.isLoading}
                  onOpen={(id) => setView({ kind: "account", id })}
                  onAdd={() => setSheet({ account: null })}
                  onLaunch={(a) => run(() => api.launchCli(a.id), `Terminal ouvert sur ${a.label}`)}
                />
              ) : (
                <AccountDetail
                  account={current}
                  usage={usageOf(current.id)}
                  recommended={rec.data?.accountId === current.id}
                  isPersistent={persistent.data?.accountId === current.id}
                  onEdit={() => setSheet({ account: current })}
                  onLaunchCli={() => run(() => api.launchCli(current.id), "Terminal Claude Code ouvert")}
                  onLaunchDesktop={() => run(() => api.launchDesktop(current.id), "Claude Desktop lancé")}
                  onMakePersistent={() =>
                    run(() => api.setPersistentProfile(current.id), `${current.label} est le compte par défaut des nouveaux terminaux`)
                  }
                />
              )}
            </motion.div>
          </AnimatePresence>
        </div>
      </main>

      <AnimatePresence>
        {sheet && (
          <AccountSheet
            account={sheet.account}
            defaultTaken={list.some((a) => a.configDir === null && a.id !== sheet.account?.id)}
            onClose={() => setSheet(null)}
            onSave={async (input: AccountInput) => {
              await save.mutateAsync(input).catch(() => undefined);
            }}
            onDelete={
              sheet.account
                ? async () => {
                    const id = sheet.account!.id;
                    await run(() => api.deleteAccount(id), "Compte retiré");
                    setSheet(null);
                    setView({ kind: "overview" });
                  }
                : undefined
            }
          />
        )}
      </AnimatePresence>

      <AnimatePresence>
        {toast && (
          <motion.div
            className={`toast ${toast.error ? "error" : ""}`}
            role="status"
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 8 }}
          >
            {toast.text}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function pctText(p: number | null | undefined) {
  return p === null || p === undefined ? "—" : `${Math.round(p)} %`;
}

/** Legend row for one ring: level-coloured dot, name, value right-aligned. */
function QuotaRow(props: { label: string; ring: string; pct: number | null | undefined; tokens?: number; large?: boolean }) {
  const known = props.pct !== null && props.pct !== undefined;
  return (
    <div className={`quota${props.large ? " large" : ""}`} title={props.ring === "aucun" ? undefined : `Anneau ${props.ring}`}>
      <i style={{ background: levelColor(props.pct) }} />
      <span>{props.label}</span>
      <b className={known ? "" : "unknown"} style={known ? { color: levelColor(props.pct) } : undefined}>
        {!known && props.tokens ? `${tokens(props.tokens)} tok.` : pctText(props.pct)}
      </b>
    </div>
  );
}

const MODEL_LABEL: Record<string, string> = { opus: "Opus", sonnet: "Sonnet", haiku: "Haiku", fable: "Fable", "": "Autre" };

/** Re-renders every `ms` so countdowns stay current. */
function useNow(ms = 30_000) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

/** "4 h 12 min", "38 min", "3 j 5 h". */
function remaining(ms: number) {
  const min = Math.max(0, Math.round(ms / 60_000));
  if (min < 60) return `${min} min`;
  const h = Math.floor(min / 60);
  if (h < 48) return `${h} h ${String(min % 60).padStart(2, "0")} min`;
  return `${Math.floor(h / 24)} j ${h % 24} h`;
}

/** One quota window: used share, then time left before reset with a bar of the elapsed window. */
function WindowStat(props: { label: string; pct: number | null | undefined; resetsAt: string | null | undefined; spanH: number }) {
  const now = useNow();
  const left = props.resetsAt ? new Date(props.resetsAt).getTime() - now : null;
  const elapsed = left === null ? 0 : Math.min(1, Math.max(0, 1 - left / (props.spanH * 3_600_000)));
  const known = props.pct !== null && props.pct !== undefined;
  return (
    <div className="window-stat">
      <div className="window-head">
        <i style={{ background: levelColor(props.pct) }} />
        <span>{props.label}</span>
      </div>
      <div className="window-pct" style={known ? { color: levelColor(props.pct) } : undefined}>
        {pctText(props.pct)}
        {known && <small> utilisé</small>}
      </div>
      <div className="window-time">
        <div className="time-bar" aria-hidden>
          <span style={{ width: `${elapsed * 100}%` }} />
        </div>
        <span>{left === null || left <= 0 ? "Pas de fenêtre en cours" : <>Réinitialisation dans <b>{remaining(left)}</b></>}</span>
      </div>
    </div>
  );
}

/** Small (i) that reveals details on hover or keyboard focus. */
function InfoTip(props: { children: ReactNode }) {
  return (
    <span className="info-tip" tabIndex={0} aria-label="Détails">
      <Info size={16} />
      <span className="info-pop" role="tooltip">
        {props.children}
      </span>
    </span>
  );
}

function capNote(u?: UsageSummary) {
  if (!u || (u.capSource === "none" && u.weekly.tokens === 0)) return "Aucune conso enregistrée";
  if (u.capSource === "none") return "Pas encore assez d'historique pour un %";
  if (u.capSource === "official") return `Quotas officiels, relevés ${relativeTime(u.officialAt ?? "")}`;
  if (u.capSource === "peak") return "Estimé d'après votre plus forte utilisation";
  return "D'après vos plafonds";
}

function Overview(props: {
  accounts: Account[];
  usageOf: (id: string) => UsageSummary | undefined;
  recommendedId: string | null;
  reason?: string;
  loading: boolean;
  onOpen: (id: string) => void;
  onAdd: () => void;
  onLaunch: (a: Account) => void;
}) {
  const { accounts, usageOf, recommendedId } = props;
  if (!props.loading && accounts.length === 0) {
    return (
      <section className="empty">
        <Rings fiveHour={62} weekly={34} size={120} stroke={13} label="Illustration" />
        <h1>Vos comptes Claude, côte à côte</h1>
        <p>Ajoutez votre premier compte. Il reprendra votre configuration Claude Code actuelle, sans rien modifier.</p>
        <button className="btn primary large" onClick={props.onAdd}>
          Ajouter un compte
        </button>
      </section>
    );
  }
  const best = accounts.find((a) => a.id === recommendedId);
  const monthly = accounts.reduce<Record<string, number>>((acc, a) => {
    if (a.price) acc[a.price.currency] = (acc[a.price.currency] ?? 0) + a.price.amount;
    return acc;
  }, {});
  return (
    <section className="page">
      <header className="page-header">
        <h1>Aperçu</h1>
        <p className="subtle">
          {accounts.length} compte{accounts.length > 1 ? "s" : ""}
          {Object.entries(monthly).map(([c, v]) => ` — ${money(v, c)} par mois`)}
        </p>
      </header>

      {best && (
        <div className="hero" style={{ ["--account" as string]: best.color }}>
          <div>
            <p className="hero-kicker">À utiliser maintenant</p>
            <h2>{best.label}</h2>
            <p className="subtle">{props.reason}</p>
          </div>
          <button className="btn primary large" onClick={() => props.onLaunch(best)}>
            <Terminal size={16} /> Ouvrir Claude Code
          </button>
        </div>
      )}
      {!best && props.reason && accounts.length > 0 && <div className="hero muted">{props.reason}</div>}

      <div className="tiles">
        {accounts.map((a) => {
          const u = usageOf(a.id);
          return (
            <button key={a.id} className="tile" onClick={() => props.onOpen(a.id)}>
              <Rings fiveHour={u?.fiveHour.pct ?? null} weekly={u?.weekly.pct ?? null} size={88} stroke={10} />
              <div className="tile-text">
                <div className="tile-head">
                  <i style={{ background: a.color }} />
                  <strong>{a.label}</strong>
                  <span className="subtle">{PLAN_LABEL[a.plan]}</span>
                </div>
                <QuotaRow label="Session 5 h" ring="extérieur" pct={u?.fiveHour.pct} tokens={u?.fiveHour.tokens} />
                <QuotaRow label="7 jours" ring="intérieur" pct={u?.weekly.pct} tokens={u?.weekly.tokens} />
                {u?.scoped.map((s) => <QuotaRow key={s.label} label={`${s.label} 7 j`} ring="aucun" pct={s.pct} />)}
                <span className="tile-note">{capNote(u)}</span>
              </div>
            </button>
          );
        })}
      </div>
    </section>
  );
}

function AccountDetail(props: {
  account: Account;
  usage?: UsageSummary;
  recommended: boolean;
  isPersistent: boolean;
  onEdit: () => void;
  onLaunchCli: () => void;
  onLaunchDesktop: () => void;
  onMakePersistent: () => void;
}) {
  const { account: a, usage: u } = props;
  const profile = useQuery({ queryKey: ["profile", a.id], queryFn: () => api.profileStatus(a.id) });
  const renewal = a.renewalDate ? daysUntil(a.renewalDate) : null;
  return (
    <section className="page" style={{ ["--account" as string]: a.color }}>
      <header className="page-header row-between">
        <div>
          <h1>{a.label}</h1>
          <p className="subtle">
            {PLAN_LABEL[a.plan]}
            {a.email && ` — ${a.email}`}
          </p>
        </div>
        <button className="btn" onClick={props.onEdit}>
          <Pencil size={14} /> Modifier
        </button>
      </header>

      <div className="usage-card">
        <Rings fiveHour={u?.fiveHour.pct ?? null} weekly={u?.weekly.pct ?? null} size={176} stroke={18} />
        <div className="usage-stats">
          <WindowStat label="Session de 5 h" pct={u?.fiveHour.pct} resetsAt={u?.fiveHour.resetsAt} spanH={5} />
          <WindowStat label="7 jours" pct={u?.weekly.pct} resetsAt={u?.weekly.resetsAt} spanH={168} />
          {u?.scoped.map((sc) => (
            <WindowStat key={sc.label} label={`${sc.label} · 7 jours`} pct={sc.pct} resetsAt={sc.resetsAt} spanH={168} />
          ))}
        </div>
        <InfoTip>
          <p>
            {u?.capSource === "official" &&
              `Quotas officiels du compte (claude.ai, Desktop et Claude Code compris), relevés ${relativeTime(u.officialAt ?? "")}. Actualisés toutes les 3 minutes tant que le compte est connecté.`}
            {u?.capSource === "manual" && "Calculé sur les plafonds que vous avez saisis."}
            {u?.capSource === "peak" &&
              "Estimation : comparé à votre plus grosse session de 5 h et à votre plus grosse semaine passées sur ce compte (pas au quota officiel). La conso claude.ai et Desktop n'est pas comptée."}
            {u?.capSource === "none" &&
              "Pas encore de quota officiel ni assez d'historique pour estimer un %. Connectez Claude Code avec ce compte, ou saisissez vos plafonds dans « Modifier »."}
          </p>
          <p>
            Claude Code sur ce PC : {tokens(u?.fiveHour.tokens ?? 0)} tokens sur la session, {tokens(u?.weekly.tokens ?? 0)} sur 7 jours
            {u && u.weeklyByModel.length > 0 &&
              ` (${u.weeklyByModel.map((m) => `${MODEL_LABEL[m.model] ?? m.model} ${tokens(m.tokens)}`).join(", ")})`}
            .{u?.lastActivity && ` Dernière activité ${relativeTime(u.lastActivity)}.`}
          </p>
        </InfoTip>
      </div>

      <div className="actions">
        <button className={`btn large ${props.recommended ? "primary" : ""}`} onClick={props.onLaunchCli}>
          <Terminal size={16} /> Ouvrir Claude Code
        </button>
        <button className="btn large" onClick={props.onLaunchDesktop}>
          <AppWindow size={16} /> Ouvrir Claude Desktop
        </button>
      </div>

      <div className="group list">
        <div className="row">
          <span>Prix</span>
          <span className="value">{a.price ? `${money(a.price.amount, a.price.currency)} par mois` : "Non renseigné"}</span>
        </div>
        <div className="row">
          <span>Renouvellement</span>
          <span className="value">
            {a.renewalDate ? (
              <>
                {longDate(a.renewalDate)}
                {renewal !== null && renewal >= 0 && <span className="subtle"> dans {renewal} j</span>}
                {renewal !== null && renewal < 0 && <span className="warn"> date dépassée</span>}
              </>
            ) : (
              "Non renseigné"
            )}
          </span>
        </div>
        <div className="row">
          <span>Connexion Claude Code</span>
          <span className="value">
            {profile.data?.loggedIn ? (
              <span className="ok">
                <Check size={14} /> Connecté
              </span>
            ) : (
              <span className="warn">
                <KeyRound size={14} /> À connecter : ouvrez Claude Code puis tapez /login
              </span>
            )}
          </span>
        </div>
        <div className="row">
          <span>Nouveaux terminaux</span>
          <span className="value">
            {props.isPersistent ? (
              <span className="ok">
                <Check size={14} /> Compte par défaut
              </span>
            ) : (
              <button className="btn plain" onClick={props.onMakePersistent}>
                Utiliser par défaut
              </button>
            )}
          </span>
        </div>
        <div className="row">
          <span>Dossier du profil</span>
          <span className="value path" title={profile.data?.configDir}>
            {profile.data?.configDir}
            {a.configDir === null && <span className="subtle"> (profil principal)</span>}
          </span>
        </div>
      </div>
    </section>
  );
}
