//! AWS Bedrock provider — wraps the existing `crate::bedrock` free functions
//! behind the `Provider` trait.
//!
//! The free-function layer in `crate::bedrock` is still used directly by the
//! Bedrock-specific Tauri commands (`list_bedrock_models`, `test_bedrock`) that
//! the onboarding wizard talks to, because those commands accept raw
//! `AuthInput` / region rather than a fully-formed `DanbiConfig`. Everything
//! else — routing, preview, compound, qa, ghost-links, briefing — goes through
//! `Provider::converse_text`.

use crate::bedrock::{self, AuthMode};
use crate::error::DanbiResult;
use crate::providers::{ModelInfo, Provider, TestResult};
use crate::secrets::ManualCredentials;
use async_trait::async_trait;

/// Owned auth handle — holds onto any credentials fetched from Keychain so
/// they outlive each `converse_text` call without us having to re-fetch.
#[derive(Debug)]
pub enum OwnedAuth {
    Profile(String),
    Manual(ManualCredentials),
    Env,
}

impl OwnedAuth {
    fn as_mode(&self) -> AuthMode<'_> {
        match self {
            OwnedAuth::Profile(name) => AuthMode::Profile(name),
            OwnedAuth::Manual(creds) => AuthMode::Manual(creds),
            OwnedAuth::Env => AuthMode::Env,
        }
    }
}

pub struct BedrockProvider {
    pub auth: OwnedAuth,
    pub region: String,
}

#[async_trait]
impl Provider for BedrockProvider {
    fn kind(&self) -> &'static str {
        "bedrock"
    }

    async fn converse_text(
        &self,
        model_id: &str,
        system: Option<&str>,
        user_text: &str,
        max_tokens: i32,
        temperature: f32,
    ) -> DanbiResult<String> {
        bedrock::converse_text(
            self.auth.as_mode(),
            &self.region,
            model_id,
            system,
            user_text,
            max_tokens,
            temperature,
        )
        .await
    }

    async fn list_models(&self) -> DanbiResult<Vec<ModelInfo>> {
        let models = bedrock::list_foundation_models(self.auth.as_mode(), &self.region).await?;
        Ok(models
            .into_iter()
            .map(|m| ModelInfo {
                id: m.id,
                name: m.name,
                provider: m.provider,
                on_demand: m.on_demand,
                modalities_in: m.modalities_in,
                modalities_out: m.modalities_out,
            })
            .collect())
    }

    async fn test_connection(&self, model_id: Option<&str>) -> DanbiResult<TestResult> {
        let r = bedrock::test_connection(self.auth.as_mode(), &self.region, model_id).await?;
        Ok(TestResult {
            ok: r.ok,
            detail: r.detail,
        })
    }

    async fn embed(
        &self,
        model_id: &str,
        inputs: &[String],
    ) -> DanbiResult<Vec<Vec<f32>>> {
        bedrock_embed(self.auth.as_mode(), &self.region, model_id, inputs).await
    }

    fn default_embed_model(&self) -> &'static str {
        "amazon.titan-embed-text-v2:0"
    }
}

/// Bedrock InvokeModel wrapper for Titan Text Embeddings. We send one
/// request per input since Titan's embedding API doesn't batch.
async fn bedrock_embed(
    auth: AuthMode<'_>,
    region: &str,
    model_id: &str,
    inputs: &[String],
) -> DanbiResult<Vec<Vec<f32>>> {
    use aws_sdk_bedrockruntime::Client as RuntimeClient;
    use aws_smithy_types::Blob;

    let sdk = bedrock::sdk_config_for_embed(auth, region).await?;
    let client = RuntimeClient::new(&sdk);

    let mut out: Vec<Vec<f32>> = Vec::with_capacity(inputs.len());
    for text in inputs {
        let body = serde_json::json!({
            "inputText": text,
            "normalize": true
        })
        .to_string();
        let resp = client
            .invoke_model()
            .model_id(model_id)
            .content_type("application/json")
            .accept("application/json")
            .body(Blob::new(body.into_bytes()))
            .send()
            .await
            .map_err(|e| {
                crate::error::DanbiError::Aws(format!("titan embed: {e}"))
            })?;
        let bytes = resp.body.into_inner();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| crate::error::DanbiError::Other(format!("titan parse: {e}")))?;
        // Titan returns `inputTextTokenCount` on every embed response; record
        // it so the dashboard can price embedding runs too. Missing/0 falls
        // back silently rather than failing the embed.
        let input_tokens = parsed
            .get("inputTextTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if input_tokens > 0 {
            crate::usage::record("bedrock", model_id, input_tokens, 0);
        }
        let vec = parsed
            .get("embedding")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|n| n.as_f64().map(|f| f as f32))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.push(vec);
    }
    Ok(out)
}
