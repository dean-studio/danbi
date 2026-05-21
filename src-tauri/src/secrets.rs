use crate::error::DanbiResult;
use keyring::Entry;

const SERVICE: &str = "com.danbi.app";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ManualCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

fn entry(key: &str) -> DanbiResult<Entry> {
    Entry::new(SERVICE, key).map_err(Into::into)
}

/// Stores the manual credentials as a single JSON blob in the macOS Keychain.
pub fn set_manual_credentials(profile_label: &str, creds: &ManualCredentials) -> DanbiResult<()> {
    let blob = serde_json::to_string(creds)?;
    let e = entry(profile_label)?;
    e.set_password(&blob)?;
    Ok(())
}

pub fn get_manual_credentials(profile_label: &str) -> DanbiResult<Option<ManualCredentials>> {
    let e = entry(profile_label)?;
    match e.get_password() {
        Ok(blob) => Ok(Some(serde_json::from_str(&blob)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_manual_credentials(profile_label: &str) -> DanbiResult<()> {
    let e = entry(profile_label)?;
    match e.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

// ---- Generic API key storage (used by non-AWS providers) ----
//
// API keys are single-string secrets (unlike manual AWS creds which need three
// fields), so they get their own set/get pair. The `ref` convention used by
// `ProviderConfig::Nvidia.api_key_ref` is `"keychain:<label>"` — only the part
// after the colon is passed to these helpers.

fn label_from_ref(key_ref: &str) -> &str {
    key_ref
        .strip_prefix("keychain:")
        .unwrap_or(key_ref)
}

pub fn set_api_key(key_ref: &str, api_key: &str) -> DanbiResult<()> {
    let e = entry(label_from_ref(key_ref))?;
    e.set_password(api_key)?;
    Ok(())
}

pub fn get_api_key(key_ref: &str) -> DanbiResult<Option<String>> {
    let e = entry(label_from_ref(key_ref))?;
    match e.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_api_key(key_ref: &str) -> DanbiResult<()> {
    let e = entry(label_from_ref(key_ref))?;
    match e.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
