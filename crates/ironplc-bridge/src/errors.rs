use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("parse error: {0}")]
    Parse(String),

    #[error("analyze error: {0}")]
    Analyze(String),

    #[error("codegen error: {0}")]
    Codegen(String),
}
