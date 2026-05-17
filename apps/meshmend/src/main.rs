use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use meshmend_project::MeshMendProject;
use meshmend_stl::{load_binary_stl_with_options, LoadOptions};
use meshmend_worker_api::{discover_worker_binary, WorkerOperation, WorkerRequest, WorkerRunner};
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
    HoleFill {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "STL")]
        output: PathBuf,
    },
    LocalWrap {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "STL")]
        output: PathBuf,
        #[arg(long, value_name = "MODEL_UNITS")]
        voxel_size: Option<f64>,
    },
    Cut {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "STL")]
        output: PathBuf,
        #[arg(long, num_args = 3, value_names = ["X", "Y", "Z"])]
        normal: Vec<f64>,
        #[arg(long, default_value_t = 0.0)]
        offset: f64,
        #[arg(long, value_enum, default_value_t = CutKeepSide::Positive)]
        keep: CutKeepSide,
    },
    Remesh {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "STL")]
        output: PathBuf,
        #[arg(long, value_name = "MODEL_UNITS")]
        target_edge_length: Option<f64>,
        #[arg(long, value_name = "MICRONS")]
        target_microns: Option<f64>,
        #[arg(long, value_name = "MODEL_UNITS_PER_MM")]
        model_units_per_mm: Option<f64>,
    },
    Export {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "STL")]
        output: PathBuf,
        #[arg(long, value_name = "JSON")]
        report_json: Option<PathBuf>,
        #[arg(long, value_name = "MD")]
        report_md: Option<PathBuf>,
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
    WorkerSmoke {
        #[arg(value_name = "BACKEND")]
        backend: WorkerBackend,
        #[arg(value_name = "STL")]
        path: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    Validate {
        #[arg(value_name = "PROJECT")]
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum WorkerBackend {
    Cgal,
    Openvdb,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CutKeepSide {
    Positive,
    Negative,
}

impl CutKeepSide {
    fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }
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
        Some(Command::HoleFill { path, output }) => {
            let binary = discover_worker_binary("meshmend-cgal-worker").ok_or_else(|| {
                anyhow::anyhow!("CGAL worker was not found; run `just worker-build`")
            })?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            let response_path = PathBuf::from("outputs")
                .join("workers")
                .join("hole-fill-response.json");
            let request_path = PathBuf::from("outputs")
                .join("workers")
                .join("hole-fill-request.json");
            let mut request =
                WorkerRequest::new(WorkerOperation::HoleFill, path.clone(), response_path);
            request.output_mesh = Some(output.clone());
            request.preview = false;
            let result = WorkerRunner::new(binary).run(&request, &request_path)?;
            if !result.response.success {
                anyhow::bail!(
                    "hole fill worker failed: {}",
                    result
                        .response
                        .error
                        .unwrap_or_else(|| "unknown".to_string())
                );
            }
            let parsed = load_binary_stl_with_options(
                &output,
                &LoadOptions {
                    parallel: true,
                    ..LoadOptions::default()
                },
            )?;
            let report = app::analyze_parsed_stl(&parsed);
            println!("wrote: {}", output.display());
            println!("triangles: {}", parsed.stats.triangle_count);
            println!("boundary loops: {}", report.topology.boundary_loop_count);
            println!(
                "non-manifold edges: {}",
                report.topology.non_manifold_edge_count
            );
        }
        Some(Command::LocalWrap {
            path,
            output,
            voxel_size,
        }) => {
            let binary = discover_worker_binary("meshmend-openvdb-worker").ok_or_else(|| {
                anyhow::anyhow!("OpenVDB worker was not found; run `just worker-build`")
            })?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            let response_path = PathBuf::from("outputs")
                .join("workers")
                .join("local-wrap-response.json");
            let request_path = PathBuf::from("outputs")
                .join("workers")
                .join("local-wrap-request.json");
            let mut request =
                WorkerRequest::new(WorkerOperation::LocalSdfWrap, path.clone(), response_path);
            request.output_mesh = Some(output.clone());
            request.preview = false;
            request.target_edge_length = voxel_size;
            let result = WorkerRunner::new(binary).run(&request, &request_path)?;
            if !result.response.success {
                anyhow::bail!(
                    "local wrap worker failed: {}",
                    result
                        .response
                        .error
                        .unwrap_or_else(|| "unknown".to_string())
                );
            }
            let parsed = load_binary_stl_with_options(
                &output,
                &LoadOptions {
                    parallel: true,
                    ..LoadOptions::default()
                },
            )?;
            let report = app::analyze_parsed_stl(&parsed);
            println!("wrote: {}", output.display());
            println!("triangles: {}", parsed.stats.triangle_count);
            println!("boundary loops: {}", report.topology.boundary_loop_count);
            println!(
                "non-manifold edges: {}",
                report.topology.non_manifold_edge_count
            );
        }
        Some(Command::Cut {
            path,
            output,
            normal,
            offset,
            keep,
        }) => {
            if normal.len() != 3 {
                anyhow::bail!("cut --normal requires exactly three values");
            }
            let binary = discover_worker_binary("meshmend-cgal-worker").ok_or_else(|| {
                anyhow::anyhow!("CGAL worker was not found; run `just worker-build`")
            })?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            let response_path = PathBuf::from("outputs")
                .join("workers")
                .join("cut-response.json");
            let request_path = PathBuf::from("outputs")
                .join("workers")
                .join("cut-request.json");
            let mut request = WorkerRequest::new(WorkerOperation::Cut, path.clone(), response_path);
            request.output_mesh = Some(output.clone());
            request.preview = false;
            request.options = serde_json::json!({
                "plane_nx": normal[0],
                "plane_ny": normal[1],
                "plane_nz": normal[2],
                "plane_offset": offset,
                "keep": keep.as_str(),
            });
            let result = WorkerRunner::new(binary).run(&request, &request_path)?;
            if !result.response.success {
                anyhow::bail!(
                    "cut worker failed: {}",
                    result
                        .response
                        .error
                        .unwrap_or_else(|| "unknown".to_string())
                );
            }
            let parsed = load_binary_stl_with_options(
                &output,
                &LoadOptions {
                    parallel: true,
                    ..LoadOptions::default()
                },
            )?;
            let report = app::analyze_parsed_stl(&parsed);
            println!("wrote: {}", output.display());
            println!("triangles: {}", parsed.stats.triangle_count);
            println!("boundary loops: {}", report.topology.boundary_loop_count);
            println!(
                "non-manifold edges: {}",
                report.topology.non_manifold_edge_count
            );
        }
        Some(Command::Remesh {
            path,
            output,
            target_edge_length,
            target_microns,
            model_units_per_mm,
        }) => {
            let target_edge_length = match (target_edge_length, target_microns, model_units_per_mm)
            {
                (Some(target), _, _) => target,
                (None, Some(microns), Some(model_units_per_mm)) => {
                    microns / 1000.0 * model_units_per_mm
                }
                _ => anyhow::bail!(
                    "remesh requires --target-edge-length or both --target-microns and --model-units-per-mm"
                ),
            };
            let binary = discover_worker_binary("meshmend-cgal-worker").ok_or_else(|| {
                anyhow::anyhow!("CGAL worker was not found; run `just worker-build`")
            })?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            let response_path = PathBuf::from("outputs")
                .join("workers")
                .join("remesh-response.json");
            let request_path = PathBuf::from("outputs")
                .join("workers")
                .join("remesh-request.json");
            let mut request =
                WorkerRequest::new(WorkerOperation::Remesh, path.clone(), response_path);
            request.output_mesh = Some(output.clone());
            request.preview = false;
            request.target_edge_length = Some(target_edge_length);
            request.options = serde_json::json!({
                "target_edge_length": target_edge_length,
                "target_microns": target_microns,
                "model_units_per_mm": model_units_per_mm,
            });
            let result = WorkerRunner::new(binary).run(&request, &request_path)?;
            if !result.response.success {
                anyhow::bail!(
                    "remesh worker failed: {}",
                    result
                        .response
                        .error
                        .unwrap_or_else(|| "unknown".to_string())
                );
            }
            let parsed = load_binary_stl_with_options(
                &output,
                &LoadOptions {
                    parallel: true,
                    ..LoadOptions::default()
                },
            )?;
            let report = app::analyze_parsed_stl(&parsed);
            println!("wrote: {}", output.display());
            println!("target edge length: {target_edge_length:.6}");
            println!("triangles: {}", parsed.stats.triangle_count);
            println!(
                "average edge length: {:.6}",
                report.geometry.average_edge_length
            );
            println!("boundary loops: {}", report.topology.boundary_loop_count);
            println!(
                "non-manifold edges: {}",
                report.topology.non_manifold_edge_count
            );
        }
        Some(Command::Export {
            path,
            output,
            report_json,
            report_md,
        }) => {
            export_mesh_with_reports(&path, &output, report_json.as_deref(), report_md.as_deref())?;
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
        Some(Command::WorkerSmoke { backend, path }) => {
            let (binary_name, operation) = match backend {
                WorkerBackend::Cgal => ("meshmend-cgal-worker", WorkerOperation::CgalInspect),
                WorkerBackend::Openvdb => {
                    ("meshmend-openvdb-worker", WorkerOperation::OpenVdbInspect)
                }
            };
            let binary = discover_worker_binary(binary_name).ok_or_else(|| {
                anyhow::anyhow!(
                    "worker binary {binary_name} was not found; run `just worker-build`"
                )
            })?;
            let response_path = PathBuf::from("outputs")
                .join("workers")
                .join(format!("{binary_name}-response.json"));
            let request_path = PathBuf::from("outputs")
                .join("workers")
                .join(format!("{binary_name}-request.json"));
            let request = WorkerRequest::new(operation, path, response_path);
            let result = WorkerRunner::new(binary).run(&request, &request_path)?;
            println!("worker success: {}", result.response.success);
            println!(
                "input triangles: {}",
                result
                    .response
                    .metrics
                    .input_triangles
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            );
            println!("progress events: {}", result.progress.len());
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

fn export_mesh_with_reports(
    source: &std::path::Path,
    output: &std::path::Path,
    report_json: Option<&std::path::Path>,
    report_md: Option<&std::path::Path>,
) -> Result<()> {
    if same_file_path(source, output) {
        anyhow::bail!("export output must not overwrite the source STL");
    }
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, output)?;

    let parsed = load_binary_stl_with_options(
        output,
        &LoadOptions {
            parallel: true,
            ..LoadOptions::default()
        },
    )?;
    let report = app::analyze_parsed_stl(&parsed);
    let export_report = serde_json::json!({
        "version": 1,
        "source": source,
        "output": output,
        "validation": {
            "triangle_count": report.summary.triangle_count,
            "component_count": report.topology.component_count,
            "boundary_loop_count": report.topology.boundary_loop_count,
            "non_manifold_edge_count": report.topology.non_manifold_edge_count,
            "contained_internal_shell_count": report.topology.contained_component_count,
            "defect_count": report.defects.len(),
        },
        "analysis": &report,
    });

    if let Some(path) = report_json {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&export_report)?)?;
        println!("wrote json report: {}", path.display());
    }
    if let Some(path) = report_md {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, export_markdown_report(source, output, &report))?;
        println!("wrote markdown report: {}", path.display());
    }

    println!("exported: {}", output.display());
    println!("triangles: {}", report.summary.triangle_count);
    println!("defects: {}", report.defects.len());
    println!("components: {}", report.topology.component_count);
    println!("boundary loops: {}", report.topology.boundary_loop_count);
    println!(
        "non-manifold edges: {}",
        report.topology.non_manifold_edge_count
    );
    Ok(())
}

fn export_markdown_report(
    source: &std::path::Path,
    output: &std::path::Path,
    report: &meshmend_analysis::AnalysisReport,
) -> String {
    let mut markdown = String::new();
    markdown.push_str("# MeshMend Export Report\n\n");
    markdown.push_str(&format!("- Source: `{}`\n", source.display()));
    markdown.push_str(&format!("- Output: `{}`\n", output.display()));
    markdown.push_str(&format!("- Triangles: {}\n", report.summary.triangle_count));
    markdown.push_str(&format!("- Defects: {}\n", report.defects.len()));
    markdown.push_str(&format!(
        "- Components: {}\n",
        report.topology.component_count
    ));
    markdown.push_str(&format!(
        "- Boundary loops: {}\n",
        report.topology.boundary_loop_count
    ));
    markdown.push_str(&format!(
        "- Non-manifold edges: {}\n",
        report.topology.non_manifold_edge_count
    ));
    markdown.push_str(&format!(
        "- Contained internal shells: {}\n",
        report.topology.contained_component_count
    ));
    if !report.defects.is_empty() {
        markdown.push_str("\n## Findings\n\n");
        for defect in &report.defects {
            markdown.push_str(&format!(
                "- {:?} {:?}: {}\n",
                defect.kind, defect.severity, defect.recommendation
            ));
        }
    }
    markdown
}

fn same_file_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}
