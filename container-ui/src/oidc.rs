use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, JwkSet};
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::state::{OidcConfig, random_b64url};

pub struct Oidc {
    pub config: OidcConfig,
    discovery: Arc<Mutex<Option<Discovery>>>,
    jwks: Arc<Mutex<JwksCache>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Discovery {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    #[serde(default)]
    pub issuer: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub id_token: Option<String>,
    #[serde(default)]
    pub access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IdClaims {
    pub iss: String,
    pub exp: i64,
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub preferred_username: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

struct JwksCache {
    keys: Vec<Jwk>,
    fetched_at: Option<SystemTime>,
}

impl Oidc {
    pub fn new(config: OidcConfig) -> Self {
        Oidc {
            config,
            discovery: Arc::new(Mutex::new(None)),
            jwks: Arc::new(Mutex::new(JwksCache {
                keys: Vec::new(),
                fetched_at: None,
            })),
        }
    }

    pub async fn discover(&self, http: &reqwest::Client) -> Result<Discovery, String> {
        {
            let guard = self.discovery.lock().unwrap();
            if let Some(d) = guard.as_ref() {
                return Ok(d.clone());
            }
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer.trim_end_matches('/')
        );
        let d: Discovery = http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("fetch OIDC discovery: {e}"))?
            .error_for_status()
            .map_err(|e| format!("OIDC discovery endpoint error: {e}"))?
            .json()
            .await
            .map_err(|e| format!("parse OIDC discovery: {e}"))?;
        let d = Discovery {
            issuer: if d.issuer.is_empty() {
                self.config.issuer.clone()
            } else {
                d.issuer
            },
            ..d
        };
        *self.discovery.lock().unwrap() = Some(d.clone());
        Ok(d)
    }

    pub async fn jwks(&self, http: &reqwest::Client) -> Result<Vec<Jwk>, String> {
        let needs_refresh = {
            let cache = self.jwks.lock().unwrap();
            match cache.fetched_at {
                Some(t) => t
                    .elapsed()
                    .map(|e| e > Duration::from_secs(3600))
                    .unwrap_or(true),
                None => true,
            }
        };
        if !needs_refresh {
            return Ok(self.jwks.lock().unwrap().keys.clone());
        }
        let disc = self.discover(http).await?;
        let set: JwkSet = http
            .get(&disc.jwks_uri)
            .send()
            .await
            .map_err(|e| format!("fetch JWKS: {e}"))?
            .error_for_status()
            .map_err(|e| format!("JWKS endpoint error: {e}"))?
            .json()
            .await
            .map_err(|e| format!("parse JWKS: {e}"))?;
        let keys = set.keys;
        if keys.is_empty() {
            return Err("no keys in JWKS".to_string());
        }
        let mut cache = self.jwks.lock().unwrap();
        cache.keys = keys.clone();
        cache.fetched_at = Some(SystemTime::now());
        Ok(keys)
    }

    pub fn authorize_url(&self, disc: &Discovery, state: &str, challenge: &str) -> String {
        let mut url = Url::parse(&disc.authorization_endpoint).expect("valid authorize endpoint");
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code");
            q.append_pair("client_id", &self.config.client_id);
            q.append_pair("redirect_uri", &self.config.redirect_uri);
            q.append_pair("scope", "openid profile email groups");
            q.append_pair("state", state);
            q.append_pair("code_challenge", challenge);
            q.append_pair("code_challenge_method", "S256");
        }
        url.to_string()
    }

    pub async fn exchange_code(
        &self,
        http: &reqwest::Client,
        code: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, String> {
        let disc = self.discover(http).await?;
        let body = serde_urlencoded::to_string([
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("code_verifier", code_verifier),
        ])
        .expect("encode token request");
        let resp = http
            .post(&disc.token_endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("token exchange request: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("token exchange failed ({status}): {text}"));
        }
        resp.json()
            .await
            .map_err(|e| format!("parse token response: {e}"))
    }

    pub async fn verify_id_token(
        &self,
        http: &reqwest::Client,
        id_token: &str,
    ) -> Result<IdClaims, String> {
        let disc = self.discover(http).await?;
        let header = decode_header(id_token)
            .map_err(|e| format!("decode id_token header: {e}"))?;
        let keys = self.jwks(http).await?;
        let kid = header.kid.clone();
        let key = if let Some(k) = kid.as_deref() {
            keys.iter().find(|jwk| jwk.common.key_id.as_deref() == Some(k))
        } else {
            keys.first()
        }
        .ok_or_else(|| format!("no JWKS key found for kid {kid:?}"))?;
        let decoding_key = match &key.algorithm {
            AlgorithmParameters::RSA(rsa) => DecodingKey::from_rsa_components(&rsa.n, &rsa.e)
                .map_err(|e| format!("build RSA decoding key: {e}"))?,
            AlgorithmParameters::EllipticCurve(ec) => {
                DecodingKey::from_ec_components(&ec.x, &ec.y)
                    .map_err(|e| format!("build EC decoding key: {e}"))?
            }
            _ => return Err("unsupported JWKS key type".to_string()),
        };
        let mut validation = Validation::new(header.alg);
        let issuer: &str = &disc.issuer;
        let client_id: &str = &self.config.client_id;
        validation.set_issuer(&[issuer]);
        validation.set_audience(&[client_id]);
        validation.validate_exp = true;
        decode::<IdClaims>(id_token, &decoding_key, &validation)
            .map_err(|e| format!("id_token validation: {e}"))
            .map(|d| d.claims)
    }
}

/// PKCE: 32 random bytes, base64url-no-pad.
pub fn code_verifier() -> String {
    random_b64url(32)
}

/// PKCE S256 challenge for a verifier.
pub fn code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}
