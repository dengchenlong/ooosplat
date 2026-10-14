use std::{
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use crate::error::{Result, SplatError};

#[derive(Debug, Clone, PartialEq)]
pub struct ColmapCamera {
    pub id: u32,
    pub model_id: i32,
    pub model: String,
    pub width: u64,
    pub height: u64,
}

/// A COLMAP camera with its raw intrinsics parameters, in the order defined
/// by the camera model (see `camera_model`).
#[derive(Debug, Clone, PartialEq)]
pub struct CameraIntrinsics {
    pub camera: ColmapCamera,
    pub params: Vec<f64>,
}

/// COLMAP world-to-camera pose from `images.bin`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImagePose {
    /// COLMAP `qvec` (w, x, y, z): rotation from world to camera frame.
    pub quaternion: [f64; 4],
    /// COLMAP `tvec`: translation from world to camera frame.
    pub translation: [f64; 3],
}

/// A registered COLMAP image together with its world-to-camera pose.
#[derive(Debug, Clone, PartialEq)]
pub struct PosedImage {
    pub id: u32,
    pub camera_id: u32,
    pub name: String,
    pub pose: ImagePose,
}

fn read_f64(reader: &mut impl Read) -> Result<f64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(f64::from_le_bytes(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredImage {
    pub id: u32,
    pub camera_id: u32,
    pub name: String,
}

fn read_u32(reader: &mut impl Read) -> Result<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_i32(reader: &mut impl Read) -> Result<i32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn skip(reader: &mut impl Read, bytes: u64) -> Result<()> {
    let copied = std::io::copy(&mut reader.take(bytes), &mut std::io::sink())?;
    if copied != bytes {
        return Err(SplatError::Process(
            "COLMAP 模型文件提前结束，内容可能已损坏".into(),
        ));
    }
    Ok(())
}

fn camera_model(model_id: i32) -> Option<(&'static str, u64)> {
    Some(match model_id {
        0 => ("SIMPLE_PINHOLE", 3),
        1 => ("PINHOLE", 4),
        2 => ("SIMPLE_RADIAL", 4),
        3 => ("RADIAL", 5),
        4 => ("OPENCV", 8),
        5 => ("OPENCV_FISHEYE", 8),
        6 => ("FULL_OPENCV", 12),
        7 => ("FOV", 5),
        8 => ("SIMPLE_RADIAL_FISHEYE", 4),
        9 => ("RADIAL_FISHEYE", 5),
        10 => ("THIN_PRISM_FISHEYE", 12),
        11 => ("RAD_TAN_THIN_PRISM_FISHEYE", 16),
        _ => return None,
    })
}

pub fn read_single_camera(model: &Path) -> Result<ColmapCamera> {
    let path = model.join("cameras.bin");
    let mut reader = BufReader::new(File::open(&path)?);
    let count = read_u64(&mut reader)?;
    if count != 1 {
        return Err(SplatError::Process(format!(
            "高清补拍要求原项目只使用一台共享相机，当前模型包含 {count} 台相机"
        )));
    }
    let id = read_u32(&mut reader)?;
    let model_id = read_i32(&mut reader)?;
    let width = read_u64(&mut reader)?;
    let height = read_u64(&mut reader)?;
    let (model_name, parameter_count) = camera_model(model_id).ok_or_else(|| {
        SplatError::Process(format!("原项目使用了暂不支持的相机模型编号 {model_id}"))
    })?;
    skip(&mut reader, parameter_count * 8)?;
    Ok(ColmapCamera {
        id,
        model_id,
        model: model_name.into(),
        width,
        height,
    })
}

pub fn read_registered_images(model: &Path) -> Result<Vec<RegisteredImage>> {
    let path = model.join("images.bin");
    let mut reader = BufReader::new(File::open(&path)?);
    let count = read_u64(&mut reader)?;
    let capacity = usize::try_from(count)
        .map_err(|_| SplatError::Process("COLMAP 图像数量超出可读取范围".into()))?;
    let mut images = Vec::with_capacity(capacity);
    for _ in 0..count {
        let id = read_u32(&mut reader)?;
        skip(&mut reader, 7 * 8)?;
        let camera_id = read_u32(&mut reader)?;
        let mut name = Vec::new();
        loop {
            let mut byte = [0_u8; 1];
            reader.read_exact(&mut byte)?;
            if byte[0] == 0 {
                break;
            }
            name.push(byte[0]);
            if name.len() > 32 * 1024 {
                return Err(SplatError::Process("COLMAP 图像名称异常过长".into()));
            }
        }
        let point_count = read_u64(&mut reader)?;
        skip(&mut reader, point_count.saturating_mul(24))?;
        images.push(RegisteredImage {
            id,
            camera_id,
            name: String::from_utf8(name)
                .map_err(|_| SplatError::Process("COLMAP 图像名称不是有效 UTF-8".into()))?,
        });
    }
    Ok(images)
}

/// Reads the single shared camera together with its intrinsics parameters.
///
/// The projection/voting stage needs the actual focal lengths and principal
/// point, not just the model name, so this complements `read_single_camera`.
pub fn read_camera_intrinsics(model: &Path) -> Result<CameraIntrinsics> {
    let path = model.join("cameras.bin");
    let mut reader = BufReader::new(File::open(&path)?);
    let count = read_u64(&mut reader)?;
    if count != 1 {
        return Err(SplatError::Process(format!(
            "高清补拍要求原项目只使用一台共享相机，当前模型包含 {count} 台相机"
        )));
    }
    let id = read_u32(&mut reader)?;
    let model_id = read_i32(&mut reader)?;
    let width = read_u64(&mut reader)?;
    let height = read_u64(&mut reader)?;
    let (model_name, parameter_count) = camera_model(model_id).ok_or_else(|| {
        SplatError::Process(format!("原项目使用了暂不支持的相机模型编号 {model_id}"))
    })?;
    let mut params = Vec::with_capacity(parameter_count as usize);
    for _ in 0..parameter_count {
        params.push(read_f64(&mut reader)?);
    }
    Ok(CameraIntrinsics {
        camera: ColmapCamera {
            id,
            model_id,
            model: model_name.into(),
            width,
            height,
        },
        params,
    })
}

/// Reads every registered image together with its world-to-camera pose.
pub fn read_posed_images(model: &Path) -> Result<Vec<PosedImage>> {
    let path = model.join("images.bin");
    let mut reader = BufReader::new(File::open(&path)?);
    let count = read_u64(&mut reader)?;
    let capacity = usize::try_from(count)
        .map_err(|_| SplatError::Process("COLMAP 图像数量超出可读取范围".into()))?;
    let mut images = Vec::with_capacity(capacity);
    for _ in 0..count {
        let id = read_u32(&mut reader)?;
        let mut quaternion = [0_f64; 4];
        for value in &mut quaternion {
            *value = read_f64(&mut reader)?;
        }
        let mut translation = [0_f64; 3];
        for value in &mut translation {
            *value = read_f64(&mut reader)?;
        }
        let camera_id = read_u32(&mut reader)?;
        let mut name = Vec::new();
        loop {
            let mut byte = [0_u8; 1];
            reader.read_exact(&mut byte)?;
            if byte[0] == 0 {
                break;
            }
            name.push(byte[0]);
            if name.len() > 32 * 1024 {
                return Err(SplatError::Process("COLMAP 图像名称异常过长".into()));
            }
        }
        let point_count = read_u64(&mut reader)?;
        skip(&mut reader, point_count.saturating_mul(24))?;
        images.push(PosedImage {
            id,
            camera_id,
            name: String::from_utf8(name)
                .map_err(|_| SplatError::Process("COLMAP 图像名称不是有效 UTF-8".into()))?,
            pose: ImagePose {
                quaternion,
                translation,
            },
        });
    }
    Ok(images)
}

pub fn required_model_files(model: &Path) -> [PathBuf; 3] {
    [
        model.join("cameras.bin"),
        model.join("images.bin"),
        model.join("points3D.bin"),
    ]
}

/// Returns the files that must be copied to preserve a COLMAP model for reuse.
///
/// COLMAP 3.x models contain the three core files. COLMAP 4.x additionally
/// writes rig and frame metadata; keep those files together when they exist so
/// an incremental mapper sees the same reconstruction that was validated.
pub fn reusable_model_files(model: &Path) -> Result<Vec<PathBuf>> {
    let mut files = required_model_files(model).into_iter().collect::<Vec<_>>();
    for path in &files {
        if !std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
            return Err(SplatError::Process(format!(
                "可复用 COLMAP 模型缺少文件：{}",
                path.display()
            )));
        }
    }

    let rigs = model.join("rigs.bin");
    let frames = model.join("frames.bin");
    let rigs_metadata = std::fs::symlink_metadata(&rigs).ok();
    let frames_metadata = std::fs::symlink_metadata(&frames).ok();
    if rigs_metadata
        .as_ref()
        .is_some_and(|metadata| !metadata.file_type().is_file())
        || frames_metadata
            .as_ref()
            .is_some_and(|metadata| !metadata.file_type().is_file())
    {
        return Err(SplatError::Process(
            "可复用 COLMAP 4.x 模型包含不安全的 rigs.bin 或 frames.bin".into(),
        ));
    }
    match (rigs_metadata.is_some(), frames_metadata.is_some()) {
        (true, true) => files.extend([rigs, frames]),
        (false, false) => {}
        _ => {
            return Err(SplatError::Process(
                "可复用 COLMAP 4.x 模型的 rigs.bin 与 frames.bin 不完整".into(),
            ));
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_one_camera_and_registered_image_names() {
        let root = tempfile::tempdir().unwrap();
        let mut cameras = File::create(root.path().join("cameras.bin")).unwrap();
        cameras.write_all(&1_u64.to_le_bytes()).unwrap();
        cameras.write_all(&7_u32.to_le_bytes()).unwrap();
        cameras.write_all(&2_i32.to_le_bytes()).unwrap();
        cameras.write_all(&1920_u64.to_le_bytes()).unwrap();
        cameras.write_all(&1080_u64.to_le_bytes()).unwrap();
        cameras.write_all(&[0_u8; 32]).unwrap();

        let mut images = File::create(root.path().join("images.bin")).unwrap();
        images.write_all(&1_u64.to_le_bytes()).unwrap();
        images.write_all(&3_u32.to_le_bytes()).unwrap();
        images.write_all(&[0_u8; 56]).unwrap();
        images.write_all(&7_u32.to_le_bytes()).unwrap();
        images.write_all(b"frame_000001.jpg\0").unwrap();
        images.write_all(&0_u64.to_le_bytes()).unwrap();

        let camera = read_single_camera(root.path()).unwrap();
        assert_eq!((camera.id, camera.width, camera.height), (7, 1920, 1080));
        assert_eq!(camera.model, "SIMPLE_RADIAL");
        assert_eq!(
            read_registered_images(root.path()).unwrap()[0].name,
            "frame_000001.jpg"
        );
    }

    #[test]
    fn reusable_model_files_supports_legacy_and_colmap_four_models() {
        let root = tempfile::tempdir().unwrap();
        for name in ["cameras.bin", "images.bin", "points3D.bin"] {
            std::fs::write(root.path().join(name), b"model").unwrap();
        }
        assert_eq!(reusable_model_files(root.path()).unwrap().len(), 3);

        std::fs::write(root.path().join("rigs.bin"), b"rigs").unwrap();
        assert!(reusable_model_files(root.path()).is_err());
        std::fs::write(root.path().join("frames.bin"), b"frames").unwrap();
        assert_eq!(reusable_model_files(root.path()).unwrap().len(), 5);
    }

    fn write_camera_fixture(root: &Path, model_id: i32, params: &[f64]) {
        let mut cameras = File::create(root.join("cameras.bin")).unwrap();
        cameras.write_all(&1_u64.to_le_bytes()).unwrap();
        cameras.write_all(&7_u32.to_le_bytes()).unwrap();
        cameras.write_all(&model_id.to_le_bytes()).unwrap();
        cameras.write_all(&64_u64.to_le_bytes()).unwrap();
        cameras.write_all(&48_u64.to_le_bytes()).unwrap();
        for value in params {
            cameras.write_all(&value.to_le_bytes()).unwrap();
        }
    }

    fn write_image_fixture(root: &Path, quaternion: &[f64], translation: &[f64]) {
        let mut images = File::create(root.join("images.bin")).unwrap();
        images.write_all(&1_u64.to_le_bytes()).unwrap();
        images.write_all(&3_u32.to_le_bytes()).unwrap();
        for value in quaternion {
            images.write_all(&value.to_le_bytes()).unwrap();
        }
        for value in translation {
            images.write_all(&value.to_le_bytes()).unwrap();
        }
        images.write_all(&7_u32.to_le_bytes()).unwrap();
        images.write_all(b"frame_000001.jpg\0").unwrap();
        images.write_all(&0_u64.to_le_bytes()).unwrap();
    }

    #[test]
    fn reads_camera_intrinsics_with_parameters() {
        let root = tempfile::tempdir().unwrap();
        // PINHOLE: fx, fy, cx, cy.
        write_camera_fixture(root.path(), 1, &[800.0, 820.0, 32.0, 24.0]);
        let intrinsics = read_camera_intrinsics(root.path()).unwrap();
        assert_eq!(intrinsics.camera.id, 7);
        assert_eq!(intrinsics.camera.width, 64);
        assert_eq!(intrinsics.camera.height, 48);
        assert_eq!(intrinsics.params, vec![800.0, 820.0, 32.0, 24.0]);
    }

    #[test]
    fn rejects_unknown_camera_model_when_reading_intrinsics() {
        let root = tempfile::tempdir().unwrap();
        write_camera_fixture(root.path(), 99, &[]);
        assert!(read_camera_intrinsics(root.path()).is_err());
    }

    #[test]
    fn reads_posed_images_with_quaternion_and_translation() {
        let root = tempfile::tempdir().unwrap();
        write_camera_fixture(root.path(), 1, &[800.0, 800.0, 32.0, 24.0]);
        write_image_fixture(root.path(), &[1.0, 0.0, 0.0, 0.0], &[0.1, -0.2, 0.3]);
        let images = read_posed_images(root.path()).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].id, 3);
        assert_eq!(images[0].camera_id, 7);
        assert_eq!(images[0].name, "frame_000001.jpg");
        assert_eq!(images[0].pose.quaternion, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(images[0].pose.translation, [0.1, -0.2, 0.3]);
    }
}
