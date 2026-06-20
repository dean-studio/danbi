use crate::aws_profiles::{self, AwsDetection};
use crate::bedrock::{self, AuthMode, BedrockModel, BedrockTestResult};
use crate::config::{self, DanbiConfig, ProviderConfig, default_vault_path};
use crate::providers::{
    anthropic::AnthropicProvider,
    bedrock::{BedrockProvider, OwnedAuth},
    google::GoogleProvider,
    nvidia::NvidiaProvider,
    ollama::{OllamaProvider, DEFAULT_BASE_URL as OLLAMA_DEFAULT_URL},
    openai::OpenaiProvider,
    Provider,
};
use crate::edit_ops::{self, EditOp};
use crate::error::{DanbiError, DanbiResult};
use crate::briefing::{self, BriefingResult};
use crate::ghost_links::{self, GhostStore};
use crate::ingest::{self, Extracted};
use crate::project_qa::{self, QaAnswer};
use crate::journal;
use crate::preview::{self, Attachment, PlanInput, PlanPreview};
use crate::routing::{self, RoutingContext, RoutingResult};
use crate::secrets::{self, ManualCredentials};
use crate::vault::{self, VaultTree};
use crate::vcs;
use crate::watcher;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

#[tauri::command]
pub fn ping() -> &'static str {
    "danbi: pong"
}

/// Returns the default vault path suggestion.
#[tauri::command]
pub fn default_vault() -> DanbiResult<String> {
    Ok(default_vault_path()?.to_string_lossy().to_string())
}

/// Returns an existing config if a vault is already set up, otherwise None.
/// Looks at the provided vault path; if absent, tries the default one.
#[tauri::command]
pub fn load_config(vault_path: Option<String>) -> DanbiResult<Option<DanbiConfig>> {
    let vault = match vault_path {
        Some(s) => PathBuf::from(s),
        None => default_vault_path()?,
    };
    config::load_config(&vault)
}

/// Saves config.json into the vault directory (creates dir if needed).
#[tauri::command]
pub fn save_config(vault_path: String, cfg: DanbiConfig) -> DanbiResult<()> {
    let vault = PathBuf::from(&vault_path);
    config::save_config(&vault, &cfg)
}

#[tauri::command]
pub fn detect_aws() -> DanbiResult<AwsDetection> {
    aws_profiles::detect_profiles()
}

/// Stores manual AWS credentials into macOS Keychain.
/// `label` is used as the Keychain item key (e.g. "manual-default").
#[tauri::command]
pub fn store_manual_credentials(
    label: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
) -> DanbiResult<()> {
    let creds = ManualCredentials {
        access_key_id,
        secret_access_key,
        session_token,
    };
    secrets::set_manual_credentials(&label, &creds)
}

#[tauri::command]
pub fn delete_manual_credentials(label: String) -> DanbiResult<()> {
    secrets::delete_manual_credentials(&label)
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AuthInput {
    Profile { name: String },
    Manual { label: String },
    Env,
}

fn resolve_auth<'a>(
    auth: &'a AuthInput,
    loaded: &'a mut Option<ManualCredentials>,
) -> DanbiResult<AuthMode<'a>> {
    match auth {
        AuthInput::Profile { name } => Ok(AuthMode::Profile(name)),
        AuthInput::Manual { label } => {
            let fetched = secrets::get_manual_credentials(label)?
                .ok_or_else(|| DanbiError::Config(format!("manual creds not found: {label}")))?;
            *loaded = Some(fetched);
            Ok(AuthMode::Manual(loaded.as_ref().unwrap()))
        }
        AuthInput::Env => Ok(AuthMode::Env),
    }
}

#[tauri::command]
pub async fn list_bedrock_models(
    auth: AuthInput,
    region: String,
) -> DanbiResult<Vec<BedrockModel>> {
    let mut loaded: Option<ManualCredentials> = None;
    let mode = resolve_auth(&auth, &mut loaded)?;
    bedrock::list_foundation_models(mode, &region).await
}

#[tauri::command]
pub async fn test_bedrock(
    auth: AuthInput,
    region: String,
    model_id: Option<String>,
) -> DanbiResult<BedrockTestResult> {
    let mut loaded: Option<ManualCredentials> = None;
    let mode = resolve_auth(&auth, &mut loaded)?;
    bedrock::test_connection(mode, &region, model_id.as_deref()).await
}

// ---------- NVIDIA ----------
//
// The API key lives in macOS Keychain; `config.json` only holds the
// `api_key_ref` pointer. The onboarding wizard calls `store_nvidia_api_key`
// before writing the config, and `test_nvidia` to verify the key before
// committing.

const NVIDIA_KEY_REF: &str = "keychain:danbi-nvidia";

#[tauri::command]
pub fn store_nvidia_api_key(api_key: String) -> DanbiResult<String> {
    secrets::set_api_key(NVIDIA_KEY_REF, &api_key)?;
    Ok(NVIDIA_KEY_REF.to_string())
}

#[tauri::command]
pub fn delete_nvidia_api_key() -> DanbiResult<()> {
    secrets::delete_api_key(NVIDIA_KEY_REF)
}

#[tauri::command]
pub fn list_nvidia_models() -> DanbiResult<Vec<crate::providers::ModelInfo>> {
    Ok(crate::providers::nvidia::catalog())
}

#[tauri::command]
pub async fn test_nvidia(
    api_key: Option<String>,
    model_id: Option<String>,
) -> DanbiResult<crate::providers::TestResult> {
    // Prefer the explicit key passed in by the wizard (user just typed it in
    // the input); fall back to whatever is already stored in Keychain so the
    // Settings panel's "Test Connection" button works without a re-entry.
    let key = match api_key {
        Some(k) if !k.is_empty() => k,
        _ => secrets::get_api_key(NVIDIA_KEY_REF)?.ok_or_else(|| {
            DanbiError::Config("nvidia api key not set".into())
        })?,
    };
    let provider = crate::providers::nvidia::NvidiaProvider { api_key: key };
    use crate::providers::Provider;
    provider.test_connection(model_id.as_deref()).await
}

// ---------- OpenAI / Anthropic / Google / Ollama ----------
//
// Shared key-reference constants. Using stable keychain labels means the
// Settings panel can re-use a saved key without asking again.

const OPENAI_KEY_REF: &str = "keychain:danbi-openai";
const ANTHROPIC_KEY_REF: &str = "keychain:danbi-anthropic";
const GOOGLE_KEY_REF: &str = "keychain:danbi-google";
const VOYAGE_KEY_REF: &str = "keychain:danbi-voyage";

#[tauri::command]
pub fn store_openai_api_key(api_key: String) -> DanbiResult<String> {
    secrets::set_api_key(OPENAI_KEY_REF, &api_key)?;
    Ok(OPENAI_KEY_REF.to_string())
}
#[tauri::command]
pub fn delete_openai_api_key() -> DanbiResult<()> {
    secrets::delete_api_key(OPENAI_KEY_REF)
}
#[tauri::command]
pub fn list_openai_models() -> DanbiResult<Vec<crate::providers::ModelInfo>> {
    Ok(crate::providers::openai::catalog())
}
#[tauri::command]
pub async fn test_openai(
    api_key: Option<String>,
    base_url: Option<String>,
    model_id: Option<String>,
) -> DanbiResult<crate::providers::TestResult> {
    let key = match api_key {
        Some(k) if !k.is_empty() => k,
        _ => secrets::get_api_key(OPENAI_KEY_REF)?
            .ok_or_else(|| DanbiError::Config("openai api key not set".into()))?,
    };
    let provider = crate::providers::openai::OpenaiProvider {
        api_key: key,
        base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".into()),
    };
    use crate::providers::Provider;
    provider.test_connection(model_id.as_deref()).await
}

#[tauri::command]
pub fn store_anthropic_api_key(api_key: String) -> DanbiResult<String> {
    secrets::set_api_key(ANTHROPIC_KEY_REF, &api_key)?;
    Ok(ANTHROPIC_KEY_REF.to_string())
}
#[tauri::command]
pub fn delete_anthropic_api_key() -> DanbiResult<()> {
    secrets::delete_api_key(ANTHROPIC_KEY_REF)
}
#[tauri::command]
pub fn list_anthropic_models() -> DanbiResult<Vec<crate::providers::ModelInfo>> {
    Ok(crate::providers::anthropic::catalog())
}
#[tauri::command]
pub async fn test_anthropic(
    api_key: Option<String>,
    model_id: Option<String>,
) -> DanbiResult<crate::providers::TestResult> {
    let key = match api_key {
        Some(k) if !k.is_empty() => k,
        _ => secrets::get_api_key(ANTHROPIC_KEY_REF)?.ok_or_else(|| {
            DanbiError::Config("anthropic api key not set".into())
        })?,
    };
    let provider = crate::providers::anthropic::AnthropicProvider { api_key: key };
    use crate::providers::Provider;
    provider.test_connection(model_id.as_deref()).await
}

#[tauri::command]
pub fn store_google_api_key(api_key: String) -> DanbiResult<String> {
    secrets::set_api_key(GOOGLE_KEY_REF, &api_key)?;
    Ok(GOOGLE_KEY_REF.to_string())
}
#[tauri::command]
pub fn delete_google_api_key() -> DanbiResult<()> {
    secrets::delete_api_key(GOOGLE_KEY_REF)
}
#[tauri::command]
pub fn list_google_models() -> DanbiResult<Vec<crate::providers::ModelInfo>> {
    Ok(crate::providers::google::catalog())
}
#[tauri::command]
pub async fn test_google(
    api_key: Option<String>,
    model_id: Option<String>,
) -> DanbiResult<crate::providers::TestResult> {
    let key = match api_key {
        Some(k) if !k.is_empty() => k,
        _ => secrets::get_api_key(GOOGLE_KEY_REF)?.ok_or_else(|| {
            DanbiError::Config("google api key not set".into())
        })?,
    };
    let provider = crate::providers::google::GoogleProvider { api_key: key };
    use crate::providers::Provider;
    provider.test_connection(model_id.as_deref()).await
}

// ---------- Voyage AI (embeddings only) ----------

#[tauri::command]
pub fn store_voyage_api_key(api_key: String) -> DanbiResult<String> {
    secrets::set_api_key(VOYAGE_KEY_REF, &api_key)?;
    Ok(VOYAGE_KEY_REF.to_string())
}

#[tauri::command]
pub fn delete_voyage_api_key() -> DanbiResult<()> {
    secrets::delete_api_key(VOYAGE_KEY_REF)
}

#[tauri::command]
pub fn list_voyage_models() -> DanbiResult<Vec<crate::providers::ModelInfo>> {
    Ok(crate::providers::voyage::catalog())
}

#[tauri::command]
pub async fn test_voyage(
    api_key: Option<String>,
    model_id: Option<String>,
) -> DanbiResult<crate::providers::TestResult> {
    let key = match api_key {
        Some(k) if !k.is_empty() => k,
        _ => secrets::get_api_key(VOYAGE_KEY_REF)?.ok_or_else(|| {
            DanbiError::Config("voyage api key not set".into())
        })?,
    };
    let provider = crate::providers::voyage::VoyageProvider { api_key: key };
    use crate::providers::Provider;
    provider.test_connection(model_id.as_deref()).await
}

#[tauri::command]
pub async fn list_ollama_models(
    base_url: Option<String>,
) -> DanbiResult<Vec<crate::providers::ModelInfo>> {
    let url = base_url
        .unwrap_or_else(|| crate::providers::ollama::DEFAULT_BASE_URL.into());
    let provider = crate::providers::ollama::OllamaProvider { base_url: url };
    use crate::providers::Provider;
    provider.list_models().await
}
#[tauri::command]
pub async fn test_ollama(
    base_url: Option<String>,
    model_id: Option<String>,
) -> DanbiResult<crate::providers::TestResult> {
    let url = base_url
        .unwrap_or_else(|| crate::providers::ollama::DEFAULT_BASE_URL.into());
    let provider = crate::providers::ollama::OllamaProvider { base_url: url };
    use crate::providers::Provider;
    provider.test_connection(model_id.as_deref()).await
}

// ---------- Vault / Watcher ----------

#[tauri::command]
pub fn init_vault(vault_path: String) -> DanbiResult<()> {
    vault::init_vault(&PathBuf::from(vault_path))
}

/// async 로 두면 Tauri 가 worker thread 에서 돌려 main thread (UI) 가 블록되지
/// 않는다. 큰 vault 의 list_tree 가 100ms 넘으면 sync command 일 때 그 시간
/// 동안 webview 가 OS 입장 "busy" 로 보여 마우스 spinner 가 뜬다.
#[tauri::command]
pub async fn list_tree(vault_path: String) -> DanbiResult<VaultTree> {
    tokio::task::spawn_blocking(move || vault::list_tree(&PathBuf::from(vault_path)))
        .await
        .map_err(|e| DanbiError::Other(format!("join: {e}")))?
}

#[tauri::command]
pub async fn create_project(
    vault_path: String,
    name: String,
    default_domains: Vec<String>,
    default_folders: Option<Vec<String>>,
) -> DanbiResult<()> {
    tokio::task::spawn_blocking(move || {
        let folders = match default_folders {
            Some(v) => v,
            None => config::load_config(&PathBuf::from(&vault_path))?
                .map(|c| c.default_folders)
                .unwrap_or_default(),
        };
        vault::create_project_with_folders(
            &PathBuf::from(vault_path),
            &name,
            &default_domains,
            &folders,
        )
    })
    .await
    .map_err(|e| DanbiError::Other(format!("join: {e}")))?
}

#[tauri::command]
pub async fn rename_project(
    vault_path: String,
    old: String,
    new: String,
) -> DanbiResult<()> {
    tokio::task::spawn_blocking(move || {
        let vp = PathBuf::from(vault_path);
        vault::rename_project(&vp, &old, &new)?;
        // Group membership stores plain project names — rewrite them so the
        // renamed project doesn't fall out of its group on next sidebar load.
        if let Ok(vault_root) = default_vault_path() {
            if let Ok(Some(mut cfg)) = config::load_config(&vault_root) {
                let mut touched = false;
                for g in cfg.project_groups.iter_mut() {
                    for p in g.projects.iter_mut() {
                        if *p == old {
                            *p = new.clone();
                            touched = true;
                        }
                    }
                }
                if touched {
                    let _ = config::save_config(&vault_root, &cfg);
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| DanbiError::Other(format!("join: {e}")))?
}

/// Soft-delete a project — moves the entire project directory into
/// `.danbi/trash/` (recoverable from the Trash panel for 30 days) and
/// removes the project from any group it was a member of so the sidebar
/// doesn't keep a stale group entry around.
#[tauri::command]
pub async fn delete_project(vault_path: String, name: String) -> DanbiResult<()> {
    tokio::task::spawn_blocking(move || {
        let vp = PathBuf::from(vault_path);
        crate::trash::trash_project(&vp, &name)?;
        if let Ok(vault_root) = default_vault_path() {
            if let Ok(Some(mut cfg)) = config::load_config(&vault_root) {
                let mut touched = false;
                for g in cfg.project_groups.iter_mut() {
                    let before = g.projects.len();
                    g.projects.retain(|p| p != &name);
                    if g.projects.len() != before {
                        touched = true;
                    }
                }
                if touched {
                    let _ = config::save_config(&vault_root, &cfg);
                }
            }
        }
        Ok::<(), DanbiError>(())
    })
    .await
    .map_err(|e| DanbiError::Other(format!("join: {e}")))?
}

#[tauri::command]
pub fn create_domain(
    vault_path: String,
    project: String,
    domain: String,
) -> DanbiResult<String> {
    vault::create_domain(&PathBuf::from(vault_path), &project, &domain)
}

#[tauri::command]
pub fn rename_domain(
    vault_path: String,
    project: String,
    old: String,
    new: String,
) -> DanbiResult<String> {
    vault::rename_domain(&PathBuf::from(vault_path), &project, &old, &new)
}

/// Soft-delete a domain file. The file is moved into `.danbi/trash/`
/// rather than removed from disk so the user can restore it from the
/// trash panel. To wipe permanently, use `trash_purge` after deletion.
#[tauri::command]
pub fn delete_domain(
    vault_path: String,
    project: String,
    domain: String,
) -> DanbiResult<()> {
    let p = PathBuf::from(vault_path);
    crate::trash::trash_file(&p, &project, &domain).map(|_| ())
}

#[tauri::command]
pub fn create_folder(
    vault_path: String,
    project: String,
    folder: String,
) -> DanbiResult<()> {
    vault::create_folder(&PathBuf::from(vault_path), &project, &folder)
}

#[tauri::command]
pub fn rename_folder(
    vault_path: String,
    project: String,
    old: String,
    new: String,
) -> DanbiResult<()> {
    vault::rename_folder(&PathBuf::from(vault_path), &project, &old, &new)
}

/// Soft-delete a sub-folder (and everything inside) into trash.
#[tauri::command]
pub fn delete_folder(
    vault_path: String,
    project: String,
    folder: String,
) -> DanbiResult<()> {
    let p = PathBuf::from(vault_path);
    crate::trash::trash_folder(&p, &project, &folder).map(|_| ())
}

#[tauri::command]
pub fn trash_list() -> DanbiResult<Vec<crate::trash::TrashEntry>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::trash::list(&vault_path)
}

#[tauri::command]
pub fn trash_restore(id: String) -> DanbiResult<crate::trash::TrashEntry> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::trash::restore(&vault_path, &id)
}

#[tauri::command]
pub fn trash_purge(id: String) -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::trash::purge(&vault_path, &id)
}

#[tauri::command]
pub fn trash_empty() -> DanbiResult<usize> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::trash::empty_all(&vault_path)
}

/// Move a domain file between top-level / sub-folder. Pass `to_folder = None`
/// (or null from the frontend) to move into the project root.
#[tauri::command]
pub fn move_domain(
    vault_path: String,
    project: String,
    from: String,
    to_folder: Option<String>,
) -> DanbiResult<String> {
    vault::move_domain(
        &PathBuf::from(vault_path),
        &project,
        &from,
        to_folder.as_deref(),
    )
}

#[tauri::command]
pub fn read_doc(
    vault_path: String,
    project: String,
    domain: String,
) -> DanbiResult<String> {
    vault::read_doc(&PathBuf::from(vault_path), &project, &domain)
}

#[tauri::command]
pub fn write_doc(
    vault_path: String,
    project: String,
    domain: String,
    content: String,
) -> DanbiResult<()> {
    vault::write_doc(&PathBuf::from(vault_path), &project, &domain, &content)
}

#[tauri::command]
pub fn save_asset(
    vault_path: String,
    project: String,
    filename: String,
    bytes_b64: String,
) -> DanbiResult<String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_b64.as_bytes())
        .map_err(|e| DanbiError::Other(format!("base64 decode: {e}")))?;
    vault::save_asset(&PathBuf::from(vault_path), &project, &filename, &bytes)
}

#[tauri::command]
pub fn resolve_asset(
    vault_path: String,
    project: String,
    rel_path: String,
) -> DanbiResult<String> {
    let abs = vault::resolve_asset_absolute(&PathBuf::from(vault_path), &project, &rel_path)?;
    Ok(abs.to_string_lossy().to_string())
}

#[tauri::command]
pub fn start_watching(app: AppHandle, vault_path: String) -> DanbiResult<()> {
    watcher::start(&app, &PathBuf::from(vault_path))
}

#[tauri::command]
pub fn stop_watching(app: AppHandle) -> DanbiResult<()> {
    watcher::stop(&app)
}

// ---------- Chat / Routing ----------

/// Builds a fresh `Provider` trait object from the active config — used by
/// every LLM-driven command (routing, preview, compound, qa, briefing,
/// ghost-links, search). The object is short-lived and holds any
/// Keychain-resolved credentials for its own lifetime, so callers don't have
/// to juggle `&mut Option<ManualCredentials>` anymore.
fn resolve_provider(cfg: &DanbiConfig) -> DanbiResult<Box<dyn Provider>> {
    let pc = cfg
        .provider
        .as_ref()
        .ok_or_else(|| DanbiError::Config("provider not configured".into()))?;
    match pc {
        ProviderConfig::Bedrock {
            auth_mode,
            profile,
            region,
        } => {
            let auth = match auth_mode.as_str() {
                "profile" => OwnedAuth::Profile(profile.clone().ok_or_else(|| {
                    DanbiError::Config("profile name missing".into())
                })?),
                "manual" => {
                    let creds = secrets::get_manual_credentials("manual-default")?
                        .ok_or_else(|| {
                            DanbiError::Config("manual creds not found in keychain".into())
                        })?;
                    OwnedAuth::Manual(creds)
                }
                "env" => OwnedAuth::Env,
                other => {
                    return Err(DanbiError::Config(format!("unknown auth mode: {other}")))
                }
            };
            Ok(Box::new(BedrockProvider {
                auth,
                region: region.clone(),
            }))
        }
        ProviderConfig::Nvidia { api_key_ref } => {
            let api_key = secrets::get_api_key(api_key_ref)?.ok_or_else(|| {
                DanbiError::Config(format!(
                    "nvidia api key not found in keychain ({api_key_ref})"
                ))
            })?;
            Ok(Box::new(NvidiaProvider { api_key }))
        }
        ProviderConfig::Openai {
            api_key_ref,
            base_url,
        } => {
            let api_key = secrets::get_api_key(api_key_ref)?.ok_or_else(|| {
                DanbiError::Config(format!(
                    "openai api key not found in keychain ({api_key_ref})"
                ))
            })?;
            let base_url = base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            Ok(Box::new(OpenaiProvider { api_key, base_url }))
        }
        ProviderConfig::Anthropic { api_key_ref } => {
            let api_key = secrets::get_api_key(api_key_ref)?.ok_or_else(|| {
                DanbiError::Config(format!(
                    "anthropic api key not found in keychain ({api_key_ref})"
                ))
            })?;
            Ok(Box::new(AnthropicProvider { api_key }))
        }
        ProviderConfig::Google { api_key_ref } => {
            let api_key = secrets::get_api_key(api_key_ref)?.ok_or_else(|| {
                DanbiError::Config(format!(
                    "google api key not found in keychain ({api_key_ref})"
                ))
            })?;
            Ok(Box::new(GoogleProvider { api_key }))
        }
        ProviderConfig::Ollama { base_url } => {
            let base_url = base_url
                .clone()
                .unwrap_or_else(|| OLLAMA_DEFAULT_URL.into());
            Ok(Box::new(OllamaProvider { base_url }))
        }
        ProviderConfig::Voyage { api_key_ref } => {
            let api_key = secrets::get_api_key(api_key_ref)?.ok_or_else(|| {
                DanbiError::Config(format!(
                    "voyage api key not found in keychain ({api_key_ref})"
                ))
            })?;
            Ok(Box::new(crate::providers::voyage::VoyageProvider {
                api_key,
            }))
        }
    }
}

/// Build the provider to call for embedding requests. If `embed_provider`
/// is configured in the vault config, use that — this lets users pair a
/// paid LLM provider with a free local Ollama embedding backend (the
/// canonical "don't burn tokens on vector indexing" setup). Falls back
/// to the main LLM provider to preserve old single-provider behaviour.
pub fn resolve_embed_provider(cfg: &DanbiConfig) -> DanbiResult<Box<dyn Provider>> {
    if cfg.embed_provider.is_some() {
        // Synthesize a mini-config whose `provider` is the embed one,
        // then re-use the same resolver for keyring lookup, auth mode
        // handling, etc.
        let mut synth = cfg.clone();
        synth.provider = cfg.embed_provider.clone();
        return resolve_provider(&synth);
    }
    resolve_provider(cfg)
}

/// The model id to pass to `embed()`. Uses the explicit config override
/// first, then the embed provider's default.
pub fn resolve_embed_model(
    cfg: &DanbiConfig,
    provider: &dyn Provider,
    override_id: Option<String>,
) -> String {
    if let Some(m) = override_id.filter(|s| !s.trim().is_empty()) {
        return m;
    }
    if let Some(m) = cfg
        .embed_model
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return m.clone();
    }
    provider.default_embed_model().to_string()
}

/// Embed a single query string using whatever embed provider the user has
/// configured. Returns None when no provider is configured, the model id
/// resolves to empty, or the embed call fails (rate-limit, network, etc.) —
/// callers can then fall back to BM25-only search transparently.
pub async fn embed_query_for_search(cfg: &DanbiConfig, query: &str) -> Option<Vec<f32>> {
    let provider = resolve_embed_provider(cfg).ok()?;
    let model = resolve_embed_model(cfg, provider.as_ref(), None);
    if model.is_empty() {
        return None;
    }
    let res = crate::usage::with_role(
        "embed",
        provider.embed(&model, &[query.to_string()]),
    )
    .await
    .ok()?;
    res.into_iter().next()
}

#[tauri::command]
pub async fn route_message(
    message: String,
    ctx: RoutingContext,
    attachments: Option<Vec<Attachment>>,
) -> DanbiResult<RoutingResult> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let model = cfg
        .models
        .routing
        .clone()
        .ok_or_else(|| DanbiError::Config("routing model missing".into()))?;

    let provider = resolve_provider(&cfg)?;

    // Give the router short hints about attached files without paying for
    // their full content — filenames are usually enough signal for intent
    // classification ("여기 PDF 내용 요약해서 plan.md에 넣어줘").
    let enriched = match attachments.as_ref() {
        Some(atts) if !atts.is_empty() => {
            let hints = atts
                .iter()
                .map(|a| format!("- {} ({}, {} chars)", a.filename, a.kind, a.text.len()))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{message}\n\n[첨부됨]\n{hints}")
        }
        _ => message,
    };

    let route = routing::route(provider.as_ref(), &model, &enriched, &ctx).await?;

    // Auto-enqueue a review item when the router reports low confidence —
    // the user will see it in the Review inbox and can decide later what
    // to do (pick a project manually, rephrase, dismiss).
    if route.confidence < 0.5 || route.needs_clarification {
        let vault_path = require_vault(&cfg).ok();
        if let Some(vp) = vault_path {
            let _ = crate::reviews::enqueue(
                &vp,
                "low_confidence_plan",
                route.project.clone(),
                route.domain.clone(),
                format!(
                    "라우팅 신뢰도 {:.2} · {}",
                    route.confidence,
                    route.summary.clone()
                ),
            );
        }
    }

    Ok(route)
}

#[tauri::command]
pub fn extract_file_path(path: String) -> DanbiResult<Extracted> {
    ingest::extract_from_path(&PathBuf::from(path))
}

#[tauri::command]
pub fn extract_file_bytes(filename: String, bytes_b64: String) -> DanbiResult<Extracted> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_b64.as_bytes())
        .map_err(|e| DanbiError::Other(format!("base64 decode: {e}")))?;
    ingest::extract_from_bytes(&filename, &bytes)
}

#[tauri::command]
pub async fn preview_plan(
    message: String,
    project: String,
    domain: String,
    intent: String,
    attachments: Option<Vec<Attachment>>,
) -> DanbiResult<PlanPreview> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let model = cfg
        .models
        .writer
        .clone()
        .ok_or_else(|| DanbiError::Config("writer model missing".into()))?;
    let vault_path = cfg
        .vault_path
        .clone()
        .ok_or_else(|| DanbiError::Config("vault path missing".into()))?;

    let doc = vault::read_doc(&PathBuf::from(&vault_path), &project, &domain)?;
    let provider = resolve_provider(&cfg)?;
    let atts = attachments.unwrap_or_default();

    // Wiki grounding: pull related passages from the same project so the
    // Writer can reference accumulated knowledge and insert [[link]]s
    // instead of re-deriving everything from scratch.
    let grounding_embedding = embed_query_for_search(&cfg, &message).await;
    let grounding = crate::grounding::gather_grounding(
        &PathBuf::from(&vault_path),
        Some(&project),
        &message,
        grounding_embedding.as_deref(),
        Some(&domain),
        4,
        800,
    )
    .unwrap_or_default();

    let project_ctx =
        crate::project_context::load(&PathBuf::from(&vault_path), &project);

    preview::build_plan(
        provider.as_ref(),
        &model,
        PlanInput {
            intent: &intent,
            project: &project,
            domain: &domain,
            user_message: &message,
            doc_content: &doc,
            attachments: &atts,
            grounding: &grounding,
            project_ctx: &project_ctx,
        },
    )
    .await
}

// ---------- Apply / Undo ----------

#[derive(Debug, Serialize, Clone)]
pub struct ApplyResult {
    pub project: String,
    pub domain: String,
    pub commit_before: Option<String>,
    pub commit_after: Option<String>,
    pub bytes_before: usize,
    pub bytes_after: usize,
}

#[derive(Debug, Serialize)]
struct HistoryEvent<'a> {
    ts: String,
    kind: &'a str, // "apply" | "undo"
    project: Option<&'a str>,
    domain: Option<&'a str>,
    intent: Option<&'a str>,
    user_message: Option<&'a str>,
    summary: Option<&'a str>,
    op: Option<&'a EditOp>,
    commit_before: Option<&'a str>,
    commit_after: Option<&'a str>,
}

fn require_vault(cfg: &DanbiConfig) -> DanbiResult<PathBuf> {
    let s = cfg
        .vault_path
        .clone()
        .ok_or_else(|| DanbiError::Config("vault path missing".into()))?;
    Ok(PathBuf::from(s))
}

#[tauri::command]
pub fn apply_plan(
    project: String,
    domain: String,
    intent: String,
    user_message: String,
    summary: String,
    op: EditOp,
) -> DanbiResult<ApplyResult> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;

    vcs::ensure_repo(&vault_path)?;
    edit_ops::validate(&op)?;

    let before = vault::read_doc(&vault_path, &project, &domain)?;
    let after = edit_ops::apply(&before, &op)?;

    // Snapshot the current state FIRST so undo restores exactly what the user saw.
    let commit_before = vcs::snapshot(
        &vault_path,
        &format!("danbi: pre-edit snapshot · {}/{}", project, domain),
    )?;

    vault::write_doc(&vault_path, &project, &domain, &after)?;

    let op_label = match &op {
        EditOp::Append { .. } => "append",
        EditOp::InsertAfter { .. } => "insert_after",
        EditOp::ReplaceSection { .. } => "replace_section",
        EditOp::RewriteAll { .. } => "rewrite_all",
        EditOp::UpsertItem { .. } => "upsert_item",
    };

    let commit_msg = format!(
        "danbi: {} · {}/{} · {}",
        op_label, project, domain, summary
    );
    let commit_after = vcs::snapshot(&vault_path, &commit_msg)?;

    // log.md — human-readable timeline
    let log_entry = format!(
        "**{intent}** `{project}/{domain}` — {summary}",
        intent = intent,
        project = project,
        domain = domain,
        summary = summary
    );
    let _ = journal::append_log(&vault_path, &log_entry);

    // history.jsonl — structured event for recall / undo
    let event = HistoryEvent {
        ts: chrono::Local::now().to_rfc3339(),
        kind: "apply",
        project: Some(&project),
        domain: Some(&domain),
        intent: Some(&intent),
        user_message: Some(&user_message),
        summary: Some(&summary),
        op: Some(&op),
        commit_before: commit_before.as_deref(),
        commit_after: commit_after.as_deref(),
    };
    let _ = journal::append_history(&vault_path, &event);

    Ok(ApplyResult {
        project,
        domain,
        commit_before,
        commit_after,
        bytes_before: before.len(),
        bytes_after: after.len(),
    })
}

#[tauri::command]
pub fn daily_snapshot() -> DanbiResult<crate::daily::DailySnapshot> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?;
    let vault_path = match cfg.and_then(|c| c.vault_path) {
        Some(p) => PathBuf::from(p),
        None => {
            return Ok(crate::daily::DailySnapshot {
                today: chrono::Local::now().format("%Y-%m-%d").to_string(),
                today_notes: Vec::new(),
                one_week_ago: Vec::new(),
                one_month_ago: Vec::new(),
                one_year_ago: Vec::new(),
            })
        }
    };
    crate::daily::snapshot(&vault_path)
}

// ---------- MCP server control ----------

#[derive(Debug, Serialize, Clone)]
pub struct McpProjectEndpoint {
    pub project: String,
    pub id: String,
    pub url: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct McpStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub url: String,
    pub token: String,
    pub projects: Vec<McpProjectEndpoint>,
}

fn build_project_endpoints(vault_path: &PathBuf, port: u16) -> Vec<McpProjectEndpoint> {
    let tree = match vault::list_tree(vault_path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for p in tree.projects {
        match vault::ensure_project_id(vault_path, &p.name) {
            Ok(id) => out.push(McpProjectEndpoint {
                project: p.name.clone(),
                url: format!("http://127.0.0.1:{port}/mcp/{id}"),
                id,
            }),
            Err(_) => continue,
        }
    }
    out
}

#[tauri::command]
pub fn mcp_status(app: AppHandle) -> DanbiResult<McpStatus> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let server = app.state::<crate::mcp::McpServer>();
    let info = server.info();
    let projects = build_project_endpoints(&vault_path, cfg.mcp.port);
    Ok(McpStatus {
        enabled: cfg.mcp.enabled,
        running: info.is_some(),
        port: cfg.mcp.port,
        url: info
            .as_ref()
            .map(|i| i.url.clone())
            .unwrap_or_else(|| format!("http://127.0.0.1:{}/mcp", cfg.mcp.port)),
        token: cfg.mcp.token,
        projects,
    })
}

#[tauri::command]
pub fn mcp_enable(app: AppHandle, port: Option<u16>) -> DanbiResult<McpStatus> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    cfg.mcp.enabled = true;
    if let Some(p) = port {
        cfg.mcp.port = p;
    }
    if cfg.mcp.token.is_empty() {
        cfg.mcp.token = crate::mcp::generate_token();
    }
    let vault_path = require_vault(&cfg)?;
    config::save_config(&vault_path, &cfg)?;

    let server = app.state::<crate::mcp::McpServer>();
    server.start(cfg.mcp.port, cfg.mcp.token.clone());

    mcp_status(app)
}

#[tauri::command]
pub fn mcp_disable(app: AppHandle) -> DanbiResult<McpStatus> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    cfg.mcp.enabled = false;
    let vault_path = require_vault(&cfg)?;
    config::save_config(&vault_path, &cfg)?;

    let server = app.state::<crate::mcp::McpServer>();
    server.stop();

    mcp_status(app)
}

#[tauri::command]
pub fn mcp_project_endpoint(project: String) -> DanbiResult<McpProjectEndpoint> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let id = vault::ensure_project_id(&vault_path, &project)?;
    Ok(McpProjectEndpoint {
        project,
        url: format!("http://127.0.0.1:{}/mcp/{id}", cfg.mcp.port),
        id,
    })
}

#[tauri::command]
pub fn mcp_rotate_token(app: AppHandle) -> DanbiResult<McpStatus> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    cfg.mcp.token = crate::mcp::generate_token();
    let vault_path = require_vault(&cfg)?;
    config::save_config(&vault_path, &cfg)?;

    let server = app.state::<crate::mcp::McpServer>();
    if cfg.mcp.enabled {
        server.start(cfg.mcp.port, cfg.mcp.token.clone());
    }
    mcp_status(app)
}

#[tauri::command]
pub fn list_templates() -> Vec<crate::templates::VaultTemplate> {
    crate::templates::list_templates()
}

#[tauri::command]
pub fn apply_template(vault_path: String, template_id: String) -> DanbiResult<()> {
    let tpl = crate::templates::get_template(&template_id)
        .ok_or_else(|| DanbiError::Config(format!("unknown template: {template_id}")))?;
    crate::templates::apply_template(&PathBuf::from(vault_path), &tpl)
}

#[tauri::command]
pub fn ensure_today_note(project: String) -> DanbiResult<String> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::daily::ensure_today_note(&vault_path, &project)
}

#[tauri::command]
pub fn vault_suggestions() -> DanbiResult<Vec<crate::healing::Suggestion>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?;
    let vault_path = match cfg.and_then(|c| c.vault_path) {
        Some(p) => PathBuf::from(p),
        None => return Ok(Vec::new()),
    };
    crate::healing::scan(&vault_path)
}

#[tauri::command]
pub async fn build_link_index() -> DanbiResult<crate::links::LinkIndex> {
    tokio::task::spawn_blocking(|| {
        let vault = default_vault_path()?;
        let cfg = config::load_config(&vault)?;
        let vault_path = match cfg.and_then(|c| c.vault_path) {
            Some(p) => PathBuf::from(p),
            None => return Ok(crate::links::LinkIndex::default()),
        };
        crate::links::build_index(&vault_path)
    })
    .await
    .map_err(|e| DanbiError::Other(format!("join: {e}")))?
}

#[tauri::command]
pub fn recent_commits(limit: Option<usize>) -> DanbiResult<Vec<crate::vcs::CommitSummary>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?;
    let vault_path = match cfg.and_then(|c| c.vault_path) {
        Some(p) => PathBuf::from(p),
        None => return Ok(Vec::new()),
    };
    vcs::recent_commits(&vault_path, limit.unwrap_or(100))
}

/// How many commits each project has received since the user last looked
/// at it. Drives the `N` badge in the sidebar.
#[tauri::command]
pub async fn project_updates() -> DanbiResult<std::collections::HashMap<String, u32>> {
    tokio::task::spawn_blocking(|| {
        let vault = default_vault_path()?;
        let cfg = match config::load_config(&vault)? {
            Some(c) => c,
            None => return Ok(Default::default()),
        };
        let vault_path = require_vault(&cfg)?;
        crate::vcs::commits_per_project_since(&vault_path, &cfg.project_last_seen_at)
    })
    .await
    .map_err(|e| DanbiError::Other(format!("join: {e}")))?
}

/// Read the project's daily/*.md files and return a structured
/// "Auto-Journal" view — per-trigger counts for today, the latest 8
/// entries across the last 7 days, and per-day buckets. Used by the
/// new ProjectHome dashboard.
#[tauri::command]
pub fn project_journal_view(
    project: String,
) -> DanbiResult<crate::journal_view::ProjectJournalView> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::journal_view::view(&vault_path, &project)
}

/// Mark a project as "just seen" — stores the current epoch seconds so
/// `project_updates` will exclude everything up to this point going
/// forward. Called whenever the user selects a project.
#[tauri::command]
pub fn project_mark_seen(project: String) -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let now = chrono::Utc::now().timestamp();
    cfg.project_last_seen_at.insert(project, now);
    config::save_config(&vault, &cfg)?;
    Ok(())
}

/// Per-domain change list. Returns a map "<project>/<domain>" → kind
/// where kind is "modified" or "new". A domain is "new" if its
/// `domain_last_seen_at` entry is missing entirely; "modified" if its
/// file mtime > stored timestamp. Files with mtime < timestamp are
/// excluded (already seen). Designed to drive sidebar badges.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainChangeKind {
    New,
    Modified,
}

#[tauri::command]
pub async fn domain_updates() -> DanbiResult<std::collections::HashMap<String, DomainChangeKind>> {
    tokio::task::spawn_blocking(domain_updates_blocking)
        .await
        .map_err(|e| DanbiError::Other(format!("join: {e}")))?
}

fn domain_updates_blocking()
-> DanbiResult<std::collections::HashMap<String, DomainChangeKind>> {
    let vault = default_vault_path()?;
    let mut cfg = match config::load_config(&vault)? {
        Some(c) => c,
        None => return Ok(Default::default()),
    };
    let vault_path = require_vault(&cfg)?;
    let tree = vault::list_tree(&vault_path)?;
    // First-run / migration: if there are no last-seen entries at all,
    // treat *every* existing file as already seen. Otherwise the user
    // would face a wall of green dots on legacy vaults — most of them
    // are old files, not actual new work.
    let bootstrap = cfg.domain_last_seen_at.is_empty();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut out = std::collections::HashMap::new();
    // Recursively visit every domain in a project (top-level + nested
    // subfolders). The vault tree caps depth at 2 today but the walker
    // is structured to handle deeper trees if we ever lift the cap.
    fn walk_domains<'a, F: FnMut(&'a crate::vault::DomainNode)>(
        project: &'a crate::vault::ProjectNode,
        cb: &mut F,
    ) {
        for d in &project.domains {
            cb(d);
        }
        fn recurse<'b, F: FnMut(&'b crate::vault::DomainNode)>(
            subs: &'b [crate::vault::SubfolderNode],
            cb: &mut F,
        ) {
            for sub in subs {
                for d in &sub.domains {
                    cb(d);
                }
                recurse(&sub.subfolders, cb);
            }
        }
        recurse(&project.subfolders, cb);
    }

    if bootstrap {
        for project in &tree.projects {
            walk_domains(project, &mut |d| {
                let stamp = d
                    .modified_ms
                    .map(|m| m as i64)
                    .unwrap_or(0)
                    .max(now_ms);
                cfg.domain_last_seen_at
                    .insert(format!("{}/{}", project.name, d.name), stamp);
            });
        }
        config::save_config(&vault, &cfg)?;
        return Ok(out);
    }
    for project in &tree.projects {
        // We store last_seen as MILLISECONDS so it compares directly
        // against modified_ms — seconds-precision dropped sub-second
        // distinctions and made same-second clicks resurrect the dot on
        // the next watcher tick.
        let scan = |key: String, modified_ms: Option<u128>| {
            let Some(mtime_ms) = modified_ms else { return None };
            let mtime_ms = mtime_ms as i128;
            let last_seen = cfg.domain_last_seen_at.get(&key).copied();
            // Older vaults stored last_seen as Unix seconds; current
            // code stores milliseconds. Auto-promote so the comparison
            // doesn't always read as "modified" forever.
            let last_seen_ms: Option<i128> = last_seen.map(|t| {
                let v = t as i128;
                // Anything below year-2200-in-seconds (~7.2e9) is
                // definitely seconds. Anything above is already ms.
                if v < 10_000_000_000 { v * 1000 } else { v }
            });
            match last_seen_ms {
                None => Some(DomainChangeKind::New),
                Some(t) if mtime_ms > t => Some(DomainChangeKind::Modified),
                _ => None,
            }
        };
        walk_domains(project, &mut |d| {
            let key = format!("{}/{}", project.name, d.name);
            if let Some(k) = scan(key.clone(), d.modified_ms) {
                out.insert(key, k);
            }
        });
    }
    Ok(out)
}

/// Clear the badge for a single domain by recording "I just saw this".
/// We store the MAX of (file's current mtime, wall clock now) in ms so
/// the badge stays cleared even if the watcher fires again moments
/// later — `mtime > last_seen` will be false.
#[tauri::command]
pub fn domain_mark_seen(project: String, domain: String) -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let file = vault_path
        .join(crate::vault::PROJECTS_DIRNAME)
        .join(&project)
        .join(&domain);
    let mtime_ms: i64 = std::fs::metadata(&file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let stamp = mtime_ms.max(now_ms);
    cfg.domain_last_seen_at
        .insert(format!("{project}/{domain}"), stamp);
    config::save_config(&vault, &cfg)?;
    Ok(())
}

/// Mark every domain in a project as read. Walks the project tree and
/// stamps `domain_last_seen_at` for each .md file. Also updates
/// `project_last_seen_at` so the project-level "N" badge clears.
#[tauri::command]
pub fn project_mark_all_read(project: String) -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let tree = vault::list_tree(&vault_path)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    if let Some(p) = tree.projects.iter().find(|p| p.name == project) {
        let mut stamp_one = |domain_name: &str, modified_ms: Option<u128>| {
            let mtime_ms = modified_ms.map(|m| m as i64).unwrap_or(0);
            let stamp = mtime_ms.max(now_ms);
            cfg.domain_last_seen_at
                .insert(format!("{}/{}", project, domain_name), stamp);
        };
        for d in &p.domains {
            stamp_one(&d.name, d.modified_ms);
        }
        // Recurse through every nesting level so users can "mark all
        // read" on projects that have nested folders like
        // daily/2026-05/.
        fn recurse<F: FnMut(&str, Option<u128>)>(
            subs: &[crate::vault::SubfolderNode],
            cb: &mut F,
        ) {
            for sub in subs {
                for d in &sub.domains {
                    cb(&d.name, d.modified_ms);
                }
                recurse(&sub.subfolders, cb);
            }
        }
        recurse(&p.subfolders, &mut stamp_one);
    }
    cfg.project_last_seen_at
        .insert(project, chrono::Utc::now().timestamp());
    config::save_config(&vault, &cfg)?;
    Ok(())
}

/// Mark every project + every domain in the vault as read in one shot.
/// Used by the sidebar's "모두 읽음" button so the user can dismiss
/// every "N" badge across all projects without clicking each one.
#[tauri::command]
pub fn vault_mark_all_read() -> DanbiResult<usize> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let tree = vault::list_tree(&vault_path)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let now_secs = chrono::Utc::now().timestamp();

    fn recurse<F: FnMut(&str, Option<u128>)>(
        subs: &[crate::vault::SubfolderNode],
        cb: &mut F,
    ) {
        for sub in subs {
            for d in &sub.domains {
                cb(&d.name, d.modified_ms);
            }
            recurse(&sub.subfolders, cb);
        }
    }

    let mut stamped = 0usize;
    for p in &tree.projects {
        // Stamp every domain — both top-level files and recursively
        // nested ones — so per-domain "modified" dots clear too.
        let project = &p.name;
        for d in &p.domains {
            let mtime_ms = d.modified_ms.map(|m| m as i64).unwrap_or(0);
            let stamp = mtime_ms.max(now_ms);
            cfg.domain_last_seen_at
                .insert(format!("{}/{}", project, d.name), stamp);
            stamped += 1;
        }
        let mut stamp_one = |domain_name: &str, modified_ms: Option<u128>| {
            let mtime_ms = modified_ms.map(|m| m as i64).unwrap_or(0);
            let stamp = mtime_ms.max(now_ms);
            cfg.domain_last_seen_at
                .insert(format!("{}/{}", project, domain_name), stamp);
            stamped += 1;
        };
        recurse(&p.subfolders, &mut stamp_one);

        // Project-level seen stamp (drives the project-row badge).
        cfg.project_last_seen_at
            .insert(project.clone(), now_secs);
    }
    config::save_config(&vault, &cfg)?;
    Ok(stamped)
}

/// Replace the full group list. Sidebar sends the whole array back on
/// every edit (drag-reorder, rename, create, delete) — keeps the IPC
/// surface dead simple. Groups that reference unknown project names are
/// silently filtered, and any project not mentioned in any group is
/// implicitly "Ungrouped" (rendered at the top).
#[tauri::command]
pub fn groups_set(
    groups: Vec<crate::config::ProjectGroup>,
) -> DanbiResult<Vec<crate::config::ProjectGroup>> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    // `cfg.projects` can be stale/empty on older vaults — scan the actual
    // Projects/ directory so DnD targets are never filtered out.
    let vault_dir = cfg
        .vault_path
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| vault.clone());
    let known: std::collections::HashSet<String> = vault::list_tree(&vault_dir)
        .map(|t| t.projects.into_iter().map(|p| p.name).collect())
        .unwrap_or_default();
    // Sanitize: strip unknown project names, drop empty-label groups but
    // keep empty-project groups (user might be setting up a new bucket).
    let mut seen_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut seen_projects: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut cleaned: Vec<crate::config::ProjectGroup> = Vec::new();
    for mut g in groups {
        if g.label.trim().is_empty() {
            continue;
        }
        if g.id.trim().is_empty() {
            continue;
        }
        if !seen_ids.insert(g.id.clone()) {
            continue; // drop duplicate IDs
        }
        g.projects.retain(|p| {
            if !known.contains(p) {
                return false;
            }
            // A project can only live in one group at a time — first
            // occurrence wins.
            seen_projects.insert(p.clone())
        });
        cleaned.push(g);
    }
    cfg.project_groups = cleaned;
    config::save_config(&vault, &cfg)?;
    Ok(cfg.project_groups)
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum QuickCaptureResult {
    /// Successfully stored. `where` is "project/domain".
    Stored {
        project: String,
        domain: String,
        intent: String,
        summary: String,
        commit_after: Option<String>,
    },
    /// Router needs more info; UI falls back to a chip picker.
    NeedsClarification {
        clarification_type: Option<String>,
        candidate_projects: Vec<String>,
        candidate_domains: Vec<String>,
        project: Option<String>,
    },
}

/// One-shot capture: route → plan (append) → apply → commit. Reuses the
/// existing pipeline so the entry point is indistinguishable from a regular
/// chat message.
#[tauri::command]
pub async fn quick_capture(
    app: AppHandle,
    message: String,
    project: Option<String>,
    domain: Option<String>,
) -> DanbiResult<QuickCaptureResult> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path_str = cfg
        .vault_path
        .clone()
        .ok_or_else(|| DanbiError::Config("vault path missing".into()))?;
    let vault_path = PathBuf::from(&vault_path_str);

    // 0.1 부터는 LLM 우회 경로가 메인이다 — 사용자가 chip picker 로
    // 프로젝트·도메인을 직접 고르면 routing/writer 없이 순수 append.
    // 도메인 미지정이면 오늘자 daily 노트로 자동 라우팅 (LLM 무관).
    if let Some(proj) = project.clone() {
        let dom = match domain.clone() {
            Some(d) => d,
            None => format!("daily/{}.md", chrono::Local::now().format("%Y-%m-%d")),
        };
        return capture_append_no_llm(&app, &vault_path, &proj, &dom, &message, &cfg).await;
    }

    // 프로젝트가 비어있으면 frontend chip picker 로 강제 — LLM 부르지 않고
    // 즉시 NeedsClarification 반환.
    let tree = vault::list_tree(&vault_path)?;
    let projects: Vec<String> = tree.projects.iter().map(|p| p.name.clone()).collect();
    let _ = (&app, &message);
    return Ok(QuickCaptureResult::NeedsClarification {
        clarification_type: Some("project".into()),
        candidate_projects: projects,
        candidate_domains: Vec::new(),
        project: None,
    });

    // 아래 LLM 경로는 향후 LLM 옵션을 다시 켤 때를 위해 보존. 0.1 에서는
    // 위 분기가 항상 먼저 return 하므로 dead code 지만 컴파일은 유지.
    #[allow(unreachable_code)]
    {
    let routing_model = cfg
        .models
        .routing
        .clone()
        .ok_or_else(|| DanbiError::Config("routing model missing".into()))?;
    let writer_model = cfg
        .models
        .writer
        .clone()
        .ok_or_else(|| DanbiError::Config("writer model missing".into()))?;

    let mut domain_map = std::collections::HashMap::new();
    for p in &tree.projects {
        domain_map.insert(
            p.name.clone(),
            p.domains.iter().map(|d| d.name.clone()).collect(),
        );
    }

    let ctx = routing::RoutingContext {
        projects: projects.clone(),
        domains: domain_map,
        sticky_project: project.clone(),
        sticky_domain: domain.clone(),
    };

    let provider = resolve_provider(&cfg)?;
    let route = routing::route(provider.as_ref(), &routing_model, &message, &ctx).await?;

    if route.needs_clarification || route.project.is_none() || route.domain.is_none() {
        let kind = route.clarification_type.clone().unwrap_or_default();
        let what = if kind == "project" { "프로젝트" } else { "도메인" };
        notify(
            &app,
            "단비 · 추가 정보가 필요해요",
            &format!("{what}을 지정한 뒤 다시 시도해 주세요."),
        );
        return Ok(QuickCaptureResult::NeedsClarification {
            clarification_type: route.clarification_type,
            candidate_projects: route.candidate_projects,
            candidate_domains: route.candidate_domains,
            project: route.project,
        });
    }

    let proj = route.project.clone().unwrap();
    let dom = route.domain.clone().unwrap();
    let doc = vault::read_doc(&vault_path, &proj, &dom)?;

    let grounding_embedding = embed_query_for_search(&cfg, &message).await;
    let grounding = crate::grounding::gather_grounding(
        &vault_path,
        Some(&proj),
        &message,
        grounding_embedding.as_deref(),
        Some(&dom),
        4,
        800,
    )
    .unwrap_or_default();

    let project_ctx = crate::project_context::load(&vault_path, &proj);

    let plan = preview::build_plan(
        provider.as_ref(),
        &writer_model,
        preview::PlanInput {
            intent: &route.intent,
            project: &proj,
            domain: &dom,
            user_message: &message,
            doc_content: &doc,
            attachments: &[],
            grounding: &grounding,
            project_ctx: &project_ctx,
        },
    )
    .await?;

    let op = plan.op.unwrap_or(EditOp::Append {
        content: message.clone(),
    });
    edit_ops::validate(&op)?;

    vcs::ensure_repo(&vault_path)?;
    let before = vault::read_doc(&vault_path, &proj, &dom)?;
    let after = edit_ops::apply(&before, &op)?;

    let commit_before = vcs::snapshot(
        &vault_path,
        &format!("danbi: quick-capture pre · {}/{}", proj, dom),
    )?;
    vault::write_doc(&vault_path, &proj, &dom, &after)?;
    let commit_after = vcs::snapshot(
        &vault_path,
        &format!("danbi: quick-capture · {}/{} · {}", proj, dom, plan.summary),
    )?;

    let _ = journal::append_log(
        &vault_path,
        &format!(
            "**quick** `{proj}/{dom}` — {summary}",
            proj = proj,
            dom = dom,
            summary = plan.summary
        ),
    );
    let event = HistoryEvent {
        ts: chrono::Local::now().to_rfc3339(),
        kind: "quick_capture",
        project: Some(&proj),
        domain: Some(&dom),
        intent: Some(&route.intent),
        user_message: Some(&message),
        summary: Some(&plan.summary),
        op: Some(&op),
        commit_before: commit_before.as_deref(),
        commit_after: commit_after.as_deref(),
    };
    let _ = journal::append_history(&vault_path, &event);

    // Persist sticky capture context so the next popup remembers the chip.
    let mut updated = cfg.clone();
    updated.capture = crate::config::CaptureState {
        last_project: Some(proj.clone()),
        last_domain: Some(dom.clone()),
    };
    let _ = config::save_config(&vault_path, &updated);

    notify(
        &app,
        &format!("단비 · {}/{}", proj, dom),
        &plan.summary,
    );

    Ok(QuickCaptureResult::Stored {
        project: proj,
        domain: dom,
        intent: route.intent,
        summary: plan.summary,
        commit_after,
    })
    }
}

/// LLM 우회 append 경로. Quick Capture 의 0.1 default.
/// 사용자가 chip picker 로 project + domain 을 명시했을 때 호출된다.
/// daily 노트 또는 임의 도메인 파일에 message 를 그대로 append.
async fn capture_append_no_llm(
    app: &AppHandle,
    vault_path: &std::path::Path,
    project: &str,
    domain: &str,
    message: &str,
    cfg: &config::DanbiConfig,
) -> DanbiResult<QuickCaptureResult> {
    vcs::ensure_repo(vault_path)?;
    // 파일이 없으면 자동 생성 후 append 한다 — daily 노트는 일자별로
    // 매번 새로 만들어지는 게 정상 동작.
    let before = match vault::read_doc(vault_path, project, domain) {
        Ok(s) => s,
        Err(_) => {
            vault::write_doc(vault_path, project, domain, "")?;
            String::new()
        }
    };
    let op = EditOp::Append {
        content: message.to_string(),
    };
    let after = edit_ops::apply(&before, &op)?;
    let commit_before = vcs::snapshot(
        vault_path,
        &format!("danbi: capture pre · {}/{}", project, domain),
    )?;
    vault::write_doc(vault_path, project, domain, &after)?;
    let commit_after = vcs::snapshot(
        vault_path,
        &format!("danbi: capture · {}/{}", project, domain),
    )?;

    // summary: 메시지의 첫 줄만 잘라서 알림에 쓴다.
    let summary = message
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();

    let _ = journal::append_log(
        vault_path,
        &format!(
            "**capture** `{proj}/{dom}` — {summary}",
            proj = project,
            dom = domain,
            summary = if summary.is_empty() { "(empty)" } else { &summary }
        ),
    );
    let event = HistoryEvent {
        ts: chrono::Local::now().to_rfc3339(),
        kind: "quick_capture",
        project: Some(project),
        domain: Some(domain),
        intent: None,
        user_message: Some(message),
        summary: Some(&summary),
        op: Some(&op),
        commit_before: commit_before.as_deref(),
        commit_after: commit_after.as_deref(),
    };
    let _ = journal::append_history(vault_path, &event);

    let mut updated = cfg.clone();
    updated.capture = crate::config::CaptureState {
        last_project: Some(project.to_string()),
        last_domain: Some(domain.to_string()),
    };
    let _ = config::save_config(vault_path, &updated);

    // 시스템 알림은 0.1 에서 제거 — Quick Capture 가 매번 macOS Notification
    // Center 를 띄우면 사용자 흐름을 끊어서. 저장 성공 신호는 popup 자체가
    // 자동으로 닫히는 것으로 충분.
    let _ = app;

    Ok(QuickCaptureResult::Stored {
        project: project.to_string(),
        domain: domain.to_string(),
        intent: "append".into(),
        summary,
        commit_after,
    })
}

#[tauri::command]
pub fn toggle_capture(app: AppHandle) -> DanbiResult<()> {
    crate::capture::toggle_capture_window(&app)
        .map_err(|e| DanbiError::Other(format!("capture toggle: {e}")))
}

#[derive(Debug, Serialize, Clone)]
pub struct CaptureContext {
    pub projects: Vec<String>,
    pub domains: std::collections::HashMap<String, Vec<String>>,
    pub last_project: Option<String>,
    pub last_domain: Option<String>,
}

#[tauri::command]
pub fn search_local(query: String, limit: Option<usize>) -> DanbiResult<Vec<crate::search::SearchHit>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?;
    let vault_path = match cfg.and_then(|c| c.vault_path) {
        Some(p) => PathBuf::from(p),
        None => return Ok(Vec::new()),
    };
    let index = crate::search::build_index(&vault_path)?;
    Ok(crate::search::local_search(&index, &query, limit.unwrap_or(8)))
}

#[tauri::command]
pub async fn search_full(
    query: String,
    limit: Option<usize>,
) -> DanbiResult<Vec<crate::search::SearchHit>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?;
    let cfg = match cfg {
        Some(c) => c,
        None => return Ok(Vec::new()),
    };
    let vault_path = match cfg.vault_path.clone() {
        Some(p) => PathBuf::from(p),
        None => return Ok(Vec::new()),
    };
    let lim = limit.unwrap_or(10);

    // 사용자가 임베딩 provider 를 설정해뒀으면 RRF 하이브리드 검색,
    // 아니면 BM25 만 사용.
    let embedding = embed_query_for_search(&cfg, &query).await;
    crate::search::full_search_hybrid(&vault_path, &query, lim, embedding.as_deref())
}

#[derive(Debug, Serialize, Clone)]
pub struct CompoundPreview {
    pub target_project: String,
    pub target_domain: String,
    pub plan: crate::compound::CompoundPlan,
    pub sources: Vec<crate::compound::CompoundSource>,
    /// Rough estimate of Writer tokens consumed for cost transparency.
    pub approx_input_chars: usize,
    pub approx_output_chars: usize,
}

#[tauri::command]
pub async fn compound_preview(
    topic: String,
    project: String,
    target_domain: String,
    max_sources: Option<usize>,
) -> DanbiResult<CompoundPreview> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let writer_model = cfg
        .models
        .writer
        .clone()
        .ok_or_else(|| DanbiError::Config("writer model missing".into()))?;
    let vault_path = require_vault(&cfg)?;

    let topic_embedding = embed_query_for_search(&cfg, &topic).await;
    let mut sources =
        crate::compound::gather_sources(&vault_path, &topic, topic_embedding.as_deref())?;
    if let Some(cap) = max_sources {
        sources.truncate(cap.max(1));
    }

    let input_chars: usize = sources.iter().map(|s| s.content.chars().count()).sum();

    let provider = resolve_provider(&cfg)?;

    // Normalize target filename — ensure .md extension.
    let target = if target_domain.to_lowercase().ends_with(".md") {
        target_domain.clone()
    } else {
        format!("{target_domain}.md")
    };

    let plan = crate::compound::build_plan(
        &vault_path,
        provider.as_ref(),
        &writer_model,
        &topic,
        &format!("{project}/{target}"),
        &sources,
    )
    .await?;

    let output_chars = plan.draft.chars().count();

    Ok(CompoundPreview {
        target_project: project,
        target_domain: target,
        plan,
        sources,
        approx_input_chars: input_chars,
        approx_output_chars: output_chars,
    })
}

#[tauri::command]
pub fn compound_apply(
    project: String,
    domain: String,
    draft: String,
    summary: String,
    user_message: String,
) -> DanbiResult<ApplyResult> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;

    vcs::ensure_repo(&vault_path)?;

    // Target may be a new file — ensure project exists, then decide whether we're
    // creating or overwriting.
    let existing = vault::read_doc(&vault_path, &project, &domain).unwrap_or_default();
    let bytes_before = existing.len();

    let commit_before = vcs::snapshot(
        &vault_path,
        &format!("danbi: pre-compound · {}/{}", project, domain),
    )?;

    // Ensure the domain file exists first so write_doc succeeds even for new
    // targets; create_domain is a no-op if it already exists.
    let _ = vault::create_domain(&vault_path, &project, &domain);

    vault::write_doc(&vault_path, &project, &domain, &draft)?;

    let commit_after = vcs::snapshot(
        &vault_path,
        &format!("danbi: compound · {}/{} · {}", project, domain, summary),
    )?;

    let _ = journal::append_log(
        &vault_path,
        &format!(
            "**compound** `{project}/{domain}` — {summary}",
            project = project,
            domain = domain,
            summary = summary
        ),
    );
    let event = HistoryEvent {
        ts: chrono::Local::now().to_rfc3339(),
        kind: "compound",
        project: Some(&project),
        domain: Some(&domain),
        intent: Some("compound"),
        user_message: Some(&user_message),
        summary: Some(&summary),
        op: None,
        commit_before: commit_before.as_deref(),
        commit_after: commit_after.as_deref(),
    };
    let _ = journal::append_history(&vault_path, &event);

    Ok(ApplyResult {
        project,
        domain,
        commit_before,
        commit_after,
        bytes_before,
        bytes_after: draft.len(),
    })
}

#[tauri::command]
pub async fn search_vault(query: String) -> DanbiResult<crate::search::SearchResponse> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let model = cfg
        .models
        .routing
        .clone()
        .ok_or_else(|| DanbiError::Config("routing model missing".into()))?;
    let vault_path = require_vault(&cfg)?;

    let index = crate::search::build_index(&vault_path)?;
    let provider = resolve_provider(&cfg)?;

    crate::search::search(provider.as_ref(), &model, &query, &index).await
}

#[tauri::command]
pub fn capture_context() -> DanbiResult<CaptureContext> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let tree = vault::list_tree(&vault_path)?;
    let projects: Vec<String> = tree.projects.iter().map(|p| p.name.clone()).collect();
    let mut domains = std::collections::HashMap::new();
    for p in &tree.projects {
        domains.insert(
            p.name.clone(),
            p.domains.iter().map(|d| d.name.clone()).collect(),
        );
    }
    Ok(CaptureContext {
        projects,
        domains,
        last_project: cfg.capture.last_project,
        last_domain: cfg.capture.last_domain,
    })
}

#[tauri::command]
pub fn apply_capture_shortcut(app: AppHandle, accelerator: String) -> DanbiResult<()> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    crate::shortcuts::apply_capture_shortcut(&app.global_shortcut(), &accelerator)
}

#[tauri::command]
pub fn validate_shortcut(accelerator: String) -> DanbiResult<()> {
    crate::shortcuts::validate_accelerator(&accelerator)
}

#[tauri::command]
pub fn hide_capture(app: AppHandle) -> DanbiResult<()> {
    if let Some(win) = app.get_webview_window(crate::capture::CAPTURE_LABEL) {
        let _ = win.hide();
    }
    Ok(())
}

/// Quick Capture popup 의 높이만 동적으로 조정한다. 검색 결과 list /
/// 스니펫 미리보기를 펼칠 때 사용. 화면 위치는 capture.rs 의 reposition
/// 함수가 보정한다 (높이가 늘어나도 화면 하단에서 같은 거리 유지).
#[tauri::command]
pub fn resize_capture(app: AppHandle, height: f64) -> DanbiResult<()> {
    crate::capture::resize_capture_window(&app, height)
        .map_err(|e| DanbiError::Other(format!("capture resize: {e}")))
}

/// AI 연동 (cfg.embed_provider) 이 켜져있을 때, 자동화 (요약·purpose 작성·
/// ghost 제안) 에 쓸 LLM 모델 id 를 결정. cfg.automation_model 이 명시돼
/// 있으면 그것을 우선, 없으면 provider 별 가벼운 디폴트.
fn resolve_summarize_model(cfg: &DanbiConfig) -> Option<(String, &'static str)> {
    let kind = cfg.embed_provider.as_ref()?.kind_str();
    let kind_static: &'static str = match kind {
        "google" => "google",
        "bedrock" => "bedrock",
        _ => return None,
    };
    if let Some(m) = cfg.automation_model.as_ref().filter(|s| !s.trim().is_empty()) {
        return Some((m.clone(), kind_static));
    }
    // Claude on Bedrock 은 on-demand throughput 이 안 되고 region 별
    // inference profile (us./eu./apac. prefix) 로만 호출 가능하다. 기본
    // 'us.' prefix 디폴트. 사용자가 다른 region 쓰면 Settings 에서 모델
    // ID 직접 넣을 수 있다.
    let default_model = match kind_static {
        "google" => "gemini-2.5-flash-lite",
        "bedrock" => "us.anthropic.claude-haiku-4-5-20251001-v1:0",
        _ => return None,
    };
    Some((default_model.to_string(), kind_static))
}

#[derive(Debug, Serialize, Clone)]
pub struct SummarizeDailyResult {
    pub summary_md: String,
    pub html: String,
    pub provider: String,
    pub model: String,
    /// 결과는 자동으로 vault 의 `.danbi/exports/` 에 영구 저장된다.
    /// frontend 가 이 id 로 history 패널 / 다시 열기 등을 연결.
    pub export_id: String,
}

/// 사용자가 daily 노트 화면에서 "요약 / HTML 추출" 버튼을 누르면 호출.
/// AI 연동 켜진 provider 의 가벼운 LLM 으로 본문을 받아 요약 markdown
/// + 공유용 HTML 한 페이지를 함께 돌려준다. AI 미연동이면 명시적
/// error — frontend 가 disabled 처리하므로 정상 흐름에선 도달하지 않음.
#[tauri::command]
pub async fn summarize_daily(
    project: String,
    domain: String,
) -> DanbiResult<SummarizeDailyResult> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let body = vault::read_doc(&vault_path, &project, &domain)?;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(DanbiError::Config("empty note".into()));
    }
    let (model, kind) = resolve_summarize_model(&cfg).ok_or_else(|| {
        DanbiError::Config("AI 연동이 꺼져있어요. Settings 에서 임베딩 provider 를 먼저 연결하세요.".into())
    })?;
    let provider = resolve_embed_provider(&cfg)?;

    let system = "당신은 한국어 마크다운 작성기입니다. 입력으로 받은 daily 노트를 사람이 다음에 다시 봤을 때 그 날의 작업이 또렷이 기억나도록 정리합니다.\n\n원칙:\n- 압축이 아니라 재구성. 본문에 있는 사실은 빠뜨리지 말고 옮기되, 잡담·중복은 제거.\n- 다음 4개 H2 섹션을 사용 (없으면 해당 섹션 생략):\n  - '## 결정' — 무엇을 정했고 왜 정했는지. 결정마다 2~5줄.\n  - '## 구현 변경' — 코드/파일/UI 단위로 무엇이 바뀌었는지. 파일 경로·함수명·핵심 코드 키워드 포함.\n  - '## 노하우' — 다음에 도움될 인사이트. 외부 서비스 한도, 라이브러리 함정 등.\n  - '## 다음 후보' — 이어갈 작업. 본문에서 명시된 항목만.\n- 각 섹션은 2~6개 bullet. bullet 안에 1~3줄 본문 가능.\n- 코드명·파일명은 backtick.\n- 출력은 markdown 만 — HTML, 부연 설명, 인사말 금지.";
    let user = format!(
        "다음 daily 노트를 위 원칙대로 정리해줘. 하루치 작업이 빠짐없이 보이게 풍부하게 적되, 같은 내용을 두 번 적지는 마.\n\n---\n{}\n---",
        trimmed.chars().take(20_000).collect::<String>(),
    );
    let summary_md = crate::usage::with_role(
        "summarize",
        provider.converse_text(&model, Some(system), &user, 2400, 0.5),
    )
    .await?;

    // markdown → 간단 HTML 변환. 외부 의존성 없이 줄 단위 변환만 수행.
    let html = simple_md_to_html(&summary_md);

    // 자동으로 .danbi/exports/ 에 영구 저장. 사용자가 명시 트리거한
    // 시점만 호출되니 자동 누적이 부담스럽지 않고, "이전 요약 다시
    // 보기" 가 1클릭으로 가능해진다.
    let record = crate::exports::save_export(
        &vault_path,
        &project,
        &domain,
        &summary_md,
        &html,
        kind,
        &model,
    )
    .map_err(|e| DanbiError::Other(format!("export save: {e}")))?;

    Ok(SummarizeDailyResult {
        summary_md,
        html,
        provider: kind.to_string(),
        model,
        export_id: record.id,
    })
}

/// 사용자가 daily 노트의 export history 를 볼 때 호출. 같은 노트의
/// 이전 요약 list 를 newest-first 로 반환.
#[tauri::command]
pub fn list_exports(
    project: Option<String>,
    source_domain: Option<String>,
) -> DanbiResult<Vec<crate::exports::ExportRecord>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::exports::list(
        &vault_path,
        project.as_deref(),
        source_domain.as_deref(),
    )
}

/// 저장된 export 한 건을 다시 webview 로 열기. id 만 받고 디스크에서
/// 직접 읽어 새 윈도우 띄움 — frontend 가 매번 html 들고 다닐 필요 X.
#[tauri::command]
pub fn open_export(
    app: AppHandle,
    id: String,
) -> DanbiResult<String> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let html = crate::exports::read_html(&vault_path, &id)?;
    open_html_preview(app, html, format!("export-{id}"))
}

#[derive(Debug, Serialize, Clone)]
pub struct ExportDocHtmlResult {
    pub html: String,
    /// `.danbi/exports/` 에 저장된 export id — frontend 가 history
    /// pill / 알림 / 다시 열기 같은 동작에 같이 묶어 처리.
    pub export_id: String,
}

/// 임의의 md 파일을 LLM 없이 곧바로 카드형 HTML 페이지로 변환.
/// daily 노트 전용 `summarize_daily` 와 달리 본문을 그대로 변환만 하므로
/// AI 연동 / 토큰 소비 0. 모든 .md 파일에서 동작.
///
/// 결과는 `.danbi/exports/` 에도 저장되어 history 패널에 등장한다.
#[tauri::command]
pub fn export_doc_html(
    project: String,
    domain: String,
) -> DanbiResult<ExportDocHtmlResult> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let body = vault::read_doc(&vault_path, &project, &domain)?;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(DanbiError::Config("empty note".into()));
    }
    let html = simple_md_to_html(trimmed);
    // history 인덱스에는 LLM 호출 0 임을 분명히 하기 위해 provider/model
    // 자리에 "raw" 라고 박는다.
    let record = crate::exports::save_export(
        &vault_path,
        &project,
        &domain,
        trimmed,
        &html,
        "raw",
        "raw",
    )
    .map_err(|e| DanbiError::Other(format!("export save: {e}")))?;
    Ok(ExportDocHtmlResult {
        html,
        export_id: record.id,
    })
}

/// 사용자가 "HTML 페이지로 열기" 버튼을 누르면 호출. 임시 파일에 HTML
/// 을 쓰고 그 경로를 webview 윈도우로 띄운다. 임시 파일은 OS 의 temp
/// 디렉토리 (`std::env::temp_dir()`) 에 두고, 같은 세션에서 여러 번
/// 띄워도 충돌하지 않도록 timestamp + nanos 를 파일명에 포함.
#[tauri::command]
pub fn open_html_preview(
    app: AppHandle,
    html: String,
    title: String,
) -> DanbiResult<String> {
    use std::time::SystemTime;
    let stamp = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let safe_title = title
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect::<String>();
    let stem = if safe_title.is_empty() {
        format!("danbi-summary-{stamp}")
    } else {
        format!("danbi-{safe_title}-{stamp}")
    };
    let temp = std::env::temp_dir().join(format!("{stem}.html"));
    std::fs::write(&temp, &html)
        .map_err(|e| DanbiError::Other(format!("temp html write: {e}")))?;

    let label = format!("preview-{stamp}");
    let path_str = temp.to_string_lossy().to_string();
    let file_url = format!("file://{path_str}");
    let url = tauri::WebviewUrl::External(
        file_url
            .parse()
            .map_err(|e| DanbiError::Other(format!("preview url parse: {e}")))?,
    );
    tauri::WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("단비 · {title}"))
        .inner_size(820.0, 720.0)
        .resizable(true)
        .build()
        .map_err(|e| DanbiError::Other(format!("preview window: {e}")))?;
    Ok(path_str)
}

/// HTML 을 사용자가 고른 경로에 영구 저장. 다이얼로그는 frontend 에서
/// `@tauri-apps/plugin-dialog` 의 save() 로 열고, 결과 경로를 받아서
/// 이 IPC 가 실제 write_all 만 담당.
#[tauri::command]
pub fn save_html_to_path(path: String, html: String) -> DanbiResult<()> {
    std::fs::write(&path, &html)
        .map_err(|e| DanbiError::Other(format!("html save {path}: {e}")))?;
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
pub struct ComposePurposeSchemaResult {
    pub markdown: String,
    pub provider: String,
    pub model: String,
}

/// purpose.md / schema.md 빈 골격을 채워주는 LLM 호출. 사용자가 빈 골격
/// 배너에서 "단비가 작성" 버튼을 누르면 호출된다. vault 의 daily 노트
/// 와 다른 도메인 파일을 grounding 으로 읽고 한국어 markdown 한 페이지를
/// 돌려준다. **vault 에 자동으로 쓰지 않는다** — frontend 가 미리보기
/// 모달로 보여주고, 사용자가 명시적으로 "적용" 을 누를 때만 write_doc 가
/// 별개 IPC 로 실행됨 (덮어쓰기는 사용자 책임).
#[tauri::command]
pub async fn compose_purpose_schema(
    project: String,
    kind: String, // "purpose" | "schema"
) -> DanbiResult<ComposePurposeSchemaResult> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let (model, provider_kind) = resolve_summarize_model(&cfg).ok_or_else(|| {
        DanbiError::Config("AI 연동이 꺼져있어요. Settings 에서 임베딩 provider 를 먼저 연결하세요.".into())
    })?;
    let provider = resolve_embed_provider(&cfg)?;

    // grounding: 같은 프로젝트의 daily 노트 (최신 5개) + notes/ 디렉토리
    // 도메인 (최대 6개) 를 짧게 발췌해서 LLM 컨텍스트에 넣는다. 너무
    // 길면 토큰 / RPD 무료 한도가 빨리 닳으므로 도메인당 1500 char 까지만.
    let tree = vault::list_tree(&vault_path)?;
    let proj = tree
        .projects
        .iter()
        .find(|p| p.name == project)
        .ok_or_else(|| DanbiError::Config(format!("unknown project: {project}")))?;

    let mut grounding = String::new();
    grounding.push_str(&format!("# 프로젝트: {}\n\n", project));

    // 프로젝트 안의 모든 .md 파일을 grounding 으로 사용한다.
    //   1) 루트 도메인 (예: purpose.md, schema.md, todo.md, 주의사항.md)
    //   2) 모든 sub-folder 의 모든 도메인 (재귀)
    //
    // 단, daily/ 는 양이 너무 커서 무료 한도 / 컨텍스트 윈도우를 잡아
    // 먹으니 최신 7개만. 그 외 폴더는 사이즈 큰 순으로 우선 (정리된
    // 노트가 일반적으로 큼).
    //
    // 자기 자신 (purpose.md / schema.md) 은 건너뛴다 — 우리가 새로
    // 작성하는 거니까.

    fn collect_recursive(
        base: &str,
        sub: &crate::vault::SubfolderNode,
        out: &mut Vec<(String, u64)>,
    ) {
        for d in &sub.domains {
            let path = if base.is_empty() {
                d.name.clone()
            } else {
                format!("{base}/{}", d.name)
            };
            out.push((path, d.bytes));
        }
        for nested in &sub.subfolders {
            let next_base = if base.is_empty() {
                nested.name.clone()
            } else {
                format!("{base}/{}", nested.name)
            };
            collect_recursive(&next_base, nested, out);
        }
    }

    let exclude_self: &str = match kind.as_str() {
        "purpose" => "purpose.md",
        "schema" => "schema.md",
        _ => "",
    };
    let per_file_chars = 1000;
    let total_cap = 30_000;

    // 1) 루트 도메인 (사이즈 큰 순). purpose/schema 자기 자신은 제외.
    let mut root_files: Vec<(String, u64)> = proj
        .domains
        .iter()
        .filter(|d| d.name != exclude_self)
        .map(|d| (d.name.clone(), d.bytes))
        .collect();
    root_files.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, _) in &root_files {
        if grounding.len() >= total_cap {
            break;
        }
        if let Ok(body) = vault::read_doc(&vault_path, &project, name) {
            grounding.push_str(&format!(
                "\n## {}\n{}\n",
                name,
                body.chars().take(per_file_chars).collect::<String>(),
            ));
        }
    }

    // 2) 모든 sub-folder. daily 는 별도 처리, 나머지는 재귀로 모아 사이즈순.
    let mut other_files: Vec<(String, u64)> = Vec::new();
    for sub in &proj.subfolders {
        if sub.name == "daily" {
            // 최신 7개만
            let mut domains: Vec<&str> =
                sub.domains.iter().map(|d| d.name.as_str()).collect();
            domains.sort();
            domains.reverse();
            for dom_name in domains.iter().take(7) {
                if grounding.len() >= total_cap {
                    break;
                }
                let full = format!("daily/{}", dom_name);
                if let Ok(body) = vault::read_doc(&vault_path, &project, &full) {
                    grounding.push_str(&format!(
                        "\n## {}\n{}\n",
                        full,
                        body.chars().take(per_file_chars).collect::<String>(),
                    ));
                }
            }
        } else {
            collect_recursive(&sub.name, sub, &mut other_files);
        }
    }
    // 사이즈 큰 순 (정리된 문서 우선).
    other_files.sort_by(|a, b| b.1.cmp(&a.1));
    for (path, _) in &other_files {
        if grounding.len() >= total_cap {
            break;
        }
        if let Ok(body) = vault::read_doc(&vault_path, &project, path) {
            grounding.push_str(&format!(
                "\n## {}\n{}\n",
                path,
                body.chars().take(per_file_chars).collect::<String>(),
            ));
        }
    }

    let (system, ask) = match kind.as_str() {
        "purpose" => (
            "당신은 한국어 마크다운 작성기입니다. 입력으로 받은 vault 발췌를 보고 해당 프로젝트의 `purpose.md` 한 페이지를 작성합니다. 항상 다음 4개 H2 섹션을 사용하세요: '## 이 프로젝트는 무엇인가요?', '## 무엇을 다루나요?', '## 무엇을 다루지 않나요?', '## 지금 우선순위'. 각 섹션은 2~5줄 또는 짧은 bullet. 추측은 피하고 발췌에서 확인 가능한 사실만 적으세요. 출력은 markdown 만 — 부연 설명 없이.",
            "위 발췌를 바탕으로 `purpose.md` 작성해줘.",
        ),
        "schema" => (
            "당신은 한국어 마크다운 작성기입니다. 입력으로 받은 vault 발췌의 실제 사용 패턴을 보고 해당 프로젝트의 `schema.md` 를 작성합니다. 항상 다음 4개 H2 섹션을 사용하세요: '## 파일 네이밍', '## 문서 구조', '## 링크 정책', '## 스타일'. 각 섹션은 짧은 bullet. 발췌에서 실제로 보이는 패턴만 규칙으로 옮기고, 일반론은 피하세요. 출력은 markdown 만.",
            "위 발췌를 바탕으로 `schema.md` 작성해줘.",
        ),
        _ => return Err(DanbiError::Config("kind must be 'purpose' or 'schema'".into())),
    };

    let user = format!("{ask}\n\n---\n발췌:\n{}", grounding.chars().take(20_000).collect::<String>());

    let markdown = crate::usage::with_role(
        "compose",
        provider.converse_text(&model, Some(system), &user, 1200, 0.5),
    )
    .await?;

    Ok(ComposePurposeSchemaResult {
        markdown,
        provider: provider_kind.to_string(),
        model,
    })
}

/// 의도적으로 단순한 markdown → HTML. 진짜 markdown 파서 (pulldown-cmark
/// 등) 의존을 더하지 않으려고 줄 단위로만 처리. 헤더·bullet·강조만
/// 다루고 나머지는 escape + <p> 로 감싼다. 단비의 요약 출력이 워낙
/// 짧고 정형화돼있어서 충분.
fn simple_md_to_html(md: &str) -> String {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    fn inline(s: &str) -> String {
        let escaped = esc(s);
        // **bold** + *italic* + `code` 만 처리. backref-free.
        let mut out = String::with_capacity(escaped.len());
        let chars: Vec<char> = escaped.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // **bold**
            if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
                if let Some(end) = (i + 2..chars.len() - 1).find(|&j| {
                    chars[j] == '*' && chars[j + 1] == '*'
                }) {
                    let inner: String = chars[i + 2..end].iter().collect();
                    out.push_str("<strong>");
                    out.push_str(&inner);
                    out.push_str("</strong>");
                    i = end + 2;
                    continue;
                }
            }
            // `code`
            if chars[i] == '`' {
                if let Some(end) = (i + 1..chars.len()).find(|&j| chars[j] == '`') {
                    let inner: String = chars[i + 1..end].iter().collect();
                    out.push_str("<code>");
                    out.push_str(&inner);
                    out.push_str("</code>");
                    i = end + 1;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    // 본문 변환. H2 마다 새 card 로 감싸서 섹션 구분이 시각적으로
    // 또렷해지도록 한다. H1 은 별도 섹션 없이 페이지 제목으로.
    //
    // 구조:
    //   <h1>제목</h1>           (있으면)
    //   <section class="card" data-tone="…">  <!-- H2 한 개당 -->
    //     <h2>섹션 헤더</h2>
    //     ... bullet / paragraph
    //   </section>
    let mut body = String::new();
    let mut in_list = false;
    let mut in_section = false;
    let close_list = |out: &mut String, in_list: &mut bool| {
        if *in_list {
            out.push_str("</ul>\n");
            *in_list = false;
        }
    };
    let close_section = |out: &mut String, in_section: &mut bool, in_list: &mut bool| {
        close_list(out, in_list);
        if *in_section {
            out.push_str("</section>\n");
            *in_section = false;
        }
    };
    // H2 텍스트로 section 톤 (좌측 strip 색) 결정. 한국어 단비 노트의
    // 표준 4섹션을 인지하고 각각에 다른 accent 줘서 한눈에 구분.
    fn section_tone(header: &str) -> &'static str {
        if header.contains("결정") {
            "decision"
        } else if header.contains("구현") || header.contains("변경") {
            "change"
        } else if header.contains("노하우") || header.contains("배움") {
            "learn"
        } else if header.contains("재발") || header.contains("주의") {
            "warn"
        } else if header.contains("다음") || header.contains("후보") {
            "next"
        } else if header.contains("관련") {
            "link"
        } else {
            "default"
        }
    }
    for raw in md.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            close_list(&mut body, &mut in_list);
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            close_section(&mut body, &mut in_section, &mut in_list);
            let tone = section_tone(rest);
            body.push_str(&format!(
                "<section class=\"card\" data-tone=\"{}\">\n<h2>{}</h2>\n",
                tone,
                inline(rest),
            ));
            in_section = true;
        } else if let Some(rest) = line.strip_prefix("# ") {
            close_section(&mut body, &mut in_section, &mut in_list);
            body.push_str(&format!("<h1>{}</h1>\n", inline(rest)));
        } else if let Some(rest) = line.strip_prefix("### ") {
            close_list(&mut body, &mut in_list);
            body.push_str(&format!("<h3>{}</h3>\n", inline(rest)));
        } else if let Some(rest) = line.strip_prefix("> ") {
            close_list(&mut body, &mut in_list);
            body.push_str(&format!("<blockquote>{}</blockquote>\n", inline(rest)));
        } else if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            if !in_list {
                body.push_str("<ul>\n");
                in_list = true;
            }
            body.push_str(&format!("  <li>{}</li>\n", inline(rest)));
        } else {
            close_list(&mut body, &mut in_list);
            body.push_str(&format!("<p>{}</p>\n", inline(line)));
        }
    }
    close_section(&mut body, &mut in_section, &mut in_list);

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    // 모던 typographic 톤. 라이트/다크 prefers-color-scheme 양쪽 대응.
    // 외부 폰트 (Inter / Pretendard) CDN 의존을 안 둬서 오프라인에서도
    // 일관되게 보이고, 시스템 fallback 로 한국어도 자연스럽게 렌더된다.
    // 항상 화이트 배경 — daily 요약은 공유 / 인쇄 / 캡쳐 에 자주 쓰일
    // 의도라 다크 모드 자동 전환은 안 함. color-scheme 도 light 로 고정해
    // 스크롤바·input 등 native chrome 도 라이트로 뽑힌다.
    let css = r#"
:root {
  color-scheme: light;
  --bg: #f4f5f7;
  --bg-card: #ffffff;
  --ink: #14161a;
  --body: #2a2c33;
  --mute: #56585f;
  --stone: #8a8a92;
  --rule: #e7e8ec;
  --shadow: 0 1px 2px rgba(0,0,0,.04), 0 12px 32px rgba(15,23,42,.06);
  --code-bg: #f1f2f5;
  --code-ink: #c2185b;
  --tone-decision: #1f6feb;
  --tone-change:   #16a34a;
  --tone-learn:    #b45309;
  --tone-warn:     #dc2626;
  --tone-next:     #7c3aed;
  --tone-link:     #0891b2;
  --tone-default:  #6b7280;
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  font-family: -apple-system, BlinkMacSystemFont, 'Pretendard',
    'Apple SD Gothic Neo', 'Inter', 'Segoe UI', Roboto, sans-serif;
  background: var(--bg);
  color: var(--body);
  line-height: 1.75;
  font-size: 16px;
  letter-spacing: -0.005em;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}
.danbi-page {
  max-width: 780px;
  margin: 0 auto;
  padding: 56px 28px 96px;
}
.danbi-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 28px;
}
.danbi-mark {
  display: inline-grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: 8px;
  background: linear-gradient(135deg, #57c1ff 0%, #1f6feb 100%);
  color: #fff;
  font-weight: 800;
  font-size: 13px;
}
.danbi-title-block {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.danbi-eyebrow {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--stone);
  font-weight: 600;
}
.danbi-page h1 {
  font-size: 30px;
  font-weight: 700;
  letter-spacing: -0.02em;
  line-height: 1.2;
  color: var(--ink);
  margin: 0;
}
.danbi-meta {
  margin-left: auto;
  font-variant-numeric: tabular-nums;
  color: var(--stone);
  font-size: 12px;
}

.card {
  position: relative;
  background: var(--bg-card);
  border: 1px solid var(--rule);
  border-radius: 14px;
  padding: 22px 26px 24px;
  margin: 16px 0;
  box-shadow: var(--shadow);
  overflow: hidden;
}
.card::before {
  content: "";
  position: absolute;
  inset: 0 auto 0 0;
  width: 4px;
  background: var(--accent, var(--tone-default));
}
.card[data-tone="decision"] { --accent: var(--tone-decision); }
.card[data-tone="change"]   { --accent: var(--tone-change); }
.card[data-tone="learn"]    { --accent: var(--tone-learn); }
.card[data-tone="warn"]     { --accent: var(--tone-warn); }
.card[data-tone="next"]     { --accent: var(--tone-next); }
.card[data-tone="link"]     { --accent: var(--tone-link); }
.card[data-tone="default"]  { --accent: var(--tone-default); }

.card h2 {
  font-size: 17px;
  font-weight: 700;
  letter-spacing: -0.01em;
  line-height: 1.35;
  color: var(--ink);
  margin: 0 0 12px;
  padding-left: 8px;
  display: flex;
  align-items: center;
  gap: 8px;
}
.card h2::before {
  content: "";
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent, var(--tone-default));
  flex: 0 0 auto;
}
.card h3 {
  font-size: 14px;
  font-weight: 650;
  color: var(--ink);
  margin: 16px 0 6px;
  padding-left: 8px;
}
.card p {
  margin: 0 0 10px;
  padding-left: 8px;
}
.card ul {
  margin: 6px 0 10px;
  padding-left: 28px;
}
.card li {
  margin: 5px 0;
  position: relative;
}
.card li::marker { color: var(--accent, var(--tone-default)); }
.card strong {
  font-weight: 650;
  color: var(--ink);
}
.card code {
  background: var(--code-bg);
  color: var(--code-ink);
  padding: 1.5px 6px;
  border-radius: 5px;
  font-size: 0.88em;
  font-family: 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo,
    monospace;
}
.card blockquote {
  margin: 14px 0;
  padding: 10px 16px;
  border-left: 3px solid var(--accent, var(--tone-default));
  background: color-mix(in srgb, var(--accent, var(--tone-default)) 8%, transparent);
  border-radius: 0 8px 8px 0;
  color: var(--mute);
  font-size: 14.5px;
}

.danbi-footer {
  margin-top: 32px;
  text-align: center;
  font-size: 11px;
  letter-spacing: 0.4px;
  color: var(--stone);
}
.danbi-footer a { color: inherit; text-decoration: none; }
@media print {
  body { background: white; }
  .card { border: 1px solid #ddd; box-shadow: none; }
  .card::before { background: #888; }
}
"#;

    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"ko\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    html.push_str("<title>단비 · 요약</title>");
    html.push_str("<style>");
    html.push_str(css);
    html.push_str("</style>");
    html.push_str("</head><body>\n");
    html.push_str("<div class=\"danbi-page\">\n");
    html.push_str(&format!(
        "<div class=\"danbi-header\"><span class=\"danbi-mark\">단</span>\
         <div class=\"danbi-title-block\">\
         <span class=\"danbi-eyebrow\">단비 요약</span>\
         </div>\
         <span class=\"danbi-meta\">{}</span></div>\n",
        now,
    ));
    html.push_str(&body);
    html.push_str(
        "<div class=\"danbi-footer\">Generated by Danbi · 외부 AI 와 vault 의 다리</div>\n",
    );
    html.push_str("</div>\n</body></html>");
    html
}

#[tauri::command]
pub fn hide_popover(app: AppHandle) -> DanbiResult<()> {
    crate::popover::hide_popover_window(&app);
    Ok(())
}

#[tauri::command]
pub fn open_main_window(app: AppHandle) -> DanbiResult<()> {
    crate::popover::hide_popover_window(&app);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
        let _ = main.unminimize();
    }
    Ok(())
}

/// Set when the user explicitly requested a real quit (popover 종료
/// 버튼, tray Quit). Cmd+Q / macOS menu Quit don't set this flag, so
/// the run-loop's ExitRequested handler can tell them apart and deny
/// the implicit exits.
pub static QUIT_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Real quit — fully exits the process. Distinct from the main window's
/// CloseRequested handler which only hides the window (single-instance
/// tray app pattern). Wired into the popover's tiny "종료" button as the
/// user-explicit exit path.
#[tauri::command]
pub fn quit_app(app: AppHandle) -> DanbiResult<()> {
    QUIT_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
    app.exit(0);
    Ok(())
}

/// Open main window AND tell the React app to switch sidebar selection
/// to the given project. Used by the popover's project quick-shortcuts.
#[tauri::command]
pub fn open_project_in_main(app: AppHandle, project: String) -> DanbiResult<()> {
    crate::popover::hide_popover_window(&app);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
        let _ = main.unminimize();
    }
    use tauri::Emitter;
    let _ = app.emit(
        "danbi:select-project",
        serde_json::json!({ "project": project }),
    );
    Ok(())
}

/// Quick Capture 검색 결과를 클릭했을 때 메인 윈도우로 selection 을
/// 전달하고 vault 화면을 띄운다. capture popup 은 호출자가 별도로
/// 닫는다 (UI 분기 분리).
#[tauri::command]
pub fn capture_open_hit(
    app: AppHandle,
    project: String,
    domain: String,
) -> DanbiResult<()> {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
        let _ = main.unminimize();
    }
    // 메인 윈도우 React 가 listen 해서 selectDomain 호출. payload 형식은
    // 단비 store 의 selection 과 동일.
    use tauri::Emitter;
    let _ = app.emit(
        "danbi:open-doc",
        serde_json::json!({ "project": project, "domain": domain }),
    );
    Ok(())
}

#[tauri::command]
pub fn autostart_status(app: AppHandle) -> DanbiResult<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .is_enabled()
        .map_err(|e| DanbiError::Config(e.to_string()))
}

#[tauri::command]
pub fn autostart_set(app: AppHandle, enabled: bool) -> DanbiResult<()> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable()
            .map_err(|e| DanbiError::Config(e.to_string()))?;
    } else {
        mgr.disable()
            .map_err(|e| DanbiError::Config(e.to_string()))?;
    }
    Ok(())
}

#[tauri::command]
pub fn tray_badge_count(app: AppHandle) -> u32 {
    app.state::<crate::tray_badge::TrayBadgeState>().count() as u32
}

#[tauri::command]
pub fn tray_badge_reset(app: AppHandle) -> DanbiResult<()> {
    crate::tray_badge::clear_and_sync(&app);
    Ok(())
}

#[tauri::command]
pub fn tray_badge_set_enabled(app: AppHandle, enabled: bool) -> DanbiResult<()> {
    app.state::<crate::tray_badge::TrayBadgeState>()
        .set_enabled(enabled);
    crate::tray_badge::sync_tray_title(&app);
    Ok(())
}

// ---------- Ghost Links ----------

#[tauri::command]
pub fn ghost_list(project: String) -> DanbiResult<GhostStore> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    ghost_links::load(&vault_path, &project)
}

#[tauri::command]
pub async fn ghost_scan(project: String) -> DanbiResult<GhostStore> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    // 0.1 부터는 LLM 호출 = AI 연동 (embed provider) 의 가벼운 모델 재사용.
    // 별도 routing 모델 없이 ghost scan 도 같은 키로 동작.
    let (model, _kind) = resolve_summarize_model(&cfg).ok_or_else(|| {
        DanbiError::Config(
            "AI 연동이 꺼져있어요. Settings 에서 임베딩 provider 를 먼저 연결하세요.".into(),
        )
    })?;
    let provider = resolve_embed_provider(&cfg)?;
    ghost_links::scan_project(&vault_path, &project, provider.as_ref(), &model).await
}

#[tauri::command]
pub fn ghost_accept(project: String, id: String) -> DanbiResult<GhostStore> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;

    vcs::ensure_repo(&vault_path)?;
    let commit_before = vcs::snapshot(
        &vault_path,
        &format!("danbi: pre-ghost-accept · {project}"),
    )?;
    let store = ghost_links::accept(&vault_path, &project, &id)?;
    let commit_after = vcs::snapshot(
        &vault_path,
        &format!("danbi: ghost-accept · {project} · {id}"),
    )?;
    let _ = journal::append_log(
        &vault_path,
        &format!("**ghost-accept** `{project}` · id={id}"),
    );
    let event = HistoryEvent {
        ts: chrono::Local::now().to_rfc3339(),
        kind: "ghost-accept",
        project: Some(&project),
        domain: None,
        intent: None,
        user_message: None,
        summary: None,
        op: None,
        commit_before: commit_before.as_deref(),
        commit_after: commit_after.as_deref(),
    };
    let _ = journal::append_history(&vault_path, &event);
    Ok(store)
}

#[tauri::command]
pub async fn project_qa_ask(project: String, question: String) -> DanbiResult<QaAnswer> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let writer = cfg
        .models
        .writer
        .clone()
        .ok_or_else(|| DanbiError::Config("writer model missing".into()))?;
    let vault_path = require_vault(&cfg)?;
    let provider = resolve_provider(&cfg)?;
    let question_embedding = embed_query_for_search(&cfg, &question).await;
    project_qa::ask(
        &vault_path,
        &project,
        &question,
        question_embedding.as_deref(),
        provider.as_ref(),
        &writer,
    )
    .await
}

#[tauri::command]
pub fn dashboard_snapshot() -> DanbiResult<crate::dashboard::DashboardSnapshot> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::dashboard::snapshot(&vault_path)
}

/// Per-project activity overview for the home dashboard donut. Combines
/// commit count + MCP inbound calls/tokens within a rolling N-day window.
#[tauri::command]
pub fn project_activity_overview(
    days: Option<i64>,
) -> DanbiResult<crate::dashboard::ActivityOverview> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::dashboard::project_activity_overview(&vault_path, days.unwrap_or(30))
}

// ---------- MCP inbound dashboard (v0.4.0) -----------------------------
//
// Three resolutions matching the UI's drill-down:
//   - vault-wide (the big "오늘 단비에 X 토큰 저장됨" card)
//   - per-project (clicked row in the project list)
//   - per-domain (clicked row in the domain list)
//
// All three return a self-describing payload that includes the
// disclaimer string, so the renderer can't accidentally hide it.

#[tauri::command]
pub fn dashboard_mcp_inbound(
    range: String,
) -> DanbiResult<crate::mcp_inbound::VaultSummary> {
    let r = crate::mcp_inbound::Range::parse(&range);
    Ok(crate::mcp_inbound::summarize_vault(r))
}

#[tauri::command]
pub fn dashboard_mcp_inbound_project(
    project: String,
    range: String,
) -> DanbiResult<crate::mcp_inbound::ProjectDetail> {
    let r = crate::mcp_inbound::Range::parse(&range);
    Ok(crate::mcp_inbound::summarize_project(&project, r))
}

#[tauri::command]
pub fn dashboard_mcp_inbound_domain(
    project: String,
    domain: String,
    range: String,
) -> DanbiResult<crate::mcp_inbound::DomainDetail> {
    let r = crate::mcp_inbound::Range::parse(&range);
    Ok(crate::mcp_inbound::summarize_domain(&project, &domain, r))
}

/// Export every usage event as JSON to the supplied path. The frontend
/// presents the dialog and passes us the chosen path; we own the write
/// so we don't have to enable Tauri's `fs` plugin (every plugin we
/// add is one more attack-surface entry in the Tauri ACL).
#[tauri::command]
pub fn usage_export_json(path: String) -> DanbiResult<()> {
    let body = crate::usage::export_json()
        .map_err(|e| DanbiError::Config(format!("export failed: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| DanbiError::Config(format!("write {path}: {e}")))?;
    Ok(())
}

/// Same as `usage_export_json` but CSV.
#[tauri::command]
pub fn usage_export_csv(path: String) -> DanbiResult<()> {
    let body = crate::usage::export_csv()
        .map_err(|e| DanbiError::Config(format!("export failed: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| DanbiError::Config(format!("write {path}: {e}")))?;
    Ok(())
}

/// Run the retention sweep on demand. Normally fires automatically
/// during startup (see `lib.rs`); this command lets the user trigger
/// it from Settings if they're worried about the live log size.
#[tauri::command]
pub fn usage_retention_sweep() -> DanbiResult<usize> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let n = crate::usage::run_retention_sweep(cfg.usage.mcp_retention_days)
        .map_err(|e| DanbiError::Config(format!("retention sweep failed: {e}")))?;
    if n > 0 {
        crate::mcp_inbound::invalidate_cache();
    }
    Ok(n)
}

/// Toggle MCP inbound tracking from Settings. `true` re-enables a
/// disabled tracker; `false` causes future MCP writes to skip the
/// usage log entirely.
#[tauri::command]
pub fn usage_set_mcp_tracking(enabled: bool) -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    cfg.usage.mcp_tracking = enabled;
    config::save_config(&vault, &cfg)?;
    Ok(())
}

/// Update the MCP retention window. Negative or zero means "keep
/// forever". Triggers an immediate sweep so the user sees the file
/// shrink right after lowering the retention.
#[tauri::command]
pub fn usage_set_mcp_retention(days: i64) -> DanbiResult<usize> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    cfg.usage.mcp_retention_days = days;
    config::save_config(&vault, &cfg)?;
    let n = crate::usage::run_retention_sweep(days).unwrap_or(0);
    if n > 0 {
        crate::mcp_inbound::invalidate_cache();
    }
    Ok(n)
}

// ---------- Backup (mirror vault to external folder) ----------

#[tauri::command]
pub fn backup_now() -> DanbiResult<crate::backup::BackupReport> {
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let dest = cfg
        .backup
        .path
        .clone()
        .ok_or_else(|| DanbiError::Config("백업 경로가 설정되지 않았어요.".into()))?;

    let report = crate::backup::run(
        &vault_path,
        &PathBuf::from(&dest),
        &cfg.backup.exclude,
    )?;

    // Stamp the outcome so Settings can show "last run X minutes ago".
    cfg.backup.last_run_at = Some(chrono::Local::now().timestamp());
    cfg.backup.last_message = Some(format!(
        "{} copied · {} skipped · {} removed · {} ms",
        report.copied, report.skipped, report.removed, report.duration_ms
    ));
    config::save_config(&vault, &cfg)?;

    Ok(report)
}

#[tauri::command]
pub fn backup_validate_path(path: String) -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::backup::validate_destination(&vault_path, &PathBuf::from(path))
}

// ---------- Usage (token cost tracking) ----------

/// Returns the running month-to-date usage summary in KRW, computed from
/// the append-only usage log against the user's configured exchange rate.
/// Never errors for "no data yet" — we return an empty summary so the UI
/// can show zeros without special-casing.
#[tauri::command]
pub fn usage_month_to_date() -> DanbiResult<crate::pricing::UsageSummary> {
    let vault = default_vault_path()?;
    let krw = match config::load_config(&vault)? {
        Some(cfg) => cfg.usage.krw_per_usd,
        None => 1_400.0,
    };
    Ok(
        crate::pricing::month_to_date(krw)
            .unwrap_or_else(|_| crate::pricing::UsageSummary {
                from_ms: crate::pricing::current_month_start_ms(),
                to_ms: chrono::Utc::now().timestamp_millis() + 1,
                total_krw: 0.0,
                krw_per_usd: krw,
                by_role: Vec::new(),
                calls: 0,
            }),
    )
}

/// Update the USD → KRW conversion rate used for all price estimates.
/// Persisted to `config.json` so every subsequent summary uses the new
/// number.
#[tauri::command]
pub fn usage_set_rate(krw_per_usd: f64) -> DanbiResult<()> {
    if !(krw_per_usd.is_finite() && krw_per_usd > 0.0) {
        return Err(DanbiError::Config("환율은 0보다 큰 숫자여야 해요.".into()));
    }
    let vault = default_vault_path()?;
    let mut cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    cfg.usage.krw_per_usd = krw_per_usd;
    config::save_config(&vault, &cfg)?;
    Ok(())
}

/// Estimate the cost of a full reindex without actually running it. The
/// frontend uses this to show "약 ₩N" before the user confirms.
#[tauri::command]
pub fn vector_estimate_reindex(
    model_id: Option<String>,
) -> DanbiResult<VectorEstimateResponse> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let provider = resolve_embed_provider(&cfg)?;
    let model = resolve_embed_model(&cfg, provider.as_ref(), model_id);
    let estimate = if model.is_empty() {
        crate::vector::ReindexEstimate {
            total_files: 0,
            fresh_files: 0,
            pending_files: 0,
            pending_chars: 0,
            pending_tokens: 0,
            model: String::new(),
        }
    } else {
        crate::vector::estimate_reindex(&vault_path, &model)?
    };
    let krw_per_usd = cfg.usage.krw_per_usd;
    let krw = crate::pricing::estimate_call_krw(
        &estimate.model,
        estimate.pending_tokens,
        0,
        krw_per_usd,
    );
    Ok(VectorEstimateResponse {
        estimate,
        krw,
        krw_per_usd,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VectorEstimateResponse {
    pub estimate: crate::vector::ReindexEstimate,
    pub krw: f64,
    pub krw_per_usd: f64,
}

#[tauri::command]
pub fn vector_stats() -> DanbiResult<crate::vector::VectorIndexStats> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let idx = crate::vector::load(&vault_path)?;
    Ok(crate::vector::stats(&idx))
}

/// Emit a `vector:reindex_progress` Tauri event for every step pushed by
/// the reindex loop. The frontend listens on this and renders a progress
/// bar + "다음 호출까지 N초" countdown so users see why Gemini's free
/// tier appears to "stall" (it's pacing, not stuck).
fn emit_reindex_progress(app: &AppHandle, p: &crate::vector::ReindexProgress) {
    use tauri::Emitter;
    let _ = app.emit("vector:reindex_progress", p);
}

#[tauri::command]
pub async fn vector_reindex(
    app: AppHandle,
    model_id: Option<String>,
    batch_size: Option<usize>,
) -> DanbiResult<crate::vector::ReindexReport> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let provider = resolve_embed_provider(&cfg)?;
    let model = resolve_embed_model(&cfg, provider.as_ref(), model_id);
    if model.is_empty() {
        return Err(DanbiError::Config(
            "현재 provider 는 embedding 을 지원하지 않아요".into(),
        ));
    }
    let app_for_cb = app.clone();
    let cb = move |p: crate::vector::ReindexProgress| {
        emit_reindex_progress(&app_for_cb, &p);
    };
    let report = crate::vector::reindex(
        &vault_path,
        provider.as_ref(),
        &model,
        batch_size.unwrap_or(16),
        Some(&cb),
    )
    .await?;
    use tauri::Emitter;
    let _ = app.emit("vector:reindex_done", &report);
    Ok(report)
}

#[tauri::command]
pub async fn vector_reindex_project(
    app: AppHandle,
    project: String,
    model_id: Option<String>,
    batch_size: Option<usize>,
) -> DanbiResult<crate::vector::ReindexReport> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let provider = resolve_embed_provider(&cfg)?;
    let model = resolve_embed_model(&cfg, provider.as_ref(), model_id);
    if model.is_empty() {
        return Err(DanbiError::Config(
            "현재 provider 는 embedding 을 지원하지 않아요".into(),
        ));
    }
    let app_for_cb = app.clone();
    let cb = move |p: crate::vector::ReindexProgress| {
        emit_reindex_progress(&app_for_cb, &p);
    };
    let report = crate::vector::reindex_project(
        &vault_path,
        provider.as_ref(),
        &model,
        batch_size.unwrap_or(16),
        &project,
        Some(&cb),
    )
    .await?;
    use tauri::Emitter;
    let _ = app.emit("vector:reindex_done", &report);
    Ok(report)
}

#[tauri::command]
pub fn vector_clear() -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::vector::clear(&vault_path)
}

#[tauri::command]
pub async fn vector_search(
    query: String,
    limit: Option<usize>,
    model_id: Option<String>,
) -> DanbiResult<Vec<crate::vector::VectorHit>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let provider = resolve_embed_provider(&cfg)?;
    let model = resolve_embed_model(&cfg, provider.as_ref(), model_id);
    if model.is_empty() {
        return Ok(Vec::new());
    }
    let embeddings =
        crate::usage::with_role("embed", provider.embed(&model, &[query])).await?;
    let q = embeddings.into_iter().next().unwrap_or_default();
    let idx = crate::vector::load(&vault_path)?;
    Ok(crate::vector::query(&idx, &q, limit.unwrap_or(10)))
}

#[tauri::command]
pub fn reviews_list() -> DanbiResult<crate::reviews::ReviewStore> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::reviews::load(&vault_path)
}

#[tauri::command]
pub fn reviews_resolve(
    id: String,
    status: String,
) -> DanbiResult<crate::reviews::ReviewStore> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let next = match status.as_str() {
        "resolved" => crate::reviews::ReviewStatus::Resolved,
        "dismissed" => crate::reviews::ReviewStatus::Dismissed,
        "pending" => crate::reviews::ReviewStatus::Pending,
        _ => {
            return Err(DanbiError::Config(format!(
                "unknown review status: {status}"
            )))
        }
    };
    crate::reviews::resolve(&vault_path, &id, next)
}

#[tauri::command]
pub fn cache_clear() -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::cache::clear_all(&vault_path)
}

#[tauri::command]
pub fn project_context_status(
    project: String,
) -> DanbiResult<crate::project_context::ProjectContext> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    Ok(crate::project_context::load(&vault_path, &project))
}

#[tauri::command]
pub fn project_context_ensure(project: String) -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::project_context::ensure_templates(&vault_path, &project)
}

#[tauri::command]
pub fn build_graph(project: Option<String>) -> DanbiResult<crate::graph::GraphData> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::graph::build_graph(&vault_path, project.as_deref())
}

#[tauri::command]
pub async fn project_briefing(project: String, range: String) -> DanbiResult<BriefingResult> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let writer = cfg
        .models
        .writer
        .clone()
        .ok_or_else(|| DanbiError::Config("writer model missing".into()))?;
    let vault_path = require_vault(&cfg)?;
    let provider = resolve_provider(&cfg)?;
    briefing::build(&vault_path, &project, &range, provider.as_ref(), &writer).await
}

#[tauri::command]
pub fn ghost_reject(project: String, id: String) -> DanbiResult<GhostStore> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    ghost_links::reject(&vault_path, &project, &id)
}

#[tauri::command]
pub fn undo_last() -> DanbiResult<Option<String>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;

    let prev = vcs::undo_last(&vault_path)?;

    let event = HistoryEvent {
        ts: chrono::Local::now().to_rfc3339(),
        kind: "undo",
        project: None,
        domain: None,
        intent: None,
        user_message: None,
        summary: None,
        op: None,
        commit_before: None,
        commit_after: prev.as_deref(),
    };
    let _ = journal::append_history(&vault_path, &event);
    let _ = journal::append_log(
        &vault_path,
        &format!(
            "**undo** reverted to commit `{}`",
            prev.as_deref().unwrap_or("—")
        ),
    );
    Ok(prev)
}

/// Slug-ify a project name for use as a skill directory name. The
/// `~/.claude/skills/<dir>/` lookup wants ASCII-safe names; Korean and
/// other non-ASCII project names get hashed to a stable suffix so the
/// directory is always reproducible from the project name.
fn skill_dir_for(project: &str) -> String {
    let safe: String = project
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = safe.trim_matches('-').to_string();
    // If the project was entirely non-ASCII the trimmed string is empty —
    // append a short hash of the original so we still get a unique path.
    if trimmed.is_empty() {
        format!("danbi-{}", short_hash(project))
    } else if safe.contains("--") || trimmed.len() < project.chars().count() {
        // Some chars were replaced — append the hash so similarly-shaped
        // Korean names ("단비", "단비_v2") don't collide.
        format!("danbi-{}-{}", trimmed, short_hash(project))
    } else {
        format!("danbi-{trimmed}")
    }
}

fn short_hash(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:08x}", (h.finish() as u32))
}

/// Resolve the project's scoped MCP URL using the running config + the
/// project's stable `.danbi-id`. Falls back to a placeholder when MCP
/// isn't configured yet so the skill still installs cleanly.
fn project_mcp_url(vault_path: &std::path::Path, project: &str) -> String {
    let port = config::load_config(vault_path)
        .ok()
        .flatten()
        .map(|c| c.mcp.port)
        .unwrap_or(47921);
    match vault::ensure_project_id(vault_path, project) {
        Ok(id) => format!("http://127.0.0.1:{port}/mcp/{id}"),
        Err(_) => format!("http://127.0.0.1:{port}/mcp"),
    }
}

/// Install (or refresh) a per-project Danbi skill at
/// `~/.claude/skills/danbi-<slug>/SKILL.md`. Returns the absolute path
/// so the UI can surface where it landed.
///
/// Source of truth is `<vault>/Projects/<project>/SKILL.md`. If the
/// user hasn't customised the skill yet that file is seeded with the
/// default template so they have a real entry point to edit. Every
/// install/refresh then reads from vault, substitutes the dynamic
/// placeholders ({{PROJECT}}, {{MCP_URL}}), and writes the result to
/// `~/.claude/skills/`. Editing vault's SKILL.md and pressing 갱신
/// is the round-trip.
#[tauri::command]
pub fn install_skill(project: String) -> DanbiResult<String> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    let mcp_url = project_mcp_url(&vault_path, &project);

    // Seed vault's SKILL.md from the default template the first time.
    // The vault copy keeps placeholders so the user can edit it once and
    // every project's install gets the right substitutions.
    let vault_skill = vault::ensure_project_skill(&vault_path, &project)?;
    let template = std::fs::read_to_string(&vault_skill)
        .map_err(|e| DanbiError::Other(format!("read vault SKILL.md: {e}")))?;
    let rendered = template
        .replace("{{PROJECT}}", &project)
        .replace("{{MCP_URL}}", &mcp_url);

    let home = dirs::home_dir()
        .ok_or_else(|| DanbiError::Other("home directory not found".into()))?;
    let skill_dir = home
        .join(".claude")
        .join("skills")
        .join(skill_dir_for(&project));
    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| DanbiError::Other(format!("create skill dir: {e}")))?;
    let skill_md = skill_dir.join("SKILL.md");
    std::fs::write(&skill_md, rendered)
        .map_err(|e| DanbiError::Other(format!("write SKILL.md: {e}")))?;
    Ok(skill_md.to_string_lossy().to_string())
}

/// Whether THIS project's skill file exists. Used to swap the header
/// button between "Skill 설치" and "Skill 갱신".
#[tauri::command]
pub fn skill_status(project: String) -> DanbiResult<bool> {
    let home = match dirs::home_dir() {
        Some(p) => p,
        None => return Ok(false),
    };
    let skill_md = home
        .join(".claude")
        .join("skills")
        .join(skill_dir_for(&project))
        .join("SKILL.md");
    Ok(skill_md.exists())
}

// ---------- Goals ----------------------------------------------------------

#[tauri::command]
pub fn goals_list(
    project: String,
    include_archived: Option<bool>,
) -> DanbiResult<Vec<crate::goals::Goal>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    if include_archived.unwrap_or(false) {
        crate::goals::list_all(&vault_path, &project)
    } else {
        crate::goals::list_active(&vault_path, &project)
    }
}

#[tauri::command]
pub fn goals_add(
    project: String,
    title: String,
    note: Option<String>,
) -> DanbiResult<crate::goals::Goal> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::goals::add(&vault_path, &project, &title, note)
}

#[tauri::command]
pub fn goals_edit(
    project: String,
    id: String,
    title: Option<String>,
    note: Option<String>,
    clear_note: Option<bool>,
) -> DanbiResult<crate::goals::Goal> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    // `clear_note=true` overrides any provided note and wipes it.
    // Otherwise, Some(value) sets, None leaves untouched.
    let note_patch = if clear_note.unwrap_or(false) {
        Some(None)
    } else {
        note.map(Some)
    };
    crate::goals::edit(&vault_path, &project, &id, title, note_patch)
}

#[tauri::command]
pub fn goals_archive(project: String, id: String) -> DanbiResult<crate::goals::Goal> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::goals::archive(&vault_path, &project, &id)
}

#[tauri::command]
pub fn goals_unarchive(project: String, id: String) -> DanbiResult<crate::goals::Goal> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::goals::unarchive(&vault_path, &project, &id)
}

#[tauri::command]
pub fn goals_delete(project: String, id: String) -> DanbiResult<()> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::goals::delete(&vault_path, &project, &id)
}

/// 문서 변경 히스토리 — 현재 열린 doc 우측 패널에 노출되는 list.
/// git 커밋 메시지에서 op (upsert_item / replace_section / append / …)
/// 을 분류하고, upsert_item 의 경우 update vs add 모드까지 함께.
#[tauri::command]
pub fn doc_change_history(
    project: String,
    domain: String,
    limit: Option<usize>,
) -> DanbiResult<Vec<crate::vcs::DocChangeEntry>> {
    let vault = default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("config not found".into()))?;
    let vault_path = require_vault(&cfg)?;
    crate::vcs::doc_history(&vault_path, &project, &domain, limit.unwrap_or(40))
}
