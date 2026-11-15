use std::{collections::HashMap, sync::Arc};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use refact_at_web::web_search::{execute_web_search, SearchResult, WebSearchRequest};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::web_search_settings::load_web_search_settings;

pub struct ToolWebSearch {
    pub config_path: String,
}

fn input_schema() -> Value {
    json!({"type":"object","properties":{
        "query":{"type":"string","description":"Search query."},
        "extra_queries":{"type":"array","items":{"type":"string"},"maxItems":4},
        "num_results":{"type":"integer","minimum":1,"maximum":20,"default":8},
        "allowed_domains":{"type":"array","items":{"type":"string"}},
        "blocked_domains":{"type":"array","items":{"type":"string"}},
        "freshness":{"type":"string","enum":["day","week","month","year"]},
        "category":{"type":"string","enum":["general","news","code","docs","papers","packages"],"default":"general"}
    },"required":["query"]})
}

fn array_arg(args: &HashMap<String, Value>, name: &str) -> Result<Vec<String>, String> {
    let values = match optional_arg(args, name) {
        None => return Ok(vec![]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("argument `{name}` must contain strings"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::String(text)) if text.trim().starts_with('[') => {
            serde_json::from_str::<Vec<String>>(text)
                .map_err(|e| format!("invalid `{name}` array: {e}"))?
        }
        Some(Value::String(text)) => text.split(',').map(str::to_string).collect(),
        _ => return Err(format!("argument `{name}` must be an array of strings")),
    };
    Ok(values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect())
}

fn optional_arg<'a>(args: &'a HashMap<String, Value>, name: &str) -> Option<&'a Value> {
    args.get(name).filter(|value| {
        !value.is_null() && !value.as_str().is_some_and(|text| text.trim().is_empty())
    })
}

fn enum_arg<T: serde::de::DeserializeOwned>(
    args: &HashMap<String, Value>,
    name: &str,
    error: &str,
) -> Result<Option<T>, String> {
    optional_arg(args, name)
        .map(|value| {
            let value = value
                .as_str()
                .map(|text| json!(text.trim().to_ascii_lowercase()))
                .unwrap_or_else(|| value.clone());
            serde_json::from_value(value).map_err(|_| error.to_string())
        })
        .transpose()
}

fn parse_request(args: &HashMap<String, Value>) -> Result<WebSearchRequest, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or("Missing or invalid string argument `query`")?;
    let extra = array_arg(args, "extra_queries")?;
    if extra.len() > 4 {
        return Err("argument `extra_queries` must contain at most 4 queries".into());
    }
    let num_results = match optional_arg(args, "num_results") {
        None => 8,
        Some(value) => {
            let number = match value {
                Value::String(text) => text.trim().parse::<f64>().ok(),
                Value::Number(number) => number.as_f64(),
                _ => None,
            }
            .ok_or("argument `num_results` must be an integer")?;
            if !number.is_finite() || number.fract() != 0.0 || !(1.0..=20.0).contains(&number) {
                return Err("argument `num_results` must be an integer from 1 to 20".into());
            }
            number as usize
        }
    };
    let mut request = WebSearchRequest {
        queries: std::iter::once(query.to_string()).chain(extra).collect(),
        num_results,
        allowed_domains: array_arg(args, "allowed_domains")?,
        blocked_domains: array_arg(args, "blocked_domains")?,
        freshness: enum_arg(
            args,
            "freshness",
            "freshness must be day, week, month, or year",
        )?,
        category: enum_arg(
            args,
            "category",
            "category must be general, news, code, docs, papers, or packages",
        )?
        .unwrap_or_default(),
    };
    request.normalize()?;
    request.validate()?;
    Ok(request)
}

fn result_json(result: SearchResult) -> Value {
    let mut value = json!({"title":result.title,"url":result.url,"snippet":result.snippet,"source":result.source,"sources":result.sources,"citation_span":result.snippet.chars().count()});
    if let Some(date) = result.published_date {
        value["published_date"] = json!(date);
    }
    value
}

#[async_trait]
impl Tool for ToolWebSearch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "web_search".into(), display_name: "Web Search".into(),
            source: ToolSource { source_type: ToolSourceType::Builtin, config_path: self.config_path.clone() },
            experimental: false, allow_parallel: true,
            description: format!("Search the web for current information. Today is {}. Use category for news, code, docs, papers, or packages; freshness for recent results; allowed_domains/blocked_domains to scope sources; extra_queries for complementary searches. Cite the URLs you rely on.", chrono::Utc::now().format("%Y-%m-%d")),
            input_schema: input_schema(), output_schema: None, annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let request = parse_request(args)?;
        let gcx = ccx.lock().await.global_context.clone();
        let config = load_web_search_settings(gcx).await.runtime_config();
        let outcome = execute_web_search(&request, &config).await?;
        let mut extra = serde_json::Map::new();
        extra.insert(
            "search_results".into(),
            Value::Array(outcome.results.into_iter().map(result_json).collect()),
        );
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".into(),
                content: ChatContent::SimpleText(outcome.text),
                tool_call_id: tool_call_id.clone(),
                extra,
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_arguments_accept_absent_empty_null_and_whole_floats() {
        for empty in [Value::Null, json!(""), json!("  ")] {
            let args = serde_json::from_value(json!({"query":"rust", "num_results":empty,
                "extra_queries":empty,"allowed_domains":empty,"blocked_domains":empty,
                "freshness":empty,"category":empty}))
            .unwrap();
            let request = parse_request(&args).unwrap();
            assert_eq!(request.num_results, 8);
            assert_eq!(request.queries, ["rust"]);
            assert_eq!(request.freshness, None);
            assert_eq!(serde_json::to_value(request.category).unwrap(), "general");
        }
        for number in [json!(3.0), json!("3.0")] {
            let args = serde_json::from_value(json!({"query":"rust", "num_results":number,
                "extra_queries":["", " ", "tokio", "", "  "], "allowed_domains":"[\"\",\"docs.rs\",\" \" ]",
                "freshness":" WEEK ", "category":" Code "})).unwrap();
            let request = parse_request(&args).unwrap();
            assert_eq!(request.num_results, 3);
            assert_eq!(request.queries, ["rust", "tokio"]);
            assert_eq!(request.allowed_domains, ["docs.rs"]);
            assert_eq!(serde_json::to_value(request.freshness).unwrap(), "week");
            assert_eq!(serde_json::to_value(request.category).unwrap(), "code");
        }
        for number in [json!(3.5), json!("NaN"), json!("inf"), json!(-1), json!(21)] {
            assert!(parse_request(
                &serde_json::from_value(json!({"query":"rust","num_results":number})).unwrap()
            )
            .is_err());
        }
    }

    #[test]
    fn description_uses_utc_calendar_date() {
        let before = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let description = ToolWebSearch {
            config_path: String::new(),
        }
        .tool_description()
        .description;
        let after = chrono::Utc::now().format("%Y-%m-%d").to_string();
        assert!(
            description.contains(&format!("Today is {before}."))
                || description.contains(&format!("Today is {after}."))
        );
    }

    #[test]
    fn lenient_args_normalize_and_validate() {
        let args = serde_json::from_value(json!({"query":" Rust ","num_results":"3","extra_queries":"[\"tokio\"]","allowed_domains":"docs.rs, rust-lang.org","freshness":"week","category":"code"})).expect("args");
        let request = parse_request(&args).expect("request");
        assert_eq!(request.queries, vec!["Rust", "tokio"]);
        assert_eq!(request.num_results, 3);
        assert_eq!(request.allowed_domains, vec!["docs.rs", "rust-lang.org"]);
        for bad in [
            json!({"query":"rust","num_results":0}),
            json!({"query":"rust","num_results":"bad"}),
            json!({"query":"rust","extra_queries":[1]}),
            json!({"query":"rust","category":"other"}),
            json!({"query":""}),
        ] {
            assert!(parse_request(&serde_json::from_value(bad).expect("args")).is_err());
        }
    }

    #[test]
    fn schema_and_output_contract() {
        let schema = input_schema();
        assert_eq!(schema["required"], json!(["query"]));
        assert_eq!(schema["properties"]["extra_queries"]["maxItems"], 4);
        assert_eq!(schema["properties"]["num_results"]["maximum"], 20);
        let result = SearchResult {
            title: "title".into(),
            url: "https://example.org".into(),
            snippet: "ü字".into(),
            source: Some("exa".into()),
            sources: vec!["exa".into(), "parallel".into()],
            published_date: Some("2026-01-01".into()),
        };
        assert_eq!(
            result_json(result),
            json!({"title":"title","url":"https://example.org","snippet":"ü字","source":"exa","sources":["exa","parallel"],"published_date":"2026-01-01","citation_span":2})
        );
        assert!(result_json(SearchResult::default())
            .get("published_date")
            .is_none());
    }
}
