//! Periodic official quota readout for every logged-in profile.
//!
//! Asks Claude Code itself through its SDK control protocol (`get_usage`, the data behind `/usage`):
//! no prompt is sent to a model, so nothing is consumed, and this app never reads the login token.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::statusline::{self, Limit, ScopedLimit, Snapshot};

pub const INTERVAL: Duration = Duration::from_secs(180);
const TIMEOUT: Duration = Duration::from_secs(90);
const REQUEST_ID: &str = "cam-usage";
/// Grace period for Claude Code to exit by itself once stdin is closed (lets it finish writing its config).
const EXIT_GRACE: Duration = Duration::from_secs(5);
pub const EXPIRED_MSG: &str =
    "session Claude Code expirée : ouvrez Claude Code dans ce compte et tapez /login";

/// Outcome of one `get_usage` control response.
#[derive(Debug)]
pub enum Readout {
    Usage(Snapshot),
    /// Claude Code answered but has no subscription for this profile: credentials expired or revoked.
    LoggedOut,
}

#[derive(Deserialize)]
struct Row {
    kind: String,
    percent: f64,
    resets_at: Option<String>,
    scope: Option<Scope>,
}

#[derive(Deserialize)]
struct Scope {
    model: Option<Named>,
    surface: Option<Named>,
}

#[derive(Deserialize)]
struct Named {
    display_name: String,
}

fn epoch(iso: &Option<String>) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso.as_deref()?)
        .ok()
        .map(|d| d.timestamp())
}

/// Reads the `get_usage` control response (one stdout line of the stream-json protocol).
pub fn parse_response(line: &str, now: DateTime<Utc>) -> Option<Readout> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let r = v
        .get("response")
        .filter(|r| r.get("request_id").and_then(|i| i.as_str()) == Some(REQUEST_ID))?;
    let Some(limits) = r.pointer("/response/rate_limits/limits") else {
        let body = r.get("response");
        let no_sub = body
            .and_then(|b| b.get("subscription_type"))
            .is_none_or(|t| t.is_null());
        let unavailable = body
            .and_then(|b| b.get("rate_limits_available"))
            .and_then(|a| a.as_bool())
            == Some(false);
        return (no_sub || unavailable).then_some(Readout::LoggedOut);
    };
    let rows: Vec<Row> = serde_json::from_value(limits.clone()).ok()?;
    let mut s = Snapshot {
        captured_at: now.timestamp(),
        five_hour: None,
        seven_day: None,
        scoped: Vec::new(),
    };
    for row in rows {
        let resets_at = epoch(&row.resets_at);
        match row.kind.as_str() {
            "session" => {
                s.five_hour = resets_at.map(|r| Limit {
                    used_percentage: row.percent,
                    resets_at: r,
                })
            }
            "weekly_all" => {
                s.seven_day = resets_at.map(|r| Limit {
                    used_percentage: row.percent,
                    resets_at: r,
                })
            }
            _ => {
                let label = row
                    .scope
                    .and_then(|sc| sc.model.or(sc.surface))
                    .map(|n| n.display_name);
                if let (Some(label), Some(resets_at)) = (label, resets_at) {
                    s.scoped.push(ScopedLimit {
                        label,
                        used_percentage: row.percent,
                        resets_at,
                    });
                }
            }
        }
    }
    Some(Readout::Usage(s))
}

/// Closes stdin and gives the CLI a moment to exit cleanly; kills it only past the grace period.
fn finish(mut child: Child, grace: Duration) {
    drop(child.stdin.take());
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// One readout through the CLI, for the profile at `config_dir` (`is_default`: `~/.claude`, no override).
pub fn fetch(cli: &Path, config_dir: &Path, is_default: bool) -> Result<Snapshot, String> {
    let mut cmd = Command::new(cli);
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--strict-mcp-config",
    ])
    .current_dir(std::env::temp_dir())
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    if is_default {
        cmd.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        cmd.env("CLAUDE_CONFIG_DIR", config_dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("lancement de Claude Code impossible : {e}"))?;
    let request = format!(
        r#"{{"type":"control_request","request_id":"{REQUEST_ID}","request":{{"subtype":"get_usage"}}}}"#
    );
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = writeln!(stdin, "{request}");
        let _ = stdin.flush();
    }
    let stdout = child
        .stdout
        .take()
        .ok_or("sortie de Claude Code indisponible")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let found = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .find_map(|l| parse_response(&l, Utc::now()));
        let _ = tx.send(found);
    });
    let result = rx.recv_timeout(TIMEOUT);
    let grace = if result.is_ok() {
        EXIT_GRACE
    } else {
        Duration::ZERO
    };
    finish(child, grace);
    match result {
        Ok(Some(Readout::Usage(s))) => Ok(s),
        Ok(Some(Readout::LoggedOut)) => Err(EXPIRED_MSG.into()),
        Ok(None) => Err("Claude Code s'est arrêté sans renvoyer de quotas".into()),
        Err(_) => Err("Claude Code n'a pas répondu à temps".into()),
    }
}

/// Profiles whose last readout found the session expired (cleared by the next successful readout).
static EXPIRED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

fn mark(dir: &Path, expired: bool) {
    let mut g = EXPIRED.lock().unwrap_or_else(|p| p.into_inner());
    let set = g.get_or_insert_with(HashSet::new);
    if expired {
        set.insert(dir.to_path_buf());
    } else {
        set.remove(dir);
    }
}

/// True when the last readout of this profile reported an expired Claude Code session.
pub fn session_expired(dir: &Path) -> bool {
    EXPIRED
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .is_some_and(|s| s.contains(dir))
}

/// CLI path and the logged-in profiles to read: `(config_dir, is_default)`.
pub type Targets = (PathBuf, Vec<(PathBuf, bool)>);

/// One round at a time: the poller and a manual refresh never run the CLI twice for the same profile.
static ROUND: Mutex<()> = Mutex::new(());

/// Reads every profile in parallel and stores the snapshots. Returns the profiles that failed.
pub fn refresh_all((cli, profiles): &Targets) -> Vec<(PathBuf, String)> {
    let _round = ROUND.lock().unwrap_or_else(|p| p.into_inner());
    std::thread::scope(|scope| {
        let jobs: Vec<_> = profiles
            .iter()
            .map(|(dir, is_default)| (dir, scope.spawn(move || fetch(cli, dir, *is_default))))
            .collect();
        jobs.into_iter()
            .filter_map(|(dir, job)| {
                match job
                    .join()
                    .unwrap_or_else(|_| Err("relevé interrompu".into()))
                {
                    Ok(s) => {
                        mark(dir, false);
                        statusline::write(dir, &s);
                        None
                    }
                    Err(e) => {
                        if e == EXPIRED_MSG {
                            mark(dir, true);
                        }
                        Some((dir.clone(), e))
                    }
                }
            })
            .collect()
    })
}

/// Background loop: every `INTERVAL`, refresh the snapshot of each logged-in profile.
/// `targets` is re-evaluated each round so added or removed accounts are picked up.
pub fn spawn_poller<F>(targets: F)
where
    F: Fn() -> Option<Targets> + Send + 'static,
{
    std::thread::spawn(move || {
        loop {
            if let Some(t) = targets() {
                for (dir, e) in refresh_all(&t) {
                    eprintln!("quota readout failed for {}: {e}", dir.display());
                }
            }
            std::thread::sleep(INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"cam-usage","response":{
        "subscription_type":"max","rate_limits":{"five_hour":{"utilization":30},"limits":[
        {"kind":"session","group":"session","percent":30,"severity":"normal","resets_at":"2026-11-03T10:30:00.001256+00:00","scope":null,"is_active":true},
        {"kind":"weekly_all","group":"weekly","percent":22,"severity":"normal","resets_at":"2026-11-07T10:00:00.001285+00:00","scope":null,"is_active":false},
        {"kind":"weekly_scoped","group":"weekly","percent":23,"severity":"normal","resets_at":"2026-11-07T10:00:00.001485+00:00","scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":false}]}}}}"#;

    #[test]
    fn reads_session_weekly_and_model_rows() {
        let Some(Readout::Usage(s)) = parse_response(RESPONSE, Utc::now()) else {
            panic!("usage expected");
        };
        assert_eq!(s.five_hour.as_ref().unwrap().used_percentage, 30.0);
        assert_eq!(s.five_hour.unwrap().resets_at, 1_793_701_800);
        assert_eq!(s.seven_day.unwrap().used_percentage, 22.0);
        assert_eq!(s.scoped.len(), 1);
        assert_eq!(s.scoped[0].label, "Fable");
        assert_eq!(s.scoped[0].used_percentage, 23.0);
    }

    #[test]
    fn refresh_reports_each_failed_profile() {
        let t: Targets = (
            PathBuf::from("Z:/missing/claude.exe"),
            vec![
                (PathBuf::from("Z:/a"), true),
                (PathBuf::from("Z:/b"), false),
            ],
        );
        let failed = refresh_all(&t);
        assert_eq!(
            failed.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
            vec![PathBuf::from("Z:/a"), PathBuf::from("Z:/b")]
        );
        assert!(refresh_all(&(t.0, Vec::new())).is_empty());
    }

    #[test]
    fn detects_logged_out_profile() {
        let out = r#"{"type":"control_response","response":{"subtype":"success","request_id":"cam-usage","response":{"subscription_type":null,"rate_limits_available":false}}}"#;
        assert!(matches!(
            parse_response(out, Utc::now()),
            Some(Readout::LoggedOut)
        ));
        let sub_only = out.replace("null", "\"pro\"");
        assert!(matches!(
            parse_response(&sub_only, Utc::now()),
            Some(Readout::LoggedOut)
        ));
    }

    #[test]
    fn expired_flag_follows_last_readout() {
        let d = PathBuf::from("Z:/expired-flag-test");
        assert!(!session_expired(&d));
        mark(&d, true);
        assert!(session_expired(&d));
        mark(&d, false);
        assert!(!session_expired(&d));
    }

    #[test]
    fn ignores_other_lines() {
        assert!(parse_response(r#"{"type":"system","subtype":"init"}"#, Utc::now()).is_none());
        assert!(parse_response(&RESPONSE.replace("cam-usage", "other"), Utc::now()).is_none());
        assert!(parse_response("garbage", Utc::now()).is_none());
    }
}
