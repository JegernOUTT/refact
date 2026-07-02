use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::call_validation::{ContextFile, ContextEnum};
use crate::at_commands::execute_at::{AtCommandMember, correct_at_arg};
use crate::custom_error::trace_and_default;
// use strsim::jaro_winkler;

#[derive(Debug)]
pub struct AtParamSymbolPathQuery;

impl AtParamSymbolPathQuery {
    pub fn new() -> Self {
        Self {}
    }
}

// fn full_path_score(path: &str, query: &str) -> f32 {
//     if jaro_winkler(&path, &query) <= 0.0 {
//         return 0.0;
//     }
//     let mut score = 1.0;
//     for query_comp in query.split("::") {
//         for (idx, p) in path.split("::").collect::<Vec<_>>().into_iter().rev().enumerate() {
//             let current_score = jaro_winkler(&query_comp, &p) as f32;
//             // quick exit if we have a full match in the name
//             if current_score >= 0.99 {
//                 return score;
//             }
//             score *= current_score * (1.0 / (idx + 1) as f32);
//         }
//     }
//     score
// }

pub struct AtAstDefinition {
    pub params: Vec<Box<dyn AtParam>>,
}

impl AtAstDefinition {
    pub fn new() -> Self {
        AtAstDefinition {
            params: vec![Box::new(AtParamSymbolPathQuery::new())],
        }
    }
}

#[async_trait]
impl AtParam for AtParamSymbolPathQuery {
    async fn is_value_valid(&self, _ccx: Arc<AMutex<AtCommandsContext>>, value: &String) -> bool {
        !value.is_empty()
    }

    async fn param_completion(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        value: &String,
    ) -> Vec<String> {
        if value.is_empty() {
            return vec![];
        }
        let (app, top_n) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.clone(), cgcx.top_n)
        };

        let gcx = app.gcx.clone();
        let codegraph_opt = gcx.codegraph.lock().await.clone();
        match codegraph_opt {
            Some(service) => service
                .definition_paths_fuzzy(value, top_n)
                .await
                .unwrap_or_else(trace_and_default),
            None => vec![],
        }
    }

    fn param_completion_valid(&self) -> bool {
        true
    }
}

#[async_trait]
impl AtCommand for AtAstDefinition {
    fn params(&self) -> &Vec<Box<dyn AtParam>> {
        &self.params
    }

    async fn at_execute(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        cmd: &mut AtCommandMember,
        args: &mut Vec<AtCommandMember>,
    ) -> Result<(Vec<ContextEnum>, String), String> {
        let mut arg_symbol = match args.get(0) {
            Some(x) => x.clone(),
            None => {
                cmd.ok = false;
                cmd.reason = Some("parameter is missing".to_string());
                args.clear();
                return Err("parameter `symbol` is missing".to_string());
            }
        };

        correct_at_arg(ccx.clone(), &self.params[0], &mut arg_symbol).await;
        args.clear();
        args.push(arg_symbol.clone());

        let app = ccx.lock().await.app.clone();
        let gcx = app.gcx.clone();

        let codegraph_opt = gcx.codegraph.lock().await.clone();
        match codegraph_opt {
            Some(service) => {
                let defs = service.definitions(arg_symbol.text.as_str()).await?;
                Ok(build_definition_context(gcx.clone(), &arg_symbol.text, &defs).await)
            }
            None => Err("codegraph is not available".to_string()),
        }
    }

    fn depends_on(&self) -> Vec<String> {
        vec!["codegraph".to_string()]
    }
}

async fn build_definition_context(
    gcx: Arc<crate::global_context::GlobalContext>,
    symbol_text: &str,
    defs: &[Arc<refact_core::ast_types::AstDefinition>],
) -> (Vec<ContextEnum>, String) {
    let file_paths = defs.iter().map(|x| x.cpath.clone()).collect::<Vec<_>>();
    let short_file_paths = crate::files_correction::shortify_paths(gcx.clone(), &file_paths).await;

    let text = if let Some(path0) = short_file_paths.get(0) {
        if short_file_paths.len() > 1 {
            format!("`{}` (defined in {} and other files)", symbol_text, path0)
        } else {
            format!("`{}` (defined in {})", symbol_text, path0)
        }
    } else {
        format!(
            "`{}` (definition not found in the project index)",
            symbol_text
        )
    };

    let mut result = vec![];
    for (res, cpath) in defs.iter().zip(file_paths.iter()) {
        result.push(ContextFile {
            file_name: cpath.clone(),
            file_content: "".to_string(),
            line1: res.full_line1(),
            line2: res.full_line2(),
            file_rev: None,
            symbols: vec![res.path_drop0()],
            gradient_type: 4,
            usefulness: 100.0,
            skip_pp: false,
        });
    }
    (
        result
            .into_iter()
            .map(ContextEnum::ContextFile)
            .collect::<Vec<ContextEnum>>(),
        text,
    )
}
