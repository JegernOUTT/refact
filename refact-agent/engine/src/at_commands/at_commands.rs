use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;

use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{
    ChatMessage, ContextFile, ContextEnum, SubchatParameters, PostprocessSettings,
};
use crate::app_state::AppState;
use crate::chat::types::TaskMeta;
use crate::global_context::GlobalContext;
use crate::worktrees::scope::ExecutionScope;
use crate::worktrees::types::WorktreeMeta;

use crate::at_commands::at_file::AtFile;
use crate::at_commands::at_ast_definition::AtAstDefinition;
use crate::at_commands::at_tree::AtTree;
use crate::at_commands::at_web::AtWeb;
use crate::at_commands::execute_at::AtCommandMember;

pub const MAX_SUBCHAT_DEPTH: usize = 5;

pub struct AtCommandsContext {
    pub global_context: Arc<GlobalContext>,
    pub app: AppState,
    pub n_ctx: usize,
    pub top_n: usize,
    pub tokens_for_rag: usize,
    pub messages: Vec<ChatMessage>,
    #[allow(dead_code)]
    pub is_preview: bool,
    pub pp_skeleton: bool,
    #[allow(dead_code)]
    pub correction_only_up_to_step: usize,
    pub chat_id: String,
    pub root_chat_id: String,
    pub current_model: String,
    pub task_meta: Option<TaskMeta>,
    pub execution_scope: Option<ExecutionScope>,
    pub subchat_depth: usize,

    pub at_commands: HashMap<String, Arc<dyn AtCommand + Send>>,
    pub subchat_tool_parameters: IndexMap<String, SubchatParameters>,
    pub postprocess_parameters: PostprocessSettings,

    pub subchat_tx: Arc<AMutex<mpsc::UnboundedSender<serde_json::Value>>>,
    pub subchat_rx: Arc<AMutex<mpsc::UnboundedReceiver<serde_json::Value>>>,
    pub abort_flag: Arc<AtomicBool>,
}

impl AtCommandsContext {
    pub async fn new(
        global_context: Arc<GlobalContext>,
        n_ctx: usize,
        top_n: usize,
        is_preview: bool,
        messages: Vec<ChatMessage>,
        chat_id: String,
        root_chat_id: Option<String>,
        current_model: String,
        task_meta: Option<TaskMeta>,
        worktree: Option<WorktreeMeta>,
    ) -> Self {
        let app = AppState::from_gcx(global_context).await;
        Self::new_from_app(
            app,
            n_ctx,
            top_n,
            is_preview,
            messages,
            chat_id,
            root_chat_id,
            current_model,
            task_meta,
            worktree,
        )
        .await
    }

    pub async fn new_from_app(
        app: AppState,
        n_ctx: usize,
        top_n: usize,
        is_preview: bool,
        messages: Vec<ChatMessage>,
        chat_id: String,
        root_chat_id: Option<String>,
        current_model: String,
        task_meta: Option<TaskMeta>,
        worktree: Option<WorktreeMeta>,
    ) -> Self {
        Self::new_with_abort(
            app,
            n_ctx,
            top_n,
            is_preview,
            messages,
            chat_id,
            root_chat_id,
            current_model,
            task_meta,
            worktree,
            None,
        )
        .await
    }

    pub async fn new_with_abort(
        app: AppState,
        n_ctx: usize,
        top_n: usize,
        is_preview: bool,
        messages: Vec<ChatMessage>,
        chat_id: String,
        root_chat_id: Option<String>,
        current_model: String,
        task_meta: Option<TaskMeta>,
        worktree: Option<WorktreeMeta>,
        abort_flag: Option<Arc<AtomicBool>>,
    ) -> Self {
        let execution_scope = worktree.map(|worktree| ExecutionScope::from_worktree(&worktree));
        Self::new_with_abort_and_execution_scope(
            app,
            n_ctx,
            top_n,
            is_preview,
            messages,
            chat_id,
            root_chat_id,
            current_model,
            task_meta,
            execution_scope,
            abort_flag,
        )
        .await
    }

    pub async fn new_with_abort_and_execution_scope(
        app: AppState,
        n_ctx: usize,
        top_n: usize,
        is_preview: bool,
        messages: Vec<ChatMessage>,
        chat_id: String,
        root_chat_id: Option<String>,
        current_model: String,
        task_meta: Option<TaskMeta>,
        execution_scope: Option<ExecutionScope>,
        abort_flag: Option<Arc<AtomicBool>>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<serde_json::Value>();
        let effective_root = root_chat_id.unwrap_or_else(|| chat_id.clone());
        let global_context = app.gcx.clone();
        AtCommandsContext {
            global_context,
            app: app.clone(),
            n_ctx,
            top_n,
            tokens_for_rag: (n_ctx / 4).max(64).min(n_ctx),
            messages,
            is_preview,
            pp_skeleton: true,
            correction_only_up_to_step: 0,
            chat_id,
            root_chat_id: effective_root,
            current_model,
            task_meta,
            execution_scope,
            subchat_depth: 0,
            at_commands: at_commands_dict(app).await,
            subchat_tool_parameters: IndexMap::new(),
            postprocess_parameters: PostprocessSettings::new(),
            subchat_tx: Arc::new(AMutex::new(tx)),
            subchat_rx: Arc::new(AMutex::new(rx)),
            abort_flag: abort_flag.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
        }
    }

    #[cfg(test)]
    pub fn execution_scope_root(&self) -> Option<std::path::PathBuf> {
        self.execution_scope
            .as_ref()
            .map(|scope| scope.effective_root().to_path_buf())
    }

    #[cfg(test)]
    pub fn effective_project_dirs(&self) -> Vec<std::path::PathBuf> {
        self.execution_scope
            .as_ref()
            .map(|scope| scope.effective_project_dirs())
            .unwrap_or_default()
    }

    pub fn execution_scope_worktree(&self) -> Option<WorktreeMeta> {
        self.execution_scope
            .as_ref()
            .map(|scope| scope.worktree().clone())
    }
}

#[async_trait]
pub trait AtCommand: Send + Sync {
    fn params(&self) -> &Vec<Box<dyn AtParam>>;
    // returns (messages_for_postprocessing, text_on_clip)
    async fn at_execute(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        cmd: &mut AtCommandMember,
        args: &mut Vec<AtCommandMember>,
    ) -> Result<(Vec<ContextEnum>, String), String>;
    fn depends_on(&self) -> Vec<String> {
        vec![]
    } // "ast", "vecdb"
}

#[async_trait]
pub trait AtParam: Send + Sync {
    async fn is_value_valid(&self, ccx: Arc<AMutex<AtCommandsContext>>, value: &String) -> bool;
    async fn param_completion(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        value: &String,
    ) -> Vec<String>;
    fn param_completion_valid(&self) -> bool {
        false
    }
}

pub async fn at_commands_dict(
    app: AppState,
) -> HashMap<String, Arc<dyn AtCommand + Send>> {
    let at_commands_dict = HashMap::from([
        (
            "@file".to_string(),
            Arc::new(AtFile::new()) as Arc<dyn AtCommand + Send>,
        ),
        // ("@file-search".to_string(), Arc::new(AtFileSearch::new()) as Arc<dyn AtCommand + Send>),
        (
            "@definition".to_string(),
            Arc::new(AtAstDefinition::new()) as Arc<dyn AtCommand + Send>,
        ),
        // ("@local-notes-to-self".to_string(), Arc::new(AtLocalNotesToSelf::new()) as Arc<dyn AtCommand + Send>),
        (
            "@tree".to_string(),
            Arc::new(AtTree::new()) as Arc<dyn AtCommand + Send>,
        ),
        // ("@diff".to_string(), Arc::new(AtDiff::new()) as Arc<dyn AtCommand + Send>),
        // ("@diff-rev".to_string(), Arc::new(AtDiffRev::new()) as Arc<dyn AtCommand + Send>),
        (
            "@web".to_string(),
            Arc::new(AtWeb::new()) as Arc<dyn AtCommand + Send>,
        ),
        (
            "@search".to_string(),
            Arc::new(crate::at_commands::at_search::AtSearch::new()) as Arc<dyn AtCommand + Send>,
        ),
        (
            "@knowledge-load".to_string(),
            Arc::new(crate::at_commands::at_knowledge::AtLoadKnowledge::new())
                as Arc<dyn AtCommand + Send>,
        ),
    ]);

    let ast_on = app.workspace.ast_service.is_some();
    let vecdb_on = app.workspace.vec_db.lock().await.is_some();
    let mut result = HashMap::new();
    for (key, value) in at_commands_dict {
        let depends_on = value.depends_on();
        if depends_on.contains(&"ast".to_string()) && !ast_on {
            continue;
        }
        if depends_on.contains(&"vecdb".to_string()) && !vecdb_on {
            continue;
        }
        result.insert(key, value);
    }

    result
}

pub fn vec_context_file_to_context_tools(x: Vec<ContextFile>) -> Vec<ContextEnum> {
    x.into_iter()
        .map(|i| ContextEnum::ContextFile(i))
        .collect::<Vec<ContextEnum>>()
}

pub fn filter_only_context_file_from_context_tool(tools: &Vec<ContextEnum>) -> Vec<ContextFile> {
    tools
        .iter()
        .filter_map(|x| {
            if let ContextEnum::ContextFile(data) = x {
                Some(data.clone())
            } else {
                None
            }
        })
        .collect::<Vec<ContextFile>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sample_worktree() -> (tempfile::TempDir, WorktreeMeta) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("worktree");
        let source = temp.path().join("source");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&source).unwrap();
        let root = dunce::simplified(&fs::canonicalize(root).unwrap()).to_path_buf();
        let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
        (
            temp,
            WorktreeMeta {
                id: "wt-context".to_string(),
                kind: "chat".to_string(),
                root,
                source_workspace_root: source.clone(),
                repo_root: source,
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: None,
                card_id: None,
                agent_id: None,
                enforce: true,
            },
        )
    }

    #[tokio::test]
    async fn subchat_worktree_at_commands_context_has_execution_scope() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (_temp, worktree) = sample_worktree();
        let ccx = AtCommandsContext::new_from_app(
            AppState::from_gcx(gcx).await,
            4096,
            20,
            false,
            vec![],
            "chat-1".to_string(),
            None,
            "model".to_string(),
            None,
            Some(worktree.clone()),
        )
        .await;

        assert!(ccx.execution_scope.is_some());
        assert_eq!(ccx.execution_scope_root(), Some(worktree.root.clone()));
        assert_eq!(ccx.effective_project_dirs(), vec![worktree.root.clone()]);
        assert_eq!(ccx.execution_scope_worktree(), Some(worktree));
    }
}
