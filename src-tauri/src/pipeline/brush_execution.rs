//! Local execution evidence, deliberately separate from the anonymous telemetry DTO.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{event::PipelineOperation, runtime::RuntimeSnapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrushDeviceSnapshot {
    pub name: String,
    pub total_memory_mb: Option<u64>,
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrushExecutionSnapshot {
    pub operation: Option<PipelineOperation>,
    pub process_id: u32,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub backend: Option<String>,
    pub device: Option<BrushDeviceSnapshot>,
    pub config: BTreeMap<String, String>,
    #[serde(default)]
    pub configuration: Option<crate::presets::ResolvedBrushTrainingPreset>,
}

impl BrushExecutionSnapshot {
    pub fn new(runtime: &RuntimeSnapshot, operation: Option<PipelineOperation>) -> Self {
        let mut result = Self {
            operation,
            process_id: runtime.process_id,
            started_at: runtime.updated_at,
            ended_at: None,
            exit_code: None,
            backend: None,
            device: None,
            config: BTreeMap::new(),
            configuration: None,
        };
        result.update(runtime);
        result
    }

    pub fn update(&mut self, runtime: &RuntimeSnapshot) {
        if self.ended_at.is_some() || self.process_id != runtime.process_id {
            return;
        }
        self.backend = runtime.backend.clone();
        self.config = runtime.config.clone();
        self.device = runtime.device.as_ref().map(|name| {
            let matching = runtime.resources.as_ref().map(|resources| {
                resources
                    .gpus
                    .iter()
                    .filter(|gpu| gpu.name.eq_ignore_ascii_case(name))
                    .collect::<Vec<_>>()
            });
            // Duplicate adapter names cannot establish which physical card supplied VRAM.
            let memory = match matching.as_deref() {
                Some([gpu]) => gpu.memory_total_mib.filter(|v| *v > 0),
                Some(gpus) if gpus.len() > 1 => None,
                _ => runtime
                    .config
                    .get("device_total_memory_mb")
                    .and_then(|v| v.parse().ok())
                    .filter(|v| *v > 0),
            };
            BrushDeviceSnapshot {
                name: name.clone(),
                total_memory_mb: memory,
                driver_version: runtime.config.get("device_driver_version").cloned(),
            }
        });
    }

    pub fn finish(&mut self, exit_code: Option<i32>) {
        if self.ended_at.is_none() {
            self.ended_at = Some(Utc::now());
            self.exit_code = exit_code;
        }
    }

    /// Whitelist fields: no PID, hardware UUID, bus address, image names or paths.
    pub fn diagnostic_summary(&self, index: usize) -> String {
        let device = self.device.as_ref();
        let mut result = format!(
            "Brush attempt {}: device={}; backend={}; totalMemoryMiB={}; driver={}; exitCode={:?}",
            index + 1,
            device.map_or("unknown", |d| d.name.as_str()),
            self.backend.as_deref().unwrap_or("unknown"),
            device
                .and_then(|d| d.total_memory_mb)
                .map_or_else(|| "unknown".into(), |v| v.to_string()),
            device
                .and_then(|d| d.driver_version.as_deref())
                .unwrap_or("unknown"),
            self.exit_code,
        );
        if let Some(configuration) = self.configuration {
            result.push_str(&format!(
                "; profile={}; plannedMaxSplats={:?}; maxResolution={}; totalSteps={}",
                configuration.profile.label(),
                configuration.preset.max_splats,
                configuration.preset.max_resolution,
                configuration.preset.total_steps
            ));
            if let Some(densification) = configuration.preset.densification {
                result.push_str(&format!(
                    "; growthGradThreshold={}; growthSelectFraction={}; growthStopIter={}",
                    densification.growth_grad_threshold,
                    densification.growth_select_fraction,
                    densification.growth_stop_iter
                ));
            }
        }
        for key in [
            "total_iters",
            "max_resolution",
            "max_splats",
            "growth_start_iter",
            "growth_stop_iter",
            "refine_every",
            "seed",
        ] {
            if let Some(value) = self.config.get(key).filter(|v| v.parse::<u64>().is_ok()) {
                result.push_str(&format!("; {key}={value}"));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::runtime::RuntimeTracker;

    #[test]
    fn execution_is_frozen_and_does_not_inherit_a_previous_device() {
        let mut runtime = RuntimeTracker::new();
        runtime.snapshot.process_id = 1;
        runtime.snapshot.device = Some("NVIDIA GPU".into());
        let mut first = BrushExecutionSnapshot::new(&runtime.snapshot, None);
        first.finish(Some(1));
        runtime.snapshot.device = Some("Other GPU".into());
        first.update(&runtime.snapshot);
        assert_eq!(first.device.unwrap().name, "NVIDIA GPU");
        let second = BrushExecutionSnapshot::new(&RuntimeTracker::new().snapshot, None);
        assert!(second.device.is_none());
    }

    #[test]
    fn summary_does_not_include_private_config_or_hardware_identifiers() {
        let mut runtime = RuntimeTracker::new();
        runtime
            .snapshot
            .config
            .insert("directory".into(), "C:/private/GPU-uuid".into());
        runtime
            .snapshot
            .config
            .insert("max_splats".into(), "2000000".into());
        let summary = BrushExecutionSnapshot::new(&runtime.snapshot, None).diagnostic_summary(0);
        assert!(summary.contains("max_splats=2000000"));
        assert!(!summary.contains("private"));
        assert!(!summary.contains("uuid"));
    }

    #[test]
    fn vram_requires_a_unique_actual_device_match() {
        use crate::process::resources::{GpuResources, ProcessResources};
        let gpu = |name: &str| GpuResources {
            uuid: "private-uuid".into(),
            name: name.into(),
            utilization_percent: None,
            memory_used_mib: None,
            memory_total_mib: Some(8192),
        };
        let mut runtime = RuntimeTracker::new();
        runtime.snapshot.process_id = 1;
        runtime.snapshot.device = Some("Actual GPU".into());
        runtime
            .snapshot
            .config
            .insert("device_total_memory_mb".into(), "24576".into());
        runtime.snapshot.resources = Some(ProcessResources {
            process_id: 1,
            sampled_at: Utc::now(),
            cpu_percent: None,
            memory_bytes: None,
            gpus: vec![gpu("Other GPU"), gpu("Actual GPU")],
            gpu_status: "available".into(),
        });
        let mut execution = BrushExecutionSnapshot::new(&runtime.snapshot, None);
        assert_eq!(
            execution.device.as_ref().unwrap().total_memory_mb,
            Some(8192)
        );
        runtime
            .snapshot
            .resources
            .as_mut()
            .unwrap()
            .gpus
            .push(gpu("Actual GPU"));
        execution.update(&runtime.snapshot);
        assert_eq!(execution.device.unwrap().total_memory_mb, None);
    }
}
