//! Provider abstraction — unifies Bedrock and future providers (NVIDIA NIM,
//! OpenAI, etc.) behind a single async trait so the rest of the codebase can
//! stay provider-agnostic.
//!
//! A `Provider` value is constructed from the active `DanbiConfig` + Keychain
//! secrets right before each LLM call. It is intentionally short-lived so that
//! provider switches (and credential rotations) are picked up without any
//! process restart.

use crate::error::DanbiResult;
use async_trait::async_trait;
use serde::Serialize;

pub mod anthropic;
pub mod bedrock;
pub mod google;
pub mod nvidia;
pub mod ollama;
pub mod openai;
pub mod voyage;

/// A concrete model discovered via `Provider::list_models()`.
///
/// The shape mirrors the existing Bedrock model listing so UI code that was
/// originally written against Bedrock can keep working; non-Bedrock providers
/// simply fill in synthetic values for the AWS-specific fields.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: Option<String>,
    pub provider: Option<String>,
    pub on_demand: bool,
    pub modalities_in: Vec<String>,
    pub modalities_out: Vec<String>,
}

/// Outcome of a `Provider::test_connection` probe.
#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub detail: String,
}

/// Unified LLM interface used by routing, preview, compound, briefing, etc.
///
/// Every method is async; implementations are expected to do their own network
/// I/O and credential resolution internally.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable identifier ("bedrock" | "nvidia") for logging/debug.
    fn kind(&self) -> &'static str;

    /// Single-turn completion with optional system prompt. Returns the
    /// aggregated assistant text (not streamed). `max_tokens` is a hard cap;
    /// `temperature` is clamped by the implementation if the provider has a
    /// narrower valid range.
    async fn converse_text(
        &self,
        model_id: &str,
        system: Option<&str>,
        user_text: &str,
        max_tokens: i32,
        temperature: f32,
    ) -> DanbiResult<String>;

    /// Lists the models this provider offers. For providers without a
    /// discovery API (NVIDIA NIM today) this returns a hand-curated catalog.
    async fn list_models(&self) -> DanbiResult<Vec<ModelInfo>>;

    /// Cheap probe that verifies the provider is reachable with the current
    /// credentials. May optionally invoke a specific model for a tighter
    /// check; passing `None` keeps the probe control-plane-only.
    async fn test_connection(&self, model_id: Option<&str>) -> DanbiResult<TestResult>;

    /// Produces a dense embedding vector for each input text. Providers
    /// that don't expose an embedding endpoint return an error.
    /// Callers supply the model id (e.g. "amazon.titan-embed-text-v2:0",
    /// "text-embedding-3-small") so the same provider can serve multiple
    /// embedding models.
    async fn embed(
        &self,
        _model_id: &str,
        _inputs: &[String],
    ) -> DanbiResult<Vec<Vec<f32>>> {
        Err(crate::error::DanbiError::Other(
            "이 provider는 embedding API 를 제공하지 않아요".into(),
        ))
    }

    /// Default embedding model id to suggest in UI. Empty string means
    /// "this provider doesn't support embeddings; disable vector search".
    fn default_embed_model(&self) -> &'static str {
        ""
    }
}
