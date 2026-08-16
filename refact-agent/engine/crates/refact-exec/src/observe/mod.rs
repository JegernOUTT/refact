use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(test, target_os = "macos"))]
mod macos;
mod unsupported;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use unsupported as platform;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedAccess {
    pub reads: Vec<PathBuf>,
    pub writes: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationStatus {
    Observed(ObservedAccess),
    Unavailable(String),
}

#[cfg(target_os = "linux")]
pub use linux::{Runtime as ObservationRuntime, Setup as ObservationSetup};
#[cfg(target_os = "linux")]
pub(crate) use linux::{Handle, Setup};

#[derive(Clone)]
pub struct ObservationReader {
    #[cfg(target_os = "linux")]
    handle: Option<linux::Handle>,
    unavailable: Option<String>,
}

impl ObservationReader {
    #[cfg(target_os = "linux")]
    fn active(handle: linux::Handle) -> Self {
        Self {
            handle: Some(handle),
            unavailable: None,
        }
    }

    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            #[cfg(target_os = "linux")]
            handle: None,
            unavailable: Some(reason.into()),
        }
    }

    pub fn status(&self) -> ObservationStatus {
        #[cfg(target_os = "linux")]
        if let Some(handle) = &self.handle {
            return handle.status();
        }
        ObservationStatus::Unavailable(
            self.unavailable
                .clone()
                .unwrap_or_else(|| "backend unavailable".to_string()),
        )
    }

    pub async fn wait_status(&self) -> ObservationStatus {
        #[cfg(target_os = "linux")]
        if let Some(handle) = &self.handle {
            return handle.wait_status().await;
        }
        self.status()
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn status(requested: bool) -> ObservationStatus {
    platform::status(requested)
}

pub(crate) fn unsupported_status(requested: bool) -> ObservationStatus {
    unsupported::status(requested)
}
