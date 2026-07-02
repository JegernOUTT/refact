use serde::{Deserialize, Serialize};
use std::sync::Arc;
use axum::extract::State;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use url::Url;

use refact_core::ast_types::AstDefinition;
use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use refact_core::ast_types::SymbolType;

#[derive(Deserialize)]
pub struct CodeLensPost {
    pub uri: Url,
    #[serde(default)]
    pub debug: bool,
}

#[derive(Serialize, Clone)]
struct CodeLensResponse {
    success: u8,
    code_lens: Vec<CodeLensOutput>,
}

#[derive(Serialize, Clone)]
struct CodeLensOutput {
    spath: String,
    line1: usize,
    line2: usize,
    debug_string: Option<String>,
}

pub async fn handle_v1_code_lens(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<CodeLensPost>(&body_bytes).map_err(|e| {
        tracing::info!("chat handler cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cpath = crate::files_correction::canonical_path(
        &post
            .uri
            .to_file_path()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
    );
    let cpath_str = cpath.to_string_lossy().to_string();

    let codegraph_opt = global_context.codegraph.lock().await.clone();
    let defs: Vec<Arc<AstDefinition>> = match codegraph_opt {
        Some(service) => service.doc_defs(&cpath_str).await.unwrap_or_default(),
        None => {
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(
                    serde_json::json!({"detail": "codegraph turned off"}).to_string(),
                ))
                .unwrap());
        }
    };

    let output = build_code_lens_output(&defs, post.debug);

    let response = CodeLensResponse {
        success: 1,
        code_lens: output,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

fn build_code_lens_output(defs: &[Arc<AstDefinition>], debug: bool) -> Vec<CodeLensOutput> {
    let mut output: Vec<CodeLensOutput> = Vec::new();
    for def in defs.iter() {
        if let Some(last) = def.official_path.last() {
            if last == "root" {
                continue;
            }
        }
        if !debug {
            let line1 = def.full_line1();
            let line2 = def.full_line2();
            if line2 > line1 {
                output.push(CodeLensOutput {
                    spath: def.path_drop0(),
                    line1,
                    line2,
                    debug_string: None,
                });
            }
        } else {
            let line1 = def.full_line1();
            let line2 = def.full_line2();
            let mut entity_char = 'D';
            if def.symbol_type == SymbolType::VariableDefinition {
                entity_char = '📦';
            } else if def.symbol_type == SymbolType::StructDeclaration {
                entity_char = '📂';
            } else if def.symbol_type == SymbolType::FunctionDeclaration {
                entity_char = '⭐';
            }
            output.push(CodeLensOutput {
                spath: "".to_string(),
                line1,
                line2,
                debug_string: Some(format!("{entity_char}({})", def.path_drop0())),
            });
            for u in def.usages.iter() {
                let resolved = u
                    .resolved_as
                    .rsplit("::")
                    .take(2)
                    .collect::<Vec<&str>>()
                    .iter()
                    .rev()
                    .cloned()
                    .collect::<Vec<&str>>()
                    .join("::");
                let txt = if resolved != "" {
                    format!("↗{}", resolved)
                } else {
                    format!(
                        "❌{}",
                        u.targets_for_guesswork.get(0).unwrap_or(&"".to_string())
                    )
                };
                output.push(CodeLensOutput {
                    spath: "".to_string(),
                    line1: u.uline + 1,
                    line2: u.uline + 1,
                    debug_string: Some(txt),
                });
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_includes_codegraph_doc_def_usages() {
        let store = refact_codegraph::Store::open_in_memory().unwrap();
        store
            .index_file_graph(
                "src/widget.rs",
                "fn caller() {\n    helper();\n}\nfn helper() {}\n",
                "rust",
            )
            .unwrap();
        store.connect_usages().unwrap();

        let defs = refact_codegraph::facade::doc_defs(&store, "src/widget.rs").unwrap();
        let output = build_code_lens_output(&defs, true);

        assert!(output.iter().any(|lens| {
            lens.line1 == 2 && lens.debug_string.as_deref() == Some("↗src/widget.rs::helper")
        }));
    }
}
