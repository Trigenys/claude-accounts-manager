//! Standard Web Push from the desktop: VAPID key pair (RFC 8292), payload encryption (RFC 8291,
//! aes128gcm) and delivery to the push service endpoint of each paired phone.
//!
//! Crypto comes from `web-push-native` (pure RustCrypto, no OpenSSL); this module only adds key
//! persistence, a VAPID token with an explicit expiry (needed for pushes scheduled on the relay)
//! and the HTTP delivery.

use std::fs;
use std::path::Path;
use std::time::Duration;

use base64ct::{Base64UrlUnpadded, Encoding};
use serde::{Deserialize, Serialize};
use web_push_native::jwt_simple::algorithms::{ECDSAP256KeyPairLike, ECDSAP256PublicKeyLike};
use web_push_native::jwt_simple::prelude::{Claims, ES256KeyPair, UnixTimeStamp};
use web_push_native::p256;

const FILE: &str = "push-vapid.json";

/// Hosts of the browser push services; nothing else is ever contacted with a payload.
const PUSH_HOSTS: &[&str] = &[
    "fcm.googleapis.com",
    "updates.push.services.mozilla.com",
    "web.push.apple.com",
    ".push.apple.com",
    ".notify.windows.com",
];

pub fn allowed_endpoint(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !host.contains([':', '@'])
        && PUSH_HOSTS.iter().any(|h| match h.strip_prefix('.') {
            Some(suffix) => host.ends_with(&format!(".{suffix}")),
            None => host == *h,
        })
}

/// `PushSubscription.toJSON()` as sent by the browser.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Subscription {
    pub endpoint: String,
    pub keys: SubscriptionKeys,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

fn b64(s: &str) -> Result<Vec<u8>, String> {
    Base64UrlUnpadded::decode_vec(s.trim_end_matches('=')).map_err(|_| "base64url invalide".into())
}

impl Subscription {
    /// Rejects anything that is not a browser push endpoint with a valid P-256 key and 16-byte secret.
    pub fn validate(&self) -> Result<(), String> {
        if !allowed_endpoint(&self.endpoint) || self.endpoint.len() > 1024 {
            return Err("point de terminaison push non reconnu".into());
        }
        self.keys().map(|_| ())
    }

    fn keys(&self) -> Result<(p256::PublicKey, web_push_native::Auth), String> {
        let public = p256::PublicKey::from_sec1_bytes(&b64(&self.keys.p256dh)?)
            .map_err(|_| "clé p256dh invalide")?;
        let auth = b64(&self.keys.auth)?;
        if auth.len() != 16 {
            return Err("secret auth invalide".into());
        }
        Ok((public, web_push_native::Auth::clone_from_slice(&auth)))
    }

    /// `https://host` of the endpoint: the VAPID audience.
    pub fn origin(&self) -> String {
        let rest = self.endpoint.trim_start_matches("https://");
        format!("https://{}", rest.split('/').next().unwrap_or_default())
    }
}

/// The app's VAPID key pair, generated once and kept in the app folder.
pub struct Vapid {
    key: ES256KeyPair,
    subject: String,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    private_key: String,
}

impl Vapid {
    pub fn load_or_create(dir: &Path, subject: &str) -> Result<Self, String> {
        let path = dir.join(FILE);
        let existing = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Stored>(&t).ok())
            .and_then(|s| b64(&s.private_key).ok())
            .and_then(|b| ES256KeyPair::from_bytes(&b).ok());
        let key = match existing {
            Some(k) => k,
            None => {
                let k = ES256KeyPair::generate();
                let stored = Stored {
                    private_key: Base64UrlUnpadded::encode_string(&k.to_bytes()),
                };
                fs::write(
                    &path,
                    serde_json::to_vec(&stored).map_err(|e| e.to_string())?,
                )
                .map_err(|e| format!("clé VAPID non enregistrée : {e}"))?;
                k
            }
        };
        Ok(Self {
            key,
            subject: subject.to_string(),
        })
    }

    /// Uncompressed public key, base64url: the browser's `applicationServerKey`.
    pub fn public_key(&self) -> String {
        Base64UrlUnpadded::encode_string(
            &self.key.public_key().public_key().to_bytes_uncompressed(),
        )
    }

    /// `Authorization` header value valid until `exp` (no `iat`/`nbf`: a token signed now can be
    /// sent later by the relay, as long as it is sent less than 24 h before `exp`).
    pub fn authorization(&self, sub: &Subscription, exp: i64) -> Result<String, String> {
        let mut claims = Claims::create(Duration::from_secs(60).into())
            .with_audience(sub.origin())
            .with_subject(&self.subject);
        claims.issued_at = None;
        claims.invalid_before = None;
        claims.expires_at = Some(UnixTimeStamp::from_secs(exp.max(0) as u64));
        let token = self.key.sign(claims).map_err(|e| e.to_string())?;
        Ok(format!("vapid t={token}, k={}", self.public_key()))
    }
}

/// An encrypted push ready to POST, built now and sent now (direct) or later (relay).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Prepared {
    pub authorization: String,
    pub ttl: u64,
    pub urgency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// aes128gcm record, base64url.
    pub body: String,
}

/// Encrypts `plaintext` for the subscription (RFC 8291) and signs a VAPID token expiring at `exp`.
pub fn prepare(
    vapid: &Vapid,
    sub: &Subscription,
    plaintext: &[u8],
    exp: i64,
    ttl: u64,
    urgency: &str,
    topic: Option<String>,
) -> Result<Prepared, String> {
    let (public, auth) = sub.keys()?;
    let body = web_push_native::encrypt(plaintext.to_vec(), &public, &auth)
        .map_err(|e| format!("chiffrement : {e}"))?;
    Ok(Prepared {
        authorization: vapid.authorization(sub, exp)?,
        ttl,
        urgency: urgency.into(),
        topic,
        body: Base64UrlUnpadded::encode_string(&body),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Delivered,
    /// 404/410: the subscription no longer exists; the device must be pruned.
    Gone,
    Failed(String),
}

pub fn classify(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Delivered,
        404 | 410 => Outcome::Gone,
        s => Outcome::Failed(format!("service push : HTTP {s}")),
    }
}

/// Delivers a push to its endpoint.
pub trait Pusher {
    fn send(&self, endpoint: &str, p: &Prepared) -> Outcome;
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into()
}

pub struct HttpPusher(pub ureq::Agent);

impl Pusher for HttpPusher {
    fn send(&self, endpoint: &str, p: &Prepared) -> Outcome {
        if !allowed_endpoint(endpoint) {
            return Outcome::Failed("point de terminaison refusé".into());
        }
        let Ok(body) = b64(&p.body) else {
            return Outcome::Failed("corps invalide".into());
        };
        let mut req = self
            .0
            .post(endpoint)
            .header("Authorization", &p.authorization)
            .header("TTL", p.ttl.to_string())
            .header("Urgency", &p.urgency)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream");
        if let Some(t) = &p.topic {
            req = req.header("Topic", t);
        }
        match req.send(&body[..]) {
            Ok(r) => classify(r.status().as_u16()),
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint;

    /// A browser-side subscription with its private key, to decrypt what the desktop sends.
    pub fn ua() -> (Subscription, p256::SecretKey, web_push_native::Auth) {
        let secret =
            p256::SecretKey::random(&mut web_push_native::p256::elliptic_curve::rand_core::OsRng);
        let auth = web_push_native::Auth::clone_from_slice(&[7u8; 16]);
        let sub = Subscription {
            endpoint: "https://fcm.googleapis.com/fcm/send/abc".into(),
            keys: SubscriptionKeys {
                p256dh: Base64UrlUnpadded::encode_string(
                    secret.public_key().to_encoded_point(false).as_bytes(),
                ),
                auth: Base64UrlUnpadded::encode_string(&auth),
            },
        };
        (sub, secret, auth)
    }

    pub fn decrypt(
        p: &Prepared,
        secret: &p256::SecretKey,
        auth: &web_push_native::Auth,
    ) -> Vec<u8> {
        web_push_native::decrypt(b64(&p.body).unwrap(), secret, auth).unwrap()
    }

    #[test]
    fn encrypts_rfc8291_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let vapid = Vapid::load_or_create(dir.path(), "https://example.test").unwrap();
        let (sub, secret, auth) = ua();
        let p = prepare(
            &vapid,
            &sub,
            b"{\"kind\":\"test\"}",
            2_000_000_000,
            60,
            "normal",
            None,
        )
        .unwrap();
        let raw = b64(&p.body).unwrap();
        // aes128gcm header (RFC 8188): 16-byte salt, one record (plaintext + 17), 65-byte sender key id.
        assert_eq!(&raw[16..20], &(15u32 + 17).to_be_bytes());
        assert_eq!(raw[20], 65);
        assert_eq!(decrypt(&p, &secret, &auth), b"{\"kind\":\"test\"}");
        let p2 = prepare(
            &vapid,
            &sub,
            b"{\"kind\":\"test\"}",
            2_000_000_000,
            60,
            "normal",
            None,
        )
        .unwrap();
        assert_ne!(p.body, p2.body, "fresh salt and ephemeral key on each push");
    }

    #[test]
    fn vapid_key_persists_and_token_has_audience_and_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let a = Vapid::load_or_create(dir.path(), "https://example.test").unwrap();
        let b = Vapid::load_or_create(dir.path(), "https://example.test").unwrap();
        assert_eq!(a.public_key(), b.public_key(), "same key on every launch");
        assert_eq!(b64(&a.public_key()).unwrap().len(), 65);
        let (sub, _, _) = ua();
        let h = a.authorization(&sub, 2_000_000_000).unwrap();
        let token = h
            .strip_prefix("vapid t=")
            .unwrap()
            .split(',')
            .next()
            .unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&b64(token.split('.').nth(1).unwrap()).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["exp"], 2_000_000_000);
        assert_eq!(claims["sub"], "https://example.test");
        assert!(claims.get("iat").is_none() && claims.get("nbf").is_none());
        assert!(h.ends_with(&format!(", k={}", a.public_key())));
    }

    #[test]
    fn only_browser_push_services_and_valid_keys() {
        assert!(allowed_endpoint("https://fcm.googleapis.com/wp/x"));
        assert!(allowed_endpoint(
            "https://wns2-par02p.notify.windows.com/w/?token=x"
        ));
        assert!(allowed_endpoint("https://web.push.apple.com/abc"));
        assert!(!allowed_endpoint("http://fcm.googleapis.com/x"));
        assert!(!allowed_endpoint("https://fcm.googleapis.com.evil.test/x"));
        assert!(!allowed_endpoint("https://evil.test/notify.windows.com"));
        assert!(!allowed_endpoint("https://fcm.googleapis.com:8080/x"));
        let (mut sub, _, _) = ua();
        assert!(sub.validate().is_ok());
        sub.keys.auth = "AAAA".into();
        assert!(sub.validate().is_err());
        assert_eq!(classify(201), Outcome::Delivered);
        assert_eq!(classify(410), Outcome::Gone);
        assert_eq!(classify(404), Outcome::Gone);
        assert!(matches!(classify(429), Outcome::Failed(_)));
    }
}
