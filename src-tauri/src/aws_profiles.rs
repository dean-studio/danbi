use crate::error::{DanbiError, DanbiResult};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize, Clone)]
pub struct AwsProfile {
    pub name: String,
    pub region: Option<String>,
    pub source: String, // "credentials" | "config"
}

#[derive(Debug, Serialize, Clone)]
pub struct AwsDetection {
    pub has_credentials_file: bool,
    pub has_config_file: bool,
    pub profiles: Vec<AwsProfile>,
}

fn aws_dir() -> DanbiResult<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| DanbiError::Config("home directory not found".into()))?;
    Ok(home.join(".aws"))
}

fn parse_ini(path: &std::path::Path) -> DanbiResult<Vec<(String, std::collections::HashMap<String, String>)>> {
    let text = std::fs::read_to_string(path)?;
    // We implement a minimal INI parser to avoid pulling in a heavier dep.
    let mut out: Vec<(String, std::collections::HashMap<String, String>)> = Vec::new();
    let mut current: Option<(String, std::collections::HashMap<String, String>)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(stripped) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(prev) = current.take() {
                out.push(prev);
            }
            current = Some((stripped.trim().to_string(), std::collections::HashMap::new()));
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if let Some((_, map)) = current.as_mut() {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    if let Some(prev) = current.take() {
        out.push(prev);
    }
    Ok(out)
}

/// Reads ~/.aws/credentials and ~/.aws/config, merges profile names,
/// and carries region hints from the config file.
pub fn detect_profiles() -> DanbiResult<AwsDetection> {
    let dir = aws_dir()?;
    let cred = dir.join("credentials");
    let conf = dir.join("config");

    let has_credentials_file = cred.exists();
    let has_config_file = conf.exists();

    let mut profiles: std::collections::BTreeMap<String, AwsProfile> =
        std::collections::BTreeMap::new();

    if has_credentials_file {
        for (section, _) in parse_ini(&cred)? {
            profiles.insert(
                section.clone(),
                AwsProfile {
                    name: section,
                    region: None,
                    source: "credentials".into(),
                },
            );
        }
    }

    if has_config_file {
        for (section, kv) in parse_ini(&conf)? {
            // ~/.aws/config uses "[profile foo]" except for "[default]"
            let name = section
                .strip_prefix("profile ")
                .unwrap_or(&section)
                .to_string();
            let region = kv.get("region").cloned();
            profiles
                .entry(name.clone())
                .and_modify(|p| {
                    if p.region.is_none() {
                        p.region = region.clone();
                    }
                })
                .or_insert(AwsProfile {
                    name,
                    region,
                    source: "config".into(),
                });
        }
    }

    Ok(AwsDetection {
        has_credentials_file,
        has_config_file,
        profiles: profiles.into_values().collect(),
    })
}
