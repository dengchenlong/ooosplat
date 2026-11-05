//! Brush dataset assembly for the masked dual-training route (issue #8).
//!
//! Brush natively supports per-image masks: a `masks/` directory next to
//! `images/`, where black pixels are ignored and white pixels are kept
//! (`--invert-masks` flips that meaning). Route A needs two datasets built
//! from the *same* capture:
//!
//! - `dataset-object`: white = the object, so only the object is reconstructed;
//! - `dataset-background`: white = everything but the object.
//!
//! This module owns the assembly so the pipeline (and later the dual-training
//! orchestration in #9) never hand-copies frames. It also applies the mask
//! edge treatment route A calls for: a small erosion of the kept region, so
//! the ambiguous ring on the object silhouette is ignored by both datasets
//! instead of producing floaters in one of them.

use std::path::{Path, PathBuf};

use image::{GrayImage, ImageReader, Luma};

use crate::error::{Result, SplatError};

/// Historical single-dataset directory name, kept so an unmasked run produces
/// exactly the same layout as before this module existed.
pub const SINGLE_DATASET_NAME: &str = "dataset";

/// Which side of the per-frame mask a dataset should keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskPolarity {
    /// White mask pixels are the object: keep them.
    KeepObject,
    /// White mask pixels are the object: keep everything else.
    KeepBackground,
}

/// How a dataset's `masks/` directory should be derived from the sidecar masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskSpec {
    pub polarity: MaskPolarity,
    /// Erode the kept region by this many pixels so silhouettes are ignored by
    /// both datasets. 0 keeps every pixel the mask claims.
    pub erode_px: u8,
}

/// One dataset to assemble under the Brush work root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetSpec {
    /// Directory name under the Brush root, e.g. `dataset-object`.
    pub name: String,
    /// `None` assembles an unmasked dataset (the pre-existing behaviour).
    pub masks: Option<MaskSpec>,
}

impl DatasetSpec {
    pub fn plain(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            masks: None,
        }
    }

    pub fn masked(name: impl Into<String>, polarity: MaskPolarity, erode_px: u8) -> Self {
        Self {
            name: name.into(),
            masks: Some(MaskSpec { polarity, erode_px }),
        }
    }
}

/// Default route-A pair: object and background from one capture.
pub fn dual_dataset_specs(erode_px: u8) -> [DatasetSpec; 2] {
    [
        DatasetSpec::masked("dataset-object", MaskPolarity::KeepObject, erode_px),
        DatasetSpec::masked("dataset-background", MaskPolarity::KeepBackground, erode_px),
    ]
}

/// Assembles `root/<spec.name>` with `images/`, `sparse/0/` and — when the spec
/// is masked — `masks/<frame stem>.png`.
///
/// `masks_dir` holds one sidecar mask per extracted frame, named after the
/// frame stem (`frame_000001.png` for `frame_000001.jpg`). Brush resolves masks
/// by stem inside a `masks/` component, so the assembled files keep that name.
pub async fn prepare_dataset(
    root: &Path,
    frames: &Path,
    model: &Path,
    masks_dir: Option<&Path>,
    spec: &DatasetSpec,
) -> Result<PathBuf> {
    if spec.masks.is_some() && masks_dir.is_none() {
        return Err(SplatError::Process(format!(
            "数据集 {} 需要逐帧 mask，但没有提供 mask 目录",
            spec.name
        )));
    }
    let dataset = root.join(&spec.name);
    let images = dataset.join("images");
    let sparse = dataset.join("sparse").join("0");
    tokio::fs::create_dir_all(&images).await?;
    tokio::fs::create_dir_all(&sparse).await?;

    let mut names = Vec::new();
    let mut entries = tokio::fs::read_dir(frames).await?;
    while let Some(entry) = entries.next_entry().await? {
        let source = entry.path();
        if !source.is_file() {
            continue;
        }
        let destination = images.join(entry.file_name());
        if tokio::fs::hard_link(&source, &destination).await.is_err() {
            tokio::fs::copy(&source, &destination).await?;
        }
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    if names.is_empty() {
        return Err(SplatError::Process(format!(
            "训练图片目录为空：{}",
            frames.display()
        )));
    }
    for name in ["cameras.bin", "images.bin", "points3D.bin"] {
        tokio::fs::copy(model.join(name), sparse.join(name)).await?;
    }

    if let (Some(mask_spec), Some(masks_dir)) = (spec.masks, masks_dir) {
        let masks = dataset.join("masks");
        tokio::fs::create_dir_all(&masks).await?;
        for frame in &names {
            let stem = Path::new(frame)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .ok_or_else(|| SplatError::Process(format!("训练图片缺少文件名：{frame}")))?;
            let source = masks_dir.join(format!("{stem}.png"));
            if !source.is_file() {
                return Err(SplatError::Process(format!(
                    "缺少逐帧 mask：{}（数据集 {} 需要每帧一张）",
                    source.display(),
                    spec.name
                )));
            }
            write_mask(&source, &masks.join(format!("{stem}.png")), mask_spec).await?;
        }
    }

    Ok(dataset)
}

/// Assembles every spec in order, returning the dataset roots.
pub async fn prepare_datasets(
    root: &Path,
    frames: &Path,
    model: &Path,
    masks_dir: Option<&Path>,
    specs: &[DatasetSpec],
) -> Result<Vec<PathBuf>> {
    let mut datasets = Vec::with_capacity(specs.len());
    for spec in specs {
        datasets.push(prepare_dataset(root, frames, model, masks_dir, spec).await?);
    }
    Ok(datasets)
}

/// Reads one sidecar mask, applies polarity and erosion, writes it as an 8-bit
/// binary PNG (`0` or `255`) that Brush can consume directly.
pub async fn write_mask(source: &Path, destination: &Path, spec: MaskSpec) -> Result<()> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mask = read_gray(&source)?;
        let transformed = transform_mask(&mask, spec);
        transformed.save(&destination).map_err(|error| {
            SplatError::Process(format!(
                "写入 Mask 失败：{}：{error}",
                destination.display()
            ))
        })?;
        Ok(())
    })
    .await
    .map_err(|error| SplatError::Process(format!("Mask 处理线程失败：{error}")))?
}

fn read_gray(path: &Path) -> Result<GrayImage> {
    let decoded = ImageReader::open(path)
        .map_err(|error| SplatError::Process(format!("无法读取 Mask {}：{error}", path.display())))?
        .with_guessed_format()
        .map_err(|error| SplatError::Process(format!("无法识别 Mask {}：{error}", path.display())))?
        .decode()
        .map_err(|error| {
            SplatError::Process(format!("无法解码 Mask {}：{error}", path.display()))
        })?;
    Ok(decoded.into_luma8())
}

/// Polarity then erosion: `white` marks the *kept* pixels of the produced mask.
pub fn transform_mask(mask: &GrayImage, spec: MaskSpec) -> GrayImage {
    let inverted = spec.polarity == MaskPolarity::KeepBackground;
    let binarised = binarise(mask, inverted);
    if spec.erode_px == 0 {
        binarised
    } else {
        erode(&binarised, u32::from(spec.erode_px))
    }
}

/// Thresholds at 128 and optionally flips the meaning of white.
fn binarise(mask: &GrayImage, invert: bool) -> GrayImage {
    GrayImage::from_fn(mask.width(), mask.height(), |x, y| {
        let kept = mask.get_pixel(x, y)[0] >= 128;
        let kept = if invert { !kept } else { kept };
        Luma([if kept { 255 } else { 0 }])
    })
}

/// Chebyshev-radius erosion of the white region (a pixel survives only when
/// every neighbour within `radius` is white). Separable, so cost is O(w·h·r).
pub fn erode(mask: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 {
        return mask.clone();
    }
    let (width, height) = mask.dimensions();
    let radius = radius as i64;
    let mut horizontal = GrayImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut kept = true;
            let mut dx = -radius;
            while dx <= radius {
                let sx = x as i64 + dx;
                if sx < 0 || sx >= width as i64 || mask.get_pixel(sx as u32, y)[0] == 0 {
                    kept = false;
                    break;
                }
                dx += 1;
            }
            if kept {
                horizontal.put_pixel(x, y, Luma([255]));
            }
        }
    }
    let mut eroded = GrayImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut kept = true;
            let mut dy = -radius;
            while dy <= radius {
                let sy = y as i64 + dy;
                if sy < 0 || sy >= height as i64 || horizontal.get_pixel(x, sy as u32)[0] == 0 {
                    kept = false;
                    break;
                }
                dy += 1;
            }
            if kept {
                eroded.put_pixel(x, y, Luma([255]));
            }
        }
    }
    eroded
}

/// Count of pixels marked "keep" — used by the CLI to report mask coverage.
pub fn kept_pixels(mask: &GrayImage) -> u64 {
    mask.pixels().filter(|pixel| pixel[0] > 0).count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_frame(dir: &Path, stem: &str) {
        std::fs::create_dir_all(dir).unwrap();
        image::RgbImage::new(8, 8)
            .save(dir.join(format!("{stem}.jpg")))
            .unwrap();
    }

    fn write_sparse(model: &Path) {
        std::fs::create_dir_all(model).unwrap();
        for name in ["cameras.bin", "images.bin", "points3D.bin"] {
            std::fs::write(model.join(name), b"stub").unwrap();
        }
    }

    /// White square in the middle of a black 8x8 mask.
    fn square_mask(inset: u32, size: u32) -> GrayImage {
        GrayImage::from_fn(size, size, |x, y| {
            let inside = x >= inset && y >= inset && x < size - inset && y < size - inset;
            Luma([if inside { 255 } else { 0 }])
        })
    }

    #[test]
    fn background_polarity_inverts_the_kept_region() {
        let object = transform_mask(
            &square_mask(2, 8),
            MaskSpec {
                polarity: MaskPolarity::KeepObject,
                erode_px: 0,
            },
        );
        let background = transform_mask(
            &square_mask(2, 8),
            MaskSpec {
                polarity: MaskPolarity::KeepBackground,
                erode_px: 0,
            },
        );
        assert_eq!(object.get_pixel(4, 4)[0], 255);
        assert_eq!(background.get_pixel(4, 4)[0], 0);
        assert_eq!(object.get_pixel(0, 0)[0], 0);
        assert_eq!(background.get_pixel(0, 0)[0], 255);
        // With no erosion the two sides partition the frame exactly.
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    u16::from(object.get_pixel(x, y)[0]) + u16::from(background.get_pixel(x, y)[0]),
                    255,
                    "pixel ({x},{y}) is not partitioned"
                );
            }
        }
    }

    #[test]
    fn erosion_shrinks_the_kept_region_by_the_requested_radius() {
        let mask = square_mask(2, 8);
        let kept = kept_pixels(&erode(&mask, 1));
        // 4x4 square erodes to 2x2.
        assert_eq!(kept, 4);
        let kept_two = kept_pixels(&erode(&mask, 2));
        assert_eq!(kept_two, 0);
    }

    #[test]
    fn erosion_ignores_grey_noise_below_the_threshold() {
        let noisy = GrayImage::from_fn(4, 4, |_, _| Luma([127]));
        let transformed = transform_mask(
            &noisy,
            MaskSpec {
                polarity: MaskPolarity::KeepObject,
                erode_px: 1,
            },
        );
        assert_eq!(kept_pixels(&transformed), 0);
    }

    #[tokio::test]
    async fn assembled_object_dataset_links_frames_copies_model_and_renames_masks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("brush");
        let frames = dir.path().join("frames");
        let model = dir.path().join("sparse-model");
        let masks = dir.path().join("sidecar-masks");
        write_frame(&frames, "frame_000001");
        write_frame(&frames, "frame_000002");
        write_sparse(&model);
        std::fs::create_dir_all(&masks).unwrap();
        square_mask(1, 8)
            .save(masks.join("frame_000001.png"))
            .unwrap();
        square_mask(1, 8)
            .save(masks.join("frame_000002.png"))
            .unwrap();

        let dataset = prepare_dataset(
            &root,
            &frames,
            &model,
            Some(&masks),
            &DatasetSpec::masked("dataset-object", MaskPolarity::KeepObject, 0),
        )
        .await
        .unwrap();

        assert_eq!(dataset, root.join("dataset-object"));
        assert!(dataset.join("images/frame_000001.jpg").is_file());
        assert!(dataset.join("images/frame_000002.jpg").is_file());
        assert!(dataset.join("sparse/0/images.bin").is_file());
        // Brush resolves masks by stem inside a `masks/` component.
        let mask = read_gray(&dataset.join("masks/frame_000001.png")).unwrap();
        assert_eq!(mask.dimensions(), (8, 8));
        assert_eq!(kept_pixels(&mask), 36);
    }

    #[tokio::test]
    async fn dual_datasets_keep_complementary_regions_of_the_same_capture() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("brush");
        let frames = dir.path().join("frames");
        let model = dir.path().join("sparse-model");
        let masks = dir.path().join("sidecar-masks");
        write_frame(&frames, "frame_000001");
        write_sparse(&model);
        std::fs::create_dir_all(&masks).unwrap();
        square_mask(2, 8)
            .save(masks.join("frame_000001.png"))
            .unwrap();

        let datasets =
            prepare_datasets(&root, &frames, &model, Some(&masks), &dual_dataset_specs(0))
                .await
                .unwrap();

        assert_eq!(datasets.len(), 2);
        let object = read_gray(&root.join("dataset-object/masks/frame_000001.png")).unwrap();
        let background =
            read_gray(&root.join("dataset-background/masks/frame_000001.png")).unwrap();
        assert_eq!(kept_pixels(&object), 16);
        assert_eq!(kept_pixels(&background), 48);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    u16::from(object.get_pixel(x, y)[0]) + u16::from(background.get_pixel(x, y)[0]),
                    255
                );
            }
        }
    }

    #[tokio::test]
    async fn masked_dataset_without_masks_directory_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        let model = dir.path().join("sparse-model");
        write_frame(&frames, "frame_000001");
        write_sparse(&model);
        let error = prepare_dataset(
            &dir.path().join("brush"),
            &frames,
            &model,
            None,
            &DatasetSpec::masked("dataset-object", MaskPolarity::KeepObject, 0),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("需要逐帧 mask"));
    }

    #[tokio::test]
    async fn a_missing_per_frame_mask_fails_the_dataset() {
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        let model = dir.path().join("sparse-model");
        let masks = dir.path().join("sidecar-masks");
        write_frame(&frames, "frame_000001");
        write_frame(&frames, "frame_000002");
        write_sparse(&model);
        std::fs::create_dir_all(&masks).unwrap();
        square_mask(2, 8)
            .save(masks.join("frame_000001.png"))
            .unwrap();
        let error = prepare_dataset(
            &dir.path().join("brush"),
            &frames,
            &model,
            Some(&masks),
            &DatasetSpec::masked("dataset-object", MaskPolarity::KeepObject, 0),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("frame_000002"));
    }

    #[tokio::test]
    async fn plain_dataset_has_no_masks_directory() {
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        let model = dir.path().join("sparse-model");
        write_frame(&frames, "frame_000001");
        write_sparse(&model);
        let dataset = prepare_dataset(
            &dir.path().join("brush"),
            &frames,
            &model,
            None,
            &DatasetSpec::plain(SINGLE_DATASET_NAME),
        )
        .await
        .unwrap();
        assert!(dataset.join("images/frame_000001.jpg").is_file());
        assert!(!dataset.join("masks").exists());
    }
}
