use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use meshmend_project::MeshMendProject;
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

    #[arg(long, value_name = "STL")]
    verify_view_modes: Option<PathBuf>,

    #[arg(long, value_name = "STL")]
    verify_hit_stack: Option<PathBuf>,

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
    Analyze {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "JSON")]
        output: Option<PathBuf>,
    },
    Perf {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "JSON")]
        output: PathBuf,
    },
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    Validate {
        #[arg(value_name = "PROJECT")]
        path: PathBuf,
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
    if let Some(path) = cli.verify_view_modes {
        app::run_view_mode_verification(path)?;
        return Ok(());
    }
    if let Some(path) = cli.verify_hit_stack {
        app::run_hit_stack_verification(path)?;
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
        Some(Command::Analyze { path, output }) => {
            let parsed = load_binary_stl_with_options(
                &path,
                &LoadOptions {
                    parallel: true,
                    ..LoadOptions::default()
                },
            )?;
            let report = app::analyze_parsed_stl(&parsed);
            println!("file: {}", parsed.source_path.display());
            println!("defects: {}", report.defects.len());
            println!("components: {}", report.topology.component_count);
            println!("boundary loops: {}", report.topology.boundary_loop_count);
            println!(
                "non-manifold edges: {}",
                report.topology.non_manifold_edge_count
            );
            if let Some(output) = output {
                if let Some(parent) = output
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&output, serde_json::to_string_pretty(&report)?)?;
                println!("wrote: {}", output.display());
            }
        }
        Some(Command::Perf { path, output }) => {
            app::run_perf(path, output)?;
        }
        Some(Command::Project {
            command: ProjectCommand::Validate { path },
        }) => {
            let project = MeshMendProject::load_from_dir(&path)?;
            println!("project: {}", project.metadata.name);
            println!("source: {}", project.source.path.display());
            println!("revision: {}", project.current_revision);
            println!("operations: {}", project.operations.len());
            println!("exports: {}", project.exports.len());
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
