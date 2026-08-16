use std::path::PathBuf;

#[cfg(any(test, target_os = "macos"))]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(target_os = "macos"))]
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

pub(crate) fn status(requested: bool) -> ObservationStatus {
    platform::status(requested)
}
