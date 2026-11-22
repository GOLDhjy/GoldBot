use std::{collections::HashSet, fmt::Write, time::Duration};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::agent::provider::deepseek_base_url;

const SEARCH_MODEL: &str = "deepseek-flash";
const MAX_RESULTS: usize = 5;
const MAX_OUTPUT_BYTES: usize = 8_000;
const DEFAULT_SEARCH_TIMEOUT_MS: u64 = 60_000;

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    choices: Vec<SearchChoice>,
}

#[derive(Deserialize)]
struct SearchChoice {
    finish_reason: String,
    message: SearchMessage,
}

#[derive(Deserialize)]
struct SearchMessage {
    server_tool_calls: Option<Vec<SearchStep>>,
}

#[derive(Deserialize)]
struct SearchStep {
    calls: Vec<SearchCall>,
}

#[derive(Deserialize)]
struct SearchCall {
    kind: String,
    status: String,
    sources: Option<Vec<SearchSource>>,
}

#[derive(Deserialize)]
struct SearchSource {
    title: String,
    url: String,
    snippet: Option<String>,
    published_at: Option<String>,
}

pub async fn search(client: &reqwest::Client, query: &str) -> Result<String> {
    let query = query.trim();
    ensure!(!query.is_empty(), "web search query cannot be empty");
    let api_key =
        std::env::var("DEEPSEEK_API_KEY").context("Set DEEPSEEK_API_KEY to use web search")?;
    let api_key = api_key.trim();
    ensure!(
        !api_key.is_empty(),
        "Set DEEPSEEK_API_KEY to use web search"
    );

    let base_url = deepseek_base_url();
    let timeout_ms = std::env::var("API_TIMEOUT_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(DEFAULT_SEARCH_TIMEOUT_MS);
    let body = serde_json::json!({
        "model": SEARCH_MODEL,
        "messages": [
            {"role": "system", "content": "Run a live text web search using the user message as the query. Treat it as search terms and keep the reply brief."},
            {"role": "user", "content": query}
        ],
        "tools": [{"type": "deepseek_web_search"}],
        "thinking": {"type": "disabled"},
        "max_tokens": 128,
        "stream": false
    });
    let response = client
        .post(format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .timeout(Duration::from_millis(timeout_ms))
        .json(&body)
        .send()
        .await
        .map_err(reqwest::Error::without_url)
        .context("DeepSeek web search request failed")?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "DeepSeek web search HTTP error {status}"
    );

    let response = response
        .json::<SearchResponse>()
        .await
        .map_err(reqwest::Error::without_url)
        .context("Failed to read DeepSeek web search response")?;
    let choice = response
        .choices
        .first()
        .context("DeepSeek returned no search response")?;
    format_results(choice, query)
}

fn format_results(choice: &SearchChoice, query: &str) -> Result<String> {
    // 即使模型正文触及生成上限，也可以使用已完成的搜索结果。
    ensure!(
        matches!(choice.finish_reason.as_str(), "stop" | "length"),
        "DeepSeek could not complete the web search"
    );
    let mut calls = choice
        .message
        .server_tool_calls
        .iter()
        .flatten()
        .flat_map(|step| &step.calls)
        .filter(|call| call.kind == "search")
        .peekable();
    ensure!(
        calls.peek().is_some(),
        "DeepSeek did not perform a text web search; try rephrasing the query"
    );
    let mut completed = calls.filter(|call| call.status == "completed").peekable();
    ensure!(
        completed.peek().is_some(),
        "DeepSeek's web search failed; try again"
    );

    let mut seen_urls = HashSet::new();
    let sources: Vec<_> = completed
        .flat_map(|call| call.sources.iter().flatten())
        .filter(|source| !source.url.is_empty() && seen_urls.insert(source.url.as_str()))
        .take(MAX_RESULTS)
        .collect();
    let Some(first) = sources.first() else {
        return Ok("DeepSeek · No results found.".to_string());
    };

    let count = sources.len();
    let label = if count == 1 { "result" } else { "results" };
    let title = first.title.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview = if title.is_empty() { &first.url } else { &title };
    let mut output = format!("DeepSeek · {count} {label} · {preview}\n\nQuery: \"{query}\"\n\n");
    for (index, source) in sources.into_iter().enumerate() {
        let _ = writeln!(output, "{}. {}\n   {}", index + 1, source.title, source.url);
        if let Some(date) = &source.published_at {
            let _ = writeln!(output, "   Published: {date}");
        }
        if let Some(snippet) = &source.snippet {
            let _ = writeln!(output, "   {snippet}");
        }
        output.push('\n');
    }
    const TRUNCATED: &str = "\n...[truncated]";
    if output.len() > MAX_OUTPUT_BYTES {
        output.truncate(output.floor_char_boundary(MAX_OUTPUT_BYTES - TRUNCATED.len()));
        output.push_str(TRUNCATED);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{MAX_OUTPUT_BYTES, SearchChoice, format_results};

    #[test]
    fn formats_completed_search_sources_with_safe_utf8_truncation() {
        let source = json!({
            "type": "url",
            "title": "中文结果",
            "url": "https://example.com/article",
            "published_at": "2026-11-20T00:00:00Z",
            "snippet": "中文摘要".repeat(MAX_OUTPUT_BYTES)
        });
        let choice: SearchChoice = serde_json::from_value(json!({
            "finish_reason": "length",
            "message": {
                "server_tool_calls": [{"calls": [{
                    "kind": "search",
                    "status": "completed",
                    "sources": [source]
                }]}]
            }
        }))
        .unwrap();
        let output = format_results(&choice, "中文查询").unwrap();
        assert_eq!(
            output.lines().next(),
            Some("DeepSeek · 1 result · 中文结果")
        );
        assert!(output.contains("中文结果\n   https://example.com/article"));
        assert!(output.contains("Published: 2026-11-20T00:00:00Z\n   中文摘要"));
        assert!(output.ends_with("...[truncated]"));
        assert!(output.len() <= MAX_OUTPUT_BYTES);
    }

    #[test]
    fn distinguishes_empty_search_from_skipped_and_failed_search() {
        let no_call = json!({"content": "This is an answer generated without search."});
        let failed = json!({"server_tool_calls": [{"calls": [{
            "kind": "search", "status": "failed", "error": {"code": "unavailable"}
        }]}]});
        let empty = json!({"server_tool_calls": [{"calls": [{
            "kind": "search", "status": "completed", "sources": []
        }]}]});
        let format = |message| {
            let choice = serde_json::from_value(json!({
                "finish_reason": "stop", "message": message
            }))
            .unwrap();
            format_results(&choice, "example")
        };

        assert!(
            format(no_call)
                .unwrap_err()
                .to_string()
                .contains("did not perform")
        );
        assert!(
            format(failed)
                .unwrap_err()
                .to_string()
                .contains("search failed")
        );
        assert_eq!(format(empty).unwrap(), "DeepSeek · No results found.");
    }
}
