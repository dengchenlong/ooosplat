//! Minimal glTF 2.0 binary (`.glb`) writer for a single vertex-coloured mesh.
//!
//! Hand-written container per the glTF 2.0 spec: a 12-byte header
//! (`glTF`, version 2, total length), a JSON chunk padded with spaces and a
//! BIN chunk padded with zeroes, both 4-byte aligned. Attributes are
//! POSITION / NORMAL / COLOR_0 (float32 vec3) plus uint32 indices, mode 4.

use std::path::Path;

use serde_json::{json, Value};

use crate::error::{Result, SplatError};
use crate::reconstruction::mesh::Mesh;

const MAGIC: u32 = 0x4654_6c67; // "glTF"
const VERSION: u32 = 2;
const CHUNK_JSON: u32 = 0x4E4F_534A; // "JSON"
const CHUNK_BIN: u32 = 0x004E_4942; // "BIN\0"
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
/// Written into `asset.generator` so a GLB can be traced back to this CLI.
const GENERATOR: &str = "ooosplat objmesh_mesh (M1 TSDF)";

/// A finished GLB file: the container bytes plus the numbers an acceptance
/// run needs to report.
#[derive(Debug, Clone, PartialEq)]
pub struct GlbOutput {
    pub bytes: Vec<u8>,
    pub vertex_count: usize,
    pub triangle_count: usize,
}

/// Serialises `mesh` as a single-node, single-mesh GLB.
///
/// Returns an error for an empty mesh: an empty GLB would open in Blender as
/// nothing at all, which is indistinguishable from a failed export.
pub fn encode_glb(mesh: &Mesh) -> Result<GlbOutput> {
    if mesh.is_empty() {
        return Err(SplatError::Process(
            "网格为空，无法写出 GLB（几何提取失败或物体未与网格相交）".into(),
        ));
    }
    let vertex_count = mesh.positions.len();
    if mesh.normals.len() != vertex_count || mesh.colors.len() != vertex_count {
        return Err(SplatError::Process(format!(
            "顶点属性数量不一致：POSITION {} / NORMAL {} / COLOR_0 {}",
            vertex_count,
            mesh.normals.len(),
            mesh.colors.len()
        )));
    }
    if !mesh.indices.len().is_multiple_of(3) {
        return Err(SplatError::Process("索引数量不是 3 的倍数".into()));
    }
    if vertex_count > u32::MAX as usize {
        return Err(SplatError::Process("顶点数超出 uint32 索引范围".into()));
    }

    let mut bin: Vec<u8> = Vec::with_capacity(vertex_count * 36 + mesh.indices.len() * 4);
    let push_vec3 = |bin: &mut Vec<u8>, values: &[[f32; 3]]| -> (usize, usize) {
        let offset = bin.len();
        for value in values {
            for channel in value {
                bin.extend_from_slice(&channel.to_le_bytes());
            }
        }
        (offset, bin.len() - offset)
    };
    let (positions_offset, positions_len) = push_vec3(&mut bin, &mesh.positions);
    let (normals_offset, normals_len) = push_vec3(&mut bin, &mesh.normals);
    let (colors_offset, colors_len) = push_vec3(&mut bin, &mesh.colors);
    let indices_offset = bin.len();
    for index in &mesh.indices {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    let indices_len = bin.len() - indices_offset;

    let (min, max) = mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
    let min: Vec<f64> = min.to_vec();
    let max: Vec<f64> = max.to_vec();
    let json = json!({
        "asset": { "version": "2.0", "generator": GENERATOR },
        "scene": 0,
        "scenes": [ { "nodes": [0] } ],
        "nodes": [ { "mesh": 0, "name": "object" } ],
        "meshes": [ {
            "name": "object",
            "primitives": [ {
                "attributes": { "POSITION": 0, "NORMAL": 1, "COLOR_0": 2 },
                "indices": 3,
                "mode": 4,
                "material": 0,
            } ],
        } ],
        "materials": [ {
            "name": "splat-vertex-color",
            "pbrMetallicRoughness": {
                "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
                "metallicFactor": 0.0,
                "roughnessFactor": 1.0,
            },
            "doubleSided": true,
        } ],
        "buffers": [ { "byteLength": bin.len() } ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": positions_offset, "byteLength": positions_len, "target": ARRAY_BUFFER },
            { "buffer": 0, "byteOffset": normals_offset, "byteLength": normals_len, "target": ARRAY_BUFFER },
            { "buffer": 0, "byteOffset": colors_offset, "byteLength": colors_len, "target": ARRAY_BUFFER },
            { "buffer": 0, "byteOffset": indices_offset, "byteLength": indices_len, "target": ELEMENT_ARRAY_BUFFER },
        ],
        "accessors": [
            { "bufferView": 0, "componentType": FLOAT, "count": vertex_count, "type": "VEC3", "min": min, "max": max },
            { "bufferView": 1, "componentType": FLOAT, "count": vertex_count, "type": "VEC3" },
            { "bufferView": 2, "componentType": FLOAT, "count": vertex_count, "type": "VEC3" },
            { "bufferView": 3, "componentType": UNSIGNED_INT, "count": mesh.indices.len(), "type": "SCALAR" },
        ],
    });

    let mut json_bytes = serde_json::to_vec(&json)?;
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(&MAGIC.to_le_bytes());
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&(total as u32).to_le_bytes());
    bytes.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    bytes.extend_from_slice(&json_bytes);
    bytes.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&CHUNK_BIN.to_le_bytes());
    bytes.extend_from_slice(&bin);
    debug_assert_eq!(bytes.len(), total);

    Ok(GlbOutput {
        bytes,
        vertex_count,
        triangle_count: mesh.triangle_count(),
    })
}

/// Writes `mesh` to `path` as GLB, creating parent directories as needed.
pub fn write_glb(path: &Path, mesh: &Mesh) -> Result<GlbOutput> {
    let output = encode_glb(mesh)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, &output.bytes)?;
    Ok(output)
}

/// Splits a GLB back into its JSON and BIN chunks.
///
/// Used by the acceptance check (and by tests) to validate the container
/// without a glTF library.
pub fn split_glb(bytes: &[u8]) -> Result<(Value, Vec<u8>)> {
    if bytes.len() < 20 || !bytes.len().is_multiple_of(4) {
        return Err(SplatError::Process("GLB 长度非法".into()));
    }
    if u32::from_le_bytes(bytes[0..4].try_into().expect("checked length")) != MAGIC {
        return Err(SplatError::Process("GLB magic 不是 glTF".into()));
    }
    if u32::from_le_bytes(bytes[4..8].try_into().expect("checked length")) != VERSION {
        return Err(SplatError::Process("GLB 版本不是 2".into()));
    }
    let declared = u32::from_le_bytes(bytes[8..12].try_into().expect("checked length")) as usize;
    if declared != bytes.len() {
        return Err(SplatError::Process(format!(
            "GLB 头声明长度 {declared} 与实际长度 {} 不一致",
            bytes.len()
        )));
    }
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().expect("checked length")) as usize;
    if u32::from_le_bytes(bytes[16..20].try_into().expect("checked length")) != CHUNK_JSON {
        return Err(SplatError::Process("第一个 chunk 不是 JSON".into()));
    }
    let json_start = 20;
    let json_end = json_start + json_len;
    if json_end > bytes.len() {
        return Err(SplatError::Process("JSON chunk 越界".into()));
    }
    let value: Value = serde_json::from_slice(&bytes[json_start..json_end])?;

    let mut bin = Vec::new();
    if json_end < bytes.len() {
        if json_end + 8 > bytes.len() {
            return Err(SplatError::Process("BIN chunk 头越界".into()));
        }
        let bin_len = u32::from_le_bytes(
            bytes[json_end..json_end + 4]
                .try_into()
                .expect("checked length"),
        ) as usize;
        if u32::from_le_bytes(
            bytes[json_end + 4..json_end + 8]
                .try_into()
                .expect("checked length"),
        ) != CHUNK_BIN
        {
            return Err(SplatError::Process("第二个 chunk 不是 BIN".into()));
        }
        let bin_start = json_end + 8;
        if bin_start + bin_len > bytes.len() {
            return Err(SplatError::Process("BIN chunk 越界".into()));
        }
        bin.extend_from_slice(&bytes[bin_start..bin_start + bin_len]);
    }
    Ok((value, bin))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tetrahedron() -> Mesh {
        Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            colors: vec![[1.0, 0.5, 0.25]; 4],
            indices: vec![0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3],
        }
    }

    #[test]
    fn writes_a_gltf2_container_with_the_declared_total_length() {
        let output = encode_glb(&tetrahedron()).unwrap();
        let bytes = &output.bytes;
        assert_eq!(&bytes[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        let declared = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        assert_eq!(declared, bytes.len(), "header length must match the file");
        assert_eq!(declared % 4, 0, "GLB must stay 4-byte aligned");
    }

    #[test]
    fn json_chunk_is_valid_and_padded() {
        let output = encode_glb(&tetrahedron()).unwrap();
        let bytes = &output.bytes;
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(
            u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            CHUNK_JSON
        );
        assert_eq!(json_len % 4, 0);
        let json_bytes = &bytes[20..20 + json_len];
        let parsed: Value = serde_json::from_slice(json_bytes).expect("valid JSON chunk");
        assert_eq!(parsed["asset"]["version"], "2.0");
        assert!(parsed["asset"]["generator"].is_string());
    }

    #[test]
    fn bin_chunk_follows_the_json_chunk() {
        let output = encode_glb(&tetrahedron()).unwrap();
        let bytes = &output.bytes;
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let bin_header = 20 + json_len;
        let bin_len =
            u32::from_le_bytes(bytes[bin_header..bin_header + 4].try_into().unwrap()) as usize;
        assert_eq!(
            u32::from_le_bytes(bytes[bin_header + 4..bin_header + 8].try_into().unwrap()),
            CHUNK_BIN
        );
        assert_eq!(bin_len % 4, 0);
        assert_eq!(bin_header + 8 + bin_len, bytes.len());
    }

    #[test]
    fn accessors_match_the_mesh_counts_and_buffers() {
        let mesh = tetrahedron();
        let output = encode_glb(&mesh).unwrap();
        let (json, bin) = split_glb(&output.bytes).unwrap();
        assert_eq!(output.vertex_count, 4);
        assert_eq!(output.triangle_count, 4);

        let accessors = json["accessors"].as_array().unwrap();
        assert_eq!(accessors.len(), 4, "POSITION, NORMAL, COLOR_0, indices");
        for accessor in accessors {
            let view = &json["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let offset = view["byteOffset"].as_u64().unwrap_or(0)
                + accessor["byteOffset"].as_u64().unwrap_or(0);
            let element_size = match accessor["componentType"].as_u64().unwrap() {
                value if value == FLOAT as u64 => {
                    4 * accessor["type"]
                        .as_str()
                        .map_or(1, |t| if t == "VEC3" { 3 } else { 1 })
                }
                value if value == UNSIGNED_INT as u64 => 4,
                _ => panic!("unexpected component type"),
            };
            let used = element_size * accessor["count"].as_u64().unwrap() as usize;
            assert!(
                offset as usize + used <= bin.len(),
                "accessor overruns the BIN chunk"
            );
        }
        assert_eq!(accessors[0]["count"], 4);
        assert_eq!(accessors[1]["count"], 4);
        assert_eq!(accessors[2]["count"], 4);
        assert_eq!(accessors[3]["count"], 12);
        assert_eq!(json["meshes"][0]["primitives"][0]["mode"], 4);
    }

    #[test]
    fn position_accessor_min_max_match_the_actual_vertices() {
        let mesh = tetrahedron();
        let output = encode_glb(&mesh).unwrap();
        let (json, bin) = split_glb(&output.bytes).unwrap();
        let accessor = &json["accessors"][0];
        let view = &json["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
        let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for index in 0..mesh.positions.len() {
            for axis in 0..3 {
                let start = offset + index * 12 + axis * 4;
                let value = f32::from_le_bytes(bin[start..start + 4].try_into().unwrap());
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
        for axis in 0..3 {
            assert_eq!(accessor["min"][axis].as_f64().unwrap() as f32, min[axis]);
            assert_eq!(accessor["max"][axis].as_f64().unwrap() as f32, max[axis]);
        }
    }

    #[test]
    fn rejects_an_empty_mesh() {
        assert!(encode_glb(&Mesh::default()).is_err());
    }

    #[test]
    fn round_trips_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("out.glb");
        let output = write_glb(&path, &tetrahedron()).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, output.bytes);
        let (json, _) = split_glb(&bytes).unwrap();
        assert_eq!(json["nodes"][0]["mesh"], 0);
    }
}
