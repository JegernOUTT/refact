use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{future::Future, pin::Pin};
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

fn static_bearer(model_rec: &BaseModelRecord) -> Option<&str> {
    (!model_rec.api_key.is_empty()).then_some(model_rec.api_key.as_str())
}

async fn completion_request_with_runtime_credential(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    parameters: &SamplingParameters,
) -> Result<serde_json::Value, String> {
    let command_credential = crate::providers::runtime_credential::resolve(model_rec).await?;
    let bearer = command_credential
        .as_deref()
        .or_else(|| static_bearer(model_rec));
    let first = crate::forward_to_openai_endpoint::forward_to_openai_style_endpoint_with_bearer(
        model_rec, prompt, client, parameters, bearer,
    )
    .await;

    match first {
        Ok(value) => Ok(value),
        Err(error) if error.is_auth_rejection() && command_credential.is_some() => {
            let rejected = command_credential.as_deref().unwrap_or_default();
            let refreshed =
                crate::providers::runtime_credential::refresh_after_rejection(model_rec, rejected)
                    .await?
                    .ok_or_else(|| "Command credential refresh was unavailable".to_string())?;
            crate::forward_to_openai_endpoint::forward_to_openai_style_endpoint_with_bearer(
                model_rec,
                prompt,
                client,
                parameters,
                Some(&refreshed),
            )
            .await
            .map_err(|retry_error| {
                crate::providers::runtime_credential::redact(
                    crate::providers::runtime_credential::redact(
                        retry_error.message,
                        Some(&refreshed),
                    ),
                    Some(rejected),
                )
            })
        }
        Err(error) => Err(crate::providers::runtime_credential::redact(
            error.message,
            command_credential.as_deref(),
        )),
    }
}

async fn streaming_completion_request_with_runtime_credential(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    parameters: &SamplingParameters,
) -> Result<(reqwest::Response, Vec<String>), String> {
    let command_credential = crate::providers::runtime_credential::resolve(model_rec).await?;
    let bearer = command_credential
        .as_deref()
        .or_else(|| static_bearer(model_rec));
    let first =
        crate::forward_to_openai_endpoint::forward_to_openai_style_endpoint_streaming_with_bearer(
            model_rec, prompt, client, parameters, bearer,
        )
        .await;

    match first {
        Ok(response) => Ok((response, command_credential.into_iter().collect())),
        Err(error) if error.is_auth_rejection() && command_credential.is_some() => {
            let rejected = command_credential.as_deref().unwrap_or_default();
            let refreshed =
                crate::providers::runtime_credential::refresh_after_rejection(model_rec, rejected)
                    .await?
                    .ok_or_else(|| "Command credential refresh was unavailable".to_string())?;
            let response = crate::forward_to_openai_endpoint::forward_to_openai_style_endpoint_streaming_with_bearer(
                model_rec,
                prompt,
                client,
                parameters,
                Some(&refreshed),
            )
            .await
            .map_err(|retry_error| {
                crate::providers::runtime_credential::redact(
                    crate::providers::runtime_credential::redact(
                        retry_error.message,
                        Some(&refreshed),
                    ),
                    Some(rejected),
                )
            })?;
            Ok((response, vec![rejected.to_string(), refreshed]))
        }
        Err(error) => Err(crate::providers::runtime_credential::redact(
            error.message,
            command_credential.as_deref(),
        )),
    }
}

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
    } else if model_rec.endpoint_style == "hf" {
        Err("HuggingFace endpoint style is no longer supported. Please use 'openai' endpoint_style.".to_string())
    } else {
        completion_request_with_runtime_credential(
            &model_rec,
            prompt,
            &client,
            &parameters,
        )
        .await
    }
    .map_err(|e| ScratchError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("forward_to_endpoint: {}", e),
    ))?;
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

async fn request_scratchpad_stream(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    parameters: &SamplingParameters,
) -> Result<(reqwest::Response, Vec<String>), String> {
    if model_rec.endpoint_style == "hf" {
        Err("HuggingFace endpoint style is no longer supported. Please use 'openai' endpoint_style.".to_string())
    } else {
        streaming_completion_request_with_runtime_credential(model_rec, prompt, client, parameters)
            .await
    }
}

fn scratchpad_stream_frame(value: &serde_json::Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(value).unwrap())
}

fn scratchpad_stream_error_frame(detail: &str) -> String {
    scratchpad_stream_frame(&json!({ "detail": detail }))
}

fn scratchpad_stream_message_with_compression(message: &serde_json::Value, prompt: &str) -> String {
    let mut message = message.clone();
    message["compression_strength"] =
        crate::forward_to_openai_endpoint::try_get_compression_from_prompt(prompt);
    scratchpad_stream_frame(&message)
}

fn scratchpad_spontaneous_response(
    scratchpad: &mut Box<dyn ScratchpadAbstract>,
    prompt: &str,
) -> Result<Vec<String>, String> {
    scratchpad.response_spontaneous().map(|values| {
        values
            .iter()
            .map(|value| scratchpad_stream_message_with_compression(value, prompt))
            .collect()
    })
}

async fn process_scratchpad_stream_event(
    gcx: crate::global_context::SharedGlobalContext,
    scratchpad: &mut Box<dyn ScratchpadAbstract>,
    model_name: &mut String,
    event_data: &str,
    created: f64,
    dynamic_credentials: &[String],
    was_correct_output_even_if_error: &mut bool,
) -> Result<(String, FinishReason), String> {
    let mut value = serde_json::from_str::<serde_json::Value>(event_data).unwrap();
    generate_id_and_index_for_tool_calls_if_missing(&mut value);
    crate::global_context::look_for_piggyback_fields(gcx, &value).await;
    match _push_streaming_json_into_scratchpad(
        scratchpad,
        &value,
        model_name,
        was_correct_output_even_if_error,
    ) {
        Ok((mut value, finish_reason)) => {
            value["created"] = json!(created);
            Ok((scratchpad_stream_frame(&value), finish_reason))
        }
        Err(error) => Err(dynamic_credentials.iter().fold(error, |error, credential| {
            crate::providers::runtime_credential::redact(error, Some(credential))
        })),
    }
}

fn finalize_scratchpad_stream(
    scratchpad: &mut Box<dyn ScratchpadAbstract>,
    finish_reason: FinishReason,
    created: f64,
    model_name: &str,
) -> Result<String, ScratchError> {
    let mut value = scratchpad
        .streaming_finished(finish_reason)
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    value["created"] = json!(created);
    value["model"] = json!(model_name);
    Ok(scratchpad_stream_frame(&value))
}

enum ScratchpadStreamTick {
    Event(
        Option<
            Result<eventsource_stream::Event, eventsource_stream::EventStreamError<reqwest::Error>>,
        >,
    ),
    Heartbeat,
    Timeout(&'static str),
}

enum ScratchpadPromptTick {
    Subchat(Option<serde_json::Value>),
    Prompt(Result<String, String>),
}

async fn next_scratchpad_prompt_tick(
    prompt_future: &mut Option<Pin<Box<dyn Future<Output = Result<String, String>> + Send + '_>>>,
    subchat_rx: &Arc<AMutex<mpsc::UnboundedReceiver<serde_json::Value>>>,
) -> ScratchpadPromptTick {
    tokio::select! {
        value = async { subchat_rx.lock().await.recv().await } => {
            ScratchpadPromptTick::Subchat(value)
        }
        prompt_result = async {
            if let Some(future) = prompt_future.as_mut() {
                future.await
            } else {
                std::future::pending().await
            }
        } => {
            let _ = prompt_future.take();
            ScratchpadPromptTick::Prompt(prompt_result)
        }
    }
}

async fn next_scratchpad_stream_event<S>(
    event_stream: &mut S,
    heartbeat: &mut tokio::time::Interval,
    stream_started_at: Instant,
    last_event_at: Instant,
) -> ScratchpadStreamTick
where
    S: futures::Stream<
            Item = Result<
                eventsource_stream::Event,
                eventsource_stream::EventStreamError<reqwest::Error>,
            >,
        > + Unpin,
{
    tokio::select! {
        _ = heartbeat.tick() => {
            if stream_started_at.elapsed() > STREAM_TOTAL_TIMEOUT {
                ScratchpadStreamTick::Timeout("LLM stream timeout")
            } else if last_event_at.elapsed() > STREAM_IDLE_TIMEOUT {
                ScratchpadStreamTick::Timeout("LLM stream stalled")
            } else {
                ScratchpadStreamTick::Heartbeat
            }
        }
        maybe_event = event_stream.next() => ScratchpadStreamTick::Event(maybe_event),
    }
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
            let mut prompt_future: Option<
                Pin<Box<dyn Future<Output = Result<String, String>> + Send + '_>>,
            > = Some(Box::pin(my_scratchpad.prompt(
                prompt_input,
                &mut my_parameters,
            )));
            // horrible loop that waits for prompt() future, and at the same time retranslates any streaming via my_ccx.subchat_rx/tx to the user
            // (without streaming the rx/tx is never processed, disposed with the ccx)
            loop {
                match next_scratchpad_prompt_tick(&mut prompt_future, &subchat_rx).await {
                    ScratchpadPromptTick::Subchat(Some(value)) => {
                        let tmp = serde_json::to_string(&value).unwrap();
                        if tmp == "1337" {
                            break;
                        }
                        let value_str = format!("data: {}\n\n", tmp);
                        yield Result::<_, String>::Ok(value_str);
                    }
                    ScratchpadPromptTick::Subchat(None) => {}
                    ScratchpadPromptTick::Prompt(Ok(value)) => {
                        prompt = value;
                        let _ = subchat_tx.lock().await.send(serde_json::json!(1337));
                    }
                    ScratchpadPromptTick::Prompt(Err(error)) => {
                        tracing::warn!("prompt or tool use problem inside prompt: {}", error);
                        let value_str = scratchpad_stream_error_frame(&error);
                        yield Result::<_, String>::Ok(value_str);
                        return;
                    }
                }
            }
        }
        info!("scratchpad_interaction_stream prompt {:?}", t0.elapsed());

        if let Some(ref messages) = pre_stream_messages {
            for msg in messages {
                let value_str = scratchpad_stream_message_with_compression(msg, &prompt);
                yield Result::<_, String>::Ok(value_str);
            }
        }

        loop {
            let value_maybe = scratchpad_spontaneous_response(my_scratchpad, &prompt);
            if let Ok(values) = value_maybe {
                for value_str in values {
                    info!("yield: {:?}", nicer_logs::first_n_chars(&value_str, 40));
                    yield Result::<_, String>::Ok(value_str);
                }
            } else {
                let err_str = value_maybe.unwrap_err();
                tracing::error!("response_spontaneous error: {}", err_str);
                let value_str = scratchpad_stream_error_frame(&err_str);
                yield Result::<_, String>::Ok(value_str);
            }
            if only_deterministic_messages {
                break;
            }
            // info!("prompt: {:?}", prompt);
            let (response, dynamic_credentials) = match request_scratchpad_stream(
                &model_rec,
                &prompt,
                &client,
                &my_parameters,
            )
            .await
            {
                Ok(response) => response,
                Err(error) => {
                    let error = format!("forward_to_endpoint: {:?}", error);
                    tracing::error!(error);
                    let value_str = scratchpad_stream_error_frame(&error);
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
                let event = match next_scratchpad_stream_event(
                    &mut event_stream,
                    &mut heartbeat,
                    stream_started_at,
                    last_event_at,
                )
                .await
                {
                    ScratchpadStreamTick::Timeout(err_str) => {
                        tracing::error!("{}", err_str);
                        yield Result::<_, String>::Ok(scratchpad_stream_error_frame(err_str));
                        return;
                    }
                    ScratchpadStreamTick::Heartbeat => continue,
                    ScratchpadStreamTick::Event(Some(event)) => event,
                    ScratchpadStreamTick::Event(None) => break,
                };
                last_event_at = Instant::now();

                match event {
                    Ok(message) => {
                        // info!("Message: {:#?}", message);
                        if message.data.starts_with("[DONE]") {
                            break;
                        }
                        match process_scratchpad_stream_event(
                            gcx.clone(),
                            my_scratchpad,
                            &mut model_rec.name,
                            &message.data,
                            t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64(),
                            &dynamic_credentials,
                            &mut was_correct_output_even_if_error,
                        )
                        .await
                        {
                            Ok((value_str, finish_reason)) => {
                                if finish_reason != FinishReason::None {
                                    last_finish_reason = finish_reason;
                                }
                                yield Result::<_, String>::Ok(value_str);
                            }
                            Err(err_str) => {
                                tracing::error!("unexpected error: {}", err_str);
                                let value_str = scratchpad_stream_error_frame(&err_str);
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
                        let problem_str = dynamic_credentials.iter().fold(format!("{}", err), |error, credential| {
                            crate::providers::runtime_credential::redact(error, Some(credential))
                        });
                        tracing::error!("restream error: {}\n", problem_str);
                        yield Result::<_, String>::Ok(scratchpad_stream_error_frame(&problem_str));
                        return;
                    },
                }
            }

            let value_str = finalize_scratchpad_stream(
                my_scratchpad,
                last_finish_reason,
                t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64(),
                &model_rec.name,
            )
            .map_err(|error| error.to_string())?;
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use refact_core::provider_types::{
        CredentialSpec, DEFAULT_CREDENTIAL_REFRESH_INTERVAL_MS, DEFAULT_CREDENTIAL_TIMEOUT_MS,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn completion_command_credential_refreshes_once_after_unauthorized() {
        let provider_id = "completion_helper";
        crate::providers::credential::invalidate(provider_id);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (token, status, body) in [
                ("1", "401 Unauthorized", "rejected 1"),
                (
                    "2",
                    "200 OK",
                    r#"{"choices":[{"text":"ok","finish_reason":"stop"}]}"#,
                ),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let count = socket.read(&mut chunk).await.unwrap();
                    request.extend_from_slice(&chunk[..count]);
                    if count == 0 || request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                assert!(request.contains(&format!("authorization: bearer {token}")));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("count");
        let helper = format!(
            "n=$(cat '{}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{}'; echo $n",
            counter.display(),
            counter.display()
        );
        let model = BaseModelRecord {
            id: format!("{provider_id}/completion"),
            name: "completion".to_string(),
            endpoint: format!("http://{address}/v1/completions"),
            credential: Some(CredentialSpec::Command {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), helper],
                timeout_ms: DEFAULT_CREDENTIAL_TIMEOUT_MS,
                refresh_interval_ms: DEFAULT_CREDENTIAL_REFRESH_INTERVAL_MS,
                cwd: None,
                env_passthrough: Vec::new(),
            }),
            ..Default::default()
        };

        let result = completion_request_with_runtime_credential(
            &model,
            "prompt",
            &reqwest::Client::new(),
            &SamplingParameters::default(),
        )
        .await
        .unwrap();

        assert_eq!(result["choices"][0]["text"], "ok");
        assert_eq!(
            tokio::fs::read_to_string(counter).await.unwrap().trim(),
            "2"
        );
        server.await.unwrap();
        crate::providers::credential::invalidate(provider_id);
    }
}
