//! New reshoot execution is intentionally separate from the legacy runner.
use super::*;
use crate::pipeline::reshoot::*;
use crate::reconstruction::colmap_model::read_cameras;
use crate::video::panorama::{self, VideoProjection};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Application-side work only. Engine processes keep their existing observers.
/// Counts are log messages, never synthetic stage percentages or iterations.
struct PreparationClock {
    activity: Instant,
    logged: Instant,
    notice: Option<Instant>,
}

impl PreparationClock {
    fn quiet_notice(&mut self, now: Instant) -> bool {
        if now.duration_since(self.activity) < Duration::from_secs(15)
            || self
                .notice
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(30))
        {
            return false;
        }
        self.notice = Some(now);
        true
    }
}

struct ReshootProgress {
    events: EventSink,
    stage: PipelineStage,
    label: &'static str,
    clock: Arc<std::sync::Mutex<PreparationClock>>,
    cancel: tokio_util::sync::CancellationToken,
    timer: tokio::task::JoinHandle<()>,
}

impl ReshootProgress {
    fn new(
        events: &EventSink,
        stage: PipelineStage,
        cancel: &tokio_util::sync::CancellationToken,
        label: &'static str,
        total: Option<u64>,
    ) -> Self {
        let mut scoped = events.clone();
        scoped.bound_operation = events
            .context
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .current
            .clone();
        let now = Instant::now();
        let clock = Arc::new(std::sync::Mutex::new(PreparationClock {
            activity: now,
            logged: now,
            notice: None,
        }));
        let timer_clock = clock.clone();
        let timer_events = scoped.clone();
        let timer_cancel = cancel.clone();
        let timer = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = timer_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        if timer_clock.lock().unwrap_or_else(|p| p.into_inner()).quiet_notice(Instant::now()) {
                            reshoot_log(&timer_events, stage, "补拍处理仍在进行，当前步骤可能耗时较长，请耐心等待");
                        }
                    }
                }
            }
        });
        let value = Self {
            events: scoped,
            stage,
            label,
            clock,
            cancel: cancel.clone(),
            timer,
        };
        value.log(total.map_or_else(|| label.to_owned(), |n| format!("{label}：0/{n}")));
        value
    }

    fn log(&self, message: String) {
        if !self.cancel.is_cancelled() {
            reshoot_log(&self.events, self.stage, message);
        }
    }

    fn update(&self, current: u64, total: u64) {
        let now = Instant::now();
        let mut clock = self.clock.lock().unwrap_or_else(|p| p.into_inner());
        clock.activity = now;
        clock.notice = None;
        let log = current == total || now.duration_since(clock.logged) >= Duration::from_secs(1);
        if log {
            clock.logged = now;
        }
        drop(clock);
        if log {
            self.log(format!("{}：{current}/{total}", self.label));
        }
    }
}

impl Drop for ReshootProgress {
    fn drop(&mut self) {
        self.timer.abort();
    }
}

async fn copy_reshoot_file(
    source: &Path,
    target: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    let mut input = tokio::fs::File::open(source).await?;
    let mut output = tokio::fs::File::create(target).await?;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(SplatError::Cancelled);
        }
        let count = input.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count]).await?;
    }
    output.flush().await?;
    Ok(())
}

fn reshoot_log(events: &EventSink, stage: PipelineStage, message: impl Into<String>) {
    let _dispatch = events.dispatch.lock().unwrap_or_else(|p| p.into_inner());
    let context = events.context.lock().unwrap_or_else(|p| p.into_inner());
    if events
        .bound_operation
        .as_ref()
        .is_some_and(|op| context.current.as_ref().map(|active| active.id) != Some(op.id))
    {
        return;
    }
    (events.emit)(PipelineEvent {
        operation: context.current.clone(),
        task_id: None,
        run_id: None,
        revision: 0,
        sequence: events.sequence.fetch_add(1, Ordering::Relaxed) + 1,
        timestamp: Utc::now(),
        kind: EventKind::Log,
        level: EventLevel::Info,
        stage: context.stage.unwrap_or(stage),
        substage: context.substage.clone(),
        engine: Some(PipelineEngine::System),
        progress: events.last_progress_milli_percent.load(Ordering::Relaxed) as f32 / 1000.,
        stage_progress: None,
        indeterminate: true,
        message: message.into(),
        current: None,
        total: None,
        unit: None,
        elapsed_ms: events.started.elapsed().as_millis() as u64,
        acceleration: None,
        runtime: None,
    });
}

fn reusable_camera(
    relation: Option<CameraRelation>,
    cameras: &[ColmapCamera],
    images: &[(u32, u32, u8)],
) -> Option<u32> {
    if relation != Some(CameraRelation::Same) || cameras.len() != 1 || images.is_empty() {
        return None;
    }
    let camera = &cameras[0];
    (camera.model_id != 17
        && camera.width > 0
        && camera.height > 0
        && camera.params.first().is_some_and(|v| *v > 0.)
        && camera.params.iter().all(|v| v.is_finite())
        && images.iter().all(|&(w, h, orientation)| {
            (w as u64, h as u64, orientation) == (camera.width, camera.height, 1)
        }))
    .then_some(camera.id)
}

fn normal_training_preset(
    quality: Quality,
    automatic: bool,
    memory: Option<u64>,
    long_edge: u32,
    face_size: Option<u32>,
    initial_points: u64,
    resolution: Option<crate::presets::PlannerResolutionPlan>,
) -> Result<ResolvedBrushTrainingPreset> {
    if let Some(face_size) = face_size {
        resolve_panorama_training_preset(quality, automatic, memory, face_size, initial_points)
    } else if let Some(resolution) = resolution.filter(|_| automatic) {
        Ok(resolve_brush_training_preset_for_plan(
            quality,
            memory,
            long_edge,
            initial_points,
            &resolution,
        ))
    } else {
        Ok(resolve_brush_training_preset(
            quality,
            automatic,
            memory,
            long_edge,
            initial_points,
        ))
    }
}

fn changed() -> SplatError {
    SplatError::Process("素材文件已发生变化，请重新分析后再生成".into())
}
fn invalid(detail: &str) -> SplatError {
    SplatError::Process(detail.into())
}

async fn validate_ply(
    path: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(crate::reconstruction::ply::PlyInfo, u32, String)> {
    let path = path.to_owned();
    let cancel = cancel.clone();
    crate::presets::spawn_pipeline_blocking(move || {
        crate::reconstruction::splat_transform::validate_training_ply(&path, &cancel)
    })
    .await
    .map_err(|e| invalid(&format!("Gaussian validation worker: {e}")))?
}

struct SourceAssets {
    root: PathBuf,
    metadata: ProjectMetadata,
    state: PipelineStateFile,
    images: PathBuf,
    colmap_images: PathBuf,
    masks: PathBuf,
    database: PathBuf,
    model: PathBuf,
}

async fn source_assets(id: uuid::Uuid) -> Result<SourceAssets> {
    let (root, metadata) = catalog::load_registered_project(id).await?;
    if !catalog::project_is_durably_completed(&root, &metadata).await {
        return Err(invalid("只有已完成且结果完整的项目可以进行高清补拍"));
    }
    let state: PipelineStateFile =
        serde_json::from_slice(&tokio::fs::read(root.join("state.json")).await?)?;
    if let Some(imported) = &metadata.imported {
        let colmap = imported
            .colmap
            .as_ref()
            .ok_or_else(|| invalid("外部项目没有可复用的 COLMAP 数据"))?;
        crate::imports::revalidate_imported_colmap(colmap)?;
        let images = colmap
            .images_path
            .clone()
            .ok_or_else(|| invalid("外部 COLMAP 项目缺少原始图片"))?;
        return Ok(SourceAssets {
            images: images.clone(),
            colmap_images: images,
            masks: root.join("work/masks"),
            database: colmap
                .database_path
                .clone()
                .ok_or_else(|| invalid("外部 COLMAP 项目缺少 database.db"))?,
            model: colmap.sparse_model_path.clone(),
            root,
            metadata,
            state,
        });
    }
    let images = root.join("work/frames");
    let colmap_images = root.join("work/colmap_frames");
    let database = root.join("work/colmap/database.db");
    let masks = root.join("work/masks");
    let expected = state
        .frames
        .as_ref()
        .and_then(|f| f.extracted_frames)
        .ok_or_else(|| invalid("项目缺少已准备图片数量"))?;
    let names = crate::video::list_images(&images)?;
    if names.len() as u64 != expected
        || state.frames.as_ref().and_then(|f| f.colmap_input_version) != Some(COLMAP_INPUT_VERSION)
    {
        return Err(invalid(
            "原项目的 COLMAP 专用图片不完整或与原始帧不一致，无法安全进行增量补拍",
        ));
    }
    for image in names {
        let counterpart = colmap_images.join(
            image
                .file_name()
                .ok_or_else(|| invalid("Invalid image name"))?,
        );
        let header = crate::video::image_projection::inspect(&image)?;
        let counterpart_header = crate::video::image_projection::inspect(&counterpart)?;
        if (header.width, header.height) != (counterpart_header.width, counterpart_header.height) {
            return Err(invalid(
                "原项目缺少 COLMAP 专用图片，无法安全进行增量补拍。",
            ));
        }
    }
    let full = state
        .reshoot
        .as_ref()
        .and_then(|r| r.recipe.as_ref())
        .is_some_and(|r| r.actual.reconstruction == ReconstructionMode::Full);
    let model = if state
        .gps
        .as_ref()
        .is_some_and(|g| matches!(g.status.as_str(), "complete" | "matching"))
    {
        let model = root.join("work/colmap/gps-v1/model");
        if super::gps_runner::model_identity(&model).ok()
            != state.gps.as_ref().and_then(|g| g.output_identity.clone())
        {
            return Err(invalid("Source georeference model identity changed"));
        }
        ReconstructionValidator::validate(&images, &model)?;
        model
    } else if full {
        best_sparse_model(&images, &root.join("work/colmap/sparse-full"))
            .await?
            .0
    } else {
        reusable_reconstruction(&root, &images, metadata.output.as_ref())
            .await?
            .model
    };
    let cameras = read_cameras(&model)?;
    for image in read_registered_images(&model)? {
        let camera = cameras
            .iter()
            .find(|c| c.id == image.camera_id)
            .ok_or_else(|| invalid("Source image has no camera"))?;
        let header = crate::video::image_projection::inspect(&images.join(image.name))?;
        if (header.width as u64, header.height as u64) != (camera.width, camera.height) {
            return Err(invalid("Source image/camera dimensions differ"));
        }
    }
    let db = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| invalid(&e.to_string()))?;
    for camera in &cameras {
        // BA legitimately refines intrinsics only in the sparse model. Verify
        // identity/geometry here; copy refined values into the DERIVED DB later.
        let row: (i32, u64, u64) = db
            .query_row(
                "SELECT model,width,height FROM cameras WHERE camera_id=?1",
                [camera.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| invalid(&e.to_string()))?;
        if row != (camera.model_id, camera.width, camera.height) {
            return Err(invalid("Source database/model camera identity differs"));
        }
    }
    if cameras
        .iter()
        .any(|c| (c.model_id == 17) != state.panorama.is_some())
    {
        return Err(invalid("Invalid source projection/camera mapping"));
    }
    Ok(SourceAssets {
        root,
        metadata,
        state,
        images,
        colmap_images,
        masks,
        database,
        model,
    })
}

fn source_identity(source: &SourceAssets) -> Result<String> {
    let mut hash = Sha256::new();
    for path in [
        source.root.join("project.json"),
        source.root.join("state.json"),
    ] {
        hash.update(std::fs::read(path)?);
    }
    let mut paths = reusable_model_files(&source.model)?;
    paths.push(source.database.clone());
    paths.push(
        catalog::project_model_path(&source.root, &source.metadata)
            .unwrap_or_else(|| source.root.join("final.ply")),
    );
    for path in paths {
        hash.update(path.to_string_lossy().as_bytes());
        let stat = path.metadata()?;
        hash.update(stat.len().to_le_bytes());
        hash.update(
            stat.modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| changed())?
                .as_nanos()
                .to_le_bytes(),
        );
    }
    // Cheap per-member identity, including already-prepared supplemental images.
    for dir in [&source.images, &source.colmap_images] {
        hash.update(panorama::source_identity(dir)?.as_bytes());
    }
    if source.masks.is_dir() {
        hash.update(panorama::source_identity(&source.masks)?.as_bytes());
    }
    Ok(format!("{:x}", hash.finalize()))
}

impl PipelineRunner {
    pub async fn validate_reshoot_request_v2(
        &self,
        project: uuid::Uuid,
        input: &Path,
        input_type: ReshootInputType,
        quality: Quality,
        options: &ReshootOptions,
    ) -> Result<()> {
        let source = self.inspect_reshoot_source_v2(project).await?;
        let probe = self
            .probe_reshoot_input_v2(project, input, input_type, quality)
            .await?;
        if let Some(images) = &probe.image_sequence {
            if let Some(tracker) = &self.effectiveness {
                tracker.record_frame_plan(PlannerFramePlanMetrics::images(
                    images.width,
                    images.height,
                    images.image_count,
                ));
            }
        } else if let Some(video) = &probe.video {
            let plan = if let Some(policy) = self.policy() {
                policy.frame_plan(video, quality)
            } else if self.planner_enabled {
                QualityV2FrameSelection.create_plan(video, &quality.preset())
            } else {
                UniformRatioFrameSelection.create_plan(video, &quality.preset())
            };
            self.record_input_plan(quality, video, &plan, true);
        }
        if source.source_identity != options.source_identity
            || probe.source_identity.as_deref() != Some(options.input_identity.as_str())
            || source.projection != options.projection
        {
            return Err(changed());
        }
        if !probe.compatible {
            return Err(invalid(
                probe
                    .incompatibility_reason
                    .as_deref()
                    .unwrap_or("Invalid input"),
            ));
        }
        let (root, metadata) = catalog::load_registered_project(project).await?;
        ensure_reshoot_input_is_new(&root, &metadata.source_path, input, input_type).await
    }

    pub async fn inspect_reshoot_source_v2(
        &self,
        project: uuid::Uuid,
    ) -> Result<ReshootSourceInfo> {
        let source = source_assets(project).await?;
        let cameras = read_cameras(&source.model)?;
        let first = cameras.first().ok_or_else(|| invalid("Missing cameras"))?;
        let model = catalog::project_model_path(&source.root, &source.metadata);
        let warm_available = source.metadata.imported.is_none()
            && model
                .as_ref()
                .is_some_and(|p| inspect_gaussian_ply(p).is_ok());
        Ok(ReshootSourceInfo {
            source_identity: source_identity(&source)?,
            projection: if source.state.panorama.is_some() {
                VideoProjection::Equirectangular
            } else {
                VideoProjection::Perspective
            },
            warm_available,
            project_id: project.to_string(),
            project_name: source.metadata.name,
            quality: source.metadata.quality,
            planner_enabled: source.state.planner_enabled,
            camera_id: first.id,
            camera_model: first.model.clone(),
            width: first.width,
            height: first.height,
            source_image_count: crate::video::list_images(&source.images)?.len() as u64,
            eligible: true,
            reason: None,
            imported: source.metadata.imported.is_some(),
        })
    }

    pub async fn probe_reshoot_input_v2(
        &self,
        project: uuid::Uuid,
        input: &Path,
        input_type: ReshootInputType,
        quality: Quality,
    ) -> Result<ReshootInputInfo> {
        self.probe_reshoot_input_v2_cached(project, input, input_type, quality, None)
            .await
    }

    pub async fn probe_reshoot_input_v2_cached(
        &self,
        project: uuid::Uuid,
        input: &Path,
        input_type: ReshootInputType,
        quality: Quality,
        cached: Option<crate::commands::VideoAnalysis>,
    ) -> Result<ReshootInputInfo> {
        let source = self.inspect_reshoot_source_v2(project).await?;
        let identity = panorama::source_identity(input)?;
        let samples = catalog::runtime_samples().await;
        let mut analyzed_video = None;
        let (count, duration, width, height, estimated, alpha, detection, sequence, estimate) =
            if input_type == ReshootInputType::Images {
                let scan = if source.projection == VideoProjection::Equirectangular {
                    crate::video::image_projection::analyze(input)?
                } else {
                    crate::video::image_projection::analyze_reshoot(input)?
                };
                let plan = crate::video::create_image_plan(&scan.info, &quality.preset());
                let estimate = estimate_runtime_for_images(
                    source.source_image_count + scan.info.image_count,
                    scan.info.width.max(scan.info.height),
                    &plan,
                    quality,
                    &samples,
                );
                (
                    Some(scan.info.image_count),
                    None,
                    scan.info.width,
                    scan.info.height,
                    scan.info.image_count,
                    scan.info.has_alpha,
                    scan.detection,
                    Some(scan.info),
                    estimate,
                )
            } else {
                let (video, mut detection) = if let Some(cached) =
                    cached.filter(|c| c.projection_detection.source_identity == identity)
                {
                    (cached.video, cached.projection_detection)
                } else {
                    let (video, json) = crate::engines::ffprobe::probe_video_metadata(
                        &self.engines.ffprobe,
                        input,
                        None,
                        &self.process_manager,
                    )
                    .await?;
                    let detection = panorama::detect_metadata(
                        &json,
                        &video,
                        input
                            .extension()
                            .is_some_and(|s| s.eq_ignore_ascii_case("mp4")),
                    );
                    (video, detection)
                };
                detection.source_identity = identity.clone();
                analyzed_video = Some(video.clone());
                let plan = if let Some(policy) = self.policy() {
                    policy.frame_plan(&video, quality)
                } else if self.planner_enabled {
                    QualityV2FrameSelection.create_plan(&video, &quality.preset())
                } else {
                    UniformRatioFrameSelection.create_plan(&video, &quality.preset())
                };
                let estimate = estimate_runtime(&video, &plan, quality, &samples);
                (
                    None,
                    Some(video.duration),
                    video.width,
                    video.height,
                    plan.estimated_frames,
                    video.has_alpha,
                    detection,
                    None,
                    estimate,
                )
            };
        let compatible = if source.projection == VideoProjection::Equirectangular {
            detection.supported_erp
        } else {
            !detection.confirmed_erp
        };
        Ok(ReshootInputInfo {
            video: analyzed_video,
            source_identity: Some(identity),
            projection_detection: Some(detection),
            image_sequence: sequence,
            input_type,
            image_count: count,
            duration,
            prepared_width: width,
            prepared_height: height,
            estimated_frames: estimated,
            has_alpha: alpha,
            mask_count: if alpha { estimated } else { 0 },
            compatible,
            incompatibility_reason: (!compatible)
                .then(|| "全景项目仅支持完整全景视频或全景图片补拍，暂不支持普通素材。".into()),
            estimate,
        })
    }

    pub(super) async fn generate_reshoot_v2(
        &self,
        project: uuid::Uuid,
        input: &Path,
        input_type: ReshootInputType,
        root: &Path,
        quality: Quality,
        options: ReshootOptions,
    ) -> Result<PipelineResult> {
        let source = source_assets(project).await?;
        let info = self.inspect_reshoot_source_v2(project).await?;
        if options.source_identity != info.source_identity
            || options.input_identity != panorama::source_identity(input)?
            || options.projection != info.projection
        {
            return Err(changed());
        }
        let probe = self
            .probe_reshoot_input_v2(project, input, input_type, quality)
            .await?;
        if let Some(images) = &probe.image_sequence {
            if let Some(tracker) = &self.effectiveness {
                tracker.record_frame_plan(PlannerFramePlanMetrics::images(
                    images.width,
                    images.height,
                    images.image_count,
                ));
            }
        } else if let Some(video) = &probe.video {
            let plan = if let Some(policy) = self.policy() {
                policy.frame_plan(video, quality)
            } else if self.planner_enabled {
                QualityV2FrameSelection.create_plan(video, &quality.preset())
            } else {
                UniformRatioFrameSelection.create_plan(video, &quality.preset())
            };
            self.record_input_plan(quality, video, &plan, true);
        }
        if !probe.compatible {
            return Err(invalid(
                probe
                    .incompatibility_reason
                    .as_deref()
                    .unwrap_or("Invalid input"),
            ));
        }
        ensure_reshoot_input_is_new(
            &source.root,
            &source.metadata.source_path,
            input,
            input_type,
        )
        .await?;
        let acceleration = self.verify_pipeline_engines().await?;
        let manager =
            ProjectManager::with_root(root.into()).with_input_boundary(self.input_boundary.clone());
        let (paths, mut metadata) = manager
            .create_reshoot_v2(input, quality, self.process_manager.child_token())
            .await?;
        if panorama::source_identity(input)? != options.input_identity
            || source_identity(&source)? != options.source_identity
        {
            return Err(changed());
        }
        metadata.name = reshoot_project_name(&source.metadata.name);
        metadata.workspace_task_id = self.workspace_task_id;
        metadata.transform = source.metadata.transform;
        metadata.reshoot = Some(ReshootProvenance {
            source_project_id: project,
            source_project_path: source.root.clone(),
            source_final_ply: catalog::project_model_path(&source.root, &source.metadata)
                .unwrap_or_else(|| source.root.join("final.ply")),
            reshoot_source_path: metadata.source_path.clone(),
            camera_id: info.camera_id,
            camera_model: info.camera_model,
            width: info.width,
            height: info.height,
            has_alpha: probe.has_alpha,
            mask_count: 0,
            source_image_count: info.source_image_count,
            reshoot_frame_count: probe.estimated_frames,
            registered_reshoot_count: 0,
        });
        manager.write_metadata(&paths.metadata, &metadata).await?;
        let mut state = manager.read_state(&paths.state).await?;
        state.video = probe.video.clone();
        state.image_sequence = probe.image_sequence.clone();
        state.generation_policy = self.policy();
        state.planner_enabled = self.planner_enabled;
        if metadata.input_type == ProjectInputType::Images {
            state.gps = Some(crate::reconstruction::gps::Checkpoint::pending());
        }
        state.video_projection = options.projection;
        state.configuration = Some((*crate::presets::pipeline_optimization_config()).clone());
        let mut options = options;
        if let Some(policy) = state.generation_policy {
            options.allow_fallback = policy.auto_retry_enabled;
        }
        let plan = ReshootPlan::resolve(options.condition, true, info.warm_available);
        state.reshoot = Some(ReshootState {
            recipe: Some(ReshootRecipe {
                version: RECIPE_VERSION,
                normal_parameters_version: Some(1),
                camera_assignment: None,
                options,
                initial: plan,
                actual: plan,
                batch_id: paths.id.simple().to_string(),
                reconstruction_fallbacks: 0,
                training_fallbacks: 0,
                expanded_matching: false,
                source_names: vec![],
                new_names: vec![],
                warm_parameters: None,
                init_sha256: None,
                init_georeference: None,
                stored_input_identity: Some(panorama::source_identity(&metadata.source_path)?),
                panorama_frames: vec![],
            }),
            ..Default::default()
        });
        manager.write_state(&paths.state, &state).await?;
        self.execute_project(manager, paths, &mut metadata, state, &acceleration)
            .await
    }
}

fn normalize_image(
    source: &Path,
    frames: &Path,
    colmap: &Path,
    masks: &Path,
    name: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<String> {
    use image::ImageDecoder;
    if cancel.is_cancelled() {
        return Err(SplatError::Cancelled);
    }
    let header = crate::video::image_projection::inspect(source)?;
    let name = if header.orientation != 1 {
        format!(
            "{}.png",
            Path::new(name).file_stem().unwrap().to_string_lossy()
        )
    } else {
        name.into()
    };
    let target = frames.join(&name);
    if header.alpha || header.orientation != 1 {
        let mut reader = image::ImageReader::open(source)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(panorama::MAX_DECODE_BYTES);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().map_err(|e| invalid(&e.to_string()))?;
        let orientation = decoder.orientation().map_err(|e| invalid(&e.to_string()))?;
        let mut decoded =
            image::DynamicImage::from_decoder(decoder).map_err(|e| invalid(&e.to_string()))?;
        decoded.apply_orientation(orientation);
        if header.orientation != 1 {
            decoded.save(&target).map_err(|e| invalid(&e.to_string()))?;
        } else {
            std::fs::copy(source, &target)?;
        }
        let rgba = decoded.to_rgba8();
        let mask = image::GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            image::Luma([if rgba.get_pixel(x, y)[3] >= 128 {
                255
            } else {
                0
            }])
        });
        mask.save(masks.join(format!("{name}.png")))
            .map_err(|e| invalid(&e.to_string()))?;
    } else {
        std::fs::copy(source, &target)?;
    }
    std::fs::copy(target, colmap.join(&name))?;
    Ok(name)
}

fn ensure_masks(
    images: &Path,
    masks: &Path,
    cancel: &tokio_util::sync::CancellationToken,
    progress: Option<&ReshootProgress>,
) -> Result<()> {
    let images = crate::video::list_images(images)?;
    let total = images.len() as u64;
    for (index, image) in images.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(SplatError::Cancelled);
        }
        let mask = masks.join(format!(
            "{}.png",
            image.file_name().unwrap().to_string_lossy()
        ));
        let header = crate::video::image_projection::inspect(image)?;
        if mask.exists() {
            if image::image_dimensions(&mask).map_err(|e| invalid(&e.to_string()))?
                != (header.width, header.height)
            {
                return Err(invalid("Mask/image dimensions differ"));
            }
        } else {
            if header.width as u64 * header.height as u64 > panorama::MAX_DECODE_BYTES {
                return Err(invalid("Image mask exceeds decode limit"));
            }
            image::GrayImage::from_pixel(header.width, header.height, image::Luma([255]))
                .save(mask)
                .map_err(|e| invalid(&e.to_string()))?;
        }
        if let Some(progress) = progress {
            progress.update(index as u64 + 1, total);
        }
    }
    Ok(())
}

async fn save_recipe(
    manager: &ProjectManager,
    paths: &ProjectPaths,
    state: &mut PipelineStateFile,
    checkpoint: &ReshootState,
    recipe: &ReshootRecipe,
) -> Result<()> {
    let mut checkpoint = checkpoint.clone();
    checkpoint.recipe = Some(recipe.clone());
    state.stage =
        if !checkpoint.source_snapshot_complete || !checkpoint.supplemental_frames_complete {
            PipelineStage::ExtractingFrames
        } else if !checkpoint.supplemental_features_complete {
            PipelineStage::ExtractingFeatures
        } else if !checkpoint.incremental_matching_complete {
            PipelineStage::Matching
        } else if !checkpoint.incremental_reconstruction_complete {
            PipelineStage::Reconstructing
        } else {
            PipelineStage::TrainingSplats
        };
    state.reshoot = Some(checkpoint);
    manager.write_state(&paths.state, state).await
}

fn source_panorama_records(source: &SourceAssets) -> Result<Vec<PanoramaFrameRecord>> {
    if let Some(recipe) = source
        .state
        .reshoot
        .as_ref()
        .and_then(|s| s.recipe.as_ref())
    {
        return Ok(recipe.panorama_frames.clone());
    }
    let config = source
        .state
        .panorama
        .as_ref()
        .ok_or_else(|| invalid("Missing panorama source mapping"))?;
    if let Some(images) = &config.image_sources {
        return Ok(images
            .iter()
            .map(|i| PanoramaFrameRecord {
                prepared_name: i.prepared_name.clone(),
                batch_id: source.metadata.id.simple().to_string(),
                source_name: i.source_name.clone(),
                source_sha256: i.source_sha256.clone(),
                orientation: Some(i.orientation),
                source_index: i.source_image_index,
                timestamp_seconds: None,
                pts: None,
            })
            .collect());
    }
    let times: Vec<panorama::FrameTime> = serde_json::from_slice(&std::fs::read(
        source.root.join("work/panorama-frame-times.json"),
    )?)?;
    let mut records = Vec::new();
    for image in crate::video::list_images(&source.images)? {
        let stem = image
            .file_stem()
            .and_then(|v| v.to_str())
            .and_then(|v| v.strip_prefix("frame_"))
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| invalid("Invalid panorama frame name"))?;
        let time = times
            .iter()
            .find(|t| t.source_frame_index == stem)
            .ok_or_else(|| invalid("Missing panorama timestamp mapping"))?;
        records.push(PanoramaFrameRecord {
            prepared_name: image.file_name().unwrap().to_string_lossy().into_owned(),
            batch_id: source.metadata.id.simple().to_string(),
            source_name: source
                .metadata
                .source_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            source_index: stem,
            source_sha256: config.source_sha256.clone(),
            orientation: None,
            timestamp_seconds: Some(time.timestamp_seconds),
            pts: Some(time.pts),
        });
    }
    Ok(records)
}

impl PipelineRunner {
    pub(super) async fn run_reshoot_v2(
        &self,
        manager: &ProjectManager,
        paths: &ProjectPaths,
        metadata: &mut ProjectMetadata,
        state: PipelineStateFile,
        acceleration: &crate::engines::ColmapAccelerationStatus,
    ) -> Result<PipelineResult> {
        let source_id = metadata
            .reshoot
            .as_ref()
            .ok_or_else(|| invalid("Missing reshoot provenance"))?
            .source_project_id;
        let source = source_assets(source_id).await?;
        self.execute_reshoot_v2(manager, paths, metadata, state, acceleration, source)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_reshoot_v2(
        &self,
        manager: &ProjectManager,
        paths: &ProjectPaths,
        metadata: &mut ProjectMetadata,
        mut state: PipelineStateFile,
        acceleration: &crate::engines::ColmapAccelerationStatus,
        source: SourceAssets,
    ) -> Result<PipelineResult> {
        let mut checkpoint = state
            .reshoot
            .clone()
            .ok_or_else(|| invalid("Missing reshoot checkpoint"))?;
        let mut recipe = checkpoint
            .recipe
            .clone()
            .ok_or_else(|| invalid("Missing reshoot recipe"))?;
        if recipe.version != RECIPE_VERSION
            || recipe.normal_parameters_version.is_some_and(|v| v != 1)
            || recipe
                .camera_assignment
                .as_ref()
                .is_some_and(|v| v.version != 1)
        {
            return Err(invalid("Unsupported reshoot recipe"));
        }
        let provenance = metadata
            .reshoot
            .clone()
            .ok_or_else(|| invalid("Missing reshoot provenance"))?;
        if source_identity(&source)? != recipe.options.source_identity
            || recipe.stored_input_identity.as_deref()
                != Some(&panorama::source_identity(&metadata.source_path)?)
        {
            return Err(changed());
        }
        let is_panorama = recipe.options.projection == VideoProjection::Equirectangular;
        self.events
            .panorama_phase
            .store(u64::from(is_panorama), Ordering::Relaxed);
        self.events.panorama_images.store(
            metadata.input_type == ProjectInputType::Images,
            Ordering::Relaxed,
        );
        let cancel = self.process_manager.child_token();
        let database = paths.colmap.join("database.db");
        let base = paths.colmap.join("base-model");
        if !checkpoint.source_snapshot_complete {
            self.events.stage(
                PipelineStage::ExtractingFrames,
                0.,
                "正在复用原项目的相机与重建结果",
            );
            reset_directory(&paths.frames).await?;
            reset_directory(&paths.colmap_frames).await?;
            reset_directory(&paths.masks).await?;
            reset_directory(&paths.colmap).await?;
            let mut files = Vec::new();
            for (from, to) in [
                (&source.images, &paths.frames),
                (&source.colmap_images, &paths.colmap_frames),
                (&source.masks, &paths.masks),
            ] {
                if from.is_dir() {
                    for file in crate::video::list_images(from)? {
                        let target = to.join(file.file_name().unwrap());
                        files.push((file, target));
                    }
                }
            }
            files.push((source.database.clone(), database.clone()));
            tokio::fs::create_dir_all(&base).await?;
            for file in reusable_model_files(&source.model)? {
                let target = base.join(file.file_name().unwrap());
                files.push((file, target));
            }
            let progress = ReshootProgress::new(
                &self.events,
                PipelineStage::ExtractingFrames,
                &cancel,
                "正在准备原项目文件",
                Some(files.len() as u64),
            );
            for (index, (from, to)) in files.iter().enumerate() {
                copy_reshoot_file(from, to, &cancel).await?;
                progress.update(index as u64 + 1, files.len() as u64);
            }
            drop(progress);
            sync_refined_cameras(&database, &source.model)?;
            recipe.source_names = crate::video::list_images(&paths.frames)?
                .into_iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            if is_panorama {
                recipe.panorama_frames = source_panorama_records(&source)?;
            }
            checkpoint.source_image_count = recipe.source_names.len() as u64;
            checkpoint.colmap_input_version = Some(COLMAP_INPUT_VERSION);
            checkpoint.source_snapshot_complete = true;
            save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
        } else {
            reshoot_log(
                &self.events,
                PipelineStage::ExtractingFrames,
                "复用已完成的补拍缓存：原项目文件",
            );
        }
        if !checkpoint.supplemental_frames_complete {
            self.events
                .stage(PipelineStage::ExtractingFrames, 0., "正在检查补拍素材");
            // Remove only this batch's uncommitted files after an interrupted preparation.
            let prefix = format!("reshoot_{}_", recipe.batch_id);
            for dir in [&paths.frames, &paths.colmap_frames, &paths.masks] {
                for entry in std::fs::read_dir(dir)? {
                    let p = entry?.path();
                    if p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
                        && p.is_file()
                    {
                        std::fs::remove_file(p)?;
                    }
                }
            }
            recipe
                .panorama_frames
                .retain(|f| f.batch_id != recipe.batch_id);
            let stage = paths.work.join("supplemental-preparation");
            reset_directory(&stage).await?;
            let stage_frames = stage.join("frames");
            let stage_colmap = stage.join("colmap");
            let stage_masks = stage.join("masks");
            for dir in [&stage_frames, &stage_colmap, &stage_masks] {
                tokio::fs::create_dir_all(dir).await?;
            }
            let mut batch_records = Vec::new();
            if metadata.input_type == ProjectInputType::Images && is_panorama {
                let progress = ReshootProgress::new(
                    &self.events,
                    PipelineStage::ExtractingFrames,
                    &cancel,
                    "正在准备补拍画面",
                    None,
                );
                let config = crate::video::image_projection::checkpoint_for_policy(
                    &metadata.source_path,
                    metadata.quality,
                    &cancel,
                    state.generation_policy.is_some(),
                )?;
                crate::video::image_projection::prepare(
                    &metadata.source_path,
                    &config,
                    &stage_frames,
                    &stage_colmap,
                    &stage_masks,
                    &cancel,
                    |current, total| {
                        progress.update(current, total);
                        self.events.send(
                            PipelineStage::ExtractingFrames,
                            Some(PipelineEngine::System),
                            EventKind::Progress,
                            EventLevel::Info,
                            Some(current as f32 / total.max(1) as f32),
                            false,
                            "正在准备全景图片",
                            Some(current),
                            Some(total),
                            Some("images"),
                        )
                    },
                )?;
                batch_records = config
                    .image_sources
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|i| PanoramaFrameRecord {
                        prepared_name: i.prepared_name.clone(),
                        batch_id: recipe.batch_id.clone(),
                        source_name: i.source_name.clone(),
                        source_sha256: i.source_sha256.clone(),
                        orientation: Some(i.orientation),
                        source_index: i.source_image_index,
                        timestamp_seconds: None,
                        pts: None,
                    })
                    .collect();
                state.panorama = Some(config);
            } else if metadata.input_type == ProjectInputType::Images {
                let input = metadata.source_path.clone();
                let stage_frames = stage_frames.clone();
                let stage_colmap = stage_colmap.clone();
                let stage_masks = stage_masks.clone();
                let cancel = cancel.clone();
                let total = crate::video::list_images(&input)?.len() as u64;
                let progress = ReshootProgress::new(
                    &self.events,
                    PipelineStage::ExtractingFrames,
                    &cancel,
                    "正在准备补拍画面",
                    Some(total),
                );
                crate::presets::spawn_pipeline_blocking(move || -> Result<()> {
                    for (index, image) in crate::video::list_images(&input)?.iter().enumerate() {
                        normalize_image(
                            image,
                            &stage_frames,
                            &stage_colmap,
                            &stage_masks,
                            &format!(
                                "frame_{index:010}.{}",
                                image.extension().unwrap().to_string_lossy()
                            ),
                            &cancel,
                        )?;
                        progress.update(index as u64 + 1, total);
                    }
                    Ok(())
                })
                .await
                .map_err(|e| invalid(&format!("Image preparation worker: {e}")))??;
            } else {
                let video = probe_video(
                    &self.engines.ffprobe,
                    &metadata.source_path,
                    None,
                    &self.process_manager,
                )
                .await?;
                let config = if is_panorama {
                    Some(panorama::PanoramaCheckpoint::new_for_policy(
                        &metadata.source_path,
                        metadata.quality,
                        &video,
                        &cancel,
                        state.generation_policy.is_some(),
                    )?)
                } else {
                    None
                };
                let dimensions = config
                    .as_ref()
                    .map(|c| (c.sfm_width, c.sfm_height))
                    .or_else(|| {
                        (recipe.uses_normal_parameters() && state.planner_enabled).then(|| {
                            let (w, h) = oriented_video_dimensions(&video);
                            let plan = self.resolution_plan(
                                metadata.quality,
                                acceleration.usable_gpu_total_memory_mb(),
                                w,
                                h,
                                true,
                            );
                            (plan.working_width, plan.working_height)
                        })
                    });
                let prepared = self
                    .prepare_frames(
                        &metadata.source_path,
                        metadata.quality,
                        &stage_frames,
                        &stage_colmap,
                        &stage_masks,
                        Some(&paths.logs),
                        state.planner_enabled,
                        Some(video),
                        dimensions,
                        is_panorama,
                    )
                    .await?;
                state.video = prepared.video;
                state.frames = Some(FrameState::from(&prepared.plan));
                if let Some(config) = config {
                    let times: Vec<panorama::FrameTime> = serde_json::from_slice(&std::fs::read(
                        stage.join("panorama-frame-times.json"),
                    )?)?;
                    for image in crate::video::list_images(&stage_frames)? {
                        let index = image
                            .file_stem()
                            .unwrap()
                            .to_string_lossy()
                            .strip_prefix("frame_")
                            .and_then(|v| v.parse::<u64>().ok())
                            .ok_or_else(|| invalid("Invalid panorama source index"))?;
                        let t = times
                            .iter()
                            .find(|t| t.source_frame_index == index)
                            .ok_or_else(|| invalid("Missing panorama timestamp"))?;
                        batch_records.push(PanoramaFrameRecord {
                            prepared_name: image
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .into_owned(),
                            batch_id: recipe.batch_id.clone(),
                            source_name: metadata
                                .source_path
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .into_owned(),
                            source_index: index,
                            source_sha256: config.source_sha256.clone(),
                            orientation: None,
                            timestamp_seconds: Some(t.timestamp_seconds),
                            pts: Some(t.pts),
                        });
                    }
                    state.panorama = Some(config);
                }
            }
            recipe.new_names.clear();
            let prepared_images = crate::video::list_images(&stage_frames)?;
            let progress = ReshootProgress::new(
                &self.events,
                PipelineStage::ExtractingFrames,
                &cancel,
                "正在整理补拍画面",
                Some(prepared_images.len() as u64),
            );
            for (index, image) in prepared_images.iter().enumerate() {
                let old = image.file_name().unwrap().to_string_lossy().into_owned();
                let name = format!(
                    "{prefix}{index:010}.{}",
                    image.extension().unwrap().to_string_lossy()
                );
                for (from, to) in [
                    (&stage_frames, &paths.frames),
                    (&stage_colmap, &paths.colmap_frames),
                ] {
                    copy_reshoot_file(&from.join(&old), &to.join(&name), &cancel).await?;
                }
                let mask = stage_masks.join(format!("{old}.png"));
                if mask.is_file() {
                    copy_reshoot_file(&mask, &paths.masks.join(format!("{name}.png")), &cancel)
                        .await?;
                }
                if let Some(mut record) = batch_records
                    .iter()
                    .find(|r| r.prepared_name == old)
                    .cloned()
                {
                    record.prepared_name = name.clone();
                    recipe.panorama_frames.push(record);
                }
                recipe.new_names.push(name);
                progress.update(index as u64 + 1, prepared_images.len() as u64);
            }
            drop(progress);
            let images = paths.colmap_frames.clone();
            let masks = paths.masks.clone();
            let mask_cancel = cancel.clone();
            let progress = ReshootProgress::new(
                &self.events,
                PipelineStage::ExtractingFrames,
                &cancel,
                "正在检查画面与蒙版",
                Some(recipe.source_names.len() as u64 + recipe.new_names.len() as u64),
            );
            crate::presets::spawn_pipeline_blocking(move || {
                ensure_masks(&images, &masks, &mask_cancel, Some(&progress))
            })
            .await
            .map_err(|e| invalid(&format!("Mask preparation worker: {e}")))??;
            checkpoint.reshoot_frame_count = recipe.new_names.len() as u64;
            checkpoint.supplemental_frames_complete = true;
            let total = checkpoint.source_image_count + checkpoint.reshoot_frame_count;
            state.frames = Some(FrameState {
                extracted_frames: Some(total),
                colmap_input_version: Some(COLMAP_INPUT_VERSION),
                estimated_frames: total,
                has_alpha: crate::video::list_images(&paths.frames)?.iter().any(|p| {
                    crate::video::image_projection::inspect(p).is_ok_and(|header| header.alpha)
                }),
                ..Default::default()
            });
            save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
            reshoot_log(
                &self.events,
                PipelineStage::ExtractingFrames,
                format!(
                    "补拍画面准备完成：原画面 {} 张，新增 {} 张",
                    checkpoint.source_image_count, checkpoint.reshoot_frame_count
                ),
            );
        } else {
            reshoot_log(
                &self.events,
                PipelineStage::ExtractingFrames,
                "复用已完成的补拍缓存：补拍画面",
            );
        }
        self.record_frame_metrics(&state, checkpoint.reshoot_frame_count, None);
        if recipe.uses_normal_parameters() && recipe.camera_assignment.is_none() {
            let headers = recipe
                .new_names
                .iter()
                .chain(&recipe.source_names)
                .map(|name| {
                    let image =
                        crate::video::image_projection::inspect(&paths.colmap_frames.join(name))?;
                    Ok((image.width, image.height, image.orientation))
                })
                .collect::<Result<Vec<_>>>()?;
            recipe.camera_assignment = Some(CameraAssignment {
                version: 1,
                reused_camera_id: if is_panorama {
                    None
                } else {
                    // Be conservative even if only one of multiple source
                    // camera groups survived into the registered model.
                    let source_db = rusqlite::Connection::open_with_flags(
                        &database,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )
                    .map_err(|e| invalid(&e.to_string()))?;
                    let count: u64 = source_db
                        .query_row("SELECT COUNT(*) FROM cameras", [], |row| row.get(0))
                        .map_err(|e| invalid(&e.to_string()))?;
                    if count == 1 {
                        reusable_camera(
                            recipe.options.camera_relation,
                            &read_cameras(&base)?,
                            &headers,
                        )
                    } else {
                        None
                    }
                },
            });
            save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
        }
        let reused_camera_id = recipe
            .camera_assignment
            .as_ref()
            .and_then(|v| v.reused_camera_id);
        reshoot_log(
            &self.events,
            PipelineStage::ExtractingFeatures,
            if reused_camera_id.is_some() {
                "补拍相机设置：沿用原设置"
            } else if recipe.options.camera_relation == Some(CameraRelation::Same) && !is_panorama {
                "补拍相机设置：条件不兼容，独立处理"
            } else {
                "补拍相机设置：独立处理"
            },
        );
        if recipe.uses_normal_parameters()
            && !is_panorama
            && state.planner_enabled
            && state.resolution_plan.is_none()
        {
            let mut largest: (u32, u32) = (1, 1);
            for name in recipe.source_names.iter().chain(&recipe.new_names) {
                let header = crate::video::image_projection::inspect(&paths.frames.join(name))?;
                if header.width.max(header.height) > largest.0.max(largest.1) {
                    largest = (header.width, header.height);
                }
            }
            state.resolution_plan = Some(self.resolution_plan(
                metadata.quality,
                acceleration.usable_gpu_total_memory_mb(),
                largest.0,
                largest.1,
                metadata.input_type == ProjectInputType::Video,
            ));
            save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
        }
        let sfm = metadata.quality.preset();
        if state.generation_policy.is_some() {
            let candidate_alpha =
                checkpoint.has_alpha || state.frames.as_ref().is_some_and(|f| f.has_alpha);
            self.actual_transparency(paths, &mut state, candidate_alpha)
                .await?;
        }
        if !checkpoint.supplemental_features_complete {
            self.events.stage(
                PipelineStage::ExtractingFeatures,
                0.,
                "正在提取补拍画面的信息",
            );
            // Reset to the immutable base database: interrupted groups must not receive different IDs on resume.
            tokio::fs::copy(&source.database, &database).await?;
            sync_refined_cameras(&database, &base)?;
            let groups: Vec<Vec<String>> = if metadata.input_type == ProjectInputType::Video
                || is_panorama
                || reused_camera_id.is_some()
            {
                vec![recipe.new_names.clone()]
            } else {
                recipe.new_names.iter().map(|n| vec![n.clone()]).collect()
            };
            for (index, group) in groups.iter().enumerate() {
                if cancel.is_cancelled() {
                    return Err(SplatError::Cancelled);
                }
                let list = paths.colmap.join(format!("group-{index}.txt"));
                write_image_list(&list, group).await?;
                let dims = if is_panorama {
                    let h = crate::video::image_projection::inspect(
                        &paths.colmap_frames.join(&group[0]),
                    )?;
                    Some((h.width, h.height))
                } else {
                    None
                };
                colmap::extract_reshoot_group(
                    &self.engines.colmap,
                    &database,
                    colmap::ReshootFeatureOptions {
                        images: &paths.colmap_frames,
                        list: &list,
                        masks: Some(&paths.masks),
                        panorama_dimensions: dims,
                        existing_camera_id: reused_camera_id,
                        max_size: if state.planner_enabled {
                            state
                                .resolution_plan
                                .map_or(sfm.planned_sfm_max_image_size, |p| p.sfm_max_image_size)
                        } else {
                            sfm.sfm_max_image_size
                        },
                        max_features: if state.planner_enabled {
                            sfm.planned_sfm_max_features
                        } else {
                            sfm.sfm_max_features
                        },
                    },
                    paths.logs.join("colmap.log"),
                    &self.process_manager,
                    Some(self.process_observer(
                        PipelineStage::ExtractingFeatures,
                        PipelineEngine::Colmap,
                        Some(group.len() as u64),
                        ObserverMode::BracketProgress,
                    )),
                    acceleration.gpu_index(),
                )
                .await?;
            }
            checkpoint.supplemental_features_complete = true;
            save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
        } else {
            reshoot_log(
                &self.events,
                PipelineStage::ExtractingFeatures,
                "复用已完成的补拍缓存：特征提取",
            );
        }
        self.reshoot_matching(
            paths,
            &state,
            &recipe,
            acceleration,
            recipe.expanded_matching,
        )
        .await?;
        checkpoint.incremental_matching_complete = true;
        save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
        if recipe.actual.reconstruction == ReconstructionMode::Full
            && !checkpoint.incremental_reconstruction_complete
        {
            self.reshoot_full_pairs(paths, &recipe, acceleration)
                .await?;
        }
        let model = loop {
            let output = paths.colmap.join(
                if recipe.actual.reconstruction == ReconstructionMode::Full {
                    "sparse-full"
                } else {
                    "sparse"
                },
            );
            if !checkpoint.incremental_reconstruction_complete {
                reset_directory(&output).await?;
                let mut names = recipe.source_names.clone();
                names.extend(recipe.new_names.clone());
                let list = paths.colmap.join("mapper-images.txt");
                write_image_list(&list, &names).await?;
                let fixed = paths.colmap.join("fixed-cameras.txt");
                let ids = read_cameras(&base)?
                    .into_iter()
                    .map(|c| c.id.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                tokio::fs::write(&fixed, ids).await?;
                self.events.stage(
                    PipelineStage::Reconstructing,
                    0.,
                    "正在把补拍画面注册到原相机重建中",
                );
                colmap::map_reshoot(
                    &self.engines.colmap,
                    &database,
                    colmap::ReshootMapperOptions {
                        allow_two_view_tracks: state.generation_policy.is_some()
                            && state.actual_transparency == Some(true),
                        images: &paths.colmap_frames,
                        input: (recipe.actual.reconstruction == ReconstructionMode::Incremental)
                            .then_some(base.as_path()),
                        output: &output,
                        image_list: &list,
                        fixed_cameras: (recipe.actual.reconstruction
                            == ReconstructionMode::Incremental)
                            .then_some(fixed.as_path()),
                        panorama: is_panorama,
                    },
                    paths.logs.join("colmap.log"),
                    &self.process_manager,
                    Some(self.process_observer(
                        PipelineStage::Reconstructing,
                        PipelineEngine::Colmap,
                        Some(names.len() as u64),
                        ObserverMode::Mapper,
                    )),
                )
                .await?;
            }
            let (model, _) = best_sparse_model(&paths.frames, &output).await?;
            let poses = panorama::read_poses(&model)?;
            let added = poses
                .iter()
                .filter(|p| recipe.new_names.contains(&p.name))
                .count();
            let compatible = recipe.actual.reconstruction == ReconstructionMode::Full
                || coordinates_match(&base, &model)?;
            if added == 0
                && recipe.actual.reconstruction == ReconstructionMode::Incremental
                && !recipe.expanded_matching
            {
                recipe.expanded_matching = true;
                save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
                self.reshoot_matching(paths, &state, &recipe, acceleration, true)
                    .await?;
                checkpoint.incremental_reconstruction_complete = false;
                continue;
            }
            if added == 0 || !compatible {
                let reason = if added == 0 {
                    AlgorithmFailure::NoNewRegistrations
                } else {
                    AlgorithmFailure::CoordinateMismatch
                };
                if recipe.fallback(reason) {
                    checkpoint.incremental_reconstruction_complete = false;
                    state.brush_complete = false;
                    save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
                    self.events.stage(
                        PipelineStage::Reconstructing,
                        0.,
                        "当前方案未能完成，正在自动改用更耗时的方案。",
                    );
                    // Full reconstruction needs a connected graph including saved, unregistered images.
                    self.reshoot_full_pairs(paths, &recipe, acceleration)
                        .await?;
                    continue;
                }
                return Err(invalid(
                    "补拍画面没有成功加入原项目。请确保新旧素材有足够重叠。",
                ));
            }
            checkpoint.registered_reshoot_count = added as u64;
            checkpoint.incremental_reconstruction_complete = true;
            save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
            break model;
        };
        let model = if state.gps.is_some() && metadata.input_type == ProjectInputType::Images {
            let snapshot = metadata.clone();
            let current_paths = paths.clone();
            let current_recipe = recipe.clone();
            let cancel = cancel.clone();
            let (records, origin) = tokio::task::spawn_blocking(move || {
                super::gps_runner::reshoot_records(
                    &snapshot,
                    &current_paths,
                    &current_recipe,
                    &cancel,
                )
            })
            .await
            .map_err(|e| SplatError::Process(e.to_string()))??;
            let fixed = paths.colmap.join("fixed-cameras.txt");
            self.apply_gps(
                manager,
                paths,
                &mut state,
                records,
                origin,
                &model,
                (recipe.actual.reconstruction == ReconstructionMode::Incremental)
                    .then_some(base.as_path()),
                (recipe.actual.reconstruction == ReconstructionMode::Incremental)
                    .then_some(fixed.as_path()),
            )
            .await?
            .unwrap_or(model)
        } else {
            model
        };
        let report = ReconstructionValidator::validate(&paths.frames, &model)?;
        self.record_reconstruction_metrics(
            &report,
            acceleration,
            state.generation_policy.is_some() && state.actual_transparency == Some(true),
            true,
        );
        reshoot_log(
            &self.events,
            PipelineStage::Reconstructing,
            format!(
                "补拍画面已加入：{}/{}",
                checkpoint.registered_reshoot_count, checkpoint.reshoot_frame_count
            ),
        );
        // The published sparse model contains refined new intrinsics. Record
        // them in this derived project's DB, never in the source project's DB.
        sync_refined_cameras(&database, &model)?;
        if !is_panorama {
            self.events
                .stage(PipelineStage::TrainingSplats, 0., "正在准备训练图片");
        }
        let dataset = if is_panorama {
            self.events.panorama_phase.store(2, Ordering::Relaxed);
            self.events
                .stage(PipelineStage::TrainingSplats, 0., "正在准备训练图片");
            let root = paths.work.join("panorama-training");
            let frames = paths.frames.clone();
            let model = model.clone();
            let records = recipe.panorama_frames.clone();
            let quality = metadata.quality;
            let native = state.generation_policy.is_some();
            let cancel = cancel.clone();
            let events = self.events.clone();
            crate::presets::spawn_pipeline_blocking(move || {
                panorama::prepare_reshoot_training_cache_for_policy(
                    &frames,
                    &model,
                    &root,
                    &records,
                    quality,
                    native,
                    &cancel,
                    |update| match update {
                        panorama::TrainingCacheUpdate::Faces {
                            completed, total, ..
                        } => events.send(
                            PipelineStage::TrainingSplats,
                            Some(PipelineEngine::System),
                            EventKind::Progress,
                            EventLevel::Info,
                            Some(completed as f32 / total.max(1) as f32),
                            false,
                            "正在准备训练图片",
                            Some(completed),
                            Some(total),
                            Some("faces"),
                        ),
                        panorama::TrainingCacheUpdate::Validating => {
                            events.panorama_phase.store(3, Ordering::Relaxed);
                            events.stage(
                                PipelineStage::TrainingSplats,
                                0.,
                                "正在校验训练图片和相机数据",
                            );
                        }
                    },
                )
            })
            .await
            .map_err(|e| invalid(&format!("Panorama conversion worker: {e}")))??
        } else {
            let dataset = paths.brush.join("dataset");
            if !state.brush_complete {
                let _progress = ReshootProgress::new(
                    &self.events,
                    PipelineStage::TrainingSplats,
                    &cancel,
                    "正在准备训练图片",
                    None,
                );
                reset_directory(&dataset).await?;
                prepare_brush_dataset(&paths.brush, &paths.frames, &model).await?
            } else {
                dataset
            }
        };
        self.events
            .panorama_phase
            .store(u64::from(is_panorama), Ordering::Relaxed);
        let mut previous_training_failure = None;
        let candidate = loop {
            self.prepare_brush_observation(&state);
            let mut attempt = paths.clone();
            let name = format!(
                "attempt-{}-{}",
                recipe.reconstruction_fallbacks, recipe.training_fallbacks
            );
            attempt.brush = paths.brush.join(&name);
            attempt.logs = paths.logs.join(&name);
            tokio::fs::create_dir_all(&attempt.logs).await?;
            if state.brush_complete {
                if let Some(candidate) = brush_candidate(&attempt.brush) {
                    let _progress = ReshootProgress::new(
                        &self.events,
                        PipelineStage::TrainingSplats,
                        &cancel,
                        "正在校验并发布 final.ply",
                        None,
                    );
                    validate_ply(&candidate, &cancel).await?;
                    reshoot_log(
                        &self.events,
                        PipelineStage::TrainingSplats,
                        "复用已完成的补拍缓存：Splat 训练",
                    );
                    break candidate;
                }
                state.brush_complete = false;
            }
            let policy = brush::BrushGpuLaunchPolicy::from_acceleration(acceleration);
            let max_edge = recipe
                .source_names
                .iter()
                .chain(&recipe.new_names)
                .filter_map(|n| crate::video::image_projection::inspect(&paths.frames.join(n)).ok())
                .map(|i| i.width.max(i.height))
                .max()
                .unwrap_or(1);
            let face_size = if state.generation_policy.is_some()
                && metadata.quality == Quality::High
                && is_panorama
            {
                // Faces live in per-source subdirectories. Their validated
                // training cameras, not a shallow image listing, define size.
                read_cameras(&dataset.join("sparse/0"))?
                    .iter()
                    .map(|c| c.width.max(c.height))
                    .max()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| invalid("Invalid panorama training camera dimensions"))?
            } else {
                match metadata.quality {
                    Quality::Fast => 512,
                    Quality::Balanced => 768,
                    Quality::High => 1024,
                }
            };
            let mut initial_points = report.points_3d;
            let mut initial_sh = 3;
            if recipe.actual.training == TrainingMode::Warm {
                let _progress = ReshootProgress::new(
                    &self.events,
                    PipelineStage::TrainingSplats,
                    &cancel,
                    "正在准备原模型训练副本",
                    None,
                );
                let init = dataset.join("init.ply");
                let model = provenance.source_final_ply.clone();
                let (original_info, original_sh, original_hash) =
                    validate_ply(&model, &cancel).await?;
                let gps_transform = state
                    .gps
                    .as_ref()
                    .filter(|g| g.status == "complete")
                    .and_then(|g| g.transform);
                let gps_key = gps_transform
                    .map(|t| serde_json::to_string(&t))
                    .transpose()?;
                if !init.is_file()
                    || recipe.init_sha256.as_deref() != Some(&original_hash)
                    || recipe.init_georeference != gps_key
                {
                    let pending = dataset.join("init.ply.partial");
                    if let Some(transform) = gps_transform {
                        let source = model.clone();
                        let output = pending.clone();
                        let cancellation = cancel.clone();
                        tokio::task::spawn_blocking(move || {
                            crate::reconstruction::splat_transform::export_transformed_ply_to(
                                &source,
                                &output,
                                transform.engine_transform(),
                                Default::default(),
                                |_, _| {},
                                || {
                                    if cancellation.is_cancelled() {
                                        Err(SplatError::Cancelled)
                                    } else {
                                        Ok(())
                                    }
                                },
                            )
                        })
                        .await
                        .map_err(|e| invalid(&e.to_string()))??;
                    } else {
                        copy_reshoot_file(&model, &pending, &cancel).await?;
                    }
                    let (snapshot_info, snapshot_sh, snapshot_hash) =
                        validate_ply(&pending, &cancel).await?;
                    if snapshot_info.splat_count != original_info.splat_count
                        || snapshot_sh != original_sh
                        || (gps_transform.is_none() && snapshot_hash != original_hash)
                    {
                        return Err(changed());
                    }
                    atomic_replace_file(&pending, &init).await?;
                    recipe.init_sha256 = Some(original_hash);
                    recipe.init_georeference = gps_key;
                }
                let (info, sh, _) = validate_ply(&init, &cancel).await?;
                initial_points = info.splat_count;
                initial_sh = sh;
                if is_panorama
                    && !(state.generation_policy.is_some() && metadata.quality == Quality::High)
                    && initial_points > crate::presets::MAX_PANORAMA_SPLATS as u64
                {
                    return Err(invalid(
                        "初始化点数超出当前全景训练预算，已停止训练以避免丢弃初始点。",
                    ));
                }
            } else {
                let init = dataset.join("init.ply");
                if init.is_file() {
                    tokio::fs::rename(init, dataset.join("init.previous")).await?;
                }
                recipe.warm_parameters = None;
            }
            self.prepare_fine_splat_budget(
                &mut state,
                metadata.quality,
                &model,
                &dataset.join("images"),
                is_panorama,
            )
            .await?;
            // Resolve after validating initialization, using the normal budget
            // protection. Existing jobs retain their saved warm-start recipe.
            let budget_points = if recipe.uses_normal_parameters() {
                initial_points
            } else {
                report.points_3d
            };
            let mut resolved = if let Some(saved) = state.brush_training.resolved {
                saved
            } else if let Some(generation) = state.generation_policy {
                let memory = if is_panorama {
                    policy.known_training_memory_mb(acceleration)
                } else {
                    acceleration.usable_gpu_total_memory_mb()
                };
                if is_panorama {
                    generation.panorama_training(
                        metadata.quality,
                        memory,
                        face_size,
                        report.points_3d,
                        initial_points,
                    )?
                } else {
                    let resolution = state.resolution_plan.unwrap_or_else(|| {
                        generation.resolution(metadata.quality, memory, max_edge, max_edge, false)
                    });
                    generation.training(
                        metadata.quality,
                        memory,
                        &resolution,
                        report.points_3d,
                        initial_points,
                        state.brush_training.fine_splat_budget.as_ref(),
                    )?
                }
            } else {
                normal_training_preset(
                    metadata.quality,
                    state.planner_enabled,
                    if is_panorama {
                        policy.known_training_memory_mb(acceleration)
                    } else {
                        acceleration.usable_gpu_total_memory_mb()
                    },
                    max_edge,
                    is_panorama.then_some(face_size),
                    budget_points,
                    state
                        .resolution_plan
                        .filter(|_| recipe.uses_normal_parameters()),
                )?
            };
            if recipe.uses_normal_parameters() {
                recipe.warm_parameters = None;
            } else if recipe.actual.training == TrainingMode::Warm {
                let views = read_registered_images(&dataset.join("sparse/0"))?.len() as u64;
                if recipe.warm_parameters.is_none() {
                    let (preset, warm) = warm_training(
                        resolved.preset,
                        metadata.quality,
                        views,
                        initial_points,
                        initial_sh,
                    )?;
                    resolved.preset = preset;
                    recipe.warm_parameters = Some(warm);
                }
                if let Some(snapshot) = resolved.panorama.as_mut() {
                    snapshot.initial_points = initial_points;
                    snapshot.initial_points_override = resolved
                        .configured_max_splats
                        .is_some_and(|cap| initial_points > cap as u64)
                        || snapshot
                            .memory_max_splats
                            .is_some_and(|cap| initial_points > cap as u64);
                }
            }
            if !state.brush_training.oom_retry_used && state.brush_training.resolved.is_none() {
                state
                    .brush_training
                    .initial_resolved
                    .get_or_insert(resolved);
            }
            state.brush_training.resolved = Some(resolved);
            if let Some(tracker) = &self.effectiveness {
                tracker.prepare_brush(
                    state.brush_training.initial_resolved,
                    Some(resolved),
                    state.brush_training.oom_retry_used,
                );
            }
            save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
            self.events
                .stage(PipelineStage::TrainingSplats, 0., "正在生成高斯泼溅");
            let outcome = self
                .train_brush_with_oom_retry(
                    manager,
                    &attempt,
                    &mut state,
                    metadata.quality,
                    acceleration,
                    &dataset,
                    initial_points,
                    resolved,
                    0,
                )
                .await;
            let candidate = match outcome {
                Ok(path) => path,
                Err(error)
                    if explicit_numeric_divergence(&error)
                        && recipe.fallback(AlgorithmFailure::InvalidWarmResult) =>
                {
                    previous_training_failure = Some(error);
                    state.brush_training.resolved = None;
                    save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
                    self.events.stage(
                        PipelineStage::TrainingSplats,
                        0.,
                        "当前方案未能完成，正在自动改用更耗时的方案。",
                    );
                    continue;
                }
                Err(error) => {
                    return Err(if let Some(previous) = previous_training_failure.take() {
                        error.with_previous_attempt(previous)
                    } else {
                        error
                    })
                }
            };
            let validation = {
                let _progress = ReshootProgress::new(
                    &self.events,
                    PipelineStage::TrainingSplats,
                    &cancel,
                    "正在校验并发布 final.ply",
                    None,
                );
                validate_ply(&candidate, &cancel).await
            };
            match validation {
                Ok(_) => {
                    state.brush_complete = true;
                    save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
                    break candidate;
                }
                Err(error) => {
                    if !matches!(error, SplatError::Io(_) | SplatError::Cancelled)
                        && recipe.fallback(AlgorithmFailure::InvalidWarmResult)
                    {
                        state.brush_training.resolved = None;
                        save_recipe(manager, paths, &mut state, &checkpoint, &recipe).await?;
                        self.events.stage(
                            PipelineStage::TrainingSplats,
                            0.,
                            "当前方案未能完成，正在自动改用更耗时的方案。",
                        );
                    } else {
                        return Err(error);
                    }
                }
            }
        };
        self.publish_reshoot_v2(
            manager,
            paths,
            metadata,
            &mut state,
            &checkpoint,
            &report,
            &candidate,
        )
        .await
    }

    async fn reshoot_matching(
        &self,
        paths: &ProjectPaths,
        state: &PipelineStateFile,
        recipe: &ReshootRecipe,
        acceleration: &crate::engines::ColmapAccelerationStatus,
        expanded: bool,
    ) -> Result<()> {
        if state
            .reshoot
            .as_ref()
            .is_some_and(|s| s.incremental_matching_complete)
            && !expanded
        {
            reshoot_log(
                &self.events,
                PipelineStage::Matching,
                "复用已完成的补拍缓存：图像匹配",
            );
            return Ok(());
        }
        self.events
            .stage(PipelineStage::Matching, 0., "正在补充新旧画面之间的联系");
        let database = paths.colmap.join("database.db");
        let list = paths.colmap.join("reshoot-images.txt");
        write_image_list(&list, &recipe.new_names).await?;
        use std::io::Write;
        let pair_list = paths.colmap.join("reshoot-pairs.txt");
        let mut pairs = std::io::BufWriter::new(std::fs::File::create(&pair_list)?);
        if recipe
            .source_names
            .len()
            .saturating_mul(recipe.new_names.len())
            <= 20_000
        {
            for old in &recipe.source_names {
                for new in &recipe.new_names {
                    writeln!(pairs, "{old} {new}")?;
                }
            }
        } else {
            let registered = read_registered_images(&paths.colmap.join("base-model"))?;
            let old: Vec<_> = recipe
                .source_names
                .iter()
                .filter(|n| registered.iter().any(|image| image.name == **n))
                .cloned()
                .collect();
            for (old, new) in colmap::retrieve_reshoot_pairs(
                &self.engines.colmap,
                &database,
                &old,
                &recipe.new_names,
                if expanded { 60 } else { 30 },
                paths.logs.join("colmap.log"),
                &self.process_manager,
                Some(self.process_observer(
                    PipelineStage::Matching,
                    PipelineEngine::Colmap,
                    None,
                    ObserverMode::Matching,
                )),
            )
            .await?
            {
                writeln!(pairs, "{old} {new}")?;
            }
        }
        let sequential = state.input_type == ProjectInputType::Video
            || (recipe.new_names.len() > 100
                && recipe.options.image_matching_strategy
                    == crate::project::ImageMatchingStrategy::Sequential);
        for (i, a) in recipe.new_names.iter().enumerate() {
            if self.process_manager.child_token().is_cancelled() {
                return Err(SplatError::Cancelled);
            }
            for b in
                recipe
                    .new_names
                    .iter()
                    .skip(i + 1)
                    .take(if sequential { 10 } else { usize::MAX })
            {
                writeln!(pairs, "{a} {b}")?;
            }
        }
        pairs.flush()?;
        drop(pairs);
        if std::fs::metadata(&pair_list)?.len() > 0 {
            colmap::match_pairs(
                &self.engines.colmap,
                &database,
                &pair_list,
                paths.logs.join("colmap.log"),
                &self.process_manager,
                Some(self.process_observer(
                    PipelineStage::Matching,
                    PipelineEngine::Colmap,
                    None,
                    ObserverMode::Matching,
                )),
                acceleration.gpu_index(),
            )
            .await?;
        }
        if sequential && state.preset.loop_detection_enabled(state.planner_enabled) {
            colmap::match_reshoot_candidates(
                &self.engines.colmap,
                &database,
                &list,
                50,
                paths.logs.join("colmap.log"),
                &self.process_manager,
                Some(self.process_observer(
                    PipelineStage::Matching,
                    PipelineEngine::Colmap,
                    None,
                    ObserverMode::Matching,
                )),
                acceleration.gpu_index(),
            )
            .await?;
        }
        Ok(())
    }

    async fn reshoot_full_pairs(
        &self,
        paths: &ProjectPaths,
        recipe: &ReshootRecipe,
        acceleration: &crate::engines::ColmapAccelerationStatus,
    ) -> Result<()> {
        let all: Vec<_> = recipe
            .source_names
            .iter()
            .chain(&recipe.new_names)
            .cloned()
            .collect();
        let database = paths.colmap.join("database.db");
        if all.len() <= 200 {
            colmap::match_exhaustive(
                &self.engines.colmap,
                &database,
                paths.logs.join("colmap.log"),
                &self.process_manager,
                Some(self.process_observer(
                    PipelineStage::Matching,
                    PipelineEngine::Colmap,
                    None,
                    ObserverMode::Matching,
                )),
                acceleration.gpu_index(),
            )
            .await?;
        } else {
            let list = paths.colmap.join("all-images.txt");
            write_image_list(&list, &all).await?;
            colmap::match_reshoot_candidates(
                &self.engines.colmap,
                &database,
                &list,
                60,
                paths.logs.join("colmap.log"),
                &self.process_manager,
                Some(self.process_observer(
                    PipelineStage::Matching,
                    PipelineEngine::Colmap,
                    None,
                    ObserverMode::Matching,
                )),
                acceleration.gpu_index(),
            )
            .await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish_reshoot_v2(
        &self,
        manager: &ProjectManager,
        paths: &ProjectPaths,
        metadata: &mut ProjectMetadata,
        state: &mut PipelineStateFile,
        checkpoint: &ReshootState,
        report: &ReconstructionReport,
        candidate: &Path,
    ) -> Result<PipelineResult> {
        self.events
            .stage(PipelineStage::Exporting, 0., "正在校验并发布 final.ply");
        let ply = inspect_gaussian_ply(candidate)?;
        let final_ply = paths.project.join("final.ply");
        atomic_replace_file(candidate, &final_ply).await?;
        let completed_at = Utc::now();
        let duration_ms = metadata.duration_ms.unwrap_or(0).saturating_add(
            metadata
                .started_at
                .map(|t| (completed_at - t).num_milliseconds().max(0) as u64)
                .unwrap_or(0),
        );
        if let Some(provenance) = metadata.reshoot.as_mut() {
            provenance.registered_reshoot_count = checkpoint.registered_reshoot_count;
            provenance.reshoot_frame_count = checkpoint.reshoot_frame_count;
        }
        metadata.status = ProjectStatus::Completed;
        metadata.completed_at = Some(completed_at);
        metadata.duration_ms = Some(duration_ms);
        metadata.output = Some(ProjectOutput {
            final_ply: final_ply.clone(),
            file_size: ply.file_size,
            splat_count: ply.splat_count,
            input_images: report.input_images,
            registered_images: report.registered_images,
            registered_ratio: report.registered_ratio,
            points_3d: report.points_3d,
        });
        state.stage = PipelineStage::Completed;
        manager.write_state(&paths.state, state).await?;
        manager.write_metadata(&paths.metadata, metadata).await?;
        if let Some(output) = &metadata.output {
            self.record_final_metrics(output);
        }
        let warning = (checkpoint.registered_reshoot_count as f64
            / (checkpoint.reshoot_frame_count.max(1) as f64)
            < crate::reconstruction::good_registered_ratio())
        .then(|| {
            format!(
                "补拍画面成功加入 {}/{}，结果已生成，但部分补拍画面未能找到与原项目的联系。",
                checkpoint.registered_reshoot_count, checkpoint.reshoot_frame_count
            )
        });
        self.events.stage(
            PipelineStage::Exporting,
            1.,
            format!("已发布 {} 个 Splat", ply.splat_count),
        );
        self.events
            .stage(PipelineStage::Completed, 1., "高清补拍处理完成");
        Ok(PipelineResult {
            project_id: paths.id.to_string(),
            project_path: paths.project.clone(),
            final_ply,
            file_size: ply.file_size,
            splat_count: ply.splat_count,
            input_images: report.input_images,
            registered_images: report.registered_images,
            registered_ratio: report.registered_ratio,
            points_3d: report.points_3d,
            duration_ms,
            completed_at,
            warning,
            logs_directory: paths.logs.clone(),
            source_duration_seconds: None,
        })
    }
}

pub(super) fn coordinates_match(base: &Path, updated: &Path) -> Result<bool> {
    let before = read_cameras(base)?;
    let after = read_cameras(updated)?;
    for camera in before {
        if !after.iter().any(|c| c.id == camera.id && c == &camera) {
            return Ok(false);
        }
    }
    let old = panorama::read_poses(base)?;
    let new = panorama::read_poses(updated)?;
    for pose in old {
        let Some(other) = new
            .iter()
            .find(|p| p.name == pose.name && p.camera_id == pose.camera_id)
        else {
            return Ok(false);
        };
        let dot: f64 = pose.qvec.iter().zip(other.qvec).map(|(a, b)| a * b).sum();
        if (1. - dot.abs()).abs() > 1e-10
            || pose
                .tvec
                .iter()
                .zip(other.tvec)
                .any(|(a, b)| (a - b).abs() > 1e-8 * (1. + a.abs()))
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sync_refined_cameras(database: &Path, model: &Path) -> Result<()> {
    let connection = rusqlite::Connection::open(database).map_err(|e| invalid(&e.to_string()))?;
    for camera in read_cameras(model)? {
        let params: Vec<_> = camera.params.iter().flat_map(|v| v.to_le_bytes()).collect();
        connection
            .execute(
                "UPDATE cameras SET params=?1 WHERE camera_id=?2",
                rusqlite::params![params, camera.id],
            )
            .map_err(|e| invalid(&e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires the completed public GPS smoke project and native engines/GPU; repeated photographs test integration, not new detail"]
    async fn native_public_gps_reshoot() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let original = repo.join(".cache/gps-v1/public-projects/20261026-203054_images");
        let source_metadata: ProjectMetadata =
            serde_json::from_slice(&std::fs::read(original.join("project.json")).unwrap()).unwrap();
        let source_state: PipelineStateFile =
            serde_json::from_slice(&std::fs::read(original.join("state.json")).unwrap()).unwrap();
        let source = SourceAssets {
            root: original.clone(),
            metadata: source_metadata.clone(),
            state: source_state,
            images: original.join("work/frames"),
            colmap_images: original.join("work/colmap_frames"),
            masks: original.join("work/masks"),
            database: original.join("work/colmap/database.db"),
            model: original.join("work/colmap/gps-v1/model"),
        };
        let before = source_identity(&source).unwrap();
        let protected = [
            "final.ply",
            "project.json",
            "state.json",
            "work/colmap/database.db",
        ];
        let hashes: Vec<_> = protected
            .iter()
            .map(|n| panorama::fingerprint(&original.join(n)).unwrap())
            .collect();
        let camera = read_cameras(&source.model).unwrap().remove(0);
        let root = repo
            .join(".cache/gps-v1/reshoot-projects")
            .join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("input");
        std::fs::create_dir(&input).unwrap();
        let originals = crate::video::list_images(&source_metadata.source_path).unwrap();
        // Repeated real photographs deliberately isolate registration/coordinate
        // and warm-init correctness. They cannot assess added scene coverage.
        for p in originals.iter().take(6) {
            std::fs::copy(p, input.join(p.file_name().unwrap())).unwrap();
        }
        let manager = ProjectManager::for_diagnostics(root.join("projects"));
        let (paths, mut metadata) = manager
            .create_reshoot_v2(
                &input,
                Quality::Fast,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap();
        metadata.reshoot = Some(ReshootProvenance {
            source_project_id: source_metadata.id,
            source_project_path: original.clone(),
            source_final_ply: original.join("final.ply"),
            reshoot_source_path: metadata.source_path.clone(),
            camera_id: camera.id,
            camera_model: camera.model.clone(),
            width: camera.width,
            height: camera.height,
            has_alpha: false,
            mask_count: 0,
            source_image_count: 18,
            reshoot_frame_count: 6,
            registered_reshoot_count: 0,
        });
        let mut state = manager.read_state(&paths.state).await.unwrap();
        state.gps = Some(crate::reconstruction::gps::Checkpoint::pending());
        let options = ReshootOptions {
            condition: ModelCondition::Good,
            camera_relation: Some(CameraRelation::Same),
            allow_fallback: false,
            source_identity: before.clone(),
            input_identity: panorama::source_identity(&input).unwrap(),
            projection: VideoProjection::Perspective,
            image_matching_strategy: crate::project::ImageMatchingStrategy::Exhaustive,
        };
        let plan = ReshootPlan::resolve(ModelCondition::Good, true, true);
        state.reshoot = Some(ReshootState {
            recipe: Some(ReshootRecipe {
                version: RECIPE_VERSION,
                normal_parameters_version: Some(1),
                camera_assignment: None,
                options,
                initial: plan,
                actual: plan,
                batch_id: paths.id.simple().to_string(),
                reconstruction_fallbacks: 0,
                training_fallbacks: 0,
                expanded_matching: false,
                source_names: vec![],
                new_names: vec![],
                warm_parameters: None,
                init_sha256: None,
                init_georeference: None,
                stored_input_identity: Some(
                    panorama::source_identity(&metadata.source_path).unwrap(),
                ),
                panorama_frames: vec![],
            }),
            ..Default::default()
        });
        manager.write_state(&paths.state, &state).await.unwrap();
        manager
            .write_metadata(&paths.metadata, &metadata)
            .await
            .unwrap();
        let runner = PipelineRunner::new(EnginePaths::from_root(repo.join("engines")), |event| {
            if event.message.starts_with("[GPS]")
                || event.message == crate::reconstruction::gps::FALLBACK_MESSAGE
            {
                println!("{}", event.message);
            }
        });
        let acceleration = runner.verify_pipeline_engines().await.unwrap();
        let result = runner
            .execute_reshoot_v2(
                &manager,
                &paths,
                &mut metadata,
                state,
                &acceleration,
                source,
            )
            .await
            .unwrap();
        let state = manager.read_state(&paths.state).await.unwrap();
        println!(
            "GPS reshoot: {} ; {} splats; GPS {}",
            paths.project.display(),
            result.splat_count,
            state.gps.as_ref().unwrap().status
        );
        assert_eq!(state.gps.unwrap().status, "complete");
        assert!(result.splat_count > 0);
        assert!(state
            .reshoot
            .unwrap()
            .recipe
            .unwrap()
            .init_georeference
            .is_some());
        for (name, expected) in protected.iter().zip(hashes) {
            assert_eq!(
                panorama::fingerprint(&original.join(name)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn reshoot_preparation_quiet_notice_is_bounded_and_resets_on_activity() {
        let now = Instant::now();
        let mut clock = PreparationClock {
            activity: now,
            logged: now,
            notice: None,
        };
        assert!(!clock.quiet_notice(now + Duration::from_secs(14)));
        assert!(clock.quiet_notice(now + Duration::from_secs(15)));
        assert!(!clock.quiet_notice(now + Duration::from_secs(44)));
        assert!(clock.quiet_notice(now + Duration::from_secs(45)));
        clock.activity = now + Duration::from_secs(46);
        clock.notice = None;
        assert!(!clock.quiet_notice(now + Duration::from_secs(60)));
        assert!(clock.quiet_notice(now + Duration::from_secs(61)));
    }

    #[test]
    fn reshoot_camera_reuse_requires_explicit_confirmation_and_compatible_pixels() {
        let camera = ColmapCamera {
            id: 7,
            model_id: 2,
            model: "SIMPLE_RADIAL".into(),
            width: 1920,
            height: 1080,
            params: vec![1500., 960., 540., 0.],
        };
        let frames = [(1920, 1080, 1), (1920, 1080, 1)];
        assert_eq!(
            reusable_camera(
                Some(CameraRelation::Same),
                std::slice::from_ref(&camera),
                &frames
            ),
            Some(7)
        );
        for relation in [
            None,
            Some(CameraRelation::Different),
            Some(CameraRelation::Unsure),
        ] {
            assert_eq!(
                reusable_camera(relation, std::slice::from_ref(&camera), &frames),
                None
            );
        }
        for frames in [
            vec![],
            vec![(1080, 1920, 1)],
            vec![(1920, 1080, 3)],
            vec![(1920, 1080, 1), (1280, 720, 1)],
        ] {
            assert_eq!(
                reusable_camera(
                    Some(CameraRelation::Same),
                    std::slice::from_ref(&camera),
                    &frames
                ),
                None
            );
        }
        assert_eq!(
            reusable_camera(
                Some(CameraRelation::Same),
                &[camera.clone(), camera.clone()],
                &frames
            ),
            None
        );
        for invalid in [
            ColmapCamera {
                model_id: 17,
                ..camera.clone()
            },
            ColmapCamera {
                params: vec![f64::NAN],
                ..camera.clone()
            },
            ColmapCamera {
                params: vec![0.],
                ..camera
            },
        ] {
            assert_eq!(
                reusable_camera(Some(CameraRelation::Same), &[invalid], &frames),
                None
            );
        }
    }

    #[test]
    fn reshoot_normal_parameters_match_generation_for_every_quality_and_memory_tier() {
        for quality in [Quality::Fast, Quality::Balanced, Quality::High] {
            for automatic in [false, true] {
                for memory in [
                    None,
                    Some(4096),
                    Some(8192),
                    Some(12288),
                    Some(16384),
                    Some(24576),
                ] {
                    for points in [10_192, 5_000_001] {
                        let plan =
                            resolve_planner_resolution_plan(quality, memory, 3840, 2160, true);
                        let actual = normal_training_preset(
                            quality,
                            automatic,
                            memory,
                            3840,
                            None,
                            points,
                            Some(plan),
                        )
                        .unwrap();
                        let expected = if automatic {
                            resolve_brush_training_preset_for_plan(
                                quality, memory, 3840, points, &plan,
                            )
                        } else {
                            resolve_brush_training_preset(quality, false, memory, 3840, points)
                        };
                        assert_eq!(actual, expected);
                        if !automatic {
                            assert_eq!(actual.preset.max_splats, None);
                        }
                        let actual = normal_training_preset(
                            quality,
                            automatic,
                            memory,
                            3840,
                            Some(512),
                            points,
                            None,
                        )
                        .unwrap();
                        assert_eq!(
                            actual,
                            resolve_panorama_training_preset(
                                quality, automatic, memory, 512, points
                            )
                            .unwrap()
                        );
                        assert!(actual.preset.max_splats.unwrap() as u64 >= points);
                    }
                }
            }
        }
        assert!(normal_training_preset(
            Quality::High,
            true,
            Some(24576),
            4096,
            Some(1024),
            10_000_001,
            None
        )
        .is_err());
    }

    #[tokio::test]
    async fn reshoot_application_logs_never_advance_progress_and_ignore_old_operations() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let output = events.clone();
        let runner = PipelineRunner::new_with_planner(
            EnginePaths::from_root(PathBuf::from("unused-engines")),
            true,
            move |event| output.lock().unwrap().push(event),
        );
        runner
            .events
            .stage(PipelineStage::ExtractingFrames, 0., "prepare");
        let before = runner
            .events
            .last_progress_milli_percent
            .load(Ordering::Relaxed);
        let token = runner.process_manager.child_token();
        let progress = ReshootProgress::new(
            &runner.events,
            PipelineStage::ExtractingFrames,
            &token,
            "正在整理补拍画面",
            Some(100),
        );
        for count in 1..=100 {
            progress.update(count, 100);
        }
        assert_eq!(
            runner
                .events
                .last_progress_milli_percent
                .load(Ordering::Relaxed),
            before
        );
        let captured = events.lock().unwrap().clone();
        let logs: Vec<_> = captured
            .iter()
            .filter(|e| e.kind == EventKind::Log)
            .collect();
        assert_eq!(logs.len(), 2);
        assert!(logs[0].message.ends_with("0/100"));
        assert!(logs[1].message.ends_with("100/100"));
        assert!(logs
            .iter()
            .all(|e| e.stage_progress.is_none() && e.current.is_none()));
        runner
            .events
            .stage(PipelineStage::ExtractingFeatures, 0., "features");
        let count = events.lock().unwrap().len();
        progress.log("late".into());
        assert_eq!(events.lock().unwrap().len(), count);
        token.cancel();
        progress.update(100, 100);
        assert_eq!(events.lock().unwrap().len(), count);
    }

    #[tokio::test]
    async fn reshoot_cancellable_copy_preserves_source_and_uncommitted_cache() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("cache");
        let data = vec![27; 600_000];
        tokio::fs::write(&source, &data).await.unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        copy_reshoot_file(&source, &target, &cancel).await.unwrap();
        assert_eq!(tokio::fs::read(&target).await.unwrap(), data);
        cancel.cancel();
        assert!(matches!(
            copy_reshoot_file(&source, &root.path().join("partial"), &cancel).await,
            Err(SplatError::Cancelled)
        ));
        assert_eq!(tokio::fs::read(&source).await.unwrap(), data);
        assert_eq!(tokio::fs::read(&target).await.unwrap(), data);
    }

    #[tokio::test]
    async fn reshoot_publication_logs_actual_count_only_after_successful_validation_and_commit() {
        for failure in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let input = root.path().join("input");
            std::fs::create_dir(&input).unwrap();
            for name in ["a.png", "b.png"] {
                image::RgbImage::new(16, 16).save(input.join(name)).unwrap();
            }
            let manager = ProjectManager::for_diagnostics(root.path().join("projects"));
            let token = tokio_util::sync::CancellationToken::new();
            let (paths, mut metadata) = manager
                .create_reshoot_v2(&input, Quality::Fast, token)
                .await
                .unwrap();
            let mut state = manager.read_state(&paths.state).await.unwrap();
            let candidate = paths.work.join("candidate.ply");
            let mut valid = b"ply\nformat binary_little_endian 1.0\nelement vertex 23\n".to_vec();
            for name in [
                "x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1",
                "scale_2", "rot_0", "rot_1", "rot_2", "rot_3",
            ] {
                valid.extend_from_slice(format!("property float {name}\n").as_bytes());
            }
            valid.extend_from_slice(b"end_header\n");
            for _ in 0..23 {
                for value in [
                    0_f32, 0., 1., 0.1, 0.1, 0.1, 0., -3., -3., -3., 1., 0., 0., 0.,
                ] {
                    valid.extend_from_slice(&value.to_le_bytes());
                }
            }
            tokio::fs::write(
                &candidate,
                if failure {
                    b"invalid".as_slice()
                } else {
                    valid.as_slice()
                },
            )
            .await
            .unwrap();
            if !failure {
                assert_eq!(
                    validate_ply(&candidate, &tokio_util::sync::CancellationToken::new())
                        .await
                        .unwrap()
                        .0
                        .splat_count,
                    23
                );
            }
            let final_ply = paths.project.join("final.ply");
            tokio::fs::write(&final_ply, b"previous result")
                .await
                .unwrap();
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let output = events.clone();
            let metrics = PlannerEffectivenessTracker::default();
            let runner = PipelineRunner::new_with_planner(
                EnginePaths::from_root(root.path().join("engines")),
                true,
                move |event| output.lock().unwrap().push(event),
            )
            .with_effectiveness_tracker(metrics.clone());
            let report = ReconstructionReport {
                input_images: 4,
                registered_images: 4,
                registered_ratio: 1.,
                points_3d: 100,
                quality: crate::reconstruction::validator::ReconstructionQuality::Good,
            };
            let checkpoint = ReshootState {
                reshoot_frame_count: 2,
                registered_reshoot_count: 2,
                ..Default::default()
            };
            let result = runner
                .publish_reshoot_v2(
                    &manager,
                    &paths,
                    &mut metadata,
                    &mut state,
                    &checkpoint,
                    &report,
                    &candidate,
                )
                .await;
            let captured = events.lock().unwrap().clone();
            assert_eq!(captured[0].stage, PipelineStage::Exporting);
            let final_metrics = metrics.snapshot().final_result;
            assert_eq!(
                final_metrics.as_ref().map(|v| v.splat_count),
                if failure { None } else { Some(23) }
            );
            if failure {
                assert!(result.is_err());
                assert!(captured.iter().all(
                    |e| !e.message.starts_with("已发布") && e.stage != PipelineStage::Completed
                ));
                assert_eq!(
                    tokio::fs::read(&final_ply).await.unwrap(),
                    b"previous result"
                );
                // A valid candidate still must not report success if the
                // filesystem refuses the final atomic replacement.
                tokio::fs::write(&candidate, &valid).await.unwrap();
                let protected = paths.project.join("protected-existing.ply");
                tokio::fs::rename(&final_ply, &protected).await.unwrap();
                tokio::fs::create_dir(&final_ply).await.unwrap();
                assert!(runner
                    .publish_reshoot_v2(
                        &manager,
                        &paths,
                        &mut metadata,
                        &mut state,
                        &checkpoint,
                        &report,
                        &candidate
                    )
                    .await
                    .is_err());
                assert!(events.lock().unwrap().iter().all(
                    |e| !e.message.starts_with("已发布") && e.stage != PipelineStage::Completed
                ));
                assert_eq!(
                    tokio::fs::read(&protected).await.unwrap(),
                    b"previous result"
                );
            } else {
                assert_eq!(result.unwrap().splat_count, 23);
                assert!(captured.iter().any(|e| e.message == "已发布 23 个 Splat"));
                assert_eq!(captured.last().unwrap().stage, PipelineStage::Completed);
                let saved: ProjectMetadata =
                    serde_json::from_slice(&tokio::fs::read(&paths.metadata).await.unwrap())
                        .unwrap();
                assert_eq!(saved.status, ProjectStatus::Completed);
                assert_eq!(saved.output.unwrap().splat_count, 23);
            }
        }
    }

    #[test]
    fn mixed_reshoot_images_leave_regular_generation_strict_and_preserve_sources() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input");
        std::fs::create_dir(&input).unwrap();
        for (name, w, h) in [("相机2.png", 12, 8), ("相机10.png", 8, 12)] {
            image::RgbaImage::from_pixel(w, h, image::Rgba([120, 40, 80, 128]))
                .save(input.join(name))
                .unwrap();
        }
        assert!(crate::video::image_projection::analyze(&input).is_err());
        let scan = crate::video::image_projection::analyze_reshoot(&input).unwrap();
        assert_eq!(scan.info.image_count, 2);
        assert_eq!(scan.images[0].path.file_name().unwrap(), "相机2.png");
        let frames = root.path().join("frames");
        let colmap = root.path().join("colmap");
        let masks = root.path().join("masks");
        for dir in [&frames, &colmap, &masks] {
            std::fs::create_dir(dir).unwrap();
        }
        let before = std::fs::read(&scan.images[0].path).unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        for (i, item) in scan.images.iter().enumerate() {
            normalize_image(
                &item.path,
                &frames,
                &colmap,
                &masks,
                &format!("reshoot_batch_{i}.png"),
                &token,
            )
            .unwrap();
        }
        ensure_masks(&colmap, &masks, &token, None).unwrap();
        assert_eq!(std::fs::read(&scan.images[0].path).unwrap(), before);
        assert_eq!(
            image::image_dimensions(masks.join("reshoot_batch_1.png.png")).unwrap(),
            (8, 12)
        );
        assert_eq!(
            image::open(frames.join("reshoot_batch_0.png"))
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)[3],
            128
        );
        token.cancel();
        assert!(matches!(
            ensure_masks(&colmap, &masks, &token, None),
            Err(SplatError::Cancelled)
        ));
    }

    #[test]
    fn independent_camera_groups_and_pose_signs_preserve_the_same_coordinate_frame() {
        let root = tempfile::tempdir().unwrap();
        for dir in ["before", "after"] {
            let path = root.path().join(dir);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(
                path.join("cameras.txt"),
                "1 SIMPLE_RADIAL 20 10 15 10 5 0\n3 SIMPLE_RADIAL 12 16 10 6 8 0\n",
            )
            .unwrap();
            std::fs::write(path.join("images.txt"), "1 1 0 0 0 1 2 3 1 old.jpg\n\n").unwrap();
        }
        assert!(
            coordinates_match(&root.path().join("before"), &root.path().join("after")).unwrap()
        );
        std::fs::write(
            root.path().join("after/images.txt"),
            "1 -1 0 0 0 1 2 3 1 old.jpg\n\n",
        )
        .unwrap();
        assert!(
            coordinates_match(&root.path().join("before"), &root.path().join("after")).unwrap()
        );
        std::fs::write(
            root.path().join("after/images.txt"),
            "1 1 0 0 0 5 2 3 1 old.jpg\n\n",
        )
        .unwrap();
        assert!(
            !coordinates_match(&root.path().join("before"), &root.path().join("after")).unwrap()
        );
    }

    #[tokio::test]
    #[ignore = "Requires the local real Ballintoy source project, bundled engines, and GPU; writes only an isolated .cache project"]
    async fn native_ballintoy_reshoot_v2() {
        run_native_ballintoy_reshoot(false).await;
    }

    #[tokio::test]
    #[ignore = "Requires real Ballintoy media, bundled engines and GPU; runs the normal full training recipe in an isolated .cache project"]
    async fn native_ballintoy_reshoot_normal_parameters() {
        run_native_ballintoy_reshoot(true).await;
    }

    async fn run_native_ballintoy_reshoot(normal: bool) {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let original = repo.join(".cache/panorama-v1/projects/20261017-191833_ballintoy-erp");
        let source_metadata: ProjectMetadata =
            serde_json::from_slice(&std::fs::read(original.join("project.json")).unwrap()).unwrap();
        let source_state: PipelineStateFile =
            serde_json::from_slice(&std::fs::read(original.join("state.json")).unwrap()).unwrap();
        let model = best_sparse_model(
            &original.join("work/frames"),
            &original.join("work/colmap/sparse"),
        )
        .await
        .unwrap()
        .0;
        let original_hash = panorama::fingerprint(&original.join("final.ply")).unwrap();
        let source = SourceAssets {
            root: original.clone(),
            metadata: source_metadata.clone(),
            state: source_state,
            images: original.join("work/frames"),
            colmap_images: original.join("work/colmap_frames"),
            masks: original.join("work/masks"),
            database: original.join("work/colmap/database.db"),
            model,
        };
        let identity = source_identity(&source).unwrap();
        let test_root = repo.join(".cache/reshoot-v2-native");
        std::fs::create_dir_all(&test_root).unwrap();
        let run = test_root.join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir(&run).unwrap();
        let input = run.join("new-input");
        std::fs::create_dir(&input).unwrap();
        let originals =
            crate::video::list_images(Path::new("E:/GaussianSplatting/test/Ballintoy/Input"))
                .unwrap();
        for i in [0, 5, 10, 15, 20, 25, 30, 35] {
            std::fs::copy(&originals[i], input.join(originals[i].file_name().unwrap())).unwrap();
        }
        let manager = ProjectManager::for_diagnostics(run.join("projects"));
        let (paths, mut metadata) = manager
            .create_reshoot_v2(
                &input,
                Quality::Fast,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap();
        metadata.reshoot = Some(ReshootProvenance {
            source_project_id: source_metadata.id,
            source_project_path: original.clone(),
            source_final_ply: original.join("final.ply"),
            reshoot_source_path: metadata.source_path.clone(),
            camera_id: 1,
            camera_model: "EQUIRECTANGULAR".into(),
            width: 1920,
            height: 960,
            has_alpha: false,
            mask_count: 0,
            source_image_count: 56,
            reshoot_frame_count: 8,
            registered_reshoot_count: 0,
        });
        let mut state = manager.read_state(&paths.state).await.unwrap();
        state.video_projection = VideoProjection::Equirectangular;
        state.planner_enabled = true;
        let options = ReshootOptions {
            condition: ModelCondition::Good,
            camera_relation: None,
            allow_fallback: false,
            source_identity: identity,
            input_identity: panorama::source_identity(&input).unwrap(),
            projection: VideoProjection::Equirectangular,
            image_matching_strategy: crate::project::ImageMatchingStrategy::Exhaustive,
        };
        let plan = ReshootPlan::resolve(ModelCondition::Good, true, true);
        state.reshoot = Some(ReshootState {
            recipe: Some(ReshootRecipe {
                version: RECIPE_VERSION,
                normal_parameters_version: normal.then_some(1),
                camera_assignment: None,
                options,
                initial: plan,
                actual: plan,
                batch_id: paths.id.simple().to_string(),
                reconstruction_fallbacks: 0,
                training_fallbacks: 0,
                expanded_matching: false,
                source_names: vec![],
                new_names: vec![],
                warm_parameters: None,
                init_sha256: None,
                init_georeference: None,
                stored_input_identity: Some(
                    panorama::source_identity(&metadata.source_path).unwrap(),
                ),
                panorama_frames: vec![],
            }),
            ..Default::default()
        });
        manager.write_state(&paths.state, &state).await.unwrap();
        manager
            .write_metadata(&paths.metadata, &metadata)
            .await
            .unwrap();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = events.clone();
        let runner = PipelineRunner::new_with_planner(
            EnginePaths::from_root(repo.join("engines")),
            true,
            move |event| {
                log.lock().unwrap().push(event);
            },
        );
        let acceleration = runner.verify_pipeline_engines().await.unwrap();
        eprintln!("Real reshoot project: {}", paths.project.display());
        let result = runner
            .execute_reshoot_v2(
                &manager,
                &paths,
                &mut metadata,
                state,
                &acceleration,
                source,
            )
            .await;
        std::fs::write(
            run.join("events.json"),
            serde_json::to_vec_pretty(&*events.lock().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            panorama::fingerprint(&original.join("final.ply")).unwrap(),
            original_hash
        );
        let result = result.unwrap();
        std::fs::write(
            run.join("result.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        let restored = manager.read_state(&paths.state).await.unwrap();
        if normal {
            assert_eq!(
                restored.brush_training.resolved.unwrap().preset.total_steps,
                resolve_panorama_training_preset(Quality::Fast, true, None, 512, 1)
                    .unwrap()
                    .preset
                    .total_steps
            );
            assert!(restored
                .reshoot
                .as_ref()
                .unwrap()
                .recipe
                .as_ref()
                .unwrap()
                .warm_parameters
                .is_none());
            let captured = events.lock().unwrap();
            assert!(captured
                .iter()
                .any(|event| event.message == format!("已发布 {} 个 Splat", result.splat_count)));
            assert_eq!(captured.last().unwrap().stage, PipelineStage::Completed);
        }
        assert!(restored.reshoot.as_ref().unwrap().registered_reshoot_count > 0);
        assert_eq!(
            restored.reshoot.unwrap().recipe.unwrap().actual.training,
            TrainingMode::Warm
        );
        assert!(result.splat_count > 0);
    }
}
