use std::env;
use std::path::Component;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use anyhow::bail;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::Hmac;
use hmac::Mac;
use http::HeaderMap;
use http::HeaderValue;
use serde::Serialize;
use sha2::Sha256;

use codex_protocol::ThreadId;
use codex_protocol::shell_environment::CODEX_CWD_ROUTE_AUDIENCE_ENV_VAR;
use codex_protocol::shell_environment::CODEX_CWD_ROUTE_SIGNING_KEY_ENV_VAR;

pub const ROUTE_CLAIM_KEY_ENV: &str = CODEX_CWD_ROUTE_SIGNING_KEY_ENV_VAR;
pub const ROUTE_CLAIM_AUDIENCE_ENV: &str = CODEX_CWD_ROUTE_AUDIENCE_ENV_VAR;
pub const ROUTE_CLAIM_HEADER: &str = "x-codex-route-claim";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RouteClaimSigner {
    key: Vec<u8>,
    audience: String,
    thread_id: String,
    cwd: String,
}

#[derive(Serialize)]
struct RouteClaimPayload<'a> {
    aud: &'a str,
    thread_id: &'a str,
    cwd: &'a str,
    iat: u64,
    exp: u64,
}

impl RouteClaimSigner {
    /// A partial sender configuration is unsafe because it silently recreates the
    /// metadata-only failure the claim protocol is meant to remove.
    pub(crate) fn from_environment(thread_id: ThreadId, cwd: &Path) -> Result<Option<Self>> {
        let key = env::var_os(ROUTE_CLAIM_KEY_ENV);
        let audience = env::var_os(ROUTE_CLAIM_AUDIENCE_ENV);
        if key.is_none() && audience.is_none() {
            return Ok(None);
        }
        let key = key
            .and_then(|value| value.into_string().ok())
            .filter(|value| value.len() >= 32 && !value.contains(['\r', '\n']))
            .ok_or_else(|| anyhow::anyhow!("invalid route-claim signing configuration"))?;
        let audience = audience
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.is_empty() && !value.contains(['\r', '\n']))
            .ok_or_else(|| anyhow::anyhow!("invalid route-claim signing configuration"))?;
        Ok(Some(Self {
            key: key.into_bytes(),
            audience,
            thread_id: thread_id.to_string(),
            cwd: normalized_posix_cwd(cwd)?,
        }))
    }

    pub(crate) fn insert_header(&self, headers: &mut HeaderMap) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("route claims require a system clock after the Unix epoch")
            .as_secs();
        let payload = RouteClaimPayload {
            aud: &self.audience,
            thread_id: &self.thread_id,
            cwd: &self.cwd,
            iat: now,
            exp: now + 300,
        };
        let encoded = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).expect("route-claim payload is serializable"));
        let signed = format!("v1.{encoded}");
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key)
            .expect("route-claim signing key was validated at session construction");
        mac.update(signed.as_bytes());
        let claim = format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        );
        let value =
            HeaderValue::from_str(&claim).expect("base64url route claims are valid HTTP headers");
        headers.insert(ROUTE_CLAIM_HEADER, value);
    }

    pub(crate) fn with_cwd(&self, cwd: &Path) -> Result<Self> {
        Ok(Self {
            cwd: normalized_posix_cwd(cwd)?,
            ..self.clone()
        })
    }
}

fn normalized_posix_cwd(cwd: &Path) -> Result<String> {
    if !cwd.is_absolute() {
        bail!("route-claim cwd must be an absolute path");
    }
    let mut segments = Vec::new();
    for component in cwd.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                segments.pop();
            }
            Component::Normal(segment) => segments.push(
                segment
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("route-claim cwd is not UTF-8"))?,
            ),
            Component::Prefix(_) => bail!("route-claim cwd must be a POSIX absolute path"),
        }
    }
    let result = format!("/{}", segments.join("/"));
    if result.chars().any(char::is_control) {
        bail!("route-claim cwd contains a control character");
    }
    Ok(result)
}

#[cfg(test)]
#[path = "route_claim_tests.rs"]
mod tests;
