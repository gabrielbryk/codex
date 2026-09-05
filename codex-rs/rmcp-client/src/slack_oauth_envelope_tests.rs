use super::SlackOAuthResponseAdapter;
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
use url::Url;

fn slack_adapter() -> SlackOAuthResponseAdapter {
    SlackOAuthResponseAdapter::from_token_endpoint(
        &Url::parse("https://slack.com/api/oauth.v2.user.access").expect("valid Slack URL"),
    )
    .expect("recognized Slack token endpoint")
}

fn response(body: &str) -> HttpResponse {
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
fn known_slack_token_endpoints_create_an_adapter() {
    for endpoint in [
        "https://slack.com/api/oauth.v2.access",
        "https://slack.com/api/oauth.v2.user.access",
        "https://slack.com:443/api/oauth.v2.user.access",
    ] {
        assert!(
            SlackOAuthResponseAdapter::from_token_endpoint(
                &Url::parse(endpoint).expect("valid URL")
            )
            .is_some(),
            "endpoint should identify Slack: {endpoint}"
        );
    }
}

#[test]
fn non_slack_lookalike_endpoints_cannot_create_an_adapter() {
    for endpoint in [
        "https://example.com/api/oauth.v2.user.access",
        "https://slack.com.evil.example/api/oauth.v2.user.access",
        "http://slack.com/api/oauth.v2.user.access",
        "https://user@slack.com/api/oauth.v2.user.access",
        "https://slack.com/api/oauth.v2.user.access?provider=slack",
        "https://slack.com/api/oauth.v2.user.access/extra",
    ] {
        assert!(
            SlackOAuthResponseAdapter::from_token_endpoint(
                &Url::parse(endpoint).expect("valid URL")
            )
            .is_none(),
            "endpoint must not identify Slack: {endpoint}"
        );
    }
}

#[test]
fn slack_ok_true_envelope_becomes_a_standard_token_response() {
    let response = slack_adapter().normalize_response(response(
        r#"{
            "ok": true,
            "access_token": "xoxp-new-access-token",
            "token_type": "user",
            "refresh_token": "xoxe-1-new-refresh-token",
            "expires_in": 43200,
            "scope": "channels:read,chat:write,search:read"
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
        token.scopes().map(|scopes| {
            scopes
                .iter()
                .map(|scope| scope.as_ref().to_string())
                .collect::<Vec<_>>()
        }),
        Some(vec![
            "channels:read".to_string(),
            "chat:write".to_string(),
            "search:read".to_string(),
        ])
    );
}

#[test]
fn slack_nested_user_token_is_normalized() {
    let response = slack_adapter().normalize_response(response(
        r#"{
            "ok": true,
            "authed_user": {
                "access_token": "xoxp-nested-access-token",
                "token_type": "user",
                "refresh_token": "xoxe-1-nested-refresh-token",
                "expires_in": 43200,
                "scope": "channels:read,chat:write"
            }
        }"#,
    ));

    let token = parse_token_response(&response);
    assert_eq!(token.access_token().secret(), "xoxp-nested-access-token");
    assert_eq!(
        token.refresh_token().map(|token| token.secret().as_str()),
        Some("xoxe-1-nested-refresh-token")
    );
}

#[test]
fn slack_ok_false_envelope_becomes_an_oauth_error() {
    let response =
        slack_adapter().normalize_response(response(r#"{"ok": false, "error": "invalid_code"}"#));

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
fn generic_responses_remain_byte_identical() {
    for body in [
        r#"{"access_token":"rfc-token","token_type":"Bearer"}"#,
        r#"{"ok": true, "team": {"id": "T0000000000"}}"#,
        r#"{"ok":"true","access_token":"string-ok-is-not-an-envelope"}"#,
        "not json at all",
    ] {
        let response = slack_adapter().normalize_response(response(body));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body().as_slice(), body.as_bytes());
    }
}

#[test]
fn non_slack_lookalike_envelope_has_no_normalization_path() {
    let endpoint = Url::parse("https://example.com/api/oauth.v2.user.access").expect("valid URL");
    let body = r#"{"ok": false, "error": "provider_specific_error"}"#;
    let response = response(body);

    let normalized = SlackOAuthResponseAdapter::from_token_endpoint(&endpoint)
        .map(|adapter| adapter.normalize_response(response.clone()))
        .unwrap_or_else(|| response.clone());

    assert_eq!(
        (normalized.status(), normalized.headers(), normalized.body(),),
        (response.status(), response.headers(), response.body())
    );
}
