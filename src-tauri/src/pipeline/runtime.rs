use std::{collections::BTreeMap, time::Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::process::resources::ProcessResources;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingRuntime {
    pub iteration: Option<u64>,
    pub total: Option<u64>,
    pub start_iter: u64,
    pub lod: u32,
    pub steps_per_second: Option<f64>,
    pub remaining_seconds: Option<f64>,
    pub splat_count: Option<u64>,
    pub psnr: Option<f64>,
    pub ssim: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub process_id: u32,
    pub phase: String,
    pub updated_at: DateTime<Utc>,
    pub last_output_age_ms: u64,
    pub training: Option<TrainingRuntime>,
    pub device: Option<String>,
    pub backend: Option<String>,
    pub config: BTreeMap<String, String>,
    pub resources: Option<ProcessResources>,
}

pub(crate) struct RuntimeTracker {
    pub snapshot: RuntimeSnapshot,
    last_output: Instant,
    previous_step: Option<(u64, Instant, u32)>,
}

impl RuntimeTracker {
    /// Metadata may enrich an explicitly reported adapter, never choose the adapter.
    pub fn enrich_device(&mut self, acceleration: &crate::engines::ColmapAccelerationStatus) {
        let Some(device) = acceleration.device.as_ref().filter(|device| {
            acceleration.detected_nvidia_device_count == 1
                && self
                    .snapshot
                    .device
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&device.name))
        }) else {
            return;
        };
        if let Some(memory) = device.total_memory_mb.filter(|v| *v > 0) {
            self.snapshot
                .config
                .insert("device_total_memory_mb".into(), memory.to_string());
        }
        if device.driver_version.chars().any(|c| c.is_ascii_digit()) {
            self.snapshot
                .config
                .entry("device_driver_version".into())
                .or_insert_with(|| device.driver_version.clone());
        }
    }

    pub fn new() -> Self {
        Self {
            snapshot: RuntimeSnapshot {
                process_id: 0,
                phase: "starting".into(),
                updated_at: Utc::now(),
                last_output_age_ms: 0,
                training: None,
                device: None,
                backend: None,
                config: BTreeMap::new(),
                resources: None,
            },
            last_output: Instant::now(),
            previous_step: None,
        }
    }

    pub fn refresh(&mut self, now: Instant) -> RuntimeSnapshot {
        self.snapshot.updated_at = Utc::now();
        self.snapshot.last_output_age_ms = now.duration_since(self.last_output).as_millis() as u64;
        if self.snapshot.phase != "training" || self.snapshot.last_output_age_ms > 10_000 {
            if let Some(training) = &mut self.snapshot.training {
                training.steps_per_second = None;
                training.remaining_seconds = None;
            }
        }
        self.snapshot.clone()
    }

    /// Returns progress only when Brush reports an actual completed iteration.
    pub fn line(&mut self, line: &str, now: Instant, brush: bool) -> Option<(u64, u64)> {
        self.last_output = now;
        if !brush {
            return None;
        }

        if line.contains("Training config:") || line.contains("Export config:") {
            let config = line.split_once("config:")?.1;
            for item in config.split_whitespace() {
                if let Some((key, value)) = item.split_once('=') {
                    if !matches!(key, "directory" | "name") {
                        self.snapshot.config.insert(key.into(), value.into());
                    }
                }
            }
            if line.contains("Training config:") {
                let training = self.snapshot.training.get_or_insert_with(Default::default);
                training.total = integer(line, "total_iters");
                training.start_iter = integer(line, "start_iter").unwrap_or(0);
            }
        }
        if line.contains("GPU initialization started") {
            self.set_phase("gpuInitialization");
        }
        if line.contains("GPU initialization completed") {
            self.snapshot.backend = field(line, "backend").map(str::to_owned);
            let device = line
                .split_once("device=")
                .and_then(|(_, rest)| rest.split_once(" type="))
                .map(|(name, _)| name.to_owned());
            if self.snapshot.device != device {
                self.snapshot.config.remove("device_driver_version");
                self.snapshot.config.remove("device_total_memory_mb");
            }
            self.snapshot.device = device;
            // `driver=NVIDIA` is a vendor, not a driver version.
            if let Some((_, driver)) = line.split_once(" driver_info=") {
                if driver.chars().any(|c| c.is_ascii_digit()) {
                    self.snapshot.config.insert(
                        "device_driver_version".into(),
                        driver.chars().take(256).collect(),
                    );
                }
            }
            self.set_phase("loading");
        }
        if line.contains("Loading dataset") || line.contains("Loading initial splats") {
            self.set_phase("loading");
        }
        if line.contains("Initial splat initialization started") {
            self.set_phase("splatInitialization");
        }
        if line.contains("Trainer preparation started") {
            self.set_phase("trainerPreparation");
        }
        if line.contains("Start training loop") || line.contains("First training step started") {
            self.set_phase("training");
        }
        if line.contains("Running evaluation for iteration") {
            self.set_phase("evaluation");
        }
        if line.contains("Export started:") {
            self.set_phase("exporting");
        }
        if line.contains("Export succeeded") || line.contains("Export failed after") {
            self.set_phase("training");
        }
        if line.contains("LOD ")
            && (line.contains("Decimating") || line.contains("Computing sensitivity"))
        {
            self.set_phase("lodPreparation");
        }
        if line.contains("Training loop completed") || line.contains("Done training!") {
            self.set_phase("finishing");
        }

        if line.contains("Refine iter ") {
            if let Some((_, rest)) = line.split_once("Refine iter ") {
                let count = rest
                    .split_once(',')
                    .and_then(|(_, count)| count.split_whitespace().next())
                    .and_then(|value| value.parse().ok());
                self.snapshot
                    .training
                    .get_or_insert_with(Default::default)
                    .splat_count = count;
            }
        }
        if line.contains("Initial splats initialized in") {
            if let Some((_, count)) = line.rsplit_once(": ") {
                self.snapshot
                    .training
                    .get_or_insert_with(Default::default)
                    .splat_count = count
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse().ok());
            }
        }
        if let Some((_, rest)) = line.split_once("PSNR ") {
            if let Some((psnr, ssim)) = rest.split_once(", ssim ") {
                let training = self.snapshot.training.get_or_insert_with(Default::default);
                training.psnr = finite(psnr.trim());
                training.ssim = finite(ssim.trim());
            }
        }

        if !line.contains("Training progress:") {
            return None;
        }
        let iteration = integer(line, "iteration")?;
        let total = integer(line, "total")?;
        let lod = integer(line, "lod")? as u32;
        if total == 0 || iteration > total {
            return None;
        }

        let previous_phase = self.snapshot.phase.clone();
        self.set_phase("training");
        let training = self.snapshot.training.get_or_insert_with(Default::default);
        if training.iteration.is_some_and(|old| iteration < old) {
            return None;
        }
        training.iteration = Some(iteration);
        training.total = Some(total);
        training.lod = lod;
        training.steps_per_second = None;
        training.remaining_seconds = None;
        if previous_phase == "training" {
            if let Some((last, at, previous_lod)) = self.previous_step {
                let elapsed = now.duration_since(at).as_secs_f64();
                if previous_lod == lod && iteration > last && elapsed > 0.0 && elapsed <= 10.0 {
                    let speed = (iteration - last) as f64 / elapsed;
                    training.steps_per_second = Some(speed);
                    training.remaining_seconds = Some((total - iteration) as f64 / speed);
                }
            }
        }
        self.previous_step = Some((iteration, now, lod));
        Some((iteration, total))
    }

    fn set_phase(&mut self, phase: &str) {
        if self.snapshot.phase != phase {
            self.previous_step = None;
        }
        self.snapshot.phase = phase.into();
    }
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|part| {
        part.split_once('=')
            .filter(|(candidate, _)| *candidate == key)
            .map(|(_, value)| value)
    })
}

fn integer(line: &str, key: &str) -> Option<u64> {
    field(line, key)?.parse().ok()
}

pub(crate) fn brush_splat_count(line: &str) -> Option<u64> {
    if let Some((_, rest)) = line.split_once("Refine iter ") {
        let (iteration, tail) = rest.split_once(',')?;
        iteration.trim().parse::<u64>().ok()?;
        let (number, suffix) = tail.trim().split_once(' ')?;
        return suffix
            .starts_with("splats")
            .then(|| number.parse().ok())
            .flatten();
    }
    if line.contains("Initial splats initialized in") {
        return line
            .rsplit_once(": ")?
            .1
            .split_whitespace()
            .next()?
            .parse()
            .ok();
    }
    None
}

fn finite(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    #[test]
    fn splat_counts_require_a_real_brush_report_not_a_budget_or_unrelated_number() {
        assert_eq!(
            super::brush_splat_count("INFO Initial splats initialized in 1ms: 10192"),
            Some(10192)
        );
        assert_eq!(
            super::brush_splat_count("INFO Refine iter 200, 244621 splats."),
            Some(244621)
        );
        for line in [
            "max_splats=10000000",
            "read 244621 bytes",
            "Refine iter broken, 12 splats.",
            "Refine iter 200, NaN splats.",
            "Refine iter 200, 100 images.",
        ] {
            assert_eq!(super::brush_splat_count(line), None);
        }
    }
    use super::*;
    use std::time::Duration;

    #[test]
    fn hardware_metadata_cannot_select_an_adapter_or_resolve_ambiguous_cards() {
        let acceleration = |count| {
            serde_json::from_value(serde_json::json!({
                "backend":"gpu", "detectionState":"ready", "reasonCode":"gpuReady",
                "reason":"fixture", "detectedNvidiaDeviceCount": count,
                "device": {"index":0,"name":"NVIDIA RTX","driverVersion":"616.92",
                    "computeCapability":"8.6","totalMemoryMb":8192},
                "requirements":{"minimumDriverVersion":"580","minimumComputeCapability":"7.5"}
            }))
            .unwrap()
        };
        let mut tracker = RuntimeTracker::new();
        tracker.enrich_device(&acceleration(1));
        assert!(tracker.snapshot.device.is_none());
        assert!(tracker.snapshot.config.is_empty());
        tracker.line("GPU initialization completed: backend=Vulkan device=Other GPU type=DiscreteGpu driver=unknown", Instant::now(), true);
        tracker.enrich_device(&acceleration(1));
        assert!(tracker.snapshot.config.is_empty());
        tracker.line("GPU initialization completed: backend=Vulkan device=NVIDIA RTX type=DiscreteGpu driver=NVIDIA driver_info=620.00", Instant::now(), true);
        tracker.enrich_device(&acceleration(2));
        assert!(!tracker
            .snapshot
            .config
            .contains_key("device_total_memory_mb"));
        tracker.enrich_device(&acceleration(1));
        assert_eq!(tracker.snapshot.config["device_total_memory_mb"], "8192");
        assert_eq!(tracker.snapshot.config["device_driver_version"], "620.00");
    }

    #[test]
    fn driver_version_is_from_the_actual_adapter_not_the_vendor() {
        let mut tracker = RuntimeTracker::new();
        let now = Instant::now();
        tracker.line("GPU initialization completed in 0.4s: backend=Vulkan device=NVIDIA RTX type=DiscreteGpu driver=NVIDIA driver_info=616.92", now, true);
        assert_eq!(tracker.snapshot.device.as_deref(), Some("NVIDIA RTX"));
        assert_eq!(
            tracker
                .snapshot
                .config
                .get("device_driver_version")
                .map(String::as_str),
            Some("616.92")
        );
        let mut missing = RuntimeTracker::new();
        missing.line("GPU initialization completed: backend=Vulkan device=NVIDIA RTX type=DiscreteGpu driver=NVIDIA driver_info=unknown", now, true);
        assert!(!missing
            .snapshot
            .config
            .contains_key("device_driver_version"));
        tracker
            .snapshot
            .config
            .insert("device_total_memory_mb".into(), "8192".into());
        tracker.line("GPU initialization completed: backend=Vulkan device=AMD Actual type=DiscreteGpu driver=AMD driver_info=unknown", now, true);
        assert!(!tracker
            .snapshot
            .config
            .contains_key("device_driver_version"));
        assert!(!tracker
            .snapshot
            .config
            .contains_key("device_total_memory_mb"));
    }

    #[test]
    fn reads_actual_progress_rate_lod_and_output_gap() {
        let mut tracker = RuntimeTracker::new();
        let now = Instant::now();
        tracker.line("Training config: total_iters=120 start_iter=20", now, true);
        assert_eq!(
            tracker.line(
                "Training progress: iteration=25 total=120 elapsed_secs=1 lod=0",
                now,
                true,
            ),
            Some((25, 120))
        );
        assert_eq!(
            tracker.snapshot.training.as_ref().unwrap().steps_per_second,
            None
        );
        tracker.line(
            "Training progress: iteration=35 total=120 elapsed_secs=2 lod=0",
            now + Duration::from_secs(1),
            true,
        );
        assert_eq!(
            tracker.snapshot.training.as_ref().unwrap().steps_per_second,
            Some(10.0)
        );
        tracker.line(
            "Training progress: iteration=40 total=120 elapsed_secs=3 lod=1",
            now + Duration::from_secs(2),
            true,
        );
        assert_eq!(
            tracker.snapshot.training.as_ref().unwrap().steps_per_second,
            None
        );
        let gap = tracker.refresh(now + Duration::from_secs(13));
        assert_eq!(gap.last_output_age_ms, 11_000);
        assert_eq!(gap.training.unwrap().iteration, Some(40));
    }

    #[test]
    fn reads_stages_device_metrics_and_rejects_bad_progress() {
        let mut tracker = RuntimeTracker::new();
        let now = Instant::now();
        tracker.line("GPU initialization completed in 1s: backend=Vulkan device=NVIDIA RTX type=DiscreteGpu driver=NVIDIA", now, true);
        assert_eq!(tracker.snapshot.device.as_deref(), Some("NVIDIA RTX"));
        tracker.line("Refine iter 100, 200 splats.", now, true);
        tracker.line("Eval iter 100: PSNR 25.5, ssim 0.9", now, true);
        assert_eq!(
            tracker.snapshot.training.as_ref().unwrap().splat_count,
            Some(200)
        );
        assert_eq!(tracker.snapshot.training.as_ref().unwrap().psnr, Some(25.5));
        tracker.line("Export started: iteration 100, final.ply", now, true);
        assert_eq!(tracker.snapshot.phase, "exporting");
        assert_eq!(
            tracker.line(
                "Training progress: iteration=nan total=100 lod=0",
                now,
                true,
            ),
            None
        );
        assert_eq!(
            tracker.line(
                "Training progress: iteration=101 total=100 lod=0",
                now,
                true,
            ),
            None
        );
        tracker.line("Training loop completed in 1s", now, true);
        assert_eq!(tracker.snapshot.phase, "finishing");
    }
}
