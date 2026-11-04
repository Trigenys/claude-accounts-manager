mod browser;
mod claude;
mod desktop;
mod error;
mod model;
mod phones;
mod quota;
mod recommend;
mod reset_notify;
mod statusline;
mod store;
mod tray;
mod usage;
mod usage_alerts;
mod webpush;

use std::path::PathBuf;

use chrono::Utc;
use tauri::State;

use claude::{CliInfo, PersistentState, ProfileState};
use desktop::DesktopInfo;
use error::{AppError, AppResult};
use model::{Account, AccountInput, Settings, Store};
use recommend::Recommendation;
use usage::{UsageCache, UsageSummary};

struct AppState {
    dir: PathBuf,
    usage: UsageCache,
    lock: std::sync::Mutex<()>,
    /// Running as a child of a packaged app whose %APPDATA% writes are redirected.
    virtualized: bool,
}

impl AppState {
    /// Paths handed to terminals or Claude must be visible outside the package container.
    fn ensure_real_paths(&self) -> AppResult<()> {
        if self.virtualized {
            return Err(AppError::new(
                "virtualized",
                "Trigenys Claude Accounts Manager a été ouvert depuis une app empaquetée (Claude Desktop) : Windows redirige ses fichiers et le terminal ne les voit pas. Ouvrez Trigenys Claude Accounts Manager depuis le menu Démarrer ou l'Explorateur.",
            ));
        }
        Ok(())
    }
}

fn find(store: &Store, id: &str) -> AppResult<Account> {
    store
        .accounts
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .ok_or_else(|| AppError::new("not_found", format!("Compte {id} introuvable")))
}

#[tauri::command]
fn list_accounts(state: State<AppState>) -> AppResult<Vec<Account>> {
    let _g = state.lock.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = store::load(&state.dir)?;
    // Missing or past renewal dates / emails are reconciled from the profile login; save is a no-op when converged.
    let today = Utc::now().date_naive();
    let mut changed = false;
    let exe = statusline_exe(&state);
    for a in s.accounts.iter_mut() {
        let Ok(dir) = claude::effective_config_dir(a) else {
            continue;
        };
        if let Some(exe) = &exe {
            if let Err(e) = statusline::ensure_installed(&dir, exe) {
                eprintln!(
                    "status line not installed in {}: {}",
                    dir.display(),
                    e.message
                );
            }
        }
        if let Ok(id) = claude::read_identity(&dir, a.config_dir.is_none(), today) {
            changed |= claude::fill_from_identity(a, &id, today);
        }
    }
    if changed && !state.virtualized {
        store::save(&state.dir, &s)?;
    }
    Ok(s.accounts)
}

/// Exe to register as Claude Code status line: only a real install (not a dev build that may vanish,
/// nor a sandboxed copy whose writes Claude Code would not see).
fn statusline_exe(state: &AppState) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let in_build_dir = exe.components().any(|c| c.as_os_str() == "target");
    (!state.virtualized && !in_build_dir).then_some(exe)
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> AppResult<Settings> {
    Ok(store::load(&state.dir)?.settings)
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: Settings) -> AppResult<Settings> {
    let _g = state.lock.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = store::load(&state.dir)?;
    s.settings = settings;
    store::save(&state.dir, &s)?;
    Ok(s.settings)
}

#[tauri::command]
fn upsert_account(state: State<AppState>, input: AccountInput) -> AppResult<Account> {
    let _g = state.lock.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = store::load(&state.dir)?;
    let account = store::upsert(
        &mut s,
        input,
        &state.dir.join("profiles"),
        &Utc::now().to_rfc3339(),
    )?;
    store::save(&state.dir, &s)?;
    Ok(account)
}

/// Removes the account entry only; profile folders (logins, history) are kept on disk.
#[tauri::command]
fn delete_account(state: State<AppState>, id: String) -> AppResult<()> {
    let _g = state.lock.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = store::load(&state.dir)?;
    s.accounts.retain(|a| a.id != id);
    store::save(&state.dir, &s)?;
    Ok(())
}

#[tauri::command]
fn reorder_accounts(state: State<AppState>, ids: Vec<String>) -> AppResult<()> {
    let _g = state.lock.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = store::load(&state.dir)?;
    s.accounts
        .sort_by_key(|a| ids.iter().position(|i| *i == a.id).unwrap_or(usize::MAX));
    store::save(&state.dir, &s)?;
    Ok(())
}

#[tauri::command]
fn detect_cli(state: State<AppState>) -> AppResult<CliInfo> {
    let s = store::load(&state.dir)?;
    claude::detect_cli(s.settings.cli_path_override.as_deref(), &claude::home()?)
}

#[tauri::command]
fn ensure_cli_profile(state: State<AppState>, id: String) -> AppResult<ProfileState> {
    state.ensure_real_paths()?;
    let a = find(&store::load(&state.dir)?, &id)?;
    claude::ensure_profile(
        &claude::effective_config_dir(&a)?,
        &claude::default_config_dir()?,
    )
}

#[tauri::command]
fn launch_cli(state: State<AppState>, id: String, cwd: Option<String>) -> AppResult<()> {
    state.ensure_real_paths()?;
    let s = store::load(&state.dir)?;
    let a = find(&s, &id)?;
    claude::ensure_profile(
        &claude::effective_config_dir(&a)?,
        &claude::default_config_dir()?,
    )?;
    let cli = claude::detect_cli(s.settings.cli_path_override.as_deref(), &claude::home()?)?;
    let cwd = cwd.map(PathBuf::from).unwrap_or(claude::home()?);
    claude::launch_cli(&a, &cli, &state.dir, &cwd)
}

#[tauri::command]
fn get_persistent_profile(state: State<AppState>) -> AppResult<PersistentState> {
    let current = claude::read_persistent();
    let s = store::load(&state.dir)?;
    let account_id = s
        .accounts
        .iter()
        .find(|a| a.config_dir == current)
        .map(|a| a.id.clone());
    Ok(PersistentState {
        current,
        account_id,
    })
}

#[tauri::command]
fn set_persistent_profile(state: State<AppState>, id: String) -> AppResult<PersistentState> {
    state.ensure_real_paths()?;
    let a = find(&store::load(&state.dir)?, &id)?;
    if let Some(dir) = &a.config_dir {
        claude::ensure_profile(&PathBuf::from(dir), &claude::default_config_dir()?)?;
    }
    claude::set_persistent(a.config_dir.as_deref())?;
    get_persistent_profile(state)
}

#[tauri::command]
fn locate_desktop() -> AppResult<DesktopInfo> {
    desktop::locate()
}

#[tauri::command]
fn launch_desktop(state: State<AppState>, id: String) -> AppResult<()> {
    state.ensure_real_paths()?;
    desktop::launch(&find(&store::load(&state.dir)?, &id)?)
}

fn usage_for(state: &AppState, accounts: &[Account]) -> AppResult<Vec<UsageSummary>> {
    let now = Utc::now();
    accounts
        .iter()
        .map(|a| {
            let dir = claude::effective_config_dir(a)?;
            let entries = state.usage.entries(&dir);
            let mut u = usage::summarize(
                &a.id,
                &entries,
                now,
                a.five_hour_token_cap,
                a.weekly_token_cap,
            );
            if let Some(snap) = statusline::read(&dir) {
                statusline::apply(&mut u, &snap, now);
            }
            Ok(u)
        })
        .collect()
}

#[tauri::command]
async fn get_usage(state: State<'_, AppState>) -> AppResult<Vec<UsageSummary>> {
    let s = store::load(&state.dir)?;
    usage_for(&state, &s.accounts)
}

#[tauri::command]
async fn recommend(state: State<'_, AppState>) -> AppResult<Recommendation> {
    let s = store::load(&state.dir)?;
    let usage = usage_for(&state, &s.accounts)?;
    Ok(recommend::recommend(
        &s.accounts,
        &usage,
        &s.settings.recommend,
    ))
}

#[tauri::command]
fn profile_status(state: State<AppState>, id: String) -> AppResult<ProfileState> {
    let a = find(&store::load(&state.dir)?, &id)?;
    let dir = claude::effective_config_dir(&a)?;
    Ok(ProfileState {
        config_dir: dir.display().to_string(),
        is_default: a.config_dir.is_none(),
        logged_in: claude::logged_in(&dir),
        ..Default::default()
    })
}

/// Reads plan/email/subscription metadata from the profile. Pass `id: None` for a profile being created as default.
#[tauri::command]
fn detect_account_info(state: State<AppState>, id: Option<String>) -> AppResult<claude::Identity> {
    let (dir, is_default) = match id {
        Some(id) => {
            let a = find(&store::load(&state.dir)?, &id)?;
            (claude::effective_config_dir(&a)?, a.config_dir.is_none())
        }
        None => (claude::default_config_dir()?, true),
    };
    claude::read_identity(&dir, is_default, Utc::now().date_naive())
}

/// CLI and logged-in profiles for a quota readout, plus each profile's account label.
fn quota_targets(dir: &std::path::Path) -> AppResult<(quota::Targets, Vec<(PathBuf, String)>)> {
    let s = store::load(dir)?;
    let cli = claude::detect_cli(s.settings.cli_path_override.as_deref(), &claude::home()?)?;
    let mut profiles = Vec::new();
    let mut labels = Vec::new();
    for a in &s.accounts {
        let Ok(d) = claude::effective_config_dir(a) else {
            continue;
        };
        if claude::logged_in(&d) {
            labels.push((d.clone(), a.label.clone()));
            profiles.push((d, a.config_dir.is_none()));
        }
    }
    Ok(((PathBuf::from(cli.path), profiles), labels))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshReport {
    refreshed: usize,
    /// "<account>: <reason>" for each profile that could not be read.
    failed: Vec<String>,
}

/// Reads the official quotas of every logged-in account now, instead of waiting for the next round.
#[tauri::command]
async fn refresh_quotas(state: State<'_, AppState>) -> AppResult<RefreshReport> {
    let (targets, labels) = quota_targets(&state.dir)?;
    let total = targets.1.len();
    let failed = tauri::async_runtime::spawn_blocking(move || quota::refresh_all(&targets))
        .await
        .map_err(|e| AppError::new("refresh", e.to_string()))?;
    let label = |d: &PathBuf| {
        labels
            .iter()
            .find(|(p, _)| p == d)
            .map_or_else(|| d.display().to_string(), |(_, l)| l.clone())
    };
    Ok(RefreshReport {
        refreshed: total - failed.len(),
        failed: failed
            .iter()
            .map(|(d, e)| format!("{} : {e}", label(d)))
            .collect(),
    })
}

/// Shows a sample reset notification so the user can check that it appears and plays its sound.
#[tauri::command]
fn test_notification(app: tauri::AppHandle) -> AppResult<()> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title("Test · Session de 5 h")
        .body("Voici l'alerte que vous recevrez à chaque réinitialisation de quota.")
        .sound("Reminder")
        .show()
        .map_err(|e| AppError::new("notification", e.to_string()))
}

/// Live windows of every account plus each account's readout time, as pushed to phones.
fn phone_accounts(s: &Store, now: i64) -> Vec<phones::PhoneAccount> {
    let mut windows = Vec::new();
    let mut captured = Vec::new();
    for a in &s.accounts {
        if let Some(snap) = claude::effective_config_dir(a)
            .ok()
            .and_then(|d| statusline::read(&d))
        {
            windows.extend(usage_alerts::windows(&a.id, &a.label, &snap, now));
            captured.push((a.id.clone(), snap.captured_at));
        }
    }
    phones::accounts(&windows, &captured)
}

/// VAPID keys and relay client for the configured relay (HTTPS only).
fn phone_ctx(dir: &std::path::Path) -> AppResult<(webpush::Vapid, phones::HttpRelay)> {
    let s = store::load(dir)?;
    let base = s
        .settings
        .phones
        .relay_url
        .trim_end_matches('/')
        .to_string();
    if !base.starts_with("https://") {
        return Err(AppError::new(
            "phones",
            "L'adresse du relais doit être en https://",
        ));
    }
    let vapid = webpush::Vapid::load_or_create(dir, &base).map_err(phone_err)?;
    Ok((
        vapid,
        phones::HttpRelay {
            base,
            agent: webpush::agent(),
        },
    ))
}

fn phone_err(e: impl std::fmt::Display) -> AppError {
    AppError::new("phones", e.to_string())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(phone_err)?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Pairing {
    code: String,
    url: String,
    qr_svg: String,
    expires_at: i64,
}

/// Opens a pairing mailbox on the relay and returns the QR code the phone scans.
#[tauri::command]
async fn phone_pair_start(state: State<'_, AppState>) -> AppResult<Pairing> {
    let dir = state.dir.clone();
    blocking(move || {
        use phones::Relay;
        let (vapid, relay) = phone_ctx(&dir)?;
        let code = phones::new_code();
        relay.offer(&code, &vapid.public_key()).map_err(phone_err)?;
        let url = format!("{}/app/#pair={code}", relay.base);
        Ok(Pairing {
            qr_svg: phones::qr_svg(&url).map_err(phone_err)?,
            url,
            code,
            expires_at: Utc::now().timestamp() + phones::PAIR_TTL_SECS,
        })
    })
    .await
}

/// Collects the phone's subscription once it has answered; `None` while it has not.
#[tauri::command]
async fn phone_pair_poll(
    state: State<'_, AppState>,
    code: String,
) -> AppResult<Option<phones::DeviceView>> {
    let dir = state.dir.clone();
    blocking(move || {
        use phones::Relay;
        let (vapid, relay) = phone_ctx(&dir)?;
        let Some(answer) = relay.take(&code).map_err(phone_err)? else {
            return Ok(None);
        };
        let now = Utc::now().timestamp();
        let id = phones::update(&dir, |d| match phones::add(d, answer, now) {
            Ok(id) => (true, Ok(id)),
            Err(e) => (false, Err(e)),
        })
        .and_then(|r| r)
        .map_err(phone_err)?;
        let device = phones::load(&dir).into_iter().find(|d| d.id == id);
        if let Some(d) = &device {
            let accounts = phone_accounts(&store::load(&dir)?, now);
            let pusher = webpush::HttpPusher(webpush::agent());
            let msg = phones::paired_message(now, &accounts);
            let out = phones::push_now(&vapid, &pusher, d, &msg, now, 3600, "normal");
            if out != webpush::Outcome::Delivered {
                eprintln!("paired push not delivered: {out:?}");
            }
        }
        Ok(device.map(|d| d.view()))
    })
    .await
}

#[tauri::command]
fn phone_list(state: State<AppState>) -> Vec<phones::DeviceView> {
    phones::load(&state.dir).iter().map(|d| d.view()).collect()
}

/// Turns a phone on or off; the watcher empties or rebuilds its relay schedule on the next round.
#[tauri::command]
fn phone_set_enabled(
    state: State<AppState>,
    id: String,
    enabled: bool,
) -> AppResult<Vec<phones::DeviceView>> {
    phones::update(&state.dir, |d| {
        let mut changed = false;
        for x in d.iter_mut().filter(|x| x.id == id && x.enabled != enabled) {
            x.enabled = enabled;
            changed = true;
        }
        (changed, d.iter().map(|x| x.view()).collect())
    })
    .map_err(phone_err)
}

fn remove_phone(dir: &std::path::Path, id: &str) -> AppResult<Vec<phones::DeviceView>> {
    phones::update(dir, |d| {
        let before = d.len();
        d.retain(|x| x.id != id);
        (before != d.len(), d.iter().map(|x| x.view()).collect())
    })
    .map_err(phone_err)
}

/// Forgets a phone here and on the relay (its pending reset pushes are dropped).
#[tauri::command]
async fn phone_revoke(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Vec<phones::DeviceView>> {
    let dir = state.dir.clone();
    blocking(move || {
        use phones::Relay;
        if let Some(d) = phones::load(&dir).into_iter().find(|d| d.id == id) {
            let (_, relay) = phone_ctx(&dir)?;
            if let Err(e) = relay.forget(&d) {
                eprintln!("relay forget failed: {e}");
            }
        }
        remove_phone(&dir, &id)
    })
    .await
}

/// Sends a test push to one phone now; a 404/410 answer prunes it on both sides.
#[tauri::command]
async fn phone_test(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let dir = state.dir.clone();
    blocking(move || {
        use phones::Relay;
        let d = phones::load(&dir)
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| AppError::new("not_found", "Téléphone introuvable"))?;
        let (vapid, relay) = phone_ctx(&dir)?;
        let now = Utc::now().timestamp();
        let accounts = phone_accounts(&store::load(&dir)?, now);
        let pusher = webpush::HttpPusher(webpush::agent());
        let msg = phones::test_message(now, &accounts);
        match phones::push_now(&vapid, &pusher, &d, &msg, now, 600, "high") {
            webpush::Outcome::Delivered => Ok(()),
            webpush::Outcome::Gone => {
                let _ = relay.forget(&d);
                remove_phone(&dir, &id)?;
                Err(AppError::new(
                    "phones",
                    "Ce téléphone n'est plus abonné : il a été retiré. Associez-le de nouveau.",
                ))
            }
            webpush::Outcome::Failed(e) => Err(phone_err(e)),
        }
    })
    .await
}

/// Watches every account's latest readout: notifies resets (with sound) and usage alerts (thresholds, pace).
fn spawn_reset_watcher(app: tauri::AppHandle, dir: PathBuf) {
    use tauri_plugin_notification::NotificationExt;
    std::thread::spawn(move || {
        let mut tracker = reset_notify::Tracker::load(&dir);
        let mut alerts = usage_alerts::Tracker::load(&dir);
        let mut hub = phones::Hub::default();
        let pusher = webpush::HttpPusher(webpush::agent());
        let notify = |title: &str, body: &str, sound: bool| {
            let mut b = app.notification().builder().title(title).body(body);
            if sound {
                b = b.sound("Reminder");
            }
            if let Err(e) = b.show() {
                eprintln!("notification failed: {e}");
            }
        };
        loop {
            if let Ok(s) = store::load(&dir) {
                let now = Utc::now().timestamp();
                let mut changed = tracker
                    .retain_accounts(&s.accounts.iter().map(|a| a.id.clone()).collect::<Vec<_>>());
                let mut windows = Vec::new();
                let mut captured = Vec::new();
                for a in &s.accounts {
                    if let Some(snap) = claude::effective_config_dir(a)
                        .ok()
                        .and_then(|d| statusline::read(&d))
                    {
                        changed |= tracker.observe(&a.id, &a.label, &snap, now);
                        windows.extend(usage_alerts::windows(&a.id, &a.label, &snap, now));
                        captured.push((a.id.clone(), snap.captured_at));
                    }
                }
                let notices = tracker.due(now);
                for n in &notices {
                    notify(&n.title, &n.body, true);
                }
                if changed || !notices.is_empty() {
                    tracker.save(&dir);
                }
                let (fired, alerts_changed) = alerts.check(&windows, &s.settings.alerts, now);
                for a in &fired {
                    notify(&a.title, &a.body, a.sound);
                }
                if alerts_changed {
                    alerts.save(&dir);
                }
                // Phones: alerts and status now; resets go through the relay schedule (the relay owns them).
                let devices = phones::load(&dir);
                if !devices.is_empty() {
                    match phone_ctx(&dir) {
                        Ok((vapid, relay)) => {
                            let accounts = phones::accounts(&windows, &captured);
                            let resets = tracker.upcoming(now);
                            let tick = phones::Tick {
                                now,
                                accounts: &accounts,
                                resets: &resets,
                                alerts: &fired,
                            };
                            let c = hub.tick(&vapid, &devices, &tick, &pusher, &relay);
                            if !c.is_empty() {
                                if let Err(e) = phones::update(&dir, |d| (phones::apply(d, &c), ()))
                                {
                                    eprintln!("phones not saved: {e}");
                                }
                            }
                        }
                        Err(e) => eprintln!("phones disabled: {}", e.message),
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(20));
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // BROWSER handler mode for secondary profiles: open the URL privately and exit, no UI.
    let args: Vec<String> = std::env::args().collect();
    if statusline::handle(&args) {
        return;
    }
    if let Some(url) = browser::url_from_args(&args) {
        if let Err(e) = browser::open_private(url) {
            eprintln!("cannot open browser: {e}");
            std::process::exit(1);
        }
        return;
    }
    let dir = store::app_dir().expect("APPDATA introuvable");
    let virtualized = store::is_virtualized(&dir);
    if !virtualized {
        if let Err(e) = store::import_from(&dir, &store::package_copies(&dir)) {
            eprintln!("import of redirected data failed: {}", e.message);
        }
    }
    let poll_dir = dir.clone();
    quota::spawn_poller(move || quota_targets(&poll_dir).ok().map(|(t, _)| t));
    tauri::Builder::default()
        // First: a second launch only brings the running window back (one poller, one tray icon).
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            tray::show(app)
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup({
            let dir = dir.clone();
            move |app| {
                spawn_reset_watcher(app.handle().clone(), dir);
                tray::install(app)?;
                Ok(())
            }
        })
        .on_window_event(tray::on_window_event)
        .manage(AppState {
            dir,
            usage: UsageCache::default(),
            lock: Default::default(),
            virtualized,
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            get_settings,
            save_settings,
            upsert_account,
            delete_account,
            reorder_accounts,
            detect_cli,
            ensure_cli_profile,
            launch_cli,
            get_persistent_profile,
            set_persistent_profile,
            locate_desktop,
            launch_desktop,
            get_usage,
            recommend,
            profile_status,
            detect_account_info,
            refresh_quotas,
            test_notification,
            phone_pair_start,
            phone_pair_poll,
            phone_list,
            phone_set_enabled,
            phone_revoke,
            phone_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
