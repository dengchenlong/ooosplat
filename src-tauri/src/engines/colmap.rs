use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use crate::{
    error::{Result, SplatError},
    process::{ProcessManager, ProcessObserver, ProcessSpec},
};

mod vocabulary;

pub const LOCKED_VERSION: &str = "4.2.1";
pub const LOCKED_COMMIT: &str = "bd1fcf654d2dd8fefa1466999c190a246f83f4b9";

pub fn is_locked_build(help: &str) -> bool {
    let short_commit = &LOCKED_COMMIT[..7];
    [LOCKED_COMMIT, short_commit].into_iter().any(|commit| {
        let marker = format!("COLMAP {LOCKED_VERSION} (Commit {commit}");
        help.match_indices(&marker).any(|(index, _)| {
            help[index + marker.len()..]
                .chars()
                .next()
                .is_some_and(|character| character.is_whitespace() || character == ')')
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColmapCliFamily {
    Legacy39,
    Modern4,
}

impl ColmapCliFamily {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Legacy39 => "COLMAP 3.9 CLI",
            Self::Modern4 => "COLMAP 4.x CLI",
        }
    }
}

pub fn detect_cli_family(feature_help: &str, matching_help: &str) -> Option<ColmapCliFamily> {
    if feature_help.contains("--FeatureExtraction.use_gpu")
        && matching_help.contains("--FeatureMatching.use_gpu")
    {
        Some(ColmapCliFamily::Modern4)
    } else if feature_help.contains("--SiftExtraction.use_gpu")
        && matching_help.contains("--SiftMatching.use_gpu")
    {
        Some(ColmapCliFamily::Legacy39)
    } else {
        None
    }
}

async fn command_help(
    executable: &Path,
    command_name: &str,
    manager: &ProcessManager,
) -> Result<String> {
    let output = manager
        .run(ProcessSpec {
            executable: executable.to_path_buf(),
            args: vec![command_name.into(), "-h".into()],
            working_directory: executable.parent().map(Path::to_path_buf),
            log_path: None,
            observer: None,
        })
        .await?;
    if !output.success {
        return Err(SplatError::UnsupportedEngine(format!(
            "COLMAP {command_name} -h 退出码 {:?}",
            output.exit_code
        )));
    }
    Ok(format!("{}\n{}", output.stdout, output.stderr))
}

async fn feature_gpu_options(
    executable: &Path,
    manager: &ProcessManager,
) -> Result<(&'static str, &'static str)> {
    let help = command_help(executable, "feature_extractor", manager).await?;
    if help.contains("--FeatureExtraction.use_gpu") {
        Ok((
            "--FeatureExtraction.use_gpu",
            "--FeatureExtraction.gpu_index",
        ))
    } else if help.contains("--SiftExtraction.use_gpu") {
        Ok(("--SiftExtraction.use_gpu", "--SiftExtraction.gpu_index"))
    } else {
        Err(SplatError::UnsupportedEngine(
            "COLMAP feature_extractor 不支持已知的 SIFT GPU 参数".into(),
        ))
    }
}

async fn matching_gpu_options(
    executable: &Path,
    matcher: &str,
    manager: &ProcessManager,
) -> Result<(&'static str, &'static str)> {
    let help = command_help(executable, matcher, manager).await?;
    if help.contains("--FeatureMatching.use_gpu") {
        Ok(("--FeatureMatching.use_gpu", "--FeatureMatching.gpu_index"))
    } else if help.contains("--SiftMatching.use_gpu") {
        Ok(("--SiftMatching.use_gpu", "--SiftMatching.gpu_index"))
    } else {
        Err(SplatError::UnsupportedEngine(format!(
            "COLMAP {matcher} 不支持已知的 SIFT GPU 参数"
        )))
    }
}

pub fn require_verified_cli(executable: &Path) -> Result<()> {
    if executable.is_file() {
        Ok(())
    } else {
        Err(SplatError::EngineMissing(executable.display().to_string()))
    }
}

pub(crate) async fn run_colmap(
    executable: &Path,
    mut args: Vec<OsString>,
    working_directory: &Path,
    log_path: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
) -> Result<()> {
    let camera_config = working_directory.join("panorama-camera.json");
    if args.first().is_some_and(|a| a == "feature_extractor") && camera_config.is_file() {
        let config: crate::video::panorama::PanoramaCheckpoint =
            serde_json::from_slice(&tokio::fs::read(camera_config).await?)?;
        apply_panorama_camera(&mut args, &config);
    }
    let operation = args
        .first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .unwrap_or_default();
    let output = manager
        .run(ProcessSpec {
            executable: executable.to_path_buf(),
            args,
            working_directory: Some(working_directory.to_path_buf()),
            log_path: Some(log_path),
            observer,
        })
        .await?;
    if output.success {
        Ok(())
    } else {
        Err(SplatError::EngineProcess(Box::new(
            crate::failure::EngineProcessFailure::from_output(
                crate::pipeline::PipelineEngine::Colmap,
                &operation,
                &output,
            ),
        )))
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_mapper_with_ceres_fallback(
    executable: &Path,
    caspar_args: Vec<OsString>,
    mut ceres_args: Vec<OsString>,
    output: &Path,
    working_directory: &Path,
    log_path: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    selection: &super::colmap_ba::BaSelection,
) -> Result<()> {
    let result = run_colmap(
        executable,
        caspar_args,
        working_directory,
        log_path.clone(),
        manager,
        observer.clone(),
    )
    .await;
    let Some(index) = selection.gpu_index else {
        return result;
    };
    let caspar_error = match result {
        Ok(()) => return Ok(()),
        Err(error) if should_retry_mapper_with_ceres(selection, &error) => error,
        Err(error) => return Err(error),
    };

    super::colmap_ba::disable_after_runtime_failure(
        executable,
        index,
        "Caspar mapper process failed; Ceres fallback activated",
    )
    .await;
    append_log(
        &log_path,
        &format!(
            "[OOOSplat] Global Caspar mapper failed; resetting mapper output and retrying once with local/global Ceres: {caspar_error}\n"
        ),
    )
    .await?;
    tracing::warn!(
        "Global Caspar mapper failed on GPU {}; retrying once with Ceres: {}",
        index,
        caspar_error
    );
    reset_mapper_output(output).await?;
    super::colmap_ba::append_explicit_ceres_options(&mut ceres_args);
    match run_colmap(
        executable,
        ceres_args,
        working_directory,
        log_path.clone(),
        manager,
        observer,
    )
    .await
    {
        Ok(()) => {
            append_log(
                &log_path,
                "[OOOSplat] Ceres fallback completed successfully; Caspar is disabled for this engine/GPU for the remainder of the application process.\n",
            )
            .await?;
            Ok(())
        }
        // Cancellation/start/IO failures retain their own type; an earlier
        // Caspar failure must not override the final attempt's outcome.
        Err(error) => Err(error.with_previous_attempt(caspar_error)),
    }
}

async fn reset_mapper_output(output: &Path) -> Result<()> {
    if output.exists() {
        tokio::fs::remove_dir_all(output).await?;
    }
    tokio::fs::create_dir_all(output).await?;
    Ok(())
}

fn should_retry_mapper_with_ceres(
    selection: &super::colmap_ba::BaSelection,
    error: &SplatError,
) -> bool {
    selection.gpu_index.is_some()
        && matches!(error, SplatError::Process(_) | SplatError::EngineProcess(_))
}

async fn append_log(path: &Path, message: &str) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?
        .write_all(message.as_bytes())
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn extract_features(
    executable: &Path,
    database: &Path,
    images: &Path,
    masks: Option<&Path>,
    max_image_size: Option<u32>,
    max_features: Option<u32>,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
    camera_params: Option<&str>,
) -> Result<()> {
    let (use_gpu_option, gpu_index_option) = feature_gpu_options(executable, manager).await?;
    run_colmap(
        executable,
        feature_extraction_args(
            database,
            images,
            masks,
            gpu_index,
            use_gpu_option,
            gpu_index_option,
            max_image_size,
            max_features,
            None,
            camera_params,
        ),
        database.parent().unwrap_or(images),
        log,
        manager,
        observer,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn extract_features_quality_v2(
    executable: &Path,
    database: &Path,
    images: &Path,
    masks: Option<&Path>,
    image_list: Option<&Path>,
    max_image_size: u32,
    max_features: u32,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
    camera_params: Option<&str>,
) -> Result<()> {
    let (use_gpu_option, gpu_index_option) = feature_gpu_options(executable, manager).await?;
    run_colmap(
        executable,
        feature_extraction_args(
            database,
            images,
            masks,
            gpu_index,
            use_gpu_option,
            gpu_index_option,
            Some(max_image_size),
            Some(max_features),
            image_list,
            camera_params,
        ),
        database.parent().unwrap_or(images),
        log,
        manager,
        observer,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn extract_incremental_features(
    executable: &Path,
    database: &Path,
    images: &Path,
    image_list: &Path,
    existing_camera_id: u32,
    masks: Option<&Path>,
    max_image_size: Option<u32>,
    max_features: Option<u32>,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
) -> Result<()> {
    let (use_gpu_option, gpu_index_option) = feature_gpu_options(executable, manager).await?;
    run_colmap(
        executable,
        incremental_feature_extraction_args(
            database,
            images,
            image_list,
            existing_camera_id,
            masks,
            gpu_index,
            use_gpu_option,
            gpu_index_option,
            max_image_size,
            max_features,
        ),
        database.parent().unwrap_or(images),
        log,
        manager,
        observer,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn match_sequential(
    executable: &Path,
    database: &Path,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
    loop_detection: bool,
) -> Result<()> {
    let vocab_tree = if loop_detection {
        Some(vocabulary::prepare(executable, manager.child_token()).await?)
    } else {
        None
    };
    let (use_gpu_option, gpu_index_option) =
        matching_gpu_options(executable, "sequential_matcher", manager).await?;
    run_colmap(
        executable,
        sequential_matching_args(
            database,
            gpu_index,
            use_gpu_option,
            gpu_index_option,
            vocab_tree.as_deref(),
        ),
        database.parent().unwrap_or(Path::new(".")),
        log,
        manager,
        observer,
    )
    .await
}

pub async fn match_exhaustive(
    executable: &Path,
    database: &Path,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
) -> Result<()> {
    let (use_gpu_option, gpu_index_option) =
        matching_gpu_options(executable, "exhaustive_matcher", manager).await?;
    run_colmap(
        executable,
        matching_args(
            "exhaustive_matcher",
            database,
            gpu_index,
            use_gpu_option,
            gpu_index_option,
        ),
        database.parent().unwrap_or(Path::new(".")),
        log,
        manager,
        observer,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn match_pairs(
    executable: &Path,
    database: &Path,
    pair_list: &Path,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
) -> Result<()> {
    let (use_gpu_option, gpu_index_option) =
        matching_gpu_options(executable, "matches_importer", manager).await?;
    let mut args = matching_args(
        "matches_importer",
        database,
        gpu_index,
        use_gpu_option,
        gpu_index_option,
    );
    args.extend([
        "--match_list_path".into(),
        pair_list.into(),
        "--match_type".into(),
        "pairs".into(),
    ]);
    run_colmap(
        executable,
        args,
        database.parent().unwrap_or(Path::new(".")),
        log,
        manager,
        observer,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
fn feature_extraction_args(
    database: &Path,
    images: &Path,
    masks: Option<&Path>,
    gpu_index: Option<u32>,
    use_gpu_option: &str,
    gpu_index_option: &str,
    max_image_size: Option<u32>,
    max_features: Option<u32>,
    image_list: Option<&Path>,
    camera_params: Option<&str>,
) -> Vec<OsString> {
    let mut args = vec![
        "feature_extractor".into(),
        "--database_path".into(),
        database.into(),
        "--image_path".into(),
        images.into(),
        "--ImageReader.camera_model".into(),
        "SIMPLE_RADIAL".into(),
        "--ImageReader.single_camera".into(),
        "1".into(),
        use_gpu_option.into(),
        (if gpu_index.is_some() { "1" } else { "0" }).into(),
    ];
    append_feature_extraction_limits(&mut args, use_gpu_option, max_image_size, max_features);
    if let Some(index) = gpu_index {
        args.push(gpu_index_option.into());
        args.push(index.to_string().into());
    }
    if let Some(masks) = masks {
        args.push("--ImageReader.mask_path".into());
        args.push(masks.into());
    }
    if let Some(image_list) = image_list {
        args.push("--image_list_path".into());
        args.push(image_list.into());
    }
    if let Some(params) = camera_params {
        args.extend(["--ImageReader.camera_params".into(), params.into()]);
    }
    args
}

fn append_feature_extraction_limits(
    args: &mut Vec<OsString>,
    use_gpu_option: &str,
    max_image_size: Option<u32>,
    max_features: Option<u32>,
) {
    if let Some(max_image_size) = max_image_size {
        let max_image_size_option = if use_gpu_option.starts_with("--FeatureExtraction") {
            "--FeatureExtraction.max_image_size"
        } else {
            "--SiftExtraction.max_image_size"
        };
        args.extend([
            max_image_size_option.into(),
            max_image_size.to_string().into(),
        ]);
    }
    if let Some(max_features) = max_features {
        args.extend([
            "--SiftExtraction.max_num_features".into(),
            max_features.to_string().into(),
        ]);
    }
}

#[allow(clippy::too_many_arguments)]
fn incremental_feature_extraction_args(
    database: &Path,
    images: &Path,
    image_list: &Path,
    existing_camera_id: u32,
    masks: Option<&Path>,
    gpu_index: Option<u32>,
    use_gpu_option: &str,
    gpu_index_option: &str,
    max_image_size: Option<u32>,
    max_features: Option<u32>,
) -> Vec<OsString> {
    let mut args = vec![
        "feature_extractor".into(),
        "--database_path".into(),
        database.into(),
        "--image_path".into(),
        images.into(),
        "--image_list_path".into(),
        image_list.into(),
        "--ImageReader.existing_camera_id".into(),
        existing_camera_id.to_string().into(),
        use_gpu_option.into(),
        (if gpu_index.is_some() { "1" } else { "0" }).into(),
    ];
    append_feature_extraction_limits(&mut args, use_gpu_option, max_image_size, max_features);
    if let Some(index) = gpu_index {
        args.push(gpu_index_option.into());
        args.push(index.to_string().into());
    }
    if let Some(masks) = masks {
        args.push("--ImageReader.mask_path".into());
        args.push(masks.into());
    }
    args
}

fn sequential_matching_args(
    database: &Path,
    gpu_index: Option<u32>,
    use_gpu_option: &str,
    gpu_index_option: &str,
    vocab_tree: Option<&Path>,
) -> Vec<OsString> {
    let mut args = matching_args(
        "sequential_matcher",
        database,
        gpu_index,
        use_gpu_option,
        gpu_index_option,
    );
    args.extend([
        OsString::from("--SequentialMatching.overlap"),
        OsString::from("10"),
        OsString::from("--SequentialMatching.loop_detection"),
        OsString::from(if vocab_tree.is_some() { "1" } else { "0" }),
    ]);
    if let Some(path) = vocab_tree {
        args.extend([
            "--SequentialMatching.loop_detection_period".into(),
            "10".into(),
            "--SequentialMatching.loop_detection_num_images".into(),
            "50".into(),
            "--SequentialMatching.vocab_tree_path".into(),
            path.into(),
        ]);
    }
    args
}

fn matching_args(
    matcher: &str,
    database: &Path,
    gpu_index: Option<u32>,
    use_gpu_option: &str,
    gpu_index_option: &str,
) -> Vec<OsString> {
    let mut args = vec![
        matcher.into(),
        "--database_path".into(),
        database.into(),
        use_gpu_option.into(),
        (if gpu_index.is_some() { "1" } else { "0" }).into(),
    ];
    if let Some(index) = gpu_index {
        args.push(gpu_index_option.into());
        args.push(index.to_string().into());
    }
    args
}

#[allow(clippy::too_many_arguments)]
pub async fn map(
    executable: &Path,
    database: &Path,
    images: &Path,
    output: &Path,
    allow_two_view_tracks: bool,
    automatic_optimization: bool,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
) -> Result<()> {
    tokio::fs::create_dir_all(output).await?;
    let ceres_args = mapper_args(database, images, None, output, allow_two_view_tracks);
    let (args, selection) = with_ba(
        executable,
        database,
        ceres_args.clone(),
        gpu_index,
        automatic_optimization,
        &log,
    )
    .await?;
    run_mapper_with_ceres_fallback(
        executable,
        args,
        ceres_args,
        output,
        database.parent().unwrap_or(output),
        log,
        manager,
        observer,
        &selection,
    )
    .await
}

/// Continues the Incremental Mapper from an existing sparse model. The
/// baseline model is never overwritten; COLMAP writes the continued model to
/// a separate output directory so the caller can safely roll back.
#[allow(clippy::too_many_arguments)]
pub async fn map_from_existing(
    executable: &Path,
    database: &Path,
    images: &Path,
    input_model: &Path,
    output: &Path,
    automatic_optimization: bool,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
) -> Result<()> {
    tokio::fs::create_dir_all(output).await?;
    let ceres_args = mapper_args(database, images, Some(input_model), output, false);
    let (args, selection) = with_ba(
        executable,
        database,
        ceres_args.clone(),
        gpu_index,
        automatic_optimization,
        &log,
    )
    .await?;
    run_mapper_with_ceres_fallback(
        executable,
        args,
        ceres_args,
        output,
        database.parent().unwrap_or(output),
        log,
        manager,
        observer,
        &selection,
    )
    .await
}

fn mapper_args(
    database: &Path,
    images: &Path,
    input_model: Option<&Path>,
    output: &Path,
    allow_two_view_tracks: bool,
) -> Vec<OsString> {
    let mut args = vec![
        "mapper".into(),
        "--database_path".into(),
        database.into(),
        "--image_path".into(),
        images.into(),
    ];
    if let Some(input_model) = input_model {
        args.push("--input_path".into());
        args.push(input_model.into());
        args.push("--Mapper.multiple_models".into());
        args.push("0".into());
    }
    args.push("--output_path".into());
    args.push(output.into());
    if allow_two_view_tracks {
        args.push("--Mapper.tri_ignore_two_view_tracks".into());
        args.push("0".into());
    }
    args
}

#[allow(clippy::too_many_arguments)]
pub async fn map_incremental(
    executable: &Path,
    database: &Path,
    images: &Path,
    input: &Path,
    output: &Path,
    image_list: &Path,
    automatic_optimization: bool,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu_index: Option<u32>,
) -> Result<()> {
    tokio::fs::create_dir_all(output).await?;
    let ceres_args = incremental_mapper_args(database, images, input, output, image_list);
    let (args, selection) = with_ba(
        executable,
        database,
        ceres_args.clone(),
        gpu_index,
        automatic_optimization,
        &log,
    )
    .await?;
    run_mapper_with_ceres_fallback(
        executable,
        args,
        ceres_args,
        output,
        database.parent().unwrap_or(output),
        log,
        manager,
        observer,
        &selection,
    )
    .await
}

async fn with_ba(
    executable: &Path,
    database: &Path,
    mut args: Vec<OsString>,
    gpu_index: Option<u32>,
    automatic_optimization: bool,
    log: &Path,
) -> Result<(Vec<OsString>, super::colmap_ba::BaSelection)> {
    // The marker is generated from the persisted pipeline checkpoint, never
    // from a CLI version string. All mapper paths, including Bridge, share it.
    if database
        .parent()
        .is_some_and(|p| p.join("panorama-camera.json").is_file())
    {
        super::colmap_ba::append_explicit_ceres_options(&mut args);
        args.extend([
            "--Mapper.init_min_tri_angle".into(),
            "1".into(),
            "--Mapper.init_max_forward_motion".into(),
            "1".into(),
        ]);
        let selection = super::colmap_ba::BaSelection {gpu_index:None, detail:"Panorama: explicit local/global CERES; no Caspar; initial triangulation >= 1 degree".into()};
        append_log(log, &format!("[OOOSplat] {}\n", selection.detail)).await?;
        return Ok((args, selection));
    }
    let selection = super::colmap_ba::select_for_mapper(
        executable,
        database,
        gpu_index,
        automatic_optimization,
    )
    .await;
    super::colmap_ba::append_mapper_options(&mut args, &selection);
    append_log(log, &format!("[OOOSplat] {}\n", selection.detail)).await?;
    tracing::info!("{}", selection.detail);
    Ok((args, selection))
}

fn apply_panorama_camera(
    args: &mut Vec<OsString>,
    config: &crate::video::panorama::PanoramaCheckpoint,
) {
    if let Some(i) = args.iter().position(|a| a == "--ImageReader.camera_model") {
        args[i + 1] = "EQUIRECTANGULAR".into();
        args.extend([
            "--ImageReader.camera_params".into(),
            format!("{},{}", config.sfm_width, config.sfm_height).into(),
        ]);
    }
}

fn incremental_mapper_args(
    database: &Path,
    images: &Path,
    input: &Path,
    output: &Path,
    image_list: &Path,
) -> Vec<OsString> {
    vec![
        "mapper".into(),
        "--database_path".into(),
        database.into(),
        "--image_path".into(),
        images.into(),
        "--input_path".into(),
        input.into(),
        "--output_path".into(),
        output.into(),
        "--Mapper.image_list_path".into(),
        image_list.into(),
        "--Mapper.fix_existing_frames".into(),
        "1".into(),
        "--Mapper.ba_refine_focal_length".into(),
        "0".into(),
        "--Mapper.ba_refine_principal_point".into(),
        "0".into(),
        "--Mapper.ba_refine_extra_params".into(),
        "0".into(),
    ]
}

pub struct ReshootFeatureOptions<'a> {
    pub images: &'a Path,
    pub list: &'a Path,
    pub masks: Option<&'a Path>,
    pub panorama_dimensions: Option<(u32, u32)>,
    pub existing_camera_id: Option<u32>,
    pub max_size: u32,
    pub max_features: u32,
}

pub async fn extract_reshoot_group(
    executable: &Path,
    database: &Path,
    options: ReshootFeatureOptions<'_>,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu: Option<u32>,
) -> Result<()> {
    let (use_gpu, gpu_option) = feature_gpu_options(executable, manager).await?;
    let focal = if options.panorama_dimensions.is_none() && options.existing_camera_id.is_none() {
        crate::video::temporary_png_focal_workaround::detect(
            executable,
            manager,
            crate::video::temporary_png_focal_workaround::Request {
                images: options.images,
                image_list: Some(options.list),
                ordinary_images: true,
                camera_model: "SIMPLE_RADIAL",
                single_camera: true,
                explicit_params: None,
                existing_camera_id: None,
                calibrated: false,
            },
        )
        .await
    } else {
        None
    };
    let params = options
        .panorama_dimensions
        .map(|(w, h)| format!("{w},{h}"))
        .or_else(|| focal.as_ref().map(|f| f.camera_params()));
    let mut args = if let Some(camera_id) = options.existing_camera_id {
        incremental_feature_extraction_args(
            database,
            options.images,
            options.list,
            camera_id,
            options.masks,
            gpu,
            use_gpu,
            gpu_option,
            Some(options.max_size),
            Some(options.max_features),
        )
    } else {
        feature_extraction_args(
            database,
            options.images,
            options.masks,
            gpu,
            use_gpu,
            gpu_option,
            Some(options.max_size),
            Some(options.max_features),
            Some(options.list),
            params.as_deref(),
        )
    };
    if options.panorama_dimensions.is_some() {
        let index = args
            .iter()
            .position(|a| a == "--ImageReader.camera_model")
            .expect("model argument");
        args[index + 1] = "EQUIRECTANGULAR".into();
    }
    run_colmap(
        executable,
        args,
        database.parent().unwrap_or(options.images),
        log,
        manager,
        observer,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn match_reshoot_candidates(
    executable: &Path,
    database: &Path,
    query_list: &Path,
    candidates: u32,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
    gpu: Option<u32>,
) -> Result<()> {
    let vocab = vocabulary::prepare(executable, manager.child_token()).await?;
    let (use_gpu, gpu_option) =
        matching_gpu_options(executable, "vocab_tree_matcher", manager).await?;
    let mut args = matching_args("vocab_tree_matcher", database, gpu, use_gpu, gpu_option);
    args.extend([
        "--VocabTreeMatching.vocab_tree_path".into(),
        vocab.into(),
        "--VocabTreeMatching.match_list_path".into(),
        query_list.into(),
        "--VocabTreeMatching.num_images".into(),
        candidates.to_string().into(),
    ]);
    run_colmap(
        executable,
        args,
        database.parent().unwrap_or(Path::new(".")),
        log,
        manager,
        observer,
    )
    .await
}

/// Query only the immutable registered source images, not other new images.
/// The retriever has no output-file option; consume bounded per-query native
/// log records before ProcessManager truncates its diagnostic capture.
#[allow(clippy::too_many_arguments)]
pub async fn retrieve_reshoot_pairs(
    executable: &Path,
    database: &Path,
    old_names: &[String],
    new_names: &[String],
    candidates: u32,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
) -> Result<Vec<(String, String)>> {
    use crate::process::ProcessUpdate;
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };
    let directory = database.parent().unwrap_or(Path::new("."));
    let old_list = directory.join("retrieval-source.txt");
    let new_list = directory.join("retrieval-query.txt");
    tokio::fs::write(&old_list, old_names.join("\n")).await?;
    tokio::fs::write(&new_list, new_names.join("\n")).await?;
    let vocab = vocabulary::prepare(executable, manager.child_token()).await?;
    let old: HashSet<_> = old_names.iter().cloned().collect();
    let new: HashSet<_> = new_names.iter().cloned().collect();
    let pairs = Arc::new(Mutex::new((None::<String>, 0u32, Vec::new())));
    let output = pairs.clone();
    let collector: ProcessObserver = Arc::new(move |update| {
        if let ProcessUpdate::Line { line, .. } = &update {
            let mut state = output.lock().expect("retrieval collector");
            if let Some((_, rest)) = line.split_once("Querying for image ") {
                state.0 = rest
                    .rsplit_once(" [")
                    .map(|(name, _)| name.to_owned())
                    .filter(|name| new.contains(name));
                state.1 = 0;
            } else if let Some((_, rest)) = line.split_once("image_name=") {
                if let Some((name, _)) = rest.rsplit_once(", score=") {
                    if old.contains(name) && state.1 < candidates {
                        if let Some(query) = state.0.clone() {
                            state.2.push((name.to_owned(), query));
                            state.1 += 1;
                        }
                    }
                }
            }
        }
        if let Some(observer) = &observer {
            observer(update);
        }
    });
    run_colmap(
        executable,
        vec![
            "vocab_tree_retriever".into(),
            "--database_path".into(),
            database.into(),
            "--vocab_tree_path".into(),
            vocab.into(),
            "--database_image_list_path".into(),
            old_list.into(),
            "--query_image_list_path".into(),
            new_list.into(),
            "--num_images".into(),
            candidates.to_string().into(),
        ],
        directory,
        log,
        manager,
        Some(collector),
    )
    .await?;
    let result = std::mem::take(&mut pairs.lock().expect("retrieval collector").2);
    if result.is_empty() && !new_names.is_empty() && !old_names.is_empty() {
        return Err(SplatError::Process(
            "COLMAP retrieval returned no verifiable source candidates".into(),
        ));
    }
    Ok(result)
}

pub struct ReshootMapperOptions<'a> {
    pub images: &'a Path,
    pub input: Option<&'a Path>,
    pub output: &'a Path,
    pub image_list: &'a Path,
    pub fixed_cameras: Option<&'a Path>,
    pub panorama: bool,
}

pub async fn map_reshoot(
    executable: &Path,
    database: &Path,
    options: ReshootMapperOptions<'_>,
    log: PathBuf,
    manager: &ProcessManager,
    observer: Option<ProcessObserver>,
) -> Result<()> {
    tokio::fs::create_dir_all(options.output).await?;
    let mut args = mapper_args(
        database,
        options.images,
        options.input,
        options.output,
        false,
    );
    args.extend([
        "--Mapper.image_list_path".into(),
        options.image_list.into(),
        "--Mapper.fix_existing_frames".into(),
        if options.input.is_some() {
            "1".into()
        } else {
            "0".into()
        },
        "--Mapper.ba_refine_principal_point".into(),
        "0".into(),
    ]);
    if let Some(list) = options.fixed_cameras {
        args.extend(["--Mapper.constant_camera_list_path".into(), list.into()]);
    }
    if options.panorama {
        args.extend([
            "--Mapper.init_min_tri_angle".into(),
            "1".into(),
            "--Mapper.init_max_forward_motion".into(),
            "1".into(),
        ]);
    }
    super::colmap_ba::append_explicit_ceres_options(&mut args);
    run_colmap(
        executable,
        args,
        database.parent().unwrap_or(options.images),
        log,
        manager,
        observer,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires the local real PNG and bundled COLMAP; feature extraction only in an isolated database"]
    async fn native_reshoot_features_confirm_reused_and_independent_camera_ids() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let source = repo.join(".tmp/focal-diagnosis-20261019/png/images/003A9246.png");
        let original_hash = crate::video::panorama::fingerprint(&source).unwrap();
        let dir = tempfile::tempdir_in(repo.join(".cache")).unwrap();
        let images = dir.path().join("images");
        std::fs::create_dir(&images).unwrap();
        for name in ["original.png", "same.png", "different.png"] {
            std::fs::copy(&source, images.join(name)).unwrap();
        }
        let executable = repo.join("engines/colmap/bin/colmap.exe");
        let database = dir.path().join("database.db");
        let manager = ProcessManager::default();
        let mut camera_id = None;
        for (index, name) in ["original.png", "same.png", "different.png"]
            .iter()
            .enumerate()
        {
            let list = dir.path().join(format!("group-{index}.txt"));
            std::fs::write(&list, format!("{name}\n")).unwrap();
            extract_reshoot_group(
                &executable,
                &database,
                ReshootFeatureOptions {
                    images: &images,
                    list: &list,
                    masks: None,
                    panorama_dimensions: None,
                    existing_camera_id: if index == 1 { camera_id } else { None },
                    max_size: 800,
                    max_features: 1024,
                },
                dir.path().join("features.log"),
                &manager,
                None,
                None,
            )
            .await
            .unwrap();
            let db = rusqlite::Connection::open_with_flags(
                &database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let assigned: u32 = db
                .query_row(
                    "SELECT camera_id FROM images WHERE name=?1",
                    [name],
                    |row| row.get(0),
                )
                .unwrap();
            let cameras: u32 = db
                .query_row("SELECT COUNT(*) FROM cameras", [], |row| row.get(0))
                .unwrap();
            if index == 0 {
                camera_id = Some(assigned);
            }
            if index == 1 {
                assert_eq!(Some(assigned), camera_id);
                assert_eq!(cameras, 1);
            }
            if index == 2 {
                assert_ne!(Some(assigned), camera_id);
                assert_eq!(cameras, 2);
            }
        }
        assert_eq!(
            crate::video::panorama::fingerprint(&source).unwrap(),
            original_hash
        );
        eprintln!("Native camera reuse verified: original/same share one camera; different creates a second camera. Original PNG SHA-256 unchanged.");
    }

    #[tokio::test]
    #[ignore = "Requires the local Ballintoy DB and bundled vocabulary; read-only retrieval in an isolated copied database"]
    async fn native_reshoot_retrieval_queries_only_registered_old_images() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let source = repo.join(".cache/panorama-v1/projects/20261017-191833_ballintoy-erp");
        let root = tempfile::tempdir_in(repo.join(".cache")).unwrap();
        let database = root.path().join("database.db");
        std::fs::copy(source.join("work/colmap/database.db"), &database).unwrap();
        let images = crate::reconstruction::colmap_model::read_registered_images(
            &source.join("work/colmap/sparse/0"),
        )
        .unwrap();
        let old: Vec<_> = images.iter().take(5).map(|i| i.name.clone()).collect();
        let new: Vec<_> = images
            .iter()
            .skip(5)
            .take(2)
            .map(|i| i.name.clone())
            .collect();
        let pairs = retrieve_reshoot_pairs(
            &repo.join("engines/colmap/bin/colmap.exe"),
            &database,
            &old,
            &new,
            3,
            root.path().join("retriever.log"),
            &ProcessManager::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(pairs.len(), 6);
        assert!(pairs
            .iter()
            .all(|(a, b)| old.contains(a) && new.contains(b)));
    }

    #[test]
    fn panorama_camera_uses_model_17_dimensions_without_a_fake_focal_length() {
        let config = crate::video::panorama::PanoramaCheckpoint {
            version: 1,
            source_sha256: "test".into(),
            image_sources: None,
            sfm_width: 2048,
            sfm_height: 1024,
            face_size: 512,
            field_of_view_degrees: 92.,
        };
        let mut args = feature_extraction_args(
            Path::new("database.db"),
            Path::new("frames"),
            None,
            Some(2),
            "--FeatureExtraction.use_gpu",
            "--FeatureExtraction.gpu_index",
            Some(2048),
            Some(8192),
            None,
            None,
        );
        apply_panorama_camera(&mut args, &config);
        let args = strings(args);
        assert!(args
            .windows(2)
            .any(|p| p == ["--ImageReader.camera_model", "EQUIRECTANGULAR"]));
        assert!(args
            .windows(2)
            .any(|p| p == ["--ImageReader.camera_params", "2048,1024"]));
        assert!(args
            .windows(2)
            .any(|p| p == ["--FeatureExtraction.gpu_index", "2"]));
    }
    #[tokio::test]
    async fn panorama_mapper_and_bridge_bypass_caspar_even_with_optimization_and_gpu_enabled() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("panorama-camera.json"), b"{}").unwrap();
        let (args, selection) = with_ba(
            Path::new("not-launched"),
            &temp.path().join("database.db"),
            vec!["mapper".into()],
            Some(2),
            true,
            &temp.path().join("log"),
        )
        .await
        .unwrap();
        assert!(selection.gpu_index.is_none());
        let args = strings(args);
        for option in ["--Mapper.ba_local_backend", "--Mapper.ba_global_backend"] {
            assert!(args.windows(2).any(|p| p == [option, "CERES"]));
        }
        assert!(!args.iter().any(|a| a == "CASPAR"));
    }

    #[test]
    fn locked_runtime_identity_matches_the_shared_build_lock() {
        let lock: serde_json::Value =
            serde_json::from_str(include_str!("../../../engines/colmap-build.json")).unwrap();
        assert_eq!(lock["commit"], LOCKED_COMMIT);
        assert_eq!(lock["version"], LOCKED_VERSION);
        assert!(is_locked_build(&format!(
            "COLMAP {LOCKED_VERSION} (Commit {LOCKED_COMMIT})"
        )));
        assert!(is_locked_build(
            "COLMAP 4.2.1 (Commit bd1fcf6 on 2026-09-29 with CUDA, std hash maps)"
        ));
        assert!(!is_locked_build(
            "COLMAP 4.2.1 (Commit bd1fcf60 on 2026-09-29 with CUDA, std hash maps)"
        ));
        assert!(!is_locked_build(
            "COLMAP 4.2.1 (Commit 0000000 on 2026-09-29 with CUDA, std hash maps)"
        ));
        assert!(!is_locked_build("COLMAP 4.1.0.dev0 (Commit 5b76f53)"));
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn gpu_mode_sets_use_gpu_and_selected_index() {
        let extraction = strings(feature_extraction_args(
            Path::new("database.db"),
            Path::new("frames"),
            None,
            Some(2),
            "--FeatureExtraction.use_gpu",
            "--FeatureExtraction.gpu_index",
            None,
            None,
            None,
            None,
        ));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--FeatureExtraction.use_gpu", "1"]));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--FeatureExtraction.gpu_index", "2"]));

        let matching = strings(sequential_matching_args(
            Path::new("database.db"),
            Some(2),
            "--FeatureMatching.use_gpu",
            "--FeatureMatching.gpu_index",
            None,
        ));
        assert!(matching
            .windows(2)
            .any(|pair| pair == ["--FeatureMatching.use_gpu", "1"]));
        assert!(matching
            .windows(2)
            .any(|pair| pair == ["--FeatureMatching.gpu_index", "2"]));
    }

    #[test]
    fn cpu_mode_disables_gpu_without_passing_an_index() {
        let extraction = strings(feature_extraction_args(
            Path::new("database.db"),
            Path::new("frames"),
            None,
            None,
            "--FeatureExtraction.use_gpu",
            "--FeatureExtraction.gpu_index",
            None,
            None,
            None,
            None,
        ));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--FeatureExtraction.use_gpu", "0"]));
        assert!(!extraction
            .iter()
            .any(|arg| arg == "--FeatureExtraction.gpu_index"));

        let matching = strings(sequential_matching_args(
            Path::new("database.db"),
            None,
            "--FeatureMatching.use_gpu",
            "--FeatureMatching.gpu_index",
            None,
        ));
        assert!(matching
            .windows(2)
            .any(|pair| pair == ["--FeatureMatching.use_gpu", "0"]));
        assert!(!matching
            .iter()
            .any(|arg| arg == "--FeatureMatching.gpu_index"));
    }

    #[test]
    fn exhaustive_matching_uses_the_selected_gpu_without_video_options() {
        let matching = strings(matching_args(
            "exhaustive_matcher",
            Path::new("database.db"),
            Some(1),
            "--FeatureMatching.use_gpu",
            "--FeatureMatching.gpu_index",
        ));
        assert_eq!(matching[0], "exhaustive_matcher");
        assert!(matching
            .windows(2)
            .any(|pair| pair == ["--FeatureMatching.use_gpu", "1"]));
        assert!(matching
            .windows(2)
            .any(|pair| pair == ["--FeatureMatching.gpu_index", "1"]));
        assert!(!matching
            .iter()
            .any(|arg| arg == "--SequentialMatching.overlap"));
        assert!(!matching
            .iter()
            .any(|arg| arg.starts_with("--SequentialMatching.")));
    }

    #[test]
    fn loop_detection_is_explicit_and_preserves_sift_gpu_selection() {
        let tree = Path::new("中文 工程 with spaces/lib/colmap/vocab.bin");
        for (use_gpu, gpu_index) in [
            ("--FeatureMatching.use_gpu", "--FeatureMatching.gpu_index"),
            ("--SiftMatching.use_gpu", "--SiftMatching.gpu_index"),
        ] {
            for selected_gpu in [None, Some(2)] {
                for vocabulary in [None, Some(tree)] {
                    let args = strings(sequential_matching_args(
                        Path::new("database.db"),
                        selected_gpu,
                        use_gpu,
                        gpu_index,
                        vocabulary,
                    ));
                    let value = |name: &str| {
                        args.windows(2)
                            .find(|pair| pair[0] == name)
                            .map(|pair| pair[1].as_str())
                    };
                    assert_eq!(
                        value(use_gpu),
                        Some(if selected_gpu.is_some() { "1" } else { "0" })
                    );
                    assert_eq!(value(gpu_index), selected_gpu.map(|_| "2"));
                    assert_eq!(value("--SequentialMatching.overlap"), Some("10"));
                    assert_eq!(
                        value("--SequentialMatching.loop_detection"),
                        Some(if vocabulary.is_some() { "1" } else { "0" })
                    );
                    if vocabulary.is_some() {
                        assert_eq!(value("--SequentialMatching.vocab_tree_path"), tree.to_str());
                        assert_eq!(
                            value("--SequentialMatching.loop_detection_period"),
                            Some("10")
                        );
                        assert_eq!(
                            value("--SequentialMatching.loop_detection_num_images"),
                            Some("50")
                        );
                    } else {
                        assert_eq!(value("--SequentialMatching.vocab_tree_path"), None);
                        assert_eq!(value("--SequentialMatching.loop_detection_period"), None);
                    }
                }
            }
        }
        let bridge = strings(matching_args(
            "matches_importer",
            Path::new("database.db"),
            None,
            "--FeatureMatching.use_gpu",
            "--FeatureMatching.gpu_index",
        ));
        assert!(!bridge
            .iter()
            .any(|arg| arg.starts_with("--SequentialMatching.")));
    }

    #[tokio::test]
    async fn cancelled_loop_matching_stops_before_probing_or_launching_colmap() {
        let manager = ProcessManager::new();
        manager.cancel();
        let error = match_sequential(
            Path::new("missing/bin/colmap"),
            Path::new("database.db"),
            PathBuf::from("colmap.log"),
            &manager,
            None,
            None,
            true,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, SplatError::Cancelled));
    }

    #[tokio::test]
    #[ignore = "Requires the locked bundled COLMAP runtime; runs only CPU extraction and matching"]
    async fn bundled_offline_loop_matching_smoke() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let engines = super::super::EnginePaths::discover(None);
        let executable = std::path::absolute(&engines.colmap).unwrap();
        assert!(
            executable.is_file(),
            "Prepare the locked bundled COLMAP runtime first"
        );
        let temporary = tempfile::tempdir().unwrap();
        let working = temporary.path().join("中文 loop check with spaces");
        let images = working.join("images");
        std::fs::create_dir_all(&images).unwrap();
        let picture = std::fs::read(workspace.join("src-tauri/icons/128x128.png")).unwrap();
        for index in 1..=12 {
            std::fs::write(images.join(format!("frame_{index:06}.png")), &picture).unwrap();
        }
        let database = working.join("database.db");
        let log = working.join("colmap.log");
        let manager = ProcessManager::new();
        // Relative image paths mirror the production Windows Unicode-path
        // workaround, while the database and vocabulary use absolute paths.
        extract_features_quality_v2(
            &executable,
            &database,
            Path::new("images"),
            None,
            None,
            128,
            4096,
            log.clone(),
            &manager,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let database_connection = rusqlite::Connection::open(&database).unwrap();
        let features: u64 = database_connection
            .query_row("SELECT SUM(rows) FROM keypoints", [], |row| row.get(0))
            .unwrap();
        assert!(features > 0, "Smoke input must have real SIFT features");
        drop(database_connection);
        match_sequential(
            &executable,
            &database,
            log.clone(),
            &manager,
            None,
            None,
            true,
        )
        .await
        .unwrap();
        let output = std::fs::read_to_string(&log).unwrap();
        let logged_arguments: Vec<_> = output.lines().map(str::trim).collect();
        assert!(logged_arguments
            .windows(2)
            .any(|pair| pair == ["--SequentialMatching.loop_detection", "1"]));
        assert!(
            output.contains("Indexing image"),
            "Bundled vocabulary must be read, not silently skipped"
        );
        assert!(output.contains("Generating sequential image pairs"));
        println!("Verified offline CPU loop matching: 12 images, {features} SIFT features, bundled hash-locked vocabulary, no Mapper or Brush.");
    }

    #[test]
    fn detects_supported_colmap_cli_families() {
        assert_eq!(
            detect_cli_family("--SiftExtraction.use_gpu", "--SiftMatching.use_gpu"),
            Some(ColmapCliFamily::Legacy39)
        );
        assert_eq!(
            detect_cli_family("--FeatureExtraction.use_gpu", "--FeatureMatching.use_gpu"),
            Some(ColmapCliFamily::Modern4)
        );
        assert_eq!(detect_cli_family("unknown", "unknown"), None);
    }

    #[test]
    fn legacy_cli_uses_legacy_gpu_option_names() {
        let extraction = strings(feature_extraction_args(
            Path::new("database.db"),
            Path::new("frames"),
            None,
            Some(0),
            "--SiftExtraction.use_gpu",
            "--SiftExtraction.gpu_index",
            None,
            None,
            None,
            None,
        ));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--SiftExtraction.use_gpu", "1"]));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--SiftExtraction.gpu_index", "0"]));
    }

    #[test]
    fn transparent_input_passes_the_colmap_mask_root() {
        let extraction = strings(feature_extraction_args(
            Path::new("database.db"),
            Path::new("../colmap_frames"),
            Some(Path::new("../masks")),
            None,
            "--FeatureExtraction.use_gpu",
            "--FeatureExtraction.gpu_index",
            None,
            None,
            None,
            None,
        ));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--ImageReader.mask_path", "../masks"]));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--image_path", "../colmap_frames"]));
    }

    #[test]
    fn optional_camera_parameters_preserve_explicit_values_masks_gpu_and_limits() {
        let args = |params| {
            feature_extraction_args(
                Path::new("database.db"),
                Path::new("../colmap_frames"),
                Some(Path::new("../masks")),
                Some(0),
                "--FeatureExtraction.use_gpu",
                "--FeatureExtraction.gpu_index",
                Some(1600),
                Some(8192),
                Some(Path::new("chosen.txt")),
                params,
            )
        };
        let original = args(None);
        assert!(!original
            .iter()
            .any(|arg| arg == "--ImageReader.camera_params"));
        // The builder must not reinterpret or replace an explicit calibration.
        let explicit = "1234.5,400,300,-0.01";
        let mut overridden = args(Some(explicit));
        assert_eq!(overridden.pop().unwrap(), explicit);
        assert_eq!(overridden.pop().unwrap(), "--ImageReader.camera_params");
        assert_eq!(overridden, original);
    }

    #[test]
    fn quality_v2_feature_limits_and_image_list_reach_colmap() {
        let extraction = strings(feature_extraction_args(
            Path::new("database.db"),
            Path::new("frames"),
            None,
            None,
            "--FeatureExtraction.use_gpu",
            "--FeatureExtraction.gpu_index",
            Some(1600),
            Some(4096),
            Some(Path::new("bridge-images.txt")),
            None,
        ));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--FeatureExtraction.max_image_size", "1600"]));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--SiftExtraction.max_num_features", "4096"]));
        assert!(extraction
            .windows(2)
            .any(|pair| pair == ["--image_list_path", "bridge-images.txt"]));
    }

    #[test]
    fn image_size_only_limit_preserves_default_feature_count_for_both_cli_families() {
        for (use_gpu_option, gpu_index_option, expected_image_option) in [
            (
                "--FeatureExtraction.use_gpu",
                "--FeatureExtraction.gpu_index",
                "--FeatureExtraction.max_image_size",
            ),
            (
                "--SiftExtraction.use_gpu",
                "--SiftExtraction.gpu_index",
                "--SiftExtraction.max_image_size",
            ),
        ] {
            let extraction = strings(feature_extraction_args(
                Path::new("database.db"),
                Path::new("frames"),
                None,
                None,
                use_gpu_option,
                gpu_index_option,
                Some(1200),
                None,
                None,
                None,
            ));
            assert!(extraction
                .windows(2)
                .any(|pair| pair == [expected_image_option, "1200"]));
            assert!(!extraction
                .iter()
                .any(|arg| arg == "--SiftExtraction.max_num_features"));
        }
    }

    #[test]
    fn incremental_mapper_arguments_include_the_baseline_model() {
        let args = strings(mapper_args(
            Path::new("database.db"),
            Path::new("../colmap_frames"),
            Some(Path::new("sparse/0")),
            Path::new("sparse-bridge"),
            false,
        ));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--input_path", "sparse/0"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--Mapper.multiple_models", "0"]));
        assert_eq!(args[0], "mapper");
        assert!(!args.iter().any(|arg| arg == "global_mapper"));
    }

    #[test]
    fn high_mapper_can_triangulate_two_view_tracks() {
        let args = strings(mapper_args(
            Path::new("database.db"),
            Path::new("../colmap_frames"),
            None,
            Path::new("sparse"),
            true,
        ));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--Mapper.tri_ignore_two_view_tracks", "0"]));

        let default_args = strings(mapper_args(
            Path::new("database.db"),
            Path::new("../colmap_frames"),
            None,
            Path::new("sparse"),
            false,
        ));
        assert!(!default_args
            .iter()
            .any(|arg| arg == "--Mapper.tri_ignore_two_view_tracks"));
    }

    #[test]
    fn incremental_features_reuse_the_existing_camera_and_only_new_images() {
        let args = strings(incremental_feature_extraction_args(
            Path::new("database.db"),
            Path::new("../colmap_frames"),
            Path::new("reshoot-images.txt"),
            7,
            Some(Path::new("../reshoot-masks")),
            Some(0),
            "--FeatureExtraction.use_gpu",
            "--FeatureExtraction.gpu_index",
            Some(1600),
            Some(8192),
        ));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--image_list_path", "reshoot-images.txt"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--ImageReader.existing_camera_id", "7"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--FeatureExtraction.max_image_size", "1600"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--SiftExtraction.max_num_features", "8192"]));
        assert!(!args.iter().any(|arg| arg == "--ImageReader.single_camera"));
    }

    #[test]
    fn incremental_feature_limits_support_the_legacy_cli_family() {
        let args = strings(incremental_feature_extraction_args(
            Path::new("database.db"),
            Path::new("../colmap_frames"),
            Path::new("reshoot-images.txt"),
            3,
            None,
            None,
            "--SiftExtraction.use_gpu",
            "--SiftExtraction.gpu_index",
            Some(2000),
            Some(16384),
        ));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--SiftExtraction.max_image_size", "2000"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--SiftExtraction.max_num_features", "16384"]));
    }

    #[test]
    fn incremental_mapper_keeps_existing_frames_and_intrinsics_fixed() {
        let args = strings(incremental_mapper_args(
            Path::new("database.db"),
            Path::new("../colmap_frames"),
            Path::new("base-model"),
            Path::new("incremental-model"),
            Path::new("mapper-images.txt"),
        ));
        for pair in [
            ["--Mapper.fix_existing_frames", "1"],
            ["--Mapper.ba_refine_focal_length", "0"],
            ["--Mapper.ba_refine_principal_point", "0"],
            ["--Mapper.ba_refine_extra_params", "0"],
        ] {
            assert!(args.windows(2).any(|window| window == pair));
        }
    }

    #[test]
    fn only_a_failed_caspar_mapper_process_triggers_the_ceres_retry() {
        let caspar = super::super::colmap_ba::BaSelection {
            gpu_index: Some(0),
            detail: String::new(),
        };
        let ceres = super::super::colmap_ba::BaSelection {
            gpu_index: None,
            detail: String::new(),
        };
        assert!(should_retry_mapper_with_ceres(
            &caspar,
            &SplatError::Process("Caspar solver failed".into())
        ));
        assert!(!should_retry_mapper_with_ceres(
            &ceres,
            &SplatError::Process("Ceres solver failed".into())
        ));
        assert!(!should_retry_mapper_with_ceres(
            &caspar,
            &SplatError::Cancelled
        ));
        let output = crate::process::ProcessOutput {
            success: false,
            exit_code: Some(-1073740940),
            termination: crate::process::ProcessTermination {
                exit_code: Some(-1073740940),
                windows_status: Some("0xC0000374".into()),
                ..Default::default()
            },
            stdout: String::new(),
            stderr: "Could not register image".into(),
        };
        let error =
            SplatError::EngineProcess(Box::new(crate::failure::EngineProcessFailure::from_output(
                crate::pipeline::PipelineEngine::Colmap,
                "mapper",
                &output,
            )));
        assert!(should_retry_mapper_with_ceres(&caspar, &error));
        assert!(!should_retry_mapper_with_ceres(&ceres, &error));
    }

    #[tokio::test]
    async fn ceres_retry_discards_partial_mapper_output() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("sparse");
        tokio::fs::create_dir_all(output.join("0")).await.unwrap();
        tokio::fs::write(output.join("0/partial.bin"), b"partial")
            .await
            .unwrap();
        reset_mapper_output(&output).await.unwrap();
        assert!(output.is_dir());
        assert!(!output.join("0/partial.bin").exists());
    }
}
