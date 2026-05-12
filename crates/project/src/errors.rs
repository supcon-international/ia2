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

    #[error(
        "invalid name '{0}': each path segment must be non-empty, not start with '.', \
         and contain no backslashes or colons"
    )]
    InvalidName(String),

    #[error("folder '{0}' already exists")]
    FolderExists(String),

    #[error("application '{0}' not found")]
    AppNotFound(String),

    #[error("device '{0}' not found")]
    DeviceNotFound(String),

    #[error("edge '{0}' not found")]
    EdgeNotFound(String),
}
