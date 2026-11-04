import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bell, BellOff } from "lucide-react";
import { AlertSettings, api, errorMessage } from "../api";
import { PhonesSection } from "./PhonesSection";

/** Bell in the title bar: usage-alert settings, saved as soon as they change. */
export function AlertsPopover(props: { onToast: (msg: string) => void }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const qc = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });
  const save = useMutation({
    mutationFn: (alerts: AlertSettings) => api.saveSettings({ ...settings.data!, alerts }),
    onSuccess: (s) => qc.setQueryData(["settings"], s),
    onError: (e) => props.onToast(errorMessage(e)),
  });
  const a = settings.data?.alerts;

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => box.current && !box.current.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  const set = (patch: Partial<AlertSettings>) => a && save.mutate({ ...a, ...patch });
  const setThreshold = (i: number, v: number) => {
    if (!a || Number.isNaN(v)) return;
    const t = [...a.thresholds];
    t[i] = Math.min(100, Math.max(1, Math.round(v)));
    set({ thresholds: [...new Set(t)].sort((x, y) => x - y) });
  };

  return (
    <div className="alerts" ref={box}>
      <button
        className="refresh-btn icon-only"
        onClick={() => setOpen((o) => !o)}
        aria-label="Alertes de consommation"
        aria-expanded={open}
        title="Alertes de consommation et de réinitialisation"
      >
        {a && !a.enabled ? <BellOff size={13} /> : <Bell size={13} />}
      </button>
      <AnimatePresence>
        {open && a && (
          <motion.div
            className="alerts-pop"
            role="dialog"
            aria-label="Alertes"
            initial={{ opacity: 0, y: -4, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.98 }}
            transition={{ duration: 0.16 }}
          >
            <label className="alerts-row">
              <span>Alertes de consommation</span>
              <input type="checkbox" checked={a.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
            </label>
            <fieldset className="alerts-body" disabled={!a.enabled}>
              <div className="alerts-row">
                <span>Seuils (5 h et 7 jours)</span>
                <span className="alerts-thresholds">
                  {a.thresholds.map((t, i) => (
                    <input
                      key={`${i}-${t}`}
                      type="number"
                      min={1}
                      max={100}
                      defaultValue={t}
                      aria-label={`Seuil ${i + 1}`}
                      onBlur={(e) => setThreshold(i, e.target.valueAsNumber)}
                      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                    />
                  ))}
                  <span className="alerts-unit">%</span>
                </span>
              </div>
              <label className="alerts-row">
                <span>Alerte de rythme</span>
                <input type="checkbox" checked={a.pace} onChange={(e) => set({ pace: e.target.checked })} />
              </label>
              <div className="alerts-row">
                <span>Son à partir de</span>
                <span className="alerts-thresholds">
                  <input
                    key={a.soundFrom}
                    type="number"
                    min={1}
                    max={100}
                    defaultValue={a.soundFrom}
                    aria-label="Son à partir de"
                    onBlur={(e) => !Number.isNaN(e.target.valueAsNumber) && set({ soundFrom: Math.min(100, Math.max(1, e.target.valueAsNumber)) })}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  />
                  <span className="alerts-unit">%</span>
                </span>
              </div>
            </fieldset>
            <p className="alerts-hint">
              Une alerte par seuil et par fenêtre, avec le temps restant et un compte moins entamé. Le rythme prévient si
              le quota s'épuise avant la réinitialisation. Les réinitialisations sont toujours signalées avec son.
            </p>
            <button
              className="alerts-test"
              onClick={() => api.testNotification().then(() => props.onToast("Notification de test envoyée"), (e) => props.onToast(errorMessage(e)))}
            >
              Tester une notification
            </button>
            <PhonesSection onToast={props.onToast} />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
