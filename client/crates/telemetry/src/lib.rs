use anyhow::Result;
use std::io::IsTerminal;
use tracing_subscriber::EnvFilter;

pub fn init(level: &str) -> Result<()> {
    let filter = EnvFilter::try_new(level).or_else(|_| EnvFilter::try_new("info"))?;
    tracing_subscriber::fmt()
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(filter)
        .init();
    Ok(())
}
