use super::normalize_slack_token_response;
use oauth2::HttpResponse;
use oauth2::StandardErrorResponse;
use oauth2::TokenResponse;
use oauth2::basic::BasicErrorResponseType;
use oauth2::basic::BasicTokenType;
use oauth2::http::Response;
use oauth2::http::StatusCode;
use oauth2::http::header::CONTENT_TYPE;
use pretty_assertions::assert_eq;
use rmcp::transport::auth::OAuthTokenResponse;
use std::time::Duration;

/// Slack answers `200 application/json` for both success and failure.
fn slack_response(body: &str) -> HttpResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .body(body.as_bytes().to_vec())
        .expect("build response")
}

fn parse_token_response(response: &HttpResponse) -> OAuthTokenResponse {
    serde_json::from_slice(response.body()).expect("parse standard token response")
}

fn parse_error_response(response: &HttpResponse) -> StandardErrorResponse<BasicErrorResponseType> {
    serde_json::from_slice(response.body()).expect("parse standard error response")
}

#[test]
fn slack_ok_true_envelope_becomes_a_standard_token_response() {
    let response = normalize_slack_token_response(slack_response(
        r#"{
            "ok": true,
            "app_id": "A0000000000",
            "access_token": "xoxp-new-access-token",
            "token_type": "user",
            "refresh_token": "xoxe-1-new-refresh-token",
            "expires_in": 43200,
            "scope": "channels:read,chat:write,search:read",
            "team": { "id": "T0000000000", "name": "Example" },
            "enterprise": null
        }"#,
    ));

    assert_eq!(response.status(), StatusCode::OK);
    let token = parse_token_response(&response);
    assert_eq!(token.access_token().secret(), "xoxp-new-access-token");
    assert_eq!(token.token_type(), &BasicTokenType::Bearer);
    assert_eq!(
        token.refresh_token().map(|token| token.secret().as_str()),
        Some("xoxe-1-new-refresh-token")
    );
    assert_eq!(token.expires_in(), Some(Duration::from_secs(43200)));
    assert_eq!(
        token.scopes().map(|scopes| scopes
            .iter()
            .map(|scope| scope.to_string())
            .collect::<Vec<_>>()),
        Some(vec![
            "channels:read".to_string(),
            "chat:write".to_string(),
            "search:read".to_string(),
        ])
    );
}

#[test]
fn slack_ok_true_envelope_without_rotation_omits_the_refresh_token() {
    let response = normalize_slack_token_response(slack_response(
        r#"{"ok": true, "access_token": "xoxp-new-access-token", "token_type": "user"}"#,
    ));

    assert_eq!(response.status(), StatusCode::OK);
    let token = parse_token_response(&response);
    assert_eq!(token.access_token().secret(), "xoxp-new-access-token");
    assert_eq!(token.token_type(), &BasicTokenType::Bearer);
    assert!(token.refresh_token().is_none());
    assert_eq!(token.expires_in(), None);
    assert!(token.scopes().is_none());
}

#[test]
fn slack_user_token_nested_under_authed_user_is_used() {
    let response = normalize_slack_token_response(slack_response(
        r#"{
            "ok": true,
            "app_id": "A0000000000",
            "authed_user": {
                "id": "U0000000000",
                "access_token": "xoxp-nested-access-token",
                "token_type": "user",
                "refresh_token": "xoxe-1-nested-refresh-token",
                "expires_in": 43200,
                "scope": "channels:read,chat:write"
            }
        }"#,
    ));

    assert_eq!(response.status(), StatusCode::OK);
    let token = parse_token_response(&response);
    assert_eq!(token.access_token().secret(), "xoxp-nested-access-token");
    assert_eq!(
        token.refresh_token().map(|token| token.secret().as_str()),
        Some("xoxe-1-nested-refresh-token")
    );
    assert_eq!(token.expires_in(), Some(Duration::from_secs(43200)));
}

#[test]
fn slack_ok_false_envelope_becomes_an_oauth_error_carrying_the_slack_error() {
    let response =
        normalize_slack_token_response(slack_response(r#"{"ok": false, "error": "invalid_code"}"#));

    // `oauth2` only reads an error body on a non-2xx status, so the rewrite must restate the
    // failure Slack reported under HTTP 200.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = parse_error_response(&response);
    assert_eq!(
        error.error(),
        &BasicErrorResponseType::Extension("invalid_code".to_string())
    );
    assert_eq!(
        error.to_string(),
        "invalid_code: Slack token endpoint returned ok=false"
    );
}

#[test]
fn slack_ok_false_envelope_without_an_error_still_maps_to_an_oauth_error() {
    let response = normalize_slack_token_response(slack_response(r#"{"ok": false}"#));

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = parse_error_response(&response);
    assert_eq!(
        error.error(),
        &BasicErrorResponseType::Extension("slack_api_error".to_string())
    );
}

#[test]
fn rfc_compliant_token_response_is_left_untouched() {
    let body = r#"{"access_token":"rfc-access-token","token_type":"Bearer","expires_in":3600,"refresh_token":"rfc-refresh-token","scope":"mcp:read mcp:write"}"#;

    let response = normalize_slack_token_response(slack_response(body));

    // Byte identity is the invariant that matters: standards-compliant providers must reach
    // `oauth2` exactly as their server sent them.
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().as_slice(), body.as_bytes());
    let token = parse_token_response(&response);
    assert_eq!(token.access_token().secret(), "rfc-access-token");
    assert_eq!(token.token_type(), &BasicTokenType::Bearer);
    assert_eq!(token.expires_in(), Some(Duration::from_secs(3600)));
    assert_eq!(
        token.scopes().map(|scopes| scopes
            .iter()
            .map(|scope| scope.to_string())
            .collect::<Vec<_>>()),
        Some(vec!["mcp:read".to_string(), "mcp:write".to_string()])
    );
}

#[test]
fn rfc_compliant_error_response_is_left_untouched() {
    let body = r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#;
    let original = Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(CONTENT_TYPE, "application/json")
        .body(body.as_bytes().to_vec())
        .expect("build response");

    let response = normalize_slack_token_response(original);

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.body().as_slice(), body.as_bytes());
}

#[test]
fn slack_ok_true_envelope_without_any_access_token_is_left_untouched() {
    let body = r#"{"ok": true, "team": {"id": "T0000000000"}}"#;

    let response = normalize_slack_token_response(slack_response(body));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().as_slice(), body.as_bytes());
}

#[test]
fn non_json_and_non_envelope_bodies_are_left_untouched() {
    for body in [
        "not json at all",
        r#"{"issuer":"https://example.com","token_endpoint":"https://example.com/token"}"#,
        r#"{"ok":"true","access_token":"string-ok-is-not-an-envelope"}"#,
    ] {
        let response = normalize_slack_token_response(slack_response(body));
        assert_eq!(response.body().as_slice(), body.as_bytes());
        assert_eq!(response.status(), StatusCode::OK);
    }
}
