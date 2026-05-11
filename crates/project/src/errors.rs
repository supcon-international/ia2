use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("toml emit: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("project not found at {0}")]
    NotFound(String),

    #[error("project already exists at {0}")]
    AlreadyExists(String),

    #[error("invalid name '{0}': must be a single path segment, no slashes or dots")]
    InvalidName(String),

    #[error("application '{0}' not found")]
    AppNotFound(String),

    #[error("device '{0}' not found")]
    DeviceNotFound(String),
}
