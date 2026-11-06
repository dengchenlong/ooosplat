//! Offline mesh extraction from an object Gaussian-splat cloud (issue #10, M1).
//!
//! Pipeline: per-frame depth rendering from the object splats → TSDF fusion
//! over the splat AABB → marching cubes → vertex-coloured `.glb`.
//!
//! Everything here is pure Rust with no new crate dependencies: the depth
//! rasteriser, the voxel grid, the surface extractor and the glTF 2.0 binary
//! writer are all implemented in this module tree.

pub mod depth;
pub mod glb;
pub mod surface;
pub mod tsdf;

pub use depth::{
    fill_holes, render_depth, scale_intrinsics, smooth_depth, DepthMap, DepthRasterOptions,
};
pub use glb::{encode_glb, split_glb, write_glb, GlbOutput};
pub use surface::{assign_nearest_colors, extract_surface};
pub use tsdf::{fuse_into, DepthFrame, FuseOptions, GridSpec, TsdfGrid};

/// A triangle mesh with per-vertex normals and colours, in glTF conventions
/// (right-handed, `+Y` up, counter-clockwise front faces).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty() || self.indices.is_empty()
    }

    /// Axis-aligned bounds of the vertices, or `None` for an empty mesh.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let first = *self.positions.first()?;
        let mut min = [first[0] as f64, first[1] as f64, first[2] as f64];
        let mut max = min;
        for position in &self.positions {
            for (axis, value) in position.iter().enumerate() {
                let value = *value as f64;
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
        Some((min, max))
    }

    /// Longest bounding-box edge, `0.0` for an empty mesh.
    pub fn longest_edge(&self) -> f64 {
        self.bounds()
            .map(|(min, max)| {
                (0..3)
                    .map(|axis| max[axis] - min[axis])
                    .fold(0.0_f64, f64::max)
            })
            .unwrap_or(0.0)
    }

    /// Uniformly scales positions (and only positions) about the origin.
    pub fn scale_positions(&mut self, factor: f64) {
        let factor = factor as f32;
        for position in &mut self.positions {
            for value in position.iter_mut() {
                *value *= factor;
            }
        }
    }
}
