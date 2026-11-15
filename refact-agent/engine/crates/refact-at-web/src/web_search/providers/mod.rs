use std::{future::Future, pin::Pin};
use super::{WebSearchRequest, WebSearchConfig, SearchResult};
mod brave;
mod common;
mod context7;
mod crossref;
mod duckduckgo;
mod exa;
mod firecrawl;
mod github;
mod hacker_news;
mod keenable;
mod langsearch;
mod legacy_common;
mod npm;
mod openalex;
mod parallel;
mod perplexity;
mod searxng;
mod serper;
mod stack_overflow;
mod tavily;
mod wikipedia;
mod you;
pub(super) type SearchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, String>> + Send + 'a>>;
pub(super) trait SearchProvider: Send + Sync {
    fn search<'a>(
        &'a self,
        id: &'a str,
        request: &'a WebSearchRequest,
        config: &'a WebSearchConfig,
        key: Option<&'a str>,
    ) -> SearchFuture<'a>;
}
pub(super) struct CatalogProvider;
impl SearchProvider for CatalogProvider {
    fn search<'a>(
        &'a self,
        id: &'a str,
        request: &'a WebSearchRequest,
        config: &'a WebSearchConfig,
        key: Option<&'a str>,
    ) -> SearchFuture<'a> {
        Box::pin(async move {
            match id {
                "exa" => exa::search(request, config, key).await,
                "parallel" => parallel::search(request, config, key).await,
                "tavily" => tavily::search(request, config, key).await,
                "brave" => brave::search(request, config, key).await,
                "serper" => serper::search(request, config, key).await,
                "langsearch" => langsearch::search(request, config, key).await,
                "perplexity" => perplexity::search(request, config, key).await,
                "firecrawl" => firecrawl::search(request, config, key).await,
                "searxng" => searxng::search(request, config, key).await,
                "you" => you::search(request, config, key).await,
                "keenable" => keenable::search(request, config, key).await,
                "duckduckgo" => duckduckgo::search(request, config, key).await,
                "wikipedia" => wikipedia::search(request, config, key).await,
                "github" => github::search(request, config, key).await,
                "stack_overflow" => stack_overflow::search(request, config, key).await,
                "npm" => npm::search(request, config, key).await,
                "openalex" => openalex::search(request, config, key).await,
                "crossref" => crossref::search(request, config, key).await,
                "hacker_news" => hacker_news::search(request, config, key).await,
                "context7" => context7::search(request, config, key).await,
                _ => Err("Unknown provider".into()),
            }
        })
    }
}
