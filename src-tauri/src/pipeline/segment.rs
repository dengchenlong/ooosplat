use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, SplatError};

/// Bump when the mask manifest layout changes in a way that makes older
/// checkpoints unsafe to resume from (e.g. different frame identity).
pub const SEGMENT_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentManifestFrame {
    pub frame_id: String,
    pub mask_path: String,
    pub coverage: f64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentManifest {
    pub version: u32,
    pub prompt_digest: String,
    pub frames: Vec<SegmentManifestFrame>,
}

/// Stable digest binding the manifest to one prompt + one frame set.
///
/// `canonical` must be the exact canonical JSON the SAM2 sidecar hashes into
/// its own `framesHash` (see `engines/sam2/sidecar/sam2_sidecar/server.py`), so
/// a checkpoint written by the sidecar validates identically on the Rust side.
pub fn prompt_digest(canonical: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

impl SegmentManifest {
    /// Load and fully validate a checkpoint manifest from `work/segment/`.
    ///
    /// Returns `None` when absent — the stage has never run. Any inconsistency
    /// (version, digest, missing mask file, missing field, unsorted or
    /// duplicate frame ids) yields an error so the caller restarts the stage
    /// instead of resuming from a silently incomplete checkpoint.
    pub async fn load_checked(dir: &Path, expected_digest: &str) -> Result<Option<Self>> {
        let path = dir.join("manifest.json");
        if !path.is_file() {
            return Ok(None);
        }
        let raw = tokio::fs::read(&path).await?;
        let manifest: SegmentManifest = serde_json::from_slice(&raw)
            .map_err(|e| SplatError::Process(format!("分割清单损坏，无法断点续跑：{e}")))?;
        manifest.check(dir, expected_digest).await?;
        Ok(Some(manifest))
    }

    pub async fn check(&self, dir: &Path, expected_digest: &str) -> Result<()> {
        if self.version != SEGMENT_MANIFEST_VERSION {
            return Err(SplatError::Process(format!(
                "分割清单版本不兼容：{} != {}",
                self.version, SEGMENT_MANIFEST_VERSION
            )));
        }
        if self.prompt_digest != expected_digest {
            return Err(SplatError::Process(
                "分割清单与当前提示词/帧集不匹配，检查点作废".into(),
            ));
        }
        let mut previous: Option<&SegmentManifestFrame> = None;
        for frame in &self.frames {
            if frame.frame_id.is_empty() || frame.mask_path.is_empty() {
                return Err(SplatError::Process(
                    "分割清单存在缺少 frameId 或 maskPath 的帧".into(),
                ));
            }
            if frame.coverage < 0.0 || frame.coverage > 1.0 {
                return Err(SplatError::Process(format!(
                    "分割清单帧 {} 的覆盖率越界：{}",
                    frame.frame_id, frame.coverage
                )));
            }
            if let Some(prev) = previous {
                if frame.frame_id <= prev.frame_id {
                    return Err(SplatError::Process(format!(
                        "分割清单帧顺序必须按 frameId 严格递增：{} 出现在 {} 之后",
                        frame.frame_id, prev.frame_id
                    )));
                }
            }
            previous = Some(frame);
            let mask = dir.join(&frame.mask_path);
            if !mask.is_file() {
                return Err(SplatError::Process(format!(
                    "分割清单引用的 mask 文件缺失：{}",
                    frame.mask_path
                )));
            }
        }
        Ok(())
    }

    pub async fn write(&self, dir: &Path) -> Result<()> {
        tokio::fs::create_dir_all(dir).await?;
        let payload = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(dir.join("manifest.json"), payload).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &str, mask: &str, coverage: f64) -> SegmentManifestFrame {
        SegmentManifestFrame {
            frame_id: id.into(),
            mask_path: mask.into(),
            coverage,
            width: 64,
            height: 48,
        }
    }

    #[tokio::test]
    async fn missing_manifest_means_never_ran() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SegmentManifest::load_checked(dir.path(), "digest")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn valid_checkpoint_round_trips_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("000000.png"), b"png")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("000001.png"), b"png")
            .await
            .unwrap();
        let manifest = SegmentManifest {
            version: SEGMENT_MANIFEST_VERSION,
            prompt_digest: "digest".into(),
            frames: vec![
                frame("000000", "000000.png", 0.1),
                frame("000001", "000001.png", 0.2),
            ],
        };
        manifest.write(dir.path()).await.unwrap();
        let loaded = SegmentManifest::load_checked(dir.path(), "digest")
            .await
            .unwrap()
            .expect("manifest should load");
        assert_eq!(loaded, manifest);
    }

    #[tokio::test]
    async fn digest_mismatch_invalidates_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = SegmentManifest {
            version: SEGMENT_MANIFEST_VERSION,
            prompt_digest: "old".into(),
            frames: vec![frame("000000", "000000.png", 0.1)],
        };
        manifest.write(dir.path()).await.unwrap();
        let err = SegmentManifest::load_checked(dir.path(), "new")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("不匹配"));
    }

    #[tokio::test]
    async fn missing_mask_file_invalidates_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = SegmentManifest {
            version: SEGMENT_MANIFEST_VERSION,
            prompt_digest: "digest".into(),
            frames: vec![frame("000000", "000000.png", 0.1)],
        };
        manifest.write(dir.path()).await.unwrap();
        let err = SegmentManifest::load_checked(dir.path(), "digest")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("缺失"));
    }

    #[tokio::test]
    async fn unsorted_or_duplicate_frames_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("000000.png"), b"png")
            .await
            .unwrap();
        let manifest = SegmentManifest {
            version: SEGMENT_MANIFEST_VERSION,
            prompt_digest: "digest".into(),
            frames: vec![
                frame("000001", "000000.png", 0.1),
                frame("000000", "000000.png", 0.1),
            ],
        };
        manifest.write(dir.path()).await.unwrap();
        assert!(SegmentManifest::load_checked(dir.path(), "digest")
            .await
            .is_err());

        let duplicate = SegmentManifest {
            frames: vec![
                frame("000000", "000000.png", 0.1),
                frame("000000", "000000.png", 0.1),
            ],
            ..manifest
        };
        assert!(duplicate.check(dir.path(), "digest").await.is_err());
    }

    #[tokio::test]
    async fn out_of_range_coverage_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("000000.png"), b"png")
            .await
            .unwrap();
        let manifest = SegmentManifest {
            version: SEGMENT_MANIFEST_VERSION,
            prompt_digest: "digest".into(),
            frames: vec![frame("000000", "000000.png", 1.5)],
        };
        manifest.write(dir.path()).await.unwrap();
        let err = SegmentManifest::load_checked(dir.path(), "digest")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("越界"));
    }

    #[tokio::test]
    async fn wrong_manifest_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("000000.png"), b"png")
            .await
            .unwrap();
        let manifest = SegmentManifest {
            version: SEGMENT_MANIFEST_VERSION + 7,
            prompt_digest: "digest".into(),
            frames: vec![frame("000000", "000000.png", 0.1)],
        };
        manifest.write(dir.path()).await.unwrap();
        let err = SegmentManifest::load_checked(dir.path(), "digest")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("版本"));
    }

    #[tokio::test]
    async fn empty_manifest_is_a_valid_no_masks_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = SegmentManifest {
            version: SEGMENT_MANIFEST_VERSION,
            prompt_digest: "digest".into(),
            frames: Vec::new(),
        };
        manifest.write(dir.path()).await.unwrap();
        let loaded = SegmentManifest::load_checked(dir.path(), "digest")
            .await
            .unwrap()
            .expect("no-masks checkpoint should validate");
        assert!(loaded.frames.is_empty());
    }

    #[test]
    fn digest_is_stable_sha256_hex() {
        // 64-char lowercase hex, differs per input, independent of the sidecar.
        let digest = prompt_digest("{\"frames\":[]}");
        assert_eq!(digest.len(), 64);
        assert!(digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_ne!(digest, prompt_digest("{\"frames\":[],\"x\":1}"));
    }

    #[test]
    fn old_state_json_without_segment_key_still_parses() {
        let json = r#"{
          "stage":"extractingFrames","preset":"balanced","video":null,
          "featuresComplete":false,"matchingComplete":false,
          "reconstructionComplete":false,"brushComplete":false
        }"#;
        let state: crate::project::metadata::PipelineStateFile =
            serde_json::from_str(json).unwrap();
        assert_eq!(
            state.segment,
            crate::project::metadata::SegmentCheckpoint::default()
        );
        assert!(!state.segment.complete);
    }
}
