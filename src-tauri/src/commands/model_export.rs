//! Token-scoped exports. Binary IPC exposes only edited views and owned outputs.
use super::{preview_client_path, read_mask, PreviewController};
use crate::{
    error::{Result, SplatError},
    project::{catalog, manager::atomic_replace_file, GaussianTransform},
    reconstruction::{
        ply::{inspect_gaussian_ply, PlyInfo},
        splat_transform::{export_view::EditedPlyView, GaussianExportEdits},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{
    ipc::{InvokeBody, Request, Response},
    Emitter, State,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
const CHUNK: usize = 1_024_000;
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelFormat {
    Ply,
    CompressedPly,
    Spz,
    Sog,
}
impl ModelFormat {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Ply => "ply",
            Self::CompressedPly => "compressed.ply",
            Self::Spz => "spz",
            Self::Sog => "sog",
        }
    }
    fn filename(self) -> &'static str {
        match self {
            Self::Ply => "edit.ply",
            Self::CompressedPly => "edit.compressed.ply",
            Self::Spz => "edit.spz",
            Self::Sog => "edit.sog",
        }
    }
}
#[derive(Default)]
struct Encoding {
    view: Option<Arc<std::sync::Mutex<EditedPlyView>>>,
    info: Option<PlyInfo>,
    sh_bands: u8,
    format: Option<ModelFormat>,
    written: u64,
}
pub(super) struct ModelExportSession {
    pub(super) id: Uuid,
    pub(super) project_id: Uuid,
    source: PathBuf,
    root: PathBuf,
    output: PathBuf,
    format: ModelFormat,
    destination: Option<PathBuf>,
    publishing: AtomicBool,
    revision: u64,
    transform: GaussianTransform,
    cancel: CancellationToken,
    operation: Mutex<()>,
    encoding: Mutex<Encoding>,
    _guard: OwnedMutexGuard<()>,
}
impl Drop for ModelExportSession {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.output);
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelExportProgress {
    export_id: Uuid,
    project_id: Uuid,
    phase: &'static str,
    progress: f64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedModelExport {
    export_id: Uuid,
    file_size: u64,
    splat_count: u64,
    sh_bands: u8,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelValidation {
    splat_count: u64,
    sh_bands: u8,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelExportResult {
    path: PathBuf,
    file_size: u64,
    splat_count: u64,
    format: ModelFormat,
}
fn error(s: impl Into<String>) -> SplatError {
    SplatError::Process(s.into())
}
fn running(s: &ModelExportSession) -> Result<()> {
    if s.cancel.is_cancelled() {
        Err(SplatError::Cancelled)
    } else {
        Ok(())
    }
}
async fn session(state: &PreviewController, id: Uuid) -> Result<Arc<ModelExportSession>> {
    state
        .model_export
        .lock()
        .await
        .as_ref()
        .filter(|s| s.id == id)
        .cloned()
        .ok_or_else(|| error("模型导出会话不存在或已结束"))
}
async fn release(state: &PreviewController, id: Uuid) {
    let mut slot = state.model_export.lock().await;
    if slot.as_ref().is_some_and(|s| s.id == id) {
        slot.take();
    }
}
fn check_revision(expected: u64, actual: u64) -> Result<()> {
    if expected != actual {
        Err(error("编辑状态已变化，请重新保存编辑并导出"))
    } else {
        Ok(())
    }
}
fn check_current_export(
    s: &ModelExportSession,
    source: &std::path::Path,
    revision: u64,
) -> Result<()> {
    check_revision(s.revision, revision)?;
    if source != s.source {
        return Err(error("模型源文件已发生变化"));
    }
    Ok(())
}

pub(crate) async fn ensure_editable(state: &PreviewController, project: Uuid) -> Result<()> {
    if state
        .model_export
        .lock()
        .await
        .as_ref()
        .is_some_and(|s| s.project_id == project)
    {
        return Err(error("已有导出正在进行"));
    }
    Ok(())
}

pub(crate) async fn export_busy(state: &PreviewController) -> bool {
    let held = state.export.try_lock().is_err();
    held || state.video_export.lock().await.is_some() || state.html_export.lock().await.is_some()
}

fn check_saved_transform(s: &ModelExportSession, transform: &GaussianTransform) -> Result<()> {
    if s.destination.is_some()
        && serde_json::to_value(transform)? != serde_json::to_value(s.transform)?
    {
        return Err(error("编辑状态已变化，请重新保存编辑并导出"));
    }
    Ok(())
}

/// MCP may only publish new files below this real (non-redirected) directory.
fn mcp_directory(root: &std::path::Path, create: bool) -> Result<PathBuf> {
    let root = std::fs::canonicalize(root)?;
    let directory = root.join("exports");
    if create && !directory.exists() {
        std::fs::create_dir(&directory)?;
    }
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => {
            let redirected = metadata.file_type().is_symlink();
            #[cfg(windows)]
            let redirected = {
                use std::os::windows::fs::MetadataExt;
                redirected || metadata.file_attributes() & 0x400 != 0
            };
            if redirected || !metadata.is_dir() || std::fs::canonicalize(&directory)? != directory {
                return Err(error("Export directory must not be redirected"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !create => {}
        Err(e) => return Err(e.into()),
    }
    Ok(directory)
}
#[cfg(test)]
fn bands(path: &std::path::Path) -> Result<u8> {
    use std::io::{BufRead, BufReader};
    let mut count = 0;
    let mut header_bytes = 0;
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        header_bytes += read;
        if read == 0 || header_bytes > 256 * 1024 {
            return Err(error("无效的 Gaussian PLY 文件头"));
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.first() == Some(&"property")
            && fields.get(2).is_some_and(|p| p.starts_with("f_rest_"))
        {
            count += 1;
        }
        if line.trim() == "end_header" {
            break;
        }
    }
    match count {
        0 => Ok(0),
        9 => Ok(1),
        24 => Ok(2),
        45 => Ok(3),
        _ => Err(error("暂不支持此模型的 SH 阶数")),
    }
}
#[tauri::command]
pub async fn begin_gaussian_model_export(
    state: State<'_, PreviewController>,
    project_id: Uuid,
    transform: GaussianTransform,
    edit_revision: u64,
    format: ModelFormat,
) -> Result<Uuid> {
    reserve(&state, project_id, transform, edit_revision, format, false).await
}

pub(crate) async fn begin_saved_mcp_export(
    state: &PreviewController,
    project_id: Uuid,
    format: ModelFormat,
) -> Result<(Uuid, GaussianTransform, u64)> {
    if state
        .edit_save
        .lock()
        .await
        .as_ref()
        .is_some_and(|s| s.project_id == project_id)
    {
        return Err(error("编辑状态已变化，请重新保存编辑并导出"));
    }
    let (_, _, metadata) = catalog::registered_final_ply_for_project(project_id).await?;
    let id = reserve(
        state,
        project_id,
        metadata.transform,
        metadata.editing.revision,
        format,
        true,
    )
    .await?;
    Ok((id, metadata.transform, metadata.editing.revision))
}
async fn reserve(
    state: &PreviewController,
    project_id: Uuid,
    transform: GaussianTransform,
    edit_revision: u64,
    format: ModelFormat,
    mcp: bool,
) -> Result<Uuid> {
    let _lifecycle = state.lifecycle.lock().await;
    if mcp
        && state
            .edit_save
            .lock()
            .await
            .as_ref()
            .is_some_and(|s| s.project_id == project_id)
    {
        return Err(error("编辑状态已变化，请重新保存编辑并导出"));
    }
    let guard = state
        .export
        .clone()
        .try_lock_owned()
        .map_err(|_| error("已有导出正在进行"))?;
    if state.video_export.lock().await.is_some() || state.html_export.lock().await.is_some() {
        return Err(error("已有导出正在进行"));
    }
    let (root, source, metadata) = catalog::registered_final_ply_for_project(project_id).await?;
    check_revision(edit_revision, metadata.editing.revision)?;
    if mcp && serde_json::to_value(metadata.transform)? != serde_json::to_value(transform)? {
        return Err(error("编辑状态已变化，请重新保存编辑并导出"));
    }
    let id = Uuid::new_v4();
    let destination = if mcp {
        Some(mcp_directory(&root, false)?.join(format!("mcp-{id}.{}", format.extension())))
    } else {
        None
    };
    let s = Arc::new(ModelExportSession {
        id,
        project_id,
        source,
        transform: transform.validate()?,
        revision: edit_revision,
        output: root.join(format!(".ooosplat-model-{id}.encoded.tmp")),
        format,
        destination,
        publishing: AtomicBool::new(false),
        root,
        cancel: CancellationToken::new(),
        operation: Mutex::new(()),
        encoding: Mutex::new(Encoding::default()),
        _guard: guard,
    });
    let mut slot = state.model_export.lock().await;
    if slot.is_some() {
        return Err(error("已有导出正在进行"));
    }
    *slot = Some(s);
    Ok(id)
}
#[tauri::command]
pub async fn prepare_gaussian_model_export(
    app: tauri::AppHandle,
    state: State<'_, PreviewController>,
    export_id: Uuid,
) -> Result<PreparedModelExport> {
    let s = session(&state, export_id).await?;
    let _op = s.operation.lock().await;
    running(&s)?;
    if s.encoding.lock().await.info.is_some() {
        return Err(error("模型导出已经准备完成"));
    }
    let _metadata = state.metadata_write.lock().await;
    let (_, source, metadata) = catalog::registered_final_ply_for_project(s.project_id).await?;
    check_revision(s.revision, metadata.editing.revision)?;
    check_saved_transform(&s, &metadata.transform)?;
    if source != s.source {
        return Err(error("模型源文件已发生变化"));
    }
    let job = s.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<_> {
        let mut last_progress = std::time::Instant::now();
        let info = inspect_gaussian_ply(&job.source)?;
        let editing = metadata.editing.validate(info.splat_count)?;
        let mask = read_mask(&job.root, editing)?;
        let view = EditedPlyView::open(
            &job.source,
            job.transform,
            GaussianExportEdits {
                crop: editing.crop,
                deleted_mask: Some(&mask),
            },
            |done, total| {
                if done != total && last_progress.elapsed() < std::time::Duration::from_millis(50) {
                    return;
                }
                last_progress = std::time::Instant::now();
                let _ = app.emit(
                    "gaussian-model-export-progress",
                    ModelExportProgress {
                        export_id: job.id,
                        project_id: job.project_id,
                        phase: "preparing",
                        progress: done as f64 / total.max(1) as f64 * 100.,
                    },
                );
            },
            || running(&job),
        )?;
        Ok(view)
    })
    .await
    .map_err(|e| error(e.to_string()))?;
    match result {
        Ok(view) => {
            running(&s)?;
            let info = view.info();
            let sh = view.sh_bands();
            let prepared = PreparedModelExport {
                export_id,
                file_size: info.file_size,
                splat_count: info.splat_count,
                sh_bands: sh,
            };
            let mut e = s.encoding.lock().await;
            e.info = Some(info);
            e.sh_bands = sh;
            e.view = Some(Arc::new(std::sync::Mutex::new(view)));
            Ok(prepared)
        }
        Err(e) => {
            release(&state, export_id).await;
            Err(e)
        }
    }
}
#[tauri::command]
pub async fn start_gaussian_model_encoding(
    app: tauri::AppHandle,
    state: State<'_, PreviewController>,
    export_id: Uuid,
    format: ModelFormat,
) -> Result<String> {
    let s = session(&state, export_id).await?;
    let _op = s.operation.lock().await;
    running(&s)?;
    if format != s.format {
        return Err(error("导出格式与预约不一致"));
    }
    // Consent may have taken arbitrarily long. Recheck before creating output.
    let _metadata = state.metadata_write.lock().await;
    let (_, source, metadata) = catalog::registered_final_ply_for_project(s.project_id).await?;
    check_current_export(&s, &source, metadata.editing.revision)?;
    check_saved_transform(&s, &metadata.transform)?;
    let mut e = s.encoding.lock().await;
    if e.format.is_some() {
        return Err(error("模型导出已开始"));
    }
    let info = e
        .info
        .as_ref()
        .ok_or_else(|| error("模型导出尚未准备完成"))?;
    let view = e
        .view
        .as_ref()
        .ok_or_else(|| error("导出视图未就绪"))?
        .clone();
    view.lock()
        .map_err(|_| error("导出视图不可用"))?
        .check_source()?;
    if format != ModelFormat::Ply {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&s.output)?;
    }
    let size = info.file_size;
    e.format = Some(format);
    if format == ModelFormat::Ply {
        let job = s.clone();
        drop(e);
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut last_progress = std::time::Instant::now();
            view.lock().map_err(|_| error("导出视图不可用"))?.write_to(
                &job.output,
                |done, total| {
                    if done != total
                        && last_progress.elapsed() < std::time::Duration::from_millis(50)
                    {
                        return;
                    }
                    last_progress = std::time::Instant::now();
                    let _ = app.emit(
                        "gaussian-model-export-progress",
                        ModelExportProgress {
                            export_id: job.id,
                            project_id: job.project_id,
                            phase: "encoding",
                            progress: done as f64 / total.max(1) as f64 * 100.,
                        },
                    );
                },
                || running(&job),
            )
        })
        .await
        .map_err(|err| error(err.to_string()))??;
        s.encoding.lock().await.written = size;
    }
    Ok(format.filename().into())
}
fn header_id(request: &Request<'_>) -> Result<Uuid> {
    request
        .headers()
        .get("x-ooosplat-export-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| error("无效的模型导出令牌"))
}
fn offset(request: &Request<'_>) -> Result<u64> {
    request
        .headers()
        .get("x-ooosplat-offset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| error("无效的模型导出偏移"))
}
#[tauri::command]
pub async fn read_gaussian_model_export_chunk(
    state: State<'_, PreviewController>,
    request: Request<'_>,
) -> Result<Response> {
    let s = session(&state, header_id(&request)?).await?;
    let output = match request
        .headers()
        .get("x-ooosplat-export-file")
        .and_then(|v| v.to_str().ok())
    {
        Some("source") => false,
        Some("output") => true,
        _ => return Err(error("无效的导出文件类型")),
    };
    let position = offset(&request)?;
    let length = request
        .headers()
        .get("x-ooosplat-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= CHUNK)
        .ok_or_else(|| error("导出分块长度超出限制"))?;
    Ok(Response::new(
        read_chunk(&s, output, position, length).await?,
    ))
}
async fn read_chunk(
    s: &ModelExportSession,
    output: bool,
    position: u64,
    length: usize,
) -> Result<Vec<u8>> {
    let _op = s.operation.lock().await;
    running(s)?;
    let e = s.encoding.lock().await;
    let info = e
        .info
        .as_ref()
        .ok_or_else(|| error("模型导出尚未准备完成"))?;
    if length == 0 || length > CHUNK {
        return Err(error("导出分块长度超出限制"));
    }
    let size = if output { e.written } else { info.file_size };
    if position > size {
        return Err(error("导出读取越界"));
    }
    let length = length.min((size - position) as usize);
    let path = s.output.clone();
    let view = e.view.clone();
    let cancel = s.cancel.clone();
    let data = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        if !output {
            return view
                .ok_or_else(|| error("导出视图未就绪"))?
                .lock()
                .map_err(|_| error("导出视图不可用"))?
                .read(position, length, || {
                    if cancel.is_cancelled() {
                        Err(SplatError::Cancelled)
                    } else {
                        Ok(())
                    }
                });
        }
        let mut f = File::open(path)?;
        f.seek(SeekFrom::Start(position))?;
        let mut bytes = vec![0; length];
        f.read_exact(&mut bytes)?;
        Ok(bytes)
    })
    .await
    .map_err(|e| error(e.to_string()))??;
    Ok(data)
}
#[tauri::command]
pub async fn write_gaussian_model_export_chunk(
    state: State<'_, PreviewController>,
    request: Request<'_>,
) -> Result<u64> {
    let s = session(&state, header_id(&request)?).await?;
    let bytes = match request.body() {
        InvokeBody::Raw(b) if !b.is_empty() && b.len() <= CHUNK => b.clone(),
        _ => return Err(error("导出必须使用不超过 1,024,000 字节的二进制分块")),
    };
    write_chunk(&s, offset(&request)?, bytes).await
}
async fn write_chunk(s: &ModelExportSession, position: u64, bytes: Vec<u8>) -> Result<u64> {
    let _op = s.operation.lock().await;
    running(s)?;
    let mut e = s.encoding.lock().await;
    if !matches!(
        e.format,
        Some(ModelFormat::CompressedPly | ModelFormat::Spz | ModelFormat::Sog)
    ) || e.info.is_none()
    {
        return Err(error("导出格式未就绪"));
    }
    if bytes.is_empty() || bytes.len() > CHUNK {
        return Err(error("导出分块长度超出限制"));
    }
    if position != e.written {
        return Err(error("模型导出分块顺序无效"));
    }
    let maximum = e
        .info
        .as_ref()
        .unwrap()
        .file_size
        .saturating_mul(4)
        .saturating_add(64 * MIB);
    if e.written.saturating_add(bytes.len() as u64) > maximum {
        return Err(error("压缩输出超过安全大小限制"));
    }
    let length = bytes.len() as u64;
    let path = s.output.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut f = std::fs::OpenOptions::new().append(true).open(path)?;
        f.write_all(&bytes)?;
        Ok(())
    })
    .await
    .map_err(|e| error(e.to_string()))??;
    e.written += length;
    Ok(e.written)
}
fn validate(info: &PlyInfo, sh: u8, validation: &ModelValidation) -> Result<()> {
    if validation.splat_count != info.splat_count || validation.sh_bands != sh {
        return Err(error("导出回读校验失败：Gaussian 数量或 SH 阶数发生变化"));
    }
    Ok(())
}
#[tauri::command]
pub async fn complete_gaussian_model_export(
    app: tauri::AppHandle,
    state: State<'_, PreviewController>,
    export_id: Uuid,
    validation: Option<ModelValidation>,
) -> Result<ModelExportResult> {
    let s = session(&state, export_id).await?;
    let _op = s.operation.lock().await;
    running(&s)?;
    let _metadata = state.metadata_write.lock().await;
    let (_, source, metadata) = catalog::registered_final_ply_for_project(s.project_id).await?;
    check_current_export(&s, &source, metadata.editing.revision)?;
    check_saved_transform(&s, &metadata.transform)?;
    s.publishing.store(true, Ordering::SeqCst);
    let result = match publish(&s, validation).await {
        Ok(result) => result,
        Err(error) => {
            s.publishing.store(false, Ordering::SeqCst);
            return Err(error);
        }
    };
    if let Some(destination) = &s.destination {
        use tauri::Manager;
        if let Err(e) = app
            .state::<crate::mcp::exports::ExportController>()
            .published(export_id, &result)
            .await
        {
            // This UUID path was just created without replacement by this session.
            let _ = std::fs::remove_file(destination);
            s.publishing.store(false, Ordering::SeqCst);
            return Err(error(e.to_string()));
        }
    }
    release(&state, export_id).await;
    Ok(result)
}
// Caller holds the operation lock and has revalidated the registered source/revision.
async fn publish(
    s: &ModelExportSession,
    validation: Option<ModelValidation>,
) -> Result<ModelExportResult> {
    running(s)?;
    let e = s.encoding.lock().await;
    let info = e
        .info
        .as_ref()
        .ok_or_else(|| error("模型导出尚未准备完成"))?;
    let format = e.format.ok_or_else(|| error("导出格式未就绪"))?;
    if let Some(view) = &e.view {
        view.lock()
            .map_err(|_| error("导出视图不可用"))?
            .check_source()?;
    }
    if format != ModelFormat::Ply {
        validate(
            info,
            e.sh_bands,
            &validation.ok_or_else(|| error("缺少模型回读校验结果"))?,
        )?;
    } else if inspect_gaussian_ply(&s.output)? != *info {
        return Err(error("PLY 输出校验失败"));
    }
    if e.written == 0 || std::fs::metadata(&s.output)?.len() != e.written {
        return Err(error("模型输出不完整"));
    }
    let temporary = s.output.clone();
    let destination = s
        .destination
        .clone()
        .unwrap_or_else(|| s.root.join(format.filename()));
    let size = std::fs::metadata(&temporary)?.len();
    let count = info.splat_count;
    running(s)?;
    if s.destination.is_some() {
        mcp_directory(&s.root, true)?;
        // Same-filesystem atomic create, unlike rename this never replaces a rival.
        tokio::fs::hard_link(&temporary, &destination).await?;
    } else {
        atomic_replace_file(&temporary, &destination).await?;
    }
    Ok(ModelExportResult {
        path: preview_client_path(&destination),
        file_size: size,
        splat_count: count,
        format,
    })
}
#[tauri::command]
pub async fn cancel_gaussian_model_export(
    state: State<'_, PreviewController>,
    export_id: Uuid,
) -> Result<()> {
    let s = match session(&state, export_id).await {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    if s.publishing.load(Ordering::SeqCst) {
        return Err(error("模型正在发布，请等待完成"));
    }
    s.cancel.cancel();
    let _op = s.operation.lock().await;
    release(&state, export_id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::splat_transform::export_transformed_ply_to;
    use std::path::Path;
    fn gaussian_fixture(path: &Path, count: usize) {
        let mut fields = vec!["x".to_string(), "y".to_string(), "z".to_string()];
        fields.extend((0..3).map(|i| format!("f_dc_{i}")));
        fields.extend((0..45).map(|i| format!("f_rest_{i}")));
        fields.push("opacity".into());
        fields.extend((0..3).map(|i| format!("scale_{i}")));
        fields.extend((0..4).map(|i| format!("rot_{i}")));
        let mut writer =
            std::io::BufWriter::with_capacity(MIB as usize, File::create(path).unwrap());
        write!(
            writer,
            "ply\nformat binary_little_endian 1.0\nelement vertex {count}\n"
        )
        .unwrap();
        for field in &fields {
            writeln!(writer, "property float {field}").unwrap();
        }
        writeln!(writer, "end_header").unwrap();
        let mut row = vec![0; fields.len() * 4];
        for i in 6..51 {
            row[i * 4..(i + 1) * 4].copy_from_slice(&(i as f32 * 0.01).to_le_bytes());
        }
        row[51 * 4..52 * 4].copy_from_slice(&1.0_f32.to_le_bytes());
        row[55 * 4..56 * 4].copy_from_slice(&1.0_f32.to_le_bytes());
        for i in 0..count {
            row[..4].copy_from_slice(&((i % 8) as f32).to_le_bytes());
            writer.write_all(&row).unwrap();
        }
        writer.flush().unwrap();
    }
    #[test]
    fn virtual_view_matches_export_for_sequential_random_and_cross_row_reads() {
        use crate::project::GaussianCrop;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("final.ply");
        gaussian_fixture(&source, 8);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"opaque trailing payload")
            .unwrap();
        let original = std::fs::read(&source).unwrap();
        for transform in [
            GaussianTransform::default(),
            GaussianTransform {
                position: [1.0, 2.0, 3.0],
                rotation: [35.0, 70.0, -15.0],
                scale: 2.0,
            },
        ] {
            for crop in [
                None,
                Some(GaussianCrop::Box {
                    center: [0.0; 3],
                    size: [100.0; 3],
                    inverted: false,
                }),
                Some(GaussianCrop::Sphere {
                    center: [0.0; 3],
                    radius: 1.0,
                    inverted: true,
                }),
            ] {
                for deleted_mask in [None, Some(&[0b0001_0010][..])] {
                    let edits = GaussianExportEdits { crop, deleted_mask };
                    let path = dir.path().join("reference.ply");
                    export_transformed_ply_to(
                        &source,
                        &path,
                        transform,
                        edits,
                        |_, _| {},
                        || Ok(()),
                    )
                    .unwrap();
                    let reference = std::fs::read(path).unwrap();
                    let mut view =
                        EditedPlyView::open(&source, transform, edits, |_, _| {}, || Ok(()))
                            .unwrap();
                    assert_eq!(view.info().file_size, reference.len() as u64);
                    assert_eq!(view.sh_bands(), 3);
                    for size in [1, 7, 239, 1_024_000] {
                        let mut collected = Vec::new();
                        while collected.len() < reference.len() {
                            collected.extend(
                                view.read(collected.len() as u64, size, || Ok(())).unwrap(),
                            );
                        }
                        assert_eq!(collected, reference);
                    }
                    for offset in [reference.len() - 3, 0, 1490, 100, reference.len() / 2, 0] {
                        let bytes = view.read(offset as u64, 57, || Ok(())).unwrap();
                        assert_eq!(bytes, reference[offset..reference.len().min(offset + 57)]);
                    }
                    let output = dir.path().join("direct.ply.tmp");
                    view.write_to(&output, |_, _| {}, || Ok(())).unwrap();
                    assert_eq!(std::fs::read(&output).unwrap(), reference);
                    std::fs::remove_file(output).unwrap();
                }
            }
        }
        assert_eq!(std::fs::read(source).unwrap(), original);
    }
    #[test]
    fn virtual_view_fast_path_cancel_zero_and_source_changes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("final.ply");
        gaussian_fixture(&source, 8);
        let mut scans = 0;
        let mut view = EditedPlyView::open(
            &source,
            GaussianTransform::default(),
            GaussianExportEdits::default(),
            |_, _| scans += 1,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(scans, 0);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(view.read(0, 10, || Err(SplatError::Cancelled)).is_err());
        assert!(EditedPlyView::open(
            &source,
            GaussianTransform::default(),
            GaussianExportEdits {
                crop: None,
                deleted_mask: Some(&[255])
            },
            |_, _| {},
            || Ok(())
        )
        .is_err());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"changed")
            .unwrap();
        assert!(view.read(0, 10, || Ok(())).is_err());
        assert!(view.check_source().is_err());
    }
    #[tokio::test]
    #[ignore = "Manual browser + Rust edited-view smoke; set OOOSPLAT_EXPORT_SMOKE_INPUT to a small real PLY"]
    async fn virtual_view_browser_smoke() {
        let input = PathBuf::from(
            std::env::var("OOOSPLAT_EXPORT_SMOKE_INPUT").expect("Set OOOSPLAT_EXPORT_SMOKE_INPUT"),
        );
        run_virtual_view_browser_smoke(input).await;
    }

    #[tokio::test]
    #[ignore = "Real browser encoding above the former 512 MiB estimate; uses an isolated 400K SH3 fixture"]
    async fn large_virtual_view_browser_smoke() {
        let cache = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".cache");
        let dir = tempfile::Builder::new()
            .prefix("unrestricted-model-export-")
            .tempdir_in(cache)
            .unwrap();
        let input = dir.path().join("source.ply");
        gaussian_fixture(&input, 400_000);
        run_virtual_view_browser_smoke(input).await;
    }

    async fn run_virtual_view_browser_smoke(input: PathBuf) {
        use crate::project::GaussianCrop;
        use axum::{
            extract::{Query, State as HttpState},
            routing::get,
            Router,
        };
        let count = inspect_gaussian_ply(&input).unwrap().splat_count;
        let mut mask = vec![0; count.div_ceil(8) as usize];
        for index in (0..count).step_by(7) {
            mask[index as usize / 8] |= 1 << (index % 8);
        }
        let view = EditedPlyView::open(
            &input,
            GaussianTransform {
                position: [0.25, -0.3, 0.1],
                rotation: [12.0, 25.0, -15.0],
                scale: 0.75,
            },
            GaussianExportEdits {
                crop: Some(GaussianCrop::Sphere {
                    center: [0.0; 3],
                    radius: 1000.0,
                    inverted: false,
                }),
                deleted_mask: Some(&mask),
            },
            |_, _| {},
            || Ok(()),
        )
        .unwrap();
        let info = view.info();
        let sh = view.sh_bands();
        let view = Arc::new(std::sync::Mutex::new(view));
        #[derive(Deserialize)]
        struct Range {
            offset: u64,
            length: usize,
        }
        async fn read(
            HttpState(view): HttpState<Arc<std::sync::Mutex<EditedPlyView>>>,
            Query(range): Query<Range>,
        ) -> std::result::Result<Vec<u8>, (axum::http::StatusCode, String)> {
            if range.length == 0 || range.length > CHUNK {
                return Err((axum::http::StatusCode::BAD_REQUEST, "Invalid chunk".into()));
            }
            tokio::task::spawn_blocking(move || {
                view.lock()
                    .unwrap()
                    .read(range.offset, range.length, || Ok(()))
            })
            .await
            .unwrap()
            .map_err(|error| (axum::http::StatusCode::BAD_REQUEST, error.to_string()))
        }
        let payload = serde_json::json!({"fileSize":info.file_size,"splatCount":info.splat_count,"shBands":sh});
        let router = Router::new()
            .route("/info", get(move || async move { axum::Json(payload) }))
            .route("/read", get(read))
            .with_state(view.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut command = tokio::process::Command::new("node");
        command
            .arg("scripts/smoke-model-export.mjs")
            .arg(&input)
            .current_dir(root)
            .env("MODEL_VIEW_ENDPOINT", format!("http://127.0.0.1:{port}"))
            .env_remove("SMOKE_POINTS");
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let result = tokio::time::timeout(std::time::Duration::from_secs(660), command.output())
            .await
            .unwrap()
            .unwrap();
        server.abort();
        eprintln!(
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.status.success());
        // Test-only reference for independent numeric/render comparisons.
        let path = root
            .join(".cache")
            .join(format!("edited-view-reference-{}.ply", Uuid::new_v4()));
        view.lock()
            .unwrap()
            .write_to(&path, |_, _| {}, || Ok(()))
            .unwrap();
        eprintln!("Test-only edited reference: {}", path.display());
    }
    #[test]
    fn edited_export_keeps_sh_and_applies_filter_and_transform_only_once() {
        use crate::project::GaussianCrop;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("final.ply");
        gaussian_fixture(&source, 3);
        let original = std::fs::read(&source).unwrap();
        let output = dir.path().join("edit.ply.tmp");
        // Source x=0,1,2 maps into engine -x; delete 0, crop out 2.
        let result = export_transformed_ply_to(
            &source,
            &output,
            GaussianTransform {
                position: [10.0, 0.0, 0.0],
                scale: 2.0,
                ..GaussianTransform::default()
            },
            GaussianExportEdits {
                crop: Some(GaussianCrop::Box {
                    center: [8.0, 0.0, 0.0],
                    size: [2.0, 2.0, 2.0],
                    inverted: false,
                }),
                deleted_mask: Some(&[1]),
            },
            |_, _| {},
            || Ok(()),
        )
        .unwrap();
        assert_eq!(result.splat_count, 1);
        assert_eq!(bands(&output).unwrap(), 3);
        let bytes = std::fs::read(&output).unwrap();
        let header_end = bytes
            .windows(11)
            .position(|v| v == b"end_header\n")
            .unwrap()
            + 11;
        let x = f32::from_le_bytes(bytes[header_end..header_end + 4].try_into().unwrap());
        assert!((x + 8.0).abs() < 1e-6);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        // An empty result must not replace an existing export.
        assert!(export_transformed_ply_to(
            &source,
            &output,
            GaussianTransform::default(),
            GaussianExportEdits {
                crop: None,
                deleted_mask: Some(&[7])
            },
            |_, _| {},
            || Ok(())
        )
        .is_err());
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
    }
    #[test]
    #[ignore = "Manual 1.099 GiB/5M Gaussian export smoke; needs 2.4 GB temporary disk space"]
    fn large_standard_ply_export_smoke() {
        let cache = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".cache");
        let dir = tempfile::Builder::new()
            .prefix("large-model-export-")
            .tempdir_in(cache)
            .unwrap();
        let source = dir.path().join("final.ply");
        let output = dir.path().join("edit.ply.atomic.tmp");
        gaussian_fixture(&source, 5_000_000);
        let started = std::time::Instant::now();
        let mut view = EditedPlyView::open(
            &source,
            GaussianTransform::default(),
            GaussianExportEdits::default(),
            |_, _| {},
            || Ok(()),
        )
        .unwrap();
        let info = view.info();
        view.write_to(&output, |_, _| {}, || Ok(())).unwrap();
        assert_eq!(info.splat_count, 5_000_000);
        assert!(info.file_size > 1024 * MIB);
        assert_eq!(info.file_size, std::fs::metadata(&source).unwrap().len());
        assert_eq!(bands(&output).unwrap(), 3);
        eprintln!("Large PLY: {} bytes, {} points, {:.2}s; no estimated-memory gate; temporary fixture removed on exit",info.file_size,info.splat_count,started.elapsed().as_secs_f64());
    }
    async fn fixture(root: &Path, format: ModelFormat) -> ModelExportSession {
        let id = Uuid::new_v4();
        let output = root.join(format!("{id}.encoded.tmp"));
        let source = root.join("final.ply");
        gaussian_fixture(&source, 2);
        let mut view = EditedPlyView::open(
            &source,
            GaussianTransform::default(),
            GaussianExportEdits::default(),
            |_, _| {},
            || Ok(()),
        )
        .unwrap();
        let info = view.info();
        if format == ModelFormat::Ply {
            view.write_to(&output, |_, _| {}, || Ok(())).unwrap();
        } else {
            std::fs::write(&output, b"").unwrap();
        }
        let written = if format == ModelFormat::Ply {
            info.file_size
        } else {
            0
        };
        ModelExportSession {
            destination: None,
            publishing: AtomicBool::new(false),
            id,
            project_id: Uuid::new_v4(),
            source,
            root: root.into(),
            output,
            format,
            revision: 3,
            transform: GaussianTransform::default(),
            cancel: CancellationToken::new(),
            operation: Mutex::new(()),
            encoding: Mutex::new(Encoding {
                view: Some(Arc::new(std::sync::Mutex::new(view))),
                info: Some(info),
                sh_bands: 3,
                format: Some(format),
                written,
            }),
            _guard: Arc::new(Mutex::new(())).lock_owned().await,
        }
    }
    #[tokio::test]
    async fn binary_chunks_are_bounded_ordered_and_cancellable() {
        let dir = tempfile::tempdir().unwrap();
        let s = fixture(dir.path(), ModelFormat::Spz).await;
        assert_eq!(read_chunk(&s, false, 0, 3).await.unwrap(), b"ply");
        let size = std::fs::metadata(&s.source).unwrap().len();
        assert!(read_chunk(&s, false, size + 1, 1).await.is_err());
        assert!(read_chunk(&s, false, 0, 0).await.is_err());
        assert!(read_chunk(&s, false, 0, CHUNK + 1).await.is_err());
        assert!(write_chunk(&s, 0, vec![0; CHUNK + 1]).await.is_err());
        assert!(write_chunk(&s, 0, vec![]).await.is_err());
        assert!(write_chunk(&s, 1, vec![1]).await.is_err());
        assert_eq!(write_chunk(&s, 0, vec![1, 2]).await.unwrap(), 2);
        assert!(write_chunk(&s, 0, vec![3]).await.is_err());
        assert_eq!(write_chunk(&s, 2, vec![3]).await.unwrap(), 3);
        assert_eq!(read_chunk(&s, true, 0, 8).await.unwrap(), [1, 2, 3]);
        s.cancel.cancel();
        assert!(read_chunk(&s, false, 0, 1).await.is_err());
        assert!(write_chunk(&s, 3, vec![4]).await.is_err());
        let output = s.output.clone();
        drop(s);
        assert!(!output.exists());
    }
    #[tokio::test]
    async fn mcp_publish_is_unique_no_clobber_and_keeps_source_and_gui_exports() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = fixture(dir.path(), ModelFormat::Ply).await;
        let original = std::fs::read(&s.source).unwrap();
        let destination = mcp_directory(dir.path(), false)
            .unwrap()
            .join(format!("mcp-{}.ply", s.id));
        s.destination = Some(destination.clone());
        assert!(!destination.parent().unwrap().exists());
        std::fs::write(dir.path().join("edit.ply"), b"existing GUI export").unwrap();
        publish(&s, None).await.unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), original);
        assert!(publish(&s, None).await.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), original);
        let temporary = s.output.clone();
        drop(s);
        assert!(!temporary.exists());
        assert!(destination.exists());
        assert_eq!(
            std::fs::read(dir.path().join("final.ply")).unwrap(),
            original
        );
        assert_eq!(
            std::fs::read(dir.path().join("edit.ply")).unwrap(),
            b"existing GUI export"
        );
    }
    #[tokio::test]
    async fn mcp_rejects_redirected_exports_and_holds_target_edit_lock() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), dir.path().join("exports"))
            .expect("temporary symlink permission");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), dir.path().join("exports")).unwrap();
        assert!(mcp_directory(dir.path(), true).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        let state = PreviewController::default();
        let s = Arc::new(fixture(dir.path(), ModelFormat::Spz).await);
        let project = s.project_id;
        *state.model_export.lock().await = Some(s.clone());
        assert!(ensure_editable(&state, project).await.is_err());
        assert!(ensure_editable(&state, Uuid::new_v4()).await.is_ok());
        release(&state, s.id).await;
        assert!(ensure_editable(&state, project).await.is_ok());
    }
    #[tokio::test]
    async fn prepared_view_reads_before_encoding_without_an_output_and_rechecks_source_and_revision(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let s = fixture(dir.path(), ModelFormat::Spz).await;
        std::fs::remove_file(&s.output).unwrap();
        s.encoding.lock().await.format = None;
        std::fs::write(dir.path().join("edit.spz"), b"existing export").unwrap();
        assert_eq!(read_chunk(&s, false, 0, 3).await.unwrap(), b"ply");
        assert!(!s.output.exists());
        assert!(write_chunk(&s, 0, vec![1]).await.is_err());
        assert!(publish(
            &s,
            Some(ModelValidation {
                splat_count: 2,
                sh_bands: 3
            })
        )
        .await
        .is_err());
        assert!(check_current_export(&s, &s.source, s.revision).is_ok());
        assert!(check_current_export(&s, &s.source, s.revision + 1).is_err());
        assert!(check_current_export(&s, &dir.path().join("replacement.ply"), s.revision).is_err());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&s.source)
            .unwrap()
            .write_all(b"changed")
            .unwrap();
        assert!(read_chunk(&s, false, 0, 3).await.is_err());
        assert!(!s.output.exists());
        assert_eq!(
            std::fs::read(dir.path().join("edit.spz")).unwrap(),
            b"existing export"
        );
    }

    #[tokio::test]
    async fn invalid_tokens_and_old_cleanup_do_not_touch_current_session() {
        let dir = tempfile::tempdir().unwrap();
        let state = PreviewController::default();
        let s = Arc::new(fixture(dir.path(), ModelFormat::Spz).await);
        *state.model_export.lock().await = Some(s.clone());
        assert!(session(&state, Uuid::new_v4()).await.is_err());
        release(&state, Uuid::new_v4()).await;
        assert!(session(&state, s.id).await.is_ok());
        release(&state, s.id).await;
        assert!(session(&state, s.id).await.is_err());
    }
    #[tokio::test]
    async fn failed_validation_cancel_and_io_error_preserve_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let s = fixture(dir.path(), ModelFormat::Spz).await;
        let original = std::fs::read(&s.source).unwrap();
        let destination = dir.path().join("edit.spz");
        std::fs::write(&destination, b"old export").unwrap();
        write_chunk(&s, 0, vec![1, 2, 3]).await.unwrap();
        assert!(publish(&s, None).await.is_err());
        assert!(publish(
            &s,
            Some(ModelValidation {
                splat_count: 1,
                sh_bands: 3
            })
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"old export");
        // Simulate a partial write/disk error before publication.
        std::fs::write(&s.output, b"truncated").unwrap();
        assert!(publish(
            &s,
            Some(ModelValidation {
                splat_count: 2,
                sh_bands: 3
            })
        )
        .await
        .is_err());
        s.cancel.cancel();
        assert!(publish(
            &s,
            Some(ModelValidation {
                splat_count: 2,
                sh_bands: 3
            })
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"old export");
        assert_eq!(std::fs::read(&s.source).unwrap(), original);
        let ply = fixture(dir.path(), ModelFormat::Ply).await;
        let blocked_destination = dir.path().join("edit.ply");
        std::fs::create_dir(&blocked_destination).unwrap();
        std::fs::write(blocked_destination.join("sentinel"), b"keep").unwrap();
        assert!(publish(&ply, None).await.is_err());
        assert_eq!(
            std::fs::read(blocked_destination.join("sentinel")).unwrap(),
            b"keep"
        );
        assert_eq!(std::fs::read(&ply.output).unwrap(), original);
    }
    #[tokio::test]
    async fn standard_ply_publication_is_atomic_without_compression_or_validation() {
        let dir = tempfile::tempdir().unwrap();
        let s = fixture(dir.path(), ModelFormat::Ply).await;
        let original = std::fs::read(&s.source).unwrap();
        std::fs::write(dir.path().join("edit.ply"), b"old export").unwrap();
        assert!(write_chunk(&s, 0, vec![1]).await.is_err());
        let result = publish(&s, None).await.unwrap();
        assert_eq!(result.file_size, original.len() as u64);
        assert_eq!(
            std::fs::read(dir.path().join("edit.ply")).unwrap(),
            original
        );
        assert_eq!(std::fs::read(&s.source).unwrap(), original);
        assert!(!s.output.exists());
    }
    #[test]
    fn preparation_metadata_has_no_compression_budget_or_permission_fields() {
        let prepared = PreparedModelExport {
            export_id: Uuid::nil(),
            file_size: 3 * 1024 * MIB,
            splat_count: 5_000_000,
            sh_bands: 3,
        };
        let json = serde_json::to_value(prepared).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 4);
        assert_eq!(json["fileSize"], 3 * 1024 * MIB);
        for removed in [
            "estimatedMemoryBytes",
            "memoryBudgetBytes",
            "compressionAllowed",
        ] {
            assert!(json.get(removed).is_none());
        }
    }

    #[test]
    fn virtual_view_accepts_filtered_models_above_former_memory_thresholds() {
        use crate::project::GaussianCrop;
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.ply");
        gaussian_fixture(&input, 400_000);
        let mut mask = vec![0; 400_000_usize.div_ceil(8)];
        mask[0] = 1;
        for crop in [
            None,
            Some(GaussianCrop::Sphere {
                center: [0.0; 3],
                radius: 1000.0,
                inverted: false,
            }),
        ] {
            let mut view = EditedPlyView::open(
                &input,
                GaussianTransform::default(),
                GaussianExportEdits {
                    crop,
                    deleted_mask: Some(&mask),
                },
                |_, _| {},
                || Ok(()),
            )
            .unwrap();
            let info = view.info();
            // Demonstrate this really exceeds the old cap, without retaining
            // the removed estimate as a production resource policy.
            let former_estimate = 6 * info.file_size + 128 * info.splat_count + 64 * MIB;
            assert!(former_estimate > 512 * MIB);
            assert_eq!(info.splat_count, 399_999);
            assert_eq!(view.sh_bands(), 3);
            assert!(!view
                .read(info.file_size - 16, 16, || Ok(()))
                .unwrap()
                .is_empty());
            assert!(view.read(0, 1, || Err(SplatError::Cancelled)).is_err());
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(inspect_gaussian_ply(&input).unwrap().splat_count, 400_000);
    }
    #[test]
    fn formats_are_a_closed_set_and_paths_are_fixed() {
        assert!(serde_json::from_str::<ModelFormat>("\"../../final.ply\"").is_err());
        assert_eq!(ModelFormat::Sog.filename(), "edit.sog");
        assert_eq!(ModelFormat::CompressedPly.filename(), "edit.compressed.ply");
    }
    #[test]
    fn revision_and_round_trip_require_exact_counts_and_sh() {
        let info = PlyInfo {
            file_size: 10,
            splat_count: 2,
        };
        assert!(check_revision(3, 4).is_err());
        assert!(validate(
            &info,
            3,
            &ModelValidation {
                splat_count: 2,
                sh_bands: 3
            }
        )
        .is_ok());
        assert!(validate(
            &info,
            3,
            &ModelValidation {
                splat_count: 1,
                sh_bands: 3
            }
        )
        .is_err());
        assert!(validate(
            &info,
            3,
            &ModelValidation {
                splat_count: 2,
                sh_bands: 0
            }
        )
        .is_err());
    }
}
