mod catalog;
mod circuit;
mod config;
mod format;
mod http;
mod mcp;
mod orchestrator;
mod providers;
mod rank;
mod request;
pub use request::{Freshness, SearchCategory, WebSearchRequest};
pub use catalog::{provider_catalog, ProviderInfo, ProviderTier, KeyRequirement};
pub use config::{WebSearchConfig, ProviderRuntimeConfig};
pub use orchestrator::{
    execute_web_search, test_providers, ProviderReport, ProviderStatus, ProviderTopResult,
    WebSearchOutcome,
};
pub use refact_core::chat_types::{SearchResult, format_search_results};
