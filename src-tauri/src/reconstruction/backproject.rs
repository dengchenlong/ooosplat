use crate::error::{Result, SplatError};
use crate::reconstruction::colmap_model::{CameraIntrinsics, ImagePose};

/// Pinhole (+ optional distortion) projection for one COLMAP camera.
///
/// Distortion coefficients are honoured for SIMPLE_RADIAL / RADIAL / OPENCV
/// models; purely pinhole models (SIMPLE_PINHOLE / PINHOLE) carry none.
/// FOV / fisheye / full-opencv models are rejected: the shooting pipeline
/// configures SIMPLE_RADIAL or PINHOLE, and back-projection voting tolerates
/// small model mismatches through footprint dilation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraProjection {
    fx: f64,
    fy: f64,
    cx: f64,
    cy: f64,
    k1: f64,
    k2: f64,
    p1: f64,
    p2: f64,
}

impl CameraProjection {
    pub fn from_intrinsics(intrinsics: &CameraIntrinsics) -> Result<Self> {
        let params = intrinsics.params.as_slice();
        let projection = match intrinsics.camera.model_id {
            // SIMPLE_PINHOLE: f, cx, cy.
            0 => Self::undistorted(
                Self::param(params, 0)?,
                Self::param(params, 0)?,
                Self::param(params, 1)?,
                Self::param(params, 2)?,
            ),
            // PINHOLE: fx, fy, cx, cy.
            1 => Self::undistorted(
                Self::param(params, 0)?,
                Self::param(params, 1)?,
                Self::param(params, 2)?,
                Self::param(params, 3)?,
            ),
            // SIMPLE_RADIAL: f, cx, cy, k.
            2 => Self {
                k1: Self::param(params, 3)?,
                ..Self::undistorted(
                    Self::param(params, 0)?,
                    Self::param(params, 0)?,
                    Self::param(params, 1)?,
                    Self::param(params, 2)?,
                )
            },
            // RADIAL: f, cx, cy, k1, k2.
            3 => Self {
                k1: Self::param(params, 3)?,
                k2: Self::param(params, 4)?,
                ..Self::undistorted(
                    Self::param(params, 0)?,
                    Self::param(params, 0)?,
                    Self::param(params, 1)?,
                    Self::param(params, 2)?,
                )
            },
            // OPENCV: fx, fy, cx, cy, k1, k2, p1, p2.
            4 => Self {
                fx: Self::param(params, 0)?,
                fy: Self::param(params, 1)?,
                cx: Self::param(params, 2)?,
                cy: Self::param(params, 3)?,
                k1: Self::param(params, 4)?,
                k2: Self::param(params, 5)?,
                p1: Self::param(params, 6)?,
                p2: Self::param(params, 7)?,
            },
            _ => {
                return Err(SplatError::Process(format!(
                    "反投影投票暂不支持相机模型 {}",
                    intrinsics.camera.model
                )))
            }
        };
        Ok(projection)
    }

    fn undistorted(fx: f64, fy: f64, cx: f64, cy: f64) -> Self {
        Self {
            fx,
            fy,
            cx,
            cy,
            k1: 0.0,
            k2: 0.0,
            p1: 0.0,
            p2: 0.0,
        }
    }

    fn param(params: &[f64], index: usize) -> Result<f64> {
        params
            .get(index)
            .copied()
            .ok_or_else(|| SplatError::Process("COLMAP 相机内参参数数量与模型不匹配".into()))
    }

    /// Projects a world-space point into pixel coordinates.
    ///
    /// Returns `None` when the point lies on or behind the camera plane.
    pub fn world_to_pixel(&self, pose: &ImagePose, point: [f64; 3]) -> Option<[f64; 2]> {
        let cam = world_to_camera(pose, point);
        if cam[2] <= 0.0 {
            return None;
        }
        let xu = cam[0] / cam[2];
        let yu = cam[1] / cam[2];
        let r2 = xu * xu + yu * yu;
        let radial = 1.0 + self.k1 * r2 + self.k2 * r2 * r2;
        let xd = xu * radial + 2.0 * self.p1 * xu * yu + self.p2 * (r2 + 2.0 * xu * xu);
        let yd = yu * radial + self.p1 * (r2 + 2.0 * yu * yu) + 2.0 * self.p2 * xu * yu;
        Some([self.fx * xd + self.cx, self.fy * yd + self.cy])
    }

    /// Camera-space depth of a world point (distance along the optical axis).
    pub fn depth(&self, pose: &ImagePose, point: [f64; 3]) -> f64 {
        world_to_camera(pose, point)[2]
    }

    /// Approximate splat footprint radius in pixels at the given camera-space
    /// depth. 3DGS PLY stores scales as logarithms, so `exp` recovers the
    /// world-space standard deviation; the largest axis dominates the disk.
    pub fn footprint_radius_px(&self, scale_log: [f64; 3], depth: f64) -> f64 {
        let sigma = scale_log
            .iter()
            .map(|value| value.exp())
            .fold(0.0_f64, f64::max);
        sigma * self.fx / depth
    }
}

/// Applies the COLMAP world-to-camera transform: rotate by the pose quaternion
/// (w, x, y, z) and translate by `tvec`.
pub fn world_to_camera(pose: &ImagePose, point: [f64; 3]) -> [f64; 3] {
    let [qw, qx, qy, qz] = pose.quaternion;
    let [x, y, z] = point;
    // Rotation q * v * q^-1 via the cross-product form.
    let tx = 2.0 * (qy * z - qz * y);
    let ty = 2.0 * (qz * x - qx * z);
    let tz = 2.0 * (qx * y - qy * x);
    [
        x + qw * tx + (qy * tz - qz * ty) + pose.translation[0],
        y + qw * ty + (qz * tx - qx * tz) + pose.translation[1],
        z + qw * tz + (qx * ty - qy * tx) + pose.translation[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruction::colmap_model::ColmapCamera;

    fn intrinsics(model_id: i32, params: &[f64]) -> CameraIntrinsics {
        CameraIntrinsics {
            camera: ColmapCamera {
                id: 1,
                model_id,
                model: "test".into(),
                width: 64,
                height: 48,
            },
            params: params.to_vec(),
        }
    }

    const IDENTITY: ImagePose = ImagePose {
        quaternion: [1.0, 0.0, 0.0, 0.0],
        translation: [0.0, 0.0, 0.0],
    };

    fn pinhole_projection() -> CameraProjection {
        CameraProjection::from_intrinsics(&intrinsics(1, &[800.0, 800.0, 32.0, 24.0])).unwrap()
    }

    #[test]
    fn projects_optical_axis_center_to_principal_point() {
        let projection =
            CameraProjection::from_intrinsics(&intrinsics(1, &[800.0, 800.0, 32.0, 24.0])).unwrap();
        assert_eq!(
            projection.world_to_pixel(&IDENTITY, [0.0, 0.0, 5.0]),
            Some([32.0, 24.0])
        );
    }

    #[test]
    fn projects_offset_point_with_focal_scaling() {
        let projection =
            CameraProjection::from_intrinsics(&intrinsics(1, &[800.0, 800.0, 32.0, 24.0])).unwrap();
        // x: 0.1/5 * 800 = 16 px right; y: -0.05/5 * 800 = -8 px up.
        assert_eq!(
            projection.world_to_pixel(&IDENTITY, [0.1, -0.05, 5.0]),
            Some([48.0, 16.0])
        );
    }

    #[test]
    fn applies_pose_translation() {
        let pose = ImagePose {
            quaternion: [1.0, 0.0, 0.0, 0.0],
            translation: [0.1, 0.0, 0.0],
        };
        assert_eq!(
            pinhole_projection().world_to_pixel(&pose, [0.0, 0.0, 5.0]),
            Some([48.0, 24.0])
        );
    }

    #[test]
    fn applies_pose_rotation() {
        // 90-degree rotation about the y axis (right-hand rule): world +z maps
        // to camera +x and world -x maps to camera +z.
        let quarter_pi = std::f64::consts::FRAC_PI_4;
        let turned = ImagePose {
            quaternion: [quarter_pi.cos(), 0.0, quarter_pi.sin(), 0.0],
            translation: [0.0, 0.0, 0.0],
        };
        // World +z lands on the camera plane (depth 0): invisible.
        assert_eq!(
            pinhole_projection().world_to_pixel(&turned, [0.0, 0.0, 5.0]),
            None
        );
        // World -x maps onto the optical axis at depth 5 (float noise aside).
        let pixel = pinhole_projection()
            .world_to_pixel(&turned, [-5.0, 0.0, 0.0])
            .unwrap();
        assert!((pixel[0] - 32.0).abs() < 1e-9);
        assert!((pixel[1] - 24.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_points_on_or_behind_camera_plane() {
        let projection = pinhole_projection();
        assert_eq!(projection.world_to_pixel(&IDENTITY, [0.0, 0.0, 0.0]), None);
        assert_eq!(projection.world_to_pixel(&IDENTITY, [0.0, 0.0, -1.0]), None);
    }

    #[test]
    fn applies_radial_distortion_for_simple_radial_model() {
        // SIMPLE_RADIAL params: f, cx, cy, k.
        let projection =
            CameraProjection::from_intrinsics(&intrinsics(2, &[800.0, 32.0, 24.0, -0.5])).unwrap();
        // Undistorted: xu = 0.1/5 = 0.02, r2 = 0.0004.
        // xd = 0.02 * (1 - 0.5 * 0.0004) = 0.019996.
        let expected_x = 800.0 * 0.019996 + 32.0;
        assert_eq!(
            projection.world_to_pixel(&IDENTITY, [0.1, 0.0, 5.0]),
            Some([expected_x, 24.0])
        );
    }

    #[test]
    fn rejects_unsupported_camera_models() {
        assert!(CameraProjection::from_intrinsics(&intrinsics(7, &[1.0; 5])).is_err());
        assert!(CameraProjection::from_intrinsics(&intrinsics(6, &[1.0; 12])).is_err());
    }

    #[test]
    fn footprint_radius_uses_exp_scale_and_focal_over_depth() {
        let projection = pinhole_projection();
        // Largest axis: exp(ln(0.02)) = 0.02 -> 0.02 * 800 / 5 = 3.2 px.
        assert_eq!(
            projection.footprint_radius_px([0.0_f64.ln(), 0.02_f64.ln(), 0.01_f64.ln()], 5.0),
            3.2
        );
        // depth() reports the camera-space z of the same point.
        assert_eq!(projection.depth(&IDENTITY, [0.1, 0.0, 5.0]), 5.0);
    }
}
