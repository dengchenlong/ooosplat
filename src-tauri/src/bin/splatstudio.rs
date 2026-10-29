use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ooo_splat::{
    engines::{ffmpeg::extract_selected_frames, ffprobe::probe_video},
    error::{Result, SplatError},
    pipeline::runner::{default_engine_paths, PipelineRunner},
    presets::Quality,
    process::ProcessManager,
    video::{analyze_image_sequence, create_image_plan, prepare_image_sequence},
};

#[derive(Debug, Parser)]
#[command(name = "splatstudio", version, about = "OOOSplat local pipeline CLI")]
struct Cli {
    /// Override the bundled engine directory (also supports OOOSPLAT_ENGINE_DIR).
    #[arg(long, global = true)]
    engine_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, clap::Args)]
struct PolicyArgs {
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    auto_retry: bool,
    /// Fraction of source frames for the first extraction (0.001..=1).
    #[arg(long)]
    frame_sampling_ratio: Option<f64>,
}
impl PolicyArgs {
    fn policy(&self) -> Result<ooo_splat::pipeline::generation_policy::GenerationPolicy> {
        ooo_splat::pipeline::generation_policy::GenerationPolicy {
            auto_retry_enabled: self.auto_retry,
            frame_sampling_ratio: self.frame_sampling_ratio,
            ..Default::default()
        }
        .validate()
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate FFmpeg, FFprobe, CPU COLMAP and Brush.
    Health,
    /// Read video metadata or image-sequence information.
    Probe { input: PathBuf },
    /// Show the video frame plan or image-sequence plan.
    Plan {
        #[command(flatten)]
        policy: PolicyArgs,
        input: PathBuf,
        #[arg(long, value_enum, default_value_t = Quality::Balanced)]
        quality: Quality,
    },
    /// Prepare video frames or an image sequence and optional COLMAP masks.
    Extract {
        #[command(flatten)]
        policy: PolicyArgs,
        input: PathBuf,
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = Quality::Balanced)]
        quality: Quality,
    },
    /// Run the end-to-end pipeline after all fixed engine CLIs are verified.
    Generate {
        #[command(flatten)]
        policy: PolicyArgs,
        input: PathBuf,
        /// Override the remembered projects root (useful for diagnostics).
        #[arg(long)]
        projects_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = Quality::Balanced)]
        quality: Quality,
        #[arg(long, value_enum, default_value_t = ooo_splat::video::panorama::VideoProjection::Auto)]
        video_projection: ooo_splat::video::panorama::VideoProjection,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();
    if let Err(error) = execute(Cli::parse()).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn execute(cli: Cli) -> Result<()> {
    let engines = default_engine_paths(cli.engine_dir);
    match cli.command {
        Commands::Health => {
            println!(
                "{}",
                serde_json::to_string_pretty(&engines.check_all().await)?
            );
        }
        Commands::Probe { input } => {
            if input.is_dir() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&analyze_image_sequence(&input)?)?
                );
            } else {
                let video =
                    probe_video(&engines.ffprobe, &input, None, &ProcessManager::new()).await?;
                println!("{}", serde_json::to_string_pretty(&video)?);
            }
        }
        Commands::Plan {
            input,
            quality,
            policy,
        } => {
            let policy = policy.policy()?;
            let plan = if input.is_dir() {
                create_image_plan(&analyze_image_sequence(&input)?, &quality.preset())
            } else {
                let video =
                    probe_video(&engines.ffprobe, &input, None, &ProcessManager::new()).await?;
                policy.frame_plan(&video, quality)
            };
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        Commands::Extract {
            policy,
            input,
            output,
            quality,
        } => {
            let policy = policy.policy()?;
            let masks = output
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("masks");
            let colmap_frames = output
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("colmap_frames");
            if input.is_dir() {
                let extraction = prepare_image_sequence(&input, &output, &colmap_frames, &masks)?;
                println!(
                    "prepared {} images in {} and {} masks in {}",
                    extraction.image_count,
                    output.display(),
                    extraction.mask_count,
                    masks.display()
                );
                return Ok(());
            }
            ensure_engine(&engines.ffprobe)?;
            ensure_engine(&engines.ffmpeg)?;
            let video = probe_video(&engines.ffprobe, &input, None, &ProcessManager::new()).await?;
            let plan = policy.frame_plan(&video, quality);
            let (w, h) = if video.rotation.rem_euclid(180) == 90 {
                (video.height, video.width)
            } else {
                (video.width, video.height)
            };
            let resolution = policy.resolution(quality, None, w, h, true);
            let extraction = extract_selected_frames(
                &engines.ffmpeg,
                &input,
                &output,
                &colmap_frames,
                &masks,
                &plan.selected_frames,
                video.has_alpha,
                Some((resolution.working_width, resolution.working_height)),
                None,
                &ProcessManager::new(),
                None,
            )
            .await?;
            if extraction.has_alpha {
                println!(
                    "extracted {} RGBA frames to {} and {} masks to {}",
                    extraction.frame_count,
                    output.display(),
                    extraction.mask_count,
                    masks.display()
                );
            } else {
                println!(
                    "extracted {} frames to {}",
                    extraction.frame_count,
                    output.display()
                );
            }
        }
        Commands::Generate {
            policy,
            input,
            projects_root,
            quality,
            video_projection,
        } => {
            let runner = PipelineRunner::new(engines, |event| {
                eprintln!(
                    "{:>6.2}% {:?}: {}",
                    event.progress, event.stage, event.message
                );
            })
            .with_video_projection(video_projection)
            .with_generation_policy(Some(policy.policy()?));
            let result = match projects_root {
                Some(root) => {
                    runner
                        .generate_for_diagnostics(&input, quality, &root)
                        .await?
                }
                None => {
                    let root = ooo_splat::project::catalog::load_settings()
                        .await?
                        .projects_root;
                    runner.generate(&input, quality, &root).await?
                }
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

fn ensure_engine(path: &std::path::Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(SplatError::EngineMissing(path.display().to_string()))
    }
}
