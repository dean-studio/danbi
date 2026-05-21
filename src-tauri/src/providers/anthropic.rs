//! Native Anthropic API provider (messages API).
//!
//! Different wire shape from OpenAI — top-level `system` field and a
//! `content` array in the response. Handled separately here to avoid
//! sprinkling conditionals through the OpenAI-compatible path.

use crate::error::{DanbiError, DanbiResult};
use crate::providers::{ModelInfo, Provider, TestResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const USER_AGENT: &str = "danbi/0.1";

pub struct AnthropicProvider {
    pub api_key: String,
}

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    max_tokens: i32,
    temperature: f32,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<Block>,
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn kind(&self) -> &'static str {
        "anthropic"
    }

    async fn converse_text(
        &self,
        model_id: &str,
        system: Option<&str>,
        user_text: &str,
        max_tokens: i32,
        temperature: f32,
    ) -> DanbiResult<String> {
        let body = MessagesRequest {
            model: model_id,
            messages: vec![Message { role: "user", content: user_text }],
            system,
            max_tokens: max_tokens.max(1),
            temperature: temperature.max(0.0),
        };

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;

        let resp = client
            .post(format!("{BASE_URL}/messages"))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| DanbiError::Other(format!("anthropic request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!("anthropic {status}: {snippet}")));
        }

        let parsed: MessagesResponse = resp.json().await.map_err(|e| {
            DanbiError::Other(format!("anthropic response parse: {e}"))
        })?;

        // Concat any text blocks (Anthropic may interleave tool_use, image,
        // etc. in other contexts; we only care about text here).
        let text = parsed
            .content
            .into_iter()
            .filter(|b| b.kind == "text")
            .map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");
        Ok(text)
    }

    async fn list_models(&self) -> DanbiResult<Vec<ModelInfo>> {
        Ok(catalog())
    }

    async fn test_connection(&self, model_id: Option<&str>) -> DanbiResult<TestResult> {
        let model = model_id.unwrap_or("claude-haiku-4-5");
        match self.converse_text(model, None, "ping", 4, 0.0).await {
            Ok(_) => Ok(TestResult {
                ok: true,
                detail: format!("connected; invoke on {model} ok"),
            }),
            Err(e) => Ok(TestResult {
                ok: false,
                detail: format!("invoke failed on {model}: {e}"),
            }),
        }
    }
}

pub fn catalog() -> Vec<ModelInfo> {
    let mk = |id: &str, name: &str| ModelInfo {
        id: id.into(),
        name: Some(name.into()),
        provider: Some("Anthropic".into()),
        on_demand: true,
        modalities_in: vec!["TEXT".into()],
        modalities_out: vec!["TEXT".into()],
    };
    vec![
        mk("claude-haiku-4-5", "Claude Haiku 4.5"),
        mk("claude-sonnet-4-6", "Claude Sonnet 4.6"),
        mk("claude-opus-4-7", "Claude Opus 4.7"),
    ]
}
