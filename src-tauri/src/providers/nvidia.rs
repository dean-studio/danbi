//! NVIDIA NIM provider — OpenAI-compatible HTTP client against
//! `https://integrate.api.nvidia.com/v1/chat/completions`.
//!
//! The API shape is a near-exact OpenAI v1 clone: POST chat/completions with
//! `model`, `messages`, optional `temperature`, `max_tokens`. We only use the
//! non-streaming path — single roundtrip, single assistant message back — so
//! the call site (`converse_text`) mirrors the Bedrock implementation.
//!
//! Models come from a hand-curated catalog (see `curated_catalog`) because
//! NVIDIA build doesn't expose a public model-discovery endpoint; the catalog
//! is updated manually when new families ship.

use crate::error::{DanbiError, DanbiResult};
use crate::providers::{ModelInfo, Provider, TestResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://integrate.api.nvidia.com/v1";
const USER_AGENT: &str = "danbi/0.1";

pub struct NvidiaProvider {
    /// Fully-resolved API key (already fetched from Keychain by the caller).
    pub api_key: String,
}

// ---- Request / response wire types ----

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i32>,
    /// We always run non-streaming here. Kept explicit so the NIM gateway
    /// doesn't default to its own streaming behavior for any model.
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

// ---- Public trait impl ----

#[async_trait]
impl Provider for NvidiaProvider {
    fn kind(&self) -> &'static str {
        "nvidia"
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
            messages.push(Message {
                role: "system",
                content: s,
            });
        }
        messages.push(Message {
            role: "user",
            content: user_text,
        });

        let body = ChatRequest {
            model: model_id,
            messages,
            // NVIDIA clamps temperature to (0.0, 1.0] for many models; 0.0 is
            // accepted. We pass the caller value as-is and let the gateway
            // reject invalid combinations with a clear message.
            temperature: Some(temperature.max(0.0)),
            max_tokens: Some(max_tokens.max(1)),
            stream: false,
        };

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;

        let resp = client
            .post(format!("{BASE_URL}/chat/completions"))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| DanbiError::Other(format!("nvidia request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            // Keep the body snippet short — API error payloads can be big
            // and we want it readable in the UI notifier.
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!(
                "nvidia {status}: {snippet}"
            )));
        }

        let parsed: ChatResponse = resp
            .json()
            .await
            .map_err(|e| DanbiError::Other(format!("nvidia response parse: {e}")))?;

        let text = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();
        Ok(text)
    }

    async fn list_models(&self) -> DanbiResult<Vec<ModelInfo>> {
        Ok(curated_catalog())
    }

    async fn test_connection(&self, model_id: Option<&str>) -> DanbiResult<TestResult> {
        // Minimum viable probe: one tiny completion against the given model,
        // or against a known-cheap default if none provided. We deliberately
        // don't fall back to a control-plane "list models" call here because
        // NVIDIA build doesn't expose one.
        let model = model_id.unwrap_or("meta/llama-3.3-70b-instruct");
        match self.converse_text(model, None, "ping", 4, 0.0).await {
            Ok(_) => Ok(TestResult {
                ok: true,
                detail: format!("connected; runtime invoke on {model} ok"),
            }),
            Err(e) => Ok(TestResult {
                ok: false,
                detail: format!("runtime invoke failed on {model}: {e}"),
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
            #[serde(skip_serializing_if = "Option::is_none")]
            input_type: Option<&'a str>,
        }
        #[derive(Deserialize)]
        struct EmbedResponse {
            data: Vec<EmbedItem>,
        }
        #[derive(Deserialize)]
        struct EmbedItem {
            embedding: Vec<f32>,
        }

        // NVIDIA's embedqa models require an `input_type` hint; we pass
        // "passage" which is what llm_wiki-style indexing wants. Other
        // embedding model families just ignore the field.
        let body = EmbedRequest {
            model: model_id,
            input: inputs,
            input_type: Some("passage"),
        };
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;
        let resp = client
            .post(format!("{BASE_URL}/embeddings"))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| DanbiError::Other(format!("nvidia embed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!("nvidia embed {status}: {snippet}")));
        }
        let parsed: EmbedResponse = resp
            .json()
            .await
            .map_err(|e| DanbiError::Other(format!("nvidia embed parse: {e}")))?;
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }

    fn default_embed_model(&self) -> &'static str {
        "nvidia/nv-embedqa-e5-v5"
    }
}

// ---- Hand-curated model catalog ----
//
// Keep the list short and obviously-useful rather than comprehensive. These
// IDs are the exact strings NVIDIA's OpenAI-compatible endpoint expects in
// the `model` field. Descriptions double as UI tooltip material.

fn curated_catalog() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "meta/llama-3.3-70b-instruct".into(),
            name: Some("Llama 3.3 70B Instruct".into()),
            provider: Some("Meta".into()),
            on_demand: true,
            modalities_in: vec!["TEXT".into()],
            modalities_out: vec!["TEXT".into()],
        },
        ModelInfo {
            id: "meta/llama-3.1-8b-instruct".into(),
            name: Some("Llama 3.1 8B Instruct".into()),
            provider: Some("Meta".into()),
            on_demand: true,
            modalities_in: vec!["TEXT".into()],
            modalities_out: vec!["TEXT".into()],
        },
        ModelInfo {
            id: "deepseek-ai/deepseek-r1".into(),
            name: Some("DeepSeek R1".into()),
            provider: Some("DeepSeek".into()),
            on_demand: true,
            modalities_in: vec!["TEXT".into()],
            modalities_out: vec!["TEXT".into()],
        },
        ModelInfo {
            id: "qwen/qwen2.5-7b-instruct".into(),
            name: Some("Qwen2.5 7B Instruct".into()),
            provider: Some("Alibaba".into()),
            on_demand: true,
            modalities_in: vec!["TEXT".into()],
            modalities_out: vec!["TEXT".into()],
        },
        ModelInfo {
            id: "mistralai/mixtral-8x22b-instruct-v0.1".into(),
            name: Some("Mixtral 8x22B Instruct".into()),
            provider: Some("Mistral AI".into()),
            on_demand: true,
            modalities_in: vec!["TEXT".into()],
            modalities_out: vec!["TEXT".into()],
        },
    ]
}

/// Public view of the catalog for UI code that wants it without instantiating
/// a provider (e.g. the onboarding wizard before the API key is confirmed).
pub fn catalog() -> Vec<ModelInfo> {
    curated_catalog()
}
