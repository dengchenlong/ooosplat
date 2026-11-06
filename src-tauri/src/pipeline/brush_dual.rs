//! Route-A dual training orchestration (issue #9).
//!
//! Route A trains twice from the same capture: once with the object masks and
//! once with the background masks. Brush supports this natively (a `masks/`
//! directory next to `images/`), so the trainer stays untouched — what this
//! module owns is the *sequencing*: one job per assembled dataset, each with
//! its own output directory and export name, plus a single place that decides
//! what "both sides succeeded" means.
//!
//! The two runs share the GPU, so they are sequential by design: running them
//! concurrently on a 4GB card would just produce an out-of-memory classification
//! from one of them.

use std::path::{Path, PathBuf};

use crate::error::{Result, SplatError};

/// One side of the split: where its dataset lives and where its PLY goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DualJob {
    /// Dataset directory (must contain `images/`, `sparse/0/` and `masks/`).
    pub dataset: PathBuf,
    /// Directory the trained PLY is written into.
    pub output_directory: PathBuf,
    /// File name inside `output_directory`, without a `.ply` suffix.
    pub export_name: String,
    /// Which side of the split this job trains.
    pub polarity: SidePolarity,
}

/// Object or background. Kept separate from [`super::brush_datasets::MaskPolarity`]
/// because the dataset already has its masks baked with the right polarity — the
/// trainer only needs to label which side it is training.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePolarity {
    Object,
    Background,
}

impl SidePolarity {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Background => "background",
        }
    }

    /// Brush writes the background PLY under its own name so the two runs never
    /// collide in the same directory.
    pub const fn export_name(self) -> &'static str {
        match self {
            Self::Object => "object.ply.tmp",
            Self::Background => "background.ply.tmp",
        }
    }
}

/// Where both sides end up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DualTrainingOutcome {
    pub object: PathBuf,
    pub background: PathBuf,
}

/// Builds the two jobs for a route-A pair. `datasets` must be the object and
/// background dataset roots in that order (the order
/// [`super::brush_datasets::dual_dataset_specs`] produces).
pub fn plan_dual_training(
    datasets: &[PathBuf],
    output_root: &Path,
    object_dir: &str,
    background_dir: &str,
) -> Result<[DualJob; 2]> {
    if datasets.len() != 2 {
        return Err(SplatError::Process(format!(
            "双训练需要两个数据集（object/background），实际拿到 {} 个",
            datasets.len()
        )));
    }
    let mut jobs = [
        DualJob {
            dataset: datasets[0].clone(),
            output_directory: output_root.join(object_dir),
            export_name: SidePolarity::Object.export_name().to_string(),
            polarity: SidePolarity::Object,
        },
        DualJob {
            dataset: datasets[1].clone(),
            output_directory: output_root.join(background_dir),
            export_name: SidePolarity::Background.export_name().to_string(),
            polarity: SidePolarity::Background,
        },
    ];
    for job in &mut jobs {
        if !job.dataset.join("images").is_dir() {
            return Err(SplatError::Process(format!(
                "{} 数据集缺少 images/：{}",
                job.polarity.label(),
                job.dataset.display()
            )));
        }
        if !job.dataset.join("masks").is_dir() {
            return Err(SplatError::Process(format!(
                "{} 数据集缺少 masks/：{}（masked 双训练需要逐帧 mask）",
                job.polarity.label(),
                job.dataset.display()
            )));
        }
    }
    Ok(jobs)
}

/// Runs both jobs in order through `run_one`, which performs the actual training
/// call. Stopping at the first failure keeps the second side from burning GPU
/// time on a split whose other half is already broken.
pub fn run_dual_training<F>(jobs: &[DualJob; 2], mut run_one: F) -> Result<DualTrainingOutcome>
where
    F: FnMut(&DualJob) -> Result<PathBuf>,
{
    let object = run_one(&jobs[0])
        .map_err(|error| annotate(SidePolarity::Object, &jobs[0].dataset, error))?;
    let background = run_one(&jobs[1])
        .map_err(|error| annotate(SidePolarity::Background, &jobs[1].dataset, error))?;
    Ok(DualTrainingOutcome { object, background })
}

fn annotate(side: SidePolarity, dataset: &Path, error: SplatError) -> SplatError {
    SplatError::Process(format!(
        "{} 训练失败（数据集 {}）：{error}",
        side.label(),
        dataset.display()
    ))
}

/// Verifies both exports exist and are Gaussian PLY files. Route A publishes
/// these as derived assets, so a run that reports success but leaves a missing
/// or truncated file must not reach the caller.
pub fn verify_dual_outputs(outcome: &DualTrainingOutcome) -> Result<()> {
    for (side, path) in [
        (SidePolarity::Object, &outcome.object),
        (SidePolarity::Background, &outcome.background),
    ] {
        if !path.is_file() {
            return Err(SplatError::Process(format!(
                "{} 训练产物不存在：{}",
                side.label(),
                path.display()
            )));
        }
        crate::reconstruction::ply::inspect_gaussian_ply(path).map_err(|error| {
            SplatError::Process(format!(
                "{} 训练产物不是有效的 Gaussian PLY：{}：{error}",
                side.label(),
                path.display()
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dataset(root: &Path, name: &str, with_masks: bool) -> PathBuf {
        let dataset = root.join(name);
        std::fs::create_dir_all(dataset.join("images")).unwrap();
        std::fs::create_dir_all(dataset.join("sparse").join("0")).unwrap();
        if with_masks {
            std::fs::create_dir_all(dataset.join("masks")).unwrap();
        }
        dataset
    }

    #[test]
    fn plan_gives_each_side_its_own_directory_and_export_name() {
        let dir = tempfile::tempdir().unwrap();
        let datasets = vec![
            dataset(dir.path(), "dataset-object", true),
            dataset(dir.path(), "dataset-background", true),
        ];
        let jobs = plan_dual_training(
            &datasets,
            &dir.path().join("brush"),
            "dataset-object",
            "dataset-background",
        )
        .unwrap();
        assert_eq!(jobs[0].polarity, SidePolarity::Object);
        assert_eq!(jobs[0].export_name, "object.ply.tmp");
        assert_eq!(
            jobs[0].output_directory,
            dir.path().join("brush").join("dataset-object")
        );
        assert_eq!(jobs[1].polarity, SidePolarity::Background);
        assert_eq!(jobs[1].export_name, "background.ply.tmp");
        assert_ne!(jobs[0].export_name, jobs[1].export_name);
    }

    #[test]
    fn plan_rejects_a_pair_without_masks() {
        let dir = tempfile::tempdir().unwrap();
        let datasets = vec![
            dataset(dir.path(), "dataset-object", false),
            dataset(dir.path(), "dataset-background", true),
        ];
        let error = plan_dual_training(
            &datasets,
            dir.path(),
            "dataset-object",
            "dataset-background",
        )
        .unwrap_err();
        assert!(error.to_string().contains("masks/"));
    }

    #[test]
    fn plan_rejects_the_wrong_number_of_datasets() {
        let dir = tempfile::tempdir().unwrap();
        let datasets = vec![dataset(dir.path(), "dataset-object", true)];
        let error = plan_dual_training(
            &datasets,
            dir.path(),
            "dataset-object",
            "dataset-background",
        )
        .unwrap_err();
        assert!(error.to_string().contains("两个数据集"));
    }

    #[test]
    fn both_sides_run_in_order_and_return_their_exports() {
        let dir = tempfile::tempdir().unwrap();
        let datasets = vec![
            dataset(dir.path(), "dataset-object", true),
            dataset(dir.path(), "dataset-background", true),
        ];
        let jobs = plan_dual_training(
            &datasets,
            &dir.path().join("brush"),
            "dataset-object",
            "dataset-background",
        )
        .unwrap();
        let mut seen: Vec<&'static str> = Vec::new();
        let outcome = run_dual_training(&jobs, |job| {
            seen.push(job.polarity.label());
            Ok(job.output_directory.join(&job.export_name))
        })
        .unwrap();
        assert_eq!(seen, vec!["object", "background"]);
        assert!(outcome.object.ends_with("object.ply.tmp"));
        assert!(outcome.background.ends_with("background.ply.tmp"));
    }

    #[test]
    fn a_failed_object_side_stops_before_training_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let datasets = vec![
            dataset(dir.path(), "dataset-object", true),
            dataset(dir.path(), "dataset-background", true),
        ];
        let jobs = plan_dual_training(
            &datasets,
            dir.path(),
            "dataset-object",
            "dataset-background",
        )
        .unwrap();
        let mut calls = 0;
        let error = run_dual_training(&jobs, |_| {
            calls += 1;
            Err(SplatError::Process("boom".into()))
        })
        .unwrap_err();
        assert_eq!(calls, 1, "the second side must not run after a failure");
        assert!(error.to_string().contains("object 训练失败"));
    }

    #[test]
    fn verify_rejects_a_missing_export() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = DualTrainingOutcome {
            object: dir.path().join("object.ply.tmp"),
            background: dir.path().join("background.ply.tmp"),
        };
        assert!(verify_dual_outputs(&outcome).is_err());
    }

    #[test]
    fn verify_accepts_two_valid_plies() {
        let dir = tempfile::tempdir().unwrap();
        let header = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nproperty float f_dc_0\nproperty float opacity\nproperty float scale_0\nproperty float rot_0\nend_header\n";
        let mut object = header.to_vec();
        let mut background = header.to_vec();
        object.extend_from_slice(&[0_u8; 28]);
        background.extend_from_slice(&[0_u8; 28]);
        let object_path = dir.path().join("object.ply.tmp");
        let background_path = dir.path().join("background.ply.tmp");
        std::fs::write(&object_path, &object).unwrap();
        std::fs::write(&background_path, &background).unwrap();
        verify_dual_outputs(&DualTrainingOutcome {
            object: object_path,
            background: background_path,
        })
        .unwrap();
    }
}
