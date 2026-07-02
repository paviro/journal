pub mod cli;
pub mod config;
pub mod crypto;
pub mod feelings;
pub mod markdown;
pub mod migrate;
pub mod storage;
pub mod tui;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
