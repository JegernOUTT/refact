use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PathDenyRules {
    pub prefixes: Vec<std::path::PathBuf>,
    pub files: Vec<std::path::PathBuf>,
    #[serde(default)]
    pub allow_prefixes: Vec<std::path::PathBuf>,
}

pub const MAX_STDOUT_BYTES: usize = 1024 * 1024;
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KernelCommand {
    Configure {
        project_root: std::path::PathBuf,
        env: std::collections::BTreeMap<String, String>,
        capture_dir: Option<std::path::PathBuf>,
        #[serde(default)]
        deny_write: PathDenyRules,
        #[serde(default)]
        deny_read: PathDenyRules,
    },
    Feed {
        run_id: String,
        code: String,
        globals: Vec<GlobalBinding>,
        limits: RunLimits,
    },
    Results {
        run_id: String,
        results: Vec<(u32, HostResult)>,
    },
    SyncResult {
        call_id: u32,
        result: HostResult,
    },
    Abort {
        run_id: String,
    },
    Snapshot {
        path: String,
    },
    LoadSnapshot {
        #[serde(default = "default_state_bytes")]
        max_state_bytes: u64,
        expected_sha256: String,
        path: String,
    },
    Reset,
    Inspect,
    Shutdown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalBinding {
    pub name: String,
    pub kind: GlobalKind,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GlobalKind {
    Json(Value),
    AsyncFn,
    /// Async function whose leading positional arguments map, in order, to these keyword names.
    AsyncFnPositional(Vec<String>),
    SyncFn,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostResult {
    Ok(Value),
    Err { exc_type: String, message: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLimits {
    pub max_memory_bytes: u64,
    pub max_compute_ms: u64,
    #[serde(default = "default_value_nodes")]
    pub max_value_nodes: u32,
    #[serde(default = "default_value_bytes")]
    pub max_value_bytes: u64,
    #[serde(default = "default_arg_bytes")]
    pub max_arg_bytes: u64,
    /// Conversion budget of the arguments of `show()` (see [`DISPLAY_FUNCTION`]).
    #[serde(default = "default_display_nodes")]
    pub max_display_nodes: u32,
    #[serde(default = "default_display_bytes")]
    pub max_display_bytes: u64,
    /// Budget of one host result (async call, sync helper, JSON global) entering a script.
    /// A result over it raises RuntimeError at the call; it is never cut.
    #[serde(default = "default_result_nodes")]
    pub max_result_nodes: u32,
    #[serde(default = "default_result_bytes")]
    pub max_result_bytes: u64,
}
/// Synchronous helper whose arguments use the display conversion budget.
pub const DISPLAY_FUNCTION: &str = "show";
fn default_result_bytes() -> u64 {
    16 * 1024 * 1024
}
fn default_result_nodes() -> u32 {
    crate::convert::result_nodes_for(default_result_bytes())
}
fn default_display_nodes() -> u32 {
    1_000_000
}
fn default_display_bytes() -> u64 {
    32 * 1024 * 1024
}
fn default_value_nodes() -> u32 {
    200_000
}
fn default_value_bytes() -> u64 {
    2 * 1024 * 1024
}
fn default_arg_bytes() -> u64 {
    16 * 1024 * 1024
}
impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: 128 * 1024 * 1024,
            max_compute_ms: 1000,
            max_value_nodes: default_value_nodes(),
            max_value_bytes: default_value_bytes(),
            max_arg_bytes: default_arg_bytes(),
            max_display_nodes: default_display_nodes(),
            max_display_bytes: default_display_bytes(),
            max_result_nodes: default_result_nodes(),
            max_result_bytes: default_result_bytes(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LimitKind {
    Memory,
    Compute,
    Output,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableSummary {
    pub name: String,
    pub type_name: String,
    pub preview: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KernelEvent {
    ProtocolError {
        message: String,
    },
    Ready {
        monty_version: String,
    },
    CallStarted {
        truncated: bool,
        run_id: String,
        call_id: u32,
        function: String,
        kwargs: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        col: Option<u32>,
    },
    SyncCall {
        truncated: bool,
        run_id: String,
        call_id: u32,
        function: String,
        args: Value,
        /// Keyword arguments other than the `name`/`p` ones folded into `args`.
        #[serde(default, skip_serializing_if = "is_empty_object")]
        kwargs: Value,
    },
    NeedResults {
        run_id: String,
        pending: Vec<u32>,
    },
    Stdout {
        run_id: String,
        text: String,
    },
    Fs {
        run_id: String,
        op: String,
        path: String,
        bytes: u64,
    },
    Renamed {
        run_id: String,
        src: String,
        dst: String,
    },
    WriteCapture {
        run_id: String,
        path: String,
        original: WriteOriginal,
    },
    Completed {
        truncated: bool,
        run_id: String,
        value: Value,
        repr: String,
    },
    Raised {
        run_id: String,
        exc_type: String,
        message: String,
        traceback: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
    },
    LimitExceeded {
        run_id: String,
        kind: LimitKind,
        message: String,
    },
    Cancelled {
        run_id: String,
    },
    SnapshotDone {
        bytes: u64,
        sha256: String,
    },
    SnapshotFailed {
        message: String,
    },
    Loaded,
    LoadFailed {
        message: String,
    },
    ResetDone,
    Variables {
        items: Vec<VariableSummary>,
        /// The list itself was capped; see `total`.
        truncated: bool,
        #[serde(default)]
        total: Option<u64>,
        /// At least one preview was shortened.
        #[serde(default)]
        previews_truncated: bool,
    },
}
pub fn write_frame<W: Write, T: Serialize>(w: &mut W, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}
pub fn read_frame<R: Read, T: serde::de::DeserializeOwned>(r: &mut R) -> io::Result<T> {
    let mut n = [0; 4];
    r.read_exact(&mut n)?;
    let n = u32::from_le_bytes(n) as usize;
    if n > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut bytes = vec![0; n];
    r.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames() {
        let mut b = vec![];
        write_frame(&mut b, &KernelCommand::Reset).unwrap();
        assert!(matches!(
            read_frame(&mut &b[..]).unwrap(),
            KernelCommand::Reset
        ));
        assert!(read_frame::<_, KernelCommand>(&mut &u32::MAX.to_le_bytes()[..]).is_err());
        assert!(read_frame::<_, KernelCommand>(&mut &b[..3]).is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteOriginal {
    Missing,
    File { capture_path: String },
    TooLarge,
}

fn is_empty_object(value: &Value) -> bool {
    value.is_null() || value.as_object().is_some_and(|o| o.is_empty())
}

fn default_state_bytes() -> u64 {
    1024 * 1024 * 1024
}
