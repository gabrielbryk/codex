use super::enforce_stock_updater_disabled;
use crate::Daemon;
use crate::settings::DaemonSettings;
use tempfile::TempDir;

#[test]
fn forces_auto_update_off_when_settings_enable_it() {
    let mut settings = DaemonSettings {
        auto_update_enabled: true,
        ..DaemonSettings::default()
    };

    enforce_stock_updater_disabled(&mut settings);

    assert!(!settings.auto_update_enabled);
}

#[test]
fn keeps_auto_update_off_when_settings_already_disable_it() {
    let mut settings = DaemonSettings {
        auto_update_enabled: false,
        ..DaemonSettings::default()
    };

    enforce_stock_updater_disabled(&mut settings);

    assert!(!settings.auto_update_enabled);
}

#[test]
fn leaves_other_settings_fields_untouched() {
    let mut settings = DaemonSettings {
        remote_control_enabled: true,
        auto_update_enabled: true,
        update_interval_minutes: 42,
        shutdown_grace_seconds: 99,
    };

    enforce_stock_updater_disabled(&mut settings);

    assert!(settings.remote_control_enabled);
    assert!(!settings.auto_update_enabled);
    assert_eq!(settings.update_interval_minutes, 42);
    assert_eq!(settings.shutdown_grace_seconds, 99);
}

/// Regression test for the choke-point fix: `ensure_managed_updater` must enforce
/// the disabled policy itself, so any caller reaching it with settings loaded
/// straight from `settings.json` (auto_update_enabled = true) is still refused.
#[tokio::test]
async fn ensure_managed_updater_refuses_to_start_even_when_settings_enable_it() {
    let home = TempDir::new().expect("home");
    let state = home.path().join("app-server-daemon");
    let daemon = Daemon {
        socket_path: home.path().join("server.sock"),
        pid_file: state.join("server.pid"),
        update_pid_file: state.join("updater.pid"),
        operation_lock_file: state.join("daemon.lock"),
        settings_file: state.join("settings.json"),
        managed_codex_bin: home.path().join("codex"),
    };
    let settings = DaemonSettings {
        auto_update_enabled: true,
        ..DaemonSettings::default()
    };

    let started = daemon
        .ensure_managed_updater(&settings)
        .await
        .expect("ensure_managed_updater");

    assert!(
        !started,
        "the managed updater must never start, regardless of settings.json"
    );
}
