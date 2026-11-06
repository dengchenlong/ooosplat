//! Per-frame depth rendering of the object splats.
//!
//! Each splat is rasterised as a constant-depth disk whose radius comes from
//! `CameraProjection::footprint_radius_px`, and a per-pixel minimum depth
//! z-buffer keeps the nearest surface. The resulting map feeds the TSDF.

use crate::error::{Result, SplatError};
use crate::reconstruction::backproject::CameraProjection;
use crate::reconstruction::colmap_model::{CameraIntrinsics, ImagePose};
use crate::reconstruction::ply::SplatCore;

/// A single-channel depth image in camera-space units.
///
/// `depths` is row-major; non-finite entries mark pixels no splat covered.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthMap {
    pub width: u32,
    pub height: u32,
    pub depths: Vec<f32>,
}

impl DepthMap {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            depths: vec![f32::INFINITY; (width as usize) * (height as usize)],
        }
    }

    pub fn depth_at(&self, x: u32, y: u32) -> Option<f64> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let value = self.depths[(y as usize) * (self.width as usize) + x as usize];
        value.is_finite().then_some(value as f64)
    }

    pub fn valid_pixels(&self) -> usize {
        self.depths.iter().filter(|value| value.is_finite()).count()
    }
}

/// Rasterisation knobs. Radii are clamped so a single close-range splat
/// cannot flood the frame, and a sub-pixel splat still marks one pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthRasterOptions {
    pub max_radius_px: f64,
    pub min_radius_px: f64,
}

impl Default for DepthRasterOptions {
    fn default() -> Self {
        Self {
            max_radius_px: 16.0,
            // An object cloud is a sparse point sample of its surface: at
            // 1.5 px a splat always paints a small disk, which is what keeps
            // neighbouring splats from leaving pinholes in the depth map.
            min_radius_px: 1.5,
        }
    }
}

/// Downsamples the intrinsics by `scale` so a 540x960 camera renders into
/// 270x480 at `scale = 2`. Distortion coefficients are dimensionless and are
/// carried over unchanged; `width`/`height` round up.
pub fn scale_intrinsics(intrinsics: &CameraIntrinsics, scale: u32) -> Result<CameraIntrinsics> {
    if scale == 0 {
        return Err(SplatError::Process("深度图降采样倍率必须大于 0".into()));
    }
    if scale == 1 {
        return Ok(intrinsics.clone());
    }
    let mut scaled = intrinsics.clone();
    let factor = scale as f64;
    scaled.camera.width = intrinsics.camera.width.div_ceil(scale as u64);
    scaled.camera.height = intrinsics.camera.height.div_ceil(scale as u64);
    // The parameter layout depends on the camera model; only the ones the
    // back-projection stage supports carry focal lengths and a principal
    // point, and every one of those stores them first.
    for index in 0..scaled.params.len() {
        let scaled_value = match (intrinsics.camera.model_id, index) {
            // SIMPLE_PINHOLE: f, cx, cy.
            (0, 0) => intrinsics.params[0] / factor,
            // PINHOLE / SIMPLE_RADIAL / RADIAL: fx, fy, cx, cy, ... (RADIAL has
            // a single f for SIMPLE_RADIAL, handled above).
            (1 | 4, 0 | 1) => intrinsics.params[index] / factor,
            (2 | 3, 0) => intrinsics.params[0] / factor,
            // Principal points.
            (0, 1 | 2) | (2 | 3, 1 | 2) | (1 | 4, 2 | 3) => intrinsics.params[index] / factor,
            // Distortion terms are scale-free.
            _ => intrinsics.params[index],
        };
        scaled.params[index] = scaled_value;
    }
    Ok(scaled)
}

/// Renders the splats into a depth map at the resolution the projection was
/// built for. Returns an empty map when no splat is visible.
pub fn render_depth(
    splats: &[SplatCore],
    projection: &CameraProjection,
    pose: &ImagePose,
    width: u32,
    height: u32,
    options: &DepthRasterOptions,
) -> DepthMap {
    let mut map = DepthMap::new(width, height);
    if width == 0 || height == 0 {
        return map;
    }
    let max_radius = options.max_radius_px.max(options.min_radius_px);
    let min_radius = options.min_radius_px.max(0.0);
    for splat in splats {
        let Some([px, py]) = projection.world_to_pixel(pose, splat.position) else {
            continue;
        };
        let depth = projection.depth(pose, splat.position);
        if !(depth.is_finite() && depth > 0.0) {
            continue;
        }
        let radius = projection
            .footprint_radius_px(splat.scale, depth)
            .clamp(min_radius, max_radius);
        // Splats smaller than a pixel still mark their centre pixel; larger
        // ones paint a disk, which is what keeps a sparse cloud from leaving
        // pinholes between neighbouring splats.
        let reach = radius.ceil() as i64;
        let centre_x = px.round() as i64;
        let centre_y = py.round() as i64;
        let radius_squared = radius * radius;
        for dy in -reach..=reach {
            let y = centre_y + dy;
            if y < 0 || y >= height as i64 {
                continue;
            }
            for dx in -reach..=reach {
                if (dx * dx + dy * dy) as f64 > radius_squared && !(dx == 0 && dy == 0) {
                    continue;
                }
                let x = centre_x + dx;
                if x < 0 || x >= width as i64 {
                    continue;
                }
                let index = (y as usize) * (width as usize) + x as usize;
                let depth = depth as f32;
                if depth < map.depths[index] {
                    map.depths[index] = depth;
                }
            }
        }
    }
    map
}

/// Closes interior gaps left by sparse splat coverage.
///
/// Only pixels whose axis neighbours agree on a depth are filled, and the
/// fill value is the median of those neighbours: silhouette boundaries stay
/// put, so the mesh is not inflated beyond the observed object.
pub fn fill_holes(depth: &mut DepthMap, radius: u32) {
    if radius == 0 || depth.width == 0 || depth.height == 0 {
        return;
    }
    let width = depth.width as i64;
    let height = depth.height as i64;
    let source = depth.depths.clone();
    let at = |x: i64, y: i64| -> Option<f64> {
        if x < 0 || y < 0 || x >= width || y >= height {
            return None;
        }
        let value = source[(y * width + x) as usize];
        value.is_finite().then_some(value as f64)
    };
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            if source[index].is_finite() {
                continue;
            }
            let mut neighbours = Vec::with_capacity(4);
            for (dx, dy) in [(1_i64, 0_i64), (-1, 0), (0, 1), (0, -1)] {
                for step in 1..=radius as i64 {
                    if let Some(value) = at(x + dx * step, y + dy * step) {
                        neighbours.push(value);
                        break;
                    }
                }
            }
            if neighbours.len() < 3 {
                continue;
            }
            let min = neighbours.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = neighbours.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            // Bridging across an occlusion edge would create a phantom wall;
            // 5% of the local depth is far tighter than any real surface
            // slope over one pixel.
            if max - min > (0.05 * min).max(1e-4) {
                continue;
            }
            neighbours.sort_by(|a, b| a.partial_cmp(b).expect("finite depths"));
            let fill = neighbours[neighbours.len() / 2];
            depth.depths[index] = fill as f32;
        }
    }
}

/// Smooths the depth map to remove the piecewise-constant staircase that
/// rasterising splats as flat disks leaves behind.
///
/// Each pass replaces a pixel with the median of the valid pixels in its 3x3
/// neighbourhood, but only when the median stays within `tolerance` (relative)
/// of the original value — so genuine depth discontinuities, which is where
/// two surfaces overlap, survive instead of being smeared into a ramp.
pub fn smooth_depth(depth: &mut DepthMap, passes: u32, tolerance: f64) {
    if depth.width == 0 || depth.height == 0 {
        return;
    }
    let width = depth.width as i64;
    let height = depth.height as i64;
    for _ in 0..passes {
        let source = depth.depths.clone();
        for y in 0..height {
            for x in 0..width {
                let index = (y * width + x) as usize;
                let centre = source[index] as f64;
                if !centre.is_finite() {
                    continue;
                }
                let mut window = [0.0_f64; 9];
                let mut count = 0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < 0 || ny < 0 || nx >= width || ny >= height {
                            continue;
                        }
                        let value = source[(ny * width + nx) as usize];
                        if value.is_finite() {
                            window[count] = value as f64;
                            count += 1;
                        }
                    }
                }
                if count < 3 {
                    continue;
                }
                let slice = &mut window[..count];
                slice.sort_by(|a, b| a.partial_cmp(b).expect("finite depths"));
                let median = slice[count / 2];
                if (median - centre).abs() <= tolerance * centre.abs().max(1e-6) {
                    depth.depths[index] = median as f32;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::colmap_model::ColmapCamera;

    fn intrinsics(width: u64, height: u64, f: f64) -> CameraIntrinsics {
        // SIMPLE_RADIAL: f, cx, cy, k.
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

    fn splat(position: [f64; 3], sigma: f64) -> SplatCore {
        SplatCore {
            position,
            scale: [sigma.ln(); 3],
        }
    }

    #[test]
    fn renders_the_nearest_splat_depth_at_the_projected_pixel() {
        let intrinsics = intrinsics(100, 80, 100.0);
        let projection = CameraProjection::from_intrinsics(&intrinsics).unwrap();
        let splats = [splat([0.0, 0.0, 5.0], 0.01), splat([0.0, 0.0, 3.0], 0.01)];
        let map = render_depth(
            &splats,
            &projection,
            &identity_pose(),
            100,
            80,
            &DepthRasterOptions::default(),
        );
        let center = map.depth_at(50, 40).expect("center pixel covered");
        assert!((center - 3.0).abs() < 1e-6, "got {center}");
        assert!(map.depth_at(0, 0).is_none(), "corner must stay empty");
    }

    #[test]
    fn disks_cover_a_footprint_around_the_center() {
        let intrinsics = intrinsics(100, 80, 100.0);
        let projection = CameraProjection::from_intrinsics(&intrinsics).unwrap();
        // sigma 0.05 at depth 5 -> 1 px radius.
        let splats = [splat([0.0, 0.0, 5.0], 0.05)];
        let map = render_depth(
            &splats,
            &projection,
            &identity_pose(),
            100,
            80,
            &DepthRasterOptions::default(),
        );
        assert!(map.depth_at(51, 40).is_some());
        assert!(map.valid_pixels() >= 5, "got {}", map.valid_pixels());
        assert!(map.depth_at(70, 40).is_none());
    }

    #[test]
    fn splats_behind_the_camera_are_skipped() {
        let intrinsics = intrinsics(100, 80, 100.0);
        let projection = CameraProjection::from_intrinsics(&intrinsics).unwrap();
        let splats = [splat([0.0, 0.0, -5.0], 0.01)];
        let map = render_depth(
            &splats,
            &projection,
            &identity_pose(),
            100,
            80,
            &DepthRasterOptions::default(),
        );
        assert_eq!(map.valid_pixels(), 0);
    }

    #[test]
    fn fill_holes_closes_interior_gaps_without_growing_the_silhouette() {
        let mut map = DepthMap::new(21, 21);
        for y in 8..=12 {
            for x in 8..=12 {
                map.depths[y * 21 + x] = 2.0;
            }
        }
        map.depths[10 * 21 + 10] = f32::INFINITY;
        fill_holes(&mut map, 2);
        assert!((map.depth_at(10, 10).expect("interior hole filled") - 2.0).abs() < 1e-6);
        assert!(
            map.depth_at(7, 10).is_none(),
            "pixels outside the silhouette stay empty"
        );
    }

    #[test]
    fn smooth_depth_removes_staircase_but_keeps_discontinuities() {
        let mut map = DepthMap::new(9, 9);
        for y in 0..9 {
            for x in 0..9 {
                // A 0.02 ramp plus a hard 1.0-unit step at x == 4.
                map.depths[y * 9 + x] = if x < 4 { 5.0 } else { 6.0 };
            }
        }
        map.depths[4 * 9 + 1] = 5.02;
        smooth_depth(&mut map, 2, 0.02);
        assert!(
            (map.depth_at(1, 4).unwrap() - 5.0).abs() < 1e-6,
            "noise averaged out"
        );
        assert!(
            (map.depth_at(4, 4).unwrap() - 6.0).abs() < 1e-6,
            "the step survives at the median"
        );
        assert_eq!(map.depth_at(8, 8).unwrap(), 6.0);
    }

    #[test]
    fn smooth_depth_ignores_empty_pixels() {
        let mut map = DepthMap::new(5, 5);
        map.depths[2 * 5 + 2] = 3.0;
        smooth_depth(&mut map, 3, 0.5);
        assert_eq!(map.valid_pixels(), 1);
        assert_eq!(map.depth_at(2, 2).unwrap(), 3.0);
    }

    #[test]
    fn scales_intrinsics_and_resolution_for_half_resolution_depth() {
        let intrinsics = intrinsics(540, 960, 442.03);
        let scaled = scale_intrinsics(&intrinsics, 2).unwrap();
        assert_eq!(scaled.camera.width, 270);
        assert_eq!(scaled.camera.height, 480);
        assert!((scaled.params[0] - 221.015).abs() < 1e-9);
        assert!((scaled.params[1] - 135.0).abs() < 1e-9);
        assert!((scaled.params[2] - 240.0).abs() < 1e-9);
        assert_eq!(scaled.params[3], 0.0, "distortion is scale-free");
    }

    #[test]
    fn rejects_zero_scale() {
        assert!(scale_intrinsics(&intrinsics(540, 960, 442.03), 0).is_err());
    }
}
