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
pub(crate) use linux::{Handle, Setup};

#[cfg(not(target_os = "linux"))]
pub(crate) fn status(requested: bool) -> ObservationStatus {
    platform::status(requested)
}

pub(crate) fn unsupported_status(requested: bool) -> ObservationStatus {
    unsupported::status(requested)
}
