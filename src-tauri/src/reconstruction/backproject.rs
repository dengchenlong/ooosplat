use std::path::Path;

use crate::error::{Result, SplatError};
use crate::reconstruction::colmap_model::{CameraIntrinsics, ImagePose};
use crate::reconstruction::ply::stream_splat_cores;

/// Pinhole (+ optional distortion) projection for one COLMAP camera.
///
/// Distortion coefficients are honoured for SIMPLE_RADIAL / RADIAL / OPENCV
/// models; purely pinhole models (SIMPLE_PINHOLE / PINHOLE) carry none.
/// FOV / fisheye / full-opencv models are rejected: the shooting pipeline
/// configures SIMPLE_RADIAL or PINHOLE, and back-projection voting tolerates
/// small model mismatches through footprint dilation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraProjection {
    fx: f64,
    fy: f64,
    cx: f64,
    cy: f64,
    k1: f64,
    k2: f64,
    p1: f64,
    p2: f64,
}

impl CameraProjection {
    pub fn from_intrinsics(intrinsics: &CameraIntrinsics) -> Result<Self> {
        let params = intrinsics.params.as_slice();
        let projection = match intrinsics.camera.model_id {
            // SIMPLE_PINHOLE: f, cx, cy.
            0 => Self::undistorted(
                Self::param(params, 0)?,
                Self::param(params, 0)?,
                Self::param(params, 1)?,
                Self::param(params, 2)?,
            ),
            // PINHOLE: fx, fy, cx, cy.
            1 => Self::undistorted(
                Self::param(params, 0)?,
                Self::param(params, 1)?,
                Self::param(params, 2)?,
                Self::param(params, 3)?,
            ),
            // SIMPLE_RADIAL: f, cx, cy, k.
            2 => Self {
                k1: Self::param(params, 3)?,
                ..Self::undistorted(
                    Self::param(params, 0)?,
                    Self::param(params, 0)?,
                    Self::param(params, 1)?,
                    Self::param(params, 2)?,
                )
            },
            // RADIAL: f, cx, cy, k1, k2.
            3 => Self {
                k1: Self::param(params, 3)?,
                k2: Self::param(params, 4)?,
                ..Self::undistorted(
                    Self::param(params, 0)?,
                    Self::param(params, 0)?,
                    Self::param(params, 1)?,
                    Self::param(params, 2)?,
                )
            },
            // OPENCV: fx, fy, cx, cy, k1, k2, p1, p2.
            4 => Self {
                fx: Self::param(params, 0)?,
                fy: Self::param(params, 1)?,
                cx: Self::param(params, 2)?,
                cy: Self::param(params, 3)?,
                k1: Self::param(params, 4)?,
                k2: Self::param(params, 5)?,
                p1: Self::param(params, 6)?,
                p2: Self::param(params, 7)?,
            },
            _ => {
                return Err(SplatError::Process(format!(
                    "反投影投票暂不支持相机模型 {}",
                    intrinsics.camera.model
                )))
            }
        };
        Ok(projection)
    }

    fn undistorted(fx: f64, fy: f64, cx: f64, cy: f64) -> Self {
        Self {
            fx,
            fy,
            cx,
            cy,
            k1: 0.0,
            k2: 0.0,
            p1: 0.0,
            p2: 0.0,
        }
    }

    fn param(params: &[f64], index: usize) -> Result<f64> {
        params
            .get(index)
            .copied()
            .ok_or_else(|| SplatError::Process("COLMAP 相机内参参数数量与模型不匹配".into()))
    }

    /// Projects a world-space point into pixel coordinates.
    ///
    /// Returns `None` when the point lies on or behind the camera plane.
    pub fn world_to_pixel(&self, pose: &ImagePose, point: [f64; 3]) -> Option<[f64; 2]> {
        let cam = world_to_camera(pose, point);
        if cam[2] <= 0.0 {
            return None;
        }
        let xu = cam[0] / cam[2];
        let yu = cam[1] / cam[2];
        let r2 = xu * xu + yu * yu;
        let radial = 1.0 + self.k1 * r2 + self.k2 * r2 * r2;
        let xd = xu * radial + 2.0 * self.p1 * xu * yu + self.p2 * (r2 + 2.0 * xu * xu);
        let yd = yu * radial + self.p1 * (r2 + 2.0 * yu * yu) + 2.0 * self.p2 * xu * yu;
        Some([self.fx * xd + self.cx, self.fy * yd + self.cy])
    }

    /// Camera-space depth of a world point (distance along the optical axis).
    pub fn depth(&self, pose: &ImagePose, point: [f64; 3]) -> f64 {
        world_to_camera(pose, point)[2]
    }

    /// Approximate splat footprint radius in pixels at the given camera-space
    /// depth. 3DGS PLY stores scales as logarithms, so `exp` recovers the
    /// world-space standard deviation; the largest axis dominates the disk.
    pub fn footprint_radius_px(&self, scale_log: [f64; 3], depth: f64) -> f64 {
        let sigma = scale_log
            .iter()
            .map(|value| value.exp())
            .fold(0.0_f64, f64::max);
        sigma * self.fx / depth
    }
}

/// Applies the COLMAP world-to-camera transform: rotate by the pose quaternion
/// (w, x, y, z) and translate by `tvec`.
pub fn world_to_camera(pose: &ImagePose, point: [f64; 3]) -> [f64; 3] {
    let [qw, qx, qy, qz] = pose.quaternion;
    let [x, y, z] = point;
    // Rotation q * v * q^-1 via the cross-product form.
    let tx = 2.0 * (qy * z - qz * y);
    let ty = 2.0 * (qz * x - qx * z);
    let tz = 2.0 * (qx * y - qy * x);
    [
        x + qw * tx + (qy * tz - qz * ty) + pose.translation[0],
        y + qw * ty + (qz * tx - qx * tz) + pose.translation[1],
        z + qw * tz + (qx * ty - qy * tx) + pose.translation[2],
    ]
}

/// One frame's binary object mask, packed row-major (bit set = object pixel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMask {
    width: u32,
    height: u32,
    bits: Vec<u8>,
}

impl ObjectMask {
    /// Builds a mask from an occupancy predicate (handy for tests and for
    /// synthetic scenes; the sidecar PNG loader is the production path).
    pub fn from_occupied(width: u32, height: u32, occupied: impl Fn(u32, u32) -> bool) -> Self {
        let mut bits = vec![0_u8; (width as usize * height as usize).div_ceil(8)];
        for y in 0..height {
            for x in 0..width {
                if occupied(x, y) {
                    let index = y as usize * width as usize + x as usize;
                    bits[index / 8] |= 1 << (index % 8);
                }
            }
        }
        Self {
            width,
            height,
            bits,
        }
    }

    /// Loads an 8-bit grayscale PNG written by the SAM2 sidecar (0/255).
    pub fn load_png(path: &Path) -> Result<Self> {
        let image = image::ImageReader::open(path)?
            .with_guessed_format()?
            .decode()
            .map_err(|error| SplatError::Process(format!("分割 mask PNG 无法解码：{error}")))?;
        let gray = image.to_luma8();
        let (width, height) = (gray.width(), gray.height());
        Ok(Self::from_occupied(width, height, |x, y| {
            gray.get_pixel(x, y).0[0] > 0
        }))
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    fn covers(&self, x: f64, y: f64) -> bool {
        if x < 0.0 || y < 0.0 || x >= f64::from(self.width) || y >= f64::from(self.height) {
            return false;
        }
        let index = y as usize * self.width as usize + x as usize;
        self.bits[index / 8] & (1 << (index % 8)) != 0
    }

    /// Fraction of the pixel disk of the given radius (centred at the
    /// projected splat) covered by the mask. Pixels outside the image count
    /// as background so partially off-screen splats are not over-credited.
    fn coverage_in_disk(&self, cx: f64, cy: f64, radius: f64) -> f64 {
        let r2 = radius * radius;
        let mut covered = 0_u64;
        let mut total = 0_u64;
        for y in (cy - radius).floor() as i64..=(cy + radius).ceil() as i64 {
            for x in (cx - radius).floor() as i64..=(cx + radius).ceil() as i64 {
                let dx = x as f64 - cx;
                let dy = y as f64 - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                total += 1;
                if self.covers(x as f64, y as f64) {
                    covered += 1;
                }
            }
        }
        if total == 0 {
            return 0.0;
        }
        covered as f64 / total as f64
    }
}

/// One registered frame ready for voting: pose, projection and object mask.
#[derive(Debug, Clone)]
pub struct FrameView {
    pub name: String,
    pub projection: CameraProjection,
    pub pose: ImagePose,
    pub mask: ObjectMask,
}

/// Voting parameters. Defaults follow the plan document: 60% vote threshold,
/// 2 px footprint dilation for occlusion/edge tolerance, and a frame counts
/// as "object covers this splat" when the majority of the footprint disk
/// falls inside the mask.
#[derive(Debug, Clone, PartialEq)]
pub struct VoteConfig {
    pub vote_threshold: f64,
    pub footprint_dilation_px: f64,
    pub frame_fill_ratio: f64,
}

impl Default for VoteConfig {
    fn default() -> Self {
        Self {
            vote_threshold: 0.6,
            footprint_dilation_px: 2.0,
            frame_fill_ratio: 0.5,
        }
    }
}

/// Per-splat voting result for the whole point cloud.
#[derive(Debug, Clone, PartialEq)]
pub struct VoteOutcome {
    pub splat_count: u64,
    /// yes-votes / visible-frame ratio per splat, in PLY vertex order.
    pub ratios: Vec<f32>,
    /// `ratios[i] >= vote_threshold` — object candidates before clustering.
    pub candidates: Vec<bool>,
}

impl VoteOutcome {
    fn push_splat(&mut self, ratio: f32, config: &VoteConfig) {
        self.ratios.push(ratio);
        self.candidates.push(ratio >= config.vote_threshold as f32);
    }

    /// Object-candidate vote statistics (used for acceptance evidence).
    pub fn ratio_bounds(&self, of_candidates: bool) -> Option<(f32, f32)> {
        let values = self
            .ratios
            .iter()
            .zip(&self.candidates)
            .filter(|(_, candidate)| **candidate == of_candidates)
            .map(|(ratio, _)| *ratio)
            .collect::<Vec<_>>();
        let first = *values.first()?;
        Some(
            values
                .iter()
                .fold((first, first), |(lo, hi), v| (lo.min(*v), hi.max(*v))),
        )
    }
}

/// Votes every splat of a binary Gaussian PLY against the per-frame masks.
///
/// The PLY is streamed in `batch_size`-splat batches (see
/// `stream_splat_cores`); only the compact masks and the per-splat vote
/// vectors stay in memory, so multi-GB `final.ply` files never get
/// materialised. `final.ply` is opened read-only.
pub fn vote_splats(
    ply_path: &Path,
    frames: &[FrameView],
    config: &VoteConfig,
    batch_size: usize,
) -> Result<VoteOutcome> {
    if frames.is_empty() {
        return Err(SplatError::Process(
            "反投影投票需要至少一帧带 mask 的画面".into(),
        ));
    }
    let mut outcome = VoteOutcome {
        splat_count: 0,
        ratios: Vec::new(),
        candidates: Vec::new(),
    };
    stream_splat_cores(ply_path, batch_size, &mut |batch| {
        for splat in batch {
            let mut yes = 0_u32;
            let mut visible = 0_u32;
            for frame in frames {
                let Some(pixel) = frame.projection.world_to_pixel(&frame.pose, splat.position)
                else {
                    continue;
                };
                let (width, height) = (
                    f64::from(frame.mask.width()),
                    f64::from(frame.mask.height()),
                );
                if pixel[0] < 0.0 || pixel[1] < 0.0 || pixel[0] >= width || pixel[1] >= height {
                    continue;
                }
                visible += 1;
                let depth = frame.projection.depth(&frame.pose, splat.position);
                let radius = frame.projection.footprint_radius_px(splat.scale, depth)
                    + config.footprint_dilation_px;
                if frame.mask.coverage_in_disk(pixel[0], pixel[1], radius)
                    >= config.frame_fill_ratio
                {
                    yes += 1;
                }
            }
            let ratio = if visible == 0 {
                0.0
            } else {
                yes as f32 / visible as f32
            };
            outcome.push_splat(ratio, config);
        }
        Ok(())
    })?;
    outcome.splat_count = outcome.ratios.len() as u64;
    Ok(outcome)
}

/// Clustering parameters: eps-neighbor graph over candidate splats, then
/// components smaller than `min_points` are dropped as outliers
/// (DBSCAN-style cleanup of isolated false positives).
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterConfig {
    /// World-space maximum distance between neighbouring object splats.
    pub eps: f64,
    /// Components with fewer splats than this are treated as background.
    pub min_points: usize,
}

/// Connected-component labelling of object candidates.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterOutcome {
    /// Per splat: 0 = background, 1.. = object instance id.
    pub labels: Vec<i32>,
    pub cluster_count: usize,
}

/// Groups voting candidates into spatially connected object instances.
///
/// Positions are indexed through a uniform hash grid with cell size `eps`, so
/// neighbour lookup is O(1) per splat instead of a full O(n²) scan; the vote
/// vectors, positions and labels stay the only in-memory state.
pub fn label_object_clusters(
    positions: &[[f64; 3]],
    candidates: &[bool],
    config: &ClusterConfig,
) -> ClusterOutcome {
    let mut labels = vec![0_i32; positions.len()];
    if config.eps <= 0.0 {
        return ClusterOutcome {
            labels,
            cluster_count: 0,
        };
    }
    let eps2 = config.eps * config.eps;
    let cell = |p: [f64; 3]| -> (i64, i64, i64) {
        (
            (p[0] / config.eps).floor() as i64,
            (p[1] / config.eps).floor() as i64,
            (p[2] / config.eps).floor() as i64,
        )
    };
    let mut grid: std::collections::HashMap<(i64, i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for (index, position) in positions.iter().enumerate() {
        if candidates.get(index).copied().unwrap_or(false) {
            grid.entry(cell(*position)).or_default().push(index);
        }
    }
    let neighbours_within_eps = |index: usize| -> Vec<usize> {
        let (cx, cy, cz) = cell(positions[index]);
        let mut found = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &other in grid.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                        if other == index {
                            continue;
                        }
                        let delta = [
                            positions[index][0] - positions[other][0],
                            positions[index][1] - positions[other][1],
                            positions[index][2] - positions[other][2],
                        ];
                        if delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2] <= eps2 {
                            found.push(other);
                        }
                    }
                }
            }
        }
        found
    };

    let mut cluster_count = 0_usize;
    for seed in 0..positions.len() {
        if !candidates.get(seed).copied().unwrap_or(false) || labels[seed] != 0 {
            continue;
        }
        let mut component = vec![seed];
        labels[seed] = -1; // visiting marker
        let mut cursor = 0;
        while cursor < component.len() {
            for neighbour in neighbours_within_eps(component[cursor]) {
                if labels[neighbour] == 0 {
                    labels[neighbour] = -1;
                    component.push(neighbour);
                }
            }
            cursor += 1;
        }
        if component.len() >= config.min_points {
            cluster_count += 1;
            for index in component {
                labels[index] = cluster_count as i32;
            }
        } else {
            for index in component {
                labels[index] = 0;
            }
        }
    }
    ClusterOutcome {
        labels,
        cluster_count,
    }
}

/// Writes the object/environment split as an OOSMASK-compatible packed bitmap
/// (one bit per splat, set = object instance) to a derived-asset path.
///
/// The source `final.ply` is never touched: the bitmap is a sidecar file that
/// downstream export steps turn into `object.ply` / `background.ply` subsets.
/// Returns the number of object splats.
pub fn write_instance_mask(destination: &Path, splat_count: u64, labels: &[i32]) -> Result<u64> {
    if labels.len() as u64 != splat_count {
        return Err(SplatError::Process(format!(
            "实例标签数量 {} 与 splat 数量 {} 不一致",
            labels.len(),
            splat_count
        )));
    }
    let mut mask = vec![0_u8; (labels.len()).div_ceil(8)];
    let mut object_count = 0_u64;
    for (index, label) in labels.iter().enumerate() {
        if *label != 0 {
            mask[index / 8] |= 1 << (index % 8);
            object_count += 1;
        }
    }
    // Revision 0 marks a derived instance bitmap (edit bitmaps start at 1).
    super::edit_mask::write_mask_container_atomic(destination, splat_count, 0, &mask)?;
    Ok(object_count)
}

/// Pairs the COLMAP model (single camera + posed images) with the per-frame
/// sidecar masks (`<frame stem>.png`, 0/255 grayscale) into `FrameView`s.
///
/// Frames without a mask file are skipped — they simply cast no votes. An
/// empty result (no masks at all) is an error: voting without masks is a
/// misconfiguration, not a valid no-op.
pub fn load_frame_views(model: &Path, masks_dir: &Path) -> Result<Vec<FrameView>> {
    let intrinsics = crate::reconstruction::colmap_model::read_camera_intrinsics(model)?;
    let projection = CameraProjection::from_intrinsics(&intrinsics)?;
    let images = crate::reconstruction::colmap_model::read_posed_images(model)?;
    let mut frames = Vec::with_capacity(images.len());
    for image in images {
        if image.camera_id != intrinsics.camera.id {
            return Err(SplatError::Process(format!(
                "图像 {} 引用了未知的相机 {}",
                image.name, image.camera_id
            )));
        }
        let stem = Path::new(&image.name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| SplatError::Process(format!("图像名称无效：{}", image.name)))?;
        let mask_path = masks_dir.join(format!("{stem}.png"));
        if !mask_path.is_file() {
            continue;
        }
        frames.push(FrameView {
            name: image.name.clone(),
            projection,
            pose: image.pose,
            mask: ObjectMask::load_png(&mask_path)?,
        });
    }
    if frames.is_empty() {
        return Err(SplatError::Process(
            "COLMAP 模型与 mask 目录没有可配对的帧，无法反投影投票".into(),
        ));
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::colmap_model::ColmapCamera;
    use std::io::Write;

    fn intrinsics(model_id: i32, params: &[f64]) -> CameraIntrinsics {
        CameraIntrinsics {
            camera: ColmapCamera {
                id: 1,
                model_id,
                model: "test".into(),
                width: 64,
                height: 48,
            },
            params: params.to_vec(),
        }
    }

    const IDENTITY: ImagePose = ImagePose {
        quaternion: [1.0, 0.0, 0.0, 0.0],
        translation: [0.0, 0.0, 0.0],
    };

    fn pinhole_projection() -> CameraProjection {
        CameraProjection::from_intrinsics(&intrinsics(1, &[800.0, 800.0, 32.0, 24.0])).unwrap()
    }

    #[test]
    fn projects_optical_axis_center_to_principal_point() {
        let projection =
            CameraProjection::from_intrinsics(&intrinsics(1, &[800.0, 800.0, 32.0, 24.0])).unwrap();
        assert_eq!(
            projection.world_to_pixel(&IDENTITY, [0.0, 0.0, 5.0]),
            Some([32.0, 24.0])
        );
    }

    #[test]
    fn projects_offset_point_with_focal_scaling() {
        let projection =
            CameraProjection::from_intrinsics(&intrinsics(1, &[800.0, 800.0, 32.0, 24.0])).unwrap();
        // x: 0.1/5 * 800 = 16 px right; y: -0.05/5 * 800 = -8 px up.
        assert_eq!(
            projection.world_to_pixel(&IDENTITY, [0.1, -0.05, 5.0]),
            Some([48.0, 16.0])
        );
    }

    #[test]
    fn applies_pose_translation() {
        let pose = ImagePose {
            quaternion: [1.0, 0.0, 0.0, 0.0],
            translation: [0.1, 0.0, 0.0],
        };
        assert_eq!(
            pinhole_projection().world_to_pixel(&pose, [0.0, 0.0, 5.0]),
            Some([48.0, 24.0])
        );
    }

    #[test]
    fn applies_pose_rotation() {
        // 90-degree rotation about the y axis (right-hand rule): world +z maps
        // to camera +x and world -x maps to camera +z.
        let quarter_pi = std::f64::consts::FRAC_PI_4;
        let turned = ImagePose {
            quaternion: [quarter_pi.cos(), 0.0, quarter_pi.sin(), 0.0],
            translation: [0.0, 0.0, 0.0],
        };
        // World +z lands on the camera plane (depth 0): invisible.
        assert_eq!(
            pinhole_projection().world_to_pixel(&turned, [0.0, 0.0, 5.0]),
            None
        );
        // World -x maps onto the optical axis at depth 5 (float noise aside).
        let pixel = pinhole_projection()
            .world_to_pixel(&turned, [-5.0, 0.0, 0.0])
            .unwrap();
        assert!((pixel[0] - 32.0).abs() < 1e-9);
        assert!((pixel[1] - 24.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_points_on_or_behind_camera_plane() {
        let projection = pinhole_projection();
        assert_eq!(projection.world_to_pixel(&IDENTITY, [0.0, 0.0, 0.0]), None);
        assert_eq!(projection.world_to_pixel(&IDENTITY, [0.0, 0.0, -1.0]), None);
    }

    #[test]
    fn applies_radial_distortion_for_simple_radial_model() {
        // SIMPLE_RADIAL params: f, cx, cy, k.
        let projection =
            CameraProjection::from_intrinsics(&intrinsics(2, &[800.0, 32.0, 24.0, -0.5])).unwrap();
        // Undistorted: xu = 0.1/5 = 0.02, r2 = 0.0004.
        // xd = 0.02 * (1 - 0.5 * 0.0004) = 0.019996.
        let expected_x = 800.0 * 0.019996 + 32.0;
        assert_eq!(
            projection.world_to_pixel(&IDENTITY, [0.1, 0.0, 5.0]),
            Some([expected_x, 24.0])
        );
    }

    #[test]
    fn rejects_unsupported_camera_models() {
        assert!(CameraProjection::from_intrinsics(&intrinsics(7, &[1.0; 5])).is_err());
        assert!(CameraProjection::from_intrinsics(&intrinsics(6, &[1.0; 12])).is_err());
    }

    #[test]
    fn footprint_radius_uses_exp_scale_and_focal_over_depth() {
        let projection = pinhole_projection();
        // Largest axis: exp(ln(0.02)) = 0.02 -> 0.02 * 800 / 5 = 3.2 px.
        assert_eq!(
            projection.footprint_radius_px([0.0_f64.ln(), 0.02_f64.ln(), 0.01_f64.ln()], 5.0),
            3.2
        );
        // depth() reports the camera-space z of the same point.
        assert_eq!(projection.depth(&IDENTITY, [0.1, 0.0, 5.0]), 5.0);
    }

    // ---- mask + voting -------------------------------------------------

    fn rect_mask(width: u32, height: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> ObjectMask {
        ObjectMask::from_occupied(width, height, |x, y| {
            (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
        })
    }

    #[test]
    fn mask_reports_disk_coverage_and_bounds() {
        let mask = rect_mask(64, 48, 8, 16, 24, 32);
        assert_eq!(mask.width(), 64);
        assert_eq!(mask.height(), 48);
        // Fully inside the rectangle.
        assert_eq!(mask.coverage_in_disk(16.0, 24.0, 3.0), 1.0);
        // Fully outside.
        assert_eq!(mask.coverage_in_disk(48.0, 24.0, 3.0), 0.0);
        // Pixels beyond the image count as background, not as covered.
        assert!(mask.coverage_in_disk(63.0, 24.0, 3.0) < 1.0);
    }

    #[test]
    fn loads_sidecar_png_mask() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame_000001.png");
        let mut image = image::GrayImage::new(8, 4);
        for y in 0..4 {
            for x in 0..8 {
                let on = x < 4 && y < 2;
                image.put_pixel(x, y, image::Luma([if on { 255 } else { 0 }]));
            }
        }
        image.save(&path).unwrap();
        let mask = ObjectMask::load_png(&path).unwrap();
        assert_eq!((mask.width(), mask.height()), (8, 4));
        assert_eq!(mask.coverage_in_disk(1.5, 0.5, 1.0), 1.0);
        assert_eq!(mask.coverage_in_disk(6.0, 3.0, 1.0), 0.0);
    }

    /// Writes a tiny binary PLY with only x/y/z/scale_0..2 float properties.
    fn write_voting_fixture(path: &Path, splats: &[(f64, f64, f64)]) {
        let header = format!(
            "ply\nformat binary_little_endian 1.0\nelement vertex {}\nproperty float x\nproperty float y\nproperty float z\nproperty float scale_0\nproperty float scale_1\nproperty float scale_2\nend_header\n",
            splats.len()
        );
        let mut body = Vec::new();
        for (x, y, z) in splats {
            for value in [*x, *y, *z, 0.005_f64.ln(), 0.005_f64.ln(), 0.005_f64.ln()] {
                body.extend_from_slice(&(value as f32).to_le_bytes());
            }
        }
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend_from_slice(&body);
        std::fs::write(path, bytes).unwrap();
    }

    fn frame(translation: [f64; 3], mask: &ObjectMask) -> FrameView {
        FrameView {
            name: "frame".into(),
            projection: CameraProjection::from_intrinsics(
                &crate::reconstruction::colmap_model::CameraIntrinsics {
                    camera: crate::reconstruction::colmap_model::ColmapCamera {
                        id: 1,
                        model_id: 1,
                        model: "PINHOLE".into(),
                        width: 64,
                        height: 48,
                    },
                    params: vec![800.0, 800.0, 32.0, 24.0],
                },
            )
            .unwrap(),
            pose: ImagePose {
                quaternion: [1.0, 0.0, 0.0, 0.0],
                translation,
            },
            mask: mask.clone(),
        }
    }

    #[test]
    fn voting_separates_object_from_background_splats() {
        let dir = tempfile::tempdir().unwrap();
        let ply = dir.path().join("cloud.ply");
        // Rect mask x 8..24 px, y 16..32 px; camera depth 10, f=800.
        // Splats A..D project to px x = 32 + 80*(x_w + t_x), py = 24 + 80*(y_w + t_y).
        write_voting_fixture(
            &ply,
            &[
                (-0.2, 0.0, 10.0),  // A: inside mask in 3 of 4 visible frames -> 3/4 = 0.75
                (-0.2, 0.35, 10.0), // B: off-image in every frame -> visible 0 -> 0
                (-0.2, 0.0, 10.0),  // C: same as A -> 3/4 = 0.75
                (0.5, 0.0, 10.0),   // D: background side of the image -> 0
            ],
        );
        let mask = rect_mask(64, 48, 8, 16, 24, 32);
        let frames = vec![
            frame([0.0, 0.0, 0.0], &mask),
            frame([0.0, 0.3, 0.0], &mask), // py 48 -> out of bounds
            frame([0.0, 0.0, 0.0], &mask),
            frame([0.5, 0.0, 0.0], &mask), // px 72 (A/C) but still in-bounds
            frame([0.0, 0.0, 0.0], &mask),
        ];
        let outcome = vote_splats(&ply, &frames, &VoteConfig::default(), 3).unwrap();
        assert_eq!(outcome.splat_count, 4);
        println!(
            "[acceptance] object-splat vote ratios: A={:.3} C={:.3}; background: B={:.3} D={:.3}",
            outcome.ratios[0], outcome.ratios[2], outcome.ratios[1], outcome.ratios[3]
        );
        // Object points vote well above the 0.6 threshold; background at 0.
        assert!(outcome.candidates[0]);
        assert!(outcome.candidates[2]);
        assert!(!outcome.candidates[1]);
        assert!(!outcome.candidates[3]);
        assert!(outcome.ratios[0] > 0.6 && outcome.ratios[2] > 0.6);
        assert_eq!(outcome.ratios[1], 0.0);
        assert_eq!(outcome.ratios[3], 0.0);
        let (lo, hi) = outcome.ratio_bounds(true).unwrap();
        assert!(lo > 0.6 && hi <= 1.0);
    }

    #[test]
    fn vote_threshold_gates_partial_visibility() {
        let dir = tempfile::tempdir().unwrap();
        let ply = dir.path().join("cloud.ply");
        // Splat projects inside the mask in exactly 3 of 5 frames.
        write_voting_fixture(
            &ply,
            &[
                (-0.2, 0.0, 10.0), // 4 of 5 visible frames -> 0.8, above 0.6
                (-0.2, 0.2, 10.0), // py = 24+16=40 in-bounds but below rect -> 0/5
            ],
        );
        let mask = rect_mask(64, 48, 8, 16, 24, 32);
        let frames = vec![
            frame([0.0, 0.0, 0.0], &mask),
            frame([0.0, 0.3, 0.0], &mask),
            frame([0.0, 0.0, 0.0], &mask),
            frame([0.5, 0.0, 0.0], &mask),
            frame([0.0, 0.0, 0.0], &mask),
        ];
        let config = VoteConfig {
            vote_threshold: 0.9,
            ..VoteConfig::default()
        };
        let outcome = vote_splats(&ply, &frames, &config, 64).unwrap();
        println!("[acceptance] threshold test ratios: {:?}", outcome.ratios);
        // 0.8 < 0.9 -> rejected despite strong object evidence.
        assert!(!outcome.candidates[0]);
        assert!(!outcome.candidates[1]);
    }

    #[test]
    fn voting_requires_at_least_one_masked_frame() {
        let dir = tempfile::tempdir().unwrap();
        let ply = dir.path().join("cloud.ply");
        write_voting_fixture(&ply, &[(-0.2, 0.0, 10.0)]);
        assert!(vote_splats(&ply, &[], &VoteConfig::default(), 8).is_err());
    }

    // ---- connected components / DBSCAN cleanup --------------------------

    #[test]
    fn clusters_candidates_and_drops_outliers() {
        let positions = [
            [0.0, 0.0, 0.0], // blob 1 (chain of 3 within eps)
            [0.05, 0.0, 0.0],
            [0.1, 0.0, 0.0],
            [5.0, 5.0, 5.0], // blob 2 (trio within eps)
            [5.05, 5.0, 5.0],
            [5.0, 5.05, 5.0],
            [9.0, 9.0, 9.0],   // lone candidate outlier -> background
            [0.06, 0.02, 0.0], // near blob 1 but not a candidate -> background
        ];
        let candidates = [true, true, true, true, true, true, true, false];
        let outcome = label_object_clusters(
            &positions,
            &candidates,
            &ClusterConfig {
                eps: 0.2,
                min_points: 3,
            },
        );
        println!(
            "[acceptance] cluster labels: {:?} ({} clusters)",
            outcome.labels, outcome.cluster_count
        );
        assert_eq!(outcome.cluster_count, 2);
        assert_eq!(outcome.labels[0..3], [1, 1, 1]);
        assert_eq!(outcome.labels[3..6], [2, 2, 2]);
        assert_eq!(outcome.labels[6], 0, "outlier cleaned up");
        assert_eq!(outcome.labels[7], 0, "non-candidate stays background");
    }

    #[test]
    fn min_points_drops_small_clusters() {
        let positions = [[0.0, 0.0, 0.0], [0.05, 0.0, 0.0]];
        let outcome = label_object_clusters(
            &positions,
            &[true, true],
            &ClusterConfig {
                eps: 0.2,
                min_points: 3,
            },
        );
        assert_eq!(outcome.cluster_count, 0);
        assert_eq!(outcome.labels, vec![0, 0]);
    }

    #[test]
    fn dbscan_core_semantics_keeps_sparse_noise_chains_from_bridging() {
        // Two dense 3x3 blobs joined by a 1-D chain of splats spaced 0.08
        // apart (eps = 0.1): every interior chain splat sees only its two
        // chain neighbours (+ itself) inside eps, so with min_points = 4 true
        // DBSCAN must treat them as non-core noise and keep the objects
        // separate. Single-link labelling bridges them into one component.
        let eps = 0.1;
        let mut positions: Vec<[f64; 3]> = Vec::new();
        for (cx, cy) in [(0.0_f64, 0.0_f64), (2.0, 0.0)] {
            for i in 0..3_u32 {
                for j in 0..3_u32 {
                    positions.push([cx + 0.03 * f64::from(i), cy + 0.03 * f64::from(j), 0.0]);
                }
            }
        }
        let blob_count = positions.len();
        let chain: Vec<f64> = (0..=24).map(|k| 0.12 + 0.08 * f64::from(k)).collect();
        for x in &chain {
            positions.push([*x, 0.0, 0.0]);
        }
        let candidates = vec![true; positions.len()];
        let outcome = label_object_clusters(
            &positions,
            &candidates,
            &ClusterConfig {
                eps,
                min_points: 4,
            },
        );
        println!(
            "[acceptance] dbscan labels: {:?} ({} clusters)",
            outcome.labels, outcome.cluster_count
        );
        assert_eq!(
            outcome.cluster_count, 2,
            "sparse noise chain must not bridge the two blobs"
        );
        // Both blobs stay labelled; only the chain ends (adjacent to a blob)
        // join a cluster, the sparse interior is noise.
        let interior_noise = &outcome.labels[blob_count + 1..positions.len() - 2];
        assert!(
            interior_noise.iter().all(|label| *label == 0),
            "sparse chain interior must stay background: {interior_noise:?}"
        );
        assert_eq!(outcome.labels[blob_count], 1, "chain start joins blob A");
        assert_eq!(outcome.labels[positions.len() - 1], 2, "chain end joins blob B");
    }

    #[test]
    fn eps_boundary_is_respected() {
        let positions = [[0.0, 0.0, 0.0], [0.3, 0.0, 0.0]];
        let outcome = label_object_clusters(
            &positions,
            &[true, true],
            &ClusterConfig {
                eps: 0.2,
                min_points: 2,
            },
        );
        // Distance 0.3 > eps 0.2: two separate singletons, both dropped.
        assert_eq!(outcome.cluster_count, 0);
        let touching = label_object_clusters(
            &positions,
            &[true, true],
            &ClusterConfig {
                eps: 0.3,
                min_points: 2,
            },
        );
        assert_eq!(touching.cluster_count, 1);
    }

    // ---- instance bitmap output + frame pairing --------------------------

    #[test]
    fn writes_oosmask_instance_bitmap() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("derived").join("instance.mask");
        let object_count =
            write_instance_mask(&destination, 10, &[1, 0, 2, 0, 0, 1, 0, 0, 0, 3]).unwrap();
        assert_eq!(object_count, 4);
        let (revision, deleted_count, mask) =
            crate::reconstruction::edit_mask::read_mask_container(&destination, 10).unwrap();
        assert_eq!(revision, 0, "derived instance bitmaps carry revision 0");
        assert_eq!(deleted_count, 4);
        // Bits 0, 2, 5 in byte 0 (labels 1, 2, 1) and bit 1 in byte 1 (label 3).
        assert_eq!(mask, vec![0b0010_0101, 0b0000_0010]);
    }

    #[test]
    fn rejects_instance_mask_length_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_instance_mask(&dir.path().join("i.mask"), 3, &[1, 0]).is_err());
    }

    fn write_model_fixture(model_dir: &Path, image_names: &[&str]) {
        std::fs::create_dir_all(model_dir).unwrap();
        let mut cameras = std::fs::File::create(model_dir.join("cameras.bin")).unwrap();
        cameras.write_all(&1_u64.to_le_bytes()).unwrap();
        cameras.write_all(&7_u32.to_le_bytes()).unwrap();
        cameras.write_all(&1_i32.to_le_bytes()).unwrap();
        cameras.write_all(&64_u64.to_le_bytes()).unwrap();
        cameras.write_all(&48_u64.to_le_bytes()).unwrap();
        for value in [800.0_f64, 800.0, 32.0, 24.0] {
            cameras.write_all(&value.to_le_bytes()).unwrap();
        }
        let mut images = std::fs::File::create(model_dir.join("images.bin")).unwrap();
        images
            .write_all(&(image_names.len() as u64).to_le_bytes())
            .unwrap();
        for (index, name) in image_names.iter().enumerate() {
            images
                .write_all(&((index + 1) as u32).to_le_bytes())
                .unwrap();
            for value in [1.0_f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] {
                images.write_all(&value.to_le_bytes()).unwrap();
            }
            images.write_all(&7_u32.to_le_bytes()).unwrap();
            images.write_all(name.as_bytes()).unwrap();
            images.write_all(&[0_u8]).unwrap();
            images.write_all(&0_u64.to_le_bytes()).unwrap();
        }
    }

    fn write_mask_png(dir: &Path, stem: &str) {
        let mut image = image::GrayImage::new(64, 48);
        for y in 0..48 {
            for x in 0..64 {
                let on = (8..=24).contains(&x) && (16..=32).contains(&y);
                image.put_pixel(x, y, image::Luma([if on { 255 } else { 0 }]));
            }
        }
        image.save(dir.join(format!("{stem}.png"))).unwrap();
    }

    #[test]
    fn pairs_colmap_frames_with_sidecar_masks() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("sparse").join("0");
        let masks_dir = dir.path().join("masks");
        std::fs::create_dir_all(&masks_dir).unwrap();
        // frame_000001.jpg and frame_000003.jpg have masks, 000002 has none.
        write_model_fixture(
            &model_dir,
            &["frame_000001.jpg", "frame_000002.jpg", "frame_000003.jpg"],
        );
        write_mask_png(&masks_dir, "frame_000001");
        write_mask_png(&masks_dir, "frame_000003");
        let frames = load_frame_views(&model_dir, &masks_dir).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].name, "frame_000001.jpg");
        assert_eq!(frames[1].name, "frame_000003.jpg");
        assert_eq!((frames[0].mask.width(), frames[0].mask.height()), (64, 48));
    }

    #[test]
    fn load_frame_views_rejects_mask_resolution_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("sparse").join("0");
        let masks_dir = dir.path().join("masks");
        std::fs::create_dir_all(&masks_dir).unwrap();
        write_model_fixture(&model_dir, &["frame_000001.jpg"]);
        // The model fixture camera is 64x48; this mask is not, and a
        // resolution mismatch would silently misplace every projection.
        let mut image = image::GrayImage::new(32, 24);
        image.put_pixel(0, 0, image::Luma([255_u8]));
        image.save(masks_dir.join("frame_000001.png")).unwrap();
        let error = load_frame_views(&model_dir, &masks_dir).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("分辨率"), "unexpected error: {message}");
    }

    #[test]
    fn rejects_pairing_without_any_masks() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("sparse").join("0");
        let masks_dir = dir.path().join("masks");
        std::fs::create_dir_all(&masks_dir).unwrap();
        write_model_fixture(&model_dir, &["frame_000001.jpg"]);
        assert!(load_frame_views(&model_dir, &masks_dir).is_err());
    }

    // ---- end-to-end acceptance: vote -> cluster -> instance bitmap ------

    const SCENE_WIDTH: u32 = 640;
    const SCENE_HEIGHT: u32 = 480;
    const SCENE_F: f64 = 800.0;
    const OBJECT_RADIUS: f64 = 0.3;
    const OBJECT_SPLATS: usize = 800;
    const BACKGROUND_SPLATS: usize = 1200;

    fn lcg(state: &mut u64) -> f64 {
        // Deterministic pseudo-random in [-1, 1] for reproducible scenes.
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*state >> 33) as f64 / (u32::MAX as f64 / 2.0)) - 1.0
    }

    fn sample_cloud() -> Vec<[f64; 3]> {
        let mut state = 0x5eed_cafe_u64;
        let mut splats = Vec::with_capacity(OBJECT_SPLATS + BACKGROUND_SPLATS);
        // Object: filled ball at the origin.
        for _ in 0..OBJECT_SPLATS {
            loop {
                let p = [lcg(&mut state), lcg(&mut state), lcg(&mut state)];
                if p.iter().map(|v| v * v).sum::<f64>() <= 1.0 {
                    splats.push([
                        p[0] * OBJECT_RADIUS,
                        p[1] * OBJECT_RADIUS,
                        p[2] * OBJECT_RADIUS,
                    ]);
                    break;
                }
            }
        }
        // Background: shell around the object (projection misses the mask).
        for _ in 0..BACKGROUND_SPLATS {
            loop {
                let p = [lcg(&mut state), lcg(&mut state), lcg(&mut state)];
                let r2 = p.iter().map(|v| v * v).sum::<f64>();
                if r2 > 1.0 && r2 <= 4.0 {
                    splats.push([p[0] * 0.9, p[1] * 0.9, p[2] * 0.9]);
                    break;
                }
            }
        }
        splats
    }

    fn write_scene_cloud(path: &Path, splats: &[[f64; 3]]) {
        let header = format!(
            "ply\nformat binary_little_endian 1.0\nelement vertex {}\nproperty float x\nproperty float y\nproperty float z\nproperty float scale_0\nproperty float scale_1\nproperty float scale_2\nend_header\n",
            splats.len()
        );
        let mut bytes = header.into_bytes();
        let scale = 0.005_f64.ln();
        for splat in splats {
            for value in [splat[0], splat[1], splat[2], scale, scale, scale] {
                bytes.extend_from_slice(&(value as f32).to_le_bytes());
            }
        }
        std::fs::write(path, bytes).unwrap();
    }

    /// Cameras orbit the origin in the x-z plane (world y up) at depth 10;
    /// the object silhouette is a disk around the principal point (radius
    /// 32 px) in each mask. Orbiting views break the view-axis ambiguity so
    /// background points behind/in front of the object align with at most 2
    /// of the 6 view axes and stay below the vote threshold.
    fn write_scene_model_and_masks(model_dir: &Path, masks_dir: &Path) {
        std::fs::create_dir_all(model_dir).unwrap();
        std::fs::create_dir_all(masks_dir).unwrap();
        let angles: [f64; 6] = [0.0, 60.0, 120.0, 180.0, 240.0, 300.0];
        let mut cameras = std::fs::File::create(model_dir.join("cameras.bin")).unwrap();
        cameras.write_all(&1_u64.to_le_bytes()).unwrap();
        cameras.write_all(&1_u32.to_le_bytes()).unwrap();
        cameras.write_all(&1_i32.to_le_bytes()).unwrap();
        cameras
            .write_all(&(SCENE_WIDTH as u64).to_le_bytes())
            .unwrap();
        cameras
            .write_all(&(SCENE_HEIGHT as u64).to_le_bytes())
            .unwrap();
        for value in [
            SCENE_F,
            SCENE_F,
            SCENE_WIDTH as f64 / 2.0,
            SCENE_HEIGHT as f64 / 2.0,
        ] {
            cameras.write_all(&value.to_le_bytes()).unwrap();
        }
        let mut images = std::fs::File::create(model_dir.join("images.bin")).unwrap();
        images
            .write_all(&(angles.len() as u64).to_le_bytes())
            .unwrap();
        for (index, degrees) in angles.iter().enumerate() {
            let (sin, cos) = degrees.to_radians().sin_cos();
            // Camera at (10 cos, 0, 10 sin), looking at the origin, y up.
            // World-to-camera rotation rows: right, down, forward.
            let rotation = [[sin, 0.0, -cos], [0.0, -1.0, 0.0], [-cos, 0.0, -sin]];
            let quaternion = rotation_to_quaternion(&rotation);
            // The origin projects to the principal point, so t = (0, 0, 10).
            let translation = [0.0_f64, 0.0, 10.0];
            images
                .write_all(&((index + 1) as u32).to_le_bytes())
                .unwrap();
            for value in quaternion {
                images.write_all(&value.to_le_bytes()).unwrap();
            }
            for value in translation {
                images.write_all(&value.to_le_bytes()).unwrap();
            }
            images.write_all(&1_u32.to_le_bytes()).unwrap();
            let name = format!("frame_{:06}.jpg", index + 1);
            images.write_all(name.as_bytes()).unwrap();
            images.write_all(&[0_u8]).unwrap();
            images.write_all(&0_u64.to_le_bytes()).unwrap();

            let mut image = image::GrayImage::new(SCENE_WIDTH, SCENE_HEIGHT);
            for y in 0..SCENE_HEIGHT {
                for x in 0..SCENE_WIDTH {
                    let inside = (x as f64 - SCENE_WIDTH as f64 / 2.0).powi(2)
                        + (y as f64 - SCENE_HEIGHT as f64 / 2.0).powi(2)
                        <= 32.0_f64.powi(2);
                    image.put_pixel(x, y, image::Luma([if inside { 255 } else { 0 }]));
                }
            }
            let stem = name.trim_end_matches(".jpg");
            image.save(masks_dir.join(format!("{stem}.png"))).unwrap();
        }
    }

    /// Shepperd's method: rotation matrix (rows = camera axes in world
    /// coordinates) to quaternion (w, x, y, z).
    fn rotation_to_quaternion(m: &[[f64; 3]; 3]) -> [f64; 4] {
        let trace = m[0][0] + m[1][1] + m[2][2];
        let (qw, qx, qy, qz);
        if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            qw = 0.25 * s;
            qx = (m[2][1] - m[1][2]) / s;
            qy = (m[0][2] - m[2][0]) / s;
            qz = (m[1][0] - m[0][1]) / s;
        } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
            qw = (m[2][1] - m[1][2]) / s;
            qx = 0.25 * s;
            qy = (m[0][1] + m[1][0]) / s;
            qz = (m[0][2] + m[2][0]) / s;
        } else if m[1][1] > m[2][2] {
            let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
            qw = (m[0][2] - m[2][0]) / s;
            qx = (m[0][1] + m[1][0]) / s;
            qy = 0.25 * s;
            qz = (m[1][2] + m[2][1]) / s;
        } else {
            let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
            qw = (m[1][0] - m[0][1]) / s;
            qx = (m[0][2] + m[2][0]) / s;
            qy = (m[1][2] + m[2][1]) / s;
            qz = 0.25 * s;
        }
        [qw, qx, qy, qz]
    }

    #[test]
    fn end_to_end_vote_cluster_instance_mask_on_sample_cloud() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("sparse").join("0");
        let masks_dir = dir.path().join("masks");
        write_scene_model_and_masks(&model_dir, &masks_dir);
        let splats = sample_cloud();
        let ply = dir.path().join("final.ply");
        write_scene_cloud(&ply, &splats);
        let original_bytes = std::fs::read(&ply).unwrap();

        let frames = load_frame_views(&model_dir, &masks_dir).unwrap();
        assert_eq!(frames.len(), 6);
        let outcome = vote_splats(&ply, &frames, &VoteConfig::default(), 256).unwrap();
        let positions: Vec<[f64; 3]> = splats.to_vec();
        let clusters = label_object_clusters(
            &positions,
            &outcome.candidates,
            &ClusterConfig {
                eps: 0.1,
                min_points: 10,
            },
        );

        let object_ratios = &outcome.ratios[..OBJECT_SPLATS];
        let background_ratios = &outcome.ratios[OBJECT_SPLATS..];
        let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
        println!(
            "[acceptance] object splats: n={} mean ratio={:.3} min={:.3}; background: n={} mean={:.3} max={:.3}; candidates: {}; clusters: {}",
            object_ratios.len(),
            mean(object_ratios),
            object_ratios.iter().cloned().fold(f32::MAX, f32::min),
            background_ratios.len(),
            mean(background_ratios),
            background_ratios.iter().cloned().fold(f32::MIN, f32::max),
            outcome.candidates.iter().filter(|c| **c).count(),
            clusters.cluster_count
        );

        // Voting: object points must vote far above background points. The
        // worst background splats sit on a view axis (2/6 views covered by
        // the object silhouette) and still stay below the 0.6 threshold.
        assert!(mean(object_ratios) > 0.9, "object mean ratio too low");
        assert!(
            background_ratios.iter().cloned().fold(f32::MIN, f32::max) < 0.5,
            "a background splat voted too high"
        );
        assert_eq!(
            outcome.candidates.iter().filter(|c| **c).count(),
            OBJECT_SPLATS,
            "candidate set must be exactly the object"
        );
        // Clustering: the object forms exactly one component covering it.
        assert_eq!(clusters.cluster_count, 1);
        assert!(clusters.labels[..OBJECT_SPLATS].iter().all(|l| *l == 1));
        assert!(clusters.labels[OBJECT_SPLATS..].iter().all(|l| *l == 0));

        // Instance bitmap: derived OOSMASK asset, final.ply untouched.
        let destination = dir.path().join("derived").join("instance.mask");
        let object_count =
            write_instance_mask(&destination, splats.len() as u64, &clusters.labels).unwrap();
        assert_eq!(object_count, OBJECT_SPLATS as u64);
        let (_, _, mask) = crate::reconstruction::edit_mask::read_mask_container(
            &destination,
            splats.len() as u64,
        )
        .unwrap();
        assert_eq!(
            mask.iter().map(|b| u64::from(b.count_ones())).sum::<u64>(),
            OBJECT_SPLATS as u64
        );
        assert_eq!(
            std::fs::read(&ply).unwrap(),
            original_bytes,
            "final.ply must be read-only"
        );
    }
}
