use std::sync::Arc;

use indexmap::IndexSet;
use tokio::sync::{Mutex as AMutex, Notify as ANotify};

use crate::ast::ast_structs::{AstDB, AstStatus};

pub struct AstIndexService {
    pub ast_index: Arc<AstDB>,
    pub ast_status: Arc<AMutex<AstStatus>>,
    pub ast_sleeping_point: Arc<ANotify>,
    pub ast_todo: IndexSet<String>,
}

impl AstIndexService {
    pub fn new(
        ast_index: Arc<AstDB>,
        ast_status: Arc<AMutex<AstStatus>>,
        ast_sleeping_point: Arc<ANotify>,
    ) -> Self {
        Self {
            ast_index,
            ast_status,
            ast_sleeping_point,
            ast_todo: IndexSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::ast_db::ast_index_init;

    #[tokio::test]
    async fn new_starts_with_empty_todo() {
        let ast_index = ast_index_init(String::new(), 25).await;
        let ast_status = Arc::new(AMutex::new(AstStatus {
            astate_notify: Arc::new(ANotify::new()),
            astate: String::from("starting"),
            files_unparsed: 0,
            files_total: 0,
            ast_index_files_total: 0,
            ast_index_symbols_total: 0,
            ast_index_usages_total: 0,
            ast_max_files_hit: false,
        }));
        let ast_sleeping_point = Arc::new(ANotify::new());

        let service = AstIndexService::new(
            ast_index.clone(),
            ast_status.clone(),
            ast_sleeping_point.clone(),
        );

        assert!(Arc::ptr_eq(&service.ast_index, &ast_index));
        assert!(Arc::ptr_eq(&service.ast_status, &ast_status));
        assert!(Arc::ptr_eq(
            &service.ast_sleeping_point,
            &ast_sleeping_point
        ));
        assert!(service.ast_todo.is_empty());
        assert_eq!(service.ast_index.ast_max_files, 25);
    }
}
