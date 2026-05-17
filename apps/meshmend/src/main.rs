use std::path::PathBuf;

use crate::commands::ViewModeName;
use anyhow::Result;
use clap::{Parser, Subcommand};
use glam::Vec3;
use meshmend_geometry::{
    split_and_cap_mesh, CapDensity, CutMeshOptions, CutMeshResult, CutPlane, CutSide,
};
use meshmend_project::MeshMendProject;
use meshmend_stl::{load_binary_stl_with_options, write_binary_stl, LoadOptions};
use meshmend_worker_api::{discover_worker_binary, WorkerOperation, WorkerRequest, WorkerRunner};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod app;
mod commands;
mod icons;
mod input;
mod render_script;
mod scenario;
mod session;

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
        #[arg(long, value_name = "STL")]
        output_positive: Option<PathBuf>,
        #[arg(long, value_name = "STL")]
        output_negative: Option<PathBuf>,
        #[arg(long, num_args = 3, value_names = ["X", "Y", "Z"])]
        normal: Vec<f64>,
        #[arg(long, default_value_t = 0.0)]
        offset: f64,
        #[arg(long, value_enum, default_value_t = CutKeepSide::Positive)]
        keep: CutKeepSide,
        #[arg(long)]
        smooth_cap: bool,
        #[arg(long, value_name = "JSON")]
        report_json: Option<PathBuf>,
        #[arg(long, value_name = "MD")]
        report_md: Option<PathBuf>,
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
    Render {
        #[arg(value_name = "STL")]
        path: PathBuf,
        #[arg(long, value_name = "PNG")]
        output: PathBuf,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 800)]
        height: u32,
        #[arg(long, value_enum, default_value = "rendered")]
        view: ViewModeName,
        #[arg(long, value_name = "JSON")]
        camera: Option<PathBuf>,
        #[arg(long, value_name = "JSON")]
        state: Option<PathBuf>,
    },
    Scenario {
        #[arg(value_name = "SCENARIO_JSON")]
        path: PathBuf,
        #[arg(long, value_name = "DIR")]
        output_dir: PathBuf,
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
            output_positive,
            output_negative,
            normal,
            offset,
            keep,
            smooth_cap,
            report_json,
            report_md,
        }) => {
            if normal.len() != 3 {
                anyhow::bail!("cut --normal requires exactly three values");
            }
            let parsed = load_binary_stl_with_options(
                &path,
                &LoadOptions {
                    parallel: true,
                    ..LoadOptions::default()
                },
            )?;
            let triangles = parsed
                .chunks
                .iter()
                .flat_map(|chunk| chunk.triangles.iter().copied())
                .collect::<Vec<_>>();
            let normal = Vec3::new(normal[0] as f32, normal[1] as f32, normal[2] as f32);
            let plane = CutPlane {
                normal: normal.normalize_or_zero(),
                offset: offset as f32,
            };
            if plane.normal.length_squared() <= f32::EPSILON {
                anyhow::bail!("cut --normal requires a non-zero vector");
            }
            let result = split_and_cap_mesh(
                &triangles,
                plane,
                CutMeshOptions {
                    weld_tolerance: parsed.stats.bounds.radius().max(1.0) * 1.0e-6,
                    target_edge_length: None,
                    cap_density: CapDensity::Automatic,
                    smooth_cap,
                },
            )?;
            let side = match keep {
                CutKeepSide::Positive => CutSide::Positive,
                CutKeepSide::Negative => CutSide::Negative,
            };
            let piece = result
                .pieces
                .iter()
                .find(|piece| piece.side == side)
                .expect("cut result should contain requested side");
            write_binary_stl(&output, &piece.triangles)?;
            if let Some(path) = output_positive.as_ref() {
                let positive = result
                    .pieces
                    .iter()
                    .find(|piece| piece.side == CutSide::Positive)
                    .expect("cut result should contain positive side");
                write_binary_stl(path, &positive.triangles)?;
            }
            if let Some(path) = output_negative.as_ref() {
                let negative = result
                    .pieces
                    .iter()
                    .find(|piece| piece.side == CutSide::Negative)
                    .expect("cut result should contain negative side");
                write_binary_stl(path, &negative.triangles)?;
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
            println!("cut loops: {}", result.loops.len());
            println!("cap triangles: {}", piece.cap_triangle_count);
            println!("target cap edge length: {:.6}", result.target_edge_length);
            println!("boundary loops: {}", report.topology.boundary_loop_count);
            println!(
                "non-manifold edges: {}",
                report.topology.non_manifold_edge_count
            );
            write_cut_reports(
                &path,
                &output,
                output_positive.as_deref(),
                output_negative.as_deref(),
                plane,
                side,
                &result,
                &report,
                report_json.as_deref(),
                report_md.as_deref(),
            )?;
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
        Some(Command::Render {
            path,
            output,
            width,
            height,
            view,
            camera,
            state,
        }) => {
            render_script::run_render_command(path, output, width, height, view, camera, state)?;
        }
        Some(Command::Scenario { path, output_dir }) => {
            render_script::run_scenario(path, output_dir)?;
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

#[allow(clippy::too_many_arguments)]
fn write_cut_reports(
    source: &std::path::Path,
    output: &std::path::Path,
    output_positive: Option<&std::path::Path>,
    output_negative: Option<&std::path::Path>,
    plane: CutPlane,
    kept_side: CutSide,
    cut: &CutMeshResult,
    report: &meshmend_analysis::AnalysisReport,
    report_json: Option<&std::path::Path>,
    report_md: Option<&std::path::Path>,
) -> Result<()> {
    if report_json.is_none() && report_md.is_none() {
        return Ok(());
    }
    let pieces = cut
        .pieces
        .iter()
        .map(|piece| {
            serde_json::json!({
                "side": cut_side_label(piece.side),
                "triangles": piece.triangles.len(),
                "cap_triangles": piece.cap_triangle_count,
                "bounds": {
                    "min": [piece.bounds.min.x, piece.bounds.min.y, piece.bounds.min.z],
                    "max": [piece.bounds.max.x, piece.bounds.max.y, piece.bounds.max.z],
                },
            })
        })
        .collect::<Vec<_>>();
    let cut_report = serde_json::json!({
        "version": 1,
        "source": source,
        "output": output,
        "output_positive": output_positive,
        "output_negative": output_negative,
        "kept_side": cut_side_label(kept_side),
        "cut": {
            "plane": {
                "normal": [plane.normal.x, plane.normal.y, plane.normal.z],
                "offset": plane.offset,
            },
            "loop_count": cut.loops.len(),
            "loops": cut.loops.iter().map(|cut_loop| serde_json::json!({
                "vertices": cut_loop.vertices.len(),
                "closed": cut_loop.closed,
                "length": cut_loop.length,
            })).collect::<Vec<_>>(),
            "target_cap_edge_length": cut.target_edge_length,
            "warnings": cut.warnings,
        },
        "pieces": pieces,
        "validation": {
            "triangle_count": report.summary.triangle_count,
            "component_count": report.topology.component_count,
            "boundary_loop_count": report.topology.boundary_loop_count,
            "non_manifold_edge_count": report.topology.non_manifold_edge_count,
            "defect_count": report.defects.len(),
        },
    });

    if let Some(path) = report_json {
        write_text_report(path, &serde_json::to_string_pretty(&cut_report)?)?;
        println!("wrote json report: {}", path.display());
    }
    if let Some(path) = report_md {
        write_text_report(
            path,
            &cut_markdown_report(
                source,
                output,
                output_positive,
                output_negative,
                plane,
                kept_side,
                cut,
                report,
            ),
        )?;
        println!("wrote markdown report: {}", path.display());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cut_markdown_report(
    source: &std::path::Path,
    output: &std::path::Path,
    output_positive: Option<&std::path::Path>,
    output_negative: Option<&std::path::Path>,
    plane: CutPlane,
    kept_side: CutSide,
    cut: &CutMeshResult,
    report: &meshmend_analysis::AnalysisReport,
) -> String {
    let mut markdown = String::new();
    markdown.push_str("# MeshMend Cut Report\n\n");
    markdown.push_str(&format!("- Source: `{}`\n", source.display()));
    markdown.push_str(&format!("- Output: `{}`\n", output.display()));
    markdown.push_str(&format!("- Kept side: `{}`\n", cut_side_label(kept_side)));
    if let Some(path) = output_positive {
        markdown.push_str(&format!("- Positive side: `{}`\n", path.display()));
    }
    if let Some(path) = output_negative {
        markdown.push_str(&format!("- Negative side: `{}`\n", path.display()));
    }
    markdown.push_str(&format!(
        "- Plane normal: `[{:.6}, {:.6}, {:.6}]`\n",
        plane.normal.x, plane.normal.y, plane.normal.z
    ));
    markdown.push_str(&format!("- Plane offset: `{:.6}`\n", plane.offset));
    markdown.push_str(&format!("- Cut loops: {}\n", cut.loops.len()));
    markdown.push_str(&format!(
        "- Target cap edge length: `{:.6}`\n",
        cut.target_edge_length
    ));
    for piece in &cut.pieces {
        markdown.push_str(&format!(
            "- {} side: {} triangles, {} cap triangles\n",
            cut_side_label(piece.side),
            piece.triangles.len(),
            piece.cap_triangle_count
        ));
    }
    markdown.push_str(&format!(
        "- Boundary loops after export: {}\n",
        report.topology.boundary_loop_count
    ));
    markdown.push_str(&format!(
        "- Non-manifold edges after export: {}\n",
        report.topology.non_manifold_edge_count
    ));
    if !cut.warnings.is_empty() {
        markdown.push_str("\n## Warnings\n\n");
        for warning in &cut.warnings {
            markdown.push_str(&format!("- {warning}\n"));
        }
    }
    markdown
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

fn write_text_report(path: &std::path::Path, contents: &str) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

fn cut_side_label(side: CutSide) -> &'static str {
    match side {
        CutSide::Positive => "positive",
        CutSide::Negative => "negative",
    }
}

fn same_file_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}
