//! Provider-scoped normalization for Slack's non-standard OAuth token responses.

use oauth2::HttpResponse;
use oauth2::http::HeaderValue;
use oauth2::http::Response;
use oauth2::http::StatusCode;
use oauth2::http::header::CONTENT_TYPE;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use url::Url;

const NORMALIZED_TOKEN_TYPE: &str = "bearer";
const UNSPECIFIED_SLACK_ERROR: &str = "slack_api_error";
const SLACK_ENVELOPE_ERROR_DESCRIPTION: &str = "Slack token endpoint returned ok=false";

/// Normalizes token responses from a verified Slack OAuth token endpoint.
///
/// Constructing the adapter requires an exact Slack HTTPS token endpoint URL. This provider
/// identity check prevents a generic JSON `ok` field from activating Slack-specific behavior for
/// another OAuth provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlackOAuthResponseAdapter(());

impl SlackOAuthResponseAdapter {
    /// Returns an adapter only for Slack's known Web API OAuth token endpoints.
    pub fn from_token_endpoint(token_endpoint: &Url) -> Option<Self> {
        let is_slack_token_endpoint = token_endpoint.scheme() == "https"
            && token_endpoint.username().is_empty()
            && token_endpoint.password().is_none()
            && token_endpoint.host_str() == Some("slack.com")
            && token_endpoint.port_or_known_default() == Some(443)
            && matches!(
                token_endpoint.path(),
                "/api/oauth.v2.access" | "/api/oauth.v2.user.access"
            )
            && token_endpoint.query().is_none()
            && token_endpoint.fragment().is_none();
        is_slack_token_endpoint.then_some(Self(()))
    }

    /// Rewrites a Slack Web API envelope into the RFC 6749 shape expected by `oauth2`.
    ///
    /// Non-envelope and malformed responses are returned unchanged.
    pub fn normalize_response(self, response: HttpResponse) -> HttpResponse {
        let Some(normalized) = normalize_slack_token_envelope(response.body()) else {
            return response;
        };
        let (mut parts, _) = response.into_parts();
        parts.status = normalized.status;
        parts
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Response::from_parts(parts, normalized.body)
    }
}

struct NormalizedSlackTokenResponse {
    status: StatusCode,
    body: Vec<u8>,
}

fn normalize_slack_token_envelope(body: &[u8]) -> Option<NormalizedSlackTokenResponse> {
    let envelope: Value = serde_json::from_slice(body).ok()?;
    let envelope = envelope.as_object()?;
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
