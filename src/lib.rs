pub mod config;
pub mod markdown;
pub mod storage;
pub mod tui;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
