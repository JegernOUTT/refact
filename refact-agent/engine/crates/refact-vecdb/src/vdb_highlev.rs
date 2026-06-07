use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Mutex as AMutex};
use tokio::task::JoinHandle;
use async_trait::async_trait;
use tracing::info;

use refact_core::llm_types::EmbeddingEndpointStyle;
use refact_core::vecdb_types::{
    EmbeddingModelConfig, FileReader, SearchResult, VecDbStatus, VecdbRecord, VecdbSearch,
};

use crate::fetch_embedding;
use crate::vdb_emb_aux;
use crate::vdb_sqlite::VecDBSqlite;
use crate::vdb_structs::{VecdbConstants};
use crate::vdb_thread::{vecdb_start_background_tasks, vectorizer_enqueue_files, FileVectorizerService};

pub struct VecDb {
    vecdb_emb_client: Arc<AMutex<reqwest::Client>>,
    vecdb_handler: Arc<AMutex<VecDBSqlite>>,
    pub vectorizer_service: Arc<AMutex<FileVectorizerService>>,
    constants: VecdbConstants,
}

fn validate_embedding_config(model: &EmbeddingModelConfig) -> Result<(), String> {
    if model.model_name.trim().is_empty() {
        return Err("No embedding model configured".to_string());
    }
    if model.embedding_size <= 0 {
        return Err("No embedding size configured".to_string());
    }
    if model.embedding_batch == 0 {
        return Err("No embedding batch configured".to_string());
    }
    if model.rejection_threshold <= 0.0 {
        return Err(format!(
            "VecDB embedding config error: rejection_threshold must be positive, got {}",
            model.rejection_threshold
        ));
    }

    let style = if model.embedding_endpoint_style.is_empty() {
        EmbeddingEndpointStyle::from_config(&model.endpoint_style, "embedding_endpoint_style")?
    } else {
        EmbeddingEndpointStyle::from_config(
            &model.embedding_endpoint_style,
            "embedding_endpoint_style",
        )?
    };

    if model.endpoint.trim().is_empty() && style != EmbeddingEndpointStyle::OllamaNative {
        return Err("No embedding endpoint configured".to_string());
    }
    Ok(())
}

impl VecDb {
    pub fn validate_constants(constants: &VecdbConstants) -> Result<(), String> {
        validate_embedding_config(&constants.embedding_model)
    }

    pub async fn embed_query(&self, query: &str) -> Result<Vec<f32>, String> {
        let embedding_mb = fetch_embedding::get_embedding_with_retries(
            self.vecdb_emb_client.clone(),
            &self.constants.embedding_model,
            vec![self.constants.embedding_model.prefixed_query(query)],
            5,
        )
        .await
        .map_err(|e| e.to_string())?;

        embedding_mb
            .into_iter()
            .next()
            .ok_or_else(|| "VecDB: empty embedding result".to_string())
    }

    fn compute_usefulness_and_filter(&self, mut results: Vec<VecdbRecord>) -> Vec<VecdbRecord> {
        let mut dist0 = 0.0;
        let mut filtered_results = Vec::new();
        let rejection_threshold = self.constants.embedding_model.rejection_threshold;
        for rec in results.iter_mut() {
            if dist0 == 0.0 {
                dist0 = rec.distance.abs();
            }
            rec.usefulness = 100.0
                - 75.0
                    * ((rec.distance.abs() - dist0) / (dist0 + 0.01))
                        .max(0.0)
                        .min(1.0);
            if rec.distance.abs() < rejection_threshold {
                filtered_results.push(rec.clone());
            }
        }
        filtered_results
    }

    pub async fn vecdb_search_with_embedding(
        &self,
        embedding: &Vec<f32>,
        top_n: usize,
        vecdb_scope_filter_mb: Option<String>,
    ) -> Result<Vec<VecdbRecord>, String> {
        let mut handler_locked = self.vecdb_handler.lock().await;
        let raw = handler_locked
            .vecdb_search(embedding, top_n, vecdb_scope_filter_mb)
            .await
            .map_err(|e| e.to_string())?;
        Ok(self.compute_usefulness_and_filter(raw))
    }

    pub async fn init(
        vecdb_dir: &PathBuf,
        legacy_cache_dir: &PathBuf,
        workspace_folder: String,
        insecure: bool,
        constants: VecdbConstants,
    ) -> Result<VecDb, String> {
        Self::validate_constants(&constants)?;
        let emb_table_name = vdb_emb_aux::create_emb_table_name(&vec![workspace_folder]);
        let handler = VecDBSqlite::init(
            vecdb_dir,
            legacy_cache_dir,
            &constants.embedding_model.model_name,
            constants.embedding_model.embedding_size,
            &emb_table_name,
        )
        .await?;
        let vecdb_handler = Arc::new(AMutex::new(handler));
        let vectorizer_service = Arc::new(AMutex::new(
            FileVectorizerService::new(vecdb_handler.clone(), constants.clone()).await,
        ));
        let mut http_client_builder = reqwest::Client::builder();
        if insecure {
            http_client_builder = http_client_builder.danger_accept_invalid_certs(true);
        }
        let vecdb_emb_client = Arc::new(AMutex::new(http_client_builder.build().unwrap()));
        Ok(VecDb {
            vecdb_emb_client,
            vecdb_handler,
            vectorizer_service,
            constants: constants.clone(),
        })
    }

    pub async fn vecdb_start_background_tasks(
        &self,
        shutdown_flag: Arc<AtomicBool>,
        file_reader: FileReader,
    ) -> Vec<JoinHandle<()>> {
        info!("vecdb: start_background_tasks");
        vecdb_start_background_tasks(
            self.vecdb_emb_client.clone(),
            self.vectorizer_service.clone(),
            shutdown_flag,
            file_reader,
        )
        .await
    }
}

#[async_trait]
impl VecdbSearch for VecDb {
    async fn vecdb_search(
        &self,
        query: String,
        top_n: usize,
        vecdb_scope_filter_mb: Option<String>,
    ) -> Result<SearchResult, String> {
        let t0 = std::time::Instant::now();
        let embedding = self.embed_query(&query).await?;
        info!(
            "search query {:?}, it took {:.3}s to vectorize the query",
            query,
            t0.elapsed().as_secs_f64()
        );
        let t1 = std::time::Instant::now();
        let results = self
            .vecdb_search_with_embedding(&embedding, top_n, vecdb_scope_filter_mb)
            .await?;
        info!("search itself {:.3}s", t1.elapsed().as_secs_f64());
        Ok(SearchResult {
            query_text: query,
            results,
        })
    }

    async fn get_status(&self) -> Result<VecDbStatus, String> {
        let (vstatus, vecdb_handler) = {
            let vectorizer_locked = self.vectorizer_service.lock().await;
            (
                vectorizer_locked.vstatus.clone(),
                vectorizer_locked.vecdb_handler.clone(),
            )
        };
        let mut vstatus_copy = vstatus.lock().await.clone();
        vstatus_copy.db_size = vecdb_handler.lock().await.size().await?;
        vstatus_copy.db_cache_size = vecdb_handler
            .lock()
            .await
            .cache_size()
            .await
            .map_err(|e| e.to_string())?;
        if vstatus_copy.state == "done" && vstatus_copy.queue_additions {
            vstatus_copy.state = "cooldown".to_string();
        }
        Ok(vstatus_copy)
    }

    async fn remove_file(&self, file_path: &PathBuf) -> Result<(), String> {
        let mut handler_locked = self.vecdb_handler.lock().await;
        let file_path_str = file_path.to_string_lossy().to_string();
        handler_locked
            .vecdb_records_remove(vec![file_path_str])
            .await
    }

    async fn vectorizer_enqueue_files(&self, documents: &[String], process_immediately: bool) {
        vectorizer_enqueue_files(
            self.vectorizer_service.clone(),
            documents,
            process_immediately,
        )
        .await;
    }

    fn current_constants(&self) -> (EmbeddingModelConfig, usize) {
        (
            self.constants.embedding_model.clone(),
            self.constants.splitter_window_size,
        )
    }

    async fn embed_query(&self, query: &str) -> Result<Vec<f32>, String> {
        VecDb::embed_query(self, query).await
    }

    async fn vecdb_search_with_embedding(
        &self,
        embedding: &Vec<f32>,
        top_n: usize,
        filter_mb: Option<String>,
    ) -> Result<Vec<VecdbRecord>, String> {
        VecDb::vecdb_search_with_embedding(self, embedding, top_n, filter_mb).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch_embedding::take_last_embedding_inputs;

    fn test_config() -> EmbeddingModelConfig {
        embedding_config("openai", "test://record-only")
    }

    fn embedding_config(endpoint_style: &str, endpoint: &str) -> EmbeddingModelConfig {
        EmbeddingModelConfig {
            endpoint: endpoint.to_string(),
            endpoint_style: "openai".to_string(),
            embedding_endpoint_style: endpoint_style.to_string(),
            api_key: String::new(),
            auth_token: String::new(),
            extra_headers: Default::default(),
            model_name: "test-embedding".to_string(),
            embedding_size: 2,
            dimensions: None,
            query_prefix: "query: ".to_string(),
            document_prefix: "doc: ".to_string(),
            rejection_threshold: 0.63,
            embedding_batch: 2,
            n_ctx: 128,
        }
    }

    fn test_constants(config: EmbeddingModelConfig) -> VecdbConstants {
        VecdbConstants {
            embedding_model: config,
            tokenizer: None,
            splitter_window_size: 128,
            vecdb_max_files: 100,
        }
    }

    #[test]
    fn rejects_invalid_embedding_config() {
        let mut config = test_config();
        config.embedding_batch = 0;
        let err = VecDb::validate_constants(&test_constants(config)).unwrap_err();
        assert!(err.contains("embedding batch"));
    }

    #[test]
    fn rejects_zero_rejection_threshold() {
        let mut config = test_config();
        config.rejection_threshold = 0.0;
        let err = VecDb::validate_constants(&test_constants(config)).unwrap_err();
        assert!(err.contains("rejection_threshold"));
    }

    #[test]
    fn vdb_highlev_rejects_openai_empty_endpoint() {
        let err = validate_embedding_config(&embedding_config("openai", "")).unwrap_err();

        assert_eq!(err, "No embedding endpoint configured");
    }

    #[test]
    fn vdb_highlev_accepts_ollama_native_empty_endpoint() {
        validate_embedding_config(&embedding_config("ollama_native", "")).unwrap();
    }

    #[tokio::test]
    async fn embed_query_applies_query_prefix_at_embedding_boundary() {
        let tmp =
            std::env::temp_dir().join(format!("refact-vecdb-highlev-{}", uuid::Uuid::new_v4()));
        let legacy = tmp.join("legacy");
        let vecdb = VecDb::init(
            &tmp,
            &legacy,
            "workspace".to_string(),
            false,
            test_constants(test_config()),
        )
        .await
        .unwrap();

        let embedding = vecdb.embed_query("hello").await.unwrap();

        assert_eq!(embedding, vec![1.0, 1.0]);
        assert_eq!(
            take_last_embedding_inputs(),
            vec!["query: hello".to_string()]
        );
        let _ = std::fs::remove_dir_all(tmp);
    }
}
