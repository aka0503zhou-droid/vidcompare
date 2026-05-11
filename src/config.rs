//! 配置模块
//!
//! 管理应用程序的配置选项，包括后缀规则、计算参数等

use std::path::PathBuf;

/// 压缩文件后缀列表 (按优先级排序)
/// 
/// 系统会尝试用这些后缀去匹配原文件和压缩文件的关系
/// 例如: video.mp4 和 video_hc.mp4 会被认为是配对文件
pub static COMPRESSION_SUFFIXES: &[&str] = &[
    // 中文后缀 (优先级最高)
    "_高压缩",
    "_低码率",
    "_转码",
    "_压缩",
    "_低质量",
    "_编码",
    
    // 英文后缀
    "_hc",           // High Compression
    "_crf",          // CRF encoding
    "_enc",          // Encoded
    "_out",          // Output
    "_trans",        // Transcoded
    "_264",          // H.264
    "_265",          // H.265 / HEVC
    "_hevc",         // HEVC
    "_av1",          // AV1
    "_vp9",          // VP9
    "_lossy",        // Lossy compression
    "_compressed",   // Compressed
    "_small",        // Smaller size
    "_low",          // Low bitrate
    "_high",         // High compression (same as hc)
];

/// 支持的视频文件扩展名
pub static VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "wmv", "flv", "m4v", "mpg", "mpeg"
];

/// 默认计算参数
#[derive(Debug, Clone)]
pub struct ComputeConfig {
    /// 是否启用 GPU 加速
    pub use_gpu: bool,
    /// GPU 设备 ID
    pub gpu_device: u32,
    /// 是否计算 VMAF
    pub compute_vmaf: bool,
    /// 是否计算 SSIM
    pub compute_ssim: bool,
    /// 是否计算 PSNR
    pub compute_psnr: bool,
    /// 是否计算 MS-SSIM
    pub compute_msssim: bool,
    /// 最大处理帧数 (0 = 不限制)
    pub max_frames: u32,
    /// VMAF 采样帧数 (0 = 所有帧)
    pub vmaf_frames: u32,
    /// 并行处理线程数 (0 = 自动)
    pub threads: u32,
    /// 是否使用 libvmaf (需要系统安装)
    pub use_libvmaf: bool,
    /// VMAF 模型路径
    pub vmaf_model_path: Option<String>,
    /// FFmpeg 可执行文件路径 (None = 使用系统 ffmpeg)
    pub ffmpeg_path: Option<PathBuf>,
}

impl Default for ComputeConfig {
    fn default() -> Self {
        Self {
            use_gpu: false,
            gpu_device: 0,
            compute_vmaf: false,
            compute_ssim: false,
            compute_psnr: false,
            compute_msssim: false,
            max_frames: 500,
            vmaf_frames: 100,
            threads: 0,
            use_libvmaf: false,
            vmaf_model_path: None,
            ffmpeg_path: None,
        }
    }
}

impl ComputeConfig {
    /// 从 UI 选项创建配置
    pub fn from_ui(
        use_gpu: bool,
        compute_vmaf: bool,
        compute_ssim: bool,
        compute_psnr: bool,
    ) -> Self {
        let mut config = Self::default();
        config.use_gpu = use_gpu;
        config.compute_vmaf = compute_vmaf;
        config.compute_ssim = compute_ssim;
        config.compute_psnr = compute_psnr;
        config
    }
}
