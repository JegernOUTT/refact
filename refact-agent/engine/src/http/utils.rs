use tracing::{error, info, warn};
use axum::middleware::Next;
use axum::http::{Method, Request, StatusCode, Uri};
use axum::response::Response;

use crate::custom_error::ScratchError;

const SPAM_HANDLERS: &[&str] = &["rag-status", "ping"];

fn is_spam_request(path: &Uri, method: &Method) -> bool {
    let handler_name = path.path().trim_start_matches('/');
    SPAM_HANDLERS.contains(&handler_name)
        || (method == Method::GET
            && (handler_name.starts_with("trajectories/subchat-")
                || handler_name.starts_with("v1/trajectories/subchat-")))
}

pub async fn request_logging_middleware<B>(
    path: Uri,
    method: Method,
    request: Request<B>,
    next: Next<B>,
) -> Result<Response, ScratchError> {
    let handler_name = path.path().trim_start_matches('/');
    let spam = is_spam_request(&path, &method);

    if !spam {
        info!("\n--- HTTP {} starts ---\n", handler_name);
    }
    let t0 = std::time::Instant::now();

    let mut response = next.run(request).await;

    // ScratchError::into_response creates an extension that is used to let us
    // preserve structured errors through Axum middleware.
    if let Some(e) = response.extensions_mut().remove::<ScratchError>() {
        if e.status_code.is_server_error() {
            error!("{} returning, client will see \"{}\"", path, e);
        } else if e.status_code == StatusCode::NOT_FOUND {
            if !spam {
                info!("{} returning, client will see \"{}\"", path, e);
            }
        } else {
            warn!("{} returning, client will see \"{}\"", path, e);
        }
        return Err(e);
    }

    if !spam {
        info!("{} completed {}ms", path, t0.elapsed().as_millis());
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppresses_only_get_requests_for_subchat_trajectories() {
        let uri = "/trajectories/subchat-1234".parse().unwrap();
        assert!(is_spam_request(&uri, &Method::GET));
        assert!(!is_spam_request(&uri, &Method::DELETE));
    }

    #[test]
    fn keeps_top_level_trajectory_requests_visible() {
        let uri = "/trajectories/chat-1234".parse().unwrap();
        assert!(!is_spam_request(&uri, &Method::GET));
    }
}
