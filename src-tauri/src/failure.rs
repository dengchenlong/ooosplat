//! Classification of new failures. Native status and explicit engine evidence
//! survive IPC; presentation never reclassifies a truncated/historical log.
use crate::{
    error::SplatError,
    pipeline::{PipelineEngine, PipelineStage},
    process::{ProcessOutput, ProcessTermination},
};

pub const CLASSIFICATION_VERSION: u32 = 1;

pub fn known_kind(value: Option<&str>) -> Option<&'static str> {
    [
        "brush_out_of_memory",
        "brush_resource_limit",
        "colmap_crash",
        "mapper_source",
        "mapper_storage",
        "brush_dataset",
        "brush_device_lost",
        "brush_gpu",
    ]
    .into_iter()
    .find(|kind| Some(*kind) == value)
}

#[derive(Debug, Clone, Copy)]
pub struct FailureClassification {
    pub engine: Option<PipelineEngine>,
    pub kind: Option<&'static str>,
    pub heuristic: bool,
}

#[derive(Debug, Clone)]
pub struct EngineProcessFailure {
    pub last_splat_count: Option<u64>,
    pub engine: PipelineEngine,
    pub operation: String,
    pub termination: ProcessTermination,
    pub detail: String,
    pub classification: FailureClassification,
    pub previous_attempts: Vec<EngineProcessFailure>,
}

impl EngineProcessFailure {
    pub fn from_output(engine: PipelineEngine, operation: &str, output: &ProcessOutput) -> Self {
        let streams = [output.stdout.as_str(), output.stderr.as_str()];
        let mut classification = FailureClassification {
            engine: Some(engine),
            kind: None,
            heuristic: false,
        };
        let (kind, predicate): (Option<&'static str>, fn(&str) -> bool) = match engine {
            PipelineEngine::Brush if streams.iter().any(|s| gpu_oom(s)) => {
                (Some("brush_out_of_memory"), gpu_oom)
            }
            PipelineEngine::Brush if streams.iter().any(|s| gpu_resource_limit(s)) => {
                (Some("brush_resource_limit"), gpu_resource_limit)
            }
            PipelineEngine::Brush if streams.iter().any(|s| device_lost(s)) => {
                (Some("brush_device_lost"), device_lost)
            }
            PipelineEngine::Colmap if output.termination.abnormal() => {
                (Some("colmap_crash"), |_| false)
            }
            PipelineEngine::Colmap if streams.iter().any(|s| storage(s)) => {
                classification.heuristic = true;
                (Some("mapper_storage"), storage)
            }
            PipelineEngine::Colmap
                if operation == "mapper" && streams.iter().any(|s| mapper_source(s)) =>
            {
                classification.heuristic = true;
                (Some("mapper_source"), mapper_source)
            }
            PipelineEngine::Brush if streams.iter().any(|s| dataset(s)) => {
                classification.heuristic = true;
                (Some("brush_dataset"), dataset)
            }
            _ => (None, |_| false),
        };
        classification.kind = kind;
        // Preserve bounded root evidence from BOTH current streams, including
        // early lines outside failure_detail(). No full output crosses IPC.
        let evidence = streams
            .iter()
            .flat_map(|s| s.lines())
            .filter(|line| predicate(line))
            .take(4)
            .map(|line| line.chars().take(512).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        let tail = output.failure_detail();
        let detail = if evidence.is_empty() {
            tail
        } else {
            format!("{evidence}\n--- current attempt tail ---\n{tail}")
        };
        Self {
            // Brush fills this from its per-process observer, which preserves
            // cross-stream arrival order and survives bounded output capture.
            last_splat_count: None,
            engine,
            operation: operation.into(),
            termination: output.termination.clone(),
            detail,
            classification,
            previous_attempts: vec![],
        }
    }
}

impl std::fmt::Display for EngineProcessFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let engine = match self.engine {
            PipelineEngine::Colmap => "COLMAP",
            PipelineEngine::Brush => "Brush",
            _ => "Engine",
        };
        write!(
            f,
            "{engine} {} attempt {}: {}\n{}",
            self.operation,
            self.previous_attempts.len() + 1,
            self.termination,
            self.detail
        )?;
        for (index, previous) in self.previous_attempts.iter().enumerate() {
            write!(
                f,
                "\n--- previous attempt {} (context only) ---\n{}\n{}",
                index + 1,
                previous.termination,
                previous.detail
            )?;
        }
        Ok(())
    }
}

pub fn classify(stage: Option<PipelineStage>, error: &SplatError) -> FailureClassification {
    if let SplatError::EngineProcess(failure) = error {
        return failure.classification;
    }
    let engine = match stage {
        Some(
            PipelineStage::Reconstructing
            | PipelineStage::ValidatingReconstruction
            | PipelineStage::ExtractingFeatures
            | PipelineStage::Matching,
        ) => PipelineEngine::Colmap,
        Some(PipelineStage::TrainingSplats) => PipelineEngine::Brush,
        Some(PipelineStage::ProbingVideo | PipelineStage::ExtractingFrames) => {
            PipelineEngine::Ffmpeg
        }
        _ => PipelineEngine::System,
    };
    let mut result = FailureClassification {
        engine: Some(engine),
        kind: None,
        heuristic: false,
    };
    match error {
        SplatError::Cancelled => return result,
        SplatError::BrushOutOfMemory(_) => {
            result.engine = Some(PipelineEngine::Brush);
            result.kind = Some("brush_out_of_memory");
        }
        SplatError::BrushDeviceLost(_) => {
            result.engine = Some(PipelineEngine::Brush);
            result.kind = Some("brush_device_lost");
        }
        SplatError::Io(_) if engine == PipelineEngine::Colmap => {
            result.kind = Some("mapper_storage")
        }
        SplatError::Process(message) => {
            result.kind = if engine == PipelineEngine::Colmap && storage(message) {
                Some("mapper_storage")
            } else if matches!(
                stage,
                Some(PipelineStage::Reconstructing | PipelineStage::ValidatingReconstruction)
            ) && mapper_source(message)
            {
                Some("mapper_source")
            } else if engine == PipelineEngine::Brush && gpu_oom(message) {
                Some("brush_out_of_memory")
            } else if engine == PipelineEngine::Brush && gpu_resource_limit(message) {
                Some("brush_resource_limit")
            } else if engine == PipelineEngine::Brush && device_lost(message) {
                Some("brush_device_lost")
            } else if engine == PipelineEngine::Brush && dataset(message) {
                Some("brush_dataset")
            } else {
                None
            };
            result.heuristic = result.kind.is_some();
        }
        _ => {}
    }
    result
}

pub fn gpu_oom(text: &str) -> bool {
    text.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        if [
            "out_of_host_memory",
            "host memory",
            "cpu memory",
            "system memory",
        ]
        .iter()
        .any(|s| lower.contains(s))
        {
            return false;
        }
        [
            "vk_error_out_of_device_memory",
            "error_out_of_device_memory",
            "cuda_error_out_of_memory",
            "dxgi_error_out_of_memory",
        ]
        .iter()
        .any(|s| lower.contains(s))
            || ((lower.contains("out of memory") || lower.contains("outofmemory"))
                && [
                    "wgpu", "gpu", "vram", "cuda", "vulkan", "metal", "d3d", "显存",
                ]
                .iter()
                .any(|s| lower.contains(s)))
    })
}
pub fn gpu_resource_limit(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "buffertoobig",
        "buffer too big",
        "exceeds max_buffer_size",
        "exceeds the maximum buffer size",
    ]
    .iter()
    .any(|s| lower.contains(s))
}
pub fn device_lost(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "devicelost",
        "device lost",
        "device_lost",
        "parent device is lost",
        "vk_error_device_lost",
        "dxgi_error_device_removed",
        "显卡设备连接中断",
    ]
    .iter()
    .any(|s| lower.contains(s))
}
fn dataset(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "early eof",
        "failed to load dataset",
        "no such file",
        "file not found",
        "access denied",
        "permission denied",
    ]
    .iter()
    .any(|s| lower.contains(s))
}
fn storage(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "database is locked",
        "database locked",
        "unable to open database",
        "failed to open database",
        "cannot open database",
        "read-only database",
        "readonly database",
        "no space left",
        "disk full",
        "input/output error",
        "being used by another process",
        "used by another process",
        "sharing violation",
        "access denied",
        "permission denied",
        "数据库被锁定",
        "无法打开数据库",
        "磁盘空间不足",
        "磁盘已满",
        "另一个进程正在使用",
        "拒绝访问",
        "权限不足",
    ]
    .iter()
    .any(|s| lower.contains(s))
        || ((lower.contains("io error") || lower.contains("i/o error"))
            && ["database", "sqlite", "filesystem", "project file"]
                .iter()
                .any(|s| lower.contains(s)))
}
fn mapper_source(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // A single rejected registration or discarded submodel is normal mapper
    // activity, not proof that reconstruction as a whole failed.
    [
        "no initial image pair",
        "no good initial image pair",
        "could not find a good initial image pair",
        "failed to find an initial image pair",
        "no images registered",
        "registered 0 images",
        "failed to create any sparse model",
        "did not produce a usable sparse model",
        "未生成完整的稀疏模型",
        "稀疏重建没有可用的注册图像或三维点",
        "稀疏重建输出不完整",
    ]
    .iter()
    .any(|s| lower.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(
        engine: PipelineEngine,
        code: i32,
        stdout: &str,
        stderr: &str,
    ) -> EngineProcessFailure {
        EngineProcessFailure::from_output(
            engine,
            "mapper",
            &ProcessOutput {
                success: false,
                exit_code: Some(code),
                termination: ProcessTermination {
                    exit_code: Some(code),
                    ..Default::default()
                },
                stdout: stdout.into(),
                stderr: stderr.into(),
            },
        )
    }

    #[test]
    fn issue_71_early_gpu_oom_wins_over_later_io_in_either_stream() {
        // Public redacted issue excerpts, not the unavailable full report.
        let tail = format!("{}\nFailed to read counts: An error happened during execution\nAn IO error happened\nMemory location was never initialized", "kernel skipped\n".repeat(600));
        for (stdout, stderr) in [
            ("wgpu error: Out of Memory".into(), tail.clone()),
            (String::new(), format!("wgpu error: Out of Memory\n{tail}")),
        ] {
            let result = failure(PipelineEngine::Brush, 101, &stdout, &stderr);
            assert_eq!(result.classification.kind, Some("brush_out_of_memory"));
            assert!(!result.classification.heuristic);
            assert!(result.detail.contains("wgpu error: Out of Memory"));
            assert!(result.detail.chars().count() < 7_000);
            let error = SplatError::EngineProcess(Box::new(result));
            assert!(error.is_brush_oom());
            assert_eq!(
                classify(Some(PipelineStage::TrainingSplats), &error).kind,
                Some("brush_out_of_memory")
            );
        }
    }

    #[test]
    fn gpu_limit_device_loss_host_allocation_and_exit_codes_are_not_gpu_oom() {
        for (text, kind) in [
            ("BufferTooBig(2290420416)", Some("brush_resource_limit")),
            (
                "VK_ERROR_DEVICE_LOST\nAn IO error happened",
                Some("brush_device_lost"),
            ),
            ("GPU allocation failed", None),
            ("wgpu initialized\nCPU out of memory", None),
            ("VK_ERROR_OUT_OF_HOST_MEMORY", None),
            (
                "thread panicked\nFailed to read counts\nAn IO error happened",
                None,
            ),
            ("process exited with code 101", None),
            (
                "IO error while loading dataset: early eof",
                Some("brush_dataset"),
            ),
            (
                "No such file or directory: images/a.png",
                Some("brush_dataset"),
            ),
        ] {
            let result = failure(PipelineEngine::Brush, 101, "", text);
            assert_eq!(result.classification.kind, kind, "{text}");
            assert!(!SplatError::EngineProcess(Box::new(result)).is_brush_oom());
        }
    }

    #[test]
    fn issue_72_native_termination_wins_over_rejected_registration_logs() {
        let mut result = failure(
            PipelineEngine::Colmap,
            -1073740940,
            "435 (connected 435, loaded 435)\nRegistering initial image pair #206 and #285",
            "Could not register image\nDiscarding reconstruction because it is too small",
        );
        result.termination.windows_status = Some("0xC0000374".into());
        let output = ProcessOutput {
            success: false,
            exit_code: Some(-1073740940),
            termination: result.termination.clone(),
            stdout: result.detail,
            stderr: "Failed to create any sparse model".into(),
        };
        let result = EngineProcessFailure::from_output(PipelineEngine::Colmap, "mapper", &output);
        assert_eq!(result.classification.kind, Some("colmap_crash"));
        assert!(!result.classification.heuristic);
        let message = result.to_string();
        assert!(message.contains("-1073740940"));
        assert!(message.contains("0xC0000374"));
    }

    #[test]
    fn last_attempt_owns_classification_not_previous_crash_or_material_logs() {
        let final_attempt = failure(
            PipelineEngine::Colmap,
            1,
            "",
            "SQLite error: database is locked",
        );
        let mut previous = failure(
            PipelineEngine::Colmap,
            -1073740940,
            "",
            "No good initial image pair",
        );
        previous.termination.windows_status = Some("0xC0000374".into());
        let error = SplatError::EngineProcess(Box::new(final_attempt))
            .with_previous_attempt(SplatError::EngineProcess(Box::new(previous.clone())));
        assert_eq!(
            classify(Some(PipelineStage::Reconstructing), &error).kind,
            Some("mapper_storage")
        );
        assert!(error.to_string().contains("attempt 2"));
        assert!(error
            .to_string()
            .contains("previous attempt 1 (context only)"));
        assert!(matches!(
            SplatError::Cancelled
                .with_previous_attempt(SplatError::EngineProcess(Box::new(previous))),
            SplatError::Cancelled
        ));
    }

    #[test]
    fn unknown_exit_is_not_a_material_or_gpu_diagnosis_and_cancel_is_not_classified() {
        for engine in [PipelineEngine::Colmap, PipelineEngine::Brush] {
            assert_eq!(
                failure(engine, 1, "", "Could not register image #235")
                    .classification
                    .kind,
                None
            );
            assert_eq!(
                failure(engine, -1, "", "unknown exception")
                    .classification
                    .kind,
                None
            );
        }
        assert_eq!(
            classify(Some(PipelineStage::Reconstructing), &SplatError::Cancelled).kind,
            None
        );
        assert_eq!(
            classify(
                Some(PipelineStage::Reconstructing),
                &SplatError::Process(
                    "SQLite database is locked\nFailed to create any sparse model".into()
                )
            )
            .kind,
            Some("mapper_storage")
        );
    }
}
