use anyhow::Result;
use anyhow::anyhow;

use crate::Daemon;
use crate::backend;
use crate::client;
use crate::settings::DaemonSettings;

impl Daemon {
    pub(super) async fn restart_backend_for_bootstrap(
        &self,
        settings: &DaemonSettings,
    ) -> Result<()> {
        if let Some(backend) = self.running_backend_instance(settings).await? {
            backend.stop().await?;
        }
        backend::pid_backend(self.backend_paths(settings))
            .start()
            .await?;
        Ok(())
    }

    pub(super) async fn ensure_updater_stopped_for_cutover(
        &self,
        settings: &DaemonSettings,
    ) -> Result<()> {
        let updater = backend::pid_update_loop_backend(self.backend_paths(settings));
        if updater.is_starting_or_running().await? {
            return Err(anyhow!(
                "legacy app-server updater is still running; stop it before completing the stock-updater cutover"
            ));
        }
        Ok(())
    }

    pub(super) async fn is_bootstrapped(&self, settings: &DaemonSettings) -> Result<bool> {
        if !settings.remote_control_enabled
            || self.running_backend_instance(settings).await?.is_none()
        {
            return Ok(false);
        }
        Ok(client::probe(&self.socket_path).await.is_ok())
    }
}

#[cfg(all(test, unix))]
#[path = "stock_updater_policy_tests.rs"]
mod tests;
