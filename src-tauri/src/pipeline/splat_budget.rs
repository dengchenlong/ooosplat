//! Ordinary-image Detailed ceiling, adapted from TangoGS's static allowance.
//! This does not implement TangoGS's growth/pruning controller or promise a
//! final point count. ERP faces must not be paired with spherical tracks.
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Component, Path},
};

use image::{DynamicImage, ImageDecoder, ImageReader};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{
    error::{Result, SplatError},
    reconstruction::colmap_model::read_cameras,
};

pub const VERSION: u32 = 1;
pub const COEFFICIENT: f64 = 0.5;
const TANGO_SCALE: f64 = 2_462_476.0 * 0.063_362_106;
const DECODE_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FineSplatBudget {
    pub version: u32,
    pub coefficient: f64,
    pub registered_images: u64,
    pub effective_pixels: u64,
    pub sparse_points: u64,
    pub tracked_points: u64,
    pub observations: u64,
    pub mean_track_length: f64,
    pub requested_max_splats: u32,
}

impl FineSplatBudget {
    pub fn validated_ceiling(self, sparse_points: u64) -> Result<u32> {
        if self.version != VERSION
            || self.coefficient != COEFFICIENT
            || self.sparse_points != sparse_points
            || self.tracked_points > sparse_points
            || self.registered_images == 0
            || !self.mean_track_length.is_finite()
            || (self.mean_track_length - self.observations as f64 / self.tracked_points as f64)
                .abs()
                > 1e-12 * self.mean_track_length.abs().max(1.)
            || self.requested_max_splats
                != ceiling(
                    self.effective_pixels,
                    self.tracked_points,
                    self.observations,
                )?
        {
            return Err(invalid());
        }
        Ok(self.requested_max_splats)
    }
}

fn invalid() -> SplatError {
    SplatError::Process("Cannot calculate Detailed Splat budget from invalid training data".into())
}

fn check(cancel: &CancellationToken) -> Result<()> {
    if cancel.is_cancelled() {
        Err(SplatError::Cancelled)
    } else {
        Ok(())
    }
}

fn bytes<const N: usize>(reader: &mut impl Read) -> Result<[u8; N]> {
    let mut value = [0; N];
    reader.read_exact(&mut value)?;
    Ok(value)
}

struct Image {
    id: u32,
    camera: u32,
    name: String,
    observations: u64,
}

fn images(model: &Path, cancel: &CancellationToken) -> Result<Vec<Image>> {
    let path = model.join("images.bin");
    let mut result = Vec::new();
    if path.is_file() {
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        let mut reader = BufReader::new(file);
        let count = u64::from_le_bytes(bytes(&mut reader)?);
        if count == 0 || count > 1_000_000 || count > length / 73 {
            return Err(invalid());
        }
        for _ in 0..count {
            check(cancel)?;
            let head: [u8; 64] = bytes(&mut reader)?;
            let mut name = Vec::new();
            loop {
                let [byte] = bytes(&mut reader)?;
                if byte == 0 {
                    break;
                }
                name.push(byte);
                if name.len() > 32 * 1024 {
                    return Err(invalid());
                }
            }
            let observations = u64::from_le_bytes(bytes(&mut reader)?);
            let end = observations
                .checked_mul(24)
                .and_then(|n| reader.stream_position().ok()?.checked_add(n))
                .filter(|end| *end <= length)
                .ok_or_else(invalid)?;
            // Do not read or allocate all 2D features just to count tracks.
            reader.seek(SeekFrom::Start(end))?;
            result.push(Image {
                id: u32::from_le_bytes(head[..4].try_into().unwrap()),
                camera: u32::from_le_bytes(head[60..].try_into().unwrap()),
                name: String::from_utf8(name).map_err(|_| invalid())?,
                observations,
            });
        }
        if reader.stream_position()? != length {
            return Err(invalid());
        }
    } else {
        let mut lines = BufReader::new(File::open(model.join("images.txt"))?).lines();
        while let Some(line) = lines.next() {
            check(cancel)?;
            let line = line?;
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 10 {
                return Err(invalid());
            }
            let observations = lines.next().ok_or_else(invalid)??;
            let count = observations.split_whitespace().count();
            if count % 3 != 0 {
                return Err(invalid());
            }
            result.push(Image {
                id: fields[0].parse().map_err(|_| invalid())?,
                camera: fields[8].parse().map_err(|_| invalid())?,
                name: fields[9..].join(" "),
                observations: (count / 3) as u64,
            });
        }
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    if result.is_empty()
        || result
            .iter()
            .any(|i| i.id == 0 || !ids.insert(i.id) || i.name.is_empty() || !names.insert(&i.name))
    {
        return Err(invalid());
    }
    Ok(result)
}

fn tracks(model: &Path, images: &[Image], cancel: &CancellationToken) -> Result<(u64, u64, u64)> {
    let registered: HashMap<_, _> = images.iter().map(|i| (i.id, i.observations)).collect();
    let mut points = 0_u64;
    let mut tracked = 0_u64;
    let mut observations = 0_u64;
    let mut seen = HashSet::new();
    let mut add = |entries: &[(u32, u32)]| -> Result<()> {
        seen.clear();
        let valid = entries
            .iter()
            .filter(|(id, index)| {
                registered.get(id).is_some_and(|n| u64::from(*index) < *n) && seen.insert(*id)
            })
            .count() as u64;
        points = points.checked_add(1).ok_or_else(invalid)?;
        if valid > 0 {
            tracked = tracked.checked_add(1).ok_or_else(invalid)?;
            observations = observations.checked_add(valid).ok_or_else(invalid)?;
        }
        Ok(())
    };
    let path = model.join("points3D.bin");
    if path.is_file() {
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        let mut reader = BufReader::new(file);
        let count = u64::from_le_bytes(bytes(&mut reader)?);
        if count == 0 || count > length.saturating_sub(8) / 51 {
            return Err(invalid());
        }
        let mut entries = Vec::new();
        for _ in 0..count {
            check(cancel)?;
            let head: [u8; 51] = bytes(&mut reader)?;
            let count = u64::from_le_bytes(head[43..].try_into().unwrap());
            if count > 1_000_000 || count > length / 8 {
                return Err(invalid());
            }
            entries.clear();
            for _ in 0..count {
                entries.push((
                    u32::from_le_bytes(bytes(&mut reader)?),
                    u32::from_le_bytes(bytes(&mut reader)?),
                ));
            }
            add(&entries)?;
        }
        if reader.stream_position()? != length {
            return Err(invalid());
        }
    } else {
        for line in BufReader::new(File::open(model.join("points3D.txt"))?).lines() {
            check(cancel)?;
            let line = line?;
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 8 || (fields.len() - 8) % 2 != 0 {
                return Err(invalid());
            }
            let entries = fields[8..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| {
                    Ok((
                        v[0].parse().map_err(|_| invalid())?,
                        v[1].parse().map_err(|_| invalid())?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            add(&entries)?;
        }
    }
    if points == 0 || tracked == 0 || observations == 0 {
        return Err(invalid());
    }
    Ok((points, tracked, observations))
}

/// Count Alpha > 0, not COLMAP's Alpha >= 0.5 feature mask. The latter would
/// incorrectly remove semi-transparent pixels that still participate in Brush.
fn effective_pixels(path: &Path, cancel: &CancellationToken) -> Result<((u32, u32), u64)> {
    check(cancel)?;
    let decoder = ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()
        .map_err(|e| SplatError::Process(e.to_string()))?;
    let dimensions = decoder.dimensions();
    let total = u64::from(dimensions.0) * u64::from(dimensions.1);
    if total == 0 || decoder.total_bytes() > DECODE_LIMIT {
        return Err(invalid());
    }
    if !decoder.color_type().has_alpha() {
        return Ok((dimensions, total));
    }
    let decoded =
        DynamicImage::from_decoder(decoder).map_err(|e| SplatError::Process(e.to_string()))?;
    let mut count = 0_u64;
    macro_rules! count {
        ($buffer:expr, $channels:expr, $alpha:expr) => {
            for chunk in $buffer.as_raw().chunks(8192 * $channels) {
                check(cancel)?;
                count += chunk
                    .chunks_exact($channels)
                    .filter(|p| p[$alpha] > 0)
                    .count() as u64;
            }
        };
    }
    match decoded {
        DynamicImage::ImageRgba8(v) => count!(v, 4, 3),
        DynamicImage::ImageLumaA8(v) => count!(v, 2, 1),
        DynamicImage::ImageRgba16(v) => count!(v, 4, 3),
        DynamicImage::ImageLumaA16(v) => count!(v, 2, 1),
        _ => return Err(invalid()),
    }
    Ok((dimensions, count))
}

pub fn ceiling(effective_pixels: u64, tracked: u64, observations: u64) -> Result<u32> {
    if effective_pixels == 0 || tracked == 0 || observations < tracked {
        return Err(invalid());
    }
    let value = (COEFFICIENT
        * (TANGO_SCALE * effective_pixels as f64 * tracked as f64 / observations as f64).sqrt())
    .round();
    if !value.is_finite() || value < 1.0 || value > f64::from(u32::MAX) {
        return Err(invalid());
    }
    Ok(value as u32)
}

/// Called only after reconstruction, off the async executor. Examine every
/// registered training view, sequentially, without modifying files or sampling.
pub fn calculate(
    model: &Path,
    image_root: &Path,
    cancel: &CancellationToken,
) -> Result<FineSplatBudget> {
    check(cancel)?;
    let cameras = read_cameras(model)?;
    if cameras.iter().any(|c| c.model_id == 17) {
        return Err(invalid());
    }
    let images = images(model, cancel)?;
    let (points, tracked, observations) = tracks(model, &images, cancel)?;
    let root = image_root.canonicalize()?;
    let mut pixels = 0_u64;
    for image in &images {
        check(cancel)?;
        let relative = Path::new(&image.name);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(invalid());
        }
        let path = root.join(relative).canonicalize()?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(invalid());
        }
        let (dimensions, valid) = effective_pixels(&path, cancel)?;
        let camera = cameras
            .iter()
            .find(|c| c.id == image.camera)
            .ok_or_else(invalid)?;
        if (u64::from(dimensions.0), u64::from(dimensions.1)) != (camera.width, camera.height) {
            return Err(invalid());
        }
        pixels = pixels.checked_add(valid).ok_or_else(invalid)?;
    }
    Ok(FineSplatBudget {
        version: VERSION,
        coefficient: COEFFICIENT,
        registered_images: images.len() as u64,
        effective_pixels: pixels,
        sparse_points: points,
        tracked_points: tracked,
        observations,
        mean_track_length: observations as f64 / tracked as f64,
        requested_max_splats: ceiling(pixels, tracked, observations)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn historical_formula_matches_the_confirmed_half_coefficient() {
        assert_eq!(ceiling(364_993_345, 66_218, 191_175).unwrap(), 2_220_679);
        assert_eq!(ceiling(56_578_232, 34_842, 202_872).unwrap(), 615_653);
        for inputs in [(0, 1, 1), (1, 0, 1), (1, 2, 1), (u64::MAX, 1, 1)] {
            assert!(ceiling(inputs.0, inputs.1, inputs.2).is_err());
        }
    }

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let model = temp.path().join("model");
        let root = temp.path().join("images");
        std::fs::create_dir_all(&model).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            model.join("cameras.txt"),
            "1 SIMPLE_RADIAL 4 2 2 2 1 0\n2 SIMPLE_RADIAL 2 2 2 1 1 0\n",
        )
        .unwrap();
        std::fs::write(
            model.join("images.txt"),
            "1 1 0 0 0 0 0 0 1 a.png\n0 0 1 1 0 2\n2 1 0 0 0 0 0 0 2 b.png\n0 0 1 1 0 2\n",
        )
        .unwrap();
        std::fs::write(model.join("points3D.txt"), "1 0 0 1 255 255 255 0 1 0 2 0 999 0 1 1\n2 0 1 1 255 255 255 0 1 1 2 1 2 99\n3 1 1 1 255 255 255 0 999 0\n").unwrap();
        image::RgbaImage::from_fn(4, 2, |x, y| {
            image::Rgba([
                10,
                20,
                30,
                [0, 1, 127, 128, 255, 255, 0, 0][(y * 4 + x) as usize],
            ])
        })
        .save(root.join("a.png"))
        .unwrap();
        image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(2, 2, |x, y| {
            image::Rgba([10, 20, 30, [0, 1, 32767, 65535][(y * 2 + x) as usize]])
        })
        .save(root.join("b.png"))
        .unwrap();
        // Unregistered files do not contribute and need not even decode.
        std::fs::write(root.join("unused.png"), b"unused").unwrap();
        (temp, model, root)
    }

    #[test]
    fn all_registered_pixels_are_counted_with_native_alpha_and_valid_tracks_only() {
        let (_temp, model, root) = fixture();
        let before = std::fs::read(root.join("a.png")).unwrap();
        let budget = calculate(&model, &root, &CancellationToken::new()).unwrap();
        assert_eq!(budget.effective_pixels, 8);
        assert_eq!(budget.registered_images, 2);
        assert_eq!(
            (
                budget.sparse_points,
                budget.tracked_points,
                budget.observations
            ),
            (3, 2, 4)
        );
        assert_eq!(budget.mean_track_length, 2.);
        assert_eq!(std::fs::read(root.join("a.png")).unwrap(), before);
        let restored: FineSplatBudget =
            serde_json::from_str(&serde_json::to_string(&budget).unwrap()).unwrap();
        assert_eq!(
            restored.validated_ceiling(3).unwrap(),
            budget.requested_max_splats
        );
        assert!(restored.validated_ceiling(4).is_err());
        let mut changed = restored;
        changed.coefficient = 0.6;
        assert!(changed.validated_ceiling(3).is_err());
    }

    #[test]
    fn binary_and_text_models_use_the_same_track_definition() {
        let (_temp, model, root) = fixture();
        let expected = calculate(&model, &root, &CancellationToken::new()).unwrap();
        let mut images = File::create(model.join("images.bin")).unwrap();
        images.write_all(&2_u64.to_le_bytes()).unwrap();
        for (id, name) in [(1_u32, "a.png"), (2, "b.png")] {
            images.write_all(&id.to_le_bytes()).unwrap();
            images.write_all(&[0; 56]).unwrap();
            images.write_all(&id.to_le_bytes()).unwrap();
            images.write_all(name.as_bytes()).unwrap();
            images.write_all(&[0]).unwrap();
            images.write_all(&2_u64.to_le_bytes()).unwrap();
            images.write_all(&[0; 48]).unwrap();
        }
        drop(images);
        let mut points = File::create(model.join("points3D.bin")).unwrap();
        points.write_all(&3_u64.to_le_bytes()).unwrap();
        for entries in [
            vec![(1_u32, 0_u32), (2, 0), (999, 0), (1, 1)],
            vec![(1, 1), (2, 1), (2, 99)],
            vec![(999, 0)],
        ] {
            points.write_all(&[0; 43]).unwrap();
            points
                .write_all(&(entries.len() as u64).to_le_bytes())
                .unwrap();
            for (image, index) in entries {
                points.write_all(&image.to_le_bytes()).unwrap();
                points.write_all(&index.to_le_bytes()).unwrap();
            }
        }
        drop(points);
        assert_eq!(
            calculate(&model, &root, &CancellationToken::new()).unwrap(),
            expected
        );
        File::options()
            .write(true)
            .open(model.join("points3D.bin"))
            .unwrap()
            .set_len(60)
            .unwrap();
        assert!(calculate(&model, &root, &CancellationToken::new()).is_err());
    }

    #[test]
    fn opaque_pixels_are_not_inferred_from_rgb_and_luma_alpha_is_supported() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.png");
        let cancel = CancellationToken::new();
        image::RgbImage::new(3, 2).save(&path).unwrap();
        assert_eq!(effective_pixels(&path, &cancel).unwrap().1, 6);
        image::ImageBuffer::<image::LumaA<u8>, Vec<u8>>::from_raw(3, 1, vec![0, 0, 0, 1, 0, 255])
            .unwrap()
            .save(&path)
            .unwrap();
        assert_eq!(effective_pixels(&path, &cancel).unwrap().1, 2);
        image::ImageBuffer::<image::LumaA<u16>, Vec<u16>>::from_raw(
            3,
            1,
            vec![0, 0, 0, 1, 0, 65535],
        )
        .unwrap()
        .save(&path)
        .unwrap();
        assert_eq!(effective_pixels(&path, &cancel).unwrap().1, 2);
        cancel.cancel();
        assert!(matches!(
            effective_pixels(&path, &cancel),
            Err(SplatError::Cancelled)
        ));
    }

    #[test]
    fn projection_missing_images_dimensions_paths_and_cancellation_are_guarded() {
        let (_temp, model, root) = fixture();
        let cancel = CancellationToken::new();
        std::fs::write(model.join("cameras.txt"), "1 EQUIRECTANGULAR 4 2 4 2\n").unwrap();
        assert!(calculate(&model, &root, &cancel).is_err());
        std::fs::write(model.join("cameras.txt"), "1 SIMPLE_RADIAL 2 2 2 1 1 0\n").unwrap();
        assert!(calculate(&model, &root, &cancel).is_err());
        std::fs::write(
            model.join("images.txt"),
            "1 1 0 0 0 0 0 0 1 ../a.png\n0 0 1\n",
        )
        .unwrap();
        assert!(calculate(&model, &root, &cancel).is_err());
        std::fs::write(
            model.join("images.txt"),
            "1 1 0 0 0 0 0 0 1 missing.png\n0 0 1\n",
        )
        .unwrap();
        assert!(calculate(&model, &root, &cancel).is_err());
        cancel.cancel();
        assert!(matches!(
            calculate(&model, &root, &cancel),
            Err(SplatError::Cancelled)
        ));
    }

    #[test]
    #[ignore = "read-only check against the user's existing 90-view native-resolution project"]
    fn real_history_budget_matches_the_previously_calculated_ceiling() {
        let project = Path::new("E:/GaussianSplatting/test/20261027-085019_images");
        let state_before = std::fs::read(project.join("state.json")).unwrap();
        let model = project.join("work/brush/dataset/sparse/0");
        let images = project.join("work/brush/dataset/images");
        let budget = calculate(&model, &images, &CancellationToken::new()).unwrap();
        println!("{}", serde_json::to_string(&budget).unwrap());
        assert_eq!(budget.effective_pixels, 364_993_345);
        assert_eq!(budget.requested_max_splats, 2_220_679);
        assert_eq!(budget.registered_images, 90);
        assert_eq!(
            std::fs::read(project.join("state.json")).unwrap(),
            state_before
        );
    }
}
