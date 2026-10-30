//! Application-owned task registry. Transports never own a generation future.
pub mod logs;
mod notifications;
#[cfg(test)]
mod tests;

pub use notifications::TaskView;

use crate::{
    engines::EnginePaths,
    pipeline::{
        effectiveness::PlannerEffectivenessTracker,
        runner::{PipelineResult, PipelineRunner, ReshootInputType, RunnerUpdate},
        PipelineEvent, PipelineStage,
    },
    presets::{PipelineOptimizationConfig, Quality},
    project::{catalog, manager::atomic_write_json, ProjectStatus},
    telemetry::{PipelineTelemetrySession, TelemetryInputType, TelemetryRunKind, TelemetryService},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as SyncMutex},
};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Created,
    Starting,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}
impl TaskStatus {
    pub fn project_status(self) -> ProjectStatus {
        match self {
            Self::Completed => ProjectStatus::Completed,
            Self::Failed => ProjectStatus::Failed,
            Self::Cancelled => ProjectStatus::Cancelled,
            Self::Interrupted | Self::Created => ProjectStatus::Interrupted,
            _ => ProjectStatus::Running,
        }
    }
    pub fn active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Cancelling)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskError {
    pub code: String,
    pub message: String,
    pub failed_stage: Option<PipelineStage>,
    pub engine: Option<crate::pipeline::PipelineEngine>,
    pub exit_code: Option<i32>,
    pub classification: Option<String>,
    pub classification_is_heuristic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination: Option<crate::process::ProcessTermination>,
    pub failure_id: Option<Uuid>,
    pub log_sources: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub run_id: Uuid,
    pub kind: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub logs: BTreeMap<String, logs::RegisteredLog>,
    #[serde(default)]
    pub error: Option<TaskError>,
    #[serde(default)]
    pub last_engine: Option<crate::pipeline::PipelineEngine>,
    #[serde(default)]
    pub last_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub brush_executions: Vec<crate::pipeline::brush_execution::BrushExecutionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_brush_execution: Option<usize>,
}

impl TaskRun {
    fn observe_brush(
        &mut self,
        event: &PipelineEvent,
        configuration: Option<crate::presets::ResolvedBrushTrainingPreset>,
    ) {
        if event.kind == crate::pipeline::EventKind::Stage
            && !matches!(
                event.stage,
                PipelineStage::TrainingSplats | PipelineStage::Exporting
            )
        {
            self.current_brush_execution = None;
        }
        if event.engine != Some(crate::pipeline::PipelineEngine::Brush) {
            return;
        }
        let Some(runtime) = event.runtime.as_ref().filter(|v| v.process_id != 0) else {
            return;
        };
        if let Some(last) = self.brush_executions.last_mut() {
            if last.process_id == runtime.process_id
                && last.operation.as_ref().map(|v| v.id) == event.operation.as_ref().map(|v| v.id)
            {
                last.update(runtime);
                return;
            }
        }
        // Only the initial snapshot can create an execution. Delayed samples may not.
        if runtime.phase != "starting" {
            return;
        }
        self.brush_executions.push(
            crate::pipeline::brush_execution::BrushExecutionSnapshot::new(
                runtime,
                event.operation.clone(),
            ),
        );
        self.brush_executions
            .last_mut()
            .expect("new execution")
            .configuration = configuration;
        self.current_brush_execution = Some(self.brush_executions.len() - 1);
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub schema_version: u32,
    pub task_id: Uuid,
    pub run_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub project_path: Option<PathBuf>,
    pub input_path: PathBuf,
    #[serde(default)]
    pub input_type: crate::project::ProjectInputType,
    #[serde(default)]
    pub project_deleted: bool,
    pub quality: Quality,
    pub source: Option<String>,
    #[serde(default)]
    pub configuration_inferred: bool,
    pub task_kind: String,
    pub source_project_id: Option<Uuid>,
    pub planner_enabled: bool,
    #[serde(default)]
    pub generation_policy: Option<crate::pipeline::generation_policy::GenerationPolicy>,
    #[serde(default)]
    pub automatically_retried: bool,
    pub projects_root: PathBuf,
    pub configuration: PipelineOptimizationConfig,
    pub actual_configuration: Option<Value>,
    pub status: TaskStatus,
    pub stage: Option<PipelineStage>,
    #[serde(default)]
    pub substage: Option<String>,
    #[serde(default)]
    pub operation: Option<crate::pipeline::event::PipelineOperation>,
    pub revision: u64,
    pub sequence: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub elapsed_offset_ms: u64,
    #[serde(default)]
    pub runtime: Option<crate::pipeline::runtime::RuntimeSnapshot>,
    pub progress: Option<f32>,
    pub estimated_progress: Option<f32>,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub unit: Option<String>,
    pub eta_seconds: Option<f64>,
    pub error: Option<TaskError>,
    pub result: Option<Value>,
    pub runs: Vec<TaskRun>,
    pub client_request_id: Option<String>,
    pub request_parameters: Option<Value>,
    #[serde(default)]
    pub recent_events: Vec<PipelineEvent>,
}
#[derive(Debug, Clone, Serialize)]
pub struct TaskUpdate {
    pub task: TaskView,
    pub events: Vec<PipelineEvent>,
    pub dropped_event_count: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct StartReceipt {
    pub accepted: bool,
    pub task_id: Uuid,
    pub run_id: Option<Uuid>,
    pub status: TaskStatus,
    pub revision: u64,
}
impl StartReceipt {
    fn existing(t: &TaskRecord) -> Self {
        Self {
            accepted: false,
            task_id: t.task_id,
            run_id: t.run_id,
            status: t.status,
            revision: t.revision,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ServiceError {
    pub code: String,
    pub message: String,
    pub task_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}
impl ServiceError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            task_id: None,
            run_id: None,
            details: None,
        }
    }
}
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ServiceError {}
impl From<crate::error::SplatError> for ServiceError {
    fn from(e: crate::error::SplatError) -> Self {
        Self::new("TASK_ERROR", e.to_string())
    }
}
impl From<std::io::Error> for ServiceError {
    fn from(e: std::io::Error) -> Self {
        Self::new("IO_ERROR", e.to_string())
    }
}
impl From<serde_json::Error> for ServiceError {
    fn from(e: serde_json::Error) -> Self {
        Self::new("INVALID_DATA", e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, ServiceError>;

pub(crate) fn generation_parameters(parameters: Option<Value>) -> Result<Value> {
    let mut parameters = parameters.unwrap_or_else(|| serde_json::json!({}));
    if !parameters.is_object() {
        return Err(ServiceError::new(
            "INVALID_ARGUMENT",
            "Invalid generation parameters",
        ));
    }
    let policy: crate::pipeline::generation_policy::GenerationPolicy = parameters
        .get("generationPolicy")
        .filter(|v| !v.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_else(|| crate::pipeline::generation_policy::GenerationPolicy {
            auto_retry_enabled: parameters
                .pointer("/reshootOptions/allowFallback")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            ..Default::default()
        });
    policy.validate()?;
    // This function admits new tasks/manual retries, never interrupted resumes.
    let policy = crate::pipeline::generation_policy::GenerationPolicy {
        version: crate::pipeline::generation_policy::VERSION,
        ..policy
    };
    parameters["generationPolicy"] = serde_json::to_value(policy)?;
    if parameters.get("reshootOptions").is_some_and(Value::is_null) {
        parameters.as_object_mut().unwrap().remove("reshootOptions");
    }
    if let Some(options) = parameters
        .get_mut("reshootOptions")
        .and_then(Value::as_object_mut)
    {
        options.insert(
            "allowFallback".into(),
            Value::Bool(policy.auto_retry_enabled),
        );
    }
    Ok(parameters)
}

#[derive(Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    tasks: BTreeMap<Uuid, TaskRecord>,
    #[serde(skip)]
    loaded: bool,
}
#[cfg(test)]
pub(crate) type TestExecutor = Arc<
    dyn Fn(
            TaskRecord,
            Arc<PipelineRunner>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = crate::error::Result<PipelineResult>> + Send>,
        > + Send
        + Sync,
>;
type Emit = Arc<dyn Fn(TaskUpdate) + Send + Sync>;
#[derive(Clone)]
pub struct TaskService {
    registry: Arc<SyncMutex<Registry>>,
    gate: Arc<Mutex<()>>,
    root: PathBuf,
    pub(crate) active: Arc<Mutex<Option<Arc<PipelineRunner>>>>,
    pub(crate) diagnostics: Arc<crate::diagnostics::DiagnosticService>,
    emitter: Arc<SyncMutex<Option<Emit>>>,
    notifications: Arc<SyncMutex<notifications::PendingNotifications>>,
    #[cfg(test)]
    pub(crate) executor: Arc<SyncMutex<Option<TestExecutor>>>,
}
impl Default for TaskService {
    fn default() -> Self {
        Self::at(catalog::app_data_root().unwrap_or_else(|_| PathBuf::from(".")))
    }
}
impl TaskService {
    pub fn at(root: PathBuf) -> Self {
        Self {
            registry: Arc::new(SyncMutex::new(Registry::default())),
            gate: Arc::new(Mutex::new(())),
            root,
            active: Arc::new(Mutex::new(None)),
            diagnostics: Arc::new(Default::default()),
            emitter: Arc::new(SyncMutex::new(None)),
            notifications: Arc::new(SyncMutex::new(Default::default())),
            #[cfg(test)]
            executor: Arc::new(SyncMutex::new(None)),
        }
    }
    pub fn attach(&self, app: tauri::AppHandle) {
        use tauri::Emitter;
        *self.emitter.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(move |update| {
            let _ = app.emit("task-update", &update);
        }));
    }
    fn records(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn emitter(&self) -> Option<Emit> {
        self.emitter
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Lifecycle callers hold `gate` through persistence and notification.
    /// The notification mutex serializes emission with synchronous runner events.
    fn notify(&self, id: Uuid) {
        let Some(emit) = self.emitter() else {
            return;
        };
        let mut pending = self.notifications.lock().unwrap_or_else(|p| p.into_inner());
        self.emit_pending(&mut pending, id, &emit, true);
    }

    fn emit_pending(
        &self,
        pending: &mut notifications::PendingNotifications,
        id: Uuid,
        emit: &Emit,
        immediate: bool,
    ) {
        let queued = pending.tasks.remove(&id);
        if queued.is_none() && !immediate {
            return;
        }
        let task = {
            let registry = self.records();
            let Some(task) = registry.tasks.get(&id) else {
                return;
            };
            // Only lifecycle code may announce a non-active status, after persistence.
            if !immediate && !task.status.active() {
                return;
            }
            TaskView::from(task)
        };
        let queued = queued.filter(|queued| queued.run_id == task.run_id);
        let dropped_event_count = queued
            .as_ref()
            .map_or(0, |queued| queued.dropped_event_count);
        let events = queued
            .map(|queued| queued.into_events())
            .unwrap_or_default()
            .into_iter()
            .filter(|event| {
                event.task_id == Some(id)
                    && event.run_id == task.run_id
                    && event.revision <= task.revision
            })
            .collect();
        emit(TaskUpdate {
            task,
            events,
            dropped_event_count,
        });
    }

    fn flush_notifications(&self) {
        // Do not wait for a start/cancel/finish checkpoint, or announce its state early.
        let Ok(_gate) = self.gate.try_lock() else {
            return;
        };
        let Some(emit) = self.emitter() else {
            return;
        };
        let mut pending = self.notifications.lock().unwrap_or_else(|p| p.into_inner());
        let ids: Vec<_> = pending.tasks.keys().copied().collect();
        for id in ids {
            self.emit_pending(&mut pending, id, &emit, false);
        }
    }

    pub fn spawn_notification_pump(&self) -> tauri::async_runtime::JoinHandle<()> {
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            // Construct the timer inside Tauri's runtime, never in the setup hook.
            let mut timer = tokio::time::interval_at(
                tokio::time::Instant::now() + notifications::NOTIFICATION_INTERVAL,
                notifications::NOTIFICATION_INTERVAL,
            );
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                service.flush_notifications();
            }
        })
    }
    async fn persist(&self) -> Result<()> {
        let snapshot = serde_json::to_value(&*self.records())?;
        tokio::fs::create_dir_all(&self.root).await?;
        atomic_write_json(&self.root.join("tasks.json"), &snapshot).await?;
        Ok(())
    }
    async fn load_locked(&self) -> Result<()> {
        if self.records().loaded {
            return Ok(());
        }
        let path = self.root.join("tasks.json");
        let mut registry = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice::<Registry>(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Registry::default(),
            Err(e) => return Err(e.into()),
        };
        for task in registry.tasks.values_mut() {
            if task.status.active() {
                task.status = TaskStatus::Interrupted;
                task.revision += 1;
                task.updated_at = Utc::now();
                task.eta_seconds = None;
                if let Some(run) = task.runs.last_mut() {
                    run.ended_at = None;
                }
                if let Some(project) = &task.project_path {
                    if let Ok(bytes) = tokio::fs::read(project.join("project.json")).await {
                        if let Ok(mut metadata) =
                            serde_json::from_slice::<crate::project::ProjectMetadata>(&bytes)
                        {
                            if catalog::project_is_durably_completed(project, &metadata).await {
                                task.status = TaskStatus::Completed;
                                if let Some(output) = &metadata.output {
                                    task.result = Some(serde_json::to_value(output)?);
                                }
                            } else {
                                metadata.status = ProjectStatus::Interrupted;
                                metadata.duration_ms =
                                    Some(task.elapsed_ms.max(metadata.duration_ms.unwrap_or(0)));
                                atomic_write_json(&project.join("project.json"), &metadata).await?;
                            }
                        }
                    }
                }
            }
        }
        registry.loaded = true;
        *self.records() = registry;
        self.persist().await
    }
    pub async fn initialize(&self) -> Result<()> {
        let _gate = self.gate.lock().await;
        self.load_locked().await
    }
    pub async fn get(&self, id: Uuid) -> Result<TaskRecord> {
        self.initialize().await?;
        self.records()
            .tasks
            .get(&id)
            .cloned()
            .ok_or_else(|| ServiceError::new("TASK_NOT_FOUND", "任务不存在"))
    }
    pub async fn all(&self) -> Result<Vec<TaskRecord>> {
        self.initialize().await?;
        let mut tasks: Vec<_> = self.records().tasks.values().cloned().collect();
        tasks.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.task_id.cmp(&a.task_id))
        });
        Ok(tasks)
    }
    pub async fn running(&self) -> Result<Option<TaskRecord>> {
        Ok(self.all().await?.into_iter().find(|t| t.status.active()))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        id: Option<Uuid>,
        input: PathBuf,
        quality: Quality,
        projects_root: PathBuf,
        planner_enabled: bool,
        source: &str,
        key: Option<String>,
        parameters: Option<Value>,
    ) -> Result<TaskRecord> {
        let service = self.clone();
        let source = source.to_owned();
        tokio::spawn(async move {
            service
                .create_inner(
                    id,
                    input,
                    quality,
                    projects_root,
                    planner_enabled,
                    &source,
                    key,
                    parameters,
                )
                .await
        })
        .await
        .map_err(|error| ServiceError::new("CREATE_FAILED", error.to_string()))?
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_inner(
        &self,
        id: Option<Uuid>,
        input: PathBuf,
        quality: Quality,
        projects_root: PathBuf,
        planner_enabled: bool,
        source: &str,
        key: Option<String>,
        parameters: Option<Value>,
    ) -> Result<TaskRecord> {
        let parameters = Some(generation_parameters(parameters)?);
        let _legacy_planner_enabled = planner_enabled;
        let planner_enabled = true;
        let _gate = self.gate.lock().await;
        self.load_locked().await?;
        if let Some(key) = &key {
            if key.is_empty() || key.len() > 256 {
                return Err(ServiceError::new(
                    "INVALID_ARGUMENT",
                    "client_request_id 必须为 1–256 字节",
                ));
            }
            if let Some(old) = self
                .records()
                .tasks
                .values()
                .find(|t| t.client_request_id.as_ref() == Some(key))
                .cloned()
            {
                return if old.request_parameters == parameters {
                    Ok(old)
                } else {
                    Err(ServiceError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "相同 client_request_id 已用于不同参数",
                    ))
                };
            }
        }
        if let Some(id) = id {
            if let Some(t) = self.records().tasks.get(&id).cloned() {
                return Ok(t);
            }
        }
        let input = std::fs::canonicalize(input)
            .map_err(|_| ServiceError::new("INVALID_PATH", "本地输入不存在或不可访问"))?;
        if !input.is_dir()
            && !matches!(
                input
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("mp4" | "mov")
            )
        {
            return Err(ServiceError::new(
                "INVALID_INPUT",
                "仅支持 MP4/MOV 或图片目录",
            ));
        }
        if !projects_root.is_absolute() {
            return Err(ServiceError::new(
                "INVALID_PATH",
                "项目根目录必须为绝对路径",
            ));
        }
        // Admit a supplemental task atomically with its frozen source identity.
        // A concurrent repeated MCP request must never observe it as generation.
        let source_project_id = parameters
            .as_ref()
            .filter(|p| p.get("reshootOptions").is_some())
            .and_then(|p| p.get("source_project_id"))
            .cloned()
            .map(serde_json::from_value::<Uuid>)
            .transpose()?;
        let now = Utc::now();
        let task = TaskRecord {
            schema_version: 1,
            task_id: id.unwrap_or_else(Uuid::new_v4),
            run_id: None,
            project_id: None,
            project_path: None,
            input_type: if input.is_dir() {
                crate::project::ProjectInputType::Images
            } else {
                crate::project::ProjectInputType::Video
            },
            project_deleted: false,
            input_path: input,
            quality,
            source: Some(source.into()),
            configuration_inferred: false,
            task_kind: if source_project_id.is_some() {
                "reshoot"
            } else {
                "generation"
            }
            .into(),
            source_project_id,
            planner_enabled,
            projects_root,
            configuration: (*crate::presets::pipeline_optimization_config()).clone(),
            generation_policy: parameters
                .as_ref()
                .and_then(|p| p.get("generationPolicy"))
                .cloned()
                .map(serde_json::from_value)
                .transpose()?,
            automatically_retried: false,
            actual_configuration: None,
            status: TaskStatus::Created,
            stage: None,
            substage: None,
            operation: None,
            revision: 1,
            sequence: 0,
            created_at: now,
            updated_at: now,
            elapsed_ms: 0,
            elapsed_offset_ms: 0,
            runtime: None,
            progress: None,
            estimated_progress: None,
            current: None,
            total: None,
            unit: None,
            eta_seconds: None,
            error: None,
            result: None,
            runs: Vec::new(),
            client_request_id: key,
            request_parameters: parameters,
            recent_events: Vec::new(),
        };
        self.records().tasks.insert(task.task_id, task.clone());
        if let Err(e) = self.persist().await {
            self.records().tasks.remove(&task.task_id);
            return Err(e);
        }
        self.notify(task.task_id);
        Ok(task)
    }

    pub async fn import_projects(&self) -> Result<()> {
        self.initialize().await?;
        let overview = catalog::get_overview().await?;
        for summary in overview.projects {
            let _gate = self.gate.lock().await;
            if self
                .records()
                .tasks
                .values()
                .any(|t| t.project_id == Some(summary.id))
            {
                continue;
            }
            let (path, mut metadata) = match catalog::load_registered_project(summary.id).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let checkpoint: crate::project::PipelineStateFile =
                match tokio::fs::read(path.join("state.json"))
                    .await
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                {
                    Some(s) => s,
                    None => continue,
                };
            let status = match summary.status {
                ProjectStatus::Completed => TaskStatus::Completed,
                ProjectStatus::Failed => TaskStatus::Failed,
                ProjectStatus::Cancelled => TaskStatus::Cancelled,
                _ => TaskStatus::Interrupted,
            };
            if status == TaskStatus::Interrupted && metadata.status == ProjectStatus::Running {
                metadata.status = ProjectStatus::Interrupted;
                atomic_write_json(&path.join("project.json"), &metadata).await?;
            }
            let mut registered = BTreeMap::new();
            if let Ok(mut entries) = tokio::fs::read_dir(path.join("logs")).await {
                while let Some(entry) = entries.next_entry().await? {
                    let file = entry.path();
                    if registered.len() >= 32 {
                        break;
                    }
                    if file.extension().is_some_and(|s| s == "log") {
                        if let Ok(mut log) = logs::register(&path.join("logs"), &file, 0) {
                            log.end = std::fs::metadata(&file).ok().map(|m| m.len());
                            registered.insert(log.source.clone(), log);
                        }
                    }
                }
            }
            let id = summary.workspace_task_id.unwrap_or_else(Uuid::new_v4);
            let run_id = Uuid::new_v4();
            let task = TaskRecord {
                schema_version: 1,
                task_id: id,
                run_id: Some(run_id),
                project_id: Some(metadata.id),
                project_path: Some(path.clone()),
                input_type: metadata.input_type,
                project_deleted: false,
                input_path: metadata.source_path.clone(),
                quality: metadata.quality,
                source: summary.workspace_task_id.map(|_| "gui".into()),
                configuration_inferred: checkpoint.configuration.is_none(),
                task_kind: if metadata.reshoot.is_some() {
                    "reshoot"
                } else {
                    "generation"
                }
                .into(),
                source_project_id: metadata.reshoot.as_ref().map(|r| r.source_project_id),
                planner_enabled: checkpoint.planner_enabled,
                generation_policy: checkpoint.generation_policy,
                automatically_retried: checkpoint.automatically_retried(),
                projects_root: path.parent().unwrap_or(&path).into(),
                configuration: checkpoint
                    .configuration
                    .clone()
                    .unwrap_or_else(|| (*crate::presets::pipeline_optimization_config()).clone()),
                actual_configuration: Some(actual_snapshot(serde_json::to_value(&checkpoint)?)),
                status,
                stage: Some(checkpoint.stage),
                substage: None,
                operation: None,
                revision: 1,
                sequence: 0,
                created_at: metadata.created_at,
                updated_at: metadata.completed_at.unwrap_or(metadata.created_at),
                elapsed_ms: metadata.duration_ms.unwrap_or(0),
                elapsed_offset_ms: 0,
                runtime: None,
                progress: None,
                estimated_progress: None,
                current: None,
                total: None,
                unit: None,
                eta_seconds: None,
                error: metadata.failure_message.as_ref().map(|message| TaskError {
                    code: "pipeline_failed".into(),
                    message: message.clone(),
                    failed_stage: None,
                    engine: None,
                    exit_code: None,
                    classification: None,
                    classification_is_heuristic: false,
                    classification_version: None,
                    termination: None,
                    failure_id: None,
                    log_sources: registered.keys().cloned().collect(),
                }),
                result: metadata
                    .output
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()?,
                runs: vec![TaskRun {
                    run_id,
                    kind: "legacy".into(),
                    started_at: metadata.started_at.unwrap_or(metadata.created_at),
                    ended_at: Some(metadata.completed_at.unwrap_or(Utc::now())),
                    logs: registered,
                    last_engine: None,
                    last_exit_code: None,
                    error: None,
                    brush_executions: Vec::new(),
                    current_brush_execution: None,
                }],
                client_request_id: None,
                request_parameters: None,
                recent_events: vec![],
            };
            let task = {
                let mut registry = self.records();
                if let Some(existing) = registry.tasks.get_mut(&id) {
                    if existing.status.active() {
                        continue;
                    }
                    existing.project_id = task.project_id;
                    existing.project_path = task.project_path;
                    existing.status = task.status;
                    existing.stage = task.stage;
                    existing.actual_configuration = task.actual_configuration;
                    existing.generation_policy = task.generation_policy;
                    existing.automatically_retried = task.automatically_retried;
                    existing.result = task.result;
                    existing.revision += 1;
                    if let Some(run) = existing.runs.last_mut() {
                        for (source, log) in task.runs[0].logs.clone() {
                            run.logs.entry(source).or_insert(log);
                        }
                    }
                    existing.clone()
                } else {
                    registry.tasks.insert(id, task.clone());
                    task
                }
            };
            self.persist().await?;
            self.notify(task.task_id);
        }
        Ok(())
    }

    pub(crate) async fn capture_failure(
        &self,
        error: crate::commands::PipelineCommandError,
        paths: &[PathBuf],
    ) -> crate::commands::PipelineCommandError {
        self.capture_failure_with_executions(error, paths, &[], None)
            .await
    }

    async fn capture_failure_with_executions(
        &self,
        mut error: crate::commands::PipelineCommandError,
        paths: &[PathBuf],
        executions: &[crate::pipeline::brush_execution::BrushExecutionSnapshot],
        current: Option<usize>,
    ) -> crate::commands::PipelineCommandError {
        if error.code != "cancelled" {
            let project = error
                .project_id
                .as_ref()
                .and_then(|id| Uuid::parse_str(id).ok());
            error.failure_id = Some(Box::new(
                self.diagnostics
                    .capture_with_executions(
                        error.failed_stage,
                        error.engine,
                        error.failure_kind,
                        &error.message,
                        project,
                        paths,
                        executions,
                        current,
                    )
                    .await,
            ));
        }
        error
    }

    /// Admission runs on the application's executor even if its caller disappears.
    pub async fn start(
        &self,
        id: Uuid,
        engines: EnginePaths,
        telemetry: Option<TelemetryService>,
    ) -> Result<StartReceipt> {
        let service = self.clone();
        tokio::spawn(async move {
            service
                .start_inner(id, engines, telemetry, false, None, None)
                .await
        })
        .await
        .map_err(|e| ServiceError::new("START_FAILED", e.to_string()))?
    }
    pub async fn retry_failed(
        &self,
        id: Uuid,
        failed_run: Uuid,
        engines: EnginePaths,
        telemetry: Option<TelemetryService>,
    ) -> Result<StartReceipt> {
        let service = self.clone();
        tokio::spawn(async move {
            service
                .start_inner(id, engines, telemetry, true, Some(failed_run), None)
                .await
        })
        .await
        .map_err(|e| ServiceError::new("START_FAILED", e.to_string()))?
    }

    async fn start_inner(
        &self,
        id: Uuid,
        engines: EnginePaths,
        telemetry: Option<TelemetryService>,
        resume: bool,
        retry_failed_run: Option<Uuid>,
        expected_resume_run: Option<Uuid>,
    ) -> Result<StartReceipt> {
        let _gate = self.gate.lock().await;
        self.load_locked().await?;
        let mut task = self
            .records()
            .tasks
            .get(&id)
            .cloned()
            .ok_or_else(|| ServiceError::new("TASK_NOT_FOUND", "任务不存在"))?;
        if let Some(expected) = expected_resume_run {
            if task.run_id != Some(expected) {
                return Err(ServiceError::new(
                    "STALE_RUN_ID",
                    "The task execution has changed",
                ));
            }
            if !matches!(task.status, TaskStatus::Cancelled | TaskStatus::Interrupted) {
                return Err(ServiceError::new("TASK_NOT_RESUMABLE", "Only cancelled or interrupted tasks may resume; failed tasks require explicit retry"));
            }
            let root = task.project_path.as_ref().ok_or_else(|| {
                ServiceError::new(
                    "NO_RESUMABLE_CHECKPOINT",
                    "No project checkpoint was created",
                )
            })?;
            let saved: crate::project::PipelineStateFile = tokio::fs::read(root.join("state.json"))
                .await
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .ok_or_else(|| {
                    ServiceError::new(
                        "NO_RESUMABLE_CHECKPOINT",
                        "No readable checkpoint is available",
                    )
                })?;
            if !saved.features_complete
                && !saved.matching_complete
                && !saved.reconstruction_complete
                && !saved.brush_complete
                && !saved
                    .reshoot
                    .as_ref()
                    .is_some_and(|s| s.source_snapshot_complete)
                && !saved
                    .frames
                    .as_ref()
                    .is_some_and(|f| f.extracted_frames.is_some_and(|n| n > 0))
            {
                return Err(ServiceError::new(
                    "NO_RESUMABLE_CHECKPOINT",
                    "No completed work is available to resume",
                ));
            }
            // A stale flag alone is not resumable work. The Runner still performs
            // its full per-stage validation before reusing any of these images.
            let images = crate::video::list_images(&root.join("work/frames")).unwrap_or_default();
            if images.len() < 2 {
                return Err(ServiceError::new(
                    "NO_RESUMABLE_CHECKPOINT",
                    "Saved working images are missing; no valid work can be resumed",
                ));
            }
        }
        if retry_failed_run
            .is_some_and(|run| task.run_id != Some(run) || task.status != TaskStatus::Failed)
        {
            return Ok(StartReceipt::existing(&task));
        }
        let failed_retry = task.status == TaskStatus::Failed && resume;
        if task.status.active() || (!resume && task.status != TaskStatus::Created) {
            return Ok(StartReceipt::existing(&task));
        }
        if !resume || task.project_id.is_none() {
            if task.source.as_deref() == Some("mcp") {
                if let (Some(source), Some(expected)) = (
                    task.source_project_id,
                    task.request_parameters
                        .as_ref()
                        .and_then(|p| p.pointer("/reshootOptions/sourceIdentity"))
                        .and_then(Value::as_str),
                ) {
                    let current = PipelineRunner::new(engines.clone(), |_| {})
                        .inspect_reshoot_source_v2(source)
                        .await
                        .map_err(|e| ServiceError::new("INPUT_CHANGED", e.to_string()))?;
                    if current.source_identity != expected {
                        return Err(ServiceError::new(
                            "INPUT_CHANGED",
                            "Reshoot source changed; inspect and confirm again",
                        ));
                    }
                }
            }
            if let Some(expected) = task
                .request_parameters
                .as_ref()
                .and_then(|value| value.get("videoSourceIdentity"))
                .and_then(Value::as_str)
            {
                let actual = crate::video::panorama::source_identity(&task.input_path)
                    .map_err(|e| ServiceError::new("INPUT_CHANGED", e.to_string()))?;
                if actual != expected {
                    return Err(ServiceError::new(
                        "INPUT_CHANGED",
                        "素材文件已发生变化，请重新分析后再生成",
                    ));
                }
            }
        }
        if resume && task.status == TaskStatus::Completed {
            return Err(ServiceError::new(
                "TASK_NOT_STARTABLE",
                "已完成项目无需恢复",
            ));
        }
        if let Some(active) = self.records().tasks.values().find(|t| t.status.active()) {
            return Err(ServiceError {
                code: "TASK_BUSY".into(),
                message: "已有任务正在运行".into(),
                task_id: Some(active.task_id),
                run_id: active.run_id,
                details: None,
            });
        }
        let original_task = task.clone();
        if failed_retry || (!resume && task.generation_policy.is_none()) {
            task.planner_enabled = true;
            task.configuration = (*crate::presets::pipeline_optimization_config()).clone();
            task.request_parameters = Some(generation_parameters(task.request_parameters.take())?);
        }
        let run_id = Uuid::new_v4();
        let service = self.clone();
        let lifecycle = self.clone();
        let effectiveness = PlannerEffectivenessTracker::default();
        let session = telemetry.map(|telemetry| {
            Arc::new(PipelineTelemetrySession::new_with_planner_evaluation(
                telemetry,
                task.quality,
                if task.input_path.is_dir() {
                    TelemetryInputType::Images
                } else {
                    TelemetryInputType::Video
                },
                if task.project_id.is_some() {
                    TelemetryRunKind::Resume
                } else {
                    TelemetryRunKind::New
                },
                task.planner_enabled,
                effectiveness.clone(),
            ))
        });
        let observe = session.clone();
        let reshoot_options = task
            .request_parameters
            .as_ref()
            .and_then(|v| v.get("reshootOptions"))
            .map(|v| serde_json::from_value::<crate::pipeline::reshoot::ReshootOptions>(v.clone()))
            .transpose()
            .map_err(|_| ServiceError::new("INVALID_ARGUMENT", "Invalid saved reshoot options"))?;
        let policy = task
            .request_parameters
            .as_ref()
            .and_then(|p| p.get("generationPolicy"))
            .cloned()
            .map(serde_json::from_value::<crate::pipeline::generation_policy::GenerationPolicy>)
            .transpose()?;
        let runner = Arc::new(
            PipelineRunner::new_with_planner(engines, task.planner_enabled, move |event| {
                if let Some(s) = &observe {
                    s.observe(&event);
                }
                service.on_event(id, run_id, event);
            })
            .with_effectiveness_tracker(effectiveness)
            .with_generation_policy(if !resume || failed_retry {
                Some(policy.unwrap_or_default())
            } else {
                None
            })
            .with_failed_retry_policy(failed_retry.then(|| policy.unwrap_or_default()))
            .with_video_projection(
                task.request_parameters
                    .as_ref()
                    .and_then(|v| v.get("videoProjection"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            )
            .with_workspace_task_id(Some(id))
            .with_reshoot_options(reshoot_options)
            .with_image_matching_strategy(
                task.request_parameters
                    .as_ref()
                    .and_then(|v| v.get("imageMatchingStrategy"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            )
            .with_input_boundary(
                (task.source.as_deref() == Some("mcp")).then(|| task.input_path.clone()),
            )
            .with_lifecycle_observer(move |update| lifecycle.on_runner_update(id, run_id, update)),
        );
        let starting = {
            let mut registry = self.records();
            let t = registry.tasks.get_mut(&id).expect("task");
            t.planner_enabled = task.planner_enabled;
            t.configuration = task.configuration.clone();
            t.request_parameters = task.request_parameters.clone();
            if !resume || failed_retry {
                t.generation_policy = policy;
                t.automatically_retried = false;
            }
            t.run_id = Some(run_id);
            t.status = TaskStatus::Starting;
            t.elapsed_offset_ms = t.elapsed_ms;
            t.progress = None;
            t.current = None;
            t.total = None;
            t.unit = None;
            t.eta_seconds = None;
            t.runtime = None;
            let previous_error = t.error.clone();
            if let Some(run) = t.runs.last_mut() {
                if run.error.is_none() {
                    run.error = previous_error;
                }
            }
            t.error = None;
            t.stage = None;
            t.substage = None;
            t.operation = None;
            t.estimated_progress = None;
            t.sequence = 0;
            t.recent_events.clear();
            t.revision += 1;
            t.updated_at = Utc::now();
            t.runs.push(TaskRun {
                run_id,
                kind: if failed_retry {
                    "retry"
                } else if task.project_id.is_some() {
                    "resume"
                } else {
                    &task.task_kind
                }
                .into(),
                started_at: Utc::now(),
                ended_at: None,
                logs: Default::default(),
                last_engine: None,
                last_exit_code: None,
                error: None,
                brush_executions: Vec::new(),
                current_brush_execution: None,
            });
            t.clone()
        };
        if let Err(e) = self.persist().await {
            self.records().tasks.insert(id, original_task);
            *self.active.lock().await = None;
            return Err(e);
        }
        *self.active.lock().await = Some(runner.clone());
        self.notify(id);
        let service = self.clone();
        tokio::spawn(async move {
            let _cleanup = ExecutionCleanup {
                service: service.clone(),
                task: id,
                run: run_id,
                runner: runner.clone(),
            };
            if let Some(s) = &session {
                s.generation_started();
            }
            let execute_runner = runner.clone();
            let execution = task.clone();
            #[cfg(test)]
            let test_executor = service.executor.lock().unwrap().clone();
            let worker = tokio::spawn(crate::presets::with_pipeline_config(
                task.configuration.clone(),
                async move {
                    #[cfg(test)]
                    if let Some(executor) = test_executor {
                        return executor(execution, execute_runner).await;
                    }
                    if let Some(project) = execution.project_id {
                        execute_runner.resume(project).await
                    } else if let Some(source) = execution.source_project_id {
                        execute_runner
                            .generate_incremental_reshoot(
                                source,
                                &execution.input_path,
                                if execution.input_path.is_dir() {
                                    ReshootInputType::Images
                                } else {
                                    ReshootInputType::Video
                                },
                                &execution.projects_root,
                                execution.quality,
                            )
                            .await
                    } else {
                        execute_runner
                            .generate(
                                &execution.input_path,
                                execution.quality,
                                &execution.projects_root,
                            )
                            .await
                    }
                },
            ));
            let result = worker.await.unwrap_or_else(|e| {
                runner.cancel();
                Err(crate::error::SplatError::Process(format!(
                    "任务执行异常：{e}"
                )))
            });
            if let Some(s) = session {
                match &result {
                    Ok(r) => s.generation_completed(
                        r.duration_ms,
                        r.input_images,
                        r.source_duration_seconds,
                    ),
                    Err(e) => s.generation_failed(e),
                }
            }
            service.finish(id, run_id, runner, result).await;
        });
        let mut receipt = StartReceipt::existing(&starting);
        receipt.accepted = true;
        Ok(receipt)
    }
    fn on_event(&self, id: Uuid, run: Uuid, mut event: PipelineEvent) {
        // A single lock orders record updates, queued events, and emitted snapshots.
        // In particular, an old run cannot enqueue after a new run's notification.
        let mut pending = self.notifications.lock().unwrap_or_else(|p| p.into_inner());
        let immediate = {
            let mut registry = self.records();
            let Some(t) = registry.tasks.get_mut(&id) else {
                return;
            };
            if t.run_id != Some(run) || !t.status.active() {
                return;
            }
            if event.sequence > 0 && event.sequence <= t.sequence {
                return;
            }
            // The supervisor, after durable result/error capture, owns terminal transitions.
            if matches!(
                event.stage,
                PipelineStage::Completed | PipelineStage::Failed | PipelineStage::Cancelled
            ) {
                return;
            }
            let starting = t.status == TaskStatus::Starting;
            let lifecycle = event.kind == crate::pipeline::EventKind::Stage;
            if event
                .operation
                .as_ref()
                .zip(t.operation.as_ref())
                .is_some_and(|(incoming, current)| incoming.id < current.id)
                || (!lifecycle
                    && event
                        .operation
                        .as_ref()
                        .zip(t.operation.as_ref())
                        .is_some_and(|(incoming, current)| incoming.id != current.id))
            {
                return;
            }
            let stage_changed = lifecycle
                && (t.stage != Some(event.stage)
                    || t.substage != event.substage
                    || event.operation.as_ref().map(|op| op.id)
                        != t.operation.as_ref().map(|op| op.id));
            if starting {
                t.status = TaskStatus::Running;
            }
            if stage_changed {
                t.progress = None;
                t.current = None;
                t.total = None;
                t.unit = None;
                t.eta_seconds = None;
                t.runtime = None;
            }
            let configuration = t
                .actual_configuration
                .as_ref()
                .and_then(|v| v.pointer("/brush_training/resolved"))
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            if let Some(execution) = t.runs.last_mut() {
                execution.observe_brush(&event, configuration);
            }
            t.revision += 1;
            t.sequence = t.sequence.max(event.sequence);
            if lifecycle {
                t.stage = Some(event.stage);
                t.substage = event.substage.clone();
                t.operation = event.operation.clone();
            }
            t.updated_at = event.timestamp;
            t.elapsed_ms = t.elapsed_offset_ms.saturating_add(event.elapsed_ms);
            t.estimated_progress = Some(
                t.estimated_progress
                    .unwrap_or(0.0)
                    .max(event.progress.min(99.9)),
            );
            let progress_event = matches!(
                event.kind,
                crate::pipeline::EventKind::Stage | crate::pipeline::EventKind::Progress
            ) && t.stage == Some(event.stage);
            if progress_event {
                t.current = event.current;
                t.total = event.total;
                t.unit = event.unit.clone();
                t.progress = if event.indeterminate {
                    None
                } else {
                    event.stage_progress
                };
            }
            if let Some(runtime) = event
                .runtime
                .as_ref()
                .filter(|_| t.stage == Some(event.stage))
            {
                t.runtime = Some(runtime.clone());
                t.eta_seconds = runtime
                    .training
                    .as_ref()
                    .and_then(|training| training.remaining_seconds);
            }
            event.task_id = Some(id);
            event.run_id = Some(run);
            event.revision = t.revision;
            if event.kind != crate::pipeline::EventKind::Runtime
                && event.kind != crate::pipeline::EventKind::Heartbeat
            {
                t.recent_events.push(event.clone());
                if t.recent_events.len() > 500 {
                    t.recent_events.remove(0);
                }
            }
            starting || stage_changed || event.kind == crate::pipeline::EventKind::Capability
        };
        pending.task(id, Some(run)).push(event);
        if immediate {
            // Runner callbacks are synchronous. A lifecycle operation in progress
            // will flush this batch itself after its checkpoint becomes durable.
            if let Ok(_gate) = self.gate.try_lock() {
                if let Some(emit) = self.emitter() {
                    self.emit_pending(&mut pending, id, &emit, false);
                }
            }
        }
    }
    fn on_runner_update(&self, id: Uuid, run: Uuid, update: RunnerUpdate) {
        let mut pending = self.notifications.lock().unwrap_or_else(|p| p.into_inner());
        let immediate = matches!(&update, RunnerUpdate::Project(_));
        {
            let mut registry = self.records();
            let Some(t) = registry.tasks.get_mut(&id) else {
                return;
            };
            if t.run_id != Some(run) || !t.status.active() {
                return;
            }
            match update {
                RunnerUpdate::BrushPreparation => {
                    if let Some(execution) = t.runs.last_mut() {
                        execution.current_brush_execution = None;
                    }
                }
                RunnerUpdate::Configuration(configuration) => {
                    t.generation_policy = configuration
                        .get("generation_policy")
                        .filter(|p| !p.is_null())
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok());
                    t.automatically_retried = configuration
                        .get("automatically_retried")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    t.actual_configuration = Some(configuration);
                }
                RunnerUpdate::Project(metadata) => {
                    t.project_id = Some(metadata.id);
                    t.project_path = Some(metadata.project_path);
                }
                RunnerUpdate::LogOpened { path, offset } => {
                    if let Some(root) = &t.project_path {
                        if let Ok(log) = logs::register(&root.join("logs"), &path, offset) {
                            let logs = &mut t.runs.last_mut().expect("run").logs;
                            match logs.get(&log.source) {
                                Some(existing) if existing.file_id == log.file_id => {}
                                _ => {
                                    logs.insert(log.source.clone(), log);
                                }
                            }
                        }
                    }
                }
                RunnerUpdate::ProcessExited { engine, exit_code } => {
                    if let Some(r) = t.runs.last_mut() {
                        r.last_engine = Some(engine);
                        r.last_exit_code = exit_code;
                        if engine == crate::pipeline::PipelineEngine::Brush {
                            if let Some(execution) = r
                                .brush_executions
                                .iter_mut()
                                .rev()
                                .find(|v| v.ended_at.is_none())
                            {
                                execution.finish(exit_code);
                            }
                        }
                    }
                }
            }
            t.revision += 1;
        }
        pending.task(id, Some(run));
        if immediate {
            if let Ok(_gate) = self.gate.try_lock() {
                if let Some(emit) = self.emitter() {
                    self.emit_pending(&mut pending, id, &emit, false);
                }
            }
        }
    }
    async fn finish(
        &self,
        id: Uuid,
        run: Uuid,
        runner: Arc<PipelineRunner>,
        result: crate::error::Result<PipelineResult>,
    ) {
        let _gate = self.gate.lock().await;
        let context = runner.failure_context();
        let (executions, current_execution) = {
            let mut registry = self.records();
            let Some(task) = registry
                .tasks
                .get_mut(&id)
                .filter(|t| t.run_id == Some(run))
            else {
                return;
            };
            // Cancellation and launch failures do not necessarily emit ProcessExited.
            // Freeze the local evidence before preparing a diagnostic or terminal record.
            task.runs
                .last_mut()
                .map(|v| {
                    for execution in &mut v.brush_executions {
                        execution.finish(None);
                    }
                    (v.brush_executions.clone(), v.current_brush_execution)
                })
                .unwrap_or_default()
        };
        let input = self
            .records()
            .tasks
            .get(&id)
            .map(|t| t.input_path.clone())
            .unwrap_or_default();
        let error = match &result {
            Ok(_) => None,
            Err(e) => {
                let error = crate::commands::PipelineCommandError::from_runner(e, &runner);
                Some(if executions.is_empty() {
                    self.capture_failure(error, &[input]).await
                } else {
                    self.capture_failure_with_executions(
                        error,
                        &[input],
                        &executions,
                        current_execution,
                    )
                    .await
                })
            }
        };
        let failure_id = error
            .as_ref()
            .and_then(|e| e.failure_id.as_deref())
            .copied();
        let actual = None::<Value>;
        let task = {
            let mut registry = self.records();
            let Some(t) = registry.tasks.get_mut(&id) else {
                return;
            };
            if t.run_id != Some(run) {
                return;
            }
            t.status = match &result {
                Ok(_) => TaskStatus::Completed,
                Err(crate::error::SplatError::Cancelled) => TaskStatus::Cancelled,
                Err(_) => TaskStatus::Failed,
            };
            t.stage = Some(match t.status {
                TaskStatus::Completed => PipelineStage::Completed,
                TaskStatus::Cancelled => PipelineStage::Cancelled,
                _ => PipelineStage::Failed,
            });
            if let Some(r) = t.runs.last_mut() {
                r.ended_at = Some(Utc::now());
                for log in r.logs.values_mut() {
                    log.end = std::fs::metadata(&log.path).ok().map(|m| m.len());
                }
            }
            t.project_id = context.project_id.or(t.project_id);
            t.project_path = context.project_path.or(t.project_path.clone());
            if let Some(actual) = actual {
                t.actual_configuration = Some(actual_snapshot(actual));
                if let Some(actual) = &t.actual_configuration {
                    t.generation_policy = actual
                        .get("generation_policy")
                        .filter(|p| !p.is_null())
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok());
                    t.automatically_retried = actual
                        .get("automatically_retried")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                }
            }
            t.result = result
                .as_ref()
                .ok()
                .and_then(|r| serde_json::to_value(r).ok());
            if let Some(e) = error {
                t.error = Some(TaskError {
                    code: e.code.into(),
                    message: e.message,
                    failed_stage: e.failed_stage,
                    engine: e.engine,
                    exit_code: e.termination.as_ref().and_then(|status| status.exit_code),
                    classification: e.failure_kind.map(str::to_owned),
                    classification_is_heuristic: e.classification_is_heuristic,
                    classification_version: e.classification_version,
                    termination: e.termination.map(|status| *status),
                    failure_id,
                    log_sources: t
                        .runs
                        .last()
                        .map(|r| r.logs.keys().cloned().collect())
                        .unwrap_or_default(),
                });
            }
            if let Some(run) = t.runs.last_mut() {
                run.error = t.error.clone();
            }
            t.updated_at = Utc::now();
            t.elapsed_ms = runner
                .elapsed_ms()
                .saturating_add(runner.elapsed_offset_ms().max(t.elapsed_offset_ms));
            t.eta_seconds = None;
            if t.status == TaskStatus::Completed {
                t.estimated_progress = Some(100.0);
                t.progress = Some(100.0);
            }
            t.revision += 1;
            t.clone()
        };
        if let Err(e) = self.persist().await {
            tracing::error!(code=%e.code,"Could not persist task terminal state");
        }
        *self.active.lock().await = None;
        self.notify(task.task_id);
    }
    pub async fn cancel(&self, id: Uuid, run: Uuid) -> Result<TaskRecord> {
        let service = self.clone();
        tokio::spawn(async move {
            let _gate = service.gate.lock().await;
            service.load_locked().await?;
            let old = service
                .records()
                .tasks
                .get(&id)
                .cloned()
                .ok_or_else(|| ServiceError::new("TASK_NOT_FOUND", "任务不存在"))?;
            if old.run_id != Some(run) {
                return Err(ServiceError::new("STALE_RUN_ID", "取消请求不属于当前执行"));
            }
            if !old.status.active() {
                return Ok(old);
            }
            let task = {
                let mut registry = service.records();
                let t = registry.tasks.get_mut(&id).expect("task");
                t.status = TaskStatus::Cancelling;
                t.revision += 1;
                t.updated_at = Utc::now();
                t.clone()
            };
            if let Err(e) = service.persist().await {
                service.records().tasks.insert(id, old);
                return Err(e);
            }
            service.notify(id);
            if let Some(runner) = service.active.lock().await.as_ref() {
                runner.cancel();
            }
            Ok(task)
        })
        .await
        .map_err(|e| ServiceError::new("CANCEL_FAILED", e.to_string()))?
    }
    pub async fn resume(
        &self,
        project: Uuid,
        engines: EnginePaths,
        telemetry: Option<TelemetryService>,
    ) -> Result<StartReceipt> {
        self.import_projects().await?;
        let id = self
            .all()
            .await?
            .into_iter()
            .find(|t| t.project_id == Some(project))
            .ok_or_else(|| ServiceError::new("TASK_NOT_FOUND", "项目任务不存在"))?
            .task_id;
        let service = self.clone();
        tokio::spawn(async move {
            service
                .start_inner(id, engines, telemetry, true, None, None)
                .await
        })
        .await
        .map_err(|e| ServiceError::new("START_FAILED", e.to_string()))?
    }

    pub async fn resume_identified(
        &self,
        id: Uuid,
        expected_run: Uuid,
        engines: EnginePaths,
        telemetry: Option<TelemetryService>,
    ) -> Result<StartReceipt> {
        let service = self.clone();
        tokio::spawn(async move {
            service
                .start_inner(id, engines, telemetry, true, None, Some(expected_run))
                .await
        })
        .await
        .map_err(|e| ServiceError::new("START_FAILED", e.to_string()))?
    }

    pub async fn mark_reshoot(&self, id: Uuid, source: Uuid) -> Result<()> {
        let _gate = self.gate.lock().await;
        {
            let mut r = self.records();
            let t = r
                .tasks
                .get_mut(&id)
                .ok_or_else(|| ServiceError::new("TASK_NOT_FOUND", "任务不存在"))?;
            t.task_kind = "reshoot".into();
            t.source_project_id = Some(source);
        }
        self.persist().await
    }
    pub async fn wait(&self, id: Uuid) -> Result<TaskRecord> {
        loop {
            let task = self.get(id).await?;
            if !task.status.active() {
                return Ok(task);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    pub async fn reconcile_projects(
        &self,
        projects: &[crate::project::catalog::ProjectSummary],
    ) -> Result<()> {
        let _gate = self.gate.lock().await;
        self.load_locked().await?;
        let mut changes = vec![];
        {
            let mut registry = self.records();
            for project in projects {
                if project.status != ProjectStatus::Interrupted {
                    continue;
                }
                if let Some(task) = registry.tasks.values_mut().find(|task| {
                    task.project_id == Some(project.id)
                        && task.status == TaskStatus::Completed
                        && !task.project_deleted
                }) {
                    task.status = TaskStatus::Interrupted;
                    task.result = None;
                    task.revision += 1;
                    task.updated_at = Utc::now();
                    task.error = Some(TaskError {
                        code: "ARTIFACT_UNAVAILABLE".into(),
                        message: project.failure_message.clone().unwrap_or_else(|| {
                            "项目产物当前不可访问，可以使用现有恢复入口检查并修复".into()
                        }),
                        failed_stage: None,
                        engine: None,
                        exit_code: None,
                        classification: None,
                        classification_is_heuristic: false,
                        classification_version: None,
                        termination: None,
                        failure_id: None,
                        log_sources: task
                            .runs
                            .last()
                            .map(|run| run.logs.keys().cloned().collect())
                            .unwrap_or_default(),
                    });
                    changes.push(task.clone());
                }
            }
        }
        if !changes.is_empty() {
            self.persist().await?;
            for task in changes {
                self.notify(task.task_id);
            }
        }
        Ok(())
    }
    pub async fn mark_project_deleted(&self, project: Uuid) -> Result<()> {
        let _gate = self.gate.lock().await;
        let task = {
            let mut registry = self.records();
            let Some(task) = registry
                .tasks
                .values_mut()
                .find(|task| task.project_id == Some(project))
            else {
                return Ok(());
            };
            task.project_deleted = true;
            task.project_path = None;
            task.result = None;
            task.revision += 1;
            task.updated_at = Utc::now();
            task.clone()
        };
        self.persist().await?;
        self.notify(task.task_id);
        Ok(())
    }
    pub fn spawn_checkpoint_writer(&self) -> tauri::async_runtime::JoinHandle<()> {
        let service = self.clone();
        // Tauri's synchronous setup hook runs on the UI thread, outside a
        // current Tokio runtime. Spawn on the application-owned runtime.
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let _gate = service.gate.lock().await;
                if service.records().tasks.values().any(|t| t.status.active()) {
                    let _ = service.persist().await;
                }
            }
        })
    }
}

pub fn authorize_input(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = std::fs::canonicalize(path)
        .map_err(|_| ServiceError::new("INVALID_PATH", "输入不存在或不可访问"))?;
    if !roots
        .iter()
        .filter_map(|r| {
            std::fs::canonicalize(r)
                .ok()
                .filter(|resolved| resolved == r)
        })
        .any(|r| path.starts_with(r))
    {
        return Err(ServiceError::new(
            "PATH_NOT_AUTHORIZED",
            "素材路径不在设置中授权的目录内",
        ));
    }
    Ok(path)
}

fn actual_snapshot(state: Value) -> Value {
    let frames = state.get("frames");
    let retried = state.get("generationPolicy").is_some_and(Value::is_object)
        && (state
            .pointer("/brushTraining/oomRetryStarted")
            .and_then(Value::as_bool)
            == Some(true)
            || [
                "/reshoot/recipe/reconstructionFallbacks",
                "/reshoot/recipe/trainingFallbacks",
            ]
            .iter()
            .any(|p| {
                state
                    .pointer(p)
                    .and_then(Value::as_u64)
                    .is_some_and(|n| n > 0)
            }));
    let mut result = serde_json::json!({"quality":state.get("preset"),"planner_enabled":state.get("plannerEnabled"),"resolution_policy_version":state.get("resolutionPolicyVersion"),"resolution_plan":state.get("resolutionPlan"),"brush_training":state.get("brushTraining"),"frame_plan":{"sampling_fps":frames.and_then(|f|f.get("samplingFps")),"estimated_frames":frames.and_then(|f|f.get("estimatedFrames")),"extracted_frames":frames.and_then(|f|f.get("extractedFrames")),"mask_count":frames.and_then(|f|f.get("maskCount"))}});
    result["generation_policy"] = state
        .get("generationPolicy")
        .cloned()
        .unwrap_or(Value::Null);
    result["automatically_retried"] = Value::Bool(retried);
    result["reshoot_recipe"] = state
        .pointer("/reshoot/recipe")
        .cloned()
        .unwrap_or(Value::Null);
    extend_actual_summary(&mut result, &state);
    result
}

fn extend_actual_summary(result: &mut Value, state: &Value) {
    let panorama = state.get("panorama").is_some_and(Value::is_object);
    result["input_projection"] = if panorama {
        Value::String("equirectangular".into())
    } else {
        state
            .get("videoProjection")
            .filter(|v| matches!(v.as_str(), Some("perspective" | "equirectangular")))
            .cloned()
            .unwrap_or(Value::Null)
    };
    result["image_matching_strategy"] =
        if state.get("inputType").and_then(Value::as_str) == Some("video") {
            Value::String("sequential".into())
        } else {
            state
                .get("imageMatchingStrategy")
                .cloned()
                .unwrap_or_else(|| Value::String("exhaustive".into()))
        };
    result["gps"] = serde_json::json!({"status":state.pointer("/gps/status"),"reference_valid":state.pointer("/gps/referenceValid")});
    result["backfill"] = serde_json::json!({"status":state.pointer("/bridgeBackfill/status"),"initial_frames":state.pointer("/frames/initialExtractedFrames"),"actual_frames":state.pointer("/frames/extractedFrames"),"total_frame_budget":state.pointer("/frames/rescueMaxFrames")});
}

struct ExecutionCleanup {
    service: TaskService,
    task: Uuid,
    run: Uuid,
    runner: Arc<PipelineRunner>,
}
impl Drop for ExecutionCleanup {
    fn drop(&mut self) {
        self.runner.cancel();
        let service = self.service.clone();
        let runner = self.runner.clone();
        let task = self.task;
        let run = self.run;
        // Destructors can also run outside an entered Tokio runtime.
        tauri::async_runtime::spawn(async move {
            if service
                .get(task)
                .await
                .is_ok_and(|t| t.run_id == Some(run) && t.status.active())
            {
                service
                    .finish(
                        task,
                        run,
                        runner,
                        Err(crate::error::SplatError::Process(
                            "执行监督器异常结束".into(),
                        )),
                    )
                    .await;
            }
        });
    }
}

pub(crate) fn checkpoint_snapshot(state: &crate::project::PipelineStateFile) -> Value {
    let mut result = serde_json::json!({"quality":state.preset,"planner_enabled":state.planner_enabled,"resolution_policy_version":state.resolution_policy_version,"resolution_plan":state.resolution_plan,"brush_training":state.brush_training,"reshoot_recipe":state.reshoot.as_ref().and_then(|r|r.recipe.as_ref()),"frame_plan":{"sampling_fps":state.frames.as_ref().map(|f|f.sampling_fps),"estimated_frames":state.frames.as_ref().map(|f|f.estimated_frames),"extracted_frames":state.frames.as_ref().and_then(|f|f.extracted_frames),"mask_count":state.frames.as_ref().and_then(|f|f.mask_count)}});
    if let Ok(value) = serde_json::to_value(state) {
        extend_actual_summary(&mut result, &value);
    }
    result
}
