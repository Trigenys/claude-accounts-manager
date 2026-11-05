//! Claude Code CLI: binary detection, isolated profiles (CLAUDE_CONFIG_DIR) and launching.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::model::Account;
use crate::store::write_if_changed;

/// Items shared with the default profile through directory junctions.
pub const SHARED_DIRS: &[&str] = &["skills", "agents", "hooks", "commands"];
const IDE_EXTENSION_ROOTS: &[&str] = &[".antigravity-ide", ".vscode", ".cursor", ".windsurf"];

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CliInfo {
    pub path: String,
    pub version: Option<String>,
    /// "override" | "path" | "ide-extension" | "local"
    pub source: &'static str,
}

pub fn home() -> AppResult<PathBuf> {
    dirs::home_dir().ok_or_else(|| AppError::new("no_home", "Dossier utilisateur introuvable"))
}

pub fn default_config_dir() -> AppResult<PathBuf> {
    Ok(home()?.join(".claude"))
}

/// Config dir actually used by an account (the default profile resolves to `~/.claude`).
pub fn effective_config_dir(account: &Account) -> AppResult<PathBuf> {
    match &account.config_dir {
        Some(d) => Ok(PathBuf::from(d)),
        None => default_config_dir(),
    }
}

fn parse_version(s: &str) -> Vec<u64> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// Order: configured override, `claude` on PATH, newest IDE-extension binary, `~/.local/bin`.
pub fn detect_cli(override_path: Option<&str>, home: &Path) -> AppResult<CliInfo> {
    if let Some(p) = override_path.filter(|p| Path::new(p).is_file()) {
        return Ok(CliInfo {
            path: p.to_string(),
            version: None,
            source: "override",
        });
    }
    if let Ok(out) = Command::new("where").arg("claude").output() {
        if let Some(first) = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .find(|l| l.ends_with(".exe") || l.ends_with(".cmd"))
        {
            return Ok(CliInfo {
                path: first.to_string(),
                version: None,
                source: "path",
            });
        }
    }
    let mut found: Vec<(Vec<u64>, String, PathBuf)> = Vec::new();
    for root in IDE_EXTENSION_ROOTS {
        let Ok(rd) = fs::read_dir(home.join(root).join("extensions")) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(rest) = name.strip_prefix("anthropic.claude-code-") else {
                continue;
            };
            let exe = e
                .path()
                .join("resources")
                .join("native-binary")
                .join("claude.exe");
            if exe.is_file() {
                let version = rest.split('-').next().unwrap_or(rest).to_string();
                found.push((parse_version(&version), version, exe));
            }
        }
    }
    if let Some((_, version, exe)) = found.into_iter().max_by(|a, b| a.0.cmp(&b.0)) {
        return Ok(CliInfo {
            path: exe.display().to_string(),
            version: Some(version),
            source: "ide-extension",
        });
    }
    let local = home.join(".local").join("bin").join("claude.exe");
    if local.is_file() {
        return Ok(CliInfo {
            path: local.display().to_string(),
            version: None,
            source: "local",
        });
    }
    Err(AppError::new(
        "cli_not_found",
        "Claude Code introuvable. Installez-le ou renseignez son chemin dans les réglages.",
    ))
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileState {
    pub config_dir: String,
    pub is_default: bool,
    pub logged_in: bool,
    /// The last quota readout found the Claude Code session expired: the account must log in again.
    pub session_expired: bool,
    /// Human-readable list of changes made by this call (empty = already converged).
    pub actions: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn logged_in(config_dir: &Path) -> bool {
    config_dir.join(".credentials.json").is_file()
}

/// Idempotently prepares an isolated profile. Never touches the default profile and never
/// replaces an existing real directory or file.
pub fn ensure_profile(config_dir: &Path, default_dir: &Path) -> AppResult<ProfileState> {
    let mut st = ProfileState {
        config_dir: config_dir.display().to_string(),
        ..Default::default()
    };
    if config_dir == default_dir {
        st.is_default = true;
        st.logged_in = logged_in(config_dir);
        return Ok(st);
    }
    if !config_dir.exists() {
        fs::create_dir_all(config_dir)?;
        st.actions
            .push(format!("Dossier créé : {}", config_dir.display()));
    }
    for name in SHARED_DIRS {
        let target = default_dir.join(name);
        let link = config_dir.join(name);
        if !target.is_dir() {
            continue;
        }
        if junction::exists(&link).unwrap_or(false) {
            continue;
        }
        if link.exists() {
            st.warnings.push(format!(
                "{name}/ existe déjà dans le profil : laissé tel quel (non partagé)."
            ));
            continue;
        }
        junction::create(&target, &link)?;
        st.actions
            .push(format!("{name}/ partagé avec le profil principal"));
    }
    let claude_md = config_dir.join("CLAUDE.md");
    let default_md = default_dir.join("CLAUDE.md");
    if !claude_md.exists() && default_md.is_file() {
        let import = format!("@{}\n", default_md.display().to_string().replace('\\', "/"));
        write_if_changed(&claude_md, import.as_bytes(), false)?;
        st.actions
            .push("CLAUDE.md importe les consignes globales".into());
    }
    let settings = config_dir.join("settings.json");
    let default_settings = default_dir.join("settings.json");
    if !settings.exists() && default_settings.is_file() {
        fs::copy(&default_settings, &settings)?;
        st.actions
            .push("settings.json copié depuis le profil principal".into());
    }
    st.logged_in = logged_in(config_dir);
    Ok(st)
}

/// Non-secret account metadata written by Claude Code at login. Token fields are never deserialized.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub plan: Option<crate::model::Plan>,
    pub subscription_created_at: Option<String>,
    /// Next monthly anniversary of the subscription (assumes monthly billing).
    pub next_renewal: Option<String>,
}

#[derive(serde::Deserialize)]
struct CredFile {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<CredMeta>,
}

#[derive(serde::Deserialize)]
struct CredMeta {
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

#[derive(serde::Deserialize)]
struct ConfigFile {
    #[serde(rename = "oauthAccount")]
    account: Option<OauthAccount>,
}

#[derive(serde::Deserialize)]
struct OauthAccount {
    #[serde(rename = "emailAddress")]
    email: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "subscriptionCreatedAt")]
    subscription_created_at: Option<String>,
}

pub fn plan_from(subscription: Option<&str>, tier: Option<&str>) -> Option<crate::model::Plan> {
    use crate::model::Plan;
    let tier = tier.unwrap_or_default();
    match subscription? {
        "pro" => Some(Plan::Pro),
        "max" if tier.contains("20x") => Some(Plan::Max20x),
        "max" => Some(Plan::Max5x),
        _ => Some(Plan::Other),
    }
}

/// Smallest date >= `today` falling on the subscription's day of month (clamped to month end).
pub fn next_monthly_renewal(
    start: chrono::NaiveDate,
    today: chrono::NaiveDate,
) -> chrono::NaiveDate {
    use chrono::{Datelike, NaiveDate};
    let on = |y: i32, m: u32| {
        (1..=start.day())
            .rev()
            .find_map(|d| NaiveDate::from_ymd_opt(y, m, d))
            .expect("valid month")
    };
    let (mut y, mut m) = (today.year(), today.month());
    loop {
        let candidate = on(y, m);
        if candidate >= today && candidate > start {
            return candidate;
        }
        (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    }
}

/// `.claude.json` lives inside CLAUDE_CONFIG_DIR for isolated profiles and in `~` for the default one.
pub fn read_identity(
    config_dir: &Path,
    is_default: bool,
    today: chrono::NaiveDate,
) -> AppResult<Identity> {
    let cred: Option<CredFile> = fs::read_to_string(config_dir.join(".credentials.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let cfg_path = if is_default {
        home()?.join(".claude.json")
    } else {
        config_dir.join(".claude.json")
    };
    let cfg: Option<ConfigFile> = fs::read_to_string(cfg_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let meta = cred.and_then(|c| c.oauth);
    let acct = cfg.and_then(|c| c.account);
    let created = acct
        .as_ref()
        .and_then(|a| a.subscription_created_at.clone());
    let next_renewal = created
        .as_deref()
        .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
        .map(|d| next_monthly_renewal(d.date_naive(), today).to_string());
    Ok(Identity {
        email: acct.as_ref().and_then(|a| a.email.clone()),
        display_name: acct.as_ref().and_then(|a| a.display_name.clone()),
        plan: meta
            .as_ref()
            .and_then(|m| plan_from(m.subscription_type.as_deref(), m.rate_limit_tier.as_deref())),
        subscription_created_at: created,
        next_renewal,
    })
}

/// Completes an account from its login without overriding user input: an empty email or renewal
/// date is filled, a past renewal date rolls forward monthly on its own day. Returns true if changed.
pub fn fill_from_identity(
    account: &mut crate::model::Account,
    id: &Identity,
    today: chrono::NaiveDate,
) -> bool {
    let before = (account.email.clone(), account.renewal_date.clone());
    if account.email.trim().is_empty() {
        if let Some(e) = &id.email {
            account.email = e.clone();
        }
    }
    let stored = account
        .renewal_date
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    account.renewal_date = match stored {
        Some(d) if d < today => Some(next_monthly_renewal(d, today).to_string()),
        Some(_) => account.renewal_date.clone(),
        None => id.next_renewal.clone().or(account.renewal_date.clone()),
    };
    before != (account.email.clone(), account.renewal_date.clone())
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `browser`: executable Claude Code should use to open URLs (set for secondary profiles only, so
/// `/login` opens in a private window instead of the main account's claude.ai session).
pub fn launcher_script(
    config_dir: Option<&Path>,
    cli: &str,
    label: &str,
    browser: Option<&Path>,
    login: bool,
) -> String {
    let mut env = match config_dir {
        Some(d) => format!(
            "$env:CLAUDE_CONFIG_DIR = {}",
            ps_quote(&d.display().to_string())
        ),
        None => "Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue".to_string(),
    };
    if let (Some(_), Some(b)) = (config_dir, browser) {
        env.push_str(&format!(
            "\r\n$env:BROWSER = {}",
            ps_quote(&b.display().to_string())
        ));
    }
    format!(
        "# Generated by Claude Account Manager. Regenerated when the profile changes.\r\n\
         {env}\r\n\
         $Host.UI.RawUI.WindowTitle = {title}\r\n\
         & {cli}{args}\r\n",
        title = ps_quote(&format!("Claude · {label}")),
        cli = ps_quote(cli),
        // Fixed arguments only: nothing user-controlled reaches the command line.
        args = if login { " auth login --claudeai" } else { "" },
    )
}

fn find_on_path(exe: &str) -> bool {
    Command::new("where")
        .arg(exe)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `login`: run `claude auth login` in that profile instead of an interactive session.
pub fn launch_cli(
    account: &Account,
    cli: &CliInfo,
    app_dir: &Path,
    cwd: &Path,
    login: bool,
) -> AppResult<()> {
    let browser = std::env::current_exe().ok();
    let script = launcher_script(
        account.config_dir.as_deref().map(Path::new),
        &cli.path,
        &account.label,
        browser.as_deref(),
        login,
    );
    let suffix = if login { "-login" } else { "" };
    let launcher = app_dir
        .join("launchers")
        .join(format!("{}{suffix}.ps1", account.id));
    write_if_changed(&launcher, script.as_bytes(), false)?;

    let shell = if find_on_path("pwsh") {
        "pwsh"
    } else {
        "powershell"
    };
    let ps_args = [
        "-NoExit",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ];
    let launcher_s = launcher.display().to_string();
    let cwd_s = cwd.display().to_string();

    let spawned = if find_on_path("wt") {
        Command::new("wt")
            .args([
                "-w",
                "new",
                "nt",
                "--title",
                &format!("Claude · {}", account.label),
                "--tabColor",
                &account.color,
                "-d",
                &cwd_s,
                shell,
            ])
            .args(ps_args)
            .arg(&launcher_s)
            .spawn()
    } else {
        Command::new("cmd")
            .args(["/C", "start", "", shell])
            .args(ps_args)
            .arg(&launcher_s)
            .current_dir(cwd)
            .spawn()
    };
    spawned.map(|_| ()).map_err(|e| {
        AppError::new(
            "launch_failed",
            format!("Impossible d'ouvrir le terminal : {e}"),
        )
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistentState {
    pub current: Option<String>,
    pub account_id: Option<String>,
}

#[cfg(windows)]
pub fn read_persistent() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .ok()?
        .get_value::<String, _>("CLAUDE_CONFIG_DIR")
        .ok()
        .filter(|s| !s.is_empty())
}

#[cfg(not(windows))]
pub fn read_persistent() -> Option<String> {
    None
}

/// Sets (or clears, for the default profile) the user-level CLAUDE_CONFIG_DIR.
/// Goes through .NET so Windows broadcasts WM_SETTINGCHANGE. No-op when already converged.
pub fn set_persistent(desired: Option<&str>) -> AppResult<bool> {
    if read_persistent().as_deref() == desired {
        return Ok(false);
    }
    let value = desired.map(ps_quote).unwrap_or_else(|| "$null".into());
    let script =
        format!("[Environment]::SetEnvironmentVariable('CLAUDE_CONFIG_DIR', {value}, 'User')");
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(AppError::new(
            "env_failed",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_profile_converges_and_preserves_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let default = tmp.path().join("default");
        fs::create_dir_all(default.join("skills")).unwrap();
        fs::create_dir_all(default.join("agents")).unwrap();
        fs::write(default.join("CLAUDE.md"), "# rules").unwrap();
        fs::write(default.join("settings.json"), "{}").unwrap();
        let profile = tmp.path().join("p2");
        fs::create_dir_all(profile.join("agents")).unwrap(); // pre-existing real dir

        let first = ensure_profile(&profile, &default).unwrap();
        assert!(first.actions.iter().any(|a| a.starts_with("skills/")));
        assert_eq!(first.warnings.len(), 1);
        assert!(junction::exists(profile.join("skills")).unwrap());
        assert!(!junction::exists(profile.join("agents")).unwrap_or(false));
        assert!(
            fs::read_to_string(profile.join("CLAUDE.md"))
                .unwrap()
                .starts_with('@')
        );

        let second = ensure_profile(&profile, &default).unwrap();
        assert!(
            second.actions.is_empty(),
            "second run must be a no-op: {:?}",
            second.actions
        );
        assert!(!second.logged_in);
    }

    #[test]
    fn default_profile_is_never_modified() {
        let tmp = tempfile::tempdir().unwrap();
        let st = ensure_profile(tmp.path(), tmp.path()).unwrap();
        assert!(st.is_default && st.actions.is_empty());
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn detects_newest_ide_extension() {
        let tmp = tempfile::tempdir().unwrap();
        for v in ["2.1.9", "2.1.316"] {
            let d = tmp.path().join(".vscode/extensions").join(format!(
                "anthropic.claude-code-{v}-win32-x64/resources/native-binary"
            ));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("claude.exe"), "").unwrap();
        }
        let info = detect_cli(Some("Z:/nope.exe"), tmp.path());
        // `where claude` may legitimately win on a dev machine; only assert the extension ordering otherwise.
        if let Ok(i) = info {
            if i.source == "ide-extension" {
                assert_eq!(i.version.as_deref(), Some("2.1.316"));
            }
        }
    }

    #[test]
    fn fills_account_from_login_idempotently() {
        let d = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let mut a: crate::model::Account = serde_json::from_value(serde_json::json!({
            "id": "x", "label": "X", "email": "", "plan": "pro", "price": null, "renewalDate": null,
            "color": "#000", "configDir": "C:/p", "desktopUserDataDir": null, "createdAt": "", "updatedAt": ""
        }))
        .unwrap();
        let id = Identity {
            email: Some("a@b.c".into()),
            next_renewal: Some("2026-11-29".into()),
            ..Default::default()
        };
        let today = d("2026-11-02");
        assert!(fill_from_identity(&mut a, &id, today));
        assert_eq!(
            (a.email.as_str(), a.renewal_date.as_deref()),
            ("a@b.c", Some("2026-11-29"))
        );
        assert!(
            !fill_from_identity(&mut a, &id, today),
            "second pass is a no-op"
        );
        // User date kept; once past, it rolls forward on its own day.
        a.renewal_date = Some("2026-10-15".into());
        assert!(fill_from_identity(&mut a, &id, today));
        assert_eq!(a.renewal_date.as_deref(), Some("2026-11-15"));
    }

    #[test]
    fn maps_plans_and_renewals() {
        use crate::model::Plan;
        use chrono::NaiveDate;
        assert_eq!(
            plan_from(Some("pro"), Some("default_claude_ai")),
            Some(Plan::Pro)
        );
        assert_eq!(
            plan_from(Some("max"), Some("default_claude_max_20x")),
            Some(Plan::Max20x)
        );
        assert_eq!(
            plan_from(Some("max"), Some("default_claude_max_5x")),
            Some(Plan::Max5x)
        );
        assert_eq!(plan_from(None, None), None);
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(
            next_monthly_renewal(d("2026-10-31"), d("2026-11-02")),
            d("2026-11-30")
        );
        assert_eq!(
            next_monthly_renewal(d("2026-10-31"), d("2026-10-31")),
            d("2026-11-30")
        );
        assert_eq!(
            next_monthly_renewal(d("2026-01-15"), d("2026-12-20")),
            d("2027-01-15")
        );
        assert_eq!(
            next_monthly_renewal(d("2026-03-05"), d("2026-11-05")),
            d("2026-11-05")
        );
    }

    #[test]
    fn identity_never_needs_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".credentials.json"), r#"{"claudeAiOauth":{"accessToken":"secret","subscriptionType":"max","rateLimitTier":"default_claude_max_5x"}}"#).unwrap();
        fs::write(tmp.path().join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"a@b.c","subscriptionCreatedAt":"2026-10-31T18:43:22.582458Z"}}"#).unwrap();
        let id = read_identity(
            tmp.path(),
            false,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
        )
        .unwrap();
        assert_eq!(id.email.as_deref(), Some("a@b.c"));
        assert_eq!(id.plan, Some(crate::model::Plan::Max5x));
        assert_eq!(id.next_renewal.as_deref(), Some("2026-11-30"));
        assert!(!format!("{id:?}").contains("secret"));
    }

    #[test]
    fn login_launcher_runs_auth_login_in_the_profile() {
        let s = launcher_script(
            Some(Path::new("C:/p/cli")),
            "C:/x/claude.exe",
            "Hello",
            None,
            true,
        );
        assert!(s.contains("$env:CLAUDE_CONFIG_DIR = 'C:/p/cli'"));
        assert!(s.contains("& 'C:/x/claude.exe' auth login --claudeai\r\n"));
    }

    #[test]
    fn launcher_quotes_paths() {
        let s = launcher_script(
            Some(Path::new("C:/a'b")),
            "C:/x/claude.exe",
            "Perso",
            Some(Path::new("C:/app.exe")),
            false,
        );
        assert!(s.contains("$env:CLAUDE_CONFIG_DIR = 'C:/a''b'"));
        assert!(s.contains("& 'C:/x/claude.exe'\r\n"));
        assert!(s.contains("$env:BROWSER = 'C:/app.exe'"));
        let d = launcher_script(None, "c", "d", Some(Path::new("C:/app.exe")), false);
        assert!(d.contains("Remove-Item Env:CLAUDE_CONFIG_DIR"));
        assert!(
            !d.contains("BROWSER"),
            "default profile keeps the normal browser"
        );
    }
}
