use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use meshmend_stl::{load_binary_stl_with_options, LoadOptions};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod app;
mod input;

#[derive(Debug, Parser)]
#[command(name = "meshmend")]
#[command(about = "Native MeshMend STL inspection app")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[arg(long, value_names = ["STL", "PNG"], num_args = 2)]
    screenshot: Option<Vec<PathBuf>>,

    #[arg(long, value_name = "STL")]
    verify_render: Option<PathBuf>,

    #[arg(long, value_name = "STL")]
    verify_cross_section: Option<PathBuf>,

    #[arg(long, value_names = ["STL", "PNG"], num_args = 2)]
    cross_section_screenshot: Option<Vec<PathBuf>>,

    #[arg(value_name = "STL")]
    input: Option<PathBuf>,

    #[arg(long, hide = true)]
    smoke_window: bool,

    #[arg(long, hide = true)]
    smoke_pick_center: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    Inspect {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long)]
        parallel: bool,
    },
    Perf {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "JSON")]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    init_logging();

    let cli = Cli::parse();
    if let Some(values) = cli.screenshot {
        app::run_capture(values[0].clone(), Some(values[1].clone()))?;
        return Ok(());
    }
    if let Some(path) = cli.verify_render {
        app::run_capture(path, None)?;
        return Ok(());
    }
    if let Some(path) = cli.verify_cross_section {
        app::run_cross_section_capture(path, None)?;
        return Ok(());
    }
    if let Some(values) = cli.cross_section_screenshot {
        app::run_cross_section_capture(values[0].clone(), Some(values[1].clone()))?;
        return Ok(());
    }

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
        Some(Command::Perf { path, output }) => {
            app::run_perf(path, output)?;
        }
        None => {
            app::run_native(cli.input, cli.smoke_window, cli.smoke_pick_center)?;
        }
    }

    Ok(())
}

fn init_logging() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,wgpu=warn,naga=warn".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}
