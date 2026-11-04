use serde::Serialize;

use crate::model::{Account, RecommendSettings};
use crate::usage::UsageSummary;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub account_id: String,
    pub score: f64,
    pub excluded: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub account_id: Option<String>,
    pub reason: String,
    pub scores: Vec<Score>,
}

/// Lowest weighted usage wins; accounts at or above the 5 h threshold are excluded.
/// Ties keep display order. Unknown percentages count as 0 (no recorded usage).
pub fn recommend(
    accounts: &[Account],
    usage: &[UsageSummary],
    cfg: &RecommendSettings,
) -> Recommendation {
    let scores: Vec<Score> = accounts
        .iter()
        .map(|a| {
            let u = usage.iter().find(|u| u.account_id == a.id);
            let p5 = u.and_then(|u| u.five_hour.pct).unwrap_or(0.0);
            let pw = u.and_then(|u| u.weekly.pct).unwrap_or(0.0);
            Score {
                account_id: a.id.clone(),
                score: cfg.weight_five_hour * p5 + cfg.weight_weekly * pw,
                excluded: p5 >= cfg.exclude_above_five_hour_pct,
            }
        })
        .collect();

    let best = scores
        .iter()
        .filter(|s| !s.excluded)
        .fold(None::<&Score>, |acc, s| match acc {
            Some(b) if b.score <= s.score => Some(b),
            _ => Some(s),
        });

    let (account_id, reason) = match best {
        Some(s) => {
            let label = accounts
                .iter()
                .find(|a| a.id == s.account_id)
                .map(|a| a.label.as_str())
                .unwrap_or("?");
            (
                Some(s.account_id.clone()),
                format!("{label} est le moins entamé (score {:.0}/100).", s.score),
            )
        }
        None if accounts.is_empty() => (None, "Aucun compte configuré.".into()),
        None => (
            None,
            format!(
                "Tous les comptes dépassent {:.0} % sur la fenêtre de 5 h.",
                cfg.exclude_above_five_hour_pct
            ),
        ),
    };
    Recommendation {
        account_id,
        reason,
        scores,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Plan;
    use crate::usage::Window;

    fn acc(id: &str) -> Account {
        Account {
            id: id.into(),
            label: id.to_uppercase(),
            email: String::new(),
            plan: Plan::Pro,
            price: None,
            renewal_date: None,
            color: "#000".into(),
            config_dir: None,
            desktop_user_data_dir: None,
            five_hour_token_cap: None,
            weekly_token_cap: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn use_(id: &str, p5: f64, pw: f64) -> UsageSummary {
        let w = |p: f64| Window {
            tokens: 0,
            cap: Some(100),
            pct: Some(p),
            resets_at: None,
        };
        UsageSummary {
            account_id: id.into(),
            five_hour: w(p5),
            weekly: w(pw),
            cap_source: "manual",
            official_at: None,
            scoped: Vec::new(),
            weekly_by_model: Vec::new(),
            last_activity: None,
            has_transcripts: true,
        }
    }

    #[test]
    fn picks_lowest_and_excludes_saturated() {
        let accounts = [acc("a"), acc("b"), acc("c")];
        let usage = [
            use_("a", 95.0, 0.0),
            use_("b", 40.0, 40.0),
            use_("c", 20.0, 60.0),
        ];
        let r = recommend(&accounts, &usage, &RecommendSettings::default());
        assert_eq!(r.account_id.as_deref(), Some("c"));
        assert!(r.scores[0].excluded);
    }

    #[test]
    fn tie_keeps_display_order() {
        let accounts = [acc("a"), acc("b")];
        let r = recommend(&accounts, &[], &RecommendSettings::default());
        assert_eq!(r.account_id.as_deref(), Some("a"));
    }

    #[test]
    fn all_saturated_gives_none() {
        let accounts = [acc("a")];
        let r = recommend(
            &accounts,
            &[use_("a", 99.0, 0.0)],
            &RecommendSettings::default(),
        );
        assert_eq!(r.account_id, None);
    }
}
