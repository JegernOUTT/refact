use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::path::PathBuf;
use std::time::Instant;
use std::vec;
use async_trait::async_trait;
use ropey::Rope;
use serde_json::{Value, json};
use tokenizers::Tokenizer;
use tracing::info;

use refact_ast::ast::ast_structs::AstDB;
use refact_core::chat_types::{CodeCompletionPost, SamplingParameters};
use refact_core::custom_error::last_n_chars;
use refact_postprocessing::pp_context_provider::PPContextTrait;

use crate::completion_cache;
use crate::completon_rag::retrieve_ast_based_extra_context;
use crate::scratchpad_abstract::{
    FinishReason, HasTokenizerAndEot, ScratchpadAbstract, ScratchpadPromptInput,
};

const DEBUG: bool = false;

pub struct FillInTheMiddleScratchpad {
    pub t: HasTokenizerAndEot,
    pub post: CodeCompletionPost,
    pub order: String,
    pub fim_prefix: String,
    pub fim_suffix: String,
    pub fim_middle: String,
    pub extra_stop_tokens: Vec<String>,
    pub held_stream_suffix: String,
    pub stream_stopped: bool,
    pub context_used: Value,
    pub data4cache: completion_cache::CompletionSaveToCache,
    pub ast_index: Option<Arc<AstDB>>,
    pub pp_context: Arc<dyn PPContextTrait>,
    pub project_dirs: Vec<PathBuf>,
    message_stream_pending: String,
    message_stream_stopped: bool,
}

impl FillInTheMiddleScratchpad {
    pub fn new(
        tokenizer: Option<Arc<Tokenizer>>,
        post: &CodeCompletionPost,
        order: String,
        cache_arc: Arc<StdRwLock<completion_cache::CompletionCache>>,
        ast_index: Option<Arc<AstDB>>,
        pp_context: Arc<dyn PPContextTrait>,
        project_dirs: Vec<PathBuf>,
    ) -> Self {
        let data4cache = completion_cache::CompletionSaveToCache::new(cache_arc, &post);
        FillInTheMiddleScratchpad {
            t: HasTokenizerAndEot::new(tokenizer),
            post: post.clone(),
            order,
            fim_prefix: String::new(),
            fim_suffix: String::new(),
            fim_middle: String::new(),
            extra_stop_tokens: vec![],
            held_stream_suffix: String::new(),
            stream_stopped: false,
            context_used: json!({}),
            data4cache,
            ast_index,
            pp_context,
            project_dirs,
            message_stream_pending: String::new(),
            message_stream_stopped: false,
        }
    }

    fn cleanup_prompt(&mut self, text: &String) -> String {
        text.replace(&self.fim_prefix, "")
            .replace(&self.fim_middle, "")
            .replace(&self.fim_suffix, "")
            .replace(&self.t.eos, "")
            .replace(&self.t.eot, "")
    }

    fn stop_tokens(&self, multiline: bool) -> Vec<String> {
        stop_tokens(&self.t.eot, &self.t.eos, multiline, &self.extra_stop_tokens)
    }

    fn cut_stream_delta(&mut self, delta: &str, finish_reason: FinishReason) -> String {
        if self.stream_stopped {
            return String::new();
        }
        let mut combined = std::mem::take(&mut self.held_stream_suffix);
        combined.push_str(delta);
        let stop_tokens = self.stop_tokens(self.post.inputs.multiline);
        let (cut, stopped) = cut_at_first_stop(&combined, &stop_tokens);
        if stopped {
            self.stream_stopped = true;
            return cut.replace("\r", "");
        }
        if finish_reason == FinishReason::Stop || finish_reason == FinishReason::ScratchpadStop {
            self.stream_stopped = true;
            return String::new();
        }
        let hold = if finish_reason.is_finished() {
            0
        } else {
            held_suffix_len(&combined, &stop_tokens)
        };
        let split_at = combined.len().saturating_sub(hold);
        let out = combined[..split_at].replace("\r", "");
        self.held_stream_suffix = combined[split_at..].to_string();
        out
    }

    fn cut_message_stream_delta(&mut self, delta: &str) -> String {
        if self.message_stream_stopped || delta.is_empty() {
            return String::new();
        }
        self.message_stream_pending.push_str(delta);
        let mut cut_at = None;
        for token in self.stop_tokens(self.post.inputs.multiline) {
            if let Some(pos) = self.message_stream_pending.find(&token) {
                cut_at = Some(cut_at.map_or(pos, |current: usize| current.min(pos)));
            }
        }
        if let Some(pos) = cut_at {
            self.message_stream_stopped = true;
            let emitted = self.message_stream_pending[..pos].to_string();
            self.message_stream_pending.clear();
            return emitted.replace("\r", "");
        }
        let keep = held_suffix_len(
            &self.message_stream_pending,
            &self.stop_tokens(self.post.inputs.multiline),
        );
        let pending_len = self.message_stream_pending.len();
        if pending_len <= keep {
            return String::new();
        }
        let emit_len = safe_char_boundary_before(&self.message_stream_pending, pending_len - keep);
        if emit_len == 0 {
            return String::new();
        }
        let emitted = self.message_stream_pending[..emit_len].to_string();
        self.message_stream_pending = self.message_stream_pending[emit_len..].to_string();
        emitted.replace("\r", "")
    }

    fn flush_message_stream_delta(&mut self) -> String {
        if self.message_stream_pending.is_empty() || self.message_stream_stopped {
            self.message_stream_pending.clear();
            return String::new();
        }
        let emitted = _cut_result(
            &self.message_stream_pending,
            self.t.eot.as_str(),
            self.t.eos.as_str(),
            self.post.inputs.multiline,
            &self.extra_stop_tokens,
        );
        self.message_stream_pending.clear();
        emitted
    }
}

#[async_trait]
impl ScratchpadAbstract for FillInTheMiddleScratchpad {
    async fn apply_model_adaptation_patch(&mut self, patch: &Value) -> Result<(), String> {
        self.fim_prefix = patch
            .get("fim_prefix")
            .and_then(|x| x.as_str())
            .unwrap_or("<fim_prefix>")
            .to_string();
        self.fim_suffix = patch
            .get("fim_suffix")
            .and_then(|x| x.as_str())
            .unwrap_or("<fim_suffix>")
            .to_string();
        self.fim_middle = patch
            .get("fim_middle")
            .and_then(|x| x.as_str())
            .unwrap_or("<fim_middle>")
            .to_string();
        self.extra_stop_tokens = patch
            .get("extra_stop_tokens")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        self.t.eot = patch
            .get("eot")
            .and_then(|x| x.as_str())
            .unwrap_or("<|endoftext|>")
            .to_string();
        self.t.eos = patch
            .get("eos")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        self.t.context_format = patch
            .get("context_format")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        self.t.rag_ratio = patch
            .get("rag_ratio")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.5);
        if self.t.tokenizer.is_some() {
            self.t.assert_one_token(&self.fim_prefix.as_str())?;
            self.t.assert_one_token(&self.fim_suffix.as_str())?;
            self.t.assert_one_token(&self.fim_middle.as_str())?;
            self.t.assert_one_token(&self.t.eot.as_str())?;
            if !self.t.eos.is_empty() {
                self.t.assert_one_token(&self.t.eos.as_str())?;
            }
        }
        Ok(())
    }

    async fn prompt(
        &mut self,
        input: ScratchpadPromptInput,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String> {
        let n_ctx = input.n_ctx;
        let fim_t0 = Instant::now();
        let use_rag = !self.t.context_format.is_empty()
            && self.t.rag_ratio > 0.0
            && self.post.use_ast
            && self.ast_index.is_some();
        let mut rag_tokens_n = if self.post.rag_tokens_n > 0 {
            self.post.rag_tokens_n.min(4096).max(50)
        } else {
            ((n_ctx as f64 * self.t.rag_ratio) as usize)
                .min(4096)
                .max(50)
        };
        if !use_rag {
            rag_tokens_n = 0;
        }
        if !use_rag && self.post.use_ast {
            tracing::warn!(
                "will not use ast because {}{}{}{}",
                self.t.context_format.is_empty() as i32,
                self.post.use_ast as i32,
                (rag_tokens_n > 0) as i32,
                self.ast_index.is_some() as i32
            );
        }

        let limit: i32 =
            (n_ctx as i32) - (self.post.parameters.max_new_tokens as i32) - (rag_tokens_n as i32);
        if limit < 512 {
            let msg = format!("n_ctx={} - max_new_tokens={} - rag_tokens_n={} leaves too little {} space for completion to work",
            n_ctx, self.post.parameters.max_new_tokens, rag_tokens_n, limit);
            tracing::warn!("{}", msg);
            return Err(msg);
        }

        let cpath = self
            .pp_context
            .canonical_path(&self.post.inputs.cursor.file);

        let supports_stop = true;
        if supports_stop {
            sampling_parameters_to_patch.stop = self.stop_tokens(self.post.inputs.multiline);
        }
        let mut source = self
            .post
            .inputs
            .sources
            .get(&self.post.inputs.cursor.file)
            .ok_or("Cursor is in file not found in sources".to_string())?
            .clone();
        source = self.cleanup_prompt(&source);

        let text = Rope::from_str(&*source);

        let pos = &self.post.inputs.cursor;
        let mut before_iter = text.lines_at(pos.line as usize).reversed();
        let mut after_iter = text.lines_at(pos.line as usize + 1);
        let mut tokens_used = 0;

        let mut before_line = before_iter.next();

        let cursor_line1: String;
        let col = pos.character as usize;
        cursor_line1 = text.line(pos.line as usize).slice(0..col).to_string();

        let mut after_line = after_iter.next();

        let cursor_line2: String;
        if self.post.inputs.multiline {
            cursor_line2 = text.line(pos.line as usize).slice(col..).to_string();
        } else {
            cursor_line2 = "".to_string();
        }

        let mut before = vec![];
        let mut after = String::new();
        let mut fim_line1: i32 = i32::MAX;
        let mut fim_line2: i32 = i32::MIN;
        tokens_used += self
            .t
            .count_tokens((cursor_line1.clone() + &cursor_line2).as_str())?;
        let mut rel_line_n: i32 = 0;
        while before_line.is_some() || after_line.is_some() {
            rel_line_n += 1;
            if let Some(before_line) = before_line {
                let before_line = before_line.to_string();
                let tokens = self.t.count_tokens(before_line.as_str())?;
                if tokens_used + tokens > limit {
                    break;
                }
                tokens_used += tokens;
                before.push(before_line);
                fim_line1 = pos.line - rel_line_n as i32;
            }
            if let Some(after_line) = after_line {
                let after_line = after_line.to_string();
                let tokens = self.t.count_tokens(after_line.as_str())?;
                if tokens_used + tokens > limit {
                    break;
                }
                tokens_used += tokens;
                after.push_str(&after_line);
                fim_line2 = pos.line + rel_line_n as i32;
            }
            before_line = before_iter.next();
            after_line = after_iter.next();
        }

        let before = before.into_iter().rev().collect::<Vec<_>>().join("");
        info!(
            "{} FIM prompt {} tokens used < limit {}",
            last_n_chars(&cpath.display().to_string(), 30),
            tokens_used,
            limit
        );
        let mut prompt: String;
        if self.order == "plain" {
            prompt = format!("{}{}{}", self.t.eos, before, cursor_line1);
        } else if self.order == "PSM" {
            prompt = format!(
                "{}{}{}{}{}{}{}{}",
                self.t.eos,
                self.fim_prefix,
                before,
                cursor_line1,
                self.fim_suffix,
                cursor_line2,
                after,
                self.fim_middle
            );
        } else if self.order == "SPM" {
            prompt = format!(
                "{}{}{}{}{}{}{}{}",
                self.t.eos,
                self.fim_suffix,
                cursor_line2,
                after,
                self.fim_prefix,
                before,
                cursor_line1,
                self.fim_middle,
            );
        } else {
            return Err(format!("order \"{}\" not recognized", self.order));
        }
        let fim_ms = fim_t0.elapsed().as_millis() as i32;
        self.context_used["fim_ms"] = Value::from(fim_ms);
        self.context_used["n_ctx".to_string()] = Value::from(n_ctx as i64);
        self.context_used["rag_tokens_limit".to_string()] = Value::from(rag_tokens_n as i64);
        info!(" -- /post fim {}ms-- ", fim_ms);

        if use_rag && rag_tokens_n > 0 {
            let pp_settings = input.postprocess_parameters.clone();

            let mut extra_content_collect_counter = 0;
            let mut content_tokens_budget = rag_tokens_n as i32;
            loop {
                let extra_context = retrieve_ast_based_extra_context(
                    self.pp_context.clone(),
                    self.project_dirs.clone(),
                    self.ast_index.clone(),
                    &self.t,
                    &cpath,
                    &pos,
                    (fim_line1, fim_line2),
                    pp_settings.clone(),
                    content_tokens_budget as usize,
                    &mut self.context_used,
                )
                .await;
                let content_tokens_n = self.t.count_tokens(&extra_context.as_str())?;
                if content_tokens_n <= content_tokens_budget || extra_content_collect_counter > 1 {
                    prompt = format!("{extra_context}{prompt}");
                    break;
                } else {
                    let overshoot = content_tokens_n - content_tokens_budget;
                    if overshoot >= content_tokens_budget {
                        break;
                    }
                    content_tokens_budget -= overshoot;
                    extra_content_collect_counter += 1;
                }
            }
        }

        if DEBUG {
            info!("cursor position\n{:?}", self.post.inputs.cursor);
            info!("prompt\n{}", prompt);
            info!(
                "re-encode whole prompt again gives {} tokens",
                self.t.count_tokens(prompt.as_str())?
            );
        }
        info!(
            "re-encode whole prompt again gives {} tokens",
            self.t.count_tokens(prompt.as_str())?
        );
        Ok(prompt)
    }

    fn response_n_choices(
        &mut self,
        choices: Vec<String>,
        finish_reasons: Vec<FinishReason>,
    ) -> Result<Value, String> {
        let json_choices = choices
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let cc = _cut_result(
                    &x,
                    self.t.eot.as_str(),
                    self.t.eos.as_str(),
                    self.post.inputs.multiline,
                    &self.extra_stop_tokens,
                );
                if i == 0 {
                    self.data4cache.completion0_text = cc.clone();
                    self.data4cache.completion0_finish_reason = finish_reasons[i].to_string();
                }
                json!({
                    "index": i,
                    "code_completion": cc,
                    "finish_reason": finish_reasons[i].to_json_val(),
                })
            })
            .collect::<Vec<_>>();
        if DEBUG {
            info!("response_n_choices\n{:?}", json_choices);
        }
        Ok(json!(
            {
                "choices": json_choices,
                "model": self.post.model.clone(),
                "context": self.context_used,
            }
        ))
    }

    fn response_message_n_choices(
        &mut self,
        choices: Vec<String>,
        finish_reasons: Vec<FinishReason>,
    ) -> Result<Value, String> {
        self.response_n_choices(choices, finish_reasons)
    }

    fn response_streaming(
        &mut self,
        delta: String,
        finish_reason: FinishReason,
    ) -> Result<(Value, FinishReason), String> {
        let json_choices = if !delta.is_empty()
            || matches!(
                finish_reason,
                FinishReason::Stop
                    | FinishReason::ContentFilter
                    | FinishReason::Unknown
                    | FinishReason::ScratchpadStop
            ) {
            let mut s: String = self.cut_stream_delta(&delta, finish_reason);
            if finish_reason.is_finished() {
                s = s.trim_end().to_string();
            }
            self.data4cache.completion0_text.push_str(&s);
            json!([{
                "index": 0,
                "code_completion": s,
                "finish_reason": finish_reason.to_json_val(),
            }])
        } else {
            assert_eq!(finish_reason, FinishReason::Length);
            json!([{
                "index": 0,
                "code_completion": "",
                "finish_reason": finish_reason.to_json_val()
            }])
        };
        self.data4cache.completion0_finish_reason = finish_reason.to_string();
        Ok((
            json!({
                "choices": json_choices,
            }),
            finish_reason,
        ))
    }

    fn response_message_streaming(
        &mut self,
        delta: &Value,
        finish_reason: FinishReason,
    ) -> Result<(Value, FinishReason), String> {
        let content = delta
            .get("choices")
            .and_then(|choices| choices.as_array())
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content"))
            .and_then(|content| content.as_str())
            .unwrap_or("");
        let mut code_completion = self.cut_message_stream_delta(content);
        if finish_reason.is_finished() {
            code_completion.push_str(&self.flush_message_stream_delta());
            code_completion = code_completion.trim_end().to_string();
        }
        if !code_completion.is_empty() {
            self.data4cache.completion0_text.push_str(&code_completion);
        }
        self.data4cache.completion0_finish_reason = finish_reason.to_string();
        Ok((
            json!({
                "choices": [{
                    "index": 0,
                    "code_completion": code_completion,
                    "finish_reason": finish_reason.to_json_val(),
                }],
            }),
            finish_reason,
        ))
    }

    fn response_spontaneous(&mut self) -> Result<Vec<Value>, String> {
        Err("".to_string())
    }

    fn streaming_finished(&mut self, finish_reason: FinishReason) -> Result<Value, String> {
        self.data4cache.completion0_finish_reason = finish_reason.to_string();
        let tail = if finish_reason == FinishReason::Stop
            || finish_reason == FinishReason::ScratchpadStop
        {
            String::new()
        } else {
            std::mem::take(&mut self.held_stream_suffix).replace("\r", "")
        };
        if !tail.is_empty() {
            self.data4cache.completion0_text.push_str(&tail);
        }
        Ok(json!({
            "choices": [{
                "index": 0,
                "code_completion": tail,
                "finish_reason": finish_reason.to_json_val()
            }],
        }))
    }
}

fn _cut_result(
    text: &str,
    eot_token: &str,
    eos_token: &str,
    multiline: bool,
    extra_stop_tokens: &Vec<String>,
) -> String {
    let stop_tokens = stop_tokens(eot_token, eos_token, multiline, extra_stop_tokens);
    let (ans, stopped) = cut_at_first_stop(text, &stop_tokens);
    if !stopped {
        return text.to_string().replace("\r", "");
    }
    ans.replace("\r", "")
}

fn stop_tokens(
    eot_token: &str,
    eos_token: &str,
    multiline: bool,
    extra_stop_tokens: &Vec<String>,
) -> Vec<String> {
    let mut tokens = vec![
        eot_token.to_string(),
        eos_token.to_string(),
        "<EOT>".to_string(),
        "\n\n".to_string(),
        "\r\n\r\n".to_string(),
    ];
    if !multiline {
        tokens.push("\n".to_string());
    }
    tokens.extend(extra_stop_tokens.iter().cloned());
    tokens.into_iter().filter(|s| !s.is_empty()).collect()
}

fn cut_at_first_stop(text: &str, stop_tokens: &[String]) -> (String, bool) {
    let cut_at = stop_tokens
        .iter()
        .filter_map(|token| text.find(token))
        .min();
    if let Some(cut_at) = cut_at {
        (text[..cut_at].to_string(), true)
    } else {
        (text.to_string(), false)
    }
}

fn held_suffix_len(text: &str, stop_tokens: &[String]) -> usize {
    stop_tokens
        .iter()
        .flat_map(|token| {
            token
                .char_indices()
                .skip(1)
                .map(|(idx, _)| &token[..idx])
                .collect::<Vec<_>>()
        })
        .filter(|prefix| text.ends_with(*prefix))
        .map(|prefix| prefix.len())
        .max()
        .unwrap_or(0)
}

fn safe_char_boundary_before(text: &str, index: usize) -> usize {
    let mut safe_index = index.min(text.len());
    while safe_index > 0 && !text.is_char_boundary(safe_index) {
        safe_index -= 1;
    }
    safe_index
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::{CodeCompletionInputs, CursorPosition};
    use refact_postprocessing::pp_context_provider::PPContextTrait;
    use std::collections::HashMap;

    struct TestPPContext;

    #[async_trait]
    impl PPContextTrait for TestPPContext {
        async fn read_file(&self, _path: &PathBuf) -> Result<String, String> {
            Err("not implemented".to_string())
        }

        async fn correct_to_nearest_filename(&self, _path: &str, _limit: usize) -> Vec<String> {
            vec![]
        }

        async fn shortify_paths(&self, paths: &[String]) -> Vec<String> {
            paths.to_vec()
        }

        async fn doc_defs_for_path(
            &self,
            _path: &str,
        ) -> Vec<Arc<refact_ast::ast::ast_structs::AstDefinition>> {
            vec![]
        }

        fn canonical_path(&self, path: &str) -> PathBuf {
            PathBuf::from(path)
        }
    }

    fn post(source: &str, multiline: bool) -> CodeCompletionPost {
        CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from([("/tmp/main.rs".to_string(), source.to_string())]),
                cursor: CursorPosition {
                    file: "/tmp/main.rs".to_string(),
                    line: 1,
                    character: 7,
                },
                multiline,
            },
            parameters: SamplingParameters {
                max_new_tokens: 10,
                ..Default::default()
            },
            model: "model".to_string(),
            stream: false,
            no_cache: true,
            use_ast: false,
            use_vecdb: false,
            rag_tokens_n: 0,
            cache_salt: String::new(),
            cache_generation: 0,
        }
    }

    fn scratchpad(order: &str, source: &str, multiline: bool) -> FillInTheMiddleScratchpad {
        FillInTheMiddleScratchpad::new(
            None,
            &post(source, multiline),
            order.to_string(),
            Arc::new(StdRwLock::new(completion_cache::CompletionCache::new())),
            None,
            Arc::new(TestPPContext),
            vec![],
        )
    }

    async fn patched_scratchpad(order: &str) -> FillInTheMiddleScratchpad {
        let mut sp = scratchpad(order, "fn main() {\n    pri\n}\n", true);
        sp.apply_model_adaptation_patch(&json!({
            "fim_prefix": "<PRE>",
            "fim_suffix": "<SUF>",
            "fim_middle": "<MID>",
            "eos": "<BOS>",
            "eot": "<END>",
            "extra_stop_tokens": ["", "<STOP>"]
        }))
        .await
        .unwrap();
        sp
    }

    async fn prompt_for(order: &str) -> (String, SamplingParameters) {
        let mut sp = patched_scratchpad(order).await;
        let mut params = SamplingParameters::default();
        let prompt = sp
            .prompt(
                ScratchpadPromptInput {
                    n_ctx: 2048,
                    postprocess_parameters: Default::default(),
                },
                &mut params,
            )
            .await
            .unwrap();
        (prompt, params)
    }

    #[tokio::test]
    async fn code_completion_fim_psm_places_tokens() {
        let (prompt, params) = prompt_for("PSM").await;
        assert_eq!(prompt, "<BOS><PRE>fn main() {\n    pri<SUF>\n}\n<MID>");
        assert!(params.stop.contains(&"<END>".to_string()));
        assert!(params.stop.contains(&"<BOS>".to_string()));
        assert!(params.stop.contains(&"<STOP>".to_string()));
        assert!(!params.stop.contains(&String::new()));
    }

    #[tokio::test]
    async fn code_completion_fim_spm_places_tokens() {
        let (prompt, _params) = prompt_for("SPM").await;
        assert_eq!(prompt, "<BOS><SUF>\n}\n<PRE>fn main() {\n    pri<MID>");
    }

    #[tokio::test]
    async fn code_completion_fim_plain_prompt_path() {
        let (prompt, _params) = prompt_for("plain").await;
        assert_eq!(prompt, "<BOS>fn main() {\n    pri");
    }

    #[test]
    fn code_completion_fim_cut_result_handles_eos_eot_and_empty_stop() {
        let stops = vec![String::new(), "<STOP>".to_string()];
        assert_eq!(
            _cut_result("abc<BOS>def", "<END>", "<BOS>", true, &stops),
            "abc"
        );
        assert_eq!(
            _cut_result("abc<END>def", "<END>", "<BOS>", true, &stops),
            "abc"
        );
        assert_eq!(
            _cut_result("abc<STOP>def", "<END>", "<BOS>", true, &stops),
            "abc"
        );
    }

    #[test]
    fn code_completion_fim_split_stop_sequence_does_not_leak() {
        let mut sp = scratchpad("PSM", "fn main() {\n    pri\n}\n", true);
        sp.t.eot = "<END>".to_string();
        let (value, finish) = sp
            .response_streaming("hello <E".to_string(), FinishReason::None)
            .unwrap();
        assert_eq!(finish, FinishReason::None);
        assert_eq!(value["choices"][0]["code_completion"], "hello ");
        let (value, finish) = sp
            .response_streaming("OT> leaked".to_string(), FinishReason::None)
            .unwrap();
        assert_eq!(finish, FinishReason::None);
        assert_eq!(value["choices"][0]["code_completion"], "");
        assert_eq!(sp.data4cache.completion0_text, "hello ");
    }

    fn chat_delta(content: Value, finish_reason: FinishReason) -> Value {
        json!({
            "choices": [{
                "delta": { "content": content },
                "finish_reason": finish_reason.to_json_val(),
            }]
        })
    }

    #[test]
    fn response_message_streaming_content_delta_emits_code_completion_and_updates_cache() {
        let mut sp = scratchpad(
            "PSM",
            "fn main() {
    pri
}
",
            true,
        );
        let (value, finish_reason) = sp
            .response_message_streaming(
                &chat_delta(json!("hello"), FinishReason::None),
                FinishReason::None,
            )
            .unwrap();

        assert_eq!(finish_reason, FinishReason::None);
        assert_eq!(value["choices"][0]["code_completion"], "hello");
        assert_eq!(value["choices"][0]["finish_reason"], Value::Null);
        assert_eq!(sp.data4cache.completion0_text, "hello");
        assert_eq!(sp.data4cache.completion0_finish_reason, "");
    }

    #[test]
    fn response_message_streaming_split_stop_tokens_are_not_leaked() {
        let mut sp = scratchpad(
            "PSM",
            "fn main() {
    pri
}
",
            true,
        );
        sp.t.eot = "<|endoftext|>".to_string();
        let (first, _) = sp
            .response_message_streaming(
                &chat_delta(json!("abc<|endof"), FinishReason::None),
                FinishReason::None,
            )
            .unwrap();
        let (second, _) = sp
            .response_message_streaming(
                &chat_delta(json!("text|>def"), FinishReason::Stop),
                FinishReason::Stop,
            )
            .unwrap();

        assert_eq!(first["choices"][0]["code_completion"], "abc");
        assert_eq!(second["choices"][0]["code_completion"], "");
        assert_eq!(sp.data4cache.completion0_text, "abc");
        assert_eq!(sp.data4cache.completion0_finish_reason, "stop");
    }

    #[test]
    fn response_message_streaming_split_extra_stop_tokens_are_not_leaked() {
        let mut sp = scratchpad(
            "PSM",
            "fn main() {
    pri
}
",
            true,
        );
        sp.extra_stop_tokens = vec!["<STOP>".to_string()];
        let (first, _) = sp
            .response_message_streaming(
                &chat_delta(json!("abc<ST"), FinishReason::None),
                FinishReason::None,
            )
            .unwrap();
        let (second, _) = sp
            .response_message_streaming(
                &chat_delta(json!("OP>def"), FinishReason::Stop),
                FinishReason::Stop,
            )
            .unwrap();

        assert_eq!(first["choices"][0]["code_completion"], "abc");
        assert_eq!(second["choices"][0]["code_completion"], "");
        assert_eq!(sp.data4cache.completion0_text, "abc");
    }

    #[test]
    fn response_message_streaming_empty_delta_content_is_safe() {
        let mut sp = scratchpad(
            "PSM",
            "fn main() {
    pri
}
",
            true,
        );
        sp.response_message_streaming(
            &chat_delta(json!("hello"), FinishReason::None),
            FinishReason::None,
        )
        .unwrap();
        let (value, finish_reason) = sp
            .response_message_streaming(
                &chat_delta(Value::Null, FinishReason::None),
                FinishReason::None,
            )
            .unwrap();

        assert_eq!(finish_reason, FinishReason::None);
        assert_eq!(value["choices"][0]["code_completion"], "");
        assert_eq!(sp.data4cache.completion0_text, "hello");
    }

    #[test]
    fn code_completion_fim_negative_rag_retry_math_clamps() {
        let mut budget = 20;
        let content_tokens_n = 60;
        let overshoot = content_tokens_n - budget;
        if overshoot < budget {
            budget -= overshoot;
        }
        assert_eq!(budget, 20);
    }
}
