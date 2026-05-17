use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Parser)]
#[command(name = "meshmend")]
#[command(about = "Native MeshMend STL inspection app")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[arg(value_name = "STL")]
    input: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Inspect {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long)]
        parallel: bool,
    },
}

fn main() -> Result<()> {
    init_logging();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::Inspect { path, parallel }) => {
            let mode = if parallel { "parallel" } else { "serial" };
            println!("MeshMend inspect placeholder ({mode}): {}", path.display());
        }
        None => {
            if let Some(path) = cli.input {
                println!("MeshMend native app placeholder: {}", path.display());
            } else {
                println!("MeshMend native app placeholder");
            }
        }
    }

    Ok(())
}

fn init_logging() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "meshmend=info,wgpu=warn".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}
