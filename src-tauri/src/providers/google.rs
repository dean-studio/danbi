//! Google Gemini API provider.
//!
//! Shape: `POST
//! https://generativelanguage.googleapis.com/v1beta/models/<model>:generateContent?key=<KEY>`
//! with a `contents` array. The system prompt becomes `systemInstruction`.

use crate::error::{DanbiError, DanbiResult};
use crate::providers::{ModelInfo, Provider, TestResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const USER_AGENT: &str = "danbi/0.1";

pub struct GoogleProvider {
    pub api_key: String,
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    contents: Vec<Content<'a>>,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<Content<'a>>,
}

#[derive(Serialize)]
struct Content<'a> {
    role: &'a str,
    parts: Vec<Part<'a>>,
}

#[derive(Serialize)]
struct Part<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct GenerationConfig {
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: i32,
    temperature: f32,
}

#[derive(Deserialize)]
struct GenerateResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<CandidateContent>,
}

#[derive(Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Vec<CandidatePart>,
}

#[derive(Deserialize)]
struct CandidatePart {
    #[serde(default)]
    text: String,
}

#[async_trait]
impl Provider for GoogleProvider {
    fn kind(&self) -> &'static str {
        "google"
    }

    async fn converse_text(
        &self,
        model_id: &str,
        system: Option<&str>,
        user_text: &str,
        max_tokens: i32,
        temperature: f32,
    ) -> DanbiResult<String> {
        let body = GenerateRequest {
            contents: vec![Content {
                role: "user",
                parts: vec![Part { text: user_text }],
            }],
            generation_config: GenerationConfig {
                max_output_tokens: max_tokens.max(1),
                temperature: temperature.max(0.0),
            },
            system_instruction: system.map(|s| Content {
                role: "system",
                parts: vec![Part { text: s }],
            }),
        };

        let url = format!(
            "{BASE_URL}/models/{model}:generateContent?key={key}",
            model = model_id,
            key = self.api_key
        );

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;

        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DanbiError::Other(format!("gemini request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!("gemini {status}: {snippet}")));
        }

        let parsed: GenerateResponse = resp.json().await.map_err(|e| {
            DanbiError::Other(format!("gemini response parse: {e}"))
        })?;

        let text = parsed
            .candidates
            .into_iter()
            .next()
            .and_then(|c| c.content)
            .map(|c| {
                c.parts
                    .into_iter()
                    .map(|p| p.text)
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        Ok(text)
    }

    async fn list_models(&self) -> DanbiResult<Vec<ModelInfo>> {
        Ok(catalog())
    }

    async fn embed(
        &self,
        model_id: &str,
        texts: &[String],
    ) -> DanbiResult<Vec<Vec<f32>>> {
        // Gemini exposes a `batchEmbedContents` endpoint that takes an
        // array of per-text requests. Each sub-request must include the
        // `model` field prefixed with `models/`.
        #[derive(Serialize)]
        struct BatchRequest<'a> {
            requests: Vec<SingleEmbed<'a>>,
        }
        #[derive(Serialize)]
        struct SingleEmbed<'a> {
            model: String,
            content: Content<'a>,
        }
        #[derive(Deserialize)]
        struct BatchResponse {
            embeddings: Vec<Embedding>,
        }
        #[derive(Deserialize)]
        struct Embedding {
            #[serde(default)]
            values: Vec<f32>,
        }

        let qualified = if model_id.starts_with("models/") {
            model_id.to_string()
        } else {
            format!("models/{model_id}")
        };

        let body = BatchRequest {
            requests: texts
                .iter()
                .map(|t| SingleEmbed {
                    model: qualified.clone(),
                    content: Content {
                        role: "user",
                        parts: vec![Part { text: t }],
                    },
                })
                .collect(),
        };

        let url = format!(
            "{BASE_URL}/{qualified}:batchEmbedContents?key={key}",
            key = self.api_key
        );

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| DanbiError::Other(format!("reqwest build: {e}")))?;

        // Gemini 무료 티어는 embedding 모델에 분당 5회 요청 제한이 있다.
        // 첫 호출이 429 로 떨어지면 응답 본문의 RetryInfo.retryDelay 를
        // 파싱해 그 만큼 대기한 뒤 한 번 더 재시도. (Voyage 의 자동 재시도
        // 패턴을 재사용 — 사용자가 단일 키로 vault 전체를 reindex 할 때
        // 한 번 막히면 끝까지 막히는 문제를 완화.)
        let mut attempt = 0u8;
        loop {
            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| DanbiError::Other(format!("gemini embed request: {e}")))?;

            let status = resp.status();
            if status.is_success() {
                let parsed: BatchResponse = resp
                    .json()
                    .await
                    .map_err(|e| DanbiError::Other(format!("gemini embed parse: {e}")))?;
                return Ok(parsed.embeddings.into_iter().map(|e| e.values).collect());
            }

            let txt = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 && attempt == 0 {
                let wait_secs = parse_retry_delay_secs(&txt).unwrap_or(35);
                eprintln!(
                    "[danbi] gemini embed 429 — waiting {wait_secs}s then retrying once"
                );
                tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
                attempt += 1;
                continue;
            }

            let snippet: String = txt.chars().take(400).collect();
            return Err(DanbiError::Other(format!(
                "gemini embed {status}: {snippet}"
            )));
        }
    }

    fn default_embed_model(&self) -> &'static str {
        "gemini-embedding-001"
    }

    async fn test_connection(&self, model_id: Option<&str>) -> DanbiResult<TestResult> {
        let model = model_id.unwrap_or("gemini-2.5-flash");
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

/// Best-effort parse of the `retryDelay` field embedded in a Gemini
/// 429 response body. The body looks like:
///
/// ```json
/// {"error":{"details":[{"@type":"...RetryInfo","retryDelay":"35s"}]}}
/// ```
///
/// We do a substring scan instead of full json parse to keep the surface
/// area small — the key/value can move around but `"retryDelay":"<N>s"`
/// is stable.
fn parse_retry_delay_secs(body: &str) -> Option<u64> {
    let key = "\"retryDelay\":\"";
    let start = body.find(key)? + key.len();
    let rest = &body[start..];
    let end = rest.find('"')?;
    let value = &rest[..end];
    let trimmed = value.trim_end_matches('s');
    trimmed.parse::<u64>().ok().map(|n| n.max(5))
}

pub fn catalog() -> Vec<ModelInfo> {
    let mk = |id: &str, name: &str| ModelInfo {
        id: id.into(),
        name: Some(name.into()),
        provider: Some("Google".into()),
        on_demand: true,
        modalities_in: vec!["TEXT".into()],
        modalities_out: vec!["TEXT".into()],
    };
    vec![
        mk("gemini-2.5-flash", "Gemini 2.5 Flash"),
        mk("gemini-2.5-pro", "Gemini 2.5 Pro"),
    ]
}
