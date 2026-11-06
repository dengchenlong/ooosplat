//! TSDF fusion: a dense voxel grid whose values are the truncated signed
//! distance to the observed object surface (positive outside, negative inside).
//!
//! Each frame projects every voxel into its depth map and fuses
//! `sdf = depth - voxel_depth` when `|sdf| < truncation`. Unobserved voxels
//! keep `NaN` so the surface extractor never invents geometry in empty space.

use crate::error::{Result, SplatError};
use crate::reconstruction::backproject::CameraProjection;
use crate::reconstruction::colmap_model::ImagePose;
use crate::reconstruction::mesh::depth::DepthMap;

/// Voxel grid geometry: `origin` is the centre of voxel `(0, 0, 0)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSpec {
    pub origin: [f64; 3],
    pub voxel_size: f64,
    pub dims: [usize; 3],
}

impl GridSpec {
    /// Builds a grid covering `[min, max]` inflated by `padding` on every
    /// side, with at least one voxel per axis.
    pub fn from_bounds(
        min: [f64; 3],
        max: [f64; 3],
        voxel_size: f64,
        padding: f64,
    ) -> Result<Self> {
        if !(voxel_size.is_finite() && voxel_size > 0.0) {
            return Err(SplatError::Process("体素尺寸必须是正数".into()));
        }
        if padding < 0.0 || !padding.is_finite() {
            return Err(SplatError::Process("体素网格边距不能为负".into()));
        }
        let mut dims = [1_usize; 3];
        for axis in 0..3 {
            let span = (max[axis] - min[axis]) + 2.0 * padding;
            if !span.is_finite() {
                return Err(SplatError::Process("物体包围盒无效".into()));
            }
            dims[axis] = ((span / voxel_size).ceil() as usize).max(1);
        }
        Ok(Self {
            origin: [
                min[0] - padding + 0.5 * voxel_size,
                min[1] - padding + 0.5 * voxel_size,
                min[2] - padding + 0.5 * voxel_size,
            ],
            voxel_size,
            dims,
        })
    }

    pub fn voxel_count(&self) -> usize {
        self.dims[0] * self.dims[1] * self.dims[2]
    }

    pub fn index(&self, i: usize, j: usize, k: usize) -> usize {
        (k * self.dims[1] + j) * self.dims[0] + i
    }

    /// World-space centre of voxel `(i, j, k)`.
    pub fn position(&self, i: usize, j: usize, k: usize) -> [f64; 3] {
        [
            self.origin[0] + i as f64 * self.voxel_size,
            self.origin[1] + j as f64 * self.voxel_size,
            self.origin[2] + k as f64 * self.voxel_size,
        ]
    }
}

/// One frame's projection, pose and rendered object depth.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthFrame {
    pub projection: CameraProjection,
    pub pose: ImagePose,
    pub depth: DepthMap,
}

/// Fusion parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuseOptions {
    /// Distance band around the surface that is fused, in scene units.
    pub truncation: f64,
    /// Voxels observed by fewer frames are treated as unobserved.
    pub min_observations: u32,
}

impl FuseOptions {
    /// `truncation = truncation_voxels * voxel_size`; the 4-voxel default
    /// keeps every cell that straddles the surface fully inside the band.
    pub fn new(voxel_size: f64, truncation_voxels: f64, min_observations: u32) -> Self {
        Self {
            truncation: voxel_size * truncation_voxels,
            min_observations,
        }
    }
}

/// The fused grid. `values` are weighted-mean signed distances in scene
/// units; `NaN` means "no observation".
#[derive(Debug, Clone, PartialEq)]
pub struct TsdfGrid {
    pub spec: GridSpec,
    pub values: Vec<f32>,
    pub observations: Vec<u32>,
    /// Number of voxels that received at least one in-band observation.
    pub fused_voxels: u64,
}

impl TsdfGrid {
    pub fn new(spec: GridSpec) -> Self {
        let count = spec.voxel_count();
        Self {
            spec,
            values: vec![f32::NAN; count],
            observations: vec![0; count],
            fused_voxels: 0,
        }
    }

    /// Fills the grid analytically — used by tests and by synthetic scenes.
    pub fn from_signed_distance(
        spec: GridSpec,
        truncation: f64,
        signed_distance: impl Fn([f64; 3]) -> f64,
    ) -> Self {
        let mut grid = Self::new(spec);
        for k in 0..spec.dims[2] {
            for j in 0..spec.dims[1] {
                for i in 0..spec.dims[0] {
                    let distance = signed_distance(spec.position(i, j, k));
                    if distance.abs() < truncation {
                        let index = spec.index(i, j, k);
                        grid.values[index] = distance as f32;
                        grid.observations[index] = 1;
                        grid.fused_voxels += 1;
                    }
                }
            }
        }
        grid
    }

    pub fn value_at(&self, i: usize, j: usize, k: usize) -> Option<f64> {
        if i >= self.spec.dims[0] || j >= self.spec.dims[1] || k >= self.spec.dims[2] {
            return None;
        }
        let value = self.values[self.spec.index(i, j, k)];
        value.is_finite().then_some(value as f64)
    }

    /// Voxels that survived `min_observations` and carry a finite distance.
    pub fn observed_voxels(&self) -> usize {
        self.values.iter().filter(|value| value.is_finite()).count()
    }

    /// Gradient of the field at a voxel centre (central differences, one-sided
    /// at the border or whenever one neighbour is unobserved — the fused band
    /// is sparse, so one-sided differences are the common case, not the
    /// exception).
    pub fn gradient_at(&self, i: usize, j: usize, k: usize) -> [f64; 3] {
        let mut gradient = [0.0_f64; 3];
        for (axis, slot) in gradient.iter_mut().enumerate() {
            let centre = self.value_at(i, j, k);
            let mut plus = [i, j, k];
            plus[axis] += 1;
            let plus = if plus[axis] < self.spec.dims[axis] {
                self.value_at(plus[0], plus[1], plus[2])
            } else {
                None
            };
            let minus = if [i, j, k][axis] > 0 {
                let mut minus = [i, j, k];
                minus[axis] -= 1;
                self.value_at(minus[0], minus[1], minus[2])
            } else {
                None
            };
            let step = self.spec.voxel_size;
            *slot = match (minus, centre, plus) {
                (Some(low), _, Some(high)) => (high - low) / (2.0 * step),
                (Some(low), Some(here), None) => (here - low) / step,
                (None, Some(here), Some(high)) => (high - here) / step,
                _ => 0.0,
            };
        }
        gradient
    }
}

/// Fuses every frame into `grid`. Returns the number of in-band observations
/// that were accumulated.
///
/// Voxels observed by fewer than `options.min_observations` frames are left
/// as `NaN` at the end, so the extractor only sees geometry the cameras
/// actually agree on.
pub fn fuse_into(grid: &mut TsdfGrid, frames: &[DepthFrame], options: &FuseOptions) -> Result<u64> {
    if !(options.truncation.is_finite() && options.truncation > 0.0) {
        return Err(SplatError::Process("TSDF 截断距离必须是正数".into()));
    }
    if options.min_observations == 0 {
        return Err(SplatError::Process("最少观测帧数必须大于 0".into()));
    }
    let count = grid.spec.voxel_count();
    let mut sums = vec![0.0_f64; count];
    let mut weights = vec![0.0_f64; count];
    let mut observations = vec![0_u32; count];
    let mut total = 0_u64;
    let truncation = options.truncation;
    let spec = grid.spec;

    for frame in frames {
        let (width, height) = (frame.depth.width as i64, frame.depth.height as i64);
        if width == 0 || height == 0 {
            continue;
        }
        let rotation = rotation_matrix(&frame.pose);
        let translation = frame.pose.translation;
        for k in 0..spec.dims[2] {
            for j in 0..spec.dims[1] {
                for i in 0..spec.dims[0] {
                    let p = spec.position(i, j, k);
                    let cam = [
                        rotation[0][0] * p[0]
                            + rotation[0][1] * p[1]
                            + rotation[0][2] * p[2]
                            + translation[0],
                        rotation[1][0] * p[0]
                            + rotation[1][1] * p[1]
                            + rotation[1][2] * p[2]
                            + translation[1],
                        rotation[2][0] * p[0]
                            + rotation[2][1] * p[1]
                            + rotation[2][2] * p[2]
                            + translation[2],
                    ];
                    if cam[2] <= 1e-9 {
                        continue;
                    }
                    let Some([px, py]) = frame.projection.camera_to_pixel(cam) else {
                        continue;
                    };
                    let x = px.round();
                    let y = py.round();
                    if !(x >= 0.0 && y >= 0.0 && x < width as f64 && y < height as f64) {
                        continue;
                    }
                    // A 3x3 minimum over the depth map: sparse splats leave
                    // single-pixel gaps, and taking the nearest of the
                    // neighbourhood keeps the band from breaking up at those
                    // pinholes (it never invents a surface in front of the
                    // nearest observation).
                    let mut nearest: Option<f64> = None;
                    for dy in -1_i64..=1 {
                        for dx in -1_i64..=1 {
                            let (sx, sy) = (x as i64 + dx, y as i64 + dy);
                            if sx < 0 || sy < 0 || sx >= width || sy >= height {
                                continue;
                            }
                            if let Some(value) = frame.depth.depth_at(sx as u32, sy as u32) {
                                nearest = Some(nearest.map_or(value, |best: f64| best.min(value)));
                            }
                        }
                    }
                    let Some(depth) = nearest else {
                        continue;
                    };
                    let sdf = depth - cam[2];
                    if !sdf.is_finite() || sdf.abs() > truncation {
                        continue;
                    }
                    let index = spec.index(i, j, k);
                    sums[index] += sdf;
                    weights[index] += 1.0;
                    observations[index] = observations[index].saturating_add(1);
                    total += 1;
                }
            }
        }
    }

    let mut fused = 0_u64;
    for index in 0..count {
        if weights[index] > 0.0 && observations[index] >= options.min_observations {
            grid.values[index] = (sums[index] / weights[index]) as f32;
            grid.observations[index] = observations[index];
            fused += 1;
        } else {
            grid.values[index] = f32::NAN;
            grid.observations[index] = observations[index];
        }
    }
    grid.fused_voxels = fused;
    Ok(total)
}

/// World-to-camera rotation matrix from the COLMAP quaternion `(w, x, y, z)`.
fn rotation_matrix(pose: &ImagePose) -> [[f64; 3]; 3] {
    let [w, x, y, z] = pose.quaternion;
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::colmap_model::{CameraIntrinsics, ColmapCamera};

    fn intrinsics(width: u64, height: u64, f: f64) -> CameraIntrinsics {
        let params = vec![f, width as f64 / 2.0, height as f64 / 2.0, 0.0];
        CameraIntrinsics {
            camera: ColmapCamera {
                id: 1,
                model_id: 2,
                model: "SIMPLE_RADIAL".into(),
                width,
                height,
                params: params.clone(),
            },
            params,
        }
    }

    fn identity_pose() -> ImagePose {
        ImagePose {
            quaternion: [1.0, 0.0, 0.0, 0.0],
            translation: [0.0, 0.0, 0.0],
        }
    }

    fn flat_frame(width: u32, height: u32, depth: f32) -> DepthFrame {
        let mut map = DepthMap::new(width, height);
        map.depths.fill(depth);
        DepthFrame {
            projection: CameraProjection::from_intrinsics(&intrinsics(
                width as u64,
                height as u64,
                100.0,
            ))
            .unwrap(),
            pose: identity_pose(),
            depth: map,
        }
    }

    /// A 41^3 grid of 0.1-unit voxels centred on the origin, i.e. covering
    /// roughly [-2, 2] on every axis at a 5.0 camera depth.
    fn plane_grid() -> GridSpec {
        GridSpec {
            origin: [-2.0, -2.0, 4.5],
            voxel_size: 0.1,
            dims: [41, 41, 11],
        }
    }

    #[test]
    fn grid_spec_pads_bounds_and_counts_voxels() {
        let spec = GridSpec::from_bounds([0.0, 0.0, 0.0], [1.0, 2.0, 0.5], 0.1, 0.2).unwrap();
        assert_eq!(spec.dims, [14, 24, 9]);
        assert!((spec.origin[0] - (-0.2 + 0.05)).abs() < 1e-9);
        assert_eq!(spec.voxel_count(), 14 * 24 * 9);
        assert!(GridSpec::from_bounds([0.0; 3], [1.0; 3], 0.0, 0.0).is_err());
    }

    #[test]
    fn fuses_signed_distance_sign_against_the_flat_depth_map() {
        let mut grid = TsdfGrid::new(plane_grid());
        let frames = [flat_frame(100, 80, 5.0)];
        // 3-voxel band = 0.3, so 4.8 and 5.2 are both inside it.
        let options = FuseOptions::new(0.1, 3.0, 1);
        let observations = fuse_into(&mut grid, &frames, &options).unwrap();
        assert!(observations > 0);
        // Depth 5.0 with the voxel in front of the surface (4.8) is positive.
        let front = grid.value_at(20, 20, 3).expect("voxel at 4.8 observed");
        assert!((front - 0.2).abs() < 1e-4, "got {front}");
        // Behind the surface (5.2) is negative.
        let back = grid.value_at(20, 20, 7).expect("voxel at 5.2 observed");
        assert!((back + 0.2).abs() < 1e-4, "got {back}");
    }

    #[test]
    fn skips_voxels_outside_the_truncation_band() {
        let mut grid = TsdfGrid::new(plane_grid());
        let frames = [flat_frame(100, 80, 5.0)];
        fuse_into(&mut grid, &frames, &FuseOptions::new(0.1, 1.0, 1)).unwrap();
        assert!(
            grid.value_at(20, 20, 0).is_none(),
            "voxel at 4.5 is out of band"
        );
        assert!(
            grid.value_at(20, 20, 10).is_none(),
            "voxel at 5.5 is out of band"
        );
        assert!(
            grid.value_at(20, 20, 5).is_some(),
            "voxel at 5.0 is on the surface"
        );
    }

    #[test]
    fn requires_the_configured_minimum_observations() {
        let mut grid = TsdfGrid::new(plane_grid());
        let frames = [flat_frame(100, 80, 5.0)];
        fuse_into(&mut grid, &frames, &FuseOptions::new(0.1, 3.0, 2)).unwrap();
        assert_eq!(grid.observed_voxels(), 0, "single frame cannot meet min 2");
        assert_eq!(
            grid.observations[grid.spec.index(20, 20, 5)],
            1,
            "but the observation was still recorded"
        );
    }

    #[test]
    fn averages_observations_from_multiple_frames() {
        let mut grid = TsdfGrid::new(plane_grid());
        let frames = [flat_frame(100, 80, 5.0), flat_frame(100, 80, 5.2)];
        fuse_into(&mut grid, &frames, &FuseOptions::new(0.1, 3.0, 1)).unwrap();
        // Depth 5.0 and 5.2 average to 5.1 at the voxel sitting on 5.1.
        let value = grid.value_at(20, 20, 6).expect("observed twice");
        assert!(
            value.abs() < 0.02,
            "averaged value {value} should be near 0"
        );
    }

    #[test]
    fn ignores_frames_that_do_not_cover_the_voxel() {
        let mut grid = TsdfGrid::new(plane_grid());
        let mut narrow = flat_frame(100, 80, 5.0);
        // Blank out the frame: nothing is observed at all.
        narrow.depth.depths.fill(f32::INFINITY);
        let observations = fuse_into(&mut grid, &[narrow], &FuseOptions::new(0.1, 2.0, 1)).unwrap();
        assert_eq!(observations, 0);
        assert_eq!(grid.fused_voxels, 0);
    }

    #[test]
    fn analytic_grid_keeps_only_the_band() {
        let spec = GridSpec::from_bounds([-1.0; 3], [1.0; 3], 0.05, 0.0).unwrap();
        let grid = TsdfGrid::from_signed_distance(spec, 0.2, |p| {
            (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - 1.0
        });
        assert!(grid.fused_voxels > 0);
        assert!(grid.observed_voxels() < spec.voxel_count());
        let centre = grid.spec.dims[0] / 2;
        assert!(grid.value_at(centre, centre, centre).is_none());
    }

    #[test]
    fn gradient_points_away_from_the_sphere_centre() {
        let spec = GridSpec::from_bounds([-1.0; 3], [1.0; 3], 0.05, 0.0).unwrap();
        let grid = TsdfGrid::from_signed_distance(spec, 0.3, |p| {
            (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - 1.0
        });
        let centre = grid.spec.dims[0] / 2;
        // Walk outwards along +x until the surface band starts.
        let mut i = centre;
        while grid.value_at(i, centre, centre).is_none() && i < grid.spec.dims[0] - 2 {
            i += 1;
        }
        let gradient = grid.gradient_at(i, centre, centre);
        assert!(gradient[0] > 0.5, "gradient {gradient:?} should point +x");
        assert!(gradient[1].abs() < 0.3 && gradient[2].abs() < 0.3);
    }
}
