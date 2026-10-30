//! Application-owned export jobs; encoding stays in the existing desktop Worker.
use super::*;
use crate::{
    commands::{
        model_export::{self, ModelExportResult, ModelFormat},
        PreviewController,
    },
    project::{catalog, manager::atomic_write_json, GaussianTransform},
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::Emitter;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Ply,
    CompressedPly,
    Spz,
    Sog,
}
impl ExportFormat {
    fn internal(self) -> ModelFormat {
        match self {
            Self::Ply => ModelFormat::Ply,
            Self::CompressedPly => ModelFormat::CompressedPly,
            Self::Spz => ModelFormat::Spz,
            Self::Sog => ModelFormat::Sog,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Starting,
    Preparing,
    AwaitingConfirmation,
    Encoding,
    Validating,
    Publishing,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}
impl Phase {
    fn active(self) -> bool {
        !matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
    fn rank(self) -> u8 {
        match self {
            Self::Starting => 0,
            Self::Preparing => 1,
            Self::AwaitingConfirmation => 2,
            Self::Encoding => 3,
            Self::Validating => 4,
            Self::Publishing => 5,
            _ => 6,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adjustment {
    pub center: [f64; 3],
    pub scale: f64,
}
impl Adjustment {
    fn valid(&self) -> bool {
        self.center.iter().all(|v| v.is_finite())
            && self.scale.is_finite()
            && self.scale > 0.
            && self.scale <= 1.
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Job {
    export_id: Uuid,
    project_id: Uuid,
    format: ExportFormat,
    phase: Phase,
    progress: Option<f64>,
    confirmation_id: Option<u64>,
    adjustment: Option<Adjustment>,
    result: Option<Value>,
    error: Option<Value>,
    client_request_id: String,
    executor_id: Uuid,
    session_id: Uuid,
    transform: GaussianTransform,
    edit_revision: u64,
}
impl Job {
    fn check_confirmation(&self, id: u64) -> Result<()> {
        if self.phase != Phase::AwaitingConfirmation || self.confirmation_id != Some(id) {
            return Err(ServiceError::new(
                "STALE_CONFIRMATION",
                "No matching pending SPZ confirmation",
            ));
        }
        Ok(())
    }
    fn observe(&mut self, update: ExecutorUpdate) -> Result<bool> {
        if !self.phase.active()
            || matches!(self.phase, Phase::AwaitingConfirmation | Phase::Cancelling)
        {
            return Ok(false);
        }
        if matches!(
            update.phase,
            Phase::Completed
                | Phase::Cancelled
                | Phase::Interrupted
                | Phase::Cancelling
                | Phase::Starting
                | Phase::Failed
        ) {
            return Err(ServiceError::new(
                "INVALID_EXPORT_UPDATE",
                "Only the backend may finish or cancel exports",
            ));
        }
        if update.phase == Phase::AwaitingConfirmation
            && (self.format != ExportFormat::Spz
                || self.phase.rank() > Phase::Preparing.rank()
                || self.adjustment.is_some()
                || update.confirmation_id.is_none()
                || !update.adjustment.as_ref().is_some_and(Adjustment::valid))
        {
            return Err(ServiceError::new(
                "INVALID_EXPORT_UPDATE",
                "Invalid SPZ confirmation",
            ));
        }
        if update.phase.rank() < self.phase.rank() {
            return Ok(false);
        }
        let changed = self.phase != update.phase;
        self.phase = update.phase;
        self.progress = update
            .progress
            .filter(|p| p.is_finite() && (0.0..100.0).contains(p));
        if update.phase == Phase::AwaitingConfirmation {
            self.confirmation_id = update.confirmation_id;
            self.adjustment = update.adjustment;
            self.progress = None;
        }
        Ok(changed)
    }
    fn public(&self) -> Value {
        let description = |locale: &str| {
            [
                "export.spzAdjustReason",
                "export.spzAdjustAction",
                "export.spzAdjustImpact",
                "export.spzAdjustAdvice",
            ]
            .iter()
            .map(|key| {
                generation::text(key, 0)[locale]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n\n")
        };
        let confirmation = if self.phase == Phase::AwaitingConfirmation {
            Some(
                json!({"confirmation_id":self.confirmation_id,"adjustment":self.adjustment,
            "title":generation::text("export.spzAdjustTitle",0),"description":{"zh-CN":description("zh-CN"),"en":description("en")},
            "confirm_label":generation::text("export.spzAdjustConfirm",0),"cancel_label":generation::text("common.cancel",0)}),
            )
        } else {
            None
        };
        json!({"export_id":self.export_id,"project_id":self.project_id,"format":self.format,"phase":self.phase,"progress":self.progress,
            "edit_revision":self.edit_revision,"confirmation":confirmation,"result":self.result,"error":self.error})
    }
    fn work(&self) -> Value {
        json!({"exportId":self.export_id,"executorId":self.executor_id,"sessionId":self.session_id,
        "projectId":self.project_id,"format":self.format.internal(),"transform":self.transform,"editRevision":self.edit_revision})
    }
}
#[derive(Default)]
struct Registry {
    loaded: bool,
    executor: Option<Uuid>,
    executor_epoch: u64,
    jobs: BTreeMap<Uuid, Job>,
}
#[derive(Clone)]
pub struct ExportController {
    inner: Arc<Mutex<Registry>>,
    path: PathBuf,
    page_epoch: Arc<AtomicU64>,
}
impl Default for ExportController {
    fn default() -> Self {
        Self::at(
            catalog::app_data_root()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("mcp-exports.json"),
        )
    }
}
impl ExportController {
    fn at(path: PathBuf) -> Self {
        Self {
            inner: Default::default(),
            path,
            page_epoch: Default::default(),
        }
    }
    pub(crate) fn page_loading(&self, app: tauri::AppHandle) {
        let epoch = self.page_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let controller = self.clone();
        tauri::async_runtime::spawn(async move {
            let r = controller.inner.lock().await;
            let old = if r.executor_epoch < epoch {
                r.executor
            } else {
                None
            };
            drop(r);
            if let Some(old) = old {
                let _ = controller.invalidate(&app, Some(old)).await;
            }
        });
    }
    async fn persist(&self, r: &Registry) -> Result<()> {
        atomic_write_json(&self.path, &r.jobs.values().collect::<Vec<_>>()).await?;
        Ok(())
    }
    async fn load(&self, r: &mut Registry) -> Result<()> {
        if r.loaded {
            return Ok(());
        }
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => {
                let jobs: Vec<Job> = serde_json::from_slice(&bytes)?;
                r.jobs = jobs.into_iter().map(|j| (j.export_id, j)).collect();
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        for job in r.jobs.values_mut().filter(|j| j.phase.active()) {
            job.phase = Phase::Interrupted;
            job.confirmation_id = None;
            job.progress = None;
            if let Ok((root, _)) = catalog::load_registered_project(job.project_id).await {
                let temporary =
                    root.join(format!(".ooosplat-model-{}.encoded.tmp", job.session_id));
                let _ = tokio::fs::remove_file(temporary).await;
            }
        }
        self.persist(r).await?;
        r.loaded = true;
        Ok(())
    }
    async fn status(&self, id: Uuid) -> Result<Value> {
        let mut r = self.inner.lock().await;
        self.load(&mut r).await?;
        r.jobs
            .get(&id)
            .map(Job::public)
            .ok_or_else(|| ServiceError::new("EXPORT_NOT_FOUND", "Export job not found"))
    }
    pub(crate) async fn published(&self, session: Uuid, result: &ModelExportResult) -> Result<()> {
        let mut r = self.inner.lock().await;
        self.load(&mut r).await?;
        let old = r
            .jobs
            .values()
            .find(|j| j.session_id == session)
            .cloned()
            .ok_or_else(|| ServiceError::new("EXPORT_NOT_FOUND", "Export job not found"))?;
        if !old.phase.active() {
            return Err(ServiceError::new(
                "EXPORT_INTERRUPTED",
                "Export job is no longer active",
            ));
        }
        let mut result = snake(serde_json::to_value(result)?);
        result["format"] = serde_json::to_value(old.format)?;
        if let Some(adjustment) = &old.adjustment {
            result["coordinate_adjustment"] = serde_json::to_value(adjustment)?;
        }
        let job = r.jobs.get_mut(&old.export_id).unwrap();
        job.phase = Phase::Completed;
        job.progress = Some(100.);
        job.confirmation_id = None;
        job.result = Some(result);
        if let Err(error) = self.persist(&r).await {
            r.jobs.insert(old.export_id, old);
            return Err(error);
        }
        Ok(())
    }
    async fn start(&self, app: &tauri::AppHandle, request: Start) -> Result<Value> {
        if request.client_request_id.is_empty() || request.client_request_id.len() > 256 {
            return Err(ServiceError::new(
                "INVALID_ARGUMENT",
                "client_request_id must contain 1–256 bytes",
            ));
        }
        let mut r = self.inner.lock().await;
        self.load(&mut r).await?;
        if let Some(old) = r
            .jobs
            .values()
            .find(|j| j.client_request_id == request.client_request_id)
        {
            return if old.project_id == request.project_id && old.format == request.format {
                Ok(old.public())
            } else {
                Err(ServiceError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "Export request identifies different parameters",
                ))
            };
        }
        let executor = r
            .executor
            .filter(|_| r.executor_epoch == self.page_epoch.load(Ordering::SeqCst))
            .ok_or_else(|| {
                ServiceError::new(
                    "EXPORT_EXECUTOR_UNAVAILABLE",
                    "The desktop export executor is not ready",
                )
            })?;
        if r.jobs.values().any(|j| j.phase.active()) {
            return Err(ServiceError::new("EXPORT_BUSY", "Another export is active"));
        }
        if model_export::export_busy(app.state::<PreviewController>().inner()).await {
            return Err(ServiceError::new(
                "EXPORT_BUSY",
                "Another GUI or MCP export is active",
            ));
        }
        if app
            .state::<TaskService>()
            .all()
            .await?
            .iter()
            .any(|t| t.project_id == Some(request.project_id) && t.status.active())
        {
            return Err(ServiceError::new(
                "TASK_BUSY",
                "The target project is still running",
            ));
        }
        let (session, transform, revision) = model_export::begin_saved_mcp_export(
            app.state::<PreviewController>().inner(),
            request.project_id,
            request.format.internal(),
        )
        .await?;
        let job = Job {
            export_id: Uuid::new_v4(),
            project_id: request.project_id,
            format: request.format,
            phase: Phase::Starting,
            progress: None,
            confirmation_id: None,
            adjustment: None,
            result: None,
            error: None,
            client_request_id: request.client_request_id,
            executor_id: executor,
            session_id: session,
            transform,
            edit_revision: revision,
        };
        let id = job.export_id;
        let response = job.public();
        let work = job.work();
        r.jobs.insert(id, job);
        if let Err(error) = self.persist(&r).await {
            r.jobs.remove(&id);
            drop(r);
            let _ = model_export::cancel_gaussian_model_export(app.state(), session).await;
            return Err(error);
        }
        drop(r);
        if app.emit("mcp-model-export-work", work).is_err() {
            let _ = self
                .fail(
                    app,
                    id,
                    "EXPORT_EXECUTOR_UNAVAILABLE",
                    "Could not dispatch the desktop export",
                )
                .await;
            return self.status(id).await;
        }
        // A listener that vanished without acknowledging must not hold the export lock.
        let controller = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let r = controller.inner.lock().await;
            let unclaimed = r.jobs.get(&id).is_some_and(|j| j.phase == Phase::Starting);
            drop(r);
            if unclaimed {
                let _ = controller
                    .fail(
                        &app,
                        id,
                        "EXPORT_EXECUTOR_UNAVAILABLE",
                        "Desktop executor did not acknowledge",
                    )
                    .await;
            }
        });
        Ok(response)
    }
    async fn fail(
        &self,
        app: &tauri::AppHandle,
        id: Uuid,
        code: &str,
        message: &str,
    ) -> Result<()> {
        let mut r = self.inner.lock().await;
        let Some(job) = r.jobs.get_mut(&id).filter(|j| j.phase.active()) else {
            return Ok(());
        };
        job.phase = Phase::Failed;
        job.progress = None;
        job.confirmation_id = None;
        job.error =
            Some(json!({"code":code,"message":crate::diagnostics::redact::sanitize(message,&[])}));
        let session = job.session_id;
        let control = json!({"exportId":id,"executorId":job.executor_id,"action":"cancel"});
        let saved = self.persist(&r).await;
        drop(r);
        let _ = app.emit("mcp-model-export-control", control);
        let _ = model_export::cancel_gaussian_model_export(app.state(), session).await;
        saved
    }
    pub(crate) async fn invalidate(
        &self,
        app: &tauri::AppHandle,
        executor: Option<Uuid>,
    ) -> Result<()> {
        let mut r = self.inner.lock().await;
        self.load(&mut r).await?;
        if executor.is_some() && r.executor != executor {
            return Ok(());
        }
        r.executor = None;
        let sessions: Vec<_> = r
            .jobs
            .values_mut()
            .filter(|j| j.phase.active())
            .map(|j| {
                j.phase = Phase::Interrupted;
                j.confirmation_id = None;
                j.progress = None;
                j.session_id
            })
            .collect();
        let saved = self.persist(&r).await;
        drop(r);
        for session in sessions {
            let _ = model_export::cancel_gaussian_model_export(app.state(), session).await;
        }
        saved
    }
    async fn cancel(&self, app: &tauri::AppHandle, id: Uuid) -> Result<Value> {
        let mut r = self.inner.lock().await;
        self.load(&mut r).await?;
        let job = r
            .jobs
            .get_mut(&id)
            .ok_or_else(|| ServiceError::new("EXPORT_NOT_FOUND", "Export job not found"))?;
        if !job.phase.active() {
            return Ok(job.public());
        }
        if job.phase == Phase::Publishing {
            return Err(ServiceError::new(
                "EXPORT_PUBLISHING",
                "Publication is already in progress",
            ));
        }
        let session = job.session_id;
        let executor = job.executor_id;
        job.phase = Phase::Cancelling;
        if let Err(error) = self.persist(&r).await {
            drop(r);
            let _ = self
                .fail(
                    app,
                    id,
                    "EXPORT_STATE_FAILED",
                    "Could not persist export cancellation",
                )
                .await;
            return Err(error);
        }
        drop(r);
        let cancelled = model_export::cancel_gaussian_model_export(app.state(), session).await;
        let mut r = self.inner.lock().await;
        let job = r.jobs.get_mut(&id).unwrap();
        if job.phase == Phase::Completed {
            return Ok(job.public());
        }
        if let Err(error) = cancelled {
            job.phase = Phase::Publishing;
            self.persist(&r).await?;
            return Err(error.into());
        }
        job.phase = Phase::Cancelled;
        job.progress = None;
        job.confirmation_id = None;
        let result = job.public();
        self.persist(&r).await?;
        drop(r);
        let _ = app.emit(
            "mcp-model-export-control",
            json!({"exportId":id,"executorId":executor,"action":"cancel"}),
        );
        Ok(result)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    project_id: Uuid,
    format: ExportFormat,
    client_request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportRequest {
    export_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirm {
    export_id: Uuid,
    confirmation_id: u64,
    accepted: bool,
}

pub(super) async fn availability(app: Option<&tauri::AppHandle>) -> Result<Value> {
    let Some(app) = app else {
        return Ok(
            json!({"executor_ready":false,"busy":false,"formats":["ply","compressed_ply","spz","sog"],"active_export":null}),
        );
    };
    let controller = app.state::<ExportController>();
    let mut r = controller.inner.lock().await;
    controller.load(&mut r).await?;
    let busy = model_export::export_busy(app.state::<PreviewController>().inner()).await;
    Ok(
        json!({"executor_ready":r.executor.is_some() && r.executor_epoch==controller.page_epoch.load(Ordering::SeqCst),"busy":busy,"formats":["ply","compressed_ply","spz","sog"],"active_export":r.jobs.values().find(|j|j.phase.active()).map(Job::public)}),
    )
}
pub(super) async fn dispatch(
    app: Option<&tauri::AppHandle>,
    name: &str,
    args: Value,
) -> Result<Value> {
    let app = app.ok_or_else(|| {
        ServiceError::new(
            "EXPORT_EXECUTOR_UNAVAILABLE",
            "Desktop application required",
        )
    })?;
    let controller = app.state::<ExportController>();
    match name {
        "start_model_export" => controller.start(app, parse(args)?).await,
        "get_model_export_status" => {
            controller
                .status(parse::<ExportRequest>(args)?.export_id)
                .await
        }
        "cancel_model_export" => {
            controller
                .cancel(app, parse::<ExportRequest>(args)?.export_id)
                .await
        }
        "confirm_model_export" => {
            let request: Confirm = parse(args)?;
            let mut r = controller.inner.lock().await;
            controller.load(&mut r).await?;
            let job = r
                .jobs
                .get_mut(&request.export_id)
                .ok_or_else(|| ServiceError::new("EXPORT_NOT_FOUND", "Export job not found"))?;
            job.check_confirmation(request.confirmation_id)?;
            if !request.accepted {
                drop(r);
                return controller.cancel(app, request.export_id).await;
            }
            let executor = job.executor_id;
            job.phase = Phase::Preparing;
            job.confirmation_id = None;
            let response = job.public();
            if let Err(error) = controller.persist(&r).await {
                drop(r);
                let _ = controller
                    .fail(
                        app,
                        request.export_id,
                        "EXPORT_STATE_FAILED",
                        "Could not persist SPZ confirmation",
                    )
                    .await;
                return Err(error);
            }
            drop(r);
            if app.emit("mcp-model-export-control",json!({"exportId":request.export_id,"executorId":executor,"action":"confirm","confirmationId":request.confirmation_id})).is_err() {
                controller.fail(app,request.export_id,"EXPORT_EXECUTOR_UNAVAILABLE","Could not deliver confirmation").await?;
                return controller.status(request.export_id).await;
            }
            Ok(response)
        }
        _ => Err(ServiceError::new("UNKNOWN_TOOL", "Unknown export tool")),
    }
}

#[tauri::command]
pub async fn mcp_export_executor_ready(app: tauri::AppHandle, executor_id: Uuid) -> Result<()> {
    let controller = app.state::<ExportController>();
    controller.invalidate(&app, None).await?;
    let mut r = controller.inner.lock().await;
    r.executor = Some(executor_id);
    r.executor_epoch = controller.page_epoch.load(Ordering::SeqCst);
    Ok(())
}
#[tauri::command]
pub async fn mcp_export_executor_stopped(app: tauri::AppHandle, executor_id: Uuid) -> Result<()> {
    app.state::<ExportController>()
        .invalidate(&app, Some(executor_id))
        .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExecutorUpdate {
    export_id: Uuid,
    executor_id: Uuid,
    phase: Phase,
    progress: Option<f64>,
    confirmation_id: Option<u64>,
    adjustment: Option<Adjustment>,
    error: Option<String>,
    error_code: Option<String>,
}
#[tauri::command]
pub async fn update_mcp_model_export(app: tauri::AppHandle, update: ExecutorUpdate) -> Result<()> {
    let controller = app.state::<ExportController>();
    let mut r = controller.inner.lock().await;
    if r.executor != Some(update.executor_id) {
        return Err(ServiceError::new(
            "STALE_EXECUTOR",
            "Export executor replaced",
        ));
    }
    let job = r
        .jobs
        .get_mut(&update.export_id)
        .filter(|j| j.executor_id == update.executor_id)
        .ok_or_else(|| ServiceError::new("EXPORT_NOT_FOUND", "Export job not found"))?;
    if !job.phase.active() {
        return Ok(());
    }
    if update.phase == Phase::Failed {
        drop(r);
        return controller
            .fail(
                &app,
                update.export_id,
                update.error_code.as_deref().unwrap_or("EXPORT_FAILED"),
                update.error.as_deref().unwrap_or("Export failed"),
            )
            .await;
    }
    let changed = job.observe(update)?;
    if changed {
        controller.persist(&r).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job() -> Job {
        Job {
            export_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            format: ExportFormat::Spz,
            phase: Phase::Preparing,
            progress: None,
            confirmation_id: None,
            adjustment: None,
            result: None,
            error: None,
            client_request_id: "test".into(),
            executor_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            transform: GaussianTransform::default(),
            edit_revision: 7,
        }
    }
    fn update(job: &Job, phase: Phase) -> ExecutorUpdate {
        ExecutorUpdate {
            export_id: job.export_id,
            executor_id: job.executor_id,
            phase,
            progress: Some(25.),
            confirmation_id: None,
            adjustment: None,
            error: None,
            error_code: None,
        }
    }
    #[test]
    fn spz_consent_is_scoped_and_cannot_be_overwritten_by_late_progress() {
        let mut job = job();
        let mut request = update(&job, Phase::AwaitingConfirmation);
        request.confirmation_id = Some(9);
        request.adjustment = Some(Adjustment {
            center: [5000., 0., 0.],
            scale: 0.5,
        });
        assert!(job.observe(request).unwrap());
        assert!(job.check_confirmation(8).is_err());
        assert!(job.check_confirmation(9).is_ok());
        let late = update(&job, Phase::Encoding);
        assert!(!job.observe(late).unwrap());
        assert_eq!(job.phase, Phase::AwaitingConfirmation);
        assert!(job.progress.is_none());
        assert!(job.public()["confirmation"]["description"]["en"]
            .as_str()
            .unwrap()
            .contains("SPZ"));
        job.phase = Phase::Preparing;
        job.confirmation_id = None;
        assert!(job.check_confirmation(9).is_err());
        let encoding = update(&job, Phase::Encoding);
        assert!(job.observe(encoding).unwrap());
        let previous = update(&job, Phase::Preparing);
        assert!(!job.observe(previous).unwrap());
        assert_eq!(job.phase, Phase::Encoding);
        let complete = update(&job, Phase::Completed);
        assert_eq!(
            job.observe(complete).unwrap_err().code,
            "INVALID_EXPORT_UPDATE"
        );
        job.phase = Phase::Cancelled;
        let late = update(&job, Phase::Encoding);
        assert!(!job.observe(late).unwrap());
        assert_eq!(job.phase, Phase::Cancelled);
    }
    #[tokio::test]
    async fn restart_interrupts_active_jobs_and_never_reuses_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let controller = ExportController::at(dir.path().join("exports.json"));
        let mut pending = job();
        pending.phase = Phase::AwaitingConfirmation;
        pending.confirmation_id = Some(9);
        let id = pending.export_id;
        let mut completed = job();
        completed.phase = Phase::Completed;
        completed.result = Some(json!({"path":"project/exports/mcp-model.spz","splat_count":2}));
        let done = completed.export_id;
        let mut r = Registry::default();
        r.jobs.insert(id, pending);
        r.jobs.insert(done, completed);
        controller.persist(&r).await.unwrap();
        let restored = ExportController::at(controller.path.clone());
        let value = restored.status(id).await.unwrap();
        assert_eq!(value["phase"], "interrupted");
        assert!(value["confirmation"].is_null());
        assert!(value["progress"].is_null());
        assert_eq!(
            restored.status(done).await.unwrap()["result"]["splat_count"],
            2
        );
        assert_eq!(
            restored.status(Uuid::new_v4()).await.unwrap_err().code,
            "EXPORT_NOT_FOUND"
        );
        assert!(!restored.inner.lock().await.executor.is_some());
    }
    #[test]
    fn public_metadata_excludes_tokens_and_rejects_invalid_adjustments() {
        let job = job();
        let output = job.public().to_string();
        for secret in [
            job.session_id.to_string(),
            job.executor_id.to_string(),
            job.client_request_id.clone(),
        ] {
            assert!(!output.contains(&format!("\"{secret}\"")));
        }
        for scale in [0., -1., 2., f64::NAN, f64::INFINITY] {
            assert!(!Adjustment {
                center: [0.; 3],
                scale
            }
            .valid());
        }
        assert!(!Adjustment {
            center: [f64::NAN, 0., 0.],
            scale: 1.
        }
        .valid());
    }
}
