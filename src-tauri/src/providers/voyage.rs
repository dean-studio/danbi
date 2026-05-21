//! Voyage AI provider — embedding-only.
//!
//! Voyage doesn't offer a chat/completion endpoint, so this provider is
//! meant to be selected as `embed_provider` in DanbiConfig, never as the
//! main LLM provider. If someone tries to call `converse_text` we return
//! a clear error. `list_models` returns a curated catalog since Voyage's
//! public model listing endpoint is undocumented.

use crate::error::{DanbiError, DanbiResult};
use crate::providers::{ModelInfo, Provider, TestResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://api.voyageai.com/v1";
const USER_AGENT: &str = "danbi/0.1";

pub struct VoyageProvider {
    pub api_key: String,
}

impl VoyageProvider {
    /// Single embed call without retry. Used by both the public `embed`
    /// (which wraps with one 429-aware retry) and the connection test.
    async fn embed_once(
        &self,
        model_id: &str,
        texts: &[String],
    ) -> DanbiResult<Vec<Vec<f32>>> {
        let body = EmbedRequest {
            model: model_id,
            input: texts,
            input_type: None,
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
            .map_err(|e| DanbiError::Other(format!("voyage embed request: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!(
                "voyage embed {status}: {snippet}"
            )));
        }
        let parsed: EmbedResponse = resp
            .json()
            .await
            .map_err(|e| DanbiError::Other(format!("voyage embed parse: {e}")))?;
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
    /// `document` for indexing-side text, `query` for user queries.
    /// Defaults to `document`; we leave it unset so the caller-side
    /// bias isn't baked in — Voyage treats omission as `null` which is
    /// fine for our mixed-use (vault corpus).
    #[serde(skip_serializing_if = "Option::is_none")]
    input_type: Option<&'a str>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedItem>,
}

#[derive(Deserialize)]
struct EmbedItem {
    #[serde(default)]
    embedding: Vec<f32>,
}

#[async_trait]
impl Provider for VoyageProvider {
    fn kind(&self) -> &'static str {
        "voyage"
    }

    async fn converse_text(
        &self,
        _model_id: &str,
        _system: Option<&str>,
        _user_text: &str,
        _max_tokens: i32,
        _temperature: f32,
    ) -> DanbiResult<String> {
        Err(DanbiError::Other(
            "Voyage AI 는 embedding 전용이에요. Writer/Routing 모델로는 쓸 수 없어요.".into(),
        ))
    }

    async fn list_models(&self) -> DanbiResult<Vec<ModelInfo>> {
        Ok(catalog())
    }

    async fn test_connection(&self, model_id: Option<&str>) -> DanbiResult<TestResult> {
        let model = model_id.unwrap_or("voyage-3-lite");
        let inputs = vec!["ping".to_string()];
        match self.embed(model, &inputs).await {
            Ok(v) if !v.is_empty() && !v[0].is_empty() => Ok(TestResult {
                ok: true,
                detail: format!("connected; embed on {model} ok (dim={})", v[0].len()),
            }),
            Ok(_) => Ok(TestResult {
                ok: false,
                detail: "empty response".into(),
            }),
            Err(e) => Ok(TestResult {
                ok: false,
                detail: format!("embed failed on {model}: {e}"),
            }),
        }
    }

    async fn embed(
        &self,
        model_id: &str,
        texts: &[String],
    ) -> DanbiResult<Vec<Vec<f32>>> {
        // Voyage's free tier (no payment method) is gated at 3 RPM /
        // 10K TPM. Hitting that threshold returns 429 with a long
        // explanatory body. We retry once after a 25s sleep — long
        // enough that the 1-minute window has rolled over for at least
        // one slot. Anything beyond that bubbles up as a friendly
        // error so Settings can surface guidance.
        match self.embed_once(model_id, texts).await {
            Ok(v) => Ok(v),
            Err(e) => {
                let msg = format!("{e}");
                let is_rate_limited = msg.contains("429");
                if !is_rate_limited {
                    return Err(e);
                }
                tokio::time::sleep(std::time::Duration::from_secs(25)).await;
                self.embed_once(model_id, texts).await.map_err(|e2| {
                    DanbiError::Other(format!(
                        "voyage rate limit hit twice (free tier is 3 RPM / 10K TPM \
                         without a payment method on file). {e2}"
                    ))
                })
            }
        }
    }

    fn default_embed_model(&self) -> &'static str {
        "voyage-multilingual-2"
    }
}

pub fn catalog() -> Vec<ModelInfo> {
    let mk = |id: &str, name: &str| ModelInfo {
        id: id.into(),
        name: Some(name.into()),
        provider: Some("Voyage AI".into()),
        on_demand: true,
        modalities_in: vec!["TEXT".into()],
        modalities_out: vec!["EMBEDDING".into()],
    };
    vec![
        mk("voyage-multilingual-2", "Voyage Multilingual 2 (한국어 권장)"),
        mk("voyage-3", "Voyage 3"),
        mk("voyage-3-lite", "Voyage 3 Lite (빠름)"),
        mk("voyage-code-3", "Voyage Code 3 (코드 특화)"),
    ]
}
