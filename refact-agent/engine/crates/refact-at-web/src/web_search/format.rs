use super::{format_search_results, SearchResult, WebSearchRequest, ProviderReport, ProviderStatus};
pub(super) fn outcome(
    request: &WebSearchRequest,
    results: &[SearchResult],
    reports: &[ProviderReport],
) -> String {
    let mut text = format_search_results(&request.queries.join("\" | \""), &[]);
    let reminder = refact_core::chat_types::web_search_trailer(
        &chrono::Utc::now().format("%Y-%m-%d").to_string(),
    );
    let budget = 16_000 - reminder.chars().count();
    let query = request.queries.join("\" | \"");
    if !results.is_empty() {
        text = format!("Web search results for \"{}\":\n\n", query);
    }
    if text.chars().count() > budget {
        text = "Web search results:\n\n".into();
    }
    for end in 1..=results.len() {
        let candidate = format_search_results(&query, &results[..end]);
        if candidate.chars().count() > budget {
            break;
        }
        text = candidate;
    }
    let mut has_notes = false;
    for report in reports.iter().filter(|r| {
        matches!(r.status, ProviderStatus::Error | ProviderStatus::Timeout)
            || r.status == ProviderStatus::Skipped
                && r.detail
                    .as_deref()
                    .is_some_and(|d| d.starts_with("temporarily skipped"))
    }) {
        let note = format!(
            "{}- {}: {}\n",
            if has_notes { "" } else { "\nProvider notes:\n" },
            report.provider,
            report.detail.as_deref().unwrap_or("unavailable")
        );
        if text.chars().count() + note.chars().count() <= budget {
            text.push_str(&note);
            has_notes = true;
        }
    }
    text.push_str(&reminder);
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notes_only_describe_failures_and_circuit_cooldowns() {
        let reports = [
            (ProviderStatus::NotApplicable, "fallback not needed"),
            (ProviderStatus::Skipped, "fallback not needed"),
            (
                ProviderStatus::Skipped,
                "temporarily skipped after repeated failures",
            ),
            (ProviderStatus::Error, "network error"),
            (ProviderStatus::Timeout, "Provider timed out"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (status, detail))| ProviderReport {
            provider: format!("p{index}"),
            status,
            results: 0,
            latency_ms: 0,
            detail: Some(detail.into()),
            top: vec![],
        })
        .collect::<Vec<_>>();
        let text = outcome(&WebSearchRequest::default(), &[], &reports);
        assert!(!text.contains("fallback not needed"));
        assert!(text.contains("temporarily skipped"));
        assert!(text.contains("network error"));
        assert!(text.contains("Provider timed out"));
    }
    #[test]
    fn budget_preserves_complete_records_and_reminder() {
        let results = (0..20)
            .map(|i| SearchResult {
                title: format!("title{i}{}", "t".repeat(600)),
                url: format!("https://a.test/{i}"),
                snippet: "s".repeat(600),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let text = outcome(
            &WebSearchRequest {
                queries: vec!["q".into()],
                ..Default::default()
            },
            &results,
            &[],
        );
        assert!(text.contains(&format!(
            "Today is {} (UTC).",
            chrono::Utc::now().format("%Y-%m-%d")
        )));
        assert!(text.chars().count() <= 16_000);
        let body = text.split("\nToday is").next().unwrap();
        assert!(body.ends_with("\n\n"));
    }
}
