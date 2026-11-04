import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Smartphone, X } from "lucide-react";
import { api, errorMessage, Pairing, PhoneDevice } from "../api";

/** Paired phones (Web Push): pairing by QR code, per-phone toggle, test and revoke. */
export function PhonesSection(props: { onToast: (msg: string) => void }) {
  const qc = useQueryClient();
  const phones = useQuery({ queryKey: ["phones"], queryFn: api.phoneList });
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [now, setNow] = useState(() => Date.now() / 1000);
  const setList = (list: PhoneDevice[]) => qc.setQueryData(["phones"], list);
  const fail = (e: unknown) => props.onToast(errorMessage(e));

  const start = useMutation({ mutationFn: api.phonePairStart, onSuccess: setPairing, onError: fail });
  const toggle = useMutation({
    mutationFn: (d: PhoneDevice) => api.phoneSetEnabled(d.id, !d.enabled),
    onSuccess: setList,
    onError: fail,
  });
  const revoke = useMutation({ mutationFn: (d: PhoneDevice) => api.phoneRevoke(d.id), onSuccess: setList, onError: fail });
  const test = useMutation({
    mutationFn: (d: PhoneDevice) => api.phoneTest(d.id),
    onSuccess: () => props.onToast("Notification de test envoyée au téléphone"),
    // A gone subscription is pruned by the backend: refresh the list either way.
    onError: (e) => (fail(e), qc.invalidateQueries({ queryKey: ["phones"] })),
  });

  // Polls the relay mailbox every 2 s until the phone answers or the code expires.
  useEffect(() => {
    if (!pairing) return;
    let stop = false;
    const tick = async () => {
      const t = Date.now() / 1000;
      setNow(t);
      if (t >= pairing.expiresAt) {
        setPairing(null);
        props.onToast("Code d'association expiré");
        return;
      }
      try {
        const d = await api.phonePairPoll(pairing.code);
        if (stop || !d) return;
        setPairing(null);
        void qc.invalidateQueries({ queryKey: ["phones"] });
        props.onToast(`${d.label} associé`);
      } catch (e) {
        if (!stop) (setPairing(null), fail(e));
      }
    };
    const id = setInterval(tick, 2000);
    return () => {
      stop = true;
      clearInterval(id);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pairing]);

  const list = phones.data ?? [];
  const left = pairing ? Math.max(0, Math.ceil((pairing.expiresAt - now) / 60)) : 0;

  return (
    <div className="phones">
      <div className="phones-title">
        <Smartphone size={13} aria-hidden /> Téléphones
      </div>
      {list.map((d) => (
        <div className="phone-row" key={d.id}>
          <label className="phone-name" title={`Via ${d.service}`}>
            <input type="checkbox" checked={d.enabled} disabled={toggle.isPending} onChange={() => toggle.mutate(d)} />
            <span>{d.label}</span>
          </label>
          <button className="phone-btn" disabled={!d.enabled || test.isPending} onClick={() => test.mutate(d)}>
            Tester
          </button>
          <button
            className="phone-btn icon"
            aria-label={`Retirer ${d.label}`}
            title="Retirer ce téléphone"
            disabled={revoke.isPending}
            onClick={() => revoke.mutate(d)}
          >
            <X size={12} />
          </button>
        </div>
      ))}
      {pairing ? (
        <div className="phone-pair">
          {/* SVG built by the backend from our own pairing URL. */}
          <div className="phone-qr" dangerouslySetInnerHTML={{ __html: pairing.qrSvg }} />
          <div className="phone-code" aria-label="Code d'association">
            {pairing.code}
          </div>
          <p className="alerts-hint">
            Scannez avec l'appareil photo du téléphone, ou ouvrez ai-accounts.trigenys.com/app et saisissez le code. Expire
            dans {left} min.
          </p>
          <button className="alerts-test" onClick={() => setPairing(null)}>
            Annuler
          </button>
        </div>
      ) : (
        <button className="alerts-test" disabled={start.isPending} onClick={() => start.mutate()}>
          {start.isPending ? "Préparation…" : "Associer un téléphone"}
        </button>
      )}
      <p className="alerts-hint">
        Alertes et réinitialisations arrivent aussi sur le téléphone, réinitialisations comprises PC éteint. Seuls les
        pourcentages et heures de réinitialisation partent, chiffrés de bout en bout.
      </p>
    </div>
  );
}
