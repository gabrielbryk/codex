use super::*;

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[cfg(unix)]
#[test]
fn normalizes_absolute_posix_paths() {
    assert_eq!(
        normalized_posix_cwd(Path::new("/work/./project/../repo")).unwrap(),
        "/work/repo"
    );
}

#[cfg(not(unix))]
#[test]
fn rejects_posix_paths_without_a_platform_absolute_prefix() {
    assert!(normalized_posix_cwd(Path::new("/work/project")).is_err());
}

#[test]
fn rejects_relative_non_utf8_and_control_cwds() {
    assert!(normalized_posix_cwd(Path::new("relative/path")).is_err());
    assert!(normalized_posix_cwd(Path::new("/work/\u{7f}repo")).is_err());
    #[cfg(unix)]
    assert!(normalized_posix_cwd(Path::new(std::ffi::OsStr::from_bytes(b"/work/\xff"))).is_err());
}

#[test]
fn inserts_a_signed_short_lived_claim_header() {
    let signer = RouteClaimSigner {
        key: b"route-claim-signing-key-at-least-32".to_vec(),
        audience: "codex-cwd-proxy".to_string(),
        thread_id: "thread-123".to_string(),
        cwd: "/work/project".to_string(),
    };
    let mut headers = HeaderMap::new();
    signer.insert_header(&mut headers);
    let claim = headers[ROUTE_CLAIM_HEADER].to_str().unwrap();
    let [version, payload, signature]: [&str; 3] =
        claim.split('.').collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(version, "v1");
    let signed = format!("{version}.{payload}");
    let mut mac = Hmac::<Sha256>::new_from_slice(&signer.key).unwrap();
    mac.update(signed.as_bytes());
    assert_eq!(
        URL_SAFE_NO_PAD.decode(signature).unwrap(),
        mac.finalize().into_bytes().as_slice()
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    assert_eq!(payload["aud"], signer.audience);
    assert_eq!(payload["thread_id"], signer.thread_id);
    assert_eq!(payload["cwd"], signer.cwd);
    assert!(payload["exp"].as_u64().unwrap() - payload["iat"].as_u64().unwrap() <= 300);
}

#[test]
fn replaces_the_claim_cwd_when_session_settings_change() {
    let signer = RouteClaimSigner {
        key: b"route-claim-signing-key-at-least-32".to_vec(),
        audience: "codex-cwd-proxy".to_string(),
        thread_id: "thread-123".to_string(),
        cwd: "/personal/project".to_string(),
    };
    assert_eq!(
        signer.with_cwd(Path::new("/work/project")).unwrap().cwd,
        "/work/project"
    );
}
