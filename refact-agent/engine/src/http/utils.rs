use tracing::{error, info};
use axum::middleware::Next;
use axum::http::{Method, Request, Uri};
use axum::response::Response;

use crate::custom_error::ScratchError;

const SPAM_HANDLERS: &[&str] = &["rag-status", "ping"];

pub async fn request_logging_middleware<B>(
    path: Uri,
    _method: Method,
    request: Request<B>,
    next: Next<B>,
) -> Result<Response, ScratchError> {
    let handler_name = path.path().trim_start_matches('/');
    let spam = SPAM_HANDLERS.contains(&handler_name);

    if !spam {
        info!("\n--- HTTP {} starts ---\n", handler_name);
    }
    let t0 = std::time::Instant::now();

    let mut response = next.run(request).await;

    // ScratchError::into_response creates an extension that is used to let us
    // preserve structured errors through Axum middleware.
    if let Some(e) = response.extensions_mut().remove::<ScratchError>() {
        error!("{} returning, client will see \"{}\"", path, e);
        return Err(e);
    }

    if !spam {
        info!("{} completed {}ms", path, t0.elapsed().as_millis());
    }

    Ok(response)
}
