use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AMutex;
use tokio::sync::mpsc;
use async_stream::stream;
use futures::StreamExt;
use hyper::{Body, Response, StatusCode};
use eventsource_stream::Eventsource;
use serde_json::{json, Value};
use tracing::info;
use uuid;

const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const STREAM_TOTAL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const STREAM_HEARTBEAT: Duration = Duration::from_secs(2);

use crate::call_validation::SamplingParameters;
use crate::caps::BaseModelRecord;
use crate::custom_error::ScratchError;
use crate::nicer_logs;
use crate::scratchpad_abstract::{FinishReason, ScratchpadAbstract, ScratchpadPromptInput};
use crate::at_commands::at_commands::AtCommandsContext;

pub async fn scratchpad_interaction_not_stream_json(
    ccx: Arc<AMutex<AtCommandsContext>>,
    scratchpad: &mut Box<dyn ScratchpadAbstract>,
    _scope: String,
    prompt: &str,
    model_rec: &BaseModelRecord,
    parameters: &SamplingParameters, // includes n
    only_deterministic_messages: bool,
) -> Result<serde_json::Value, ScratchError> {
    let t2 = std::time::SystemTime::now();
    let gcx = ccx.lock().await.global_context.clone();
    let client = gcx.http_client.clone();

    let mut save_url: String = String::new();
    let mut model_says = if only_deterministic_messages {
        save_url = "only-det-messages".to_string();
        Ok(Value::Object(serde_json::Map::new()))
    } else {
        crate::forward_to_openai_endpoint::forward_to_openai_style_endpoint(
            &model_rec,
            prompt,
            &client,
            &parameters,
        )
        .await
    }
    .map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("forward_to_endpoint: {}", e),
        )
    })?;
    generate_id_and_index_for_tool_calls_if_missing(&mut model_says);
    info!(
        "forward to endpoint {:.2}ms, url was {}",
        t2.elapsed().unwrap().as_millis() as f64,
        save_url
    );
    crate::global_context::look_for_piggyback_fields(gcx.clone(), &model_says).await;

    let scratchpad_result: Result<serde_json::Value, String>;
    if only_deterministic_messages {
        if let Ok(det_msgs) = scratchpad.response_spontaneous() {
            model_says["deterministic_messages"] = json!(det_msgs);
            model_says["choices"] = serde_json::Value::Array(vec![]);
        }
        scratchpad_result = Ok(model_says.clone());
    } else if let Some(hf_arr) = model_says.as_array() {
        let choices = hf_arr
            .iter()
            .map(|x| {
                x.get("generated_text")
                    .and_then(|val| val.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        tracing::error!("Failed to get generated_text or convert to str");
                        "".to_string()
                    })
            })
            .collect::<Vec<_>>();
        let finish_reasons = vec![FinishReason::Length; choices.len()];
        scratchpad_result = scratchpad.response_n_choices(choices, finish_reasons);
    } else if let Some(oai_choices) = model_says.clone().get("choices") {
        let choices_arr = oai_choices.as_array().ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "choices is not an array".to_string(),
            )
        })?;
        if choices_arr.is_empty() {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "choices array is empty".to_string(),
            ));
        }
        let choice0 = &choices_arr[0];
        let finish_reasons = choices_arr
            .iter()
            .map(|x| {
                FinishReason::from_json_val(x.get("finish_reason").unwrap_or(&json!("")))
                    .unwrap_or_else(|err| {
                        tracing::error!(
                            "Couldn't parse finish_reason: {err}. Fallback to finish_reason=null"
                        );
                        FinishReason::None
                    })
            })
            .collect::<Vec<_>>();
        if let Some(_msg) = choice0.get("message") {
            if let Ok(det_msgs) = scratchpad.response_spontaneous() {
                model_says["deterministic_messages"] = json!(det_msgs);
            }
            let choices = choices_arr
                .iter()
                .map(|x| {
                    match (
                        x.get("message"),
                        x.get("message").and_then(|msg| msg.get("content")),
                        x.get("message")
                            .and_then(|msg| msg.get("content"))
                            .and_then(|content| content.as_str()),
                    ) {
                        (Some(_), Some(_), Some(content)) => content.to_string(),
                        (msg, content, as_str) => {
                            tracing::info!(
                                "no text content: msg={:?}, content={:?}, as_str={:?}",
                                msg,
                                content,
                                as_str
                            );
                            "".to_string()
                        }
                    }
                })
                .collect::<Vec<_>>();
            scratchpad_result = scratchpad.response_message_n_choices(choices, finish_reasons);
        } else {
            let choices = choices_arr
                .iter()
                .map(|x| {
                    x.get("text")
                        .and_then(|val| val.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            tracing::error!("Failed to get text or convert to str");
                            "".to_string()
                        })
                })
                .collect::<Vec<_>>();
            scratchpad_result = scratchpad.response_n_choices(choices, finish_reasons);
        }
    } else if let Some(err) = model_says.get("error") {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}", err),
        ));
    } else if let Some(msg) = model_says.get("human_readable_message") {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}", msg),
        ));
    } else if let Some(msg) = model_says.get("detail") {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}", msg),
        ));
    } else {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unrecognized response (1): {:?}", model_says),
        ));
    }

    if let Err(problem) = scratchpad_result {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("scratchpad: {}", problem),
        ));
    }
    return Ok(scratchpad_result.unwrap());
}

pub async fn scratchpad_interaction_not_stream(
    ccx: Arc<AMutex<AtCommandsContext>>,
    scratchpad: &mut Box<dyn ScratchpadAbstract>,
    scope: String,
    model_rec: &BaseModelRecord,
    parameters: &mut SamplingParameters,
    only_deterministic_messages: bool,
) -> Result<Response<Body>, ScratchError> {
    let t1 = std::time::Instant::now();
    let prompt_input = {
        let cgcx = ccx.lock().await;
        ScratchpadPromptInput {
            n_ctx: cgcx.n_ctx,
            postprocess_parameters: cgcx.postprocess_parameters.clone(),
        }
    };
    let prompt = scratchpad
        .prompt(prompt_input, parameters)
        .await
        .map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Prompt: {}", e))
        })?;
    info!(
        "scratchpad_interaction_not_stream prompt {:?}",
        t1.elapsed()
    );

    let t2 = std::time::SystemTime::now();
    let mut scratchpad_response_json = scratchpad_interaction_not_stream_json(
        ccx.clone(),
        scratchpad,
        scope,
        prompt.as_str(),
        &model_rec,
        parameters,
        only_deterministic_messages,
    )
    .await?;
    scratchpad_response_json["created"] = json!(t2
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64());
    scratchpad_response_json["compression_strength"] =
        crate::forward_to_openai_endpoint::try_get_compression_from_prompt(&prompt);

    let txt = serde_json::to_string_pretty(&scratchpad_response_json).unwrap();
    // info!("handle_v1_code_completion return {}", txt);
    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(txt))
        .unwrap();
    return Ok(response);
}

pub async fn scratchpad_interaction_stream(
    ccx: Arc<AMutex<AtCommandsContext>>,
    mut scratchpad: Box<dyn ScratchpadAbstract>,
    _scope: String,
    mut model_rec: BaseModelRecord,
    parameters: SamplingParameters,
    only_deterministic_messages: bool,
    pre_stream_messages: Option<Vec<serde_json::Value>>,
) -> Result<Response<Body>, ScratchError> {
    let t1: std::time::SystemTime = std::time::SystemTime::now();
    let evstream = stream! {
        let my_scratchpad: &mut Box<dyn ScratchpadAbstract> = &mut scratchpad;
        let mut my_parameters = parameters.clone();
        let my_ccx = ccx.clone();

        let gcx = ccx.lock().await.global_context.clone();
        let client = gcx.http_client.clone();

        let t0 = std::time::Instant::now();
        let mut prompt = String::new();
        {
            let subchat_tx: Arc<AMutex<mpsc::UnboundedSender<serde_json::Value>>> = my_ccx.lock().await.subchat_tx.clone();
            let subchat_rx: Arc<AMutex<mpsc::UnboundedReceiver<serde_json::Value>>> = my_ccx.lock().await.subchat_rx.clone();
            let prompt_input = {
                let cgcx = my_ccx.lock().await;
                ScratchpadPromptInput {
                    n_ctx: cgcx.n_ctx,
                    postprocess_parameters: cgcx.postprocess_parameters.clone(),
                }
            };
            let mut prompt_future = Some(Box::pin(my_scratchpad.prompt(
                prompt_input,
                &mut my_parameters,
            )));
            // horrible loop that waits for prompt() future, and at the same time retranslates any streaming via my_ccx.subchat_rx/tx to the user
            // (without streaming the rx/tx is never processed, disposed with the ccx)
            loop {
                tokio::select! {
                    value = async {
                        subchat_rx.lock().await.recv().await
                    } => {
                        if let Some(value) = value {
                            let tmp = serde_json::to_string(&value).unwrap();
                            if tmp == "1337" {
                                break;  // the only way out of this loop
                            }
                            let value_str = format!("data: {}\n\n", tmp);
                            yield Result::<_, String>::Ok(value_str);
                        }
                    },
                    prompt_maybe = async {
                        if let Some(fut) = prompt_future.as_mut() {
                            fut.await
                        } else {
                            std::future::pending().await
                        }
                    } => {
                        if let Some(_fut) = prompt_future.take() {
                            prompt = match prompt_maybe {
                                Ok(x) => x,
                                Err(e) => {
                                    // XXX: tool errors go here, check again if this what we want
                                    tracing::warn!("prompt or tool use problem inside prompt: {}", e);
                                    let value_str = format!("data: {}\n\n", serde_json::to_string(&json!({"detail": e})).unwrap());
                                    yield Result::<_, String>::Ok(value_str);
                                    return;
                                }
                            };
                            let _ = subchat_tx.lock().await.send(serde_json::json!(1337));
                        }
                    }
                }
            }
        }
        info!("scratchpad_interaction_stream prompt {:?}", t0.elapsed());

        if let Some(ref messages) = pre_stream_messages {
            for msg in messages {
                let mut msg_with_compression = msg.clone();
                msg_with_compression["compression_strength"] = crate::forward_to_openai_endpoint::try_get_compression_from_prompt(&prompt);
                let value_str = format!("data: {}\n\n", serde_json::to_string(&msg_with_compression).unwrap());
                yield Result::<_, String>::Ok(value_str);
            }
        }

        loop {
            let value_maybe = my_scratchpad.response_spontaneous();
            if let Ok(value) = value_maybe {
                for el in value {
                    let mut el_with_compression = el.clone();
                    el_with_compression["compression_strength"] = crate::forward_to_openai_endpoint::try_get_compression_from_prompt(&prompt);
                    let value_str = format!("data: {}\n\n", serde_json::to_string(&el_with_compression).unwrap());
                    info!("yield: {:?}", nicer_logs::first_n_chars(&value_str, 40));
                    yield Result::<_, String>::Ok(value_str);
                }
            } else {
                let err_str = value_maybe.unwrap_err();
                tracing::error!("response_spontaneous error: {}", err_str);
                let value_str = format!("data: {}\n\n", serde_json::to_string(&json!({"detail": err_str})).unwrap());
                yield Result::<_, String>::Ok(value_str);
            }
            if only_deterministic_messages {
                break;
            }
            // info!("prompt: {:?}", prompt);
            let event_source_maybe = crate::forward_to_openai_endpoint::forward_to_openai_style_endpoint_streaming(
                &model_rec,
                &prompt,
                &client,
                &my_parameters
            ).await;
            let response = match event_source_maybe {
                Ok(resp) => resp,
                Err(e) => {
                    let e_str = format!("forward_to_endpoint: {:?}", e);
                    tracing::error!(e_str);
                    let value_str = format!("data: {}\n\n", serde_json::to_string(&json!({"detail": e_str})).unwrap());
                    yield Result::<_, String>::Ok(value_str);
                    return;
                }
            };
            let mut event_stream = response.bytes_stream().eventsource();
            let mut was_correct_output_even_if_error = false;
            let mut last_finish_reason = FinishReason::None;
            let stream_started_at = Instant::now();
            let mut last_event_at = Instant::now();
            let mut heartbeat = tokio::time::interval(STREAM_HEARTBEAT);
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                let event = tokio::select! {
                    _ = heartbeat.tick() => {
                        if stream_started_at.elapsed() > STREAM_TOTAL_TIMEOUT {
                            let err_str = "LLM stream timeout";
                            tracing::error!("{}", err_str);
                            yield Result::<_, String>::Ok(format!("data: {}\n\n", serde_json::to_string(&json!({"detail": err_str})).unwrap()));
                            return;
                        }
                        if last_event_at.elapsed() > STREAM_IDLE_TIMEOUT {
                            let err_str = "LLM stream stalled";
                            tracing::error!("{}", err_str);
                            yield Result::<_, String>::Ok(format!("data: {}\n\n", serde_json::to_string(&json!({"detail": err_str})).unwrap()));
                            return;
                        }
                        continue;
                    }
                    maybe_event = event_stream.next() => {
                        match maybe_event {
                            Some(e) => e,
                            None => break,
                        }
                    }
                };
                last_event_at = Instant::now();

                match event {
                    Ok(message) => {
                        // info!("Message: {:#?}", message);
                        if message.data.starts_with("[DONE]") {
                            break;
                        }
                        let mut json = serde_json::from_str::<serde_json::Value>(&message.data).unwrap();
                        generate_id_and_index_for_tool_calls_if_missing(&mut json);
                        crate::global_context::look_for_piggyback_fields(gcx.clone(), &json).await;
                        match _push_streaming_json_into_scratchpad(
                            my_scratchpad,
                            &json,
                            &mut model_rec.name,
                            &mut was_correct_output_even_if_error,
                        ) {
                            Ok((mut value, finish_reason)) => {
                                if finish_reason != FinishReason::None { // last event has service info(usage and other), there is no finish_reason
                                    last_finish_reason = finish_reason;
                                }
                                value["created"] = json!(t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64());
                                let value_str = format!("data: {}\n\n", serde_json::to_string(&value).unwrap());
                                // let last_60_chars: String = crate::nicer_logs::first_n_chars(&value_str, 60);
                                // info!("yield: {:?}", last_60_chars);
                                yield Result::<_, String>::Ok(value_str);
                            },
                            Err(err_str) => {
                                tracing::error!("unexpected error: {}", err_str);
                                let value_str = format!("data: {}\n\n", serde_json::to_string(&json!({"detail": err_str})).unwrap());
                                yield Result::<_, String>::Ok(value_str);
                                break;
                            }
                        }
                    },
                    Err(err) => {
                        if was_correct_output_even_if_error {
                            // "restream error: Stream ended"
                            break;
                        }
                        let problem_str = format!("{}", err);
                        tracing::error!("restream error: {}\n", problem_str);
                        yield Result::<_, String>::Ok(format!("data: {}\n\n", serde_json::to_string(&json!({"detail": problem_str})).unwrap()));
                        return;
                    },
                }
            }

            let mut value = my_scratchpad.streaming_finished(last_finish_reason)?;
            value["created"] = json!(t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64());
            value["model"] = json!(model_rec.name.clone());
            let value_str = format!("data: {}\n\n", serde_json::to_string(&value).unwrap());
            info!("yield final: {:?}", value_str);
            yield Result::<_, String>::Ok(value_str);
            break;
        }
        info!("yield: [DONE]");
        yield Result::<_, String>::Ok("data: [DONE]\n\n".to_string());
    };
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::wrap_stream(evstream))
        .unwrap())
}

fn generate_id_and_index_for_tool_calls_if_missing(value: &mut serde_json::Value) {
    fn process_tool_call(tool_call: &mut serde_json::Value, idx: usize) {
        let needs_id = match tool_call.get("id") {
            None => true,
            Some(id) => id.is_null() || (id.is_string() && id.as_str().unwrap_or("").is_empty()),
        };
        if needs_id {
            let uuid = uuid::Uuid::new_v4().to_string().replace("-", "");
            tool_call["id"] = json!(format!("call_{uuid}"));
        }
        if tool_call.get("index").is_none() {
            tool_call["index"] = json!(idx);
        }
    }

    if let Some(tool_calls) = value.get_mut("tool_calls").and_then(|tc| tc.as_array_mut()) {
        for (i, tool_call) in tool_calls.iter_mut().enumerate() {
            process_tool_call(tool_call, i);
        }
    }

    if let Some(choices) = value.get_mut("choices").and_then(|c| c.as_array_mut()) {
        for choice in choices {
            for field in ["delta", "message"] {
                if let Some(tool_calls) = choice
                    .get_mut(field)
                    .and_then(|v| v.get_mut("tool_calls"))
                    .and_then(|tc| tc.as_array_mut())
                {
                    for (i, tool_call) in tool_calls.iter_mut().enumerate() {
                        process_tool_call(tool_call, i);
                    }
                }
            }
        }
    }
}

fn _push_streaming_json_into_scratchpad(
    scratch: &mut Box<dyn ScratchpadAbstract>,
    json: &serde_json::Value,
    model_name: &mut String,
    was_correct_output_even_if_error: &mut bool,
) -> Result<(serde_json::Value, FinishReason), String> {
    if let Some(token) = json.get("token") {
        // hf style produces this
        let text = token
            .get("text")
            .unwrap_or(&json!(""))
            .as_str()
            .unwrap_or("")
            .to_string();
        // TODO: probably we must retrieve the correct `finish_reason` from the json somehow
        let (mut value, finish_reason) = scratch.response_streaming(text, FinishReason::None)?;
        value["model"] = json!(model_name.clone());
        *was_correct_output_even_if_error |= json.get("generated_text").is_some();
        Ok((value, finish_reason))
    } else if let Some(choices) = json.get("choices") {
        // openai style
        let choice0 = &choices[0];
        let mut value: serde_json::Value;
        let mut finish_reason =
            FinishReason::from_json_val(choice0.get("finish_reason").unwrap_or(&json!("")))
                .unwrap_or_else(|err| {
                    tracing::error!(
                        "Couldn't parse finish_reason: {err}. Fallback to finish_reason=null"
                    );
                    FinishReason::None
                });
        if let Some(_delta) = choice0.get("delta") {
            (value, finish_reason) =
                scratch.response_message_streaming(&json, finish_reason.clone())?;
        } else if choices.as_array().map_or(true, |arr| arr.is_empty()) {
            value = json.clone();
        } else {
            let text = choice0
                .get("text")
                .unwrap_or(&json!(""))
                .as_str()
                .unwrap_or("")
                .to_string();
            (value, finish_reason) = scratch.response_streaming(text, finish_reason)?;
        }
        if let Some(model_value) = choice0.get("model") {
            model_name.clone_from(&model_value.as_str().unwrap_or("").to_string());
        }
        value["model"] = json!(model_name.clone());
        Ok((value, finish_reason))
    } else if json.get("type").and_then(|t| t.as_str()) == Some("ping") {
        Ok((serde_json::value::Value::Null, FinishReason::None))
    } else if let Some(err) = json.get("error") {
        Err(format!("{}", err))
    } else if let Some(msg) = json.get("human_readable_message") {
        Err(format!("{}", msg))
    } else if let Some(msg) = json.get("detail") {
        Err(format!("{}", msg))
    } else {
        Err(format!("unrecognized response (2): {:?}", json))
    }
}

pub async fn cached_not_stream(
    cached_json_value: &serde_json::Value,
) -> Result<Response<Body>, ScratchError> {
    let txt = serde_json::to_string_pretty(&cached_json_value).unwrap();
    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(txt))
        .unwrap();
    return Ok(response);
}

pub async fn cached_stream(
    cached_json_value: &serde_json::Value,
) -> Result<Response<Body>, ScratchError> {
    info!("cached_stream");
    let txt = serde_json::to_string(&cached_json_value).unwrap();
    let evstream = stream! {
        yield Result::<_, String>::Ok(format!("data: {}\n\n", txt));
        yield Result::<_, String>::Ok("data: [DONE]\n\n".to_string());
    };
    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::wrap_stream(evstream))
        .unwrap();
    return Ok(response);
}
