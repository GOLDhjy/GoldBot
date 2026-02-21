use anyhow::{Result, anyhow};
use serde_json::Value;

const BOCHA_API_URL: &str = "https://api.bocha.cn/v1/web-search";
const MAX_RESULT_CHARS: usize = 8_000;

pub struct SearchResult {
    pub output: String,
}

pub fn search(query: &str) -> Result<SearchResult> {
    let api_key = std::env::var("BOCHA_API_KEY")
        .map_err(|_| anyhow!("BOCHA_API_KEY environment variable not set"))?;

    let body = serde_json::json!({
        "query": query,
        "summary": true,
        "freshness": "noLimit",
        "count": 10
    });

    let (status, text) = tokio::task::block_in_place(|| -> Result<_> {
        let response = reqwest::blocking::Client::new()
            .post(BOCHA_API_URL)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()?;
        let status = response.status();
        let text = response.text()?;
        Ok((status, text))
    })?;

    if !status.is_success() {
        return Err(anyhow!("Bocha API error {status}: {text}"));
    }

    let json: Value =
        serde_json::from_str(&text).map_err(|e| anyhow!("Failed to parse Bocha response: {e}"))?;

    let mut output = format_results(&json, query);
    if output.len() > MAX_RESULT_CHARS {
        output.truncate(MAX_RESULT_CHARS);
        output.push_str("\n...[truncated]");
    }

    Ok(SearchResult { output })
}

fn format_results(json: &Value, query: &str) -> String {
    let mut out = format!("Web search results for: \"{query}\"\n\n");

    // AI summary
    if let Some(summary) = json
        .get("data")
        .and_then(|d| d.get("summary"))
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
    {
        out.push_str("Summary:\n");
        out.push_str(summary);
        out.push_str("\n\n");
    }

    // Individual results
    if let Some(items) = json
        .get("data")
        .and_then(|d| d.get("webPages"))
        .and_then(|w| w.get("value"))
        .and_then(|v| v.as_array())
    {
        out.push_str("Results:\n");
        for (i, item) in items.iter().take(5).enumerate() {
            let title = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!(
                "{}. {}\n   {}\n   {}\n\n",
                i + 1,
                title,
                url,
                snippet
            ));
        }
    }

    out
}
