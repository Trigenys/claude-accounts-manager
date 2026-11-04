import { useState } from "react";
import { motion } from "motion/react";
import { ACCOUNT_COLORS, Account, AccountInput, api, errorMessage, Plan, PLAN_LABEL, PLAN_LIST_PRICE } from "../api";

interface Props {
  account: Account | null;
  /** True when another account already uses ~/.claude. */
  defaultTaken: boolean;
  onSave: (input: AccountInput) => Promise<void>;
  onDelete?: () => Promise<void>;
  onClose: () => void;
}

const numOrNull = (v: string) => (v.trim() === "" ? null : Number(v));

export function AccountSheet({ account, defaultTaken, onSave, onDelete, onClose }: Props) {
  const isDefault = account ? account.configDir === null : !defaultTaken;
  const [form, setForm] = useState({
    label: account?.label ?? "",
    email: account?.email ?? "",
    plan: (account?.plan ?? "pro") as Plan,
    amount: account?.price?.amount?.toString() ?? "",
    currency: account?.price?.currency ?? "EUR",
    renewalDate: account?.renewalDate ?? "",
    color: account?.color ?? ACCOUNT_COLORS[0],
    isDefaultProfile: isDefault,
    fiveHourCap: account?.fiveHourTokenCap?.toString() ?? "",
    weeklyCap: account?.weeklyTokenCap?.toString() ?? "",
  });
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [detectNote, setDetectNote] = useState<string | null>(null);
  // A new isolated profile has no login yet; only existing or default profiles can be read.
  const canDetect = Boolean(account) || form.isDefaultProfile;
  const set = <K extends keyof typeof form>(k: K, v: (typeof form)[K]) => setForm((f) => ({ ...f, [k]: v }));

  async function detect() {
    try {
      const id = await api.detectAccountInfo(account?.id);
      if (!id.email && !id.plan) {
        setDetectNote("Aucune connexion trouvée : lancez Claude Code avec ce compte puis /login.");
        return;
      }
      const listPrice = id.plan ? PLAN_LIST_PRICE[id.plan] : undefined;
      setForm((f) => ({
        ...f,
        label: f.label || id.displayName || "",
        email: id.email ?? f.email,
        plan: id.plan ?? f.plan,
        renewalDate: id.nextRenewal ?? f.renewalDate,
        ...(!f.amount && listPrice ? { amount: String(listPrice), currency: "USD" } : {}),
      }));
      setDetectNote(id.nextRenewal ? "Renouvellement estimé : mensuel, à la date d'abonnement. Prix public, à ajuster." : "Rempli depuis la connexion.");
    } catch (e) {
      setDetectNote(errorMessage(e));
    }
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await onSave({
        id: account?.id,
        label: form.label,
        email: form.email,
        plan: form.plan,
        price: form.amount ? { amount: Number(form.amount), currency: form.currency } : null,
        renewalDate: form.renewalDate || null,
        color: form.color,
        isDefaultProfile: form.isDefaultProfile,
        fiveHourTokenCap: numOrNull(form.fiveHourCap),
        weeklyTokenCap: numOrNull(form.weeklyCap),
      });
    } finally {
      setBusy(false);
    }
  }

  return (
    <motion.div className="scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onMouseDown={onClose}>
      <motion.form
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="sheet-title"
        initial={{ y: -24, opacity: 0, scale: 0.98 }}
        animate={{ y: 0, opacity: 1, scale: 1 }}
        exit={{ y: -16, opacity: 0, scale: 0.98 }}
        transition={{ type: "spring", stiffness: 380, damping: 32 }}
        onMouseDown={(e) => e.stopPropagation()}
        onSubmit={submit}
        onKeyDown={(e) => e.key === "Escape" && onClose()}
      >
        <div className="sheet-head">
          <h2 id="sheet-title">{account ? "Modifier le compte" : "Nouveau compte"}</h2>
          {canDetect && (
            <button type="button" className="btn-link" onClick={detect}>
              Remplir depuis la connexion
            </button>
          )}
        </div>
        {detectNote && <p className="sheet-note">{detectNote}</p>}

        <div className="group">
          <label className="row">
            <span>Nom</span>
            <input autoFocus required value={form.label} onChange={(e) => set("label", e.target.value)} placeholder="Perso, Studio…" />
          </label>
          <label className="row">
            <span>Email</span>
            <input type="email" value={form.email} onChange={(e) => set("email", e.target.value)} placeholder="vous@exemple.com" />
          </label>
          <div className="row">
            <span>Couleur</span>
            <div className="swatches" role="radiogroup" aria-label="Couleur">
              {ACCOUNT_COLORS.map((c) => (
                <button
                  type="button"
                  key={c}
                  role="radio"
                  aria-checked={form.color === c}
                  aria-label={c}
                  className="swatch"
                  style={{ background: c }}
                  onClick={() => set("color", c)}
                />
              ))}
            </div>
          </div>
        </div>

        <div className="group">
          <div className="row">
            <span>Forfait</span>
            <div className="segmented" role="radiogroup" aria-label="Forfait">
              {(Object.keys(PLAN_LABEL) as Plan[]).map((p) => (
                <button type="button" key={p} role="radio" aria-checked={form.plan === p} onClick={() => set("plan", p)}>
                  {PLAN_LABEL[p]}
                </button>
              ))}
            </div>
          </div>
          <label className="row">
            <span>Prix mensuel</span>
            <div className="inline">
              <input inputMode="decimal" value={form.amount} onChange={(e) => set("amount", e.target.value)} placeholder="20" className="short" />
              <select value={form.currency} onChange={(e) => set("currency", e.target.value)} aria-label="Devise">
                {["EUR", "USD", "XAF", "GBP", "CAD"].map((c) => (
                  <option key={c}>{c}</option>
                ))}
              </select>
            </div>
          </label>
          <label className="row">
            <span>Renouvellement</span>
            <input type="date" value={form.renewalDate} onChange={(e) => set("renewalDate", e.target.value)} />
          </label>
        </div>

        <div className="group">
          <label className="row toggle-row">
            <span>
              Profil principal
              <small>Utilise votre configuration Claude Code actuelle (~/.claude).</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              className="switch"
              checked={form.isDefaultProfile}
              disabled={!isDefault && defaultTaken}
              onChange={(e) => set("isDefaultProfile", e.target.checked)}
            />
          </label>
        </div>

        <details className="group advanced">
          <summary>Plafonds de tokens</summary>
          <p className="hint">Sans valeur, le pourcentage est estimé par rapport à votre pic d'utilisation passé sur ce compte.</p>
          <label className="row">
            <span>Fenêtre de 5 h</span>
            <input inputMode="numeric" value={form.fiveHourCap} onChange={(e) => set("fiveHourCap", e.target.value)} placeholder="Automatique" />
          </label>
          <label className="row">
            <span>Semaine</span>
            <input inputMode="numeric" value={form.weeklyCap} onChange={(e) => set("weeklyCap", e.target.value)} placeholder="Automatique" />
          </label>
        </details>

        <footer>
          {onDelete &&
            (confirmDelete ? (
              <button type="button" className="btn danger" onClick={onDelete}>
                Retirer définitivement
              </button>
            ) : (
              <button type="button" className="btn plain danger-text" onClick={() => setConfirmDelete(true)}>
                Retirer le compte
              </button>
            ))}
          <span className="spacer" />
          <button type="button" className="btn" onClick={onClose}>
            Annuler
          </button>
          <button type="submit" className="btn primary" disabled={busy}>
            {account ? "Enregistrer" : "Ajouter"}
          </button>
        </footer>
        {confirmDelete && <p className="hint">Le compte disparaît de la liste ; sa connexion et son historique restent sur le disque.</p>}
      </motion.form>
    </motion.div>
  );
}
