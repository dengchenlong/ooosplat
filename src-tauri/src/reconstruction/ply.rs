use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

use crate::error::{Result, SplatError};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlyInfo {
    pub file_size: u64,
    pub splat_count: u64,
}

const MAX_PLY_HEADER_BYTES: u64 = 256 * 1024;

fn trim_line_ending(mut line: &[u8]) -> &[u8] {
    if let Some(value) = line.strip_suffix(b"\n") {
        line = value;
    }
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn read_ply_header(reader: impl Read) -> Result<String> {
    let mut reader = BufReader::new(reader.take(MAX_PLY_HEADER_BYTES + 1));
    let mut header = Vec::new();
    loop {
        let line_start = header.len();
        let read = reader.read_until(b'\n', &mut header)?;
        if header.len() as u64 > MAX_PLY_HEADER_BYTES {
            return Err(SplatError::Process(format!(
                "PLY header 超过 {} KiB 安全限制",
                MAX_PLY_HEADER_BYTES / 1024
            )));
        }
        if read == 0 {
            break;
        }
        if trim_line_ending(&header[line_start..]) == b"end_header" {
            return String::from_utf8(header)
                .map_err(|_| SplatError::Process("PLY header 不是有效 UTF-8 文本".into()));
        }
    }
    Err(SplatError::Process("PLY 缺少 end_header".into()))
}

pub fn inspect_gaussian_ply(path: &Path) -> Result<PlyInfo> {
    let size = path.metadata()?.len();
    if size == 0 {
        return Err(SplatError::Process("Brush 输出的 PLY 为空".into()));
    }
    let header = read_ply_header(File::open(path)?)?;
    let mut lines = header.lines().map(str::trim_end);
    if lines.next() != Some("ply") {
        return Err(SplatError::Process("输出不是合法 PLY 文件".into()));
    }
    let lines = lines.collect::<Vec<_>>();
    let splat_count = lines
        .iter()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next() == Some("element") && fields.next() == Some("vertex"))
                .then(|| fields.next()?.parse::<u64>().ok())
                .flatten()
        })
        .unwrap_or(0);
    if splat_count == 0 {
        return Err(SplatError::Process("PLY 不包含 Gaussian 顶点".into()));
    }
    for property in [
        " x", " y", " z", " f_dc_0", " opacity", " scale_0", " rot_0",
    ] {
        if !lines.iter().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some("property")
                && fields.next().is_some()
                && fields.next() == Some(property.trim())
        }) {
            return Err(SplatError::Process(format!(
                "PLY 缺少 Gaussian 属性：{}",
                property.trim()
            )));
        }
    }
    Ok(PlyInfo {
        file_size: size,
        splat_count,
    })
}

pub fn validate_gaussian_ply(path: &Path) -> Result<u64> {
    Ok(inspect_gaussian_ply(path)?.file_size)
}

/// Minimal per-splat data needed for back-projection voting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplatCore {
    pub position: [f64; 3],
    pub scale: [f64; 3],
}

/// Byte offsets of the properties the voting stage reads from each record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplatPropertyOffsets {
    pub x: usize,
    pub y: usize,
    pub z: usize,
    pub scale_0: usize,
    pub scale_1: usize,
    pub scale_2: usize,
}

/// PLY layout information for streaming reads: property byte offsets, the
/// record stride, and the byte length of the header (where the payload starts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlyLayout {
    pub splat_count: u64,
    pub stride: usize,
    pub header_bytes: usize,
    pub offsets: SplatPropertyOffsets,
}

fn scalar_type_bytes(type_name: &str) -> Option<usize> {
    Some(match type_name {
        "char" | "int8" | "uchar" | "uint8" => 1,
        "short" | "int16" | "ushort" | "uint16" => 2,
        "int" | "int32" | "uint" | "uint32" | "float" | "float32" => 4,
        "double" | "float64" => 8,
        _ => return None,
    })
}

fn property_offset(lines: &[&str], name: &str, offset_out: &mut usize) -> Result<usize> {
    let mut offset = 0_usize;
    let mut found = false;
    let mut found_offset = 0_usize;
    for line in lines {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("property") {
            continue;
        }
        let type_name = fields.next().unwrap_or_default();
        let property = fields.next().unwrap_or_default();
        let size = scalar_type_bytes(type_name).ok_or_else(|| {
            SplatError::Process(format!("PLY 包含无法解析的属性类型：{type_name}"))
        })?;
        if property == name {
            if type_name != "float" && type_name != "float32" {
                return Err(SplatError::Process(format!(
                    "PLY 属性 {name} 必须是 float 类型，实际为 {type_name}"
                )));
            }
            *offset_out = offset;
            found = true;
            found_offset = offset;
        }
        offset += size;
    }
    if !found {
        return Err(SplatError::Process(format!(
            "PLY 缺少 Gaussian 属性：{name}"
        )));
    }
    Ok(found_offset)
}

/// Parses a Gaussian PLY header into a streaming layout plan.
///
/// Never loads the payload: callers re-open the file and seek past
/// `header_bytes`, reading `stride`-byte records in batches.
pub fn parse_gaussian_ply_layout(path: &Path) -> Result<PlyLayout> {
    let header = read_ply_header(File::open(path)?)?;
    let mut lines = header.lines().map(str::trim_end);
    if lines.next() != Some("ply") {
        return Err(SplatError::Process("输出不是合法 PLY 文件".into()));
    }
    let is_binary_le = lines.clone().any(|line| {
        line.split_whitespace().take(2).collect::<Vec<_>>() == ["format", "binary_little_endian"]
    });
    if !is_binary_le {
        return Err(SplatError::Process(
            "仅支持 binary_little_endian 格式的 PLY".into(),
        ));
    }
    let lines = lines.collect::<Vec<_>>();
    let splat_count = lines
        .iter()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next() == Some("element") && fields.next() == Some("vertex"))
                .then(|| fields.next()?.parse::<u64>().ok())
                .flatten()
        })
        .unwrap_or(0);
    if splat_count == 0 {
        return Err(SplatError::Process("PLY 不包含 Gaussian 顶点".into()));
    }

    let mut offsets = SplatPropertyOffsets {
        x: 0,
        y: 0,
        z: 0,
        scale_0: 0,
        scale_1: 0,
        scale_2: 0,
    };
    let mut max_offset = property_offset(&lines, "x", &mut offsets.x)?;
    max_offset = max_offset.max(property_offset(&lines, "y", &mut offsets.y)?);
    max_offset = max_offset.max(property_offset(&lines, "z", &mut offsets.z)?);
    max_offset = max_offset.max(property_offset(&lines, "scale_0", &mut offsets.scale_0)?);
    max_offset = max_offset.max(property_offset(&lines, "scale_1", &mut offsets.scale_1)?);
    max_offset = max_offset.max(property_offset(&lines, "scale_2", &mut offsets.scale_2)?);

    let stride = lines
        .iter()
        .filter(|line| line.split_whitespace().next() == Some("property"))
        .try_fold(0_usize, |total, line| {
            let type_name = line
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| SplatError::Process("PLY property 行缺少类型".into()))?;
            scalar_type_bytes(type_name)
                .map(|size| total + size)
                .ok_or_else(|| {
                    SplatError::Process(format!("PLY 包含无法解析的属性类型：{type_name}"))
                })
        })?;
    if stride < max_offset + 4 {
        return Err(SplatError::Process("PLY 属性布局与 stride 不一致".into()));
    }

    Ok(PlyLayout {
        splat_count,
        stride,
        header_bytes: header.len(),
        offsets,
    })
}

/// Streams `SplatCore` values from a binary Gaussian PLY in batches.
///
/// Memory usage is bounded by `batch_size` records: the payload is never
/// materialised as a whole, which keeps voting on multi-GB point clouds
/// feasible. Returns the number of splats streamed. `final.ply` itself is
/// only ever opened for reading.
pub fn stream_splat_cores(
    path: &Path,
    batch_size: usize,
    sink: &mut dyn FnMut(&[SplatCore]) -> Result<()>,
) -> Result<u64> {
    if batch_size == 0 {
        return Err(SplatError::Process("PLY 流式读取批大小必须大于 0".into()));
    }
    let layout = parse_gaussian_ply_layout(path)?;
    let mut file = File::open(path)?;
    std::io::Seek::seek(
        &mut file,
        std::io::SeekFrom::Start(layout.header_bytes as u64),
    );
    let mut reader = BufReader::with_capacity(batch_size * layout.stride, file);
    let mut record = vec![0_u8; layout.stride];
    let mut batch: Vec<SplatCore> = Vec::with_capacity(batch_size);
    let mut streamed = 0_u64;
    let read_f32 = |bytes: &[u8], offset: usize| {
        f32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("offset in range"),
        )
    };
    while streamed < layout.splat_count {
        batch.clear();
        for _ in 0..batch_size.min((layout.splat_count - streamed) as usize) {
            reader.read_exact(&mut record)?;
            batch.push(SplatCore {
                position: [
                    read_f32(&record, layout.offsets.x) as f64,
                    read_f32(&record, layout.offsets.y) as f64,
                    read_f32(&record, layout.offsets.z) as f64,
                ],
                scale: [
                    read_f32(&record, layout.offsets.scale_0) as f64,
                    read_f32(&record, layout.offsets.scale_1) as f64,
                    read_f32(&record, layout.offsets.scale_2) as f64,
                ],
            });
        }
        streamed += batch.len() as u64;
        sink(&batch)?;
    }
    Ok(streamed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    struct ChunkedReader<R> {
        inner: R,
        chunk_size: usize,
    }

    impl<R: Read> Read for ChunkedReader<R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let limit = buffer.len().min(self.chunk_size);
            self.inner.read(&mut buffer[..limit])
        }
    }

    const VALID_PROPERTIES: &str = "property float x\nproperty float y\nproperty float z\nproperty float f_dc_0\nproperty float opacity\nproperty float scale_0\nproperty float rot_0\n";

    #[test]
    fn parses_splat_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valid.ply");
        std::fs::write(&path, b"ply\nformat binary_little_endian 1.0\nelement vertex 42\nproperty float x\nproperty float y\nproperty float z\nproperty float f_dc_0\nproperty float opacity\nproperty float scale_0\nproperty float rot_0\nend_header\n").unwrap();
        assert_eq!(inspect_gaussian_ply(&path).unwrap().splat_count, 42);
    }
    #[test]
    fn rejects_plain_point_cloud() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.ply");
        std::fs::write(&path, b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n").unwrap();
        assert!(inspect_gaussian_ply(&path).is_err());
    }

    #[test]
    fn reads_a_header_across_legal_short_reads() {
        let document = format!(
            "ply\nformat binary_little_endian 1.0\nelement vertex 42\n{VALID_PROPERTIES}end_header\n"
        );
        let reader = ChunkedReader {
            inner: Cursor::new(document.as_bytes()),
            chunk_size: 3,
        };
        assert!(read_ply_header(reader)
            .unwrap()
            .contains("element vertex 42"));
    }

    #[test]
    fn accepts_crlf_and_flexible_header_spacing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crlf.ply");
        let header = format!(
            "ply\r\nformat binary_little_endian 1.0\r\nelement   vertex   7\r\n{}end_header\r\n",
            VALID_PROPERTIES.replace('\n', "\r\n")
        );
        std::fs::write(&path, header).unwrap();
        assert_eq!(inspect_gaussian_ply(&path).unwrap().splat_count, 7);
    }

    #[test]
    fn rejects_missing_or_oversized_headers() {
        assert!(read_ply_header(Cursor::new(b"ply\ncomment no marker\n")).is_err());
        let oversized = vec![b'x'; MAX_PLY_HEADER_BYTES as usize + 1];
        assert!(read_ply_header(Cursor::new(oversized)).is_err());
    }

    const STREAM_PROPERTIES: &str = "property float x\nproperty float y\nproperty float z\nproperty float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\nproperty float opacity\nproperty float scale_0\nproperty float scale_1\nproperty float scale_2\nproperty float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3\n";

    fn write_stream_fixture(
        path: &Path,
        splat_count: usize,
        positions: &[[f64; 3]],
        scales: &[[f64; 3]],
    ) {
        let header = format!(
            "ply\nformat binary_little_endian 1.0\nelement vertex {splat_count}\n{STREAM_PROPERTIES}end_header\n"
        );
        let mut body = Vec::new();
        for i in 0..splat_count {
            for value in positions[i] {
                body.extend_from_slice(&(value as f32).to_le_bytes());
            }
            for channel in 0..3 {
                body.extend_from_slice(&(0.1_f32).to_le_bytes());
                let _ = channel;
            }
            body.extend_from_slice(&(0.9_f32).to_le_bytes());
            for value in scales[i] {
                body.extend_from_slice(&(value as f32).to_le_bytes());
            }
            for _ in 0..4 {
                body.extend_from_slice(&(0.5_f32).to_le_bytes());
            }
        }
        let mut bytes = header.into_bytes();
        bytes.extend_from_slice(&body);
        std::fs::write(path, bytes).unwrap();
    }

    fn collect_splats(path: &Path, batch_size: usize) -> (u64, Vec<[f64; 3]>, Vec<usize>) {
        let mut positions = Vec::new();
        let mut batch_sizes = Vec::new();
        let count = stream_splat_cores(path, batch_size, &mut |batch| {
            batch_sizes.push(batch.len());
            for splat in batch {
                positions.push(splat.position);
            }
            Ok(())
        })
        .unwrap();
        (count, positions, batch_sizes)
    }

    #[test]
    fn computes_layout_stride_from_property_types() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.ply");
        write_stream_fixture(
            &path,
            2,
            &[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
            &[[0.01; 3]; 2],
        );
        let layout = parse_gaussian_ply_layout(&path).unwrap();
        assert_eq!(layout.splat_count, 2);
        // 14 float properties -> 56-byte stride, not the canonical 62.
        assert_eq!(layout.stride, 56);
        assert_eq!(
            layout.header_bytes,
            path.metadata().unwrap().len() as usize - 2 * 56
        );
    }

    #[test]
    fn streams_splat_cores_in_batches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream.ply");
        let positions: Vec<[f64; 3]> = (0..1000).map(|i| [i as f64, 0.0, 0.0]).collect();
        let scales = vec![[0.01, 0.02, 0.03]; 1000];
        write_stream_fixture(&path, 1000, &positions, &scales);
        let (count, read_positions, batch_sizes) = collect_splats(&path, 300);
        assert_eq!(count, 1000);
        assert_eq!(read_positions.len(), 1000);
        assert_eq!(read_positions[0], [0.0, 0.0, 0.0]);
        assert_eq!(read_positions[999], [999.0, 0.0, 0.0]);
        assert_eq!(batch_sizes, vec![300, 300, 300, 100]);
    }

    #[test]
    fn streams_short_final_batch_with_exact_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.ply");
        let positions = vec![[1.5, 2.5, 3.5]];
        write_stream_fixture(&path, 1, &positions, &[[0.01; 3]]);
        let (count, read_positions, batch_sizes) = collect_splats(&path, 8);
        assert_eq!(count, 1);
        assert_eq!(read_positions, vec![[1.5, 2.5, 3.5]]);
        assert_eq!(batch_sizes, vec![1]);
    }

    #[test]
    fn rejects_layout_missing_scale_properties() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noscale.ply");
        std::fs::write(
            &path,
            b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n",
        )
        .unwrap();
        assert!(parse_gaussian_ply_layout(&path).is_err());
    }
}
