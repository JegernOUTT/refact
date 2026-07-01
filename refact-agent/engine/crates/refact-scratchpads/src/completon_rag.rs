use refact_core::chat_types::{ContextFile, CursorPosition, PostprocessSettings};
use refact_postprocessing::pp_context_files::postprocess_context_files;
use refact_postprocessing::pp_context_provider::PPContextTrait;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use std::vec;
use tracing::info;

const DEBUG: bool = false;

const TAKE_USAGES_AROUND_CURSOR: usize = 20;

async fn _render_context_files(
    project_dirs: &[PathBuf],
    context_format: &String,
    postprocessed_messages: &Vec<ContextFile>,
    cursor_filepath: &PathBuf,
) -> String {
    if postprocessed_messages.is_empty() {
        return "".to_string();
    }
    let (repo_name, cursor_filepath_stripped) = if let Some(project_dir) = project_dirs.get(0) {
        let repo_name = project_dir
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or("default_repo".to_string());
        let cursor_filepath_stripped = cursor_filepath
            .strip_prefix(project_dir)
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or(cursor_filepath.to_string_lossy().to_string());
        (repo_name, cursor_filepath_stripped)
    } else {
        (
            "default_repo".to_string(),
            cursor_filepath.to_string_lossy().to_string(),
        )
    };
    let mut context_files_prompt = String::new();
    match context_format.as_str() {
        "starcoder" => {
            context_files_prompt.push_str(&format!("<repo_name>{}\n", repo_name));
            for m in postprocessed_messages {
                context_files_prompt
                    .push_str(&format!("<file_sep>{}\n{}", m.file_name, m.file_content));
            }
            format!("{context_files_prompt}<file_sep>{cursor_filepath_stripped}\n")
        }
        "qwen2.5" => {
            context_files_prompt.push_str(&format!("<|repo_name|>{}\n", repo_name));
            for m in postprocessed_messages {
                context_files_prompt
                    .push_str(&format!("<|file_sep|>{}\n{}", m.file_name, m.file_content));
            }
            format!("{context_files_prompt}<|file_sep|>{cursor_filepath_stripped}\n")
        }
        "chat" => {
            for m in postprocessed_messages {
                context_files_prompt.push_str(&format!(
                    "Filename: {}\nUseful content:\n```\n{}\n```\n\n",
                    m.file_name, m.file_content
                ));
            }
            context_files_prompt
        }
        _ => {
            tracing::warn!("context_format \"{}\" not recognized", context_format);
            "".to_string()
        }
    }
}

async fn _cursor_position_to_context_file_cg(
    service: Arc<refact_codegraph::CodeGraphService>,
    cpath: String,
    cursor_line: i32,
    context_used: &mut Value,
) -> Vec<ContextFile> {
    if cursor_line < 0 || cursor_line > 65535 {
        return vec![];
    }
    let cursor_line = (cursor_line + 1) as usize;
    let usages = service.doc_usages(&cpath).await.unwrap_or_default();

    let mut distances: Vec<(i32, String)> = usages
        .into_iter()
        .map(|(line, name)| ((line as i32 - cursor_line as i32).abs(), name))
        .collect();
    distances.sort_by_key(|&(distance, _)| distance);
    let unique_names: HashSet<String> = distances
        .into_iter()
        .take(TAKE_USAGES_AROUND_CURSOR)
        .map(|(_, name)| name)
        .collect();

    let mut output = vec![];
    let mut bucket_declarations = vec![];
    for name in unique_names {
        let defs = service.definitions(&name).await.unwrap_or_default();
        for def in defs {
            output.push(ContextFile {
                file_name: def.cpath.clone(),
                file_content: "".to_string(),
                line1: def.full_line1(),
                line2: def.full_line2(),
                file_rev: None,
                symbols: vec![def.path_drop0()],
                gradient_type: 4,
                usefulness: 100.,
                skip_pp: false,
            });
            bucket_declarations.push(json!({
                "file_path": def.cpath.clone(),
                "line1": def.full_line1(),
                "line2": def.full_line2(),
                "name": def.path_drop0(),
            }));
        }
    }
    context_used["bucket_declarations"] = json!(bucket_declarations);
    output
}

pub async fn retrieve_ast_based_extra_context(
    pp_context: Arc<dyn PPContextTrait>,
    project_dirs: Vec<PathBuf>,
    codegraph: Option<Arc<refact_codegraph::CodeGraphService>>,
    t: &HasTokenizerAndEot,
    cpath: &PathBuf,
    pos: &CursorPosition,
    subblock_to_ignore_range: (i32, i32),
    pp_settings: PostprocessSettings,
    rag_tokens_n: usize,
    context_used: &mut Value,
) -> String {
    info!(" -- ast-based rag search starts --");
    let mut pp_settings = pp_settings;
    if pp_settings.max_files_n == 0 {
        pp_settings.max_files_n = 5;
    }

    let rag_t0 = Instant::now();
    let mut ast_context_file_vec: Vec<ContextFile> = if let Some(service) = codegraph {
        _cursor_position_to_context_file_cg(
            service,
            cpath.to_string_lossy().to_string(),
            pos.line,
            context_used,
        )
        .await
    } else {
        vec![]
    };

    let to_buckets_ms = rag_t0.elapsed().as_millis() as i32;
    if subblock_to_ignore_range.0 != i32::MAX && subblock_to_ignore_range.1 != i32::MIN {
        let fim_ban = ContextFile {
            file_name: cpath.to_string_lossy().to_string(),
            file_content: "".to_string(),
            line1: (subblock_to_ignore_range.0 + 1) as usize,
            line2: (subblock_to_ignore_range.1 + 1) as usize,
            file_rev: None,
            symbols: vec![],
            gradient_type: 4,
            usefulness: -1.0,
            skip_pp: false,
        };
        ast_context_file_vec.push(fim_ban);
    }

    info!(" -- post processing starts --");
    let post_t0 = Instant::now();
    let (postprocessed_messages, _notes) = postprocess_context_files(
        pp_context.clone(),
        &mut ast_context_file_vec,
        t.tokenizer.clone(),
        rag_tokens_n,
        false,
        &pp_settings,
    )
    .await;
    let rag_ms = rag_t0.elapsed().as_millis() as i32;
    let post_ms = post_t0.elapsed().as_millis() as i32;
    info!(
        " -- /post buckets {}ms, post {}ms -- ",
        to_buckets_ms, post_ms
    );

    context_used["attached_files"] = Value::Array(
        postprocessed_messages
            .iter()
            .map(|x| {
                json!({
                    "file_name": x.file_name,
                    "file_content": x.file_content,
                    "line1": x.line1,
                    "line2": x.line2,
                })
            })
            .collect(),
    );
    context_used["rag_ms"] = Value::from(rag_ms);
    _render_context_files(
        &project_dirs,
        &t.context_format,
        &postprocessed_messages,
        &cpath,
    )
    .await
}
