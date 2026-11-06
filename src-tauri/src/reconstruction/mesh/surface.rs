//! Surface extraction from the fused TSDF plus vertex colouring.
//!
//! Zero-crossing extraction runs per tetrahedron on the Kuhn 6-tet
//! decomposition of every voxel cube — the marching-cubes zero level set,
//! sampled on tetrahedra instead of the cube's 256 lookup-table cases. This
//! keeps the implementation table-free while staying watertight: the Kuhn
//! decomposition splits every shared cube face along the same diagonal in
//! both neighbours, and crossing points are deduplicated by the *global* grid
//! edge they sit on, so each interior edge is shared by exactly two
//! triangles. Vertex normals come from the TSDF gradient (central
//! differences), i.e. the outward surface normal.

use std::collections::HashMap;

use crate::reconstruction::mesh::tsdf::TsdfGrid;
use crate::reconstruction::mesh::Mesh;
use crate::reconstruction::ply::ColoredSplat;

/// The six Kuhn permutations of the three axes. Each turns a cube into the
/// tetrahedron `v`, `v+e_p0`, `v+e_p0+e_p1`, `v+e_p0+e_p1+e_p2`; together
/// they tile the cube exactly once.
const KUHN: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

/// The six edges of a tetrahedron, as index pairs into its four corners.
const TET_EDGES: [(usize, usize); 6] = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];

/// Number of colour-lookup rings searched outward from a vertex before
/// falling back to mid grey.
const MAX_COLOR_RINGS: i64 = 4;

/// Extracts the zero-crossing surface of the fused grid.
///
/// Returns an empty mesh when no cube contains a sign change (nothing fused,
/// or the object does not intersect the grid). Voxels without an observation
/// (`NaN`) never participate, so unobserved regions stay open rather than
/// being closed by invented geometry.
pub fn extract_surface(grid: &TsdfGrid) -> Mesh {
    let spec = grid.spec;
    let mut mesh = Mesh::default();
    if spec.dims.iter().any(|dim| *dim < 2) {
        return mesh;
    }
    let mut edge_vertices: HashMap<([usize; 3], [usize; 3]), u32> = HashMap::new();
    let mut cube = Cube::default();

    for k in 0..spec.dims[2] - 1 {
        for j in 0..spec.dims[1] - 1 {
            for i in 0..spec.dims[0] - 1 {
                if !cube.load(grid, [i, j, k]) {
                    continue;
                }
                for permutation in KUHN {
                    let first = 1 << permutation[0];
                    let second = first | (1 << permutation[1]);
                    let third = second | (1 << permutation[2]);
                    march_tet(
                        [0, first, second, third],
                        &cube,
                        grid,
                        &mut edge_vertices,
                        &mut mesh,
                    );
                }
            }
        }
    }
    orient_triangles(&mut mesh);
    mesh
}

/// The eight corners of one voxel cube, in bit-indexed order:
/// bit 0 = `i+1`, bit 1 = `j+1`, bit 2 = `k+1`.
#[derive(Default)]
struct Cube {
    coords: [[usize; 3]; 8],
    positions: [[f64; 3]; 8],
    values: [f64; 8],
}

impl Cube {
    /// Loads the corners at cube origin `base`. Returns false when the cube
    /// holds no observed voxel at all (the common case away from the surface).
    fn load(&mut self, grid: &TsdfGrid, base: [usize; 3]) -> bool {
        let mut any = false;
        for corner in 0..8 {
            let coords = [
                base[0] + (corner & 1),
                base[1] + ((corner >> 1) & 1),
                base[2] + ((corner >> 2) & 1),
            ];
            self.coords[corner] = coords;
            self.positions[corner] = grid.spec.position(coords[0], coords[1], coords[2]);
            self.values[corner] = grid
                .value_at(coords[0], coords[1], coords[2])
                .unwrap_or(f64::NAN);
            any |= self.values[corner].is_finite();
        }
        any
    }
}

fn march_tet(
    tet: [usize; 4],
    cube: &Cube,
    grid: &TsdfGrid,
    edge_vertices: &mut HashMap<([usize; 3], [usize; 3]), u32>,
    mesh: &mut Mesh,
) {
    let values = [
        cube.values[tet[0]],
        cube.values[tet[1]],
        cube.values[tet[2]],
        cube.values[tet[3]],
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return;
    }
    let negative: Vec<usize> = (0..4).filter(|index| values[*index] < 0.0).collect();
    if negative.is_empty() || negative.len() == 4 {
        return;
    }
    let positive: Vec<usize> = (0..4).filter(|index| values[*index] >= 0.0).collect();

    let mut crossing = |a: usize, b: usize| -> u32 {
        let (first, second) = (cube.coords[tet[a]], cube.coords[tet[b]]);
        let key = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        if let Some(index) = edge_vertices.get(&key) {
            return *index;
        }
        let (va, vb) = (values[a], values[b]);
        let denominator = va - vb;
        let t = if denominator.abs() < 1e-12 {
            0.5
        } else {
            (va / denominator).clamp(0.0, 1.0)
        };
        let from = cube.positions[tet[a]];
        let to = cube.positions[tet[b]];
        let position = [
            from[0] + t * (to[0] - from[0]),
            from[1] + t * (to[1] - from[1]),
            from[2] + t * (to[2] - from[2]),
        ];
        let normal = crossing_normal(position, cube, tet[a], tet[b], t, grid);
        let index = mesh.positions.len() as u32;
        mesh.positions
            .push([position[0] as f32, position[1] as f32, position[2] as f32]);
        mesh.normals.push(normal);
        mesh.colors.push([0.5, 0.5, 0.5]);
        edge_vertices.insert(key, index);
        index
    };

    if negative.len() == 1 || positive.len() == 1 {
        let isolated = if negative.len() == 1 {
            negative[0]
        } else {
            positive[0]
        };
        let mut points = Vec::with_capacity(3);
        for (a, b) in TET_EDGES {
            if a == isolated || b == isolated {
                points.push(crossing(a, b));
            }
        }
        if points.len() == 3 {
            mesh.indices
                .extend_from_slice(&[points[0], points[1], points[2]]);
        }
        return;
    }

    // Two-two split. The four crossings sit on the edges between the two
    // negative corners `a0/a1` and the two positive ones `b0/b1`; the quad's
    // boundary runs a0b0 -> a0b1 -> a1b1 -> a1b0, and each of those segments
    // lies on a face of the tet, which is what the neighbouring tet
    // reproduces — so the mesh stays watertight. `orient_triangles` fixes the
    // winding afterwards.
    let (group_a, group_b) = if negative.len() == 2 {
        (negative, positive)
    } else {
        (positive, negative)
    };
    let quad = [
        crossing(group_a[0], group_b[0]),
        crossing(group_a[0], group_b[1]),
        crossing(group_a[1], group_b[1]),
        crossing(group_a[1], group_b[0]),
    ];
    mesh.indices
        .extend_from_slice(&[quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
}

/// Interpolated outward normal at a crossing point.
///
/// The TSDF increases outwards, so its gradient is the outward normal. The
/// gradients at the two edge endpoints are lerped at the crossing parameter;
/// when both are degenerate (a flat or one-sided field) the direction from the
/// grid centre to the vertex is used instead.
fn crossing_normal(
    position: [f64; 3],
    cube: &Cube,
    corner_a: usize,
    corner_b: usize,
    t: f64,
    grid: &TsdfGrid,
) -> [f32; 3] {
    let gradient_at = |corner: usize| -> [f64; 3] {
        let coords = cube.coords[corner];
        grid.gradient_at(coords[0], coords[1], coords[2])
    };
    let ga = gradient_at(corner_a);
    let gb = gradient_at(corner_b);
    let gradient = [
        ga[0] + t * (gb[0] - ga[0]),
        ga[1] + t * (gb[1] - ga[1]),
        ga[2] + t * (gb[2] - ga[2]),
    ];
    let magnitude =
        (gradient[0] * gradient[0] + gradient[1] * gradient[1] + gradient[2] * gradient[2]).sqrt();
    if magnitude > 1e-9 {
        return [
            (gradient[0] / magnitude) as f32,
            (gradient[1] / magnitude) as f32,
            (gradient[2] / magnitude) as f32,
        ];
    }
    let spec = grid.spec;
    let centre = [
        spec.origin[0] + 0.5 * (spec.dims[0] as f64 - 1.0) * spec.voxel_size,
        spec.origin[1] + 0.5 * (spec.dims[1] as f64 - 1.0) * spec.voxel_size,
        spec.origin[2] + 0.5 * (spec.dims[2] as f64 - 1.0) * spec.voxel_size,
    ];
    let direction = [
        position[0] - centre[0],
        position[1] - centre[1],
        position[2] - centre[2],
    ];
    let length =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
            .sqrt();
    if length > 1e-9 {
        [
            (direction[0] / length) as f32,
            (direction[1] / length) as f32,
            (direction[2] / length) as f32,
        ]
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// Flips any triangle whose winding disagrees with the vertex normals, so
/// every front face is counter-clockwise as glTF expects.
fn orient_triangles(mesh: &mut Mesh) {
    for triangle in 0..mesh.indices.len() / 3 {
        let base = triangle * 3;
        let (a, b, c) = (
            mesh.indices[base] as usize,
            mesh.indices[base + 1] as usize,
            mesh.indices[base + 2] as usize,
        );
        let face = cross(
            sub(mesh.positions[b], mesh.positions[a]),
            sub(mesh.positions[c], mesh.positions[a]),
        );
        let reference = [
            mesh.normals[a][0] + mesh.normals[b][0] + mesh.normals[c][0],
            mesh.normals[a][1] + mesh.normals[b][1] + mesh.normals[c][1],
            mesh.normals[a][2] + mesh.normals[b][2] + mesh.normals[c][2],
        ];
        if dot(face, reference) < 0.0 {
            mesh.indices.swap(base + 1, base + 2);
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Assigns every mesh vertex the linear colour of its nearest splat.
///
/// Splats are bucketed into `cell_size` cubes once (`O(n)`), then each vertex
/// searches outward ring by ring and stops at the first ring that holds a hit
/// (bounded by `MAX_COLOR_RINGS`). Unmatched vertices fall back to mid grey.
/// Returns the number of vertices that found a splat.
pub fn assign_nearest_colors(mesh: &mut Mesh, splats: &[ColoredSplat], cell_size: f64) -> usize {
    if splats.is_empty() || mesh.positions.is_empty() {
        return 0;
    }
    let cell = if cell_size.is_finite() && cell_size > 0.0 {
        cell_size
    } else {
        1.0
    };
    let mut buckets: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    for (index, splat) in splats.iter().enumerate() {
        buckets
            .entry(bucket_of(splat.position, cell))
            .or_default()
            .push(index as u32);
    }
    let mut matched = 0;
    for vertex in 0..mesh.positions.len() {
        let position = [
            mesh.positions[vertex][0] as f64,
            mesh.positions[vertex][1] as f64,
            mesh.positions[vertex][2] as f64,
        ];
        let home = bucket_of(position, cell);
        let mut best: Option<(f64, [f32; 3])> = None;
        for ring in 0..=MAX_COLOR_RINGS {
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    for dz in -ring..=ring {
                        // Only the shell of the current ring is new.
                        if ring > 0 && dx.abs() != ring && dy.abs() != ring && dz.abs() != ring {
                            continue;
                        }
                        let Some(candidates) =
                            buckets.get(&(home.0 + dx, home.1 + dy, home.2 + dz))
                        else {
                            continue;
                        };
                        for candidate in candidates {
                            let splat = &splats[*candidate as usize];
                            let distance = squared_distance(position, splat.position);
                            if best.is_none_or(|(nearest, _)| distance < nearest) {
                                best = Some((distance, splat.color));
                            }
                        }
                    }
                }
            }
            if best.is_some() {
                // Any hit in this ring is closer than everything further out.
                break;
            }
        }
        match best {
            Some((_, color)) => {
                mesh.colors[vertex] = color;
                matched += 1;
            }
            None => mesh.colors[vertex] = [0.5, 0.5, 0.5],
        }
    }
    matched
}

fn bucket_of(position: [f64; 3], cell: f64) -> (i64, i64, i64) {
    (
        (position[0] / cell).floor() as i64,
        (position[1] / cell).floor() as i64,
        (position[2] / cell).floor() as i64,
    )
}

fn squared_distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::mesh::tsdf::GridSpec;

    /// A sphere grid padded on every side, so the whole surface (and the
    /// positive shell around it) sits strictly inside the grid.
    fn sphere_grid(radius: f64, voxel: f64, truncation: f64) -> TsdfGrid {
        let spec = GridSpec::from_bounds([-radius; 3], [radius; 3], voxel, 3.0 * voxel).unwrap();
        TsdfGrid::from_signed_distance(spec, truncation, |p| {
            (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - radius
        })
    }

    /// Counts how often each undirected edge appears; a watertight manifold
    /// has every edge used exactly twice.
    fn edge_use_counts(mesh: &Mesh) -> HashMap<(u32, u32), usize> {
        let mut counts = HashMap::new();
        for triangle in mesh.indices.as_chunks::<3>().0 {
            for (a, b) in [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ] {
                *counts.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        counts
    }

    #[test]
    fn rebuilds_a_sphere_within_its_bounding_box() {
        let grid = sphere_grid(1.0, 0.05, 0.2);
        let mesh = extract_surface(&grid);
        assert!(
            mesh.positions.len() > 100,
            "expected a dense mesh, got {} vertices",
            mesh.positions.len()
        );
        assert!(mesh.triangle_count() > 0);
        let (min, max) = mesh.bounds().expect("non-empty mesh");
        for axis in 0..3 {
            assert!(
                min[axis] > -1.06 && min[axis] < -0.94,
                "min {min:?} should hug -1"
            );
            assert!(
                max[axis] < 1.06 && max[axis] > 0.94,
                "max {max:?} should hug +1"
            );
        }
    }

    #[test]
    fn sphere_surface_is_watertight() {
        let grid = sphere_grid(1.0, 0.1, 0.3);
        let mesh = extract_surface(&grid);
        let counts = edge_use_counts(&mesh);
        assert!(!counts.is_empty());
        let broken = counts.values().filter(|count| **count != 2).count();
        assert_eq!(
            broken,
            0,
            "{broken} of {} edges are not shared by exactly two triangles",
            counts.len()
        );
    }

    #[test]
    fn vertex_normals_point_outwards_from_the_sphere_centre() {
        let grid = sphere_grid(1.0, 0.05, 0.2);
        let mesh = extract_surface(&grid);
        assert_eq!(mesh.normals.len(), mesh.positions.len());
        let aligned = mesh
            .positions
            .iter()
            .zip(&mesh.normals)
            .filter(|(position, normal)| {
                let dot =
                    position[0] * normal[0] + position[1] * normal[1] + position[2] * normal[2];
                dot > 0.7
            })
            .count();
        assert!(
            aligned * 10 >= mesh.positions.len() * 9,
            "only {aligned} of {} normals point outwards",
            mesh.positions.len()
        );
    }

    #[test]
    fn extracts_a_plane_at_its_zero_level() {
        let spec = GridSpec::from_bounds([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0], 0.1, 0.3).unwrap();
        let grid = TsdfGrid::from_signed_distance(spec, 0.3, |p| p[2] - 0.5);
        let mesh = extract_surface(&grid);
        assert!(mesh.triangle_count() > 100);
        let (min, max) = mesh.bounds().unwrap();
        assert!((min[2] - 0.5).abs() < 1e-3 && (max[2] - 0.5).abs() < 1e-3);
        assert!(max[1] - min[1] > 1.5, "plane should span the grid");
    }

    #[test]
    fn empty_grid_yields_an_empty_mesh() {
        let spec = GridSpec::from_bounds([0.0; 3], [1.0; 3], 0.1, 0.0).unwrap();
        let mesh = extract_surface(&TsdfGrid::new(spec));
        assert!(mesh.is_empty());
    }

    #[test]
    fn all_positive_field_yields_an_empty_mesh() {
        let spec = GridSpec::from_bounds([0.0; 3], [1.0; 3], 0.1, 0.0).unwrap();
        let grid = TsdfGrid::from_signed_distance(spec, 10.0, |_| 1.0);
        assert!(extract_surface(&grid).is_empty());
    }

    #[test]
    fn assigns_the_nearest_splat_colours() {
        let grid = sphere_grid(1.0, 0.1, 0.3);
        let mut mesh = extract_surface(&grid);
        let splats = vec![
            ColoredSplat {
                position: [0.0, 1.0, 0.0],
                color: [1.0, 0.0, 0.0],
            },
            ColoredSplat {
                position: [0.0, -1.0, 0.0],
                color: [0.0, 0.0, 1.0],
            },
        ];
        // A cell of 1.0 with the 4-ring search reaches every vertex of the
        // unit sphere, so all of them should resolve a colour.
        let assigned = assign_nearest_colors(&mut mesh, &splats, 1.0);
        assert_eq!(assigned, mesh.positions.len());
        let north = mesh
            .positions
            .iter()
            .zip(&mesh.colors)
            .find(|(position, _)| position[1] > 0.9)
            .expect("vertex near the north pole");
        assert_eq!(*north.1, [1.0, 0.0, 0.0]);
        let south = mesh
            .positions
            .iter()
            .zip(&mesh.colors)
            .find(|(position, _)| position[1] < -0.9)
            .expect("vertex near the south pole");
        assert_eq!(*south.1, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn unmatched_vertices_fall_back_to_grey() {
        let mut mesh = Mesh {
            positions: vec![[5.0, 5.0, 5.0]],
            normals: vec![[0.0, 1.0, 0.0]],
            colors: vec![[0.0; 3]],
            indices: vec![0],
        };
        let assigned = assign_nearest_colors(
            &mut mesh,
            &[ColoredSplat {
                position: [0.0, 0.0, 0.0],
                color: [1.0, 1.0, 0.0],
            }],
            0.1,
        );
        assert_eq!(assigned, 0);
        assert_eq!(mesh.colors[0], [0.5, 0.5, 0.5]);
    }
}
