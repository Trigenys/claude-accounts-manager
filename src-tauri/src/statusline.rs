//! Official subscription quotas without touching credentials.
//!
//! Claude Code pipes a JSON status (including `rate_limits`, the claude.ai subscription usage it already
//! fetched with its own login) to the profile's `statusLine` command. This app registers itself as that
//! command (`<exe> --statusline`), stores the percentages in a snapshot next to the profile and shows them.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::AppResult;
use crate::usage::{ScopedUsage, UsageSummary};

pub const FLAG: &str = "--statusline";
const SNAPSHOT: &str = "account-manager-usage.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Limit {
    pub used_percentage: f64,
    /// Unix seconds.
    pub resets_at: i64,
}

/// A limit scoped to one model or surface (e.g. Fable's own weekly allowance).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScopedLimit {
    pub label: String,
    pub used_percentage: f64,
    pub resets_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    /// Unix seconds.
    pub captured_at: i64,
    pub five_hour: Option<Limit>,
    pub seven_day: Option<Limit>,
    #[serde(default)]
    pub scoped: Vec<ScopedLimit>,
}

/// Extracts the subscription windows from Claude Code's status JSON; `None` when it carries none.
pub fn from_status(input: &str, now: DateTime<Utc>) -> Option<Snapshot> {
    let v: Value = serde_json::from_str(input).ok()?;
    let rl = v.get("rate_limits")?;
    let limit = |k: &str| serde_json::from_value::<Limit>(rl.get(k)?.clone()).ok();
    let s = Snapshot {
        captured_at: now.timestamp(),
        five_hour: limit("five_hour"),
        seven_day: limit("seven_day"),
        scoped: Vec::new(),
    };
    (s.five_hour.is_some() || s.seven_day.is_some()).then_some(s)
}

/// Status-bar text shown in Claude Code.
pub fn render(s: Option<&Snapshot>) -> String {
    let part = |name: &str, l: &Option<Limit>| {
        l.as_ref()
            .map(|l| format!("{name} {:.0} %", l.used_percentage))
    };
    s.map(|s| {
        [part("5 h", &s.five_hour), part("7 j", &s.seven_day)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    })
    .unwrap_or_default()
}

/// Write-then-rename so readers never see a half-written file.
pub fn write(config_dir: &Path, s: &Snapshot) {
    let tmp = config_dir.join(format!("{SNAPSHOT}.tmp"));
    if serde_json::to_vec(s)
        .ok()
        .and_then(|b| fs::write(&tmp, b).ok())
        .is_some()
    {
        let _ = fs::rename(&tmp, config_dir.join(SNAPSHOT));
    }
}

pub fn read(config_dir: &Path) -> Option<Snapshot> {
    serde_json::from_str(&fs::read_to_string(config_dir.join(SNAPSHOT)).ok()?).ok()
}

/// Official percentages override the local estimate for every window that has not reset yet.
pub fn apply(u: &mut UsageSummary, s: &Snapshot, now: DateTime<Utc>) {
    let mut used = false;
    for (w, l) in [
        (&mut u.five_hour, &s.five_hour),
        (&mut u.weekly, &s.seven_day),
    ] {
        let Some(l) = l else { continue };
        let Some(resets) = Utc
            .timestamp_opt(l.resets_at, 0)
            .single()
            .filter(|r| *r > now)
        else {
            continue;
        };
        w.pct = Some(l.used_percentage.clamp(0.0, 100.0));
        w.resets_at = Some(resets);
        w.cap = None;
        used = true;
    }
    u.scoped = s
        .scoped
        .iter()
        .filter_map(|l| {
            let resets = Utc
                .timestamp_opt(l.resets_at, 0)
                .single()
                .filter(|r| *r > now)?;
            Some(ScopedUsage {
                label: l.label.clone(),
                pct: l.used_percentage.clamp(0.0, 100.0),
                resets_at: resets,
            })
        })
        .collect();
    if used {
        u.cap_source = "official";
        u.official_at = Utc.timestamp_opt(s.captured_at, 0).single();
    }
}

/// `<exe> --statusline` mode: store the snapshot for the profile Claude Code runs under, print the bar.
pub fn handle(args: &[String]) -> bool {
    if args.len() != 2 || args[1] != FLAG {
        return false;
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")));
    let previous = dir.as_deref().and_then(read);
    let mut fresh = from_status(&input, Utc::now());
    if let (Some(dir), Some(s)) = (&dir, &mut fresh) {
        // The status JSON has no per-model rows: keep the ones from the last full readout.
        s.scoped = previous
            .as_ref()
            .map(|p| p.scoped.clone())
            .unwrap_or_default();
        write(dir, s);
    }
    let shown = fresh.or(previous);
    println!("{}", render(shown.as_ref()));
    true
}

fn command_for(exe: &Path) -> String {
    format!(
        "\"{}\" {FLAG}",
        exe.display().to_string().replace('\\', "/")
    )
}

/// Registers this exe as the profile's status line. Converged → no write. A status line configured by
/// someone else is left untouched (returns `Ok(false)`), and so is a settings file that does not parse.
pub fn ensure_installed(config_dir: &Path, exe: &Path) -> AppResult<bool> {
    let path = config_dir.join("settings.json");
    let mut v: Value = match fs::read_to_string(&path) {
        Ok(t) if t.trim().is_empty() => json!({}),
        Ok(t) => match serde_json::from_str(&t) {
            Ok(v @ Value::Object(_)) => v,
            _ => return Ok(false),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.into()),
    };
    let desired = json!({ "type": "command", "command": command_for(exe), "padding": 0 });
    match v.get("statusLine") {
        Some(cur) if cur == &desired => return Ok(false),
        Some(cur)
            if !cur
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|c| c.ends_with(FLAG)) =>
        {
            return Ok(false);
        }
        _ => {}
    }
    v["statusLine"] = desired;
    fs::create_dir_all(config_dir)?;
    fs::write(&path, serde_json::to_string_pretty(&v)? + "\n")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::summarize;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    const STATUS: &str = r#"{"model":{"id":"claude-sonnet-5-5"},"rate_limits":{
        "five_hour":{"used_percentage":1.2,"resets_at":2000},
        "seven_day":{"used_percentage":100,"resets_at":900000}}}"#;

    #[test]
    fn parses_rate_limits_and_renders() {
        let s = from_status(STATUS, t(1000)).unwrap();
        assert_eq!(
            s.five_hour,
            Some(Limit {
                used_percentage: 1.2,
                resets_at: 2000
            })
        );
        assert_eq!(render(Some(&s)), "5 h 1 % · 7 j 100 %");
        assert_eq!(from_status(r#"{"model":{}}"#, t(0)), None);
        assert_eq!(from_status("not json", t(0)), None);
    }

    #[test]
    fn official_values_override_until_reset() {
        let s = from_status(STATUS, t(1000)).unwrap();
        let mut u = summarize("a", &[], t(1500), None, None);
        apply(&mut u, &s, t(1500));
        assert_eq!(u.cap_source, "official");
        assert_eq!(u.five_hour.pct, Some(1.2));
        assert_eq!(u.weekly.pct, Some(100.0));
        // After the 5 h reset only the weekly value is still official.
        let mut later = summarize("a", &[], t(3000), None, None);
        apply(&mut later, &s, t(3000));
        assert_eq!(later.five_hour.pct, None);
        assert_eq!(later.weekly.pct, Some(100.0));
    }

    #[test]
    fn installs_idempotently_and_respects_foreign_status_lines() {
        let tmp = std::env::temp_dir().join(format!("cam-statusline-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let exe = Path::new("C:\\Apps\\Claude Accounts\\cam.exe");
        fs::write(
            tmp.join("settings.json"),
            r#"{"model":"opus","env":{"A":"1"}}"#,
        )
        .unwrap();
        assert!(ensure_installed(&tmp, exe).unwrap());
        assert!(!ensure_installed(&tmp, exe).unwrap());
        let v: Value =
            serde_json::from_str(&fs::read_to_string(tmp.join("settings.json")).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(
            v["statusLine"]["command"],
            "\"C:/Apps/Claude Accounts/cam.exe\" --statusline"
        );
        // Moved install: our own entry is updated.
        assert!(ensure_installed(&tmp, Path::new("D:/cam.exe")).unwrap());
        // Someone else's status line stays.
        fs::write(
            tmp.join("settings.json"),
            r#"{"statusLine":{"type":"command","command":"mine.sh"}}"#,
        )
        .unwrap();
        assert!(!ensure_installed(&tmp, exe).unwrap());
        fs::write(tmp.join("settings.json"), "{ broken").unwrap();
        assert!(!ensure_installed(&tmp, exe).unwrap());
        let _ = fs::remove_dir_all(&tmp);
    }
}
