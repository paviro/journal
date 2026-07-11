use notema::{AppResult, cli};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> AppResult<()> {
    cli::run()
}
