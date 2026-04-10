use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "meshlinkd", about = "MeshLink control plane client")]
pub struct Cli {
    #[arg(long, default_value = "deploy/examples/client-config.toml")]
    pub config: PathBuf,
}

pub fn parse() -> Cli {
    Cli::parse()
}
