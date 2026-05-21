//! Local Ollama provider.
//!
//! Talks to a locally-running Ollama daemon (default
//! `http://localhost:11434`). No API key required. Ollama exposes an
//! OpenAI-compatible `/v1/chat/completions` endpoint so we reuse that
//! shape directly.

use crate::error::{DanbiError, DanbiResult};
use crate::providers::{ModelInfo, Provider, TestResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const USER_AGENT: &str = "danbi/0.1";

pub struct OllamaProvider {
    pub base_url: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    options: Options,
}

#[derive(Serialize)]
struct Options {
    num_predict: i32,
    temperature: f32,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    message: Option<RespMessage>,
}

#[derive(Deserialize)]
struct RespMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

#[async_trait]
impl Provider for OllamaProvider {
    fn kind(&self) -> &'static str {
        "ollama"
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
            stream: false,
            options: Options {
                num_predict: max_tokens.max(1),
                temperature: temperature.max(0.0),
            },
        };

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;

        let resp = client
            .post(format!("{}/api/chat", self.base_url.trim_end_matches('/')))
            .json(&body)
            .send()
            .await
            .map_err(|e| DanbiError::Other(format!("ollama request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!("ollama {status}: {snippet}")));
        }

        let parsed: ChatResponse = resp.json().await.map_err(|e| {
            DanbiError::Other(format!("ollama response parse: {e}"))
        })?;
        Ok(parsed.message.map(|m| m.content).unwrap_or_default())
    }

    /// Ollama exposes a real `/api/tags` endpoint listing locally-pulled
    /// models. Use it when reachable; fall back to a suggestion catalog
    /// when the daemon isn't running or lists nothing.
    async fn list_models(&self) -> DanbiResult<Vec<ModelInfo>> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;

        let url = format!("{}/api/tags", self.base_url.trim_end_matches('/'));
        let result = client.get(&url).send().await;
        match result {
            Ok(resp) if resp.status().is_success() => {
                let tags: TagsResponse =
                    resp.json().await.unwrap_or(TagsResponse { models: vec![] });
                if tags.models.is_empty() {
                    return Ok(suggested_catalog());
                }
                Ok(tags
                    .models
                    .into_iter()
                    .map(|m| ModelInfo {
                        id: m.name.clone(),
                        name: Some(m.name),
                        provider: Some("Ollama (local)".into()),
                        on_demand: true,
                        modalities_in: vec!["TEXT".into()],
                        modalities_out: vec!["TEXT".into()],
                    })
                    .collect())
            }
            _ => Ok(suggested_catalog()),
        }
    }

    async fn test_connection(&self, model_id: Option<&str>) -> DanbiResult<TestResult> {
        let model = model_id.unwrap_or("llama3.2");
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
        // Ollama's /api/embed returns one embedding per input text.
        #[derive(Serialize)]
        struct EmbedRequest<'a> {
            model: &'a str,
            input: &'a [String],
        }
        #[derive(Deserialize)]
        struct EmbedResponse {
            #[serde(default)]
            embeddings: Vec<Vec<f32>>,
        }

        let body = EmbedRequest { model: model_id, input: inputs };
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;
        let resp = client
            .post(format!("{}/api/embed", self.base_url.trim_end_matches('/')))
            .json(&body)
            .send()
            .await
            .map_err(|e| DanbiError::Other(format!("ollama embed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!("ollama embed {status}: {snippet}")));
        }
        let parsed: EmbedResponse = resp
            .json()
            .await
            .map_err(|e| DanbiError::Other(format!("ollama embed parse: {e}")))?;
        Ok(parsed.embeddings)
    }

    fn default_embed_model(&self) -> &'static str {
        "nomic-embed-text"
    }
}

/// Fallback list when the daemon isn't reachable. Helps the onboarding
/// picker show something useful before the user pulls models.
fn suggested_catalog() -> Vec<ModelInfo> {
    let mk = |id: &str, name: &str| ModelInfo {
        id: id.into(),
        name: Some(name.into()),
        provider: Some("Ollama".into()),
        on_demand: true,
        modalities_in: vec!["TEXT".into()],
        modalities_out: vec!["TEXT".into()],
    };
    vec![
        mk("llama3.2", "Llama 3.2 3B (pull 필요)"),
        mk("llama3.1", "Llama 3.1 8B (pull 필요)"),
        mk("qwen2.5:7b", "Qwen 2.5 7B (pull 필요)"),
        mk("mistral", "Mistral 7B (pull 필요)"),
    ]
}

pub fn catalog() -> Vec<ModelInfo> {
    suggested_catalog()
}
