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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::colmap_model::ColmapCamera;

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
}
