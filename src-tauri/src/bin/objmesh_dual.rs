//! `objmesh_dual` — route-A dual training from the command line (issue #9).
//!
//! Trains the object and background datasets assembled by `objmesh_datasets`,
//! one after the other, and leaves the object and background PLYs in their own
//! output directories. The pipeline UI will call `pipeline::brush_dual` directly;
//! this binary exists so the same orchestration can be exercised and accepted
//! without the app, and so a failed long run can be reproduced in one command.
//!
//! ```text
//! objmesh_dual --engines <dir containing brush/> --out <dir> \
//!     [--object-dir NAME] [--background-dir NAME] \
//!     [--steps N] [--max-resolution N] [--invert-masks] \
//!     <dataset-object> <dataset-background>
//! ```

use std::path::PathBuf;

use ooo_splat::engines::brush::{self, BrushGpuLaunchPolicy, BrushTrainingOptions};
use ooo_splat::engines::health::{check_colmap_acceleration, EnginePaths};
use ooo_splat::pipeline::brush_dual::{plan_dual_training, run_dual_training, verify_dual_outputs};
use ooo_splat::presets::quality::BrushTrainingPreset;

const USAGE: &str = "usage: objmesh_dual --engines <dir> --out <dir> [--object-dir NAME] \
[--background-dir NAME] [--steps N] [--max-resolution N] [--invert-masks] \
<dataset-object> <dataset-background>";

struct Options {
    engines: PathBuf,
    out: PathBuf,
    object_dir: String,
    background_dir: String,
    steps: usize,
    max_resolution: u32,
    invert_masks: bool,
    datasets: Vec<PathBuf>,
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

fn parse() -> Options {
    let mut args = std::env::args().skip(1);
    let mut engines = None;
    let mut out = None;
    let mut object_dir = "dataset-object".to_string();
    let mut background_dir = "dataset-background".to_string();
    let mut steps = 300_usize;
    let mut max_resolution = 480_u32;
    let mut invert_masks = false;
    let mut datasets = Vec::new();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--engines" => engines = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--object-dir" => object_dir = args.next().unwrap_or_else(|| usage()),
            "--background-dir" => background_dir = args.next().unwrap_or_else(|| usage()),
            "--steps" => {
                steps = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            "--max-resolution" => {
                max_resolution = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            "--invert-masks" => invert_masks = true,
            other if other.starts_with("--") => usage(),
            other => datasets.push(PathBuf::from(other)),
        }
    }
    if datasets.len() != 2 {
        usage();
    }
    Options {
        engines: engines.unwrap_or_else(|| usage()),
        out: out.unwrap_or_else(|| usage()),
        object_dir,
        background_dir,
        steps,
        max_resolution,
        invert_masks,
        datasets,
    }
}

#[tokio::main]
async fn main() {
    std::process::exit(run(parse()).await);
}

async fn run(options: Options) -> i32 {
    let paths = EnginePaths::from_root(&options.engines);
    if let Err(error) = brush::require_verified_cli(&paths.brush) {
        eprintln!(
            "错误：Brush 可执行文件不可用（{}）：{error}",
            paths.brush.display()
        );
        return 1;
    }
    let jobs = match plan_dual_training(
        &options.datasets,
        &options.out,
        &options.object_dir,
        &options.background_dir,
    ) {
        Ok(jobs) => jobs,
        Err(error) => {
            eprintln!("错误：{error}");
            return 1;
        }
    };
    for job in &jobs {
        println!(
            "plan: {} <- {} -> {} / {}",
            job.polarity.label(),
            job.dataset.display(),
            job.output_directory.display(),
            job.export_name
        );
    }

    let preset = BrushTrainingPreset {
        total_steps: options.steps,
        max_resolution: options.max_resolution,
        refine_every: 200,
        max_splats: None,
        densification: None,
    };
    let acceleration = check_colmap_acceleration(&paths).await;
    let policy = BrushGpuLaunchPolicy::from_acceleration(&acceleration);
    println!("brush: {}", paths.brush.display());
    let manager = ooo_splat::process::ProcessManager::new();
    let invert_masks = options.invert_masks;

    let outcome = run_dual_training(&jobs, |job| {
        let started = std::time::Instant::now();
        println!(
            "train: {} <- {}",
            job.polarity.label(),
            job.dataset.display()
        );
        let policy = &policy;
        let brush_bin = &paths.brush;
        let manager = &manager;
        async move {
            let result = brush::train(
                brush_bin,
                &job.dataset,
                &job.output_directory,
                BrushTrainingOptions {
                    alpha_mode: None,
                    warm: None,
                    preset,
                    panorama: false,
                    log_path: job.output_directory.join("brush.log"),
                    gpu_launch_policy: policy,
                    export_name: &job.export_name,
                    invert_masks,
                },
                manager,
                None,
            )
            .await;
            match result {
                Ok(path) => {
                    println!(
                        "done: {} in {:.1}s -> {}",
                        job.polarity.label(),
                        started.elapsed().as_secs_f64(),
                        path.display()
                    );
                    Ok(path)
                }
                Err(error) => Err(error),
            }
        }
    })
    .await;

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("错误：{error}");
            return 1;
        }
    };
    if let Err(error) = verify_dual_outputs(&outcome) {
        eprintln!("错误：{error}");
        return 1;
    }
    println!(
        "ok: object={} background={}",
        outcome.object.display(),
        outcome.background.display()
    );
    0
}
