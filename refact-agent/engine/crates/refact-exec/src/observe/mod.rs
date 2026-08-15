use std::path::PathBuf;

mod unsupported;

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
    unsupported::status(requested)
}
