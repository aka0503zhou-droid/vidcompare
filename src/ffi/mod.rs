//! FFmpeg FFI 模块
//! 
//! 用于检测和管理 FFmpeg 二进制文件

use std::path::PathBuf;
use std::process::Command;
use tracing::{info, warn, error};

/// FFmpeg 信息
#[derive(Debug, Clone)]
pub struct FFmpegInfo {
    pub version: String,
    pub path: PathBuf,
    pub has_libvmaf: bool,
    pub has_cuda: bool,
}

/// 检测系统 FFmpeg
pub fn detect_ffmpeg() -> Option<FFmpegInfo> {
    // 检查 ffmpeg 命令
    let output = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .ok()?;
    
    if !output.status.success() {
        return None;
    }
    
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .to_string();
    
    // 检查 libvmaf 支持
    let has_libvmaf = check_libvmaf();
    
    // 检查 CUDA 支持
    let has_cuda = check_cuda();
    
    Some(FFmpegInfo {
        version,
        path: PathBuf::from("ffmpeg"),
        has_libvmaf,
        has_cuda,
    })
}

fn check_libvmaf() -> bool {
    let output = Command::new("ffmpeg")
        .arg("-filters")
        .output();
    
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("libvmaf"),
        Err(_) => false,
    }
}

fn check_cuda() -> bool {
    let output = Command::new("ffmpeg")
        .arg("-encoders")
        .output();
    
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("cuda"),
        Err(_) => false,
    }
}

/// 检查 FFmpeg 是否可用
pub fn is_ffmpeg_available() -> bool {
    detect_ffmpeg().is_some()
}

/// 获取 VMAF 模型路径
pub fn get_vmaf_model_path() -> Option<PathBuf> {
    // 检查常见位置
    let candidates = [
        PathBuf::from("models/vmaf_v0.6.1.json"),
        PathBuf::from("/usr/local/share/model/vmaf_v0.6.1.json"),
        PathBuf::from("/usr/share/model/vmaf_v0.6.1.json"),
    ];
    
    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }
    
    None
}
