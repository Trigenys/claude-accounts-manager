//! Usage alerts: a window crossed a threshold (50 / 75 / 90 % by default), or its current pace would
//! exhaust it before it resets. Each alert fires once per window occurrence, across restarts.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::AlertSettings;
use crate::statusline::Snapshot;

const FILE: &str = "usage-alerts.json";
const FIVE_HOURS: i64 = 5 * 3600;
const SEVEN_DAYS: i64 = 7 * 86400;
/// The official `resets_at` can move by a few seconds between readouts: still the same window.
const SAME_WINDOW_SECS: i64 = 300;

/// One quota window of one account, as read from its latest snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub account_id: String,
    pub account: String,
    pub name: String,
    pub pct: f64,
    pub resets_at: i64,
    pub span_secs: i64,
}

/// All windows of a snapshot that have not reset yet.
pub fn windows(account_id: &str, account: &str, s: &Snapshot, now: i64) -> Vec<Window> {
    let mk = |name: String, pct: f64, resets_at: i64, span_secs: i64| Window {
        account_id: account_id.into(),
        account: account.into(),
        name,
        pct,
        resets_at,
        span_secs,
    };
    s.five_hour
        .iter()
        .map(|l| {
            mk(
                "Session de 5 h".into(),
                l.used_percentage,
                l.resets_at,
                FIVE_HOURS,
            )
        })
        .chain(
            s.seven_day
                .iter()
                .map(|l| mk("7 jours".into(), l.used_percentage, l.resets_at, SEVEN_DAYS)),
        )
        .chain(s.scoped.iter().map(|l| {
            mk(
                format!("{} · 7 jours", l.label),
                l.used_percentage,
                l.resets_at,
                SEVEN_DAYS,
            )
        }))
        .filter(|w| w.resets_at > now)
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub title: String,
    pub body: String,
    pub sound: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Fired {
    resets_at: i64,
    levels: Vec<f64>,
    pace: bool,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Tracker {
    fired: BTreeMap<String, Fired>,
}

/// "35 min", "1 h 05", "2 j 5 h".
pub fn duration(secs: i64) -> String {
    let m = (secs.max(0) + 59) / 60;
    match m {
        m if m < 60 => format!("{m} min"),
        m if m < 24 * 60 => format!("{} h {:02}", m / 60, m % 60),
        m => format!("{} j {} h", m / 1440, (m % 1440) / 60),
    }
}

/// Seconds until 100 % at the average rate since the window opened, when that comes before the reset.
fn runs_out_in(w: &Window, now: i64) -> Option<i64> {
    let left = w.resets_at - now;
    let elapsed = w.span_secs - left;
    // Too early in the window, or too little usage, for the rate to mean anything.
    if elapsed < w.span_secs / 10 || w.pct < 10.0 || w.pct >= 100.0 {
        return None;
    }
    let to_full = ((100.0 - w.pct) * elapsed as f64 / w.pct) as i64;
    (to_full < left).then_some(to_full)
}

impl Tracker {
    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(dir.join(FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        if let Ok(b) = serde_json::to_vec(self) {
            let _ = fs::write(dir.join(FILE), b);
        }
    }

    /// Returns the alerts to show now, and whether the tracker changed (to persist it).
    pub fn check(&mut self, all: &[Window], cfg: &AlertSettings, now: i64) -> (Vec<Alert>, bool) {
        let before = self.fired.len();
        self.fired
            .retain(|_, f| f.resets_at + SAME_WINDOW_SECS > now);
        let mut changed = before != self.fired.len();
        if !cfg.enabled {
            return (Vec::new(), changed);
        }
        let mut alerts = Vec::new();
        for w in all {
            let key = format!("{}/{}", w.account_id, w.name);
            let rec = self.fired.entry(key).or_default();
            if (rec.resets_at - w.resets_at).abs() > SAME_WINDOW_SECS {
                *rec = Fired {
                    resets_at: w.resets_at,
                    ..Default::default()
                };
                changed = true;
            }
            let crossed: Vec<f64> = cfg
                .thresholds
                .iter()
                .copied()
                .filter(|t| w.pct >= *t && !rec.levels.contains(t))
                .collect();
            let pace = if cfg.pace && !rec.pace {
                runs_out_in(w, now)
            } else {
                None
            };
            if crossed.is_empty() && pace.is_none() {
                continue;
            }
            rec.levels.extend(&crossed);
            rec.pace |= pace.is_some();
            changed = true;

            let mut body = format!("Réinitialisation dans {}.", duration(w.resets_at - now));
            if let Some(t) = pace.or_else(|| runs_out_in(w, now)) {
                body += &format!(" Au rythme actuel, épuisé dans ~{}.", duration(t));
            }
            if let Some(alt) = suggestion(all, w) {
                body += &format!(" Basculez sur {} ({:.0} %).", alt.account, alt.pct);
            }
            let title = if crossed.is_empty() {
                format!("{} · {} : rythme trop rapide", w.account, w.name)
            } else {
                format!("{} · {} à {:.0} %", w.account, w.name, w.pct)
            };
            alerts.push(Alert {
                title,
                body,
                sound: pace.is_some() || w.pct >= cfg.sound_from,
            });
        }
        (alerts, changed)
    }
}

/// The least used other account on the same window, when it has real room left.
fn suggestion<'a>(all: &'a [Window], w: &Window) -> Option<&'a Window> {
    all.iter()
        .filter(|o| {
            o.account_id != w.account_id && o.name == w.name && o.pct < 50.0 && o.pct < w.pct
        })
        .min_by(|a, b| a.pct.total_cmp(&b.pct))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: &str, pct: f64, resets_at: i64) -> Window {
        Window {
            account_id: id.into(),
            account: id.to_uppercase(),
            name: "Session de 5 h".into(),
            pct,
            resets_at,
            span_secs: FIVE_HOURS,
        }
    }

    fn cfg() -> AlertSettings {
        AlertSettings {
            pace: false,
            ..Default::default()
        }
    }

    #[test]
    fn each_threshold_fires_once_per_window() {
        let mut t = Tracker::default();
        let now = 10_000;
        assert!(
            t.check(&[win("a", 40.0, now + 3600)], &cfg(), now)
                .0
                .is_empty()
        );
        let (a, changed) = t.check(&[win("a", 55.0, now + 3600)], &cfg(), now);
        assert!(changed);
        assert_eq!(a[0].title, "A · Session de 5 h à 55 %");
        assert!(!a[0].sound);
        assert_eq!(
            t.check(&[win("a", 60.0, now + 3600)], &cfg(), now),
            (vec![], false),
            "converged: no alert, no write"
        );
        // A jump over two thresholds gives one alert, with sound at 90 %.
        let a = t.check(&[win("a", 92.0, now + 3600)], &cfg(), now).0;
        assert_eq!(a.len(), 1);
        assert!(a[0].title.ends_with("à 92 %") && a[0].sound);
        // New window after the reset: thresholds are armed again.
        let later = now + 3600 + 10;
        assert_eq!(
            t.check(&[win("a", 51.0, later + FIVE_HOURS)], &cfg(), later)
                .0
                .len(),
            1
        );
    }

    #[test]
    fn suggests_the_least_used_other_account() {
        let mut t = Tracker::default();
        let now = 10_000;
        let all = [
            win("a", 76.0, now + 6000),
            win("b", 30.0, now + 100),
            win("c", 12.0, now + 9000),
        ];
        let a = t.check(&all, &cfg(), now).0;
        assert_eq!(
            a[0].body,
            "Réinitialisation dans 1 h 40. Au rythme actuel, épuisé dans ~1 h 04. Basculez sur C (12 %)."
        );
    }

    #[test]
    fn pace_alert_when_rate_runs_out_before_reset() {
        let mut t = Tracker::default();
        let on = AlertSettings {
            thresholds: vec![],
            ..Default::default()
        };
        let now = 100_000;
        // 40 % used in 1 h of a 5 h window: 100 % in 1 h 30, reset in 4 h.
        let a = t.check(&[win("a", 40.0, now + 4 * 3600)], &on, now).0;
        assert_eq!(a[0].title, "A · Session de 5 h : rythme trop rapide");
        assert!(a[0].body.contains("épuisé dans ~1 h 30") && a[0].sound);
        assert!(
            t.check(&[win("a", 45.0, now + 4 * 3600)], &on, now)
                .0
                .is_empty(),
            "once per window"
        );
        // 20 % in 4 h: plenty of room, no alert.
        assert!(
            Tracker::default()
                .check(&[win("a", 20.0, now + 3600)], &on, now)
                .0
                .is_empty()
        );
    }

    #[test]
    fn disabled_and_formatting() {
        let off = AlertSettings {
            enabled: false,
            ..Default::default()
        };
        assert!(
            Tracker::default()
                .check(&[win("a", 99.0, 20_000)], &off, 10_000)
                .0
                .is_empty()
        );
        assert_eq!(duration(35 * 60), "35 min");
        assert_eq!(duration(3900), "1 h 05");
        assert_eq!(duration(2 * 86400 + 5 * 3600), "2 j 5 h");
    }
}
