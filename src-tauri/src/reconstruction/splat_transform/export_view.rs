//! Seekable edited PLY bytes, without a complete in-memory or on-disk snapshot.
use super::*;
use std::io::{Seek, SeekFrom};
use std::time::SystemTime;

pub struct EditedPlyView {
    path: PathBuf,
    reader: BufReader<File>,
    cursor: u64,
    source_size: u64,
    modified: Option<SystemTime>,
    layout: PlyLayout,
    transform: RowTransform,
    header: Vec<u8>,
    indices: Option<Vec<u64>>,
    info: PlyInfo,
    cached_index: Option<u64>,
    cached_row: Vec<u8>,
}

impl EditedPlyView {
    pub fn open(
        path: &Path,
        transform: GaussianTransform,
        edits: GaussianExportEdits<'_>,
        budget: Option<u64>,
        mut progress: impl FnMut(u64, u64),
        check: impl Fn() -> Result<()>,
    ) -> Result<Self> {
        check()?;
        let path = std::fs::canonicalize(path)?;
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        let mut reader = BufReader::with_capacity(1_024_000, file);
        let layout = parse_layout(&mut reader)?;
        let source_vertex_end = (layout.header.len() as u64)
            .checked_add(
                layout
                    .count
                    .checked_mul(layout.stride as u64)
                    .ok_or_else(|| unsupported("PLY size overflow"))?,
            )
            .ok_or_else(|| unsupported("PLY size overflow"))?;
        if source_vertex_end > metadata.len() {
            return Err(unsupported("PLY 数据不完整"));
        }
        if let Some(mask) = edits.deleted_mask {
            if mask.len() as u64 != layout.count.div_ceil(8) {
                return Err(unsupported("Gaussian 删除位图与源 PLY 数量不一致"));
            }
        }
        let (mapped, rotation) = engine_transform_to_ply(transform.validate()?);
        let row_transform = RowTransform::new(&layout, mapped, rotation);
        let filtered = edits.crop.is_some()
            || edits
                .deleted_mask
                .is_some_and(|mask| mask.iter().any(|b| *b != 0));
        let mut indices = filtered.then(Vec::new);
        if let Some(indices) = &mut indices {
            let mut row = vec![0; layout.stride];
            for index in 0..layout.count {
                if index % ROWS_PER_CHUNK as u64 == 0 {
                    check()?;
                    progress(index, layout.count);
                }
                if edits.crop.is_some() {
                    reader.read_exact(&mut row)?;
                }
                if is_deleted(edits.deleted_mask, index) {
                    continue;
                }
                if let Some(crop) = edits.crop {
                    let p = row_transform
                        .xyz
                        .map(|offset| read_float(&row, offset) * mapped.scale);
                    let p = if row_transform.rotated {
                        rotation.transform_point(p)
                    } else {
                        p
                    };
                    if !crop.contains([
                        -(p[0] + mapped.position[0]),
                        -(p[1] + mapped.position[1]),
                        p[2] + mapped.position[2],
                    ]) {
                        continue;
                    }
                }
                indices
                    .try_reserve(1)
                    .map_err(|_| unsupported("无法分配导出索引，请选择标准 PLY"))?;
                indices.push(index);
                if let Some(budget) = budget {
                    if index % ROWS_PER_CHUNK as u64 == 0 || indices.len().is_power_of_two() {
                        let bytes = layout.header.len() as u64
                            + indices.len() as u64 * layout.stride as u64
                            + metadata.len()
                            - source_vertex_end;
                        if estimated(bytes, indices.len() as u64, indices.capacity() as u64 * 8)
                            > budget
                        {
                            return Err(unsupported("压缩预计超过安全内存预算，请选择标准 PLY"));
                        }
                    }
                }
            }
            progress(layout.count, layout.count);
        }
        let count = indices.as_ref().map_or(layout.count, |v| v.len() as u64);
        if count == 0 {
            return Err(unsupported(
                "当前裁切和删除状态没有保留任何 Gaussian，无法导出",
            ));
        }
        let header = if count == layout.count {
            layout.header.clone()
        } else {
            header_with_vertex_count(&layout.header, count)
        };
        let info = PlyInfo {
            file_size: header.len() as u64 + count * layout.stride as u64 + metadata.len()
                - source_vertex_end,
            splat_count: count,
        };
        let cursor = reader.stream_position()?;
        let cached_row = vec![0; layout.stride];
        let result = Self {
            path,
            reader,
            cursor,
            source_size: metadata.len(),
            modified: metadata.modified().ok(),
            layout,
            transform: row_transform,
            header,
            indices,
            info,
            cached_index: None,
            cached_row,
        };
        result.check_source()?;
        if budget.is_some_and(|budget| result.estimated_memory() > budget) {
            return Err(unsupported("压缩预计超过安全内存预算，请选择标准 PLY"));
        }
        check()?;
        Ok(result)
    }
    pub fn info(&self) -> PlyInfo {
        self.info.clone()
    }
    pub fn sh_bands(&self) -> u8 {
        match self.layout.rest_count {
            0 => 0,
            9 => 1,
            24 => 2,
            _ => 3,
        }
    }
    pub fn estimated_memory(&self) -> u64 {
        estimated(
            self.info.file_size,
            self.info.splat_count,
            self.indices.as_ref().map_or(0, |v| v.capacity() as u64 * 8),
        )
    }
    pub fn check_source(&self) -> Result<()> {
        let metadata = std::fs::metadata(&self.path)?;
        if metadata.len() != self.source_size || metadata.modified().ok() != self.modified {
            return Err(unsupported("模型源文件已发生变化，请重新打开预览"));
        }
        Ok(())
    }
    fn seek(&mut self, position: u64) -> Result<()> {
        if position != self.cursor {
            let delta = position as i128 - self.cursor as i128;
            if let Ok(delta) = i64::try_from(delta) {
                self.reader.seek_relative(delta)?;
            } else {
                self.reader.seek(SeekFrom::Start(position))?;
            }
            self.cursor = position;
        }
        Ok(())
    }
    pub fn read(
        &mut self,
        position: u64,
        length: usize,
        check: impl Fn() -> Result<()>,
    ) -> Result<Vec<u8>> {
        check()?;
        self.check_source()?;
        if position > self.info.file_size || length > 1_024_000 {
            return Err(unsupported("导出读取越界"));
        }
        let length = length.min((self.info.file_size - position) as usize);
        let mut output = vec![0; length];
        // No edits: the original file itself is the virtual view, not a copy.
        if self.indices.is_none() && self.transform.identity {
            self.seek(position)?;
            self.reader.read_exact(&mut output)?;
            self.cursor += length as u64;
            return Ok(output);
        }
        let mut offset = position;
        let end = position + length as u64;
        let header_end = self.header.len() as u64;
        let vertex_end = header_end + self.info.splat_count * self.layout.stride as u64;
        while offset < end {
            check()?;
            let target = (offset - position) as usize;
            if offset < header_end {
                let n = (end.min(header_end) - offset) as usize;
                output[target..target + n]
                    .copy_from_slice(&self.header[offset as usize..offset as usize + n]);
                offset += n as u64;
            } else if offset < vertex_end {
                let logical = (offset - header_end) / self.layout.stride as u64;
                let within = ((offset - header_end) % self.layout.stride as u64) as usize;
                if self.cached_index != Some(logical) {
                    let source = self
                        .indices
                        .as_ref()
                        .map_or(logical, |indices| indices[logical as usize]);
                    self.seek(
                        self.layout.header.len() as u64 + source * self.layout.stride as u64,
                    )?;
                    self.reader.read_exact(&mut self.cached_row)?;
                    self.cursor += self.layout.stride as u64;
                    self.transform.apply(&mut self.cached_row)?;
                    self.cached_index = Some(logical);
                }
                let n = (self.layout.stride - within).min((end - offset) as usize);
                output[target..target + n].copy_from_slice(&self.cached_row[within..within + n]);
                offset += n as u64;
            } else {
                let source = self.layout.header.len() as u64
                    + self.layout.count * self.layout.stride as u64
                    + offset
                    - vertex_end;
                self.seek(source)?;
                let n = (end - offset) as usize;
                self.reader.read_exact(&mut output[target..target + n])?;
                self.cursor += n as u64;
                offset = end;
            }
        }
        Ok(output)
    }
    /// Writes only the final format's atomic-publication temporary file.
    pub fn write_to(
        &mut self,
        path: &Path,
        mut progress: impl FnMut(u64, u64),
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        let mut writer = BufWriter::with_capacity(
            1_024_000,
            OpenOptions::new().write(true).create_new(true).open(path)?,
        );
        let mut offset = 0;
        while offset < self.info.file_size {
            let bytes = self.read(offset, 1_024_000, &check)?;
            writer.write_all(&bytes)?;
            offset += bytes.len() as u64;
            progress(offset, self.info.file_size);
        }
        check()?;
        self.check_source()?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        Ok(())
    }
}
fn estimated(bytes: u64, count: u64, index_bytes: u64) -> u64 {
    bytes
        .saturating_mul(6)
        .saturating_add(count.saturating_mul(128))
        .saturating_add(64 * 1024 * 1024)
        .saturating_add(index_bytes)
}
