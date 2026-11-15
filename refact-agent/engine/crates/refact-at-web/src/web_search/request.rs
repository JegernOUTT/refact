use chrono::{Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    Day,
    Week,
    Month,
    Year,
}
impl Freshness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
    pub fn cutoff(self) -> NaiveDate {
        Utc::now().date_naive()
            - Duration::days(match self {
                Self::Day => 1,
                Self::Week => 7,
                Self::Month => 30,
                Self::Year => 365,
            })
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchCategory {
    #[default]
    General,
    News,
    Code,
    Docs,
    Papers,
    Packages,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchRequest {
    pub queries: Vec<String>,
    pub num_results: usize,
    pub allowed_domains: Vec<String>,
    pub blocked_domains: Vec<String>,
    pub freshness: Option<Freshness>,
    pub category: SearchCategory,
}
impl Default for WebSearchRequest {
    fn default() -> Self {
        Self {
            queries: Vec::new(),
            num_results: 8,
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
            freshness: None,
            category: SearchCategory::General,
        }
    }
}
impl WebSearchRequest {
    pub fn normalize(&mut self) -> Result<(), String> {
        if !(1..=5).contains(&self.queries.len()) {
            return Err("queries must contain 1 to 5 queries".into());
        }
        for query in &mut self.queries {
            *query = query.split_whitespace().collect::<Vec<_>>().join(" ");
            if query.is_empty() {
                return Err("Search query cannot be empty".into());
            }
        }
        if !(1..=20).contains(&self.num_results) {
            return Err("num_results must be between 1 and 20".into());
        }
        normalize_domains(&mut self.allowed_domains)?;
        normalize_domains(&mut self.blocked_domains)?;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        self.clone().normalize()
    }
}
fn normalize_domains(domains: &mut Vec<String>) -> Result<(), String> {
    if domains.len() > 20 {
        return Err("domain filters may contain at most 20 domains".into());
    }
    for domain in domains.iter_mut() {
        let value = domain.trim().trim_end_matches('.');
        let parsed = Url::parse(&if value.contains("://") {
            value.to_owned()
        } else {
            format!("https://{}", value)
        })
        .map_err(|_| "Invalid domain filter".to_string())?;
        if value.is_empty()
            || value.contains('\\')
            || !matches!(parsed.scheme(), "http" | "https")
            || parsed.port().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("Domain filters must be hosts without paths or ports".into());
        }
        *domain = parsed
            .host_str()
            .ok_or("Invalid domain filter")?
            .trim_end_matches('.')
            .to_ascii_lowercase();
    }
    domains.sort();
    domains.dedup();
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn domain_urls_and_paths_normalize_to_hosts() {
        let mut domains = vec!["https://docs.rs/".into(), "docs.rs/tokio".into()];
        normalize_domains(&mut domains).unwrap();
        assert_eq!(domains, ["docs.rs"]);
    }
    #[test]
    fn validation_normalizes_and_rejects_invalid_requests() {
        let mut r = WebSearchRequest {
            queries: vec![" a   b ".into()],
            allowed_domains: vec!["EXAMPLE.COM.".into()],
            ..Default::default()
        };
        r.normalize().unwrap();
        assert_eq!(r.queries, ["a b"]);
        assert_eq!(r.allowed_domains, ["example.com"]);
        for domain in ["example.com:81", ""] {
            r.allowed_domains = vec![domain.into()];
            assert!(r.normalize().is_err(), "{domain}");
        }
        r.allowed_domains.clear();
        r.num_results = 21;
        assert!(r.validate().is_err());
        r.num_results = 1;
        r.queries = vec!["x".into(); 6];
        assert!(r.validate().is_err());
        r.queries = vec![" ".into()];
        assert!(r.validate().is_err());
    }
}
