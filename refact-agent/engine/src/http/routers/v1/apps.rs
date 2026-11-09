use std::convert::Infallible;
use axum::{
    extract::{State, Path, Query},
    http::{HeaderMap, StatusCode},
    response::{Response, IntoResponse, Sse, sse::Event},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::{
    app_state::AppState,
    apps::{self, Caller, Command},
};
type Error = (StatusCode, Json<Value>);
fn error(message: String) -> Error {
    (
        if message.contains("conflict") {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        },
        Json(json!({"error":message})),
    )
}
fn authorize(headers: &HeaderMap) -> Result<(), Error> {
    if !super::work::protected_origin_allowed(headers) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error":"untrusted app host origin"})),
        ));
    }
    Ok(())
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentQuery {
    chat_id: Option<String>,
    binding_id: Option<uuid::Uuid>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeQuery {
    chat_id: Option<String>,
    cursor: Option<String>,
}
pub async fn list(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DocumentQuery>,
) -> Result<Json<Value>, Error> {
    authorize(&headers)?;
    Ok(Json(
        apps::execute(
            &app,
            query
                .chat_id
                .map(|chat_id| Caller::HostChat {
                    chat_id,
                })
                .unwrap_or(Caller::Host),
            Command::List {},
        )
        .await
        .map_err(error)?,
    ))
}
pub async fn document(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    headers: HeaderMap,
    Query(query): Query<DocumentQuery>,
) -> Result<Json<Value>, Error> {
    authorize(&headers)?;
    Ok(Json(
        apps::document(&app, id, query.chat_id, query.binding_id)
            .await
            .map_err(error)?,
    ))
}
pub async fn commands(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DocumentQuery>,
    Json(command): Json<Command>,
) -> Result<Json<Value>, Error> {
    authorize(&headers)?;
    Ok(Json(
        apps::execute(
            &app,
            query
                .chat_id
                .map(|chat_id| Caller::HostChat {
                    chat_id,
                })
                .unwrap_or(Caller::Host),
            command,
        )
        .await
        .map_err(error)?,
    ))
}
pub async fn subscribe(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SubscribeQuery>,
) -> Result<Response, Error> {
    authorize(&headers)?;
    let runtime = apps::runtime(
        &app,
        &query
            .chat_id
            .map(|chat_id| Caller::HostChat {
                chat_id,
            })
            .unwrap_or(Caller::Host),
    )
    .await
    .map_err(error)?;
    let mut subscription = runtime
        .events
        .subscribe(
            query
                .cursor
                .as_deref()
                .or_else(|| headers.get("last-event-id").and_then(|v| v.to_str().ok())),
        )
        .map_err(error)?;
    let stream = async_stream::stream! {
     subscription.hold_capacity();
     for envelope in subscription.initial.drain(..){if let Ok(event)=Event::default().event(&envelope.kind).id(format!("{}:{}",envelope.generation,envelope.seq)).json_data(&envelope){yield Ok::<Event,Infallible>(event);}}
     loop {
      if app.runtime.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed){break;}
      tokio::select!{
       _=tokio::time::sleep(std::time::Duration::from_secs(2))=>{},
       next=subscription.rx.recv()=>match next {
        Ok(envelope)=>{let stop=envelope.kind=="shutdown";if let Ok(event)=Event::default().event(&envelope.kind).id(format!("{}:{}",envelope.generation,envelope.seq)).json_data(&envelope){yield Ok(event);}if stop{break;}},
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{yield Ok(Event::default().event("reset").data("{\"reason\":\"consumer_gap\"}"));break;},
        Err(_)=>break,
       }
      }
     }
    };
    Ok(Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;
    #[test]
    fn opaque_origin_denied_even_with_valid_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        headers.insert(header::ORIGIN, "null".parse().unwrap());
        assert_eq!(authorize(&headers).unwrap_err().0, StatusCode::FORBIDDEN);
        headers.remove(header::ORIGIN);
        assert!(authorize(&headers).is_ok());
    }
}
