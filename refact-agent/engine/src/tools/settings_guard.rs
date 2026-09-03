use crate::runtime_settings::TrajectoryRuntimeSettings;

pub struct SettingsGuard;

impl SettingsGuard {
    pub fn install(mutate: impl FnOnce(&mut TrajectoryRuntimeSettings)) -> Self {
        let mut settings = TrajectoryRuntimeSettings::default();
        mutate(&mut settings);
        crate::runtime_settings::install_live(&settings);
        Self
    }
}

impl Drop for SettingsGuard {
    fn drop(&mut self) {
        crate::runtime_settings::reset_for_test();
    }
}
