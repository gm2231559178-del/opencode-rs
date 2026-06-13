use super::{Tool, ToolContext, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WebsearchTool;

#[async_trait]
impl Tool for WebsearchTool {
    fn name(&self) -> &str {
        "websearch"
    }

    fn description(&self) -> &str {
        "Search the web for up-to-date information. Returns search results with snippets and URLs."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "numResults": {
                    "type": "integer",
                    "description": "Number of search results to return (default: 8)",
                    "default": 8
                },
                "livecrawl": {
                    "type": "string",
                    "enum": ["fallback", "preferred"],
                    "description": "Live crawl mode"
                },
                "type": {
                    "type": "string",
                    "enum": ["auto", "fast", "deep"],
                    "description": "Search type"
                },
                "contextMaxCharacters": {
                    "type": "integer",
                    "description": "Maximum characters for context"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;
        let num_results = args["numResults"].as_u64().unwrap_or(8).min(20) as usize;

        let api_key = ctx.config.as_ref().and_then(|c| {
            c.provider.get("websearch").and_then(|p| p.api_key.as_deref())
        });

        if let Some(key) = api_key {
            if !key.is_empty() {
                return search_via_provider(query, num_results, key).await;
            }
        }

        search_via_scrape(query, num_results).await
    }
}

async fn search_via_provider(query: &str, num_results: usize, api_key: &str) -> Result<ToolResult> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let response = client
        .get("https://api.exa.ai/search")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "query": query,
            "num_results": num_results,
        }))
        .send()
        .await?;

    if response.status().is_success() {
        let body: Value = response.json().await?;
        let results = body["results"].as_array().cloned().unwrap_or_default();
        let mut output = format!("Web search results for: {}\n\n", query);
        for (i, r) in results.iter().enumerate() {
            let title = r["title"].as_str().unwrap_or("(no title)");
            let url = r["url"].as_str().unwrap_or("");
            let snippet = r["snippet"].as_str().unwrap_or("");
            output.push_str(&format!("{}. {} | {}\n   {}\n\n", i + 1, title, url, snippet));
        }
        return Ok(ToolResult {
            title: format!("Web search: {}", query),
            output,
            metadata: json!({"query": query, "results_count": results.len()}),
        });
    }

    Err(anyhow::anyhow!("Search API error: {}", response.status()))
}

async fn search_via_scrape(query: &str, _num_results: usize) -> Result<ToolResult> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (compatible; opencode-rs/0.1)")
        .build()?;

    let url = format!("https://lite.duckduckgo.com/lite/?q={}", urlencoding(query));
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Search service returned HTTP {}", response.status());
    }

    let html = response.text().await?;

    let re_link = regex::Regex::new(r#"(?i)<a[^>]+class="result-link"[^>]*href="([^"]*)"[^>]*>(.*?)</a>"#).unwrap();
    let re_snippet = regex::Regex::new(r#"(?i)<td[^>]*class="result-snippet"[^>]*>(.*?)</td>"#).unwrap();

    let mut output = format!("Web search results for: {}\n\n", query);

    let links: Vec<(&str, &str)> = re_link
        .captures_iter(&html)
        .map(|c| (c.get(1).map(|m| m.as_str()).unwrap_or(""), c.get(2).map(|m| m.as_str()).unwrap_or("")))
        .collect();

    let snippets: Vec<&str> = re_snippet
        .captures_iter(&html)
        .map(|c| c.get(1).map(|m| m.as_str()).unwrap_or(""))
        .collect();

    for (i, ((url, title), snippet)) in links.iter().zip(snippets.iter()).enumerate().take(10) {
        let clean_title = strip_html_tags(title);
        let clean_snippet = strip_html_tags(snippet);
        output.push_str(&format!("{}. {} | {}\n   {}\n\n", i + 1, clean_title, url, clean_snippet));
    }

    if links.is_empty() {
        output.push_str("(No results found)\n");
    }

    Ok(ToolResult {
        title: format!("Web search: {}", query),
        output,
        metadata: json!({"query": query, "results_count": links.len()}),
    })
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => '+'.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

fn strip_html_tags(s: &str) -> String {
    let re = regex::Regex::new(r"<[^>]+>").unwrap();
    let text = re.replace_all(s, " ");
    text.trim().to_string()
}
