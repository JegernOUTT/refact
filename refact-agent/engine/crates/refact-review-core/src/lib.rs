mod handoff;
mod intake;
pub use intake::validate_context_intake;
mod reconcile;
pub mod rich_blocks;
mod snapshot;
mod stacks;
mod store;
pub mod types;
pub use store::ReviewStore;
pub use types::*;
use sha2::{Digest, Sha256};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub enum Error {
    Integrity(String),
    NonText,
    Invalid(String),
    NotFound,
    Conflict { expected: u64, actual: u64 },
    IdempotencyConflict,
    Limit(String),
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    Git(git2::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<git2::Error> for Error {
    fn from(e: git2::Error) -> Self {
        Self::Git(e)
    }
}
pub fn content_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CANDIDATE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_FILES: usize = 20_000;
pub(crate) fn invalid(s: &str) -> Error {
    Error::Invalid(s.to_owned())
}
pub(crate) fn relative(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains('\0')
        || path.split('/').any(|c| {
            c.is_empty()
                || c == "."
                || c == ".."
                || c.eq_ignore_ascii_case(".git")
                || c.eq_ignore_ascii_case(".refact")
        })
        || path.contains(':')
    {
        return Err(invalid("unsafe relative path"));
    }
    Ok(())
}
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
