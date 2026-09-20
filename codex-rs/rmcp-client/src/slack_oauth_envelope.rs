//! Normalization for Slack's non-standard OAuth token responses.
//!
//! Slack advertises MCP OAuth through the usual discovery document, but the `token_endpoint`
//! it points at (`https://slack.com/api/oauth.v2.user.access`) is a Slack Web API method rather
//! than an RFC 6749 token endpoint. It answers **HTTP 200** for both success and failure and
//! wraps the payload in Slack's Web API envelope:
//!
//! ```json
//! {"ok": true,  "access_token": "xoxp-...", "token_type": "user", "refresh_token": "xoxe-...", ...}
//! {"ok": false, "error": "invalid_refresh_token"}
//! ```
//!
//! The `oauth2` crate only accepts RFC 6749 shapes, so every Slack token response - success or
//! failure - surfaces as `RequestTokenError::Parse`, whose `Display` is the famously unhelpful
//! `Failed to parse server response`. A rejected refresh token therefore looks identical to a
//! malformed payload, and the real reason never reaches the logs.
//!
//! This module rewrites the envelope into the standard shapes before `oauth2` parses it. It is
//! applied at our [`crate::oauth_http_client::OAuthHttpClientAdapter`] seam, which every OAuth
//! HTTP request funnels through, so it covers both the refresh and the initial authorization
//! code exchange.

use oauth2::HttpResponse;
use oauth2::http::HeaderValue;
use oauth2::http::Response;
use oauth2::http::StatusCode;
use oauth2::http::header::CONTENT_TYPE;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use tracing::debug;

/// Slack reports `token_type` values such as `user` and `bot`, which describe the token's
/// audience rather than the HTTP authorization scheme. The MCP transport always sends
/// `Authorization: Bearer <token>`, so normalize to the RFC 6749 value.
const NORMALIZED_TOKEN_TYPE: &str = "bearer";

/// Used when Slack reports `ok: false` without a usable `error` string.
const UNSPECIFIED_SLACK_ERROR: &str = "slack_api_error";

const SLACK_ENVELOPE_ERROR_DESCRIPTION: &str = "Slack token endpoint returned ok=false";

/// A Slack Web API envelope rewritten into the RFC 6749 shape `oauth2` expects.
struct NormalizedSlackTokenResponse {
    status: StatusCode,
    body: Vec<u8>,
}

/// Rewrites a Slack Web API token envelope in `response` into the standard OAuth shape.
///
/// Responses that are not Slack envelopes are returned untouched, so this is a no-op for every
/// RFC-compliant provider.
pub(crate) fn normalize_slack_token_response(response: HttpResponse) -> HttpResponse {
    let Some(normalized) = normalize_slack_token_envelope(response.body()) else {
        return response;
    };
    debug!(
        status = normalized.status.as_u16(),
        "rewrote a Slack Web API OAuth token envelope into the standard OAuth shape"
    );
    let (mut parts, _) = response.into_parts();
    parts.status = normalized.status;
    parts
        .headers
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Response::from_parts(parts, normalized.body)
}

/// Returns the rewritten response when `body` is a Slack Web API token envelope.
///
/// Returns `None` for anything else - including an `ok: true` envelope with no access token
/// anywhere - so the original body still reaches `oauth2` and its diagnostics stay intact.
fn normalize_slack_token_envelope(body: &[u8]) -> Option<NormalizedSlackTokenResponse> {
    let envelope: Value = serde_json::from_slice(body).ok()?;
    let envelope = envelope.as_object()?;
    // `ok` is the Slack Web API marker. No OAuth discovery, registration, or token document
    // carries it, which keeps this rewrite off every standards-compliant response.
    let ok = envelope.get("ok")?.as_bool()?;

    if !ok {
        return Some(slack_error_response(envelope));
    }
    slack_token_response(envelope)
}

fn slack_error_response(envelope: &Map<String, Value>) -> NormalizedSlackTokenResponse {
    let error = envelope
        .get("error")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|error| !error.is_empty())
        .unwrap_or(UNSPECIFIED_SLACK_ERROR);
    let body = json!({
        "error": error,
        "error_description": SLACK_ENVELOPE_ERROR_DESCRIPTION,
    });
    NormalizedSlackTokenResponse {
        // RFC 6749 error bodies are only consulted on a non-2xx status; Slack always answers 200.
        status: StatusCode::BAD_REQUEST,
        body: body.to_string().into_bytes(),
    }
}

fn slack_token_response(envelope: &Map<String, Value>) -> Option<NormalizedSlackTokenResponse> {
    let access_token = envelope_str(envelope, "access_token")?;
    let mut body = Map::new();
    body.insert("access_token".to_string(), json!(access_token));
    body.insert("token_type".to_string(), json!(NORMALIZED_TOKEN_TYPE));
    if let Some(refresh_token) = envelope_str(envelope, "refresh_token") {
        body.insert("refresh_token".to_string(), json!(refresh_token));
    }
    if let Some(expires_in) = envelope_field(envelope, "expires_in").and_then(Value::as_u64) {
        body.insert("expires_in".to_string(), json!(expires_in));
    }
    if let Some(scope) = envelope_str(envelope, "scope").and_then(normalize_scope) {
        body.insert("scope".to_string(), json!(scope));
    }

    Some(NormalizedSlackTokenResponse {
        status: StatusCode::OK,
        body: Value::Object(body).to_string().into_bytes(),
    })
}

/// Reads `name` from the envelope root, falling back to Slack's nested `authed_user` object.
///
/// `oauth.v2.user.access` reports user-token fields at the root, while `oauth.v2.access` nests
/// them under `authed_user`; accepting both keeps user-token refreshes working either way.
fn envelope_field<'a>(envelope: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    if let Some(value) = envelope.get(name).filter(|value| !value.is_null()) {
        return Some(value);
    }
    envelope
        .get("authed_user")
        .and_then(Value::as_object)
        .and_then(|authed_user| authed_user.get(name))
        .filter(|value| !value.is_null())
}

fn envelope_str<'a>(envelope: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    envelope_field(envelope, name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Slack separates granted scopes with commas; RFC 6749 uses spaces.
fn normalize_scope(scope: &str) -> Option<String> {
    let scopes = scope
        .split([',', ' ', '\t', '\n', '\r'])
        .filter(|scope| !scope.is_empty())
        .collect::<Vec<_>>();
    (!scopes.is_empty()).then(|| scopes.join(" "))
}

#[cfg(test)]
#[path = "slack_oauth_envelope_tests.rs"]
mod tests;
