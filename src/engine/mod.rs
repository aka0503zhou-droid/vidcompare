//! 视频处理引擎模块

mod scanner;
mod matcher;
mod decoder;
mod metrics;
mod pipeline;
mod ffmpeg_manager;
mod ffmpeg_filter;
mod scan_progress;

pub use matcher::Matcher;
pub use decoder::{VideoDecoder, DecoderConfig, detect_gpu};
pub use pipeline::Pipeline;
pub use ffmpeg_manager::FfmpegManager;
pub use ffmpeg_filter::{set_pair_index, set_expected_frames, set_cache_idx};
pub use scanner::{fast_scan_directory, probe_videos_in_pairs};
pub use scan_progress::{
    get_scan_progress, set_scan_progress, reset_scan_progress, mark_scan_done,
    set_total_files, increment_processed, set_current_file,
    get_all_pair_progress, register_pair_progress, update_pair_frame, update_pair_status,
    clear_pair_progress_map, ProcessingStatus as PairProcessingStatus,
};

use std::sync::{Mutex, OnceLock};
use std::sync::atomic::AtomicBool;

/// 扫描结果共享状态
pub struct ScanResultShared {
    pub pairs: Mutex<Vec<FilePair>>,
    pub scan_done: AtomicBool,
}

static SCAN_RESULT: OnceLock<ScanResultShared> = OnceLock::new();

pub fn get_scan_result() -> &'static ScanResultShared {
    SCAN_RESULT.get_or_init(|| ScanResultShared {
        pairs: Mutex::new(Vec::new()),
        scan_done: AtomicBool::new(false),
    })
}

/// 内存中缓冲的最大帧数
pub const MAX_FRAME_BUFFER: usize = 500;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 视频文件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFile {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub bitrate: Option<u64>,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub codec: Option<String>,
    /// 总帧数 (从 ffprobe 的 nb_frames 获取)
    pub frame_count: Option<u32>,
}

impl VideoFile {
    pub fn new(name: String, path: PathBuf, size: u64) -> Self {
        Self {
            name,
            path,
            size,
            bitrate: None,
            duration_ms: None,
            width: None,
            height: None,
            codec: None,
            frame_count: None,
        }
    }
}

/// 文件对比对
#[derive(Debug, Clone)]
pub struct FilePair {
    pub index: u32,
    pub reference: Option<VideoFile>,
    pub distorted: Option<VideoFile>,
    pub selected: bool,
    pub ref_file: VideoFile,
    pub dist_file: VideoFile,
}

/// 比较记录（最终结果）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonRecord {
    pub id: Option<i64>,
    pub index: u32,
    pub ref_filename: String,
    pub dist_filename: Option<String>,
    pub ref_path: String,
    pub dist_path: Option<String>,
    pub ref_filesize: u64,
    pub dist_filesize: Option<u64>,
    pub ref_bitrate: u64,
    pub dist_bitrate: Option<u64>,
    pub ref_width: Option<u32>,
    pub ref_height: Option<u32>,
    pub psnr: Option<f32>,
    pub ssim: Option<f32>,
    pub vmaf: Option<f32>,
    pub avg_fps: Option<f32>,
    pub processing_time_ms: Option<u64>,
    pub compression_ratio: Option<f32>,
    pub frame_count: Option<u32>,
    pub status: ProcessingStatus,
    pub error_message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    /// 每帧 PSNR 数据 (用于图表)
    pub psnr_per_frame: Vec<f32>,
    /// 每帧 SSIM 数据 (用于图表)
    pub ssim_per_frame: Vec<f32>,
    /// 每帧 VMAF 数据 (用于图表)
    pub vmaf_per_frame: Vec<f32>,
}

impl ComparisonRecord {
    pub fn new(ref_file: &VideoFile, dist_file: &VideoFile) -> Self {
        Self {
            id: None,
            index: 0,
            ref_filename: ref_file.name.clone(),
            dist_filename: Some(dist_file.name.clone()),
            ref_path: ref_file.path.to_string_lossy().to_string(),
            dist_path: Some(dist_file.path.to_string_lossy().to_string()),
            ref_filesize: ref_file.size,
            dist_filesize: Some(dist_file.size),
            ref_bitrate: ref_file.bitrate.unwrap_or(0),
            dist_bitrate: dist_file.bitrate,
            ref_width: ref_file.width,
            ref_height: ref_file.height,
            psnr: None,
            ssim: None,
            vmaf: None,
            avg_fps: None,
            processing_time_ms: None,
            compression_ratio: None,
            frame_count: None,
            status: ProcessingStatus::Pending,
            error_message: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            psnr_per_frame: Vec::new(),
            ssim_per_frame: Vec::new(),
            vmaf_per_frame: Vec::new(),
        }
    }

    pub fn from_pair(pair: &FilePair) -> Self {
        // 使用 ref_file/dist_file（这些是扫描后被 probe_videos_in_pairs 更新过的）
        // 使用 pair.index 作为序号（配对顺序 1, 2, 3...）
        let mut record = Self::new(&pair.ref_file, &pair.dist_file);
        record.index = pair.index;
        record
    }
}

/// 处理状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProcessingStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl std::fmt::Display for ProcessingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessingStatus::Pending => write!(f, "Pending"),
            ProcessingStatus::Running => write!(f, "Running"),
            ProcessingStatus::Completed => write!(f, "Completed"),
            ProcessingStatus::Failed => write!(f, "Failed"),
            ProcessingStatus::Skipped => write!(f, "Skipped"),
        }
    }
}

impl From<&str> for ProcessingStatus {
    fn from(s: &str) -> Self {
        match s {
            "Pending" => ProcessingStatus::Pending,
            "Running" => ProcessingStatus::Running,
            "Completed" => ProcessingStatus::Completed,
            "Failed" => ProcessingStatus::Failed,
            "Skipped" => ProcessingStatus::Skipped,
            _ => ProcessingStatus::Pending,
        }
    }
}

/// 处理进度回调函数类型
pub type ProgressCallback = dyn Fn(usize, usize, &str, f64, Option<f64>) + Send + Sync;
