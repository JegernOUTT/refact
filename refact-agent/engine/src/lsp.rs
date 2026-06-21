use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::Ordering;
use std::path::PathBuf;
use std::sync::Arc;
use std::io::Write;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};
use tower_lsp::jsonrpc::{Error, ErrorCode, Result};
use tower_lsp::lsp_types::*;
use tower_lsp::{ClientSocket, LanguageServer, LspService};
use tracing::{error, info};

use crate::call_validation::{
    CodeCompletionInputs, CodeCompletionPost, CursorPosition, SamplingParameters,
};
use crate::files_in_workspace;
use crate::files_in_workspace::{on_did_change, on_did_delete};
use crate::app_state::AppState;
use crate::global_context::{CommandLine, GlobalContext};
use crate::http::routers::v1::code_completion::handle_v1_code_completion;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LspTransport {
    Tcp,
    Stdio,
}

pub struct LspBackend {
    pub gcx: Arc<GlobalContext>,
    pub client: tower_lsp::Client,
    transport: LspTransport,
}

struct LspTcpClientGuard {
    gcx: Arc<GlobalContext>,
}

impl LspTcpClientGuard {
    fn new(gcx: Arc<GlobalContext>) -> Self {
        gcx.lsp_tcp_client_count.fetch_add(1, Ordering::SeqCst);
        Self { gcx }
    }
}

impl Drop for LspTcpClientGuard {
    fn drop(&mut self) {
        self.gcx.lsp_tcp_client_count.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RequestParams {
    pub max_new_tokens: u32,
    pub temperature: f32,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub no_cache: bool,
    #[serde(default)]
    pub use_ast: bool,
    #[serde(default)]
    pub use_vecdb: bool,
    #[serde(default)]
    pub rag_tokens_n: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CompletionParams1 {
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    pub parameters: RequestParams,
    pub multiline: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChangeActiveFile {
    pub uri: Url,
}

fn internal_error<E: Display>(err: E) -> Error {
    let err_msg = err.to_string();
    error!(err_msg);
    Error {
        code: ErrorCode::InternalError,
        message: err_msg.into(),
        data: None,
    }
}

fn invalid_params<E: Display>(err: E) -> Error {
    let err_msg = err.to_string();
    Error {
        code: ErrorCode::InvalidParams,
        message: err_msg.into(),
        data: None,
    }
}

async fn notify_workspace_changed(gcx: &Arc<GlobalContext>) {
    let tx = gcx.workspace_changed_tx.clone();
    if let Some(tx) = tx {
        let _ = tx.send(());
    }
}

pub(crate) fn canonical_workspace_roots(folders: &[PathBuf]) -> Vec<PathBuf> {
    let mut canonical = folders.to_vec();
    canonical.sort();
    canonical.dedup();
    canonical
}

pub(crate) fn workspace_roots_changed(current: &[PathBuf], next: &[PathBuf]) -> bool {
    canonical_workspace_roots(current) != canonical_workspace_roots(next)
}

pub(crate) fn canonical_path_from_file_uri(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path()
        .ok()
        .map(|path| crate::files_correction::canonical_path(path.to_string_lossy().into_owned()))
}

fn canonical_path_from_file_uri_required(uri: &Url, field: &str) -> Result<PathBuf> {
    canonical_path_from_file_uri(uri)
        .ok_or_else(|| invalid_params(format!("{field} must be a file URI: {uri}")))
}

fn canonical_path_from_file_uri_for_notification(handler: &str, uri: &Url) -> Option<PathBuf> {
    let path = canonical_path_from_file_uri(uri);
    if path.is_none() {
        info!("{handler} ignored non-file URI {uri}");
    }
    path
}

pub(crate) fn add_workspace_root_to_set(folders: &mut Vec<PathBuf>, path: PathBuf) -> bool {
    let before = canonical_workspace_roots(folders);
    let canonical_path =
        crate::files_correction::canonical_path(path.to_string_lossy().into_owned());
    let mut next = before.clone();
    next.push(canonical_path);
    let next = canonical_workspace_roots(&next);
    *folders = next.clone();
    before != next
}

pub(crate) fn remove_workspace_root_from_set(folders: &mut Vec<PathBuf>, path: &PathBuf) -> bool {
    let before = canonical_workspace_roots(folders);
    let canonical_path =
        crate::files_correction::canonical_path(path.to_string_lossy().into_owned());
    let next = before
        .iter()
        .filter(|folder| *folder != &canonical_path)
        .cloned()
        .collect::<Vec<_>>();
    *folders = next.clone();
    before != next
}

pub(crate) fn apply_workspace_root_changes(
    folders: &mut Vec<PathBuf>,
    added: &[PathBuf],
    removed: &[PathBuf],
) -> bool {
    let before = canonical_workspace_roots(folders);
    let mut next = before.clone();
    next.extend(
        added.iter().map(|path| {
            crate::files_correction::canonical_path(path.to_string_lossy().into_owned())
        }),
    );
    next = canonical_workspace_roots(&next);
    for path in removed {
        let canonical_path =
            crate::files_correction::canonical_path(path.to_string_lossy().into_owned());
        next.retain(|folder| folder != &canonical_path);
    }
    next = canonical_workspace_roots(&next);
    *folders = next.clone();
    before != next
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Choice {
    pub index: u32,
    pub code_completion: String,
    pub finish_reason: String,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct CompletionRes {
    pub choices: Vec<Choice>,
    pub cached: Option<bool>,
    pub model: String,
    pub created: Option<f32>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct SuccessRes {
    pub success: bool,
}

impl LspBackend {
    async fn flat_params_to_code_completion_post(
        &self,
        params: &CompletionParams1,
    ) -> Result<CodeCompletionPost> {
        let path = canonical_path_from_file_uri_required(
            &params.text_document_position.text_document.uri,
            "text_document.uri",
        )?;
        let memory_document_map = { self.gcx.documents_state.memory_document_map.clone() };
        let doc = memory_document_map
            .lock()
            .await
            .get(&path)
            .cloned()
            .ok_or_else(|| internal_error("document not found"))?;
        let mut doc_snapshot = doc.write().await;
        let txt = crate::files_in_workspace::get_document_text_or_read_from_disk(
            &mut doc_snapshot,
            self.gcx.clone(),
        )
        .await
        .map_err(internal_error)?;
        let path_string = path.to_string_lossy().to_string();
        Ok(CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from([(path_string.clone(), (&txt).to_string())]),
                cursor: CursorPosition {
                    file: path_string.clone(),
                    line: params.text_document_position.position.line as i32,
                    character: params.text_document_position.position.character as i32,
                },
                multiline: params.multiline,
            },
            parameters: SamplingParameters {
                max_new_tokens: params.parameters.max_new_tokens as usize,
                temperature: Option::from(params.parameters.temperature),
                ..Default::default()
            },
            model: params.parameters.model.clone(),
            stream: false,
            no_cache: params.parameters.no_cache,
            use_ast: params.parameters.use_ast,
            use_vecdb: params.parameters.use_vecdb,
            rag_tokens_n: params.parameters.rag_tokens_n,
            cache_salt: String::new(),
            cache_generation: 0,
        })
    }

    pub async fn get_completions(&self, params: CompletionParams1) -> Result<CompletionRes> {
        let mut post = self.flat_params_to_code_completion_post(&params).await?;

        let app = AppState::from_gcx(self.gcx.clone()).await;
        let res = handle_v1_code_completion(app, &mut post)
            .await
            .map_err(|e| internal_error(e))?;

        let body_bytes = hyper::body::to_bytes(res.into_body())
            .await
            .map_err(|e| internal_error(e))?;

        let s = String::from_utf8(body_bytes.to_vec()).map_err(|e| internal_error(e))?;
        let value =
            serde_json::from_str::<CompletionRes>(s.as_str()).map_err(|e| internal_error(e))?;

        Ok(value)
    }

    pub async fn set_active_document(&self, params: ChangeActiveFile) -> Result<SuccessRes> {
        let path = canonical_path_from_file_uri_required(&params.uri, "uri")?;
        info!(
            "ACTIVE_DOC {:?}",
            crate::nicer_logs::last_n_chars(&path.to_string_lossy().to_string(), 30)
        );
        *self.gcx.documents_state.active_file_path.lock().await = Some(path);
        Ok(SuccessRes { success: true })
    }

    async fn ping_http_server(&self) -> Result<()> {
        let (port, http_client) = { (self.gcx.cmdline.http_port, self.gcx.http_client.clone()) };

        let url = "http://127.0.0.1:".to_string() + &port.to_string() + &"/v1/ping".to_string();
        let mut attempts = 0;
        while attempts < 15 {
            let response = http_client.get(&url).send().await;
            match response {
                Ok(res) if res.status().is_success() => {
                    return Ok(());
                }
                _ => {
                    attempts += 1;
                    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                }
            }
        }
        Err(internal_error("HTTP server is not ready after 15 attempts"))
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for LspBackend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        info!("LSP client_info {:?}", params.client_info);

        let mut folders: Vec<PathBuf> = Vec::new();

        if let Some(workspace_folders) = &params.workspace_folders {
            folders = workspace_folders
                .iter()
                .filter_map(|x| canonical_path_from_file_uri(&x.uri))
                .collect();
        }

        if folders.is_empty() {
            if let Some(root_uri) = &params.root_uri {
                if let Some(root_path) = canonical_path_from_file_uri(root_uri) {
                    folders.push(root_path);
                }
            } else {
                #[allow(deprecated)]
                if let Some(root_path) = &params.root_path {
                    folders.push(crate::files_correction::canonical_path(root_path.clone()));
                }
            }
        }
        let folders = canonical_workspace_roots(&folders);
        let changed = {
            let mut workspace_folders = self.gcx.documents_state.workspace_folders.lock().unwrap();
            if workspace_roots_changed(&workspace_folders, &folders) {
                *workspace_folders = folders.clone();
                info!("LSP workspace_folders {:?}", folders);
                true
            } else {
                false
            }
        };

        if changed {
            let gcx_clone = self.gcx.clone();
            tokio::spawn(async move {
                files_in_workspace::on_workspaces_init(gcx_clone.clone()).await;
                notify_workspace_changed(&gcx_clone).await;
            });
        }

        let completion_options: CompletionOptions;
        completion_options = CompletionOptions {
            resolve_provider: Some(false),
            trigger_characters: Some(vec![".(".to_owned()]),
            all_commit_characters: None,
            work_done_progress_options: WorkDoneProgressOptions {
                work_done_progress: Some(false),
            },
            completion_item: None,
        };
        let file_filter = FileOperationRegistrationOptions {
            filters: vec![FileOperationFilter {
                scheme: None,
                pattern: FileOperationPattern {
                    glob: "**".to_string(),
                    matches: None,
                    options: None,
                },
            }],
        };

        // wait for http server to be ready
        self.ping_http_server().await?;

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "refact".to_owned(),
                version: Some(VERSION.to_owned()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(completion_options),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                        did_create: Some(file_filter.clone()),
                        will_create: Some(file_filter.clone()),
                        did_rename: Some(file_filter.clone()),
                        will_rename: Some(file_filter.clone()),
                        did_delete: Some(file_filter.clone()),
                        will_delete: Some(file_filter.clone()),
                    }),
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "rust LSP received initialized()")
            .await;
        let _ = info!("rust LSP received initialized()");
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let Some(cpath) =
            canonical_path_from_file_uri_for_notification("did_open", &params.text_document.uri)
        else {
            return;
        };
        if cpath.to_string_lossy().contains("keybindings.json") {
            return;
        }
        files_in_workspace::on_did_open(
            self.gcx.clone(),
            &cpath,
            &params.text_document.text,
            &params.text_document.language_id,
        )
        .await
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "{refact-lsp} file closed")
            .await;
        let Some(cpath) =
            canonical_path_from_file_uri_for_notification("did_close", &params.text_document.uri)
        else {
            return;
        };
        files_in_workspace::on_did_close(self.gcx.clone(), &cpath).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(path) =
            canonical_path_from_file_uri_for_notification("did_change", &params.text_document.uri)
        else {
            return;
        };
        let Some(first_change) = params.content_changes.first() else {
            info!("did_change ignored notification with empty content_changes");
            return;
        };
        on_did_change(
            self.gcx.clone(),
            &path,
            &first_change.text, // TODO: This text could be just a part of the whole file (if range is not none)
        )
        .await
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let Some(path) =
            canonical_path_from_file_uri_for_notification("did_save", &params.text_document.uri)
        else {
            return;
        };
        self.client
            .log_message(MessageType::INFO, "{refact-lsp} file saved")
            .await;
        info!("{} saved", path.display());
    }

    async fn shutdown(&self) -> Result<()> {
        info!("shutdown");
        if self.transport == LspTransport::Tcp {
            return Ok(());
        }
        self.gcx
            .ask_shutdown_sender
            .lock()
            .unwrap()
            .send("LSP SHUTDOWN".to_string())
            .unwrap();
        Ok(())
    }

    async fn completion(&self, _: CompletionParams) -> Result<Option<CompletionResponse>> {
        info!("LSP asked for popup completions");
        Ok(Some(CompletionResponse::Array(vec![])))
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        for folder in params.event.added {
            info!("did_change_workspace_folders/add {}", folder.name);
            let Some(path) = canonical_path_from_file_uri(&folder.uri) else {
                info!(
                    "did_change_workspace_folders/add ignored non-file URI {}",
                    folder.uri
                );
                continue;
            };
            added.push(path);
        }
        for folder in params.event.removed {
            info!("did_change_workspace_folders/delete {}", folder.name);
            let Some(path) = canonical_path_from_file_uri(&folder.uri) else {
                info!(
                    "did_change_workspace_folders/delete ignored non-file URI {}",
                    folder.uri
                );
                continue;
            };
            removed.push(path);
        }
        let changed = {
            let mut workspace_folders = self.gcx.documents_state.workspace_folders.lock().unwrap();
            apply_workspace_root_changes(&mut workspace_folders, &added, &removed)
        };
        if changed {
            files_in_workspace::on_workspaces_init(self.gcx.clone()).await;
            notify_workspace_changed(&self.gcx).await;
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        for event in params.changes {
            if event.typ == FileChangeType::DELETED {
                let Some(cpath) = canonical_path_from_file_uri_for_notification(
                    "did_change_watched_files/delete",
                    &event.uri,
                ) else {
                    continue;
                };
                info!(
                    "UNCLEAR LSP EVENT: did_change_watched_files/delete {}",
                    cpath.display()
                );
                on_did_delete(self.gcx.clone(), &cpath).await;
            } else if event.typ == FileChangeType::CREATED {
                let Some(cpath) = canonical_path_from_file_uri_for_notification(
                    "did_change_watched_files/change",
                    &event.uri,
                ) else {
                    continue;
                };
                info!(
                    "UNCLEAR LSP EVENT: did_change_watched_files/change {}",
                    cpath.display()
                );
                // on_did_change(self.gcx.clone(), &cpath, &text).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::service::{make_service_fn, service_fn};
    use hyper::{Body, Request, Response as HyperResponse, Server};
    use serde_json::{Value, json};
    use std::net::SocketAddr;
    use std::sync::atomic::Ordering;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::time::timeout;

    async fn start_ping_server() -> (u16, JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = Server::from_tcp(listener)
            .unwrap()
            .serve(make_service_fn(|_| async {
                Ok::<_, hyper::Error>(service_fn(|_req: Request<Body>| async {
                    Ok::<_, hyper::Error>(HyperResponse::new(Body::from("pong")))
                }))
            }));
        let handle = tokio::spawn(async move {
            if let Err(err) = server.await {
                error!("test ping server failed: {}", err);
            }
        });
        (port, handle)
    }

    async fn make_lsp_test_gcx(http_port: u16) -> Arc<GlobalContext> {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cmdline.http_port = http_port;
        gcx
    }

    async fn start_lsp_test_server(gcx: Arc<GlobalContext>) -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(serve_lsp_tcp(listener, gcx));
        (addr, handle)
    }

    async fn wait_for_client_count(gcx: &Arc<GlobalContext>, expected: usize) {
        timeout(Duration::from_secs(2), async {
            loop {
                if gcx.lsp_tcp_client_count.load(Ordering::SeqCst) == expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "expected {} LSP TCP clients, got {}",
                expected,
                gcx.lsp_tcp_client_count.load(Ordering::SeqCst)
            )
        });
    }

    async fn write_lsp_message(stream: &mut TcpStream, value: Value) {
        let body = value.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        stream.write_all(frame.as_bytes()).await.unwrap();
    }

    async fn read_lsp_message(stream: &mut TcpStream) -> Value {
        timeout(Duration::from_secs(2), async {
            let mut header = Vec::new();
            let mut byte = [0_u8; 1];
            loop {
                stream.read_exact(&mut byte).await.unwrap();
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(header).unwrap();
            let content_length = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.eq_ignore_ascii_case("content-length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap();
            let mut body = vec![0_u8; content_length];
            stream.read_exact(&mut body).await.unwrap();
            serde_json::from_slice::<Value>(&body).unwrap()
        })
        .await
        .unwrap()
    }

    async fn read_lsp_response(stream: &mut TcpStream, id: u64) -> Value {
        timeout(Duration::from_secs(2), async {
            loop {
                let message = read_lsp_message(stream).await;
                if message.get("id").and_then(Value::as_u64) == Some(id) {
                    return message;
                }
            }
        })
        .await
        .unwrap()
    }

    async fn send_initialize(stream: &mut TcpStream, id: u64) -> Value {
        write_lsp_message(
            stream,
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "initialize",
                "params": {
                    "processId": null,
                    "rootUri": null,
                    "capabilities": {},
                    "workspaceFolders": null
                }
            }),
        )
        .await;
        read_lsp_response(stream, id).await
    }

    async fn send_initialized(stream: &mut TcpStream) {
        write_lsp_message(
            stream,
            json!({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {}
            }),
        )
        .await;
    }

    async fn send_did_open_and_change(stream: &mut TcpStream, path: &PathBuf, text: &str) {
        let uri = Url::from_file_path(path).unwrap().to_string();
        write_lsp_message(
            stream,
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": "rust",
                        "version": 1,
                        "text": "fn before() {}\n"
                    }
                }
            }),
        )
        .await;
        write_lsp_message(
            stream,
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "version": 2
                    },
                    "contentChanges": [{ "text": text }]
                }
            }),
        )
        .await;
    }

    async fn wait_for_document_text(gcx: &Arc<GlobalContext>, path: &PathBuf, expected: &str) {
        let canonical = crate::files_correction::canonical_path(path.to_string_lossy());
        timeout(Duration::from_secs(2), async {
            loop {
                let maybe_doc = gcx
                    .documents_state
                    .memory_document_map
                    .lock()
                    .await
                    .get(&canonical)
                    .cloned();
                if let Some(doc) = maybe_doc {
                    let doc = doc.read().await;
                    if doc
                        .doc_text
                        .as_ref()
                        .map(|text| text.to_string())
                        .as_deref()
                        == Some(expected)
                    {
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn shutdown_server(gcx: Arc<GlobalContext>, handle: JoinHandle<()>) {
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        timeout(Duration::from_secs(1), handle)
            .await
            .expect("LSP test server did not stop after shutdown flag")
            .expect("LSP test server task panicked");
    }

    #[tokio::test]
    async fn lsp_tcp_serves_two_clients_concurrently() {
        let (http_port, ping_handle) = start_ping_server().await;
        let gcx = make_lsp_test_gcx(http_port).await;
        let (addr, lsp_handle) = start_lsp_test_server(gcx.clone()).await;
        let temp = tempfile::tempdir().unwrap();
        let file_a = temp.path().join("a.rs");
        let file_b = temp.path().join("b.rs");

        let mut client_a = TcpStream::connect(addr).await.unwrap();
        wait_for_client_count(&gcx, 1).await;
        let mut client_b = TcpStream::connect(addr).await.unwrap();
        wait_for_client_count(&gcx, 2).await;

        let response_b = send_initialize(&mut client_b, 2).await;
        assert_eq!(response_b.get("id").and_then(Value::as_u64), Some(2));
        assert!(response_b.get("result").is_some());
        send_initialized(&mut client_b).await;
        send_did_open_and_change(&mut client_b, &file_b, "fn from_b() {}\n").await;
        wait_for_document_text(&gcx, &file_b, "fn from_b() {}\n").await;

        let response_a = send_initialize(&mut client_a, 1).await;
        assert_eq!(response_a.get("id").and_then(Value::as_u64), Some(1));
        assert!(response_a.get("result").is_some());
        send_initialized(&mut client_a).await;
        send_did_open_and_change(&mut client_a, &file_a, "fn from_a() {}\n").await;
        wait_for_document_text(&gcx, &file_a, "fn from_a() {}\n").await;

        drop(client_b);
        wait_for_client_count(&gcx, 1).await;
        drop(client_a);
        wait_for_client_count(&gcx, 0).await;
        shutdown_server(gcx, lsp_handle).await;
        ping_handle.abort();
    }

    #[tokio::test]
    async fn lsp_tcp_client_count_tracks_disconnect_churn() {
        let (http_port, ping_handle) = start_ping_server().await;
        let gcx = make_lsp_test_gcx(http_port).await;
        let (addr, lsp_handle) = start_lsp_test_server(gcx.clone()).await;

        assert_eq!(gcx.lsp_tcp_client_count.load(Ordering::SeqCst), 0);
        let client_a = TcpStream::connect(addr).await.unwrap();
        wait_for_client_count(&gcx, 1).await;
        let client_b = TcpStream::connect(addr).await.unwrap();
        wait_for_client_count(&gcx, 2).await;

        drop(client_a);
        wait_for_client_count(&gcx, 1).await;
        drop(client_b);
        wait_for_client_count(&gcx, 0).await;

        shutdown_server(gcx, lsp_handle).await;
        ping_handle.abort();
    }

    #[tokio::test]
    async fn lsp_tcp_shutdown_request_does_not_stop_worker_or_listener() {
        let (http_port, ping_handle) = start_ping_server().await;
        let gcx = make_lsp_test_gcx(http_port).await;
        let (addr, lsp_handle) = start_lsp_test_server(gcx.clone()).await;

        let mut client = TcpStream::connect(addr).await.unwrap();
        wait_for_client_count(&gcx, 1).await;
        let response = send_initialize(&mut client, 1).await;
        assert!(response.get("result").is_some());
        send_initialized(&mut client).await;
        write_lsp_message(
            &mut client,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "shutdown",
            }),
        )
        .await;
        let shutdown_response = read_lsp_response(&mut client, 2).await;
        assert!(
            shutdown_response.get("error").is_none(),
            "shutdown response should not be an error: {shutdown_response}"
        );
        write_lsp_message(
            &mut client,
            json!({
                "jsonrpc": "2.0",
                "method": "exit"
            }),
        )
        .await;
        drop(client);
        wait_for_client_count(&gcx, 0).await;

        assert!(!gcx.shutdown_flag.load(Ordering::Relaxed));
        let mut next_client = TcpStream::connect(addr).await.unwrap();
        wait_for_client_count(&gcx, 1).await;
        let next_response = send_initialize(&mut next_client, 3).await;
        assert_eq!(next_response.get("id").and_then(Value::as_u64), Some(3));
        assert!(next_response.get("result").is_some());
        drop(next_client);
        wait_for_client_count(&gcx, 0).await;

        shutdown_server(gcx, lsp_handle).await;
        ping_handle.abort();
    }

    #[test]
    fn sidebar_workspace_roots_changed_ignores_order_and_duplicates() {
        let first = PathBuf::from("/workspace/first");
        let second = PathBuf::from("/workspace/second");

        assert!(!workspace_roots_changed(
            &[first.clone(), second.clone()],
            &[second.clone(), first.clone()]
        ));
        assert!(!workspace_roots_changed(
            &[first.clone(), first.clone()],
            std::slice::from_ref(&first)
        ));
        assert!(workspace_roots_changed(
            std::slice::from_ref(&first),
            &[first.clone(), second]
        ));
    }

    #[test]
    fn lsp_request_params_default_completion_flags() {
        let params: RequestParams = serde_json::from_value(serde_json::json!({
            "max_new_tokens": 12,
            "temperature": 0.4
        }))
        .unwrap();

        assert_eq!(params.model, "");
        assert!(!params.no_cache);
        assert!(!params.use_ast);
        assert!(!params.use_vecdb);
        assert_eq!(params.rag_tokens_n, 0);
    }

    #[test]
    fn lsp_request_params_accept_completion_overrides() {
        let params: RequestParams = serde_json::from_value(serde_json::json!({
            "max_new_tokens": 12,
            "temperature": 0.4,
            "model": "provider/model",
            "no_cache": true,
            "use_ast": true,
            "use_vecdb": true,
            "rag_tokens_n": 256
        }))
        .unwrap();

        assert_eq!(params.model, "provider/model");
        assert!(params.no_cache);
        assert!(params.use_ast);
        assert!(params.use_vecdb);
        assert_eq!(params.rag_tokens_n, 256);
    }

    #[test]
    fn sidebar_canonical_workspace_roots_are_sorted_and_deduplicated() {
        let first = PathBuf::from("/workspace/first");
        let second = PathBuf::from("/workspace/second");

        assert_eq!(
            canonical_workspace_roots(&[second.clone(), first.clone(), second]),
            vec![first, PathBuf::from("/workspace/second")]
        );
    }

    #[test]
    fn sidebar_canonical_path_from_file_uri_ignores_non_file_uris() {
        let workspace_root = std::env::temp_dir().join("refact-sidebar-uri-root");
        let file_uri = Url::from_directory_path(&workspace_root).unwrap();

        assert_eq!(
            canonical_path_from_file_uri(&file_uri),
            Some(crate::files_correction::canonical_path(
                workspace_root.to_string_lossy().into_owned()
            ))
        );
        assert!(
            canonical_path_from_file_uri(&Url::parse("untitled:Untitled-1").unwrap()).is_none()
        );
        assert!(canonical_path_from_file_uri(
            &Url::parse("vscode-remote://ssh-remote%2Bhost/home/user/project").unwrap()
        )
        .is_none());
    }

    #[test]
    fn lsp_file_uri_required_reports_invalid_params_for_non_file_uris() {
        let err = canonical_path_from_file_uri_required(
            &Url::parse("untitled:Untitled-1").unwrap(),
            "text_document.uri",
        )
        .unwrap_err();

        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("text_document.uri"));
    }

    #[test]
    fn lsp_notification_file_uri_filter_ignores_non_file_uris() {
        assert!(canonical_path_from_file_uri_for_notification(
            "did_open",
            &Url::parse("vscode-remote://ssh-remote%2Bhost/home/user/project/src/main.rs").unwrap()
        )
        .is_none());
    }

    #[test]
    fn sidebar_workspace_root_set_helpers_compare_canonical_sets() {
        let first_raw = std::env::temp_dir().join("refact-sidebar-first");
        let first =
            crate::files_correction::canonical_path(first_raw.to_string_lossy().into_owned());
        let first_duplicate = first_raw.join(".");
        let second_raw = std::env::temp_dir().join("refact-sidebar-second");
        let second =
            crate::files_correction::canonical_path(second_raw.to_string_lossy().into_owned());
        let missing = std::env::temp_dir().join("refact-sidebar-missing");
        let mut folders = vec![first.clone()];

        assert!(!add_workspace_root_to_set(
            &mut folders,
            first_duplicate.clone()
        ));
        assert_eq!(folders, vec![first.clone()]);

        assert!(add_workspace_root_to_set(&mut folders, second_raw));
        assert_eq!(
            folders,
            canonical_workspace_roots(&[first.clone(), second.clone()])
        );

        assert!(!remove_workspace_root_from_set(&mut folders, &missing));
        assert_eq!(
            folders,
            canonical_workspace_roots(&[first.clone(), second.clone()])
        );

        assert!(remove_workspace_root_from_set(
            &mut folders,
            &first_duplicate
        ));
        assert_eq!(folders, vec![second]);

        assert!(!remove_workspace_root_from_set(&mut folders, &first_raw));
    }

    #[test]
    fn sidebar_workspace_root_changes_are_compared_after_full_mutation() {
        let first_raw = std::env::temp_dir().join("refact-sidebar-batch-first");
        let first =
            crate::files_correction::canonical_path(first_raw.to_string_lossy().into_owned());
        let second_raw = std::env::temp_dir().join("refact-sidebar-batch-second");
        let second =
            crate::files_correction::canonical_path(second_raw.to_string_lossy().into_owned());
        let missing = std::env::temp_dir().join("refact-sidebar-batch-missing");
        let mut folders = vec![first.clone()];

        assert!(!apply_workspace_root_changes(
            &mut folders,
            std::slice::from_ref(&first_raw),
            &[]
        ));
        assert_eq!(folders, vec![first.clone()]);

        assert!(!apply_workspace_root_changes(
            &mut folders,
            &[],
            std::slice::from_ref(&missing)
        ));
        assert_eq!(folders, vec![first.clone()]);

        assert!(!apply_workspace_root_changes(
            &mut folders,
            std::slice::from_ref(&second_raw),
            std::slice::from_ref(&second_raw)
        ));
        assert_eq!(folders, vec![first.clone()]);

        assert!(apply_workspace_root_changes(
            &mut folders,
            std::slice::from_ref(&second_raw),
            std::slice::from_ref(&first_raw)
        ));
        assert_eq!(folders, vec![second]);
    }
}

async fn build_lsp_service(
    gcx: Arc<GlobalContext>,
    transport: LspTransport,
) -> (LspService<LspBackend>, ClientSocket) {
    let (lsp_service, socket) = LspService::build(move |client| LspBackend {
        gcx: gcx.clone(),
        client,
        transport,
    })
    .custom_method("refact/getCompletions", LspBackend::get_completions)
    .custom_method("refact/setActiveDocument", LspBackend::set_active_document)
    .finish();
    (lsp_service, socket)
}

async fn serve_lsp_tcp(listener: TcpListener, gcx: Arc<GlobalContext>) {
    let mut shutdown_poll = tokio::time::interval(Duration::from_millis(200));
    let mut connection_tasks = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown_poll.tick() => {
                if gcx.shutdown_flag.load(Ordering::Relaxed) {
                    break;
                }
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, addr)) => {
                        info!("LSP new client connection from {}", addr);
                        let guard = LspTcpClientGuard::new(gcx.clone());
                        let gcx_for_client = gcx.clone();
                        connection_tasks.spawn(async move {
                            let _guard = guard;
                            let (read, write) = tokio::io::split(stream);
                            let (lsp_service, socket) = build_lsp_service(gcx_for_client, LspTransport::Tcp).await;
                            tower_lsp::Server::new(read, write, socket)
                                .serve(lsp_service)
                                .await;
                            info!("LSP client connection from {} ended", addr);
                        });
                    }
                    Err(e) => {
                        error!("Error accepting client connection: {}", e);
                    }
                }
            }
        }
        drain_finished_lsp_tasks(&mut connection_tasks);
    }
    connection_tasks.abort_all();
    while connection_tasks.join_next().await.is_some() {}
}

fn drain_finished_lsp_tasks(connection_tasks: &mut JoinSet<()>) {
    while let Some(result) = connection_tasks.try_join_next() {
        if let Err(err) = result {
            error!("LSP client task failed: {}", err);
        }
    }
}

pub async fn spawn_lsp_task(
    gcx: Arc<GlobalContext>,
    cmdline: CommandLine,
) -> Option<JoinHandle<()>> {
    if cmdline.lsp_stdin_stdout == 0 && cmdline.lsp_port > 0 {
        let gcx_t = gcx.clone();
        let addr: std::net::SocketAddr = ([127, 0, 0, 1], cmdline.lsp_port).into();
        return Some(tokio::spawn(async move {
            let listener_maybe = TcpListener::bind(&addr).await;
            if listener_maybe.is_err() {
                let _ = write!(
                    std::io::stderr(),
                    "PORT_BUSY\n{}: {}\n",
                    addr,
                    listener_maybe.unwrap_err()
                );
                gcx_t
                    .ask_shutdown_sender
                    .lock()
                    .unwrap()
                    .send("LSP PORT_BUSY".to_string())
                    .unwrap();
                return;
            }
            let listener = listener_maybe.unwrap();
            info!("LSP listening on {}", listener.local_addr().unwrap());
            serve_lsp_tcp(listener, gcx_t.clone()).await;
        }));
    }

    if cmdline.lsp_stdin_stdout != 0 && cmdline.lsp_port == 0 {
        let gcx_t = gcx.clone();
        return Some(tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let stdout = tokio::io::stdout();
            let (lsp_service, socket) = build_lsp_service(gcx_t.clone(), LspTransport::Stdio).await;
            tower_lsp::Server::new(stdin, stdout, socket)
                .serve(lsp_service)
                .await;
            info!("LSP loop exit");
            match gcx_t.ask_shutdown_sender.lock() {
                Ok(sender) => {
                    if let Err(err) = sender.send("going-down-because-lsp-exited".to_string()) {
                        error!("Failed to send shutdown message: {}", err);
                    }
                }
                Err(err) => {
                    error!("Failed to lock ask_shutdown_sender: {}", err);
                }
            }
        }));
    }

    None
}
