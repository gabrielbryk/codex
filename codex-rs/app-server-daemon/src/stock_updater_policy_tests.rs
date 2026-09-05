use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::JSONRPCResponse;
use codex_uds::UnixListener;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio_tungstenite::accept_async;

use crate::BackendKind;
use crate::BootstrapOptions;
use crate::BootstrapOutput;
use crate::BootstrapStatus;
use crate::Daemon;
use crate::LifecycleOutput;
use crate::LifecycleStatus;
use crate::RemoteControlStartOutput;
use crate::backend;
use crate::settings::DaemonSettings;

#[tokio::test]
async fn active_legacy_updater_blocks_bootstrap_and_remote_ensure() -> Result<()> {
    let temp_dir = TempDir::new().expect("temp dir");
    let daemon = test_daemon(&temp_dir);
    write_codex_shim(&daemon.managed_codex_bin).await?;
    let settings = DaemonSettings::default();
    let updater = backend::pid_update_loop_backend(daemon.backend_paths(&settings));
    updater.start().await.expect("start legacy updater");

    let bootstrap_error = daemon
        .bootstrap_locked(BootstrapOptions {
            remote_control_enabled: true,
        })
        .await
        .expect_err("active updater must block bootstrap");
    let ensure_error = daemon
        .ensure_remote_control_started()
        .await
        .expect_err("active updater must block remote ensure");

    let expected = "legacy app-server updater is still running; stop it before completing the stock-updater cutover";
    assert_eq!(bootstrap_error.to_string(), expected);
    assert_eq!(ensure_error.to_string(), expected);
    assert!(
        updater
            .is_starting_or_running()
            .await
            .expect("inspect updater")
    );
    updater.stop().await.expect("stop legacy updater");
    Ok(())
}

#[tokio::test]
async fn bootstrap_replaces_mismatched_backend_and_remote_ensure_is_idempotent() -> Result<()> {
    let temp_dir = TempDir::new().expect("temp dir");
    let daemon = test_daemon(&temp_dir);
    let old_codex_bin = temp_dir.path().join("old-codex");
    write_codex_shim(&old_codex_bin).await?;
    write_codex_shim(&daemon.managed_codex_bin).await?;

    let old_settings = DaemonSettings {
        remote_control_enabled: false,
    };
    let old_backend =
        backend::pid_backend(daemon.backend_paths_with_bin(&old_settings, &old_codex_bin));
    let old_pid = old_backend
        .start()
        .await
        .expect("start mismatched backend")
        .expect("new backend pid");
    let settings = DaemonSettings {
        remote_control_enabled: true,
    };
    let listener = UnixListener::bind(&daemon.socket_path)
        .await
        .expect("bind test app-server");
    let server = tokio::spawn(serve_test_app_server(listener));

    let output = daemon
        .bootstrap_locked(BootstrapOptions {
            remote_control_enabled: true,
        })
        .await
        .expect("bootstrap");
    let bootstrap_pid = read_pid(&daemon.pid_file).await?;
    let first_ensure = daemon
        .ensure_remote_control_started()
        .await
        .expect("first remote ensure");
    let first_ensure_pid = read_pid(&daemon.pid_file).await?;
    let second_ensure = daemon
        .ensure_remote_control_started()
        .await
        .expect("second remote ensure");
    let second_ensure_pid = read_pid(&daemon.pid_file).await?;

    let updater = backend::pid_update_loop_backend(daemon.backend_paths(&settings));
    assert_eq!(
        output,
        BootstrapOutput {
            status: BootstrapStatus::Bootstrapped,
            backend: BackendKind::Pid,
            auto_update_enabled: false,
            remote_control_enabled: true,
            managed_codex_path: daemon.managed_codex_bin.clone(),
            managed_codex_version: Some("1.2.3".to_string()),
            socket_path: daemon.socket_path.clone(),
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            app_server_version: "1.2.3".to_string(),
        }
    );
    assert_ne!(bootstrap_pid, u64::from(old_pid));
    assert_eq!(
        [bootstrap_pid, first_ensure_pid, second_ensure_pid],
        [bootstrap_pid; 3]
    );
    assert_eq!(first_ensure, second_ensure);
    assert_eq!(
        first_ensure,
        RemoteControlStartOutput::Start(LifecycleOutput {
            status: LifecycleStatus::AlreadyRunning,
            backend: Some(BackendKind::Pid),
            pid: None,
            managed_codex_path: daemon.managed_codex_bin.clone(),
            managed_codex_version: Some("1.2.3".to_string()),
            socket_path: daemon.socket_path.clone(),
            cli_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            app_server_version: Some("1.2.3".to_string()),
        })
    );
    assert!(
        !updater
            .is_starting_or_running()
            .await
            .expect("inspect updater")
    );

    backend::pid_backend(daemon.backend_paths(&settings))
        .stop()
        .await
        .expect("stop replacement backend");
    server.abort();
    let _ = server.await;
    Ok(())
}

async fn write_codex_shim(path: &Path) -> Result<()> {
    tokio::fs::write(
        path,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'codex-cli 1.2.3'; exit 0; fi\nexec /bin/sleep 30\n",
    )
    .await?;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await?;
    Ok(())
}

async fn read_pid(pid_file: &Path) -> Result<u64> {
    let record: serde_json::Value = serde_json::from_slice(&tokio::fs::read(pid_file).await?)?;
    record["pid"].as_u64().context("pid")
}

async fn serve_test_app_server(mut listener: UnixListener) -> Result<()> {
    loop {
        let stream = listener.accept().await?;
        let mut websocket = accept_async(stream).await?;
        while let Ok(message) = crate::client::read_message(&mut websocket).await {
            let JSONRPCMessage::Request(request) = message else {
                continue;
            };
            let result = match request.method.as_str() {
                "initialize" => serde_json::json!({
                    "userAgent": "codex_app_server/1.2.3",
                    "codexHome": "/tmp/codex-home",
                    "platformFamily": "unix",
                    "platformOs": "linux",
                }),
                "remoteControl/enable" => serde_json::json!({
                    "status": "connected",
                    "serverName": "test-server",
                    "installationId": "11111111-1111-4111-8111-111111111111",
                    "environmentId": null,
                }),
                method => panic!("unexpected test app-server request: {method}"),
            };
            crate::client::send_message(
                &mut websocket,
                &JSONRPCMessage::Response(JSONRPCResponse {
                    id: request.id,
                    result,
                }),
            )
            .await?;
        }
    }
}

fn test_daemon(temp_dir: &TempDir) -> Daemon {
    Daemon {
        socket_path: temp_dir.path().join("app-server-control.sock"),
        pid_file: temp_dir.path().join("app-server.pid"),
        update_pid_file: temp_dir.path().join("app-server-updater.pid"),
        operation_lock_file: temp_dir.path().join("daemon.lock"),
        settings_file: temp_dir.path().join("settings.json"),
        managed_codex_bin: temp_dir.path().join("codex"),
    }
}
