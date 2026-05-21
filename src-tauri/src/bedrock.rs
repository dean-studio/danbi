use crate::error::{DanbiError, DanbiResult};
use crate::secrets::ManualCredentials;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_bedrock::Client as ControlClient;
use aws_sdk_bedrockruntime::Client as RuntimeClient;
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, ConverseOutput, InferenceConfiguration, Message,
    SystemContentBlock,
};
use aws_sdk_bedrockruntime::error::SdkError as BedrockSdkError;
use aws_smithy_types::error::display::DisplayErrorContext;
use serde::Serialize;
use std::error::Error as StdError;
use std::fmt::Debug;

/// Unwraps SDK + service errors into a single human-readable string.
/// AWS's default Display is shallow ("service error"), so we walk `source()`
/// chain and join the messages.
fn aws_msg<E, R>(e: &BedrockSdkError<E, R>) -> String
where
    E: StdError + Debug + 'static,
    R: Debug + 'static,
{
    format!("{}", DisplayErrorContext(e))
}

fn aws_msg_ctrl<E, R>(e: &aws_sdk_bedrock::error::SdkError<E, R>) -> String
where
    E: StdError + Debug + 'static,
    R: Debug + 'static,
{
    format!("{}", DisplayErrorContext(e))
}

/// Auth options resolvable from UI.
#[derive(Debug)]
pub enum AuthMode<'a> {
    Profile(&'a str),
    Manual(&'a ManualCredentials),
    Env, // rely on AWS_ACCESS_KEY_ID etc. in process env
}

/// Public alias used by `providers::bedrock` for its Titan embedding
/// wrapper. Kept as a thin re-export so the crate-internal `sdk_config`
/// stays private everywhere else.
pub async fn sdk_config_for_embed(
    auth: AuthMode<'_>,
    region: &str,
) -> DanbiResult<aws_config::SdkConfig> {
    sdk_config(auth, region).await
}

async fn sdk_config(auth: AuthMode<'_>, region: &str) -> DanbiResult<aws_config::SdkConfig> {
    let region = Region::new(region.to_string());
    let loader = aws_config::defaults(BehaviorVersion::latest()).region(region);

    let sdk = match auth {
        AuthMode::Profile(name) => loader.profile_name(name).load().await,
        AuthMode::Manual(m) => {
            let creds = Credentials::new(
                m.access_key_id.clone(),
                m.secret_access_key.clone(),
                m.session_token.clone(),
                None,
                "danbi-manual",
            );
            loader.credentials_provider(creds).load().await
        }
        AuthMode::Env => loader.load().await,
    };
    Ok(sdk)
}

#[derive(Debug, Serialize, Clone)]
pub struct BedrockModel {
    pub id: String,
    pub name: Option<String>,
    pub provider: Option<String>,
    /// "ON_DEMAND" available?
    pub on_demand: bool,
    pub modalities_in: Vec<String>,
    pub modalities_out: Vec<String>,
}

pub async fn list_foundation_models(auth: AuthMode<'_>, region: &str) -> DanbiResult<Vec<BedrockModel>> {
    let sdk = sdk_config(auth, region).await?;
    let client = ControlClient::new(&sdk);

    let mut out: Vec<BedrockModel> = Vec::new();

    // 1) Base foundation models
    let resp = client
        .list_foundation_models()
        .send()
        .await
        .map_err(|e| DanbiError::Aws(format!("list_foundation_models: {}", aws_msg_ctrl(&e))))?;

    for m in resp.model_summaries() {
        let on_demand = m
            .inference_types_supported()
            .iter()
            .any(|t| t.as_str() == "ON_DEMAND");
        out.push(BedrockModel {
            id: m.model_id().to_string(),
            name: m.model_name().map(|s| s.to_string()),
            provider: m.provider_name().map(|s| s.to_string()),
            on_demand,
            modalities_in: m
                .input_modalities()
                .iter()
                .map(|x| x.as_str().to_string())
                .collect(),
            modalities_out: m
                .output_modalities()
                .iter()
                .map(|x| x.as_str().to_string())
                .collect(),
        });
    }

    // 2) Cross-region inference profiles — these are the `us.` / `eu.` etc
    //    prefixed IDs you actually call with Converse. Merge them in;
    //    Converse rejects base IDs for current-gen Claude models.
    match client
        .list_inference_profiles()
        .type_equals(aws_sdk_bedrock::types::InferenceProfileType::SystemDefined)
        .send()
        .await
    {
        Ok(prof) => {
            for ip in prof.inference_profile_summaries() {
                let id = ip.inference_profile_id().to_string();
                let name = Some(ip.inference_profile_name().to_string());
                let provider = infer_provider_from_id(&id);
                out.push(BedrockModel {
                    id,
                    name,
                    provider,
                    on_demand: true, // inference profiles are always invokable
                    modalities_in: vec!["TEXT".into()],
                    modalities_out: vec!["TEXT".into()],
                });
            }
        }
        Err(e) => {
            // Don't fail the whole listing; profiles may be unavailable in some
            // regions or missing IAM permission. We surface a readable note via
            // a synthetic sentinel entry so the UI can still explain the gap.
            out.push(BedrockModel {
                id: format!("__error__list_inference_profiles"),
                name: Some(format!("inference profile listing failed: {}", aws_msg_ctrl(&e))),
                provider: None,
                on_demand: false,
                modalities_in: vec![],
                modalities_out: vec![],
            });
        }
    }

    Ok(out)
}

fn infer_provider_from_id(id: &str) -> Option<String> {
    // e.g. "us.anthropic.claude-haiku-4-5-20251001-v1:0" -> "Anthropic"
    let stripped = id
        .split_once('.')
        .map(|(_, rest)| rest)
        .unwrap_or(id);
    let provider = stripped.split('.').next()?;
    Some(match provider {
        "anthropic" => "Anthropic".into(),
        "amazon" => "Amazon".into(),
        "meta" => "Meta".into(),
        "mistral" => "Mistral".into(),
        "ai21" => "AI21".into(),
        "cohere" => "Cohere".into(),
        "stability" => "Stability".into(),
        other => other.to_string(),
    })
}

#[derive(Debug, Serialize, Clone)]
pub struct BedrockTestResult {
    pub ok: bool,
    pub detail: String,
}

/// Performs a minimal Bedrock runtime call using an inexpensive Anthropic Haiku model
/// via the Messages API (Converse). Falls back to a dry check when no model_id is provided.
pub async fn test_connection(
    auth: AuthMode<'_>,
    region: &str,
    model_id: Option<&str>,
) -> DanbiResult<BedrockTestResult> {
    let sdk = sdk_config(auth, region).await?;

    // First: can we list models? That alone proves credentials+region are valid.
    let ctrl = ControlClient::new(&sdk);
    if let Err(e) = ctrl.list_foundation_models().send().await {
        return Ok(BedrockTestResult {
            ok: false,
            detail: format!("list_foundation_models failed: {}", aws_msg_ctrl(&e)),
        });
    }

    // Second (optional): tiny runtime call to confirm invoke permissions.
    if let Some(mid) = model_id {
        let rt = RuntimeClient::new(&sdk);
        let msg = Message::builder()
            .role(ConversationRole::User)
            .content(ContentBlock::Text("ping".into()))
            .build()
            .map_err(|e| DanbiError::Aws(format!("build message: {e}")))?;

        let inf = InferenceConfiguration::builder().max_tokens(8).build();

        match rt
            .converse()
            .model_id(mid)
            .messages(msg)
            .inference_config(inf)
            .send()
            .await
        {
            Ok(_) => Ok(BedrockTestResult {
                ok: true,
                detail: format!("connected; runtime invoke on {mid} ok"),
            }),
            Err(e) => Ok(BedrockTestResult {
                ok: false,
                detail: format!("runtime invoke failed on {mid}: {}", aws_msg(&e)),
            }),
        }
    } else {
        Ok(BedrockTestResult {
            ok: true,
            detail: "credentials + region valid (control plane only)".into(),
        })
    }
}

/// Minimal Converse call: single user turn with optional system prompt.
/// Returns the aggregated assistant text.
pub async fn converse_text(
    auth: AuthMode<'_>,
    region: &str,
    model_id: &str,
    system: Option<&str>,
    user_text: &str,
    max_tokens: i32,
    temperature: f32,
) -> DanbiResult<String> {
    let sdk = sdk_config(auth, region).await?;
    let rt = RuntimeClient::new(&sdk);

    let user_msg = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Text(user_text.to_string()))
        .build()
        .map_err(|e| DanbiError::Aws(format!("build message: {e}")))?;

    let inference = InferenceConfiguration::builder()
        .max_tokens(max_tokens)
        .temperature(temperature)
        .build();

    let mut builder = rt
        .converse()
        .model_id(model_id)
        .messages(user_msg)
        .inference_config(inference);

    if let Some(sys) = system {
        builder = builder.system(SystemContentBlock::Text(sys.to_string()));
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| DanbiError::Aws(format!("converse: {}", aws_msg(&e))))?;

    // Record usage before consuming `resp.output` — the SDK exposes token
    // counts on `resp.usage`. 0 tokens on parse failure is preferable to
    // losing the call entirely, so we swallow the option instead of
    // erroring out.
    if let Some(u) = resp.usage.as_ref() {
        let input_tokens = u.input_tokens.max(0) as u32;
        let output_tokens = u.output_tokens.max(0) as u32;
        crate::usage::record("bedrock", model_id, input_tokens, output_tokens);
    }

    let text = match resp.output {
        Some(ConverseOutput::Message(m)) => m
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    Ok(text)
}
