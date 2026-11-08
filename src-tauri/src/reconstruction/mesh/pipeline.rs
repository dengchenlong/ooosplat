//! End-to-end M1 mesh extraction: object splats + COLMAP model → depth
//! frames → TSDF → surface → vertex-coloured GLB.
//!
//! This is the library half of the `objmesh_mesh` CLI: the CLI only parses
//! arguments, calls `build_object_mesh` / `write_mesh_glb` and prints the
//! JSON summary.

use std::path::Path;

use serde::Serialize;
use uuid::Uuid;

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
///
/// The pre-#11 name, kept as a fallback so projects published before the
/// `object-XX.ply` layout still load.
pub const OBJECT_PLY_RELATIVE: &str = "assets/object.ply";

/// The numbered object cloud issue #11 publishes (`assets/object-01.ply`).
pub const OBJECT_PLY_RELATIVE_V2: &str = "assets/object-01.ply";

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
    /// Drop connected components holding less than this share of the
    /// triangles. `None` (the default) keeps every component, so the output
    /// is unchanged for callers that predate issue #17.
    pub min_component_ratio: Option<f64>,
    /// Keep only the largest connected component, whatever its ratio.
    pub keep_largest_only: bool,
    /// Simplify down to at most this many triangles.
    pub target_triangles: Option<usize>,
    /// Simplify to this share of the extracted triangles (0..1).
    pub simplify_ratio: Option<f64>,
    /// `(measured scene units, known millimetres)` calibration pair: the
    /// mesh is uniformly rescaled so the measured span equals that many
    /// millimetres.
    pub scale_from_mm_ref: Option<(f64, f64)>,
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
            min_component_ratio: None,
            keep_largest_only: false,
            target_triangles: None,
            simplify_ratio: None,
            scale_from_mm_ref: None,
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
    /// Connected components in the final mesh.
    pub components: usize,
    /// Components the cleanup pass dropped (`0` when it was not requested).
    pub components_removed: usize,
    /// Vertex/triangle counts before simplification, when it ran.
    pub vertices_before_simplify: Option<usize>,
    pub triangles_before_simplify: Option<usize>,
    /// Uniform factor applied to the positions (`1.0` when unscaled).
    pub scale_factor: f64,
    /// `"scene"` (raw COLMAP units) or `"m"` (calibrated, metric).
    pub units: &'static str,
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
    /// Components left after cleanup.
    pub components: usize,
    /// Components the cleanup pass dropped.
    pub components_removed: usize,
    /// Vertices before simplification (`None` when no simplification ran).
    pub vertices_before_simplify: Option<usize>,
    /// Triangles before simplification (`None` when no simplification ran).
    pub triangles_before_simplify: Option<usize>,
    /// Uniform factor applied to the positions (`1.0` when unscaled).
    pub scale_factor: f64,
    /// Physical unit of the reported coordinates: `"scene"` for the raw
    /// COLMAP scale, `"m"` once calibration made the mesh metric.
    pub units: String,
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
            components: build.components,
            components_removed: build.components_removed,
            vertices_before_simplify: build.vertices_before_simplify,
            triangles_before_simplify: build.triangles_before_simplify,
            scale_factor: build.scale_factor,
            units: build.units.to_string(),
            out: out.into(),
        }
    }
}

/// Reads and validates the object splat cloud for `project_root`.
///
/// Prefers the issue #11 layout (`assets/object-*.ply`, lowest index first)
/// and falls back to the pre-#11 `assets/object.ply`, so both generations of
/// projects load and a project that has both keeps using the numbered one.
pub fn object_ply_path(project_root: &Path) -> Result<std::path::PathBuf> {
    if let Some((_, path)) = crate::reconstruction::assets::object_ply_assets(
        &project_root.join(crate::reconstruction::assets::ASSETS_DIR),
    )
    .into_iter()
    .next()
    {
        return Ok(path);
    }
    let legacy = project_root.join(OBJECT_PLY_RELATIVE);
    if legacy.is_file() {
        return Ok(legacy);
    }
    let path = project_root.join(OBJECT_PLY_RELATIVE_V2);
    Err(SplatError::Process(format!(
        "缺少物体泼溅 {OBJECT_PLY_RELATIVE_V2} 或 {OBJECT_PLY_RELATIVE}（{}）；先运行 objmesh_backproject 拆分物体",
        path.display()
    )))
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

    // --- issue #17: component cleanup, simplification, scale calibration ---
    let components_before = crate::reconstruction::mesh::cleanup::components(&mesh).len();
    if options.keep_largest_only || options.min_component_ratio.is_some() {
        let ratio = options.min_component_ratio.unwrap_or(0.0);
        if !ratio.is_finite() || ratio < 0.0 || ratio > 1.0 {
            return Err(SplatError::Process(
                "--min-component-ratio must be within 0..1".into(),
            ));
        }
        mesh = crate::reconstruction::mesh::cleanup::clean_components(
            &mesh,
            ratio,
            options.keep_largest_only,
        );
        if mesh.is_empty() {
            return Err(SplatError::Process(
                "连通分量清理后没有任何三角形；检查 --min-component-ratio".into(),
            ));
        }
    }
    let components_after = crate::reconstruction::mesh::cleanup::components(&mesh).len();

    let mut vertices_before_simplify = None;
    let mut triangles_before_simplify = None;
    if options.target_triangles.is_some() || options.simplify_ratio.is_some() {
        let target = match (options.target_triangles, options.simplify_ratio) {
            (Some(target), _) => target,
            (None, Some(ratio)) => {
                if !ratio.is_finite() || ratio <= 0.0 || ratio > 1.0 {
                    return Err(SplatError::Process(
                        "--simplify-ratio must be within (0, 1]".into(),
                    ));
                }
                ((mesh.triangle_count() as f64) * ratio).round() as usize
            }
            (None, None) => unreachable!("guarded by the outer condition"),
        };
        if target == 0 {
            return Err(SplatError::Process("简化目标不能为 0 三角形".into()));
        }
        vertices_before_simplify = Some(mesh.positions.len());
        triangles_before_simplify = Some(mesh.triangle_count());
        mesh = crate::reconstruction::mesh::cleanup::simplify(&mesh, target);
        if mesh.is_empty() {
            return Err(SplatError::Process("简化后没有任何三角形".into()));
        }
    }

    // Scale: `--scale-from-mm-ref` is the calibrated (metric) path and wins
    // over the legacy `--scale-mm` longest-edge rescale.
    let mut scale_factor = 1.0_f64;
    let mut units = "scene";
    if let Some((measured, millimetres)) = options.scale_from_mm_ref {
        let factor =
            crate::reconstruction::mesh::cleanup::scale_factor_from_mm_ref(measured, millimetres)
                .ok_or_else(|| {
                SplatError::Process("--scale-from-mm-ref 需要两个正数：<实测长度>:<毫米>".into())
            })?;
        mesh.scale_positions(factor);
        scale_factor = factor;
        units = "m";
    } else if let Some(millimetres) = options.scale_mm {
        if !(millimetres.is_finite() && millimetres > 0.0) {
            return Err(SplatError::Process("--scale-mm 必须是正数".into()));
        }
        let edge = mesh.longest_edge();
        if edge > 0.0 {
            scale_factor = (millimetres / 1000.0) / edge;
            mesh.scale_positions(scale_factor);
            units = "m";
        }
    }

    Ok(MeshBuild {
        mesh,
        splats: layout.splat_count,
        frames: frames.len(),
        voxels_fused,
        voxel_size,
        object_bounds: (min, max),
        components: components_after,
        components_removed: components_before.saturating_sub(components_after),
        vertices_before_simplify,
        triangles_before_simplify,
        scale_factor,
        units,
    })
}

/// Writes the GLB and returns the summary with `out` filled in.
pub fn write_mesh_glb(path: &Path, build: &MeshBuild) -> Result<MeshSummary> {
    crate::reconstruction::mesh::glb::write_glb(path, &build.mesh)?;
    Ok(MeshSummary::from_build(build, path.display().to_string()))
}

/// Publishes the build as `<project_root>/assets/object-01.glb` (issue #11).
///
/// The GLB goes through a temporary file inside `assets/` and an atomic
/// replace, then `assets/manifest.json` records it with the mesh's vertex and
/// triangle counts, unit and scale factor. The returned summary carries the
/// published path in `out`, so the CLI's stdout contract is unchanged. A mesh
/// stage that fails leaves neither a partial GLB nor a manifest entry.
pub fn publish_mesh_glb(project_root: &Path, build: &MeshBuild) -> Result<MeshSummary> {
    use crate::reconstruction::assets::{
        manifest_path, object_glb_name, publish_file, sha256_file, source_from_final_ply,
        upsert_asset_entry, AssetEntry, AssetFormat, AssetKind, ASSETS_DIR, FIRST_OBJECT_INDEX,
    };

    let assets = project_root.join(ASSETS_DIR);
    std::fs::create_dir_all(&assets)?;
    let index = FIRST_OBJECT_INDEX;
    let destination = assets.join(object_glb_name(index));
    let temporary = assets.join(format!(".{}.tmp", Uuid::new_v4()));

    let output = (|| -> Result<_> {
        let output = crate::reconstruction::mesh::glb::encode_glb(&build.mesh)?;
        std::fs::write(&temporary, &output.bytes)?;
        Ok(output)
    })();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    // Resolve the manifest `source` before publishing anything: a project
    // without a readable final.ply must fail with nothing published, not with
    // a GLB on disk that no manifest entry describes.
    let source = match source_from_final_ply(project_root) {
        Ok(source) => source,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    // Validation of a produced artifact is not the process exit code: re-parse
    // the container we are about to publish before publishing it.
    let publish = (|| -> Result<()> {
        let bytes = std::fs::read(&temporary)?;
        let (json, bin) = crate::reconstruction::mesh::glb::split_glb(&bytes)?;
        let accessors = json["accessors"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let counted = |position: usize| {
            accessors
                .get(position)
                .and_then(|accessor| accessor["count"].as_u64())
                .unwrap_or(0)
        };
        let declared = json["buffers"]
            .get(0)
            .and_then(|buffer| buffer["byteLength"].as_u64());
        // POSITION count, index count (3 per triangle) and the BIN chunk must
        // all agree with what the writer reported, otherwise the file that
        // would be published is not the mesh this run measured.
        if counted(0) != output.vertex_count as u64
            || counted(3) != (output.triangle_count as u64) * 3
            || declared != Some(bin.len() as u64)
        {
            return Err(SplatError::Process(
                "GLB 重新解析后的顶点/三角形/缓冲长度与写出结果不一致".into(),
            ));
        }
        Ok(())
    })();
    if let Err(error) = publish {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }

    if let Err(error) = publish_file(&temporary, &destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    let entry = AssetEntry::new(
        AssetKind::Object,
        index,
        AssetFormat::Glb,
        object_glb_name(index),
        destination.metadata()?.len(),
        sha256_file(&destination)?,
    )
    .with_mesh(
        output.vertex_count as u64,
        output.triangle_count as u64,
        build.units,
        build.scale_factor,
    );
    upsert_asset_entry(
        &assets,
        source,
        entry,
        crate::reconstruction::assets::now_ms(),
    )?;
    debug_assert_eq!(manifest_path(&assets), assets.join("manifest.json"));
    Ok(MeshSummary::from_build(
        build,
        destination.display().to_string(),
    ))
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
    ///
    /// `final.ply` is written too (same splats), because issue #11 publishes the
    /// mesh into a manifest whose `source` block hashes and counts it.
    fn sphere_project(root: &Path, splat_count: usize) {
        let splats = sphere_splats(splat_count);
        write_object_ply(&root.join(OBJECT_PLY_RELATIVE_V2), &splats);
        write_object_ply(&root.join("final.ply"), &splats);
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

    fn sphere_options() -> MeshOptions {
        MeshOptions {
            voxel_size: Some(0.06),
            min_observations: 1,
            depth_scale: 2,
            ..MeshOptions::default()
        }
    }

    #[test]
    fn reports_components_without_changing_the_default_output() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        let model = root.join("work/colmap/sparse/0");
        let plain = build_object_mesh(root, &model, &sphere_options()).unwrap();
        // A component filter that cannot drop anything must be a no-op.
        let filtered = build_object_mesh(
            root,
            &model,
            &MeshOptions {
                min_component_ratio: Some(0.0),
                ..sphere_options()
            },
        )
        .unwrap();
        assert_eq!(plain.mesh, filtered.mesh);
        assert_eq!(plain.components, filtered.components);
        assert_eq!(plain.components_removed, 0);
        assert_eq!(filtered.components_removed, 0);
        assert!(plain.components >= 1);
        assert_eq!(plain.scale_factor, 1.0);
        assert_eq!(plain.units, "scene");
    }

    #[test]
    fn keep_largest_only_leaves_exactly_one_component() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        let model = root.join("work/colmap/sparse/0");
        let build = build_object_mesh(
            root,
            &model,
            &MeshOptions {
                keep_largest_only: true,
                ..sphere_options()
            },
        )
        .unwrap();
        assert_eq!(build.components, 1);
        assert!(build.mesh.triangle_count() > 100);
        let parts = crate::reconstruction::mesh::cleanup::components(&build.mesh);
        assert_eq!(parts.len(), 1);
        assert!((parts[0].ratio - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rejects_an_out_of_range_component_ratio() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 2_000);
        let error = build_object_mesh(
            root,
            &root.join("work/colmap/sparse/0"),
            &MeshOptions {
                min_component_ratio: Some(1.5),
                ..sphere_options()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("min-component-ratio"), "{error}");
    }

    #[test]
    fn target_triangles_simplifies_and_reports_the_previous_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        let model = root.join("work/colmap/sparse/0");
        let plain = build_object_mesh(root, &model, &sphere_options()).unwrap();
        let before = plain.mesh.triangle_count();
        assert!(before > 400, "need a mesh worth simplifying: {before}");
        let target = before / 2;
        let build = build_object_mesh(
            root,
            &model,
            &MeshOptions {
                target_triangles: Some(target),
                ..sphere_options()
            },
        )
        .unwrap();
        assert!(
            build.mesh.triangle_count() <= target,
            "{} > {target}",
            build.mesh.triangle_count()
        );
        assert!(build.mesh.triangle_count() > target / 2);
        assert_eq!(build.triangles_before_simplify, Some(before));
        assert_eq!(
            build.vertices_before_simplify,
            Some(plain.mesh.positions.len())
        );
        assert_eq!(build.mesh.colors.len(), build.mesh.positions.len());
        assert_eq!(build.mesh.normals.len(), build.mesh.positions.len());
        let (min, max) = build.mesh.bounds().unwrap();
        for axis in 0..3 {
            assert!(min[axis] > -1.2 && max[axis] < 1.2, "bbox {min:?} {max:?}");
        }
    }

    #[test]
    fn simplify_ratio_is_relative_to_the_extracted_triangles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 6_000);
        let model = root.join("work/colmap/sparse/0");
        let plain = build_object_mesh(root, &model, &sphere_options()).unwrap();
        let before = plain.mesh.triangle_count();
        let build = build_object_mesh(
            root,
            &model,
            &MeshOptions {
                simplify_ratio: Some(0.5),
                ..sphere_options()
            },
        )
        .unwrap();
        let target = (before as f64 * 0.5).round() as usize;
        assert!(build.mesh.triangle_count() <= target);
        assert_eq!(build.triangles_before_simplify, Some(before));
    }

    #[test]
    fn scale_from_mm_ref_makes_the_measured_span_metric() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        let model = root.join("work/colmap/sparse/0");
        let plain = build_object_mesh(root, &model, &sphere_options()).unwrap();
        let measured = plain.mesh.longest_edge();
        assert!(measured > 1.5 && measured < 2.5, "measured {measured}");
        let build = build_object_mesh(
            root,
            &model,
            &MeshOptions {
                scale_from_mm_ref: Some((measured, 1000.0)),
                ..sphere_options()
            },
        )
        .unwrap();
        let expected = (1000.0 / 1000.0) / measured;
        assert!((build.scale_factor - expected).abs() < 1e-12);
        assert_eq!(build.units, "m");
        assert!(
            (build.mesh.longest_edge() - 1.0).abs() < 1e-3,
            "longest edge {}",
            build.mesh.longest_edge()
        );
        let summary = MeshSummary::from_build(&build, "out.glb");
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"scale_factor\""), "{json}");
        assert!(json.contains("\"units\":\"m\""), "{json}");
    }

    #[test]
    fn rejects_a_non_positive_mm_reference() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 2_000);
        let error = build_object_mesh(
            root,
            &root.join("work/colmap/sparse/0"),
            &MeshOptions {
                scale_from_mm_ref: Some((0.0, 500.0)),
                ..sphere_options()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("scale-from-mm-ref"), "{error}");
    }

    /// A project whose only object cloud is the pre-#11 name still loads.
    #[test]
    fn object_ply_path_falls_back_to_the_pre_11_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let legacy = root.join(OBJECT_PLY_RELATIVE);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        write_object_ply(&legacy, &sphere_splats(64));
        assert_eq!(object_ply_path(root).unwrap(), legacy);

        // Adding a numbered cloud switches to it, without touching the old file.
        let numbered = root.join(OBJECT_PLY_RELATIVE_V2);
        write_object_ply(&numbered, &sphere_splats(64));
        assert_eq!(object_ply_path(root).unwrap(), numbered);
    }

    #[test]
    fn object_ply_path_prefers_the_lowest_numbered_cloud() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for index in [3_usize, 1, 2] {
            write_object_ply(
                &root.join(format!("assets/object-{index:02}.ply")),
                &sphere_splats(16),
            );
        }
        assert_eq!(
            object_ply_path(root).unwrap(),
            root.join("assets/object-01.ply")
        );
    }

    #[test]
    fn object_ply_path_error_names_both_accepted_locations() {
        let dir = tempfile::tempdir().unwrap();
        let error = object_ply_path(dir.path()).unwrap_err().to_string();
        assert!(error.contains("assets/object-01.ply"), "{error}");
        assert!(error.contains("assets/object.ply"), "{error}");
    }

    #[test]
    fn publish_mesh_glb_writes_object_01_glb_and_its_manifest_entry() {
        use crate::reconstruction::assets::{read_manifest, sha256_file, AssetFormat, AssetKind};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        let build =
            build_object_mesh(root, &root.join("work/colmap/sparse/0"), &sphere_options()).unwrap();

        let summary = publish_mesh_glb(root, &build).unwrap();

        let glb = root.join("assets").join("object-01.glb");
        assert!(glb.is_file());
        assert_eq!(summary.out, glb.display().to_string());
        assert_eq!(summary.vertices, build.mesh.positions.len());
        assert_eq!(summary.triangles, build.mesh.triangle_count());

        // The published container re-parses as the mesh that was measured.
        let (json, _) =
            crate::reconstruction::mesh::glb::split_glb(&std::fs::read(&glb).unwrap()).unwrap();
        assert_eq!(
            json["accessors"][0]["count"].as_u64(),
            Some(build.mesh.positions.len() as u64)
        );
        assert_eq!(
            json["accessors"][3]["count"].as_u64(),
            Some((build.mesh.triangle_count() * 3) as u64)
        );

        let manifest = read_manifest(&root.join("assets")).unwrap().unwrap();
        let entry = manifest
            .assets
            .iter()
            .find(|entry| entry.format == AssetFormat::Glb)
            .expect("glb entry");
        assert_eq!(entry.kind, AssetKind::Object);
        assert_eq!(entry.index, 1);
        assert_eq!(entry.file, "object-01.glb");
        assert_eq!(entry.file_size, glb.metadata().unwrap().len());
        assert_eq!(entry.sha256, sha256_file(&glb).unwrap());
        assert_eq!(entry.vertex_count, Some(build.mesh.positions.len() as u64));
        assert_eq!(
            entry.triangle_count,
            Some(build.mesh.triangle_count() as u64)
        );
        assert_eq!(entry.units.as_deref(), Some(build.units));
        assert_eq!(entry.scale_factor, Some(build.scale_factor));
        assert_eq!(manifest.source.splat_count, 8_000);

        // No dot-temp survives a successful publish.
        let leftovers: Vec<String> = std::fs::read_dir(root.join("assets"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn publish_mesh_glb_adds_to_a_manifest_the_split_already_wrote() {
        use crate::reconstruction::assets::{
            read_manifest, upsert_asset_entry, AssetEntry, AssetFormat, AssetKind, BACKGROUND_INDEX,
        };

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 4_000);
        let assets = root.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let background = AssetEntry::new(
            AssetKind::Background,
            BACKGROUND_INDEX,
            AssetFormat::Ply,
            "background.ply",
            128,
            "a".repeat(64),
        )
        .with_splat_count(2_000);
        upsert_asset_entry(
            &assets,
            crate::reconstruction::assets::source_from_final_ply(root).unwrap(),
            background.clone(),
            1,
        )
        .unwrap();

        let build =
            build_object_mesh(root, &root.join("work/colmap/sparse/0"), &sphere_options()).unwrap();
        publish_mesh_glb(root, &build).unwrap();

        let manifest = read_manifest(&assets).unwrap().unwrap();
        assert_eq!(manifest.assets.len(), 2);
        assert_eq!(manifest.assets[0], background);
        assert_eq!(manifest.assets[1].format, AssetFormat::Glb);
    }

    /// The cross-stage acceptance for issue #11: the split publishes the two
    /// PLY assets, the mesh stage publishes the GLB, and all three land in one
    /// `assets/manifest.json` in the documented order.
    #[test]
    fn split_and_mesh_publish_into_one_manifest() {
        use crate::reconstruction::assets::{read_manifest, AssetFormat, AssetKind};
        use crate::reconstruction::splat_transform::export_instance_assets;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        let splat_count = 8_000_u64;
        crate::reconstruction::backproject::write_instance_mask(
            &root.join("derived").join("instance.mask"),
            splat_count,
            &(0..splat_count)
                .map(|index| (index % 2) as i32)
                .collect::<Vec<_>>(),
        )
        .unwrap();

        let split = export_instance_assets(root, || Ok(())).unwrap();
        assert_eq!(split.object_path, root.join("assets/object-01.ply"));
        assert_eq!(split.background_path, root.join("assets/background.ply"));

        let build =
            build_object_mesh(root, &root.join("work/colmap/sparse/0"), &sphere_options()).unwrap();
        let summary = publish_mesh_glb(root, &build).unwrap();
        assert_eq!(
            summary.out,
            root.join("assets")
                .join("object-01.glb")
                .display()
                .to_string()
        );

        let manifest = read_manifest(&root.join("assets")).unwrap().unwrap();
        assert_eq!(
            manifest
                .assets
                .iter()
                .map(|entry| (
                    entry.kind.as_str(),
                    entry.index,
                    entry.format.as_str(),
                    entry.file.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("background", 0, "ply", "background.ply"),
                ("object", 1, "glb", "object-01.glb"),
                ("object", 1, "ply", "object-01.ply"),
            ]
        );
        assert_eq!(manifest.source.splat_count, splat_count);
        for entry in &manifest.assets {
            let path = root.join("assets").join(&entry.file);
            assert_eq!(
                entry.file_size,
                path.metadata().unwrap().len(),
                "{}",
                entry.file
            );
            assert_eq!(
                entry.sha256,
                crate::reconstruction::assets::sha256_file(&path).unwrap(),
                "{}",
                entry.file
            );
        }
        assert_eq!(
            manifest
                .assets
                .iter()
                .find(|entry| entry.format == AssetFormat::Glb)
                .and_then(|entry| entry.triangle_count),
            Some(build.mesh.triangle_count() as u64)
        );
        assert!(
            manifest
                .assets
                .iter()
                .filter(|entry| entry.kind == AssetKind::Object)
                .count()
                == 2
        );
    }

    #[test]
    fn publish_mesh_glb_refuses_a_project_without_final_ply_before_publishing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 8_000);
        std::fs::remove_file(root.join("final.ply")).unwrap();
        let build =
            build_object_mesh(root, &root.join("work/colmap/sparse/0"), &sphere_options()).unwrap();

        // The manifest source cannot be resolved, so nothing may be published:
        // no orphan GLB, no manifest entry, no dot-temp.
        assert!(publish_mesh_glb(root, &build).is_err());
        assert!(!root.join("assets/object-01.glb").exists());
        assert!(!root.join("assets/manifest.json").exists());
        let leftovers: Vec<String> = std::fs::read_dir(root.join("assets"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn publish_mesh_glb_refuses_an_empty_mesh_without_leaving_a_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sphere_project(root, 64);
        let build = MeshBuild {
            mesh: Mesh::default(),
            splats: 0,
            frames: 0,
            voxels_fused: 0,
            voxel_size: 0.1,
            object_bounds: ([0.0; 3], [0.0; 3]),
            components: 0,
            components_removed: 0,
            vertices_before_simplify: None,
            triangles_before_simplify: None,
            scale_factor: 1.0,
            units: "scene",
        };

        assert!(publish_mesh_glb(root, &build).is_err());
        assert!(!root.join("assets/object-01.glb").exists());
        let leftovers: Vec<String> = std::fs::read_dir(root.join("assets"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
