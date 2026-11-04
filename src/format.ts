const nf = new Intl.NumberFormat("fr-FR", { notation: "compact", maximumFractionDigits: 1 });

export const tokens = (n: number) => nf.format(n);

export function relativeTime(iso: string | null, now = Date.now()): string {
  if (!iso) return "jamais";
  const diff = new Date(iso).getTime() - now;
  const rtf = new Intl.RelativeTimeFormat("fr", { numeric: "auto" });
  const abs = Math.abs(diff);
  if (abs < 3_600_000) return rtf.format(Math.round(diff / 60_000), "minute");
  if (abs < 86_400_000) return rtf.format(Math.round(diff / 3_600_000), "hour");
  return rtf.format(Math.round(diff / 86_400_000), "day");
}

export function money(amount: number, currency: string) {
  try {
    return new Intl.NumberFormat("fr-FR", { style: "currency", currency }).format(amount);
  } catch {
    return `${amount} ${currency}`;
  }
}

export function longDate(iso: string | null) {
  if (!iso) return null;
  return new Intl.DateTimeFormat("fr-FR", { day: "numeric", month: "long", year: "numeric" }).format(new Date(iso));
}

/** Days until a YYYY-MM-DD date (negative when past). */
export function daysUntil(iso: string, now = new Date()) {
  const d = new Date(`${iso}T00:00:00`);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  return Math.round((d.getTime() - today.getTime()) / 86_400_000);
}
