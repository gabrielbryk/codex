//! Defense-in-depth policy that keeps the stock updater from ever starting
//! for this locally managed fork build, independent of whatever a per-home
//! `settings.json` says.
//!
//! Upstream's own `Daemon::ensure_managed_updater` already refuses to start
//! the update loop when `DaemonSettings::auto_update_enabled` is `false`, so
//! forcing that field off during bootstrap is enough to guarantee the
//! managed updater is never started for a home this build owns, without
//! depending on every home's `settings.json` being provisioned with
//! `updater.autoUpdateEnabled: false` ahead of time.

use crate::settings::DaemonSettings;

/// Force auto-update off for this bootstrap, regardless of the loaded
/// per-home settings. Call this before `ensure_managed_updater` runs so the
/// gate it already applies (`!settings.auto_update_enabled`) sees a settings
/// value that can never start the stock updater, and so the reported
/// `auto_update_enabled` in `BootstrapOutput` truthfully reflects that.
pub(crate) fn enforce_stock_updater_disabled(settings: &mut DaemonSettings) {
    settings.auto_update_enabled = false;
}

#[cfg(test)]
#[path = "stock_updater_policy_tests.rs"]
mod tests;
