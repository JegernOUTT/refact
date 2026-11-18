use std::time::Duration;
use tracing::info;
use std::sync::Arc;

use crate::global_context::GlobalContext;
use crate::http::routers::v1::status::get_rag_status;

pub async fn wait_for_indexing_if_needed(gcx: Arc<GlobalContext>) {
    let cmdline = { gcx.cmdline.clone() };

    let ast_done = async || {
        get_rag_status(gcx.clone())
            .await
            .ast
            .is_some_and(|ast_status| ast_status.astate == "done")
    };
    let mut waiting_ast = cmdline.wait_ast && !ast_done().await;

    if waiting_ast {
        info!("Waiting for AST to finish indexing.");
    }

    while waiting_ast {
        if waiting_ast && ast_done().await {
            info!("Ast finished indexing.");
            waiting_ast = false;
        }

        if waiting_ast {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}
