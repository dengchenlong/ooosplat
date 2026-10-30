//! MCP admission only: reuse GUI analysis/planning and the shared Runner.
use super::*;
use crate::{
    commands::InputAnalysis,
    pipeline::{
        generation_policy::GenerationPolicy,
        reshoot::{CameraRelation, ModelCondition, ReshootOptions},
        runner::ReshootSourceInfo,
    },
    project::ImageMatchingStrategy,
    video::panorama::{source_identity, VideoProjection},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::OnceLock,
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Answers {
    pub input_projection: Option<VideoProjection>,
    pub small_images: Option<bool>,
    pub image_order: Option<ImageMatchingStrategy>,
    pub model_condition: Option<ModelCondition>,
    pub camera_relation: Option<CameraRelation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InspectRequest {
    pub input_path: String,
    pub quality: Quality,
    pub generation_policy: Option<GenerationPolicy>,
    pub source_project_id: Option<Uuid>,
}
#[derive(Clone)]
pub(super) struct Inspection {
    pub id: Uuid,
    pub path: PathBuf,
    pub identity: String,
    pub quality: Quality,
    pub policy: GenerationPolicy,
    pub source: Option<ReshootSourceInfo>,
    pub analysis: InputAnalysis,
}
fn inspections() -> &'static Mutex<VecDeque<Inspection>> {
    static CACHE: OnceLock<Mutex<VecDeque<Inspection>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
/// Reuse approved UI text, embedded at build time; never read arbitrary files.
pub(super) fn text(key: &str, count: u64) -> Value {
    let source = include_str!("../../../src/i18n/index.tsx");
    let (zh, en) = source
        .split_once("const en:")
        .expect("UI locale dictionaries");
    let read = |source: &str| {
        let marker = format!("\"{key}\": ");
        let tail = source
            .split_once(&marker)
            .map(|(_, value)| value)
            .unwrap_or("\"\"");
        serde_json::Deserializer::from_str(tail)
            .into_iter::<String>()
            .next()
            .and_then(std::result::Result::ok)
            .unwrap_or_default()
            .replace("{count}", &count.to_string())
    };
    json!({"zh-CN":read(zh),"en":read(en)})
}
fn question(
    key: &str,
    title: &str,
    description: Option<&str>,
    options: &[(&str, &str)],
    count: u64,
) -> Value {
    json!({"key":key,"title":text(title,count),"description":description.map(|key|text(key,count)),
        "options":options.iter().map(|(value,label)|json!({"value":if key=="small_images" {json!(true)}else{json!(value)},"label":text(label,count)})).collect::<Vec<_>>()})
}
impl Inspection {
    pub fn questions(&self) -> Vec<Value> {
        let mut questions = Vec::new();
        let images = self.analysis.image_sequence.as_ref();
        let count = images.map_or(0, |i| i.image_count);
        if self.analysis.projection_detection.requires_choice {
            let image = images.is_some();
            let mut options = vec![(
                "perspective",
                if image {
                    "source.images"
                } else {
                    "panorama.ordinaryDetected"
                },
            )];
            if self.analysis.projection_detection.supported_erp {
                options.push((
                    "equirectangular",
                    if image {
                        "source.panoramaImages"
                    } else {
                        "source.fullPanorama"
                    },
                ));
            }
            questions.push(question(
                "input_projection",
                if image {
                    "source.chooseImageType"
                } else {
                    "source.chooseType"
                },
                Some(if image {
                    "source.ambiguousImages"
                } else {
                    "panorama.ambiguous"
                }),
                &options,
                count,
            ));
        }
        if let Some(source) = &self.source {
            questions.push(question(
                "model_condition",
                "reshoot.conditionTitle",
                None,
                &[
                    ("good", "reshoot.condition.good"),
                    ("appearance", "reshoot.condition.appearance"),
                    ("geometry", "reshoot.condition.geometry"),
                    ("unsure", "reshoot.condition.unsure"),
                ],
                count,
            ));
            // Keep the plain-language processing hints with each option.
            if let Some(options) = questions
                .last_mut()
                .and_then(|q| q["options"].as_array_mut())
            {
                for option in options {
                    option["description"] = text(
                        &format!(
                            "reshoot.condition.{}Hint",
                            option["value"].as_str().unwrap()
                        ),
                        count,
                    );
                }
            }
            if source.projection != VideoProjection::Equirectangular {
                questions.push(question(
                    "camera_relation",
                    "reshoot.cameraTitle",
                    Some("reshoot.cameraDescription"),
                    &[
                        ("same", "reshoot.camera.same"),
                        ("different", "reshoot.camera.different"),
                        ("unsure", "reshoot.camera.unsure"),
                    ],
                    count,
                ));
            }
        } else if images.is_some() && count < 30 {
            questions.push(question(
                "small_images",
                "dialog.smallSequenceTitle",
                Some("dialog.smallSequence"),
                &[("accepted", "preflight.acceptWarning")],
                count,
            ));
        }
        if count > 100 {
            questions.push(question(
                "image_order",
                "preflight.imageOrderTitle",
                Some("preflight.imageOrderDescription"),
                &[
                    ("sequential", "preflight.ordered"),
                    ("exhaustive", "preflight.unordered"),
                ],
                count,
            ));
        }
        questions
    }
    pub fn validate_answers(
        &self,
        answers: &Answers,
    ) -> Result<(
        VideoProjection,
        ImageMatchingStrategy,
        Option<ReshootOptions>,
    )> {
        if answers.input_projection == Some(VideoProjection::Auto) {
            return Err(ServiceError::new(
                "INVALID_ARGUMENT",
                "Projection answers must be explicit",
            ));
        }
        let missing: Vec<_> = self
            .questions()
            .into_iter()
            .filter(|q| match q["key"].as_str().unwrap() {
                "input_projection" => !matches!(
                    answers.input_projection,
                    Some(VideoProjection::Perspective | VideoProjection::Equirectangular)
                ),
                "small_images" => answers.small_images != Some(true),
                "image_order" => answers.image_order.is_none(),
                "model_condition" => answers.model_condition.is_none(),
                "camera_relation" => answers.camera_relation.is_none(),
                _ => true,
            })
            .collect();
        if !missing.is_empty() {
            let mut error = ServiceError::new(
                "CONFIRMATION_REQUIRED",
                "Collect the missing answers from the user before creating a task.",
            );
            error.details = Some(
                json!({"preflight_id":self.id,"input_source_identity":self.identity,"questions":missing}),
            );
            return Err(error);
        }
        let detection = &self.analysis.projection_detection;
        let projection = if detection.confirmed_erp {
            VideoProjection::Equirectangular
        } else {
            answers
                .input_projection
                .unwrap_or(VideoProjection::Perspective)
        };
        if (projection == VideoProjection::Equirectangular && !detection.supported_erp)
            || (detection.confirmed_erp
                && answers.input_projection == Some(VideoProjection::Perspective))
            || self
                .source
                .as_ref()
                .is_some_and(|s| s.projection != projection)
        {
            return Err(ServiceError::new(
                "INVALID_PROJECTION",
                "The selected projection is incompatible with this input or source project.",
            ));
        }
        let strategy = if self
            .analysis
            .image_sequence
            .as_ref()
            .is_some_and(|i| i.image_count > 100)
        {
            answers.image_order.unwrap_or_default()
        } else {
            ImageMatchingStrategy::Exhaustive
        };
        let options = self.source.as_ref().map(|source| ReshootOptions {
            condition: answers.model_condition.expect("validated answer"),
            camera_relation: answers.camera_relation,
            allow_fallback: self.policy.auto_retry_enabled,
            source_identity: source.source_identity.clone(),
            input_identity: self.identity.clone(),
            projection,
            image_matching_strategy: strategy,
        });
        Ok((projection, strategy, options))
    }
}
pub(super) async fn inspect(tools: &McpTools, request: InspectRequest) -> Result<Inspection> {
    let path = authorize_input(Path::new(&request.input_path), &tools.settings.input_roots)?;
    if path.is_dir() {
        // A directory authorization does not authorize symlink targets outside it.
        for image in crate::video::list_images(&path)? {
            if !std::fs::canonicalize(image)?.starts_with(&path) {
                return Err(ServiceError::new(
                    "PATH_NOT_AUTHORIZED",
                    "Redirected image escapes the selected input directory",
                ));
            }
        }
    }
    let identity = source_identity(&path)?;
    let policy = request.generation_policy.unwrap_or_default().validate()?;
    let policy = GenerationPolicy {
        version: crate::pipeline::generation_policy::VERSION,
        ..policy
    };
    let source = if let Some(id) = request.source_project_id {
        Some(
            crate::pipeline::runner::PipelineRunner::new(tools.engines.clone(), |_| {})
                .inspect_reshoot_source_v2(id)
                .await?,
        )
    } else {
        None
    };
    let mut cache = inspections().lock().await;
    if let Some(old) = cache.iter().find(|s| {
        s.identity == identity
            && s.quality == request.quality
            && s.policy == policy
            && s.source
                .as_ref()
                .map(|s| (&s.project_id, &s.source_identity))
                == source.as_ref().map(|s| (&s.project_id, &s.source_identity))
    }) {
        return Ok(old.clone());
    }
    let analysis = if let Some(cached) = cache.iter().find(|s| {
        s.identity == identity
            && s.source.as_ref().map(|s| s.projection) == source.as_ref().map(|s| s.projection)
    }) {
        cached.analysis.clone()
    } else if path.is_dir()
        && source
            .as_ref()
            .is_some_and(|s| s.projection == VideoProjection::Perspective)
    {
        let directory = path.clone();
        let scan = tokio::task::spawn_blocking(move || {
            crate::video::image_projection::analyze_reshoot(&directory)
        })
        .await
        .map_err(|e| ServiceError::new("ANALYSIS_FAILED", e.to_string()))??;
        InputAnalysis {
            input_type: crate::project::ProjectInputType::Images,
            video: None,
            image_sequence: Some(scan.info),
            projection_detection: scan.detection,
        }
    } else {
        crate::commands::analyze_media(&tools.engines, &path).await?
    };
    if source_identity(&path)? != identity {
        return Err(ServiceError::new(
            "INPUT_CHANGED",
            "Input changed; inspect it again.",
        ));
    }
    let result = Inspection {
        id: Uuid::new_v4(),
        path,
        identity,
        quality: request.quality,
        policy,
        source,
        analysis,
    };
    if cache.len() >= 64 {
        cache.pop_front();
    }
    cache.push_back(result.clone());
    Ok(result)
}
pub(super) async fn verify_id(id: Option<Uuid>, current: &Inspection) -> Result<()> {
    if id.is_some_and(|id| id != current.id) {
        return Err(ServiceError::new(
            "INPUT_CHANGED",
            "Preflight is stale or expired; inspect and confirm again.",
        ));
    }
    Ok(())
}
#[derive(Clone, Copy)]
enum GpsSummary {
    Pending,
    Checked(bool),
    Unavailable,
}
fn gps_cache() -> &'static std::sync::Mutex<BTreeMap<String, GpsSummary>> {
    static CACHE: OnceLock<std::sync::Mutex<BTreeMap<String, GpsSummary>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
fn gps_summary(inspection: &Inspection) -> Value {
    if !inspection.path.is_dir() {
        return json!({"detected":null,"status":"unavailable"});
    }
    let mut cache = gps_cache().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(value) = cache.get(&inspection.identity) {
        return match value {
            GpsSummary::Pending => json!({"detected":null,"status":"pending"}),
            GpsSummary::Checked(detected) => json!({"detected":detected,"status":"checked"}),
            GpsSummary::Unavailable => json!({"detected":null,"status":"unavailable"}),
        };
    }
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.insert(inspection.identity.clone(), GpsSummary::Pending);
    let path = inspection.path.clone();
    let identity = inspection.identity.clone();
    tokio::spawn(async move {
        let expected = identity.clone();
        let observed = tokio::task::spawn_blocking(move || {
            let images = crate::video::list_images(&path).ok()?;
            let detected = crate::video::gps::detect(&images);
            (source_identity(&path).ok().as_deref() == Some(&expected)).then_some(detected)
        })
        .await;
        if let Some(value) = gps_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&identity)
        {
            *value = match observed {
                Ok(Some(detected)) => GpsSummary::Checked(detected),
                _ => GpsSummary::Unavailable,
            };
        }
    });
    json!({"detected":null,"status":"pending"})
}

#[cfg(test)]
#[path = "generation_tests.rs"]
mod tests;
pub(super) async fn public_inspection(tools: &McpTools, inspection: Inspection) -> Result<Value> {
    let plan = crate::commands::plan_input(
        &tools.engines,
        inspection.path.clone(),
        inspection.quality,
        None,
        Some(inspection.analysis.clone()),
        Some(inspection.policy),
        None,
    )
    .await?;
    let mut plan = snake(serde_json::to_value(plan)?);
    if let Some(frame_plan) = plan.get_mut("plan").and_then(Value::as_object_mut) {
        for key in ["selected_frames", "candidate_frames"] {
            if let Some(Value::Array(frames)) = frame_plan.remove(key) {
                frame_plan.insert(
                    format!("{}_count", key.trim_end_matches('s')),
                    json!(frames.len()),
                );
            }
        }
    }
    if let Some(detection) = plan
        .get_mut("projection_detection")
        .and_then(Value::as_object_mut)
    {
        detection.remove("preview_jpeg");
    }
    let mut input = snake(serde_json::to_value(&inspection.analysis)?);
    if let Some(detection) = input
        .get_mut("projection_detection")
        .and_then(Value::as_object_mut)
    {
        detection.remove("preview_jpeg");
    }
    Ok(
        json!({"preflight_id":inspection.id,"input_source_identity":inspection.identity,"input":input,
        "source":inspection.source.as_ref().map(|s|snake(serde_json::to_value(s).unwrap())),"generation_policy":snake(serde_json::to_value(inspection.policy)?),
        "plan":plan,"plan_is_estimate":true,"max_splats":null,"questions":inspection.questions(),"gps":gps_summary(&inspection)}),
    )
}

pub(super) async fn create(tools: &McpTools, args: Value) -> Result<Value> {
    let legacy_order = args
        .pointer("/reshoot_options/imageMatchingStrategy")
        .is_some();
    let request: CreateRequest = parse(args)?;
    if request.reshoot_options.is_some() && request.source_project_id.is_none() {
        return Err(ServiceError::new(
            "INVALID_ARGUMENT",
            "reshoot_options requires source_project_id",
        ));
    }
    if request.client_request_id.is_empty() || request.client_request_id.len() > 256 {
        return Err(ServiceError::new(
            "INVALID_ARGUMENT",
            "client_request_id must contain 1–256 bytes",
        ));
    }
    let policy = request
        .generation_policy
        .unwrap_or_else(|| GenerationPolicy {
            auto_retry_enabled: request
                .reshoot_options
                .as_ref()
                .is_none_or(|o| o.allow_fallback),
            ..Default::default()
        })
        .validate()?;
    let policy = GenerationPolicy {
        version: crate::pipeline::generation_policy::VERSION,
        ..policy
    };
    let mut intent = serde_json::to_value(&request)?;
    intent.as_object_mut().unwrap().remove("preflight_id");
    intent.as_object_mut().unwrap().remove("client_request_id");
    intent["generation_policy"] = serde_json::to_value(policy)?;
    // First honor a lost response, even if input/settings have since changed.
    if let Some(old) = tools
        .tasks
        .all()
        .await?
        .into_iter()
        .find(|t| t.client_request_id.as_ref() == Some(&request.client_request_id))
    {
        if old
            .request_parameters
            .as_ref()
            .and_then(|p| p.get("mcp_intent"))
            == Some(&intent)
        {
            return public_task(old);
        }
        // Compatibility with already-created v1 MCP tasks.
        let mut legacy = json!({"input_path":request.input_path,"quality":request.quality});
        if let Some(policy) = request.generation_policy {
            legacy["generationPolicy"] = serde_json::to_value(policy)?;
        }
        if let Some(options) = &request.reshoot_options {
            legacy["reshootOptions"] = serde_json::to_value(options)?;
            legacy["source_project_id"] = json!(request.source_project_id);
        }
        if old.request_parameters == Some(crate::tasks::generation_parameters(Some(legacy))?)
            && request.preflight_id.is_none()
            && request.input_source_identity.is_none()
            && serde_json::to_value(&request.answers)? == serde_json::to_value(Answers::default())?
        {
            return public_task(old);
        }
        return Err(ServiceError::new(
            "IDEMPOTENCY_CONFLICT",
            "client_request_id already identifies different parameters",
        ));
    }
    let inspection = inspect(
        tools,
        InspectRequest {
            input_path: request.input_path.clone(),
            quality: request.quality,
            generation_policy: Some(policy),
            source_project_id: request.source_project_id,
        },
    )
    .await?;
    verify_id(request.preflight_id, &inspection).await?;
    if request
        .input_source_identity
        .as_ref()
        .is_some_and(|id| id != &inspection.identity)
    {
        return Err(ServiceError::new(
            "INPUT_CHANGED",
            "Input changed; inspect and confirm again.",
        ));
    }
    let mut answers = request.answers;
    if let Some(options) = &request.reshoot_options {
        if options.source_identity != inspection.source.as_ref().unwrap().source_identity
            || options.input_identity != inspection.identity
        {
            return Err(ServiceError::new(
                "INPUT_CHANGED",
                "Reshoot source or input changed; inspect again.",
            ));
        }
        answers.model_condition = answers.model_condition.or(Some(options.condition));
        answers.camera_relation = answers.camera_relation.or(options.camera_relation);
        answers.input_projection = answers.input_projection.or(Some(options.projection));
        if legacy_order {
            answers.image_order = answers
                .image_order
                .or(Some(options.image_matching_strategy));
        }
    }
    let (projection, strategy, options) = inspection.validate_answers(&answers)?;
    if let (Some(id), Some(options)) = (request.source_project_id, &options) {
        crate::pipeline::runner::PipelineRunner::new(tools.engines.clone(), |_| {})
            .with_generation_policy(Some(policy))
            .validate_reshoot_request_v2(
                id,
                &inspection.path,
                if inspection.path.is_dir() {
                    crate::pipeline::runner::ReshootInputType::Images
                } else {
                    crate::pipeline::runner::ReshootInputType::Video
                },
                request.quality,
                options,
            )
            .await?;
    }
    if source_identity(&inspection.path)? != inspection.identity {
        return Err(ServiceError::new(
            "INPUT_CHANGED",
            "Input changed during admission",
        ));
    }
    let mut parameters = json!({"input_path":request.input_path,"quality":request.quality,"generationPolicy":policy,
        "videoProjection":projection,"videoSourceIdentity":inspection.identity,"imageMatchingStrategy":strategy,"mcp_intent":intent});
    if let Some(options) = options {
        parameters["reshootOptions"] = serde_json::to_value(options)?;
        parameters["source_project_id"] = json!(request.source_project_id);
    }
    let settings = crate::project::catalog::load_settings().await?;
    let task = tools
        .tasks
        .create(
            None,
            inspection.path,
            request.quality,
            settings.projects_root,
            true,
            "mcp",
            Some(request.client_request_id),
            Some(parameters),
        )
        .await?;
    if let Some(id) = request.source_project_id {
        tools.tasks.mark_reshoot(task.task_id, id).await?;
    }
    public_task(tools.tasks.get(task.task_id).await?)
}
