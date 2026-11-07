use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, ContextFile, PostprocessSettings};
use crate::file_activity::{self, ReadObservation, TOOL_ACTOR};
use crate::postprocessing::pp_tool_results::{postprocess_tool_results, ToolBudget};
use crate::tools::tool_cat::ToolCat;
use crate::tools::tools_description::Tool;

const A: &str = "first\noriginal\nlast\n";
const B: &str = "first\nmodified\nlast\n";
const C: &str = "first\nunseen\nlast\n";

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    actor: String,
    ccx: Arc<Mutex<AtCommandsContext>>,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("restored.txt");
        std::fs::write(&path, A).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let actor = format!("dedup-{}", uuid::Uuid::new_v4());
        let ccx = AtCommandsContext::new_from_app(
            crate::app_state::AppState::from_gcx(gcx).await,
            32768,
            20,
            false,
            vec![],
            actor.clone(),
            None,
            "test-model".to_string(),
            None,
            None,
        )
        .await;
        Self {
            _dir: dir,
            path,
            actor,
            ccx: Arc::new(Mutex::new(ccx)),
        }
    }

    async fn cat(&self, history: &[ChatMessage], range: &str) -> Vec<ChatMessage> {
        self.cat_with_revision(history, range, None).await
    }

    async fn cat_with_revision(
        &self,
        history: &[ChatMessage],
        range: &str,
        revision: Option<String>,
    ) -> Vec<ChatMessage> {
        let mut cat = ToolCat {
            config_path: String::new(),
        };
        let args = HashMap::from([(
            "paths".to_string(),
            json!(format!("{}{range}", self.path.display())),
        )]);
        let (_, results) = cat
            .tool_execute(self.ccx.clone(), &"cat-call".to_string(), &args)
            .await
            .unwrap();
        let mut files = Vec::new();
        let mut messages = Vec::new();
        for result in results {
            match result {
                ContextEnum::ContextFile(mut file) => {
                    file.file_rev = revision.clone();
                    files.push(file);
                }
                ContextEnum::ChatMessage(message) => messages.push(message),
            }
        }
        let gcx = self.ccx.lock().await.app.gcx.clone();
        TOOL_ACTOR
            .scope(
                self.actor.clone(),
                postprocess_tool_results(
                    gcx,
                    None,
                    messages,
                    files,
                    ToolBudget {
                        tokens_for_code: 16000,
                        tokens_for_text: 4000,
                    },
                    PostprocessSettings::default(),
                    history,
                    &HashSet::new(),
                ),
            )
            .await
    }

    async fn edit(&self, old: &str, new: &str) -> Result<(), String> {
        let args: HashMap<String, Value> = HashMap::from([
            ("path".to_string(), json!(self.path)),
            ("old_str".to_string(), json!(old)),
            ("replacement".to_string(), json!(new)),
        ]);
        let gcx = self.ccx.lock().await.app.gcx.clone();
        TOOL_ACTOR
            .scope(
                self.actor.clone(),
                super::tool_update_textdoc::tool_update_text_doc_exec(
                    gcx, &args, false, None, None,
                ),
            )
            .await
            .map(|_| ())
    }

    fn stamp(&self) -> ReadObservation {
        let (key, _) = file_activity::resolve_key(&self.path);
        file_activity::observations(None)
            .reads
            .into_iter()
            .rev()
            .find(|read| read.actor == self.actor && read.key == key)
            .unwrap()
    }

    async fn modified_history(&self) -> Vec<ChatMessage> {
        let mut history = self.cat(&[], "").await;
        assert_eq!(self.stamp().digest, file_activity::digest_of(A));
        assert!(self.stamp().whole);
        self.edit("original", "modified").await.unwrap();
        assert_eq!(std::fs::read_to_string(&self.path).unwrap(), B);
        history.extend(self.cat(&history, "").await);
        assert_eq!(self.stamp().digest, file_activity::digest_of(B));
        history
    }
}

fn files(messages: &[ChatMessage]) -> Vec<ContextFile> {
    messages
        .iter()
        .flat_map(|message| match &message.content {
            ChatContent::ContextFiles(files) => files.clone(),
            _ => vec![],
        })
        .collect()
}

fn was_deduplicated(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|message| {
        let text = message.content.content_text_only();
        text.contains("already retrieved in message #") || text.contains("already in context")
    })
}

#[tokio::test]
async fn restored_bytes_seen_earlier_are_readable_again_after_a_dedup_cat() {
    let fixture = Fixture::new().await;
    let history = fixture.modified_history().await;
    std::fs::write(&fixture.path, A).unwrap();
    let reread = fixture.cat(&history, "").await;
    assert!(was_deduplicated(&reread));
    assert!(files(&reread).is_empty());
    fixture.edit("original", "recovered").await.unwrap();
    assert_eq!(fixture.stamp().digest, file_activity::digest_of(A));
    assert!(fixture.stamp().whole);
    assert_eq!(
        std::fs::read_to_string(&fixture.path).unwrap(),
        "first\nrecovered\nlast\n"
    );
}

#[tokio::test]
async fn dedup_never_stamps_when_bytes_differ() {
    let fixture = Fixture::new().await;
    let history = fixture.modified_history().await;
    std::fs::write(&fixture.path, C).unwrap();

    let reread = fixture.cat(&history, "").await;
    assert!(!was_deduplicated(&reread));
    assert!(files(&reread)[0].file_content.contains("unseen"));
    assert_eq!(fixture.stamp().digest, file_activity::digest_of(C));
    fixture.edit("unseen", "current").await.unwrap();
}

#[tokio::test]
async fn compressed_earlier_read_is_not_used_for_dedup() {
    let fixture = Fixture::new().await;
    let history = fixture.modified_history().await;
    std::fs::write(&fixture.path, A).unwrap();
    let summary = ChatMessage {
        message_id: "summary".to_string(),
        role: "user".to_string(),
        content: ChatContent::SimpleText("The file was inspected earlier.".to_string()),
        ..Default::default()
    };
    let report =
        refact_core::active_context::make_reconstruction_report(vec![summary], Default::default())
            .unwrap();
    let mut compressed = history.clone();
    compressed.push(report);
    let mut truncated = history.clone();
    for message in &mut truncated {
        if let ChatContent::ContextFiles(files) = &mut message.content {
            for file in files {
                file.file_content = "... content omitted".to_string();
            }
        }
    }
    let mut hidden = history;
    for message in &mut hidden {
        message.extra.insert("_ui_only".to_string(), json!(true));
    }

    for unavailable in [compressed, truncated, hidden, vec![]] {
        let reread = fixture.cat(&unavailable, "").await;
        assert!(!was_deduplicated(&reread));
        assert!(files(&reread)[0].file_content.contains("original"));
        assert_eq!(fixture.stamp().digest, file_activity::digest_of(A));
        assert!(fixture.stamp().whole);
    }
}

#[tokio::test]
async fn partial_dedup_records_range_without_blocking_overwrite() {
    let fixture = Fixture::new().await;
    let history = fixture.modified_history().await;
    std::fs::write(&fixture.path, A).unwrap();

    let reread = fixture.cat(&history, ":2-2").await;
    assert!(was_deduplicated(&reread));
    assert!(!fixture.stamp().whole);
    assert_eq!((fixture.stamp().line1, fixture.stamp().line2), (2, 2));
    let guard = file_activity::begin_write(
        Some(&fixture.actor),
        &fixture.path,
        file_activity::WriteIntent::Overwrite {
            authorized: false,
        },
        "create_textdoc",
    )
    .await
    .unwrap();
    drop(guard);
    fixture.edit("original", "contextual").await.unwrap();
}

#[tokio::test]
async fn early_coverage_dedup_checks_current_bytes_before_stamping() {
    let fixture = Fixture::new().await;
    let history = fixture.modified_history().await;
    let revision = files(&history)[0].file_rev.clone();
    std::fs::write(&fixture.path, C).unwrap();
    let reread = fixture
        .cat_with_revision(&history, "", revision.clone())
        .await;
    assert!(!was_deduplicated(&reread));
    assert_eq!(fixture.stamp().digest, file_activity::digest_of(C));

    std::fs::write(&fixture.path, A).unwrap();
    let reread = fixture.cat_with_revision(&history, "", revision).await;
    assert!(was_deduplicated(&reread));
    fixture.edit("original", "covered").await.unwrap();
}

#[tokio::test]
async fn rejected_prefilled_dedup_never_stamps_unseen_current_bytes() {
    let fixture = Fixture::new().await;
    fixture.modified_history().await;
    std::fs::write(&fixture.path, C).unwrap();
    let file = ContextFile {
        file_name: fixture.path.to_string_lossy().to_string(),
        file_content: A.to_string(),
        line1: 1,
        line2: 3,
        file_rev: Some(refact_core::chunk_utils::official_text_hashing_function(A)),
        symbols: vec![],
        gradient_type: -1,
        usefulness: 100.0,
        skip_pp: true,
    };
    let history = vec![ChatMessage {
        role: "context_file".to_string(),
        content: ChatContent::ContextFiles(vec![file.clone()]),
        ..Default::default()
    }];
    let gcx = fixture.ccx.lock().await.app.gcx.clone();
    let reread = TOOL_ACTOR
        .scope(
            fixture.actor.clone(),
            postprocess_tool_results(
                gcx,
                None,
                vec![ChatMessage::new(
                    "tool".to_string(),
                    "Prefilled read".to_string(),
                )],
                vec![file],
                ToolBudget {
                    tokens_for_code: 16000,
                    tokens_for_text: 4000,
                },
                PostprocessSettings::default(),
                &history,
                &HashSet::new(),
            ),
        )
        .await;
    assert!(!was_deduplicated(&reread));
    assert_eq!(fixture.stamp().digest, file_activity::digest_of(B));
    let reread = fixture.cat(&history, "").await;
    assert!(files(&reread)[0].file_content.contains("unseen"));
    assert_eq!(fixture.stamp().digest, file_activity::digest_of(C));
}
