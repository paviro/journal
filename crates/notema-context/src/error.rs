use thiserror::Error;

pub type Result<T> = std::result::Result<T, ContextError>;

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("{0}")]
    Message(String),
    #[error("context provider request failed: {0}")]
    Http(#[from] ureq::Error),
    #[error("context provider response was invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("context provider I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(target_os = "linux")]
    #[error("device location service failed: {0}")]
    Dbus(#[from] zbus::Error),
}

impl ContextError {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
