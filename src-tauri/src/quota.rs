//! Periodic official quota readout for every logged-in profile.
//!
//! Asks Claude Code itself through its SDK control protocol (`get_usage`, the data behind `/usage`):
//! no prompt is sent to a model, so nothing is consumed, and this app never reads the login token.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::statusline::{self, Limit, ScopedLimit, Snapshot};

pub const INTERVAL: Duration = Duration::from_secs(180);
const TIMEOUT: Duration = Duration::from_secs(90);
const REQUEST_ID: &str = "cam-usage";

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
pub fn parse_response(line: &str, now: DateTime<Utc>) -> Option<Snapshot> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let r = v
        .get("response")
        .filter(|r| r.get("request_id").and_then(|i| i.as_str()) == Some(REQUEST_ID))?;
    let rows: Vec<Row> =
        serde_json::from_value(r.pointer("/response/rate_limits/limits")?.clone()).ok()?;
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
    Some(s)
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
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{request}");
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
    let _ = child.kill();
    let _ = child.wait();
    match result {
        Ok(Some(s)) => Ok(s),
        Ok(None) => Err("Claude Code n'a pas renvoyé de quotas (compte non connecté ?)".into()),
        Err(_) => Err("Claude Code n'a pas répondu à temps".into()),
    }
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
                        statusline::write(dir, &s);
                        None
                    }
                    Err(e) => Some((dir.clone(), e)),
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
        let s = parse_response(RESPONSE, Utc::now()).unwrap();
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
    fn ignores_other_lines() {
        assert!(parse_response(r#"{"type":"system","subtype":"init"}"#, Utc::now()).is_none());
        assert!(parse_response(&RESPONSE.replace("cam-usage", "other"), Utc::now()).is_none());
        assert!(parse_response("garbage", Utc::now()).is_none());
    }
}
