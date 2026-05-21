//! OpenAI (and OpenAI-compatible) provider.
//!
//! Talks to `<base_url>/chat/completions` using the Chat Completions
//! format. `base_url` defaults to `https://api.openai.com/v1` but can be
//! overridden to support Azure OpenAI, self-hosted vLLM/llama.cpp, and
//! other proxies that accept the OpenAI wire protocol.

use crate::error::{DanbiError, DanbiResult};
use crate::providers::{ModelInfo, Provider, TestResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const USER_AGENT: &str = "danbi/0.1";

pub struct OpenaiProvider {
    pub api_key: String,
    pub base_url: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i32>,
    stream: bool,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: RespMessage,
}

#[derive(Deserialize)]
struct RespMessage {
    #[serde(default)]
    content: String,
}

#[async_trait]
impl Provider for OpenaiProvider {
    fn kind(&self) -> &'static str {
        "openai"
    }

    async fn converse_text(
        &self,
        model_id: &str,
        system: Option<&str>,
        user_text: &str,
        max_tokens: i32,
        temperature: f32,
    ) -> DanbiResult<String> {
        let mut messages: Vec<Message> = Vec::with_capacity(2);
        if let Some(s) = system {
            messages.push(Message { role: "system", content: s });
        }
        messages.push(Message { role: "user", content: user_text });

        let body = ChatRequest {
            model: model_id,
            messages,
            temperature: Some(temperature.max(0.0)),
            max_tokens: Some(max_tokens.max(1)),
            stream: false,
        };

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;

        let resp = client
            .post(format!("{}/chat/completions", self.base_url.trim_end_matches('/')))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| DanbiError::Other(format!("openai request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!("openai {status}: {snippet}")));
        }

        let parsed: ChatResponse = resp
            .json()
            .await
            .map_err(|e| DanbiError::Other(format!("openai response parse: {e}")))?;

        Ok(parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }

    async fn list_models(&self) -> DanbiResult<Vec<ModelInfo>> {
        Ok(catalog())
    }

    async fn test_connection(&self, model_id: Option<&str>) -> DanbiResult<TestResult> {
        let model = model_id.unwrap_or("gpt-4o-mini");
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

    async fn embed(
        &self,
        model_id: &str,
        inputs: &[String],
    ) -> DanbiResult<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        #[derive(Serialize)]
        struct EmbedRequest<'a> {
            model: &'a str,
            input: &'a [String],
        }
        #[derive(Deserialize)]
        struct EmbedResponse {
            data: Vec<EmbedItem>,
        }
        #[derive(Deserialize)]
        struct EmbedItem {
            embedding: Vec<f32>,
        }

        let body = EmbedRequest { model: model_id, input: inputs };
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;
        let resp = client
            .post(format!("{}/embeddings", self.base_url.trim_end_matches('/')))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| DanbiError::Other(format!("openai embed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!("openai embed {status}: {snippet}")));
        }
        let parsed: EmbedResponse = resp
            .json()
            .await
            .map_err(|e| DanbiError::Other(format!("openai embed parse: {e}")))?;
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }

    fn default_embed_model(&self) -> &'static str {
        "text-embedding-3-small"
    }
}

pub fn catalog() -> Vec<ModelInfo> {
    // Curated set of OpenAI chat models users most commonly reach for.
    // Expand as needed — keeping it short keeps the onboarding clean.
    let mk = |id: &str, name: &str| ModelInfo {
        id: id.into(),
        name: Some(name.into()),
        provider: Some("OpenAI".into()),
        on_demand: true,
        modalities_in: vec!["TEXT".into()],
        modalities_out: vec!["TEXT".into()],
    };
    vec![
        mk("gpt-4o", "GPT-4o"),
        mk("gpt-4o-mini", "GPT-4o mini"),
        mk("gpt-4.1", "GPT-4.1"),
        mk("gpt-4.1-mini", "GPT-4.1 mini"),
        mk("o4-mini", "o4-mini"),
    ]
}
