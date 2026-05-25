use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub trait ShutdownAccess: Send + Sync {
    fn shutdown_flag(&self) -> Arc<AtomicBool>;
}

pub trait PathsAccess: Send + Sync {
    fn cache_dir(&self) -> PathBuf;
    fn config_dir(&self) -> PathBuf;
    fn workspace_folders(&self) -> Vec<PathBuf>;
}

#[cfg(feature = "http-client")]
pub trait HttpClientAccess: Send + Sync {
    fn http_client(&self) -> reqwest::Client;
}

pub const PRIVACY_ACCESS_FOLLOW_UP: &str =
    "PrivacyAccess is left for follow-up because PrivacySettings still lives in the engine crate.";
pub const CAPS_ACCESS_FOLLOW_UP: &str =
    "CapsAccess is left for follow-up because CodeAssistantCaps still lives in the engine crate.";

pub fn omitted_traits_follow_up_notes() -> [&'static str; 2] {
    [PRIVACY_ACCESS_FOLLOW_UP, CAPS_ACCESS_FOLLOW_UP]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_up_notes_document_omitted_traits() {
        let notes = omitted_traits_follow_up_notes();

        assert!(notes.iter().any(|note| note.contains("PrivacyAccess")));
        assert!(notes.iter().any(|note| note.contains("CapsAccess")));
    }
}
