use std::error::Error;
use serde::Serialize;
use hyper::StatusCode;
use serde_json::json;
use std::fmt;
use axum::Json;
use axum::response::IntoResponse;

#[derive(Debug, Clone)]
pub struct ScratchError {
    pub status_code: StatusCode,
    pub message: String,
}

impl IntoResponse for ScratchError {
    fn into_response(self) -> axum::response::Response {
        let payload = json!({
            "detail": self.message,
        });
        let mut response = (self.status_code, Json(payload)).into_response();
        response.extensions_mut().insert(self);
        response
    }
}

impl Error for ScratchError {}
unsafe impl Send for ScratchError {}
unsafe impl Sync for ScratchError {}
impl fmt::Display for ScratchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.status_code, self.message)
    }
}

impl ScratchError {
    pub fn new(status_code: StatusCode, message: String) -> Self {
        ScratchError {
            status_code,
            message,
        }
    }
}

#[derive(Serialize, Default)]
pub struct YamlError {
    pub path: String,
    pub error_line: usize,
    pub error_msg: String,
}

impl From<(&str, &serde_yaml::Error)> for YamlError {
    fn from((path, err): (&str, &serde_yaml::Error)) -> Self {
        YamlError {
            path: path.to_string(),
            error_line: err.location().map(|loc| loc.line()).unwrap_or(0),
            error_msg: err.to_string(),
        }
    }
}

pub fn last_n_chars(msg: &str, n: usize) -> String {
    let mut result: String = msg
        .chars()
        .rev()
        .take(n)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if result.len() == n {
        result.insert_str(0, "...");
    }
    result.replace("\n", "\\n")
}

pub fn first_n_chars(msg: &str, n: usize) -> String {
    let mut result: String = msg.chars().take(n).collect();
    if result.chars().count() == n {
        result.push_str("...");
    }
    result.replace("\n", "\\n")
}

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{} {:?}",
            last_n_chars(&self.path, 40),
            self.error_line,
            self.error_msg
        )
    }
}

pub trait MapErrToString<T> {
    fn map_err_to_string(self) -> Result<T, String>;
    fn map_err_with_prefix<P: std::fmt::Display>(self, pref: P) -> Result<T, String>;
}

impl<T, E: std::fmt::Display> MapErrToString<T> for Result<T, E> {
    fn map_err_to_string(self) -> Result<T, String> {
        self.map_err(|e| e.to_string())
    }

    fn map_err_with_prefix<P: std::fmt::Display>(self, pref: P) -> Result<T, String> {
        self.map_err(|e| format!("{pref} {e}"))
    }
}

pub fn trace_and_default<T: std::default::Default, E: std::fmt::Display>(e: E) -> T {
    tracing::error!("{e}");
    Default::default()
}
