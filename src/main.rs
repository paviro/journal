use clap::Parser;
use journal::{AppResult, config, storage, tui};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "journal")]
#[command(about = "Markdown terminal journal")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() -> AppResult<()> {
    let cli = Cli::parse();
    let config = config::load_or_setup(cli.config.as_deref())?;
    storage::ensure_workspace(&config.journal_root)?;

    tui::run(config)
}
