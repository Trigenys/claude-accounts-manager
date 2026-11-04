use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub schema_version: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub accounts: Vec<Account>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            settings: Settings::default(),
            accounts: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub cli_path_override: Option<String>,
    pub recommend: RecommendSettings,
    pub alerts: AlertSettings,
    pub phones: PhoneSettings,
}

/// Phone notifications (Web Push). The relay hosts the PWA, the pairing mailbox and the reset schedule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PhoneSettings {
    pub relay_url: String,
}

impl Default for PhoneSettings {
    fn default() -> Self {
        Self {
            relay_url: "https://ai-accounts.trigenys.com".into(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cli_path_override: None,
            recommend: RecommendSettings::default(),
            alerts: AlertSettings::default(),
            phones: PhoneSettings::default(),
        }
    }
}

/// Usage alerts: a notification when a window crosses a threshold, or when its pace runs it out before reset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AlertSettings {
    pub enabled: bool,
    /// Percentages, any order.
    pub thresholds: Vec<f64>,
    /// Warn when the current rate would hit 100 % before the window resets.
    pub pace: bool,
    /// Alerts at or above this percentage (and pace alerts) play a sound; lower ones are silent.
    pub sound_from: f64,
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            thresholds: vec![50.0, 75.0, 90.0],
            pace: true,
            sound_from: 90.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RecommendSettings {
    pub weight_five_hour: f64,
    pub weight_weekly: f64,
    pub exclude_above_five_hour_pct: f64,
}

impl Default for RecommendSettings {
    fn default() -> Self {
        Self {
            weight_five_hour: 0.6,
            weight_weekly: 0.4,
            exclude_above_five_hour_pct: 90.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    Pro,
    Max5x,
    Max20x,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub email: String,
    pub plan: Plan,
    pub price: Option<Price>,
    /// ISO date (YYYY-MM-DD).
    pub renewal_date: Option<String>,
    pub color: String,
    /// `None` = default profile (`~/.claude`, no CLAUDE_CONFIG_DIR). At most one account.
    pub config_dir: Option<String>,
    /// `None` = the packaged Claude Desktop profile.
    pub desktop_user_data_dir: Option<String>,
    /// Optional manual caps overriding the historical-peak estimate.
    #[serde(default)]
    pub five_hour_token_cap: Option<u64>,
    #[serde(default)]
    pub weekly_token_cap: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Fields the user edits; ids, paths and timestamps are owned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInput {
    pub id: Option<String>,
    pub label: String,
    #[serde(default)]
    pub email: String,
    pub plan: Plan,
    pub price: Option<Price>,
    pub renewal_date: Option<String>,
    pub color: String,
    #[serde(default)]
    pub is_default_profile: bool,
    #[serde(default)]
    pub five_hour_token_cap: Option<u64>,
    #[serde(default)]
    pub weekly_token_cap: Option<u64>,
}
