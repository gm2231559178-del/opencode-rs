use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WebfetchTool;

#[async_trait]
impl Tool for WebfetchTool {
    fn name(&self) -> &str {
        "webfetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL and return it as text, markdown, or HTML"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "description": "The format to return the content in"
                },
                "timeout": {
                    "type": ["integer", "null"],
                    "description": "Optional timeout in seconds"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' argument"))?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            anyhow::bail!("URL must start with http:// or https://");
        }

        let format = args["format"].as_str().unwrap_or("markdown");
        let timeout_secs = args["timeout"].as_u64().unwrap_or(60);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .user_agent("opencode-rs/0.1")
            .build()
            .context("Failed to build HTTP client")?;

        let response = client
            .get(url)
            .send()
            .await
            .context("Failed to fetch URL")?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP {}: {}", response.status(), url);
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let output = if format == "html" || format == "markdown" {
            let bytes = response.bytes().await?;
            let body = String::from_utf8_lossy(&bytes);
            if format == "markdown" && content_type.contains("html") {
                let html = strip_html(&body);
                format!("URL: {}\nContent-Type: {}\n\n{}", url, content_type, html)
            } else {
                format!("URL: {}\nContent-Type: {}\n\n{}", url, content_type, body)
            }
        } else {
            let text = response.text().await?;
            format!("URL: {}\nContent-Type: {}\n\n{}", url, content_type, text)
        };

        Ok(ToolResult {
            title: format!("Fetched: {}", url),
            output,
            metadata: json!({"url": url, "content_type": content_type}),
        })
    }
}

fn strip_html(html: &str) -> String {
    let re_script = regex::Regex::new(r"(?i)<script[^>]*>.*?</script>").unwrap();
    let re_style = regex::Regex::new(r"(?i)<style[^>]*>.*?</style>").unwrap();
    let re_tags = regex::Regex::new(r"<[^>]+>").unwrap();
    let re_whitespace = regex::Regex::new(r"\s+").unwrap();
    let s = re_script.replace_all(html, "");
    let s = re_style.replace_all(&s, "");
    let s = re_tags.replace_all(&s, " ");
    let s = re_whitespace.replace_all(&s, " ");
    let s = s.trim();
    if s.len() > 1_000_000 {
        format!("{}... [truncated {} chars]", &s[..1_000_000], s.len() - 1_000_000)
    } else {
        s.to_string()
    }
}
