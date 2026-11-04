//! Notices when a quota window resets (5 h session, 7 days, per-model weekly limits).
//!
//! The snapshot is replaced by the next readout as soon as a window resets, so the old `resets_at` is
//! gone by the time we look again: each upcoming reset is remembered until it is due, then fired once.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::statusline::Snapshot;

const FILE: &str = "pending-resets.json";
/// A reset noticed later than this (app closed at the time) is stale: no notification.
const GRACE_SECS: i64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    pub account: String,
    pub window: String,
    pub resets_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// The notification for a reset (desktop now, phones through the relay schedule).
pub fn notice(p: &Pending) -> Notice {
    Notice {
        title: format!("{} · {}", p.account, p.window),
        body: "Quota réinitialisé : de nouveau disponible à 100 %.".into(),
    }
}

/// Upcoming resets keyed by `<account id>/<window>`.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Tracker {
    pending: BTreeMap<String, Pending>,
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

    /// Records the next reset of every window that has been used. Returns whether anything changed.
    pub fn observe(&mut self, account_id: &str, account: &str, s: &Snapshot, now: i64) -> bool {
        let windows = [
            ("Session de 5 h".to_string(), &s.five_hour),
            ("7 jours".to_string(), &s.seven_day),
        ]
        .into_iter()
        .filter_map(|(w, l)| l.as_ref().map(|l| (w, l.used_percentage, l.resets_at)))
        .chain(s.scoped.iter().map(|l| {
            (
                format!("{} · 7 jours", l.label),
                l.used_percentage,
                l.resets_at,
            )
        }));
        let mut changed = false;
        for (window, pct, resets_at) in windows {
            if resets_at <= now || pct <= 0.0 {
                continue;
            }
            let p = Pending {
                account: account.to_string(),
                window: window.clone(),
                resets_at,
            };
            let key = format!("{account_id}/{window}");
            if self.pending.get(&key) != Some(&p) {
                self.pending.insert(key, p);
                changed = true;
            }
        }
        changed
    }

    /// Removes the resets that are due and returns the notifications for the recent ones.
    pub fn due(&mut self, now: i64) -> Vec<Notice> {
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| p.resets_at <= now)
            .map(|(k, _)| k.clone())
            .collect();
        due.into_iter()
            .filter_map(|k| self.pending.remove(&k))
            .filter(|p| now - p.resets_at <= GRACE_SECS)
            .map(|p| notice(&p))
            .collect()
    }

    /// Upcoming resets with their `<account id>/<window>` key, soonest first.
    pub fn upcoming(&self, now: i64) -> Vec<(String, Pending)> {
        let mut v: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.resets_at > now)
            .map(|(k, p)| (k.clone(), p.clone()))
            .collect();
        v.sort_by_key(|(_, p)| p.resets_at);
        v
    }

    /// Drops the accounts that no longer exist.
    pub fn retain_accounts(&mut self, ids: &[String]) -> bool {
        let before = self.pending.len();
        self.pending
            .retain(|k, _| ids.iter().any(|id| k.starts_with(&format!("{id}/"))));
        before != self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statusline::{Limit, ScopedLimit};

    fn snap(five: (f64, i64), week: (f64, i64)) -> Snapshot {
        Snapshot {
            captured_at: 0,
            five_hour: Some(Limit {
                used_percentage: five.0,
                resets_at: five.1,
            }),
            seven_day: Some(Limit {
                used_percentage: week.0,
                resets_at: week.1,
            }),
            scoped: vec![ScopedLimit {
                label: "Fable".into(),
                used_percentage: 10.0,
                resets_at: week.1,
            }],
        }
    }

    #[test]
    fn fires_once_after_reset_even_when_snapshot_moved_on() {
        let mut t = Tracker::default();
        assert!(t.observe("a", "Perso", &snap((40.0, 1000), (20.0, 90_000)), 500));
        assert!(
            !t.observe("a", "Perso", &snap((40.0, 1000), (20.0, 90_000)), 600),
            "same state: no change"
        );
        assert!(t.due(999).is_empty());
        // Next readout already shows the new window; the old reset must still fire.
        t.observe("a", "Perso", &snap((0.0, 19_000), (20.0, 90_000)), 1100);
        let n = t.due(1100);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title, "Perso · Session de 5 h");
        assert!(t.due(1200).is_empty(), "fired only once");
    }

    #[test]
    fn skips_unused_windows_and_stale_resets() {
        let mut t = Tracker::default();
        t.observe("a", "Perso", &snap((0.0, 1000), (20.0, 2000)), 500);
        // App was closed past the reset: too late to be useful.
        assert!(t.due(2000 + GRACE_SECS + 1).is_empty());
        assert!(t.due(10_000_000).is_empty(), "stale entries were dropped");
    }

    #[test]
    fn scoped_limits_and_removed_accounts() {
        let mut t = Tracker::default();
        t.observe("a", "Perso", &snap((10.0, 1000), (20.0, 2000)), 500);
        let n = t.due(2000);
        assert!(n.iter().any(|n| n.title == "Perso · Fable · 7 jours"));
        t.observe("b", "Studio", &snap((10.0, 5000), (20.0, 6000)), 3000);
        assert!(t.retain_accounts(&["a".into()]));
        assert!(t.due(6000).is_empty());
    }
}
