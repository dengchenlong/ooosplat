//! Panorama-only Brush recipes. Point budgets are ceilings, not growth targets.
use serde::{Deserialize, Serialize};

use super::{
    pipeline_optimization_config, BrushDensificationPreset, BrushTrainingPreset,
    BrushTrainingProfile, Quality, ResolvedBrushTrainingPreset,
};
use crate::error::{Result, SplatError};

pub const MAX_PANORAMA_SPLATS: u32 = 10_000_000;
pub const INITIAL_POINT_BUDGET_ERROR: &str =
    "初始化点数超出当前全景训练预算，已停止训练以避免丢弃初始点。";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanoramaTrainingConfig {
    pub recipe_version: u32,
    pub fast: PanoramaBrushRecipe,
    pub balanced: PanoramaBrushRecipe,
    pub high: PanoramaBrushRecipe,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanoramaBrushRecipe {
    pub total_steps: usize,
    pub max_splats: u32,
    /// Separate manual-mode cap; absent in older external configurations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_splats_without_optimization: Option<u32>,
    pub refine_every: u32,
    pub densification: BrushDensificationPreset,
}

impl Default for PanoramaTrainingConfig {
    fn default() -> Self {
        let recipe =
            |total_steps, max_splats, manual_cap, threshold, fraction, stop| PanoramaBrushRecipe {
                total_steps,
                max_splats,
                max_splats_without_optimization: Some(manual_cap),
                refine_every: 200,
                densification: BrushDensificationPreset {
                    growth_grad_threshold: threshold,
                    growth_select_fraction: fraction,
                    growth_stop_iter: stop,
                },
            };
        Self {
            recipe_version: 1,
            fast: recipe(12_000, 1_000_000, 1_000_000, 0.0018, 0.20, 10_000),
            balanced: recipe(30_000, 5_000_000, 2_000_000, 0.0012, 0.25, 25_000),
            high: recipe(45_000, 10_000_000, 3_000_000, 0.0010, 0.30, 38_000),
        }
    }
}

impl PanoramaTrainingConfig {
    pub fn recipe(&self, quality: Quality) -> PanoramaBrushRecipe {
        match quality {
            Quality::Fast => self.fast,
            Quality::Balanced => self.balanced,
            Quality::High => self.high,
        }
    }

    pub(super) fn validate(&self) -> std::result::Result<(), String> {
        if self.recipe_version != 1 {
            return Err("unsupported panoramaTraining.recipeVersion".into());
        }
        for (name, recipe) in [
            ("fast", self.fast),
            ("balanced", self.balanced),
            ("high", self.high),
        ] {
            if recipe.total_steps == 0
                || recipe.total_steps > u32::MAX as usize
                || recipe.max_splats == 0
                || recipe.max_splats > MAX_PANORAMA_SPLATS
                || recipe
                    .max_splats_without_optimization
                    .is_some_and(|cap| cap == 0 || cap > MAX_PANORAMA_SPLATS)
                || recipe.refine_every == 0
                || recipe.densification.growth_stop_iter as usize >= recipe.total_steps
                || recipe.densification.growth_stop_iter <= recipe.refine_every
            {
                return Err(format!(
                    "invalid panoramaTraining.{name} budget or schedule"
                ));
            }
            recipe
                .densification
                .validate(&format!("panoramaTraining.{name}.densification"))?;
        }
        Ok(())
    }
}

/// Saved with the resolved preset: never infer a new recipe for an old snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanoramaTrainingSnapshot {
    pub recipe_version: u32,
    pub quality: Quality,
    pub automatic_optimization: bool,
    pub memory_max_splats: Option<u32>,
    pub initial_points: u64,
    pub initial_points_override: bool,
}

pub fn panorama_memory_max_splats(quality: Quality, memory_mib: Option<u64>) -> u32 {
    match memory_mib {
        Some(n) if n >= 24 * 1024 => 10_000_000,
        Some(n) if n >= 16 * 1024 => 7_500_000,
        Some(n) if n >= 12 * 1024 => match quality {
            Quality::Balanced => 3_000_000,
            _ => 5_000_000,
        },
        Some(n) if n >= 8 * 1024 => match quality {
            Quality::High => 3_000_000,
            _ => 2_000_000,
        },
        _ => 1_000_000,
    }
}

pub fn resolve_panorama_training_preset(
    quality: Quality,
    automatic_optimization: bool,
    detected_total_memory_mb: Option<u64>,
    face_size: u32,
    initial_points: u64,
) -> Result<ResolvedBrushTrainingPreset> {
    if initial_points > MAX_PANORAMA_SPLATS as u64 {
        return Err(SplatError::Process(INITIAL_POINT_BUDGET_ERROR.into()));
    }
    let config = pipeline_optimization_config();
    let recipe = config.panorama_training.recipe(quality);
    let memory_max_splats = automatic_optimization
        .then(|| panorama_memory_max_splats(quality, detected_total_memory_mb));
    let requested_cap = if automatic_optimization {
        recipe.max_splats
    } else {
        recipe.max_splats_without_optimization.unwrap_or_else(|| {
            // Old config files still get the current manual defaults. Saved
            // resolved task snapshots bypass this resolver and remain intact.
            PanoramaTrainingConfig::default()
                .recipe(quality)
                .max_splats_without_optimization
                .expect("manual panorama defaults must define a cap")
        })
    };
    let base_cap = requested_cap.min(memory_max_splats.unwrap_or(MAX_PANORAMA_SPLATS));
    Ok(ResolvedBrushTrainingPreset {
        // Reuse existing profile labels for compatibility with metrics consumers.
        profile: match quality {
            Quality::Fast => BrushTrainingProfile::Fast,
            Quality::Balanced => BrushTrainingProfile::Balanced,
            Quality::High => BrushTrainingProfile::HighLarge,
        },
        detected_total_memory_mb,
        configured_max_splats: Some(requested_cap),
        panorama: Some(PanoramaTrainingSnapshot {
            recipe_version: config.panorama_training.recipe_version,
            quality,
            automatic_optimization,
            memory_max_splats,
            initial_points,
            initial_points_override: initial_points > base_cap as u64,
        }),
        preset: BrushTrainingPreset {
            total_steps: recipe.total_steps,
            max_resolution: face_size.max(1),
            refine_every: recipe.refine_every,
            max_splats: Some(base_cap.max(initial_points as u32)),
            densification: Some(recipe.densification),
        },
    })
}

pub(super) fn downgrade_panorama_after_oom(
    mut resolved: ResolvedBrushTrainingPreset,
    initial_points: u64,
) -> Option<ResolvedBrushTrainingPreset> {
    let snapshot = resolved.panorama?;
    if !snapshot.automatic_optimization {
        return None;
    }
    let cap = resolved.preset.max_splats?;
    let lower = (cap / 2).max(
        initial_points
            .max(snapshot.initial_points)
            .try_into()
            .ok()?,
    );
    if lower == 0 || lower >= cap {
        return None;
    }
    resolved.preset.max_splats = Some(lower);
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipes_and_memory_boundaries_are_exact() {
        for (quality, steps, cap, threshold, fraction, stop, face) in [
            (Quality::Fast, 12_000, 1_000_000, 0.0018, 0.20, 10_000, 512),
            (
                Quality::Balanced,
                30_000,
                2_000_000,
                0.0012,
                0.25,
                25_000,
                768,
            ),
            (Quality::High, 45_000, 3_000_000, 0.0010, 0.30, 38_000, 1024),
        ] {
            let r = resolve_panorama_training_preset(quality, false, None, face, 10_192).unwrap();
            assert_eq!(
                (
                    r.preset.total_steps,
                    r.preset.max_splats,
                    r.preset.refine_every
                ),
                (steps, Some(cap), 200)
            );
            assert_eq!(r.preset.max_resolution, face);
            assert_eq!(r.configured_max_splats, Some(cap));
            assert_eq!(r.panorama.unwrap().memory_max_splats, None);
            assert_eq!(
                r.preset.densification,
                Some(BrushDensificationPreset {
                    growth_grad_threshold: threshold,
                    growth_select_fraction: fraction,
                    growth_stop_iter: stop,
                })
            );
            assert!(r.downgrade_after_oom(10_192).is_none());
        }
        for (memory, caps) in [
            (None, [1_000_000, 1_000_000, 1_000_000]),
            (Some(0), [1_000_000, 1_000_000, 1_000_000]),
            (Some(8191), [1_000_000, 1_000_000, 1_000_000]),
            (Some(8192), [1_000_000, 2_000_000, 3_000_000]),
            (Some(12287), [1_000_000, 2_000_000, 3_000_000]),
            (Some(12288), [1_000_000, 3_000_000, 5_000_000]),
            (Some(16383), [1_000_000, 3_000_000, 5_000_000]),
            (Some(16384), [1_000_000, 5_000_000, 7_500_000]),
            (Some(24575), [1_000_000, 5_000_000, 7_500_000]),
            (Some(24576), [1_000_000, 5_000_000, 10_000_000]),
            (Some(u64::MAX), [1_000_000, 5_000_000, 10_000_000]),
        ] {
            for (index, quality) in [Quality::Fast, Quality::Balanced, Quality::High]
                .into_iter()
                .enumerate()
            {
                let r = resolve_panorama_training_preset(quality, true, memory, 1024, 0).unwrap();
                assert_eq!(r.preset.max_splats, Some(caps[index]));
                let manual =
                    resolve_panorama_training_preset(quality, false, memory, 1024, 0).unwrap();
                assert_eq!(
                    manual.preset.max_splats,
                    Some([1_000_000, 2_000_000, 3_000_000][index])
                );
            }
        }
        for (quality, expected) in [
            (Quality::Fast, 49),
            (Quality::Balanced, 124),
            (Quality::High, 189),
        ] {
            let r = resolve_panorama_training_preset(quality, false, None, 512, 10_192).unwrap();
            let stop = r.preset.densification.unwrap().growth_stop_iter;
            let opportunities = (1..stop)
                .filter(|i| {
                    i.is_multiple_of(r.preset.refine_every)
                        && *i as f64 <= r.preset.total_steps as f64 * 0.95
                })
                .count();
            assert_eq!(opportunities, expected);
        }
    }

    #[test]
    fn preserve_initial_points_but_never_exceed_ten_million() {
        let r =
            resolve_panorama_training_preset(Quality::Fast, true, None, 512, 3_000_000).unwrap();
        assert_eq!(r.preset.max_splats, Some(3_000_000));
        assert!(r.panorama.unwrap().initial_points_override);
        assert!(r.downgrade_after_oom(3_000_000).is_none());
        assert!(
            resolve_panorama_training_preset(Quality::High, false, None, 1024, 10_000_001)
                .unwrap_err()
                .to_string()
                .contains(INITIAL_POINT_BUDGET_ERROR)
        );
        assert_eq!(
            resolve_panorama_training_preset(Quality::Fast, true, None, 512, 10_000_000)
                .unwrap()
                .preset
                .max_splats,
            Some(10_000_000)
        );
    }

    #[test]
    fn retry_halves_only_the_cap_and_preserves_snapshot() {
        for quality in [Quality::Fast, Quality::Balanced, Quality::High] {
            let original =
                resolve_panorama_training_preset(quality, true, Some(24_576), 512, 10_192).unwrap();
            let mut expected = original;
            expected.preset.max_splats = original.preset.max_splats.map(|n| n / 2);
            assert_eq!(original.downgrade_after_oom(10_192), Some(expected));
            let serialized = serde_json::to_value(original).unwrap();
            let restored: ResolvedBrushTrainingPreset = serde_json::from_value(serialized).unwrap();
            assert_eq!(original, restored);
        }
        let original =
            resolve_panorama_training_preset(Quality::High, true, Some(24_576), 1024, 6_000_000)
                .unwrap();
        assert_eq!(
            original
                .downgrade_after_oom(6_000_000)
                .unwrap()
                .preset
                .max_splats,
            Some(6_000_000)
        );
        let eight_gib =
            resolve_panorama_training_preset(Quality::High, true, Some(8192), 1024, 10_192)
                .unwrap();
        assert_eq!(eight_gib.preset.max_splats, Some(3_000_000));
        assert_eq!(
            eight_gib
                .downgrade_after_oom(10_192)
                .unwrap()
                .preset
                .max_splats,
            Some(1_500_000)
        );
    }

    #[test]
    fn reject_invalid_recipes_and_default_old_config() {
        let mut config = PanoramaTrainingConfig::default();
        config.validate().unwrap();
        config.high.max_splats = 10_000_001;
        assert!(config.validate().is_err());
        config = PanoramaTrainingConfig::default();
        config.balanced.densification.growth_stop_iter = 30_000;
        assert!(config.validate().is_err());
        for cap in [0, 10_000_001] {
            let mut invalid = PanoramaTrainingConfig::default();
            invalid.high.max_splats_without_optimization = Some(cap);
            assert!(invalid.validate().is_err());
        }
        let mut config: serde_json::Value =
            serde_json::from_str(include_str!("../../../config/pipeline-optimization.json"))
                .unwrap();
        // Compare after typed decoding: JSON decimals and f32 serialization
        // have different textual precision but must resolve to identical args.
        let embedded: PanoramaTrainingConfig =
            serde_json::from_value(config["panoramaTraining"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(embedded).unwrap(),
            serde_json::to_value(PanoramaTrainingConfig::default()).unwrap()
        );
        config.as_object_mut().unwrap().remove("panoramaTraining");
        let restored: super::super::PipelineOptimizationConfig =
            serde_json::from_value(config).unwrap();
        assert_eq!(restored.panorama_training.high.max_splats, 10_000_000);
    }

    #[tokio::test]
    async fn old_external_recipes_get_manual_defaults_and_custom_caps_are_respected() {
        let mut old: serde_json::Value =
            serde_json::from_str(include_str!("../../../config/pipeline-optimization.json"))
                .unwrap();
        for name in ["fast", "balanced", "high"] {
            old["panoramaTraining"][name]
                .as_object_mut()
                .unwrap()
                .remove("maxSplatsWithoutOptimization");
        }
        let mut config: super::super::PipelineOptimizationConfig =
            serde_json::from_value(old).unwrap();
        config.panorama_training.validate().unwrap();
        super::super::with_pipeline_config(config.clone(), async {
            for (quality, expected) in [
                (Quality::Fast, 1_000_000),
                (Quality::Balanced, 2_000_000),
                (Quality::High, 3_000_000),
            ] {
                let manual =
                    resolve_panorama_training_preset(quality, false, None, 512, 0).unwrap();
                assert_eq!(manual.preset.max_splats, Some(expected));
            }
            let automatic =
                resolve_panorama_training_preset(Quality::High, true, Some(24576), 1024, 0)
                    .unwrap();
            assert_eq!(automatic.preset.max_splats, Some(10_000_000));
        })
        .await;
        config
            .panorama_training
            .high
            .max_splats_without_optimization = Some(2_500_000);
        super::super::with_pipeline_config(config, async {
            let manual =
                resolve_panorama_training_preset(Quality::High, false, None, 1024, 0).unwrap();
            assert_eq!(manual.configured_max_splats, Some(2_500_000));
            assert_eq!(manual.preset.max_splats, Some(2_500_000));
            let initialized =
                resolve_panorama_training_preset(Quality::High, false, None, 1024, 4_000_000)
                    .unwrap();
            assert_eq!(initialized.preset.max_splats, Some(4_000_000));
            assert!(initialized.panorama.unwrap().initial_points_override);
            assert!(initialized.downgrade_after_oom(4_000_000).is_none());
        })
        .await;
    }
}
