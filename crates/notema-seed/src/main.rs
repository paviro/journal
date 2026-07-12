use anyhow::Result;
use clap::Parser;
use notema_seed::SeedConfig;
use notema_storage::JournalStore;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(about = "Generate development data for a Notema journal store")]
struct Args {
    /// Directory containing the journals.
    #[arg(long, value_name = "DIR")]
    root: PathBuf,

    /// Device-local config directory used for encryption keys.
    #[arg(long, value_name = "DIR")]
    config_dir: PathBuf,

    /// Journal to fill; created when it does not exist.
    #[arg(long, default_value = "Sample")]
    journal: String,

    /// Number of entries to generate.
    #[arg(long, default_value_t = 750)]
    count: usize,

    /// Spread creation dates across this many days.
    #[arg(long, default_value_t = 1_095)]
    days: i64,

    /// Seed for a reproducible data set.
    #[arg(long)]
    seed: Option<u64>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let store = JournalStore::new(&args.root, &args.config_dir);
    store.ensure()?;
    let created = notema_seed::generate(
        &store,
        &SeedConfig {
            journal: args.journal.clone(),
            count: args.count,
            days: args.days,
            seed: args.seed,
        },
    )?;
    println!("Generated {created} entries in journal {:?}.", args.journal);
    Ok(())
}
