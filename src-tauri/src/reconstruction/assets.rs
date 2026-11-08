//! Publication layout for a project's derived `assets/` directory (issue #11).
//!
//! Everything a project publishes lives under `<project_root>/assets/`:
//! `object-01.ply`, `background.ply`, `object-01.glb` plus a `manifest.json`
//! that names them, their size and their SHA-256. `final.ply` stays where the
//! training stage wrote it and is never overwritten: every file here is a
//! derived asset.
//!
//! The manifest is written through a temporary file plus an atomic replace, and
//! its entries are sorted by `(kind, index, format)` and keyed by the same
//! triple, so re-running a stage updates its own entry and leaves every other
//! entry untouched — the bytes only differ in `generatedAtMs`.

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    error::{Result, SplatError},
    reconstruction::ply::inspect_gaussian_ply,
};

/// Directory holding every published asset, relative to the project root.
pub const ASSETS_DIR: &str = "assets";
/// Manifest file name inside [`ASSETS_DIR`].
pub const MANIFEST_NAME: &str = "manifest.json";
/// The training output every asset is derived from, relative to the project root.
pub const FINAL_PLY_NAME: &str = "final.ply";
/// Background subset name inside [`ASSETS_DIR`].
pub const BACKGROUND_PLY_NAME: &str = "background.ply";
/// Object subset name written before issue #11; still recognised on read.
pub const LEGACY_OBJECT_PLY_NAME: &str = "object.ply";
/// Index of the first `object-XX` asset.
pub const FIRST_OBJECT_INDEX: usize = 1;
/// Background entries use index 0: a project has exactly one background.
pub const BACKGROUND_INDEX: usize = 0;
/// Manifest schema version this build writes.
pub const MANIFEST_VERSION: u32 = 1;

/// `object-01.ply` for `1`, `object-10.ply` for `10`, `object-100.ply` for `100`.
pub fn object_ply_name(index: usize) -> String {
    format!("object-{index:02}.ply")
}

/// `object-01.glb` for `1`, `object-10.glb` for `10`.
pub fn object_glb_name(index: usize) -> String {
    format!("object-{index:02}.glb")
}

/// Parses an `object-XX.ply` file name back into its index.
///
/// Only the canonical zero-padded spelling counts, so `object.ply` (the
/// pre-#11 name) and near misses like `object-1.ply` are never mistaken for a
/// numbered asset: the same asset can only ever be listed once.
pub fn object_ply_index(file_name: &str) -> Option<usize> {
    let digits = file_name.strip_prefix("object-")?.strip_suffix(".ply")?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let index = digits.parse::<usize>().ok()?;
    (index >= FIRST_OBJECT_INDEX && object_ply_name(index) == file_name).then_some(index)
}

/// Every numbered object cloud in `assets_dir`, ascending by index.
pub fn object_ply_assets(assets_dir: &Path) -> Vec<(usize, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(assets_dir) else {
        return Vec::new();
    };
    let mut found: Vec<(usize, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            object_ply_index(&name).map(|index| (index, entry.path()))
        })
        .collect();
    found.sort_by_key(|(index, _)| *index);
    found
}

/// Which subset an asset holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Object,
    Background,
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Background => "background",
        }
    }
}

/// Container an asset is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetFormat {
    Glb,
    Ply,
}

impl AssetFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Glb => "glb",
            Self::Ply => "ply",
        }
    }
}

/// One published file. `file` is the name inside `assets/` — never an absolute
/// path, so a manifest survives moving the project directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetEntry {
    pub kind: AssetKind,
    /// 1-based for objects, 0 for the single background.
    pub index: usize,
    pub format: AssetFormat,
    pub file: String,
    pub file_size: u64,
    pub sha256: String,
    /// Splat clouds only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splat_count: Option<u64>,
    /// Meshes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triangle_count: Option<u64>,
    /// Physical unit of a mesh's coordinates (`"scene"` or `"m"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<String>,
    /// Uniform factor a mesh's positions were rescaled by (`None` for splats).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_factor: Option<f64>,
}

impl AssetEntry {
    /// A bare entry; callers fill in the count fields they actually produced.
    pub fn new(
        kind: AssetKind,
        index: usize,
        format: AssetFormat,
        file: impl Into<String>,
        file_size: u64,
        sha256: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            index,
            format,
            file: file.into(),
            file_size,
            sha256: sha256.into(),
            splat_count: None,
            vertex_count: None,
            triangle_count: None,
            units: None,
            scale_factor: None,
        }
    }

    pub fn with_splat_count(mut self, splat_count: u64) -> Self {
        self.splat_count = Some(splat_count);
        self
    }

    /// Records what the mesh writer reported for a GLB.
    pub fn with_mesh(
        mut self,
        vertex_count: u64,
        triangle_count: u64,
        units: impl Into<String>,
        scale_factor: f64,
    ) -> Self {
        self.vertex_count = Some(vertex_count);
        self.triangle_count = Some(triangle_count);
        self.units = Some(units.into());
        self.scale_factor = Some(scale_factor);
        self
    }

    /// `(kind, index, format)`: the triple that makes an upsert idempotent.
    fn key(&self) -> (&'static str, usize, &'static str) {
        (self.kind.as_str(), self.index, self.format.as_str())
    }
}

/// The `final.ply` every asset in the manifest was derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSource {
    /// Relative to the project root.
    pub final_ply: String,
    pub final_ply_sha256: String,
    pub splat_count: u64,
}

/// `assets/manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetsManifest {
    pub version: u32,
    pub generated_at_ms: i64,
    pub source: AssetSource,
    /// Sorted by `(kind, index, format)`; unique on the same triple.
    pub assets: Vec<AssetEntry>,
}

/// `<assets_dir>/manifest.json`.
pub fn manifest_path(assets_dir: &Path) -> PathBuf {
    assets_dir.join(MANIFEST_NAME)
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// SHA-256 of a file, read in bounded chunks so multi-GB clouds stay feasible.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// `<project_root>/final.ply` identity for a manifest's `source` block.
pub fn source_from_final_ply(project_root: &Path) -> Result<AssetSource> {
    let path = project_root.join(FINAL_PLY_NAME);
    let info = inspect_gaussian_ply(&path).map_err(|error| {
        SplatError::Process(format!(
            "无法为资产清单登记源文件 {}（{error}）",
            path.display()
        ))
    })?;
    Ok(AssetSource {
        final_ply: FINAL_PLY_NAME.to_string(),
        final_ply_sha256: sha256_file(&path)?,
        splat_count: info.splat_count,
    })
}

/// Moves `source` onto `destination`, replacing it atomically: a reader either
/// sees the previous file or the complete new one, never a partial write.
#[cfg(windows)]
pub(crate) fn publish_file(source: &Path, destination: &Path) -> Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both paths are owned, NUL-terminated UTF-16 buffers that remain live for the call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn publish_file(source: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(source, destination)?;
    Ok(())
}

/// Writes `manifest` to `<assets_dir>/manifest.json` through a temporary file.
///
/// Returns the manifest path. The bytes are pretty-printed with a trailing
/// newline, so two runs over the same inputs differ only in `generatedAtMs`.
pub fn write_manifest_atomic(assets_dir: &Path, manifest: &AssetsManifest) -> Result<PathBuf> {
    std::fs::create_dir_all(assets_dir)?;
    let destination = manifest_path(assets_dir);
    let temporary = assets_dir.join(format!(".{MANIFEST_NAME}-{}.tmp", Uuid::new_v4()));
    let written = (|| -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(manifest)?;
        bytes.push(b'\n');
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = publish_file(&temporary, &destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(destination)
}

/// Reads `assets_dir/manifest.json`. A project published before the manifest
/// existed has no file, which is `Ok(None)`; a malformed one is an error so a
/// corrupt manifest can never silently accept a partial upsert.
pub fn read_manifest(assets_dir: &Path) -> Result<Option<AssetsManifest>> {
    let path = manifest_path(assets_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// Upserts one entry, keeping every other entry and the manifest's `source`.
pub fn upsert_asset_entry(
    assets_dir: &Path,
    source: AssetSource,
    entry: AssetEntry,
    generated_at_ms: i64,
) -> Result<AssetsManifest> {
    upsert_asset_entries(assets_dir, source, vec![entry], generated_at_ms)
}

/// Upserts `entries` by `(kind, index, format)` and rewrites `source`.
///
/// Entries the manifest already had under a different key are preserved, so a
/// stage only ever touches the assets it published itself.
pub fn upsert_asset_entries(
    assets_dir: &Path,
    source: AssetSource,
    entries: Vec<AssetEntry>,
    generated_at_ms: i64,
) -> Result<AssetsManifest> {
    let existing = read_manifest(assets_dir)?;
    let mut merged: BTreeMap<(&'static str, usize, &'static str), AssetEntry> = BTreeMap::new();
    if let Some(manifest) = existing {
        for entry in manifest.assets {
            merged.insert(entry.key(), entry);
        }
    }
    for entry in entries {
        merged.insert(entry.key(), entry);
    }
    let manifest = AssetsManifest {
        version: MANIFEST_VERSION,
        generated_at_ms,
        source,
        assets: merged.into_values().collect(),
    };
    write_manifest_atomic(assets_dir, &manifest)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::ply::PlyInfo;

    fn ply_info(file_size: u64, splat_count: u64) -> PlyInfo {
        PlyInfo {
            file_size,
            splat_count,
        }
    }

    fn source() -> AssetSource {
        AssetSource {
            final_ply: FINAL_PLY_NAME.to_string(),
            final_ply_sha256: "a".repeat(64),
            splat_count: 4_000,
        }
    }

    fn object_ply() -> AssetEntry {
        AssetEntry::new(
            AssetKind::Object,
            FIRST_OBJECT_INDEX,
            AssetFormat::Ply,
            object_ply_name(FIRST_OBJECT_INDEX),
            120,
            "b".repeat(64),
        )
        .with_splat_count(1_500)
    }

    fn background_ply() -> AssetEntry {
        AssetEntry::new(
            AssetKind::Background,
            BACKGROUND_INDEX,
            AssetFormat::Ply,
            BACKGROUND_PLY_NAME,
            80,
            "c".repeat(64),
        )
        .with_splat_count(2_500)
    }

    fn object_glb() -> AssetEntry {
        AssetEntry::new(
            AssetKind::Object,
            FIRST_OBJECT_INDEX,
            AssetFormat::Glb,
            object_glb_name(FIRST_OBJECT_INDEX),
            640,
            "d".repeat(64),
        )
        .with_mesh(320, 640, "scene", 1.0)
    }

    #[test]
    fn object_names_are_zero_padded_and_parsed_canonically() {
        assert_eq!(object_ply_name(1), "object-01.ply");
        assert_eq!(object_ply_name(9), "object-09.ply");
        assert_eq!(object_ply_name(10), "object-10.ply");
        assert_eq!(object_ply_name(100), "object-100.ply");
        assert_eq!(object_glb_name(1), "object-01.glb");
        assert_eq!(object_glb_name(12), "object-12.glb");

        assert_eq!(object_ply_index("object-01.ply"), Some(1));
        assert_eq!(object_ply_index("object-12.ply"), Some(12));
        assert_eq!(object_ply_index("object-100.ply"), Some(100));
        // Legacy name, unpadded spelling, other formats and stray digits are not numbered objects.
        assert_eq!(object_ply_index(LEGACY_OBJECT_PLY_NAME), None);
        assert_eq!(object_ply_index("object-1.ply"), None);
        assert_eq!(object_ply_index("object-001.ply"), None);
        assert_eq!(object_ply_index("object-00.ply"), None);
        assert_eq!(object_ply_index("object-01.glb"), None);
        assert_eq!(object_ply_index("background.ply"), None);
    }

    #[test]
    fn number_bookkeeping_helpers_read_the_assets_directory() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join(ASSETS_DIR);
        std::fs::create_dir_all(&assets).unwrap();
        for name in [
            "object-02.ply",
            "object-01.ply",
            LEGACY_OBJECT_PLY_NAME,
            BACKGROUND_PLY_NAME,
            "object-01.glb",
            "object-ten.ply",
        ] {
            std::fs::write(assets.join(name), b"x").unwrap();
        }
        std::fs::create_dir_all(assets.join("object-03.ply")).unwrap();

        let found = object_ply_assets(&assets);
        assert_eq!(
            found
                .iter()
                .map(|(index, path)| (
                    *index,
                    path.file_name().unwrap().to_string_lossy().into_owned()
                ))
                .collect::<Vec<_>>(),
            vec![
                (1, "object-01.ply".to_string()),
                (2, "object-02.ply".to_string())
            ]
        );
        assert!(object_ply_assets(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn manifest_round_trips_through_the_assets_directory() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join(ASSETS_DIR);
        assert_eq!(read_manifest(&assets).unwrap(), None);

        upsert_asset_entry(&assets, source(), object_ply(), 1_700_000_000_000).unwrap();
        let manifest = read_manifest(&assets).unwrap().expect("manifest written");
        assert_eq!(manifest.version, MANIFEST_VERSION);
        assert_eq!(manifest.generated_at_ms, 1_700_000_000_000);
        assert_eq!(manifest.source, source());
        assert_eq!(manifest.assets, vec![object_ply()]);
        assert!(!assets.join(format!(".{MANIFEST_NAME}-x.tmp")).exists());
        assert!(assets.join(MANIFEST_NAME).is_file());

        // A corrupt manifest is an error, not a silent fallback to "no manifest".
        std::fs::write(manifest_path(&assets), b"{ not json").unwrap();
        assert!(read_manifest(&assets).is_err());
    }

    #[test]
    fn upsert_is_idempotent_and_preserves_other_entries() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join(ASSETS_DIR);

        upsert_asset_entry(&assets, source(), object_ply(), 10).unwrap();
        upsert_asset_entry(&assets, source(), background_ply(), 20).unwrap();
        upsert_asset_entry(&assets, source(), object_glb(), 30).unwrap();
        let after_three = read_manifest(&assets).unwrap().unwrap();
        assert_eq!(
            after_three
                .assets
                .iter()
                .map(|entry| (entry.kind, entry.index, entry.format))
                .collect::<Vec<_>>(),
            vec![
                (AssetKind::Background, BACKGROUND_INDEX, AssetFormat::Ply),
                (AssetKind::Object, FIRST_OBJECT_INDEX, AssetFormat::Glb),
                (AssetKind::Object, FIRST_OBJECT_INDEX, AssetFormat::Ply),
            ]
        );

        // Re-publishing the same triple replaces it in place instead of adding one.
        let refreshed = object_glb()
            .with_mesh(400, 800, "m", 0.25)
            .with_splat_count(0);
        let mut refreshed = refreshed;
        refreshed.file_size = 999;
        upsert_asset_entry(&assets, source(), refreshed.clone(), 40).unwrap();
        let manifest = read_manifest(&assets).unwrap().unwrap();
        assert_eq!(manifest.assets.len(), 3);
        assert_eq!(
            manifest
                .assets
                .iter()
                .find(|entry| entry.format == AssetFormat::Glb),
            Some(&refreshed)
        );
        assert!(manifest
            .assets
            .iter()
            .any(|entry| entry.format == AssetFormat::Ply && entry.kind == AssetKind::Object));
        assert!(manifest
            .assets
            .iter()
            .any(|entry| entry.kind == AssetKind::Background));
    }

    #[test]
    fn batch_upsert_keeps_a_stale_entry_keyed_by_a_different_triple() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join(ASSETS_DIR);
        upsert_asset_entries(&assets, source(), vec![object_ply(), background_ply()], 10).unwrap();
        // A second object publish must not disturb object-01 or the background.
        let second = AssetEntry::new(
            AssetKind::Object,
            2,
            AssetFormat::Ply,
            object_ply_name(2),
            64,
            "e".repeat(64),
        )
        .with_splat_count(700);
        let manifest = upsert_asset_entries(&assets, source(), vec![second.clone()], 11).unwrap();
        assert_eq!(manifest.assets.len(), 3);
        assert_eq!(manifest.assets[1], object_ply());
        assert_eq!(manifest.assets[2], second);
        assert_eq!(manifest.generated_at_ms, 11);
    }

    #[test]
    fn two_runs_with_the_same_timestamp_are_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = |assets: &Path, stamp: i64| -> Vec<u8> {
            upsert_asset_entries(
                assets,
                source(),
                vec![object_glb(), background_ply(), object_ply()],
                stamp,
            )
            .unwrap();
            std::fs::read(manifest_path(assets)).unwrap()
        };

        let first = bytes(&dir.path().join("a"), 1_700_000_000_000);
        let second = bytes(&dir.path().join("b"), 1_700_000_000_000);
        assert_eq!(first, second);
        // Re-running in place is stable too (the entry order never drifts).
        let third = bytes(&dir.path().join("a"), 1_700_000_000_000);
        assert_eq!(first, third);

        let later = bytes(&dir.path().join("c"), 1_700_000_000_001);
        assert_ne!(first, later);
    }

    #[test]
    fn sha256_matches_an_independent_hash_of_the_same_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.bin");
        let body: Vec<u8> = (0..200_000_u32).map(|value| value as u8).collect();
        std::fs::write(&path, &body).unwrap();

        let expected = format!("{:x}", Sha256::digest(&body));
        assert_eq!(sha256_file(&path).unwrap(), expected);
        assert_eq!(sha256_bytes(&body), expected);
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn source_from_final_ply_hashes_and_counts_the_training_output() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let header = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float f_dc_0\nproperty float opacity\nproperty float scale_0\nproperty float rot_0\nend_header\n";
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(&[0_u8; 3 * 28]);
        std::fs::write(root.join(FINAL_PLY_NAME), &bytes).unwrap();

        let source = source_from_final_ply(root).unwrap();
        assert_eq!(source.final_ply, FINAL_PLY_NAME);
        assert_eq!(source.splat_count, 3);
        assert_eq!(source.final_ply_sha256, sha256_bytes(&bytes));
        assert!(source_from_final_ply(&root.join("elsewhere")).is_err());
    }

    #[test]
    fn entries_serialise_with_camel_case_sizes_and_omit_absent_counts() {
        let entry = object_ply();
        assert_eq!(entry.file_size, 120);
        assert_eq!(entry.splat_count, Some(1_500));
        assert_eq!(entry.file, "object-01.ply");
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"fileSize\":120"), "{json}");
        assert!(json.contains("\"splatCount\":1500"), "{json}");
        assert!(json.contains("\"kind\":\"object\""), "{json}");
        assert!(json.contains("\"format\":\"ply\""), "{json}");
        // A PLY entry carries no mesh counts at all.
        assert!(!json.contains("vertexCount"), "{json}");

        let glb = object_glb();
        let json = serde_json::to_string(&glb).unwrap();
        assert!(json.contains("\"vertexCount\":320"), "{json}");
        assert!(json.contains("\"triangleCount\":640"), "{json}");
        assert!(json.contains("\"units\":\"scene\""), "{json}");
        assert!(json.contains("\"scaleFactor\":1.0"), "{json}");
        assert!(!json.contains("splatCount"), "{json}");

        // The counts round-trip, so a manifest read back equals the one written.
        assert_eq!(serde_json::from_str::<AssetEntry>(&json).unwrap(), glb);
    }

    /// A `PlyInfo` converted into an entry keeps the size and the splat count.
    #[test]
    fn ply_info_feeds_an_entry_without_losing_its_numbers() {
        let info = ply_info(4_096, 1_024);
        let entry = AssetEntry::new(
            AssetKind::Object,
            FIRST_OBJECT_INDEX,
            AssetFormat::Ply,
            object_ply_name(FIRST_OBJECT_INDEX),
            info.file_size,
            "f".repeat(64),
        )
        .with_splat_count(info.splat_count);
        assert_eq!(entry.file_size, 4_096);
        assert_eq!(entry.splat_count, Some(1_024));
    }
}
