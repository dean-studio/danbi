use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum DanbiError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("keyring: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("aws: {0}")]
    Aws(String),
    #[error("config: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for DanbiError {
    fn from(e: anyhow::Error) -> Self {
        DanbiError::Other(e.to_string())
    }
}

impl Serialize for DanbiError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type DanbiResult<T> = Result<T, DanbiError>;
