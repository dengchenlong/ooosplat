//! End-to-end M1 mesh extraction: object splats + COLMAP model → depth
//! frames → TSDF → surface → vertex-coloured GLB.
//!
//! This is the library half of the `objmesh_mesh` CLI: the CLI only parses
//! arguments, calls `build_object_mesh` / `write_mesh_glb` and prints the
//! JSON summary.

use std::path::Path;

use serde::Serialize;

use crate::error::{Result, SplatError};
use crate::reconstruction::backproject::CameraProjection;
use crate::reconstruction::colmap_model::{read_camera_intrinsics, read_posed_images};
use crate::reconstruction::mesh::depth::{
    fill_holes, render_depth, scale_intrinsics, smooth_depth,
};
use crate::reconstruction::mesh::surface::{assign_nearest_colors, extract_surface};
use crate::reconstruction::mesh::tsdf::{fuse_into, DepthFrame, FuseOptions, GridSpec, TsdfGrid};
use crate::reconstruction::mesh::Mesh;
use crate::reconstruction::ply::{
    parse_gaussian_ply_layout, stream_colored_splats, stream_splat_cores, ColoredSplat, SplatCore,
};

/// Relative path of the object splat cloud inside a SplatStudio project.
pub const OBJECT_PLY_RELATIVE: &str = "assets/object.ply";

/// Mesh extraction parameters (one-to-one with the CLI flags).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshOptions {
    /// Explicit voxel size in scene units. When `None`, it is derived from
    /// the object AABB as `longest_edge / resolution`.
    pub voxel_size: Option<f64>,
    /// Voxels along the object's longest edge when `voxel_size` is `None`.
    pub resolution: usize,
    /// Truncation band, measured in voxels.
    pub truncation_voxels: f64,
    /// Voxels seen by fewer frames than this are dropped from the surface.
    pub min_observations: u32,
    /// Depth images are rendered at `1 / depth_scale` resolution.
    pub depth_scale: u32,
    /// When set, the longest mesh edge is rescaled to this many millimetres.
    pub scale_mm: Option<f64>,
    /// Cap on the number of COLMAP frames fused (first N in model order).
    pub max_frames: Option<usize>,
    /// Spatial-hash cell for nearest-splat colour lookup; defaults to two
    /// voxels.
    pub color_cell_size: Option<f64>,
}

impl Default for MeshOptions {
    fn default() -> Self {
        Self {
            voxel_size: None,
            resolution: 256,
            truncation_voxels: 4.0,
            min_observations: 2,
            depth_scale: 2,
            scale_mm: None,
            max_frames: None,
            color_cell_size: None,
        }
    }
}

/// The extracted mesh plus the numbers an acceptance run must be able to
/// check: splat count, fused frames, vertex/triangle counts, AABB and voxel
/// size.
#[derive(Debug, Clone)]
pub struct MeshBuild {
    pub mesh: Mesh,
    pub splats: u64,
    pub frames: usize,
    pub voxels_fused: u64,
    pub voxel_size: f64,
    /// World-space AABB of the object splats, in scene units.
    pub object_bounds: ([f64; 3], [f64; 3]),
}

/// JSON summary printed as the last line of the CLI output.
#[derive(Debug, Clone, Serialize)]
pub struct MeshSummary {
    pub splats: u64,
    pub frames: usize,
    pub voxels_fused: u64,
    pub vertices: usize,
    pub triangles: usize,
    /// `[min, max]` of the mesh vertices, after any `scale_mm` rescaling.
    pub bbox: [[f64; 3]; 2],
    /// `[min, max]` of the source object splats, in scene units.
    pub object_bbox: [[f64; 3]; 2],
    pub voxel_size: f64,
    pub out: String,
}

impl MeshSummary {
    pub fn from_build(build: &MeshBuild, out: impl Into<String>) -> Self {
        let (min, max) = build.mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
        Self {
            splats: build.splats,
            frames: build.frames,
            voxels_fused: build.voxels_fused,
            vertices: build.mesh.positions.len(),
            triangles: build.mesh.triangle_count(),
            bbox: [min, max],
            object_bbox: [build.object_bounds.0, build.object_bounds.1],
            voxel_size: build.voxel_size,
            out: out.into(),
        }
    }
}

/// Reads and validates the object splat cloud for `project_root`.
pub fn object_ply_path(project_root: &Path) -> Result<std::path::PathBuf> {
    let path = project_root.join(OBJECT_PLY_RELATIVE);
    if !path.is_file() {
        return Err(SplatError::Process(format!(
            "缺少物体泼溅 {OBJECT_PLY_RELATIVE}（{}）；先运行 objmesh_backproject 拆分物体",
            path.display()
        )));
    }
    Ok(path)
}

fn bounds_of(splats: &[SplatCore]) -> Result<([f64; 3], [f64; 3])> {
    let first = splats
        .first()
        .ok_or_else(|| SplatError::Process("物体泼溅为空".into()))?;
    let mut min = first.position;
    let mut max = first.position;
    for splat in splats {
        for axis in 0..3 {
            min[axis] = min[axis].min(splat.position[axis]);
            max[axis] = max[axis].max(splat.position[axis]);
        }
    }
    Ok((min, max))
}

/// Renders the object depth for every (or the first `max_frames`) COLMAP
/// frame at the configured resolution.
fn render_frames(
    splats: &[SplatCore],
    model_dir: &Path,
    options: &MeshOptions,
) -> Result<Vec<DepthFrame>> {
    let intrinsics = read_camera_intrinsics(model_dir)?;
    if intrinsics.camera.width == 0 || intrinsics.camera.height == 0 {
        return Err(SplatError::Process("COLMAP 相机分辨率为零".into()));
    }
    let scaled = scale_intrinsics(&intrinsics, options.depth_scale)?;
    let width = u32::try_from(scaled.camera.width)
        .map_err(|_| SplatError::Process("深度图宽度超出范围".into()))?;
    let height = u32::try_from(scaled.camera.height)
        .map_err(|_| SplatError::Process("深度图高度超出范围".into()))?;
    let projection = CameraProjection::from_intrinsics(&scaled)?;
    let images = read_posed_images(model_dir)?;
    if images.is_empty() {
        return Err(SplatError::Process("COLMAP 模型不包含任何注册图像".into()));
    }
    let limit = options.max_frames.unwrap_or(images.len()).min(images.len());
    let raster = crate::reconstruction::mesh::depth::DepthRasterOptions::default();
    let mut frames = Vec::with_capacity(limit);
    for image in images.iter().take(limit) {
        let mut depth = render_depth(splats, &projection, &image.pose, width, height, &raster);
        // Sparse splat clouds leave 1-2 px pinholes and a staircase profile;
        // fill the holes, then median-smooth within a depth tolerance.
        fill_holes(&mut depth, 3);
        fill_holes(&mut depth, 3);
        smooth_depth(&mut depth, 1, 0.05);
        frames.push(DepthFrame {
            projection,
            pose: image.pose,
            depth,
        });
    }
    Ok(frames)
}

/// Runs the whole pipeline for one project. Never writes to the project:
/// the caller decides where the GLB goes.
pub fn build_object_mesh(
    project_root: &Path,
    model_dir: &Path,
    options: &MeshOptions,
) -> Result<MeshBuild> {
    let ply = object_ply_path(project_root)?;
    // Reuses the existing streaming reader for positions/scales; the layout
    // check also raises a clear error for a truncated header.
    let layout = parse_gaussian_ply_layout(&ply)?;
    let mut splats: Vec<SplatCore> = Vec::with_capacity(layout.splat_count as usize);
    stream_splat_cores(&ply, 4096, &mut |batch| {
        splats.extend_from_slice(batch);
        Ok(())
    })?;
    let (min, max) = bounds_of(&splats)?;
    let longest_edge = (0..3)
        .map(|axis| max[axis] - min[axis])
        .fold(0.0_f64, f64::max);
    let voxel_size = match options.voxel_size {
        Some(value) if value.is_finite() && value > 0.0 => value,
        Some(_) => return Err(SplatError::Process("体素尺寸必须是正数".into())),
        None => {
            if options.resolution == 0 {
                return Err(SplatError::Process("--resolution 必须大于 0".into()));
            }
            longest_edge / options.resolution as f64
        }
    };
    if !(voxel_size.is_finite() && voxel_size > 0.0) {
        return Err(SplatError::Process("无法从物体包围盒推导体素尺寸".into()));
    }
    let truncation = voxel_size * options.truncation_voxels;

    let frames = render_frames(&splats, model_dir, options)?;
    let spec = GridSpec::from_bounds(min, max, voxel_size, truncation)?;
    let mut grid = TsdfGrid::new(spec);
    let fuse_options = FuseOptions {
        truncation,
        min_observations: options.min_observations,
    };
    fuse_into(&mut grid, &frames, &fuse_options)?;
    let voxels_fused = grid.fused_voxels;
    let mut mesh = extract_surface(&grid);

    let mut colored: Vec<ColoredSplat> = Vec::with_capacity(splats.len());
    stream_colored_splats(&ply, 4096, &mut |batch| {
        colored.extend_from_slice(batch);
        Ok(())
    })?;
    let cell = options.color_cell_size.unwrap_or(voxel_size * 2.0);
    assign_nearest_colors(&mut mesh, &colored, cell);

    if let Some(millimetres) = options.scale_mm {
        if !(millimetres.is_finite() && millimetres > 0.0) {
            return Err(SplatError::Process("--scale-mm 必须是正数".into()));
        }
        let edge = mesh.longest_edge();
        if edge > 0.0 {
            mesh.scale_positions((millimetres / 1000.0) / edge);
        }
    }

    Ok(MeshBuild {
        mesh,
        splats: layout.splat_count,
        frames: frames.len(),
        voxels_fused,
        voxel_size,
        object_bounds: (min, max),
    })
}

/// Writes the GLB and returns the summary with `out` filled in.
pub fn write_mesh_glb(path: &Path, build: &MeshBuild) -> Result<MeshSummary> {
    crate::reconstruction::mesh::glb::write_glb(path, &build.mesh)?;
    Ok(MeshSummary::from_build(build, path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::ply::SH_C0;
    use std::io::Write;

    const WIDTH: u32 = 320;
    const HEIGHT: u32 = 240;
    const FOCAL: f64 = 400.0;
    const ORBIT_DEPTH: f64 = 4.0;

    /// Rotation matrix (rows = camera axes in world) → COLMAP quaternion.
    fn rotation_to_quaternion(rotation: [[f64; 3]; 3]) -> [f64; 4] {
        let trace = rotation[0][0] + rotation[1][1] + rotation[2][2];
        let (w, x, y, z) = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            (
                0.25 * s,
                (rotation[2][1] - rotation[1][2]) / s,
                (rotation[0][2] - rotation[2][0]) / s,
                (rotation[1][0] - rotation[0][1]) / s,
            )
        } else if rotation[0][0] > rotation[1][1] && rotation[0][0] > rotation[2][2] {
            let s = (1.0 + rotation[0][0] - rotation[1][1] - rotation[2][2]).sqrt() * 2.0;
            (
                (rotation[2][1] - rotation[1][2]) / s,
                0.25 * s,
                (rotation[0][1] + rotation[1][0]) / s,
                (rotation[0][2] + rotation[2][0]) / s,
            )
        } else if rotation[1][1] > rotation[2][2] {
            let s = (1.0 + rotation[1][1] - rotation[0][0] - rotation[2][2]).sqrt() * 2.0;
            (
                (rotation[0][2] - rotation[2][0]) / s,
                (rotation[0][1] + rotation[1][0]) / s,
                0.25 * s,
                (rotation[1][2] + rotation[2][1]) / s,
            )
        } else {
            let s = (1.0 + rotation[2][2] - rotation[0][0] - rotation[1][1]).sqrt() * 2.0;
            (
                (rotation[1][0] - rotation[0][1]) / s,
                (rotation[0][2] + rotation[2][0]) / s,
                (rotation[1][2] + rotation[2][1]) / s,
                0.25 * s,
            )
        };
        [w, x, y, z]
    }

    /// Camera at `distance` looking at the origin, `+y` up in world space.
    fn look_at_origin(degrees_azimuth: f64, degrees_elevation: f64) -> ([[f64; 3]; 3], [f64; 3]) {
        let azimuth = degrees_azimuth.to_radians();
        let elevation = degrees_elevation.to_radians();
        // Camera position.
        let position = [
            ORBIT_DEPTH * elevation.cos() * azimuth.cos(),
            ORBIT_DEPTH * elevation.sin(),
            ORBIT_DEPTH * elevation.cos() * azimuth.sin(),
        ];
        // forward = -normalize(position); right = normalize(cross(forward, up_world));
        // down = cross(forward, right). Rows of R are (right, down, forward).
        let norm = ORBIT_DEPTH;
        let forward = [
            -position[0] / norm,
            -position[1] / norm,
            -position[2] / norm,
        ];
        let up_world = [0.0, 1.0, 0.0];
        let mut right = [
            forward[1] * up_world[2] - forward[2] * up_world[1],
            forward[2] * up_world[0] - forward[0] * up_world[2],
            forward[0] * up_world[1] - forward[1] * up_world[0],
        ];
        let right_norm = (right[0] * right[0] + right[1] * right[1] + right[2] * right[2]).sqrt();
        for value in &mut right {
            *value /= right_norm;
        }
        let down = [
            forward[1] * right[2] - forward[2] * right[1],
            forward[2] * right[0] - forward[0] * right[2],
            forward[0] * right[1] - forward[1] * right[0],
        ];
        let rotation = [right, down, forward];
        // t = -R * position.
        let mut translation = [0.0_f64; 3];
        for row in 0..3 {
            translation[row] = -(rotation[row][0] * position[0]
                + rotation[row][1] * position[1]
                + rotation[row][2] * position[2]);
        }
        (rotation, translation)
    }

    /// Fibonacci-distributed unit sphere splats, radius 1, centred at origin.
    fn sphere_splats(count: usize) -> Vec<([f64; 3], [f32; 3])> {
        let golden = std::f64::consts::PI * (1.0 + 5.0_f64.sqrt());
        (0..count)
            .map(|index| {
                let y = 1.0 - 2.0 * (index as f64 + 0.5) / count as f64;
                let r = (1.0 - y * y).max(0.0).sqrt();
                let theta = golden * index as f64;
                let position = [r * theta.cos(), y, r * theta.sin()];
                let color = [
                    ((y + 1.0) / 2.0) as f32,
                    ((position[0] + 1.0) / 2.0) as f32,
                    ((position[2] + 1.0) / 2.0) as f32,
                ];
                (position, color)
            })
            .collect()
    }

    fn write_object_ply(path: &Path, splats: &[([f64; 3], [f32; 3])]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let properties = "property float x\nproperty float y\nproperty float z\n\
property float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\n\
property float opacity\nproperty float scale_0\nproperty float scale_1\nproperty float scale_2\n\
property float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3\n";
        let header = format!(
            "ply\nformat binary_little_endian 1.0\nelement vertex {}\n{properties}end_header\n",
            splats.len()
        );
        let mut bytes = header.into_bytes();
        let log_sigma = 0.01_f64.ln() as f32;
        for (position, color) in splats {
            for value in position {
                bytes.extend_from_slice(&(*value as f32).to_le_bytes());
            }
            for channel in color {
                let dc = (*channel as f64 - 0.5) / SH_C0;
                bytes.extend_from_slice(&(dc as f32).to_le_bytes());
            }
            bytes.extend_from_slice(&0.9_f32.to_le_bytes());
            for _ in 0..3 {
                bytes.extend_from_slice(&log_sigma.to_le_bytes());
            }
            for _ in 0..4 {
                bytes.extend_from_slice(&0.5_f32.to_le_bytes());
            }
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn write_model(model_dir: &Path) {
        std::fs::create_dir_all(model_dir).unwrap();
        let mut cameras = std::fs::File::create(model_dir.join("cameras.bin")).unwrap();
        cameras.write_all(&1_u64.to_le_bytes()).unwrap();
        cameras.write_all(&1_u32.to_le_bytes()).unwrap();
        cameras.write_all(&2_i32.to_le_bytes()).unwrap();
        cameras.write_all(&(WIDTH as u64).to_le_bytes()).unwrap();
        cameras.write_all(&(HEIGHT as u64).to_le_bytes()).unwrap();
        for value in [FOCAL, WIDTH as f64 / 2.0, HEIGHT as f64 / 2.0, 0.0_f64] {
            cameras.write_all(&value.to_le_bytes()).unwrap();
        }

        let views: Vec<(f64, f64)> = (0..12)
            .map(|index| (index as f64 * 30.0, 0.0))
            .chain([(0.0, 40.0), (120.0, 40.0), (240.0, 40.0)])
            .chain([(60.0, -40.0), (180.0, -40.0), (300.0, -40.0)])
            .collect();
        let mut images = std::fs::File::create(model_dir.join("images.bin")).unwrap();
        images
            .write_all(&(views.len() as u64).to_le_bytes())
            .unwrap();
        for (index, (azimuth, elevation)) in views.iter().enumerate() {
            let (rotation, translation) = look_at_origin(*azimuth, *elevation);
            let quaternion = rotation_to_quaternion(rotation);
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
            images
                .write_all(format!("frame_{:06}.jpg", index + 1).as_bytes())
                .unwrap();
            images.write_all(&[0_u8]).unwrap();
            images.write_all(&0_u64.to_le_bytes()).unwrap();
        }
    }

    /// Writes a synthetic project: unit sphere of splats + orbiting cameras.
    fn sphere_project(root: &Path, splat_count: usize) {
        write_object_ply(&root.join(OBJECT_PLY_RELATIVE), &sphere_splats(splat_count));
        write_model(&root.join("work/colmap/sparse/0"));
    }

    #[test]
    fn rebuilds_a_synthetic_sphere_from_orbiting_views() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 30_000);
        let options = MeshOptions {
            voxel_size: Some(0.05),
            truncation_voxels: 3.0,
            min_observations: 1,
            depth_scale: 1,
            ..MeshOptions::default()
        };
        let build = build_object_mesh(root, &root.join("work/colmap/sparse/0"), &options).unwrap();
        assert_eq!(build.splats, 30_000);
        assert_eq!(build.frames, 18);
        assert!(build.voxels_fused > 0);
        let mesh = &build.mesh;
        assert!(
            mesh.positions.len() > 300,
            "sphere mesh should be dense, got {}",
            mesh.positions.len()
        );
        assert!(mesh.triangle_count() > 100);
        let (min, max) = mesh.bounds().unwrap();
        for axis in 0..3 {
            assert!(min[axis] > -1.15 && min[axis] < -0.85, "min {min:?}");
            assert!(max[axis] < 1.15 && max[axis] > 0.85, "max {max:?}");
        }
        assert_eq!(mesh.colors.len(), mesh.positions.len());
        assert!(
            mesh.colors.iter().any(|color| color[1] > 0.8),
            "the top of the sphere should carry its splat colour"
        );
    }

    #[test]
    fn summary_reports_the_mesh_and_source_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        let options = MeshOptions {
            voxel_size: Some(0.06),
            truncation_voxels: 3.0,
            min_observations: 1,
            depth_scale: 2,
            ..MeshOptions::default()
        };
        let build = build_object_mesh(root, &root.join("work/colmap/sparse/0"), &options).unwrap();
        let summary = MeshSummary::from_build(&build, "out.glb");
        assert_eq!(summary.splats, 8_000);
        assert_eq!(summary.vertices, build.mesh.positions.len());
        assert_eq!(summary.triangles, build.mesh.triangle_count());
        assert_eq!(summary.out, "out.glb");
        assert!((summary.object_bbox[0][0] + 1.0).abs() < 0.01);
        assert!((summary.object_bbox[1][1] - 1.0).abs() < 0.01);
        let json = serde_json::to_string(&summary).unwrap();
        for key in [
            "splats",
            "frames",
            "voxels_fused",
            "vertices",
            "triangles",
            "bbox",
            "voxel_size",
            "out",
        ] {
            assert!(
                json.contains(&format!("\"{key}\"")),
                "missing {key} in {json}"
            );
        }
    }

    #[test]
    fn scale_mm_rescales_the_longest_edge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        let options = MeshOptions {
            voxel_size: Some(0.06),
            min_observations: 1,
            depth_scale: 2,
            scale_mm: Some(200.0),
            ..MeshOptions::default()
        };
        let build = build_object_mesh(root, &root.join("work/colmap/sparse/0"), &options).unwrap();
        assert!(
            (build.mesh.longest_edge() - 0.2).abs() < 1e-3,
            "longest edge {} should be 0.2 m",
            build.mesh.longest_edge()
        );
    }

    #[test]
    fn derives_the_voxel_size_from_the_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 2_000);
        let options = MeshOptions {
            voxel_size: None,
            resolution: 40,
            min_observations: 1,
            depth_scale: 4,
            max_frames: Some(6),
            ..MeshOptions::default()
        };
        let build = build_object_mesh(root, &root.join("work/colmap/sparse/0"), &options).unwrap();
        // Longest splat edge is ~2.0 (unit sphere), so 2/40 = 0.05.
        assert!(
            (build.voxel_size - 0.05).abs() < 1e-3,
            "{}",
            build.voxel_size
        );
        assert_eq!(build.frames, 6);
    }

    #[test]
    fn rejects_a_project_without_an_object_cloud() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_model(&root.join("work/colmap/sparse/0"));
        let error = build_object_mesh(
            root,
            &root.join("work/colmap/sparse/0"),
            &MeshOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("object.ply"), "{error}");
    }

    #[test]
    fn rejects_an_invalid_voxel_size() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 100);
        let error = build_object_mesh(
            root,
            &root.join("work/colmap/sparse/0"),
            &MeshOptions {
                voxel_size: Some(0.0),
                ..MeshOptions::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("体素尺寸"), "{error}");
    }
}
