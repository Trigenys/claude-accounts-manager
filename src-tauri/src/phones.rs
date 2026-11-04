//! Paired phones: pairing through the relay mailbox, direct pushes (alerts, status, tests) and the
//! reset schedule kept on the relay so resets still reach the phone when this PC is off.
//!
//! Deduplication rule: the relay owns reset pushes to phones. The desktop never pushes a reset
//! itself; it only replaces each phone's schedule when the upcoming resets change.
//! The relay only ever receives the subscription, reset times and opaque encrypted payloads.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::reset_notify::{self, Pending};
use crate::usage_alerts::{Alert, Window};
use crate::webpush::{self, Outcome, Prepared, Pusher, Subscription, Vapid};

const FILE: &str = "push-devices.json";
static LOCK: Mutex<()> = Mutex::new(());

pub const PAIR_TTL_SECS: i64 = 10 * 60;
/// A reset push may still be useful this long after the reset.
const RESET_TTL_SECS: i64 = 6 * 3600;
const MAX_SCHEDULE: usize = 32;
/// Status pushes (visible but silent): only on a meaningful change, at most this often.
const STATUS_MIN_GAP_SECS: i64 = 15 * 60;
const STATUS_BUCKET_PCT: f64 = 5.0;
/// Encrypted record max is 4096 bytes: keep the JSON well below.
const MAX_PLAINTEXT: usize = 3500;
const RETRY_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    pub at: i64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub label: String,
    pub subscription: Subscription,
    pub enabled: bool,
    pub paired_at: i64,
    /// Proves ownership of this device's schedule on the relay (stored there as a hash only).
    pub schedule_key: String,
    /// Fingerprint of the schedule last accepted by the relay.
    #[serde(default)]
    pub synced: Option<String>,
    #[serde(default)]
    pub status: Option<Sent>,
}

/// What the UI sees: no keys, no endpoint.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub paired_at: i64,
    pub service: String,
}

impl Device {
    pub fn view(&self) -> DeviceView {
        DeviceView {
            id: self.id.clone(),
            label: self.label.clone(),
            enabled: self.enabled,
            paired_at: self.paired_at,
            service: self
                .subscription
                .origin()
                .trim_start_matches("https://")
                .into(),
        }
    }
}

pub fn load(dir: &Path) -> Vec<Device> {
    fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Load, change and save under one lock; saved only if `f` reports a change.
pub fn update<T>(dir: &Path, f: impl FnOnce(&mut Vec<Device>) -> (bool, T)) -> Result<T, String> {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut d = load(dir);
    let (changed, out) = f(&mut d);
    if changed {
        let b = serde_json::to_vec_pretty(&d).map_err(|e| e.to_string())?;
        fs::write(dir.join(FILE), b).map_err(|e| format!("appareils non enregistrés : {e}"))?;
    }
    Ok(out)
}

// ---------- payloads ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhoneWindow {
    pub name: String,
    pub pct: f64,
    pub resets_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhoneAccount {
    pub id: String,
    pub label: String,
    pub captured_at: i64,
    pub windows: Vec<PhoneWindow>,
}

/// Groups the live windows by account (order kept); `captured` gives each account's readout time.
pub fn accounts(windows: &[Window], captured: &[(String, i64)]) -> Vec<PhoneAccount> {
    let mut out: Vec<PhoneAccount> = Vec::new();
    for (id, at) in captured {
        let ws: Vec<_> = windows.iter().filter(|w| &w.account_id == id).collect();
        let Some(first) = ws.first() else { continue };
        out.push(PhoneAccount {
            id: id.clone(),
            label: first.account.clone(),
            captured_at: *at,
            windows: ws
                .iter()
                .map(|w| PhoneWindow {
                    name: w.name.clone(),
                    pct: (w.pct * 10.0).round() / 10.0,
                    resets_at: w.resets_at,
                })
                .collect(),
        });
    }
    out
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    v: u8,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    sound: bool,
    at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset: Option<ResetInfo<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accounts: Option<Vec<PhoneAccount>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetInfo<'a> {
    key: &'a str,
    window: &'a str,
    resets_at: i64,
}

/// JSON payload; accounts are dropped from the end until it fits in one push record.
fn encode(mut p: Payload) -> Vec<u8> {
    loop {
        let b = serde_json::to_vec(&p).unwrap_or_default();
        match &mut p.accounts {
            Some(a) if b.len() > MAX_PLAINTEXT && !a.is_empty() => {
                a.pop();
            }
            _ => return b,
        }
    }
}

fn message(
    kind: &str,
    title: &str,
    body: &str,
    sound: bool,
    now: i64,
    accounts: &[PhoneAccount],
) -> Vec<u8> {
    encode(Payload {
        v: 1,
        kind,
        title: Some(title),
        body: Some(body),
        tag: None,
        sound,
        at: now,
        reset: None,
        accounts: Some(accounts.to_vec()),
    })
}

fn status_fingerprint(accounts: &[PhoneAccount]) -> String {
    accounts
        .iter()
        .flat_map(|a| {
            a.windows.iter().map(move |w| {
                format!(
                    "{}/{}:{}:{}",
                    a.id,
                    w.name,
                    (w.pct / STATUS_BUCKET_PCT).floor(),
                    w.resets_at / 60
                )
            })
        })
        .collect::<Vec<_>>()
        .join(";")
}

// ---------- relay ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Answer {
    pub subscription: Subscription,
    #[serde(default)]
    pub label: String,
}

/// One scheduled push held by the relay until `at`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Entry {
    pub at: i64,
    pub expires: i64,
    #[serde(flatten)]
    pub push: Prepared,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stored {
    Ok,
    /// The relay pruned this device (its subscription answered 404/410).
    Gone,
}

pub trait Relay {
    fn offer(&self, code: &str, vapid_key: &str) -> Result<(), String>;
    /// `Ok(None)` while the phone has not answered yet.
    fn take(&self, code: &str) -> Result<Option<Answer>, String>;
    fn put_schedule(&self, d: &Device, entries: &[Entry]) -> Result<Stored, String>;
    fn forget(&self, d: &Device) -> Result<(), String>;
}

pub struct HttpRelay {
    pub base: String,
    pub agent: ureq::Agent,
}

fn http_err(what: &str, e: ureq::Error) -> String {
    format!("relais injoignable ({what}) : {e}")
}

impl Relay for HttpRelay {
    fn offer(&self, code: &str, vapid_key: &str) -> Result<(), String> {
        let r = self
            .agent
            .post(format!("{}/api/pair", self.base))
            .send_json(serde_json::json!({ "code": code, "vapidKey": vapid_key }))
            .map_err(|e| http_err("association", e))?;
        match r.status().as_u16() {
            201 => Ok(()),
            s => Err(format!("relais : association refusée (HTTP {s})")),
        }
    }

    fn take(&self, code: &str) -> Result<Option<Answer>, String> {
        let mut r = self
            .agent
            .post(format!("{}/api/pair/{code}/take", self.base))
            .send_empty()
            .map_err(|e| http_err("association", e))?;
        match r.status().as_u16() {
            200 => r
                .body_mut()
                .read_json::<Answer>()
                .map(Some)
                .map_err(|e| format!("réponse du téléphone illisible : {e}")),
            204 => Ok(None),
            404 => Err("code d'association expiré".into()),
            s => Err(format!("relais : HTTP {s}")),
        }
    }

    fn put_schedule(&self, d: &Device, entries: &[Entry]) -> Result<Stored, String> {
        let r = self
            .agent
            .put(format!("{}/api/devices/{}/schedule", self.base, d.id))
            .header("Authorization", format!("Bearer {}", d.schedule_key))
            .send_json(
                serde_json::json!({ "endpoint": d.subscription.endpoint, "entries": entries }),
            )
            .map_err(|e| http_err("planification", e))?;
        match r.status().as_u16() {
            200 => Ok(Stored::Ok),
            410 => Ok(Stored::Gone),
            s => Err(format!("relais : planification refusée (HTTP {s})")),
        }
    }

    fn forget(&self, d: &Device) -> Result<(), String> {
        let r = self
            .agent
            .delete(format!("{}/api/devices/{}", self.base, d.id))
            .header("Authorization", format!("Bearer {}", d.schedule_key))
            .call()
            .map_err(|e| http_err("révocation", e))?;
        match r.status().as_u16() {
            204 | 404 | 410 => Ok(()),
            s => Err(format!("relais : révocation refusée (HTTP {s})")),
        }
    }
}

// ---------- pairing ----------

const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub fn new_code() -> String {
    uuid::Uuid::new_v4().as_bytes()[..10]
        .iter()
        .map(|b| ALPHABET[(*b % 32) as usize] as char)
        .collect()
}

fn new_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Adds the phone that answered (re-pairing the same subscription replaces it). Returns its id.
pub fn add(devices: &mut Vec<Device>, a: Answer, now: i64) -> Result<String, String> {
    a.subscription.validate()?;
    devices.retain(|d| d.subscription.endpoint != a.subscription.endpoint);
    let label: String = a
        .label
        .chars()
        .filter(|c| !c.is_control())
        .take(40)
        .collect();
    let id = uuid::Uuid::new_v4().simple().to_string();
    devices.push(Device {
        id: id.clone(),
        label: if label.trim().is_empty() {
            "Téléphone".into()
        } else {
            label
        },
        subscription: a.subscription,
        enabled: true,
        paired_at: now,
        schedule_key: new_secret(),
        synced: None,
        status: None,
    });
    Ok(id)
}

pub fn qr_svg(url: &str) -> Result<String, String> {
    let code = qrcode::QrCode::with_error_correction_level(url, qrcode::EcLevel::M)
        .map_err(|e| e.to_string())?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .build())
}

// ---------- sync ----------

/// One watcher round, as seen by the phones.
pub struct Tick<'a> {
    pub now: i64,
    pub accounts: &'a [PhoneAccount],
    pub resets: &'a [(String, Pending)],
    pub alerts: &'a [Alert],
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Synced(String),
    Status(Sent),
    Remove,
}

fn fingerprint(enabled: bool, resets: &[(String, Pending)]) -> String {
    let mut h = Sha256::new();
    h.update([enabled as u8]);
    if enabled {
        for (k, p) in resets.iter().take(MAX_SCHEDULE) {
            h.update(format!("{k}@{};{};", p.resets_at, p.account).as_bytes());
        }
    }
    h.finalize()[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn topic(key: &str) -> String {
    Sha256::digest(key.as_bytes())[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Encrypted reset pushes for one phone, sent by the relay at each reset.
pub fn schedule(
    vapid: &Vapid,
    d: &Device,
    resets: &[(String, Pending)],
) -> Result<Vec<Entry>, String> {
    if !d.enabled {
        return Ok(vec![]);
    }
    resets
        .iter()
        .take(MAX_SCHEDULE)
        .map(|(key, p)| {
            let n = reset_notify::notice(p);
            let plain = encode(Payload {
                v: 1,
                kind: "reset",
                title: Some(&n.title),
                body: Some(&n.body),
                tag: Some(format!("reset-{}", topic(key))),
                sound: true,
                at: p.resets_at,
                reset: Some(ResetInfo {
                    key,
                    window: &p.window,
                    resets_at: p.resets_at,
                }),
                accounts: None,
            });
            let expires = p.resets_at + RESET_TTL_SECS;
            let push = webpush::prepare(
                vapid,
                &d.subscription,
                &plain,
                expires,
                RESET_TTL_SECS as u64,
                "high",
                Some(topic(key)),
            )?;
            Ok(Entry {
                at: p.resets_at,
                expires,
                push,
            })
        })
        .collect()
}

/// Pushes one message now; JWT valid 12 h, kept by the push service for `ttl`.
pub fn push_now(
    vapid: &Vapid,
    pusher: &dyn Pusher,
    d: &Device,
    plain: &[u8],
    now: i64,
    ttl: u64,
    urgency: &str,
) -> Outcome {
    match webpush::prepare(
        vapid,
        &d.subscription,
        plain,
        now + 12 * 3600,
        ttl,
        urgency,
        None,
    ) {
        Ok(p) => pusher.send(&d.subscription.endpoint, &p),
        Err(e) => Outcome::Failed(e),
    }
}

pub fn test_message(now: i64, accounts: &[PhoneAccount]) -> Vec<u8> {
    message(
        "test",
        "Test · Trigenys Claude Accounts Manager",
        "Les alertes et réinitialisations arriveront ici, même PC éteint.",
        true,
        now,
        accounts,
    )
}

pub fn paired_message(now: i64, accounts: &[PhoneAccount]) -> Vec<u8> {
    message(
        "paired",
        "Téléphone associé",
        "Vous recevrez les alertes de consommation et les réinitialisations de quota.",
        false,
        now,
        accounts,
    )
}

/// Drives the paired phones from the reset watcher. Keeps only retry timers in memory.
#[derive(Default)]
pub struct Hub {
    retry_after: HashMap<String, i64>,
}

impl Hub {
    /// Sends what is due and returns the per-device changes to save. A converged round sends nothing.
    pub fn tick(
        &mut self,
        vapid: &Vapid,
        devices: &[Device],
        t: &Tick,
        pusher: &dyn Pusher,
        relay: &dyn Relay,
    ) -> Vec<(String, Change)> {
        let mut changes = Vec::new();
        let status_fp = status_fingerprint(t.accounts);
        for d in devices {
            let mut gone = false;
            // Reset schedule on the relay: replaced whole, only when the upcoming resets change.
            let fp = fingerprint(d.enabled, t.resets);
            let retry_ok = self.retry_after.get(&d.id).is_none_or(|r| t.now >= *r);
            if d.synced.as_deref() != Some(fp.as_str()) && retry_ok {
                match schedule(vapid, d, t.resets).and_then(|e| relay.put_schedule(d, &e)) {
                    Ok(Stored::Ok) => {
                        self.retry_after.remove(&d.id);
                        changes.push((d.id.clone(), Change::Synced(fp)));
                    }
                    Ok(Stored::Gone) => gone = true,
                    Err(e) => {
                        eprintln!("phone schedule not stored: {e}");
                        self.retry_after.insert(d.id.clone(), t.now + RETRY_SECS);
                    }
                }
            }
            if d.enabled && !gone {
                let mut delivered = false;
                for a in t.alerts {
                    let plain = message("alert", &a.title, &a.body, a.sound, t.now, t.accounts);
                    match push_now(vapid, pusher, d, &plain, t.now, 3600, "high") {
                        Outcome::Delivered => delivered = true,
                        Outcome::Gone => {
                            gone = true;
                            break;
                        }
                        Outcome::Failed(e) => eprintln!("phone alert not sent: {e}"),
                    }
                }
                let due = match &d.status {
                    None => !t.accounts.is_empty(),
                    Some(s) => s.fingerprint != status_fp && t.now - s.at >= STATUS_MIN_GAP_SECS,
                };
                if !gone && !delivered && due {
                    let plain = encode(Payload {
                        v: 1,
                        kind: "status",
                        title: None,
                        body: None,
                        tag: None,
                        sound: false,
                        at: t.now,
                        reset: None,
                        accounts: Some(t.accounts.to_vec()),
                    });
                    match push_now(vapid, pusher, d, &plain, t.now, 12 * 3600, "low") {
                        Outcome::Delivered => delivered = true,
                        Outcome::Gone => gone = true,
                        Outcome::Failed(e) => eprintln!("phone status not sent: {e}"),
                    }
                }
                if delivered && !gone {
                    changes.push((
                        d.id.clone(),
                        Change::Status(Sent {
                            at: t.now,
                            fingerprint: status_fp.clone(),
                        }),
                    ));
                }
            }
            if gone {
                // Pruned on both sides: the relay drops its schedule too.
                let _ = relay.forget(d);
                self.retry_after.remove(&d.id);
                changes.retain(|(id, _)| id != &d.id);
                changes.push((d.id.clone(), Change::Remove));
            }
        }
        changes
    }
}

/// Applies the changes of a round to the stored devices (which may have changed meanwhile).
pub fn apply(devices: &mut Vec<Device>, changes: &[(String, Change)]) -> bool {
    let mut changed = false;
    for (id, c) in changes {
        match c {
            Change::Remove => {
                let before = devices.len();
                devices.retain(|d| &d.id != id);
                changed |= before != devices.len();
            }
            Change::Synced(fp) => {
                if let Some(d) = devices.iter_mut().find(|d| &d.id == id) {
                    d.synced = Some(fp.clone());
                    changed = true;
                }
            }
            Change::Status(s) => {
                if let Some(d) = devices.iter_mut().find(|d| &d.id == id) {
                    d.status = Some(s.clone());
                    changed = true;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webpush::tests::{decrypt, ua};
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakePusher {
        sent: RefCell<Vec<Prepared>>,
        answer: RefCell<Option<Outcome>>,
    }
    impl Pusher for FakePusher {
        fn send(&self, _: &str, p: &Prepared) -> Outcome {
            self.sent.borrow_mut().push(p.clone());
            self.answer.borrow().clone().unwrap_or(Outcome::Delivered)
        }
    }

    #[derive(Default)]
    struct FakeRelay {
        puts: RefCell<Vec<Vec<Entry>>>,
        forgot: RefCell<Vec<String>>,
        gone: bool,
        down: bool,
    }
    impl Relay for FakeRelay {
        fn offer(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn take(&self, _: &str) -> Result<Option<Answer>, String> {
            Ok(None)
        }
        fn put_schedule(&self, _: &Device, e: &[Entry]) -> Result<Stored, String> {
            if self.down {
                return Err("down".into());
            }
            self.puts.borrow_mut().push(e.to_vec());
            Ok(if self.gone { Stored::Gone } else { Stored::Ok })
        }
        fn forget(&self, d: &Device) -> Result<(), String> {
            self.forgot.borrow_mut().push(d.id.clone());
            Ok(())
        }
    }

    fn pending(key: &str, at: i64) -> (String, Pending) {
        (
            key.into(),
            Pending {
                account: "Perso".into(),
                window: "Session de 5 h".into(),
                resets_at: at,
            },
        )
    }

    fn setup() -> (tempfile::TempDir, Vapid, Vec<Device>, p256_secret::Secret) {
        let dir = tempfile::tempdir().unwrap();
        let vapid = Vapid::load_or_create(dir.path(), "https://example.test").unwrap();
        let (sub, secret, auth) = ua();
        let mut devices = vec![];
        add(
            &mut devices,
            Answer {
                subscription: sub,
                label: "Pixel".into(),
            },
            0,
        )
        .unwrap();
        (dir, vapid, devices, (secret, auth))
    }

    mod p256_secret {
        pub type Secret = (web_push_native::p256::SecretKey, web_push_native::Auth);
    }

    fn acc(pct: f64) -> Vec<PhoneAccount> {
        vec![PhoneAccount {
            id: "a".into(),
            label: "Perso".into(),
            captured_at: 100,
            windows: vec![PhoneWindow {
                name: "Session de 5 h".into(),
                pct,
                resets_at: 18_000,
            }],
        }]
    }

    fn round(
        hub: &mut Hub,
        v: &Vapid,
        d: &mut Vec<Device>,
        t: &Tick,
        p: &FakePusher,
        r: &FakeRelay,
    ) -> bool {
        let c = hub.tick(v, d, t, p, r);
        apply(d, &c)
    }

    #[test]
    fn schedule_is_replaced_only_when_resets_change() {
        let (_dir, vapid, mut devices, (secret, auth)) = setup();
        let (p, r, mut hub) = (FakePusher::default(), FakeRelay::default(), Hub::default());
        let resets = vec![pending("a/Session de 5 h", 18_000)];
        let accounts = acc(40.0);
        let t = Tick {
            now: 1000,
            accounts: &accounts,
            resets: &resets,
            alerts: &[],
        };
        assert!(round(&mut hub, &vapid, &mut devices, &t, &p, &r));
        assert_eq!(r.puts.borrow().len(), 1);
        let e = r.puts.borrow()[0][0].clone();
        assert_eq!((e.at, e.expires), (18_000, 18_000 + RESET_TTL_SECS));
        let plain: serde_json::Value =
            serde_json::from_slice(&decrypt(&e.push, &secret, &auth)).unwrap();
        assert_eq!(plain["kind"], "reset");
        assert_eq!(plain["title"], "Perso · Session de 5 h");
        assert!(
            plain.get("accounts").is_none(),
            "no percentages in relay payloads"
        );

        // Converged: same resets, nothing changed → no PUT, no push, no write.
        let t2 = Tick { now: 1020, ..t };
        assert!(!round(&mut hub, &vapid, &mut devices, &t2, &p, &r));
        assert_eq!(r.puts.borrow().len(), 1);
        assert_eq!(p.sent.borrow().len(), 1, "only the first status push");

        // New readout moves the reset: the whole schedule is replaced.
        let moved = vec![pending("a/Session de 5 h", 36_000)];
        let t3 = Tick {
            now: 1040,
            resets: &moved,
            ..t
        };
        assert!(round(&mut hub, &vapid, &mut devices, &t3, &p, &r));
        assert_eq!(r.puts.borrow().len(), 2);
        assert_eq!(r.puts.borrow()[1].len(), 1);
        assert_eq!(r.puts.borrow()[1][0].at, 36_000);

        // Disabled phone: schedule emptied on the relay, no direct push.
        devices[0].enabled = false;
        let t4 = Tick {
            now: 9000,
            accounts: &acc(90.0),
            resets: &moved,
            alerts: &[],
        };
        round(&mut hub, &vapid, &mut devices, &t4, &p, &r);
        assert!(r.puts.borrow()[2].is_empty());
        assert_eq!(p.sent.borrow().len(), 1);
    }

    #[test]
    fn gone_subscription_is_pruned_on_both_sides() {
        let (_dir, vapid, mut devices, _) = setup();
        let (p, r, mut hub) = (FakePusher::default(), FakeRelay::default(), Hub::default());
        *p.answer.borrow_mut() = Some(Outcome::Gone);
        let accounts = acc(40.0);
        let t = Tick {
            now: 1000,
            accounts: &accounts,
            resets: &[],
            alerts: &[],
        };
        let id = devices[0].id.clone();
        assert!(round(&mut hub, &vapid, &mut devices, &t, &p, &r));
        assert!(devices.is_empty());
        assert_eq!(*r.forgot.borrow(), vec![id]);

        // Relay already pruned it (its own 404/410): removed here too.
        let (_dir, vapid, mut devices, _) = setup();
        let r = FakeRelay {
            gone: true,
            ..Default::default()
        };
        assert!(round(
            &mut hub,
            &vapid,
            &mut devices,
            &t,
            &FakePusher::default(),
            &r
        ));
        assert!(devices.is_empty());
    }

    #[test]
    fn status_push_throttled_and_alerts_carry_snapshot() {
        let (_dir, vapid, mut devices, (secret, auth)) = setup();
        let (p, r, mut hub) = (FakePusher::default(), FakeRelay::default(), Hub::default());
        let a40 = acc(40.0);
        let t = Tick {
            now: 1000,
            accounts: &a40,
            resets: &[],
            alerts: &[],
        };
        round(&mut hub, &vapid, &mut devices, &t, &p, &r);
        assert_eq!(p.sent.borrow().len(), 1);
        // Big change but too soon: nothing. Later: one status push.
        let a60 = acc(60.0);
        round(
            &mut hub,
            &vapid,
            &mut devices,
            &Tick {
                now: 1100,
                accounts: &a60,
                ..t
            },
            &p,
            &r,
        );
        assert_eq!(p.sent.borrow().len(), 1);
        round(
            &mut hub,
            &vapid,
            &mut devices,
            &Tick {
                now: 1000 + STATUS_MIN_GAP_SECS,
                accounts: &a60,
                ..t
            },
            &p,
            &r,
        );
        assert_eq!(p.sent.borrow().len(), 2);
        // An alert is pushed right away, with the percentages.
        let alerts = [Alert {
            title: "Perso · 75 %".into(),
            body: "b".into(),
            sound: true,
        }];
        round(
            &mut hub,
            &vapid,
            &mut devices,
            &Tick {
                now: 2000,
                accounts: &a60,
                alerts: &alerts,
                ..t
            },
            &p,
            &r,
        );
        let last = p.sent.borrow().last().cloned().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&decrypt(&last, &secret, &auth)).unwrap();
        assert_eq!(
            (v["kind"].as_str(), v["sound"].as_bool()),
            (Some("alert"), Some(true))
        );
        assert_eq!(v["accounts"][0]["windows"][0]["pct"], 60.0);
        assert_eq!(last.urgency, "high");
    }

    #[test]
    fn relay_down_is_retried_later_not_every_round() {
        let (_dir, vapid, mut devices, _) = setup();
        let (p, mut hub) = (FakePusher::default(), Hub::default());
        let down = FakeRelay {
            down: true,
            ..Default::default()
        };
        let resets = vec![pending("k", 50_000)];
        let t = Tick {
            now: 1000,
            accounts: &[],
            resets: &resets,
            alerts: &[],
        };
        round(&mut hub, &vapid, &mut devices, &t, &p, &down);
        assert!(devices[0].synced.is_none());
        let up = FakeRelay::default();
        round(
            &mut hub,
            &vapid,
            &mut devices,
            &Tick { now: 1020, ..t },
            &p,
            &up,
        );
        assert!(up.puts.borrow().is_empty(), "waits for the retry delay");
        round(
            &mut hub,
            &vapid,
            &mut devices,
            &Tick {
                now: 1000 + RETRY_SECS,
                ..t
            },
            &p,
            &up,
        );
        assert_eq!(up.puts.borrow().len(), 1);
        assert!(devices[0].synced.is_some());
    }

    #[test]
    fn pairing_validates_and_replaces_same_phone() {
        let mut d = vec![];
        let (sub, _, _) = ua();
        add(
            &mut d,
            Answer {
                subscription: sub.clone(),
                label: "Pixel\u{7}".into(),
            },
            1,
        )
        .unwrap();
        add(
            &mut d,
            Answer {
                subscription: sub.clone(),
                label: String::new(),
            },
            2,
        )
        .unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].label, "Téléphone");
        let mut bad = sub;
        bad.endpoint = "https://evil.test/x".into();
        assert!(
            add(
                &mut d,
                Answer {
                    subscription: bad,
                    label: "x".into()
                },
                3
            )
            .is_err()
        );
        let c = new_code();
        assert_eq!(c.len(), 10);
        assert!(c.bytes().all(|b| ALPHABET.contains(&b)));
        assert!(
            qr_svg("https://example.test/app/#pair=ABC")
                .unwrap()
                .starts_with("<?xml")
        );
        assert!(!d[0].view().service.contains('/'));
    }

    #[test]
    fn oversized_payload_drops_accounts_to_fit() {
        let many: Vec<_> = (0..60)
            .map(|i| PhoneAccount {
                id: format!("{i}"),
                label: "x".repeat(30),
                ..acc(1.0).remove(0)
            })
            .collect();
        let b = message("alert", "t", "b", false, 0, &many);
        assert!(b.len() <= MAX_PLAINTEXT);
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert!(v["accounts"].as_array().unwrap().len() < 60);
    }
}
