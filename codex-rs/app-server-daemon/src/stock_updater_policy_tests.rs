use super::enforce_stock_updater_disabled;
use crate::settings::DaemonSettings;

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
