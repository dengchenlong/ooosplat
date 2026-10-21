use std::{ffi::OsString, path::Path};

use crate::{
    error::{Result, SplatError},
    process::{ProcessManager, ProcessSpec},
    video::{parse_ffprobe_json, VideoInfo},
};

pub async fn probe_video(
    executable: &Path,
    input: &Path,
    log_path: Option<std::path::PathBuf>,
    process_manager: &ProcessManager,
) -> Result<VideoInfo> {
    Ok(
        probe_video_metadata(executable, input, log_path, process_manager)
            .await?
            .0,
    )
}

pub async fn probe_video_metadata(
    executable: &Path,
    input: &Path,
    log_path: Option<std::path::PathBuf>,
    process_manager: &ProcessManager,
) -> Result<(VideoInfo, serde_json::Value)> {
    if !input.is_file() {
        return Err(SplatError::InvalidPath(input.to_path_buf()));
    }
    let output = process_manager
        .clone()
        .with_output_capture_limit(64 * 1024)
        .run(ProcessSpec {
            executable: executable.to_path_buf(),
            args: vec![
                OsString::from("-v"),
                OsString::from("error"),
                OsString::from("-select_streams"),
                OsString::from("v:0"),
                OsString::from("-show_streams"),
                OsString::from("-show_format"),
                OsString::from("-of"),
                OsString::from("json"),
                input.as_os_str().to_owned(),
            ],
            working_directory: input.parent().map(Path::to_path_buf),
            log_path,
            observer: None,
        })
        .await?;
    if !output.success {
        return Err(SplatError::InvalidVideo("FFprobe 无法解码这个文件".into()));
    }
    Ok((
        parse_ffprobe_json(&output.stdout)?,
        serde_json::from_str(&output.stdout)?,
    ))
}
