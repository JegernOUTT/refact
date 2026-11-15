use serde::Serialize;
use super::SearchCategory;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTier {
    General,
    Fallback,
    Specialist,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyRequirement {
    None,
    Optional,
    Required,
}
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub tier: ProviderTier,
    pub category: Option<SearchCategory>,
    pub key: KeyRequirement,
    pub key_env_vars: &'static [&'static str],
    pub signup_url: &'static str,
    pub description: &'static str,
    pub default_enabled: bool,
}
macro_rules! provider { ($id:literal,$name:literal,$tier:ident,$category:expr,$key:ident,[$($env:literal),*],$url:literal,$desc:literal) => { ProviderInfo{id:$id,display_name:$name,tier:ProviderTier::$tier,category:$category,key:KeyRequirement::$key,key_env_vars:&[$($env),*],signup_url:$url,description:$desc,default_enabled:true} }; }
pub fn provider_catalog() -> &'static [ProviderInfo] {
    &CATALOG
}
static CATALOG: [ProviderInfo; 20] = [
    provider!(
        "exa",
        "Exa",
        General,
        None,
        Optional,
        ["EXA_API_KEY"],
        "https://dashboard.exa.ai/",
        "Neural web search with highlights"
    ),
    provider!(
        "parallel",
        "Parallel",
        General,
        None,
        Optional,
        ["PARALLEL_API_KEY"],
        "https://platform.parallel.ai/",
        "Multi-query web research"
    ),
    provider!(
        "tavily",
        "Tavily",
        General,
        None,
        Optional,
        ["TAVILY_API_KEY"],
        "https://app.tavily.com/",
        "Web search with extracted snippets"
    ),
    provider!(
        "brave",
        "Brave",
        General,
        None,
        Required,
        ["BRAVE_SEARCH_API_KEY", "BRAVE_API_KEY"],
        "https://api.search.brave.com/",
        "Independent web index"
    ),
    provider!(
        "serper",
        "Serper",
        General,
        None,
        Required,
        ["SERPER_API_KEY"],
        "https://serper.dev/",
        "Google search results"
    ),
    provider!(
        "langsearch",
        "LangSearch",
        General,
        None,
        Required,
        ["LANGSEARCH_API_KEY"],
        "https://langsearch.com/",
        "Semantic web search"
    ),
    provider!(
        "perplexity",
        "Perplexity",
        General,
        None,
        Required,
        ["PERPLEXITY_API_KEY"],
        "https://www.perplexity.ai/api-platform",
        "Search API"
    ),
    provider!(
        "firecrawl",
        "Firecrawl",
        General,
        None,
        Required,
        ["FIRECRAWL_API_KEY"],
        "https://firecrawl.dev/",
        "Web search and extraction"
    ),
    provider!(
        "searxng",
        "SearXNG",
        General,
        None,
        None,
        [],
        "https://docs.searxng.org/",
        "Your self-hosted SearXNG instance"
    ),
    provider!(
        "you",
        "You.com",
        Fallback,
        None,
        Optional,
        ["YDC_API_KEY", "YOU_API_KEY"],
        "https://you.com/",
        "Fallback web and news search"
    ),
    provider!(
        "keenable",
        "Keenable",
        Fallback,
        None,
        Optional,
        ["KEENABLE_API_KEY"],
        "https://keenable.ai/",
        "Fallback web search"
    ),
    provider!(
        "duckduckgo",
        "DuckDuckGo",
        Fallback,
        None,
        None,
        [],
        "https://duckduckgo.com/",
        "HTML fallback search"
    ),
    provider!(
        "wikipedia",
        "Wikipedia",
        Specialist,
        Some(SearchCategory::General),
        None,
        [],
        "https://wikipedia.org/",
        "Encyclopedia search"
    ),
    provider!(
        "github",
        "GitHub",
        Specialist,
        Some(SearchCategory::Code),
        Optional,
        ["GITHUB_TOKEN"],
        "https://github.com/settings/tokens",
        "Repositories and issues"
    ),
    provider!(
        "stack_overflow",
        "Stack Overflow",
        Specialist,
        Some(SearchCategory::Code),
        None,
        [],
        "https://stackoverflow.com/",
        "Programming questions"
    ),
    provider!(
        "npm",
        "npm",
        Specialist,
        Some(SearchCategory::Packages),
        None,
        [],
        "https://npmjs.com/",
        "JavaScript packages"
    ),
    provider!(
        "openalex",
        "OpenAlex",
        Specialist,
        Some(SearchCategory::Papers),
        None,
        [],
        "https://openalex.org/",
        "Scholarly works"
    ),
    provider!(
        "crossref",
        "Crossref",
        Specialist,
        Some(SearchCategory::Papers),
        None,
        [],
        "https://crossref.org/",
        "Publication metadata"
    ),
    provider!(
        "hacker_news",
        "Hacker News",
        Specialist,
        Some(SearchCategory::News),
        None,
        [],
        "https://news.ycombinator.com/",
        "Technology news"
    ),
    provider!(
        "context7",
        "Context7",
        Specialist,
        Some(SearchCategory::Docs),
        Optional,
        ["CONTEXT7_API_KEY"],
        "https://context7.com/",
        "Library documentation"
    ),
];
