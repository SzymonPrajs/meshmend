use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use meshmend_stl::{load_binary_stl_with_options, LoadOptions};
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
            let parsed = load_binary_stl_with_options(
                &path,
                &LoadOptions {
                    parallel,
                    ..LoadOptions::default()
                },
            )?;
            println!("file: {}", parsed.source_path.display());
            println!("triangles: {}", parsed.stats.triangle_count);
            println!("vertices: {}", parsed.stats.vertex_position_count);
            println!("source bytes: {}", parsed.stats.source_bytes);
            println!("chunks: {}", parsed.chunks.len());
            println!(
                "bounds min: {:.6}, {:.6}, {:.6}",
                parsed.stats.bounds.min.x, parsed.stats.bounds.min.y, parsed.stats.bounds.min.z
            );
            println!(
                "bounds max: {:.6}, {:.6}, {:.6}",
                parsed.stats.bounds.max.x, parsed.stats.bounds.max.y, parsed.stats.bounds.max.z
            );
            println!(
                "map ms: {:.3}",
                parsed.timings.map_file.as_secs_f64() * 1000.0
            );
            println!(
                "validate ms: {:.3}",
                parsed.timings.validate.as_secs_f64() * 1000.0
            );
            println!(
                "parse ms: {:.3}",
                parsed.timings.parse.as_secs_f64() * 1000.0
            );
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
