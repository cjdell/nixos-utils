use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use axum::http::HeaderMap;
use base64::Engine;
use tokio::sync::Mutex;

use crate::oidc::Oidc;
use crate::podman::Podman;

pub const SESSION_COOKIE: &str = "container_ui_session";
const SESSION_TTL_SECS: u64 = 24 * 60 * 60;
const PENDING_TTL_SECS: u64 = 10 * 60;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen_addr: String,
    pub base_url: String,
    pub webhook_url: Option<String>,
    pub podman_bin: String,
    pub systemctl_bin: String,
    pub oidc: Option<OidcConfig>,
}

#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl Config {
    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let oidc = match (
            env("OIDC_ISSUER"),
            env("OIDC_CLIENT_ID"),
            env("OIDC_CLIENT_SECRET"),
            env("OIDC_REDIRECT_URI"),
        ) {
            (Some(issuer), Some(client_id), Some(client_secret), Some(redirect_uri)) => {
                Some(OidcConfig {
                    issuer,
                    client_id,
                    client_secret,
                    redirect_uri,
                })
            }
            _ => None,
        };
        Config {
            listen_addr: env("LISTEN_ADDR").unwrap_or_else(|| "127.0.0.1:8091".into()),
            base_url: env("BASE_URL").unwrap_or_else(|| "http://localhost:8091".into()),
            webhook_url: env("WEBHOOK_URL"),
            podman_bin: env("PODMAN_BIN").unwrap_or_else(|| "podman".into()),
            systemctl_bin: env("SYSTEMCTL_BIN").unwrap_or_else(|| "systemctl".into()),
            oidc,
        }
    }

    pub fn auth_enabled(&self) -> bool {
        self.oidc.is_some()
    }

    pub fn secure_cookies(&self) -> bool {
        self.base_url.starts_with("https://")
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub username: String,
    pub email: Option<String>,
    pub csrf: String,
    pub created_at: SystemTime,
}

#[derive(Clone, Debug)]
pub struct PendingAuth {
    pub code_verifier: String,
    pub created_at: SystemTime,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub http: reqwest::Client,
    pub podman: Podman,
    pub oidc: Option<Arc<Oidc>>,
    pub sessions: Arc<Mutex<HashMap<String, Session>>>,
    pub pending: Arc<Mutex<HashMap<String, PendingAuth>>>,
}

impl AppState {
    pub async fn insert_session(&self, session: Session) -> String {
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, s| {
            s.created_at
                .elapsed()
                .map(|e| e.as_secs() < SESSION_TTL_SECS)
                .unwrap_or(true)
        });
        let token = random_hex(16);
        sessions.insert(token.clone(), session);
        token
    }

    pub async fn insert_pending(&self, pending: PendingAuth) -> String {
        let mut pending_map = self.pending.lock().await;
        pending_map.retain(|_, p| {
            p.created_at
                .elapsed()
                .map(|e| e.as_secs() < PENDING_TTL_SECS)
                .unwrap_or(true)
        });
        let state_val = random_hex(16);
        pending_map.insert(state_val.clone(), pending);
        state_val
    }
}

pub fn random_bytes(len: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; len];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .expect("read /dev/urandom");
    buf
}

pub fn random_hex(len: usize) -> String {
    random_bytes(len)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn random_b64url(len: usize) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    URL_SAFE_NO_PAD.encode(random_bytes(len))
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .map(|s| s.trim())
        .find_map(|pair| {
            pair.split_once('=')
                .and_then(|(k, v)| (k == name).then(|| v.to_string()))
        })
}
