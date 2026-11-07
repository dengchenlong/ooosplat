//! Mesh post-processing for the M1 chain (issue #17): connected-component
//! cleanup, edge-collapse simplification and physical-scale calibration.
//!
//! Everything here is pure Rust on `Mesh` — no new crate dependencies.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::reconstruction::mesh::Mesh;

/// One connected component of the triangle graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    /// Number of triangles in the component.
    pub triangles: usize,
    /// Share of all triangles, `0..=1`.
    pub ratio: f64,
}

/// Connected components of the triangle graph (two triangles are connected
/// when they share a vertex), largest first.
pub fn components(mesh: &Mesh) -> Vec<Component> {
    let count = mesh.triangle_count();
    if count == 0 {
        return Vec::new();
    }
    let vertex_count = mesh.positions.len();
    let mut set = DisjointSet::new(vertex_count);
    for triangle in mesh.indices.chunks_exact(3) {
        set.union(triangle[0], triangle[1]);
        set.union(triangle[1], triangle[2]);
    }
    // Dense root → component slot map (each root owns one triangle count).
    let mut slot_of_root = vec![usize::MAX; vertex_count];
    let mut component_count = 0usize;
    for slot in 0..vertex_count {
        let root = set.find(slot as u32) as usize;
        if slot_of_root[root] == usize::MAX {
            slot_of_root[root] = component_count;
            component_count += 1;
        }
    }
    let mut counts = vec![0usize; component_count];
    for triangle in mesh.indices.chunks_exact(3) {
        counts[slot_of_root[set.find(triangle[0]) as usize]] += 1;
    }
    let total = count as f64;
    let mut result: Vec<Component> = counts
        .iter()
        .filter(|triangles| **triangles > 0)
        .map(|triangles| Component {
            triangles: *triangles,
            ratio: *triangles as f64 / total,
        })
        .collect();
    result.sort_by(|left, right| right.triangles.cmp(&left.triangles));
    result
}

/// Drops components whose triangle share is below `min_ratio` and, when
/// `largest_only` is set, keeps nothing but the largest one.
///
/// Vertices are re-packed in their original order, so a mesh without any
/// dropped component comes back unchanged.
pub fn clean_components(mesh: &Mesh, min_ratio: f64, largest_only: bool) -> Mesh {
    let count = mesh.triangle_count();
    if count == 0 {
        return mesh.clone();
    }
    let vertex_count = mesh.positions.len();
    if vertex_count == 0 {
        return mesh.clone();
    }
    let mut set = DisjointSet::new(vertex_count);
    for triangle in mesh.indices.chunks_exact(3) {
        set.union(triangle[0], triangle[1]);
        set.union(triangle[1], triangle[2]);
    }
    // Dense root → component slot map, in vertex order (deterministic).
    let mut slot_of_root = vec![usize::MAX; vertex_count];
    let mut component_count = 0usize;
    for slot in 0..vertex_count {
        let root = set.find(slot as u32) as usize;
        if slot_of_root[root] == usize::MAX {
            slot_of_root[root] = component_count;
            component_count += 1;
        }
    }
    let mut counts = vec![0usize; component_count];
    let mut triangle_component = Vec::with_capacity(count);
    for triangle in mesh.indices.chunks_exact(3) {
        let component = slot_of_root[set.find(triangle[0]) as usize];
        triangle_component.push(component);
        counts[component] += 1;
    }
    let largest = counts
        .iter()
        .enumerate()
        .max_by_key(|(_, triangles)| **triangles)
        .map(|(slot, _)| slot)
        .unwrap_or(0);
    let total = count as f64;
    let keep: Vec<bool> = counts
        .iter()
        .enumerate()
        .map(|(slot, triangles)| {
            if largest_only {
                slot == largest
            } else {
                *triangles as f64 / total >= min_ratio
            }
        })
        .collect();
    // Nothing to drop → hand back the input untouched. Comparing kept
    // *triangles* (not components) keeps the output byte-identical even when
    // the mesh carries unreferenced vertices.
    if triangle_component.iter().all(|component| keep[*component]) {
        return mesh.clone();
    }
    let mut vertex_map = vec![u32::MAX; vertex_count];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for (triangle_index, triangle) in mesh.indices.chunks_exact(3).enumerate() {
        if !keep[triangle_component[triangle_index]] {
            continue;
        }
        for vertex in triangle {
            let slot = *vertex as usize;
            if vertex_map[slot] == u32::MAX {
                vertex_map[slot] = positions.len() as u32;
                positions.push(mesh.positions[slot]);
                if let Some(normal) = mesh.normals.get(slot) {
                    normals.push(*normal);
                }
                if let Some(color) = mesh.colors.get(slot) {
                    colors.push(*color);
                }
            }
            indices.push(vertex_map[slot]);
        }
    }
    Mesh {
        positions,
        normals,
        colors,
        indices,
    }
}

/// Edge-collapse simplification down to at most `target_triangles`.
///
/// The shortest edges collapse first; survivors are the midpoints of the
/// collapsed pair with averaged vertex colour. Normals are recomputed from
/// the final triangles.
pub fn simplify(mesh: &Mesh, target_triangles: usize) -> Mesh {
    let triangle_total = mesh.triangle_count();
    if target_triangles == 0 || triangle_total <= target_triangles {
        return mesh.clone();
    }
    let vertex_total = mesh.positions.len();
    if vertex_total == 0 {
        return mesh.clone();
    }
    let mut positions: Vec<[f32; 3]> = mesh.positions.clone();
    let mut colors: Vec<[f32; 3]> = Vec::with_capacity(vertex_total);
    for slot in 0..vertex_total {
        colors.push(mesh.colors.get(slot).copied().unwrap_or([1.0, 1.0, 1.0]));
    }
    let mut triangles: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|chunk| [chunk[0], chunk[1], chunk[2]])
        .collect();
    let mut alive = vec![true; triangles.len()];
    let mut vertex_triangles: Vec<Vec<u32>> = vec![Vec::new(); vertex_total];
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        for vertex in triangle {
            vertex_triangles[*vertex as usize].push(triangle_index as u32);
        }
    }
    let mut parent: Vec<u32> = (0..vertex_total as u32).collect();
    let mut remaining = triangles.len();
    if remaining <= target_triangles {
        return mesh.clone();
    }

    let mut heap: BinaryHeap<Edge> = BinaryHeap::new();
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        if !alive[triangle_index] {
            continue;
        }
        for (left, right) in [(0, 1), (1, 2), (2, 0)] {
            push_edge(
                &mut heap,
                triangle[left],
                triangle[right],
                &positions,
                &mut parent,
            );
        }
    }

    while remaining > target_triangles {
        let edge = match heap.pop() {
            Some(edge) => edge,
            None => break,
        };
        let a = find(&mut parent, edge.a);
        let b = find(&mut parent, edge.b);
        if a == b {
            continue;
        }
        // Only collapse when a and b still have triangles: a lone vertex is
        // already gone from the surface.
        if vertex_triangles[a as usize].is_empty() || vertex_triangles[b as usize].is_empty() {
            continue;
        }
        // Collapse b into a (a is the lower index, so the survivor is stable).
        let (keep, gone) = if a < b { (a, b) } else { (b, a) };
        let keep_index = keep as usize;
        let gone_index = gone as usize;
        // Reject collapses that would fold the surface onto itself: the
        // neighbourhoods may share at most two triangles (the edge itself
        // appears in exactly two for a closed manifold). More than two shared
        // triangles means the collapse would pinch a boundary.
        let shared = shared_triangles(
            &vertex_triangles[keep_index],
            &vertex_triangles[gone_index],
            &triangles,
            &alive,
        );
        if shared.len() > 2 {
            continue;
        }
        let keep_position = positions[keep_index];
        let gone_position = positions[gone_index];
        positions[keep_index] = [
            (keep_position[0] + gone_position[0]) * 0.5,
            (keep_position[1] + gone_position[1]) * 0.5,
            (keep_position[2] + gone_position[2]) * 0.5,
        ];
        let keep_color = colors[keep_index];
        let gone_color = colors[gone_index];
        colors[keep_index] = [
            (keep_color[0] + gone_color[0]) * 0.5,
            (keep_color[1] + gone_color[1]) * 0.5,
            (keep_color[2] + gone_color[2]) * 0.5,
        ];
        parent[gone_index] = keep;
        let moved: Vec<u32> = std::mem::take(&mut vertex_triangles[gone_index]);
        for triangle_index in &moved {
            let index = *triangle_index as usize;
            if !alive[index] {
                continue;
            }
            for vertex in triangles[index].iter_mut() {
                if *vertex == gone {
                    *vertex = keep;
                }
            }
            let [i0, i1, i2] = triangles[index];
            if i0 == i1 || i1 == i2 || i0 == i2 {
                alive[index] = false;
                remaining -= 1;
            } else {
                vertex_triangles[keep_index].push(*triangle_index);
            }
        }
        vertex_triangles[keep_index].retain(|triangle_index| {
            let index = *triangle_index as usize;
            alive[index] && !triangles[index].contains(&gone)
        });
        // Re-push the surviving edges of `keep` with their new lengths.
        let neighbours: Vec<u32> = vertex_triangles[keep_index]
            .iter()
            .filter_map(|triangle_index| {
                let triangle = triangles[*triangle_index as usize];
                triangle
                    .iter()
                    .copied()
                    .find(|vertex| find(&mut parent, *vertex) != keep)
            })
            .collect();
        for neighbour in neighbours {
            push_edge(&mut heap, keep, neighbour, &positions, &mut parent);
        }
    }

    // Compact: representatives that still own a triangle survive.
    let mut index_map = vec![u32::MAX; vertex_total];
    let mut result_positions = Vec::new();
    let mut result_colors = Vec::new();
    let mut indices = Vec::with_capacity(remaining * 3);
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        if !alive[triangle_index] {
            continue;
        }
        for vertex in triangle {
            let root = find(&mut parent, *vertex) as usize;
            if index_map[root] == u32::MAX {
                index_map[root] = result_positions.len() as u32;
                result_positions.push(positions[root]);
                result_colors.push(colors[root]);
            }
            indices.push(index_map[root]);
        }
    }
    let mut simplified = Mesh {
        positions: result_positions,
        normals: Vec::new(),
        colors: result_colors,
        indices,
    };
    recompute_normals(&mut simplified);
    simplified
}

/// Area-weighted vertex normals from the current triangles.
pub fn recompute_normals(mesh: &mut Mesh) {
    let mut normals = vec![[0.0_f32; 3]; mesh.positions.len()];
    for triangle in mesh.indices.chunks_exact(3) {
        let [i0, i1, i2] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let (Some(p0), Some(p1), Some(p2)) = (
            mesh.positions.get(i0).copied(),
            mesh.positions.get(i1).copied(),
            mesh.positions.get(i2).copied(),
        ) else {
            continue;
        };
        let edge1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let edge2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let normal = [
            edge1[1] * edge2[2] - edge1[2] * edge2[1],
            edge1[2] * edge2[0] - edge1[0] * edge2[2],
            edge1[0] * edge2[1] - edge1[1] * edge2[0],
        ];
        for index in [i0, i1, i2] {
            for axis in 0..3 {
                normals[index][axis] += normal[axis];
            }
        }
    }
    for normal in &mut normals {
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if length > 0.0 {
            for value in normal.iter_mut() {
                *value /= length;
            }
        } else {
            *normal = [0.0, 1.0, 0.0];
        }
    }
    mesh.normals = normals;
}

/// Uniform scale factor that makes a measured scene-unit length equal a
/// known length in millimetres. The output mesh is therefore in metres.
pub fn scale_factor_from_mm_ref(measured: f64, known_mm: f64) -> Option<f64> {
    if !measured.is_finite() || measured <= 0.0 {
        return None;
    }
    if !known_mm.is_finite() || known_mm <= 0.0 {
        return None;
    }
    Some((known_mm / 1000.0) / measured)
}

/// Parses `--scale-from-mm-ref <measured>:<millimetres>`.
pub fn parse_mm_ref(text: &str) -> Option<(f64, f64)> {
    let (measured, millimetres) = text.split_once(':')?;
    let measured: f64 = measured.trim().parse().ok()?;
    let millimetres: f64 = millimetres.trim().parse().ok()?;
    if !measured.is_finite() || measured <= 0.0 {
        return None;
    }
    if !millimetres.is_finite() || millimetres <= 0.0 {
        return None;
    }
    Some((measured, millimetres))
}

fn find(parent: &mut [u32], vertex: u32) -> u32 {
    let mut root = vertex;
    while parent[root as usize] != root {
        root = parent[root as usize];
    }
    let mut current = vertex;
    while parent[current as usize] != root {
        let next = parent[current as usize];
        parent[current as usize] = root;
        current = next;
    }
    root
}

fn shared_triangles(
    left: &[u32],
    right: &[u32],
    _triangles: &[[u32; 3]],
    alive: &[bool],
) -> Vec<u32> {
    let mut shared = Vec::new();
    for triangle_index in right {
        let index = *triangle_index as usize;
        if alive[index] && left.contains(triangle_index) && !shared.contains(triangle_index) {
            shared.push(*triangle_index);
        }
    }
    shared
}

fn push_edge(
    heap: &mut BinaryHeap<Edge>,
    left: u32,
    right: u32,
    positions: &[[f32; 3]],
    parent: &mut [u32],
) {
    let a = find(parent, left);
    let b = find(parent, right);
    if a == b {
        return;
    }
    let (Some(pa), Some(pb)) = (
        positions.get(a as usize).copied(),
        positions.get(b as usize).copied(),
    ) else {
        return;
    };
    let dx = (pa[0] - pb[0]) as f64;
    let dy = (pa[1] - pb[1]) as f64;
    let dz = (pa[2] - pb[2]) as f64;
    heap.push(Edge {
        cost: dx * dx + dy * dy + dz * dz,
        a: a.min(b),
        b: a.max(b),
    });
}

/// Min-heap by squared edge length (reversed ordering on `cost`).
#[derive(Debug, Clone, Copy)]
struct Edge {
    cost: f64,
    a: u32,
    b: u32,
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.a == other.a && self.b == other.b
    }
}

impl Eq for Edge {}

impl PartialOrd for Edge {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Edge {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed so `BinaryHeap` (a max-heap) pops the shortest edge first.
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.a.cmp(&self.a))
            .then_with(|| other.b.cmp(&self.b))
    }
}

/// Union-find over vertex indices, used by the component pass.
struct DisjointSet {
    parent: Vec<u32>,
    rank: Vec<u8>,
}

impl DisjointSet {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size as u32).collect(),
            rank: vec![0; size],
        }
    }

    fn find(&mut self, vertex: u32) -> u32 {
        let mut root = vertex;
        while self.parent[root as usize] != root {
            root = self.parent[root as usize];
        }
        let mut current = vertex;
        while self.parent[current as usize] != root {
            let next = self.parent[current as usize];
            self.parent[current as usize] = root;
            current = next;
        }
        root
    }

    fn union(&mut self, left: u32, right: u32) {
        let mut a = self.find(left);
        let mut b = self.find(right);
        if a == b {
            return;
        }
        if self.rank[a as usize] < self.rank[b as usize] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b as usize] = a;
        if self.rank[a as usize] == self.rank[b as usize] {
            self.rank[a as usize] += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two triangles sharing an edge plus one isolated triangle.
    fn two_plus_one() -> Mesh {
        Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [5.0, 5.0, 5.0],
                [5.5, 5.0, 5.0],
                [5.0, 5.5, 5.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 7],
            colors: vec![[0.0, 0.0, 0.0]; 7],
            indices: vec![0, 1, 2, 1, 3, 2, 4, 5, 6],
        }
    }

    #[test]
    fn counts_components_by_shared_vertices() {
        let mesh = two_plus_one();
        let parts = components(&mesh);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].triangles, 2);
        assert!((parts[0].ratio - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(parts[1].triangles, 1);
    }

    #[test]
    fn drops_the_small_component_and_repacks_vertices() {
        let mesh = two_plus_one();
        let cleaned = clean_components(&mesh, 0.5, false);
        assert_eq!(cleaned.triangle_count(), 2);
        assert_eq!(cleaned.positions.len(), 4);
        assert_eq!(cleaned.indices, vec![0, 1, 2, 1, 3, 2]);
        assert_eq!(cleaned.positions[3], [1.0, 1.0, 0.0]);
    }

    #[test]
    fn keep_largest_only_ignores_the_ratio() {
        let mesh = two_plus_one();
        let cleaned = clean_components(&mesh, 0.0, true);
        assert_eq!(cleaned.triangle_count(), 2);
    }

    #[test]
    fn a_ratio_that_drops_nothing_is_byte_identical() {
        let mesh = {
            // Four triangles, evenly split: 2 + 2.
            let mut mesh = two_plus_one();
            mesh.positions.push([6.0, 6.0, 5.0]);
            mesh.positions.push([6.5, 6.0, 5.0]);
            mesh.normals = vec![[0.0, 0.0, 1.0]; mesh.positions.len()];
            mesh.colors = vec![[0.5, 0.5, 0.5]; mesh.positions.len()];
            mesh.indices = vec![0, 1, 2, 1, 3, 2, 4, 5, 6, 6, 5, 7];
            mesh
        };
        let cleaned = clean_components(&mesh, 0.49, false);
        assert_eq!(cleaned, mesh);
    }

    #[test]
    fn simplify_reduces_the_triangle_count_and_keeps_colours() {
        // A flat 8x8 grid of quads (128 triangles) with a colour ramp.
        let side = 9usize;
        let mut positions = Vec::new();
        let mut colors = Vec::new();
        for y in 0..side {
            for x in 0..side {
                positions.push([x as f32, y as f32, 0.0]);
                colors.push([x as f32 / 8.0, y as f32 / 8.0, 0.25]);
            }
        }
        let mut indices = Vec::new();
        for y in 0..side - 1 {
            for x in 0..side - 1 {
                let a = (y * side + x) as u32;
                let b = a + 1;
                let c = a + side as u32;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        let mut mesh = Mesh {
            positions,
            colors,
            normals: Vec::new(),
            indices,
        };
        recompute_normals(&mut mesh);
        let simplified = simplify(&mesh, 40);
        assert!(
            simplified.triangle_count() <= 40,
            "got {}",
            simplified.triangle_count()
        );
        assert!(simplified.triangle_count() >= 30);
        assert_eq!(simplified.colors.len(), simplified.positions.len());
        assert_eq!(simplified.normals.len(), simplified.positions.len());
        for color in &simplified.colors {
            assert!((color[2] - 0.25).abs() < 1e-6);
        }
        for normal in &simplified.normals {
            let length =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            assert!((length - 1.0).abs() < 1e-3, "normal {normal:?}");
        }
        let (min, max) = simplified.bounds().unwrap();
        assert!(min[0] >= -0.001 && max[0] <= 8.001);
    }

    #[test]
    fn simplify_leaves_an_already_small_mesh_alone() {
        let mesh = two_plus_one();
        assert_eq!(simplify(&mesh, 3), mesh);
        assert_eq!(simplify(&mesh, 100), mesh);
    }

    #[test]
    fn scale_factor_makes_the_reference_length_match() {
        // 1.25 scene units are known to be 500 mm.
        let factor = scale_factor_from_mm_ref(1.25, 500.0).unwrap();
        assert!((factor - 0.4).abs() < 1e-12);
        assert!((1.25 * factor - 0.5).abs() < 1e-12);
        assert!(scale_factor_from_mm_ref(0.0, 500.0).is_none());
        assert!(scale_factor_from_mm_ref(1.0, 0.0).is_none());
        assert!(scale_factor_from_mm_ref(f64::NAN, 500.0).is_none());
    }

    #[test]
    fn parses_a_mm_reference() {
        assert_eq!(parse_mm_ref("1.25:500"), Some((1.25, 500.0)));
        assert_eq!(parse_mm_ref(" 0.5 : 120.5 "), Some((0.5, 120.5)));
        assert_eq!(parse_mm_ref("1.25"), None);
        assert_eq!(parse_mm_ref("a:b"), None);
        assert_eq!(parse_mm_ref("1:0"), None);
        assert_eq!(parse_mm_ref("-1:5"), None);
    }
}
