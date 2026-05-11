//! 扫描进度共享模块

use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::collections::HashMap;

/// 扫描进度共享状态
pub struct ScanProgressShared {
    pub current_file: Mutex<String>,
    pub files_total: AtomicUsize,
    pub files_processed: AtomicUsize,
    pub scan_done: AtomicBool,
}

static SCAN_PROGRESS: OnceLock<ScanProgressShared> = OnceLock::new();

pub fn get_scan_progress() -> &'static ScanProgressShared {
    SCAN_PROGRESS.get_or_init(|| ScanProgressShared {
        current_file: Mutex::new(String::new()),
        files_total: AtomicUsize::new(0),
        files_processed: AtomicUsize::new(0),
        scan_done: AtomicBool::new(false),
    })
}

// ============================================================
// 配对级进度跟踪（用于表格中每个配对显示独立进度）
// ============================================================

/// 单个配对的进度信息
pub struct PairProgressInfo {
    pub frame: AtomicU32,
    pub expected_frames: AtomicU32,
    pub status: Mutex<ProcessingStatus>, // Pending, Running, Completed, Failed
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessingStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// 全局配对进度映射 (batch_idx -> PairProgressInfo)
static PAIR_PROGRESS_MAP: OnceLock<Mutex<HashMap<u32, PairProgressInfo>>> = OnceLock::new();

/// 获取配对进度映射
pub fn get_pair_progress_map() -> &'static Mutex<HashMap<u32, PairProgressInfo>> {
    PAIR_PROGRESS_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 注册一个新配对的进度跟踪
pub fn register_pair_progress(batch_idx: u32, expected_frames: u32) {
    let map = get_pair_progress_map();
    let mut guard = map.lock().unwrap();
    guard.insert(batch_idx, PairProgressInfo {
        frame: AtomicU32::new(0),
        expected_frames: AtomicU32::new(expected_frames),
        status: Mutex::new(ProcessingStatus::Pending),
    });
}

/// 更新配对的帧进度
pub fn update_pair_frame(batch_idx: u32, frame: u32) {
    let map = get_pair_progress_map();
    if let Some(progress) = map.lock().unwrap().get(&batch_idx) {
        progress.frame.store(frame, Ordering::Relaxed);
    }
}

/// 更新配对的状态
pub fn update_pair_status(batch_idx: u32, status: ProcessingStatus) {
    let map = get_pair_progress_map();
    if let Some(progress) = map.lock().unwrap().get(&batch_idx) {
        *progress.status.lock().unwrap() = status;
    }
}

/// 获取所有配对的进度信息（用于 GUI 轮询）
pub fn get_all_pair_progress() -> Vec<(u32, u32, u32, ProcessingStatus)> {
    let map = get_pair_progress_map();
    let guard = map.lock().unwrap();
    guard.iter()
        .map(|(idx, p)| {
            (*idx, p.frame.load(Ordering::Relaxed), p.expected_frames.load(Ordering::Relaxed), *p.status.lock().unwrap())
        })
        .collect()
}

/// 清空配对进度映射
pub fn clear_pair_progress_map() {
    if let Some(map) = PAIR_PROGRESS_MAP.get() {
        map.lock().unwrap().clear();
    }
}

// ============================================================
// 扫描进度函数
// ============================================================

/// 设置扫描进度 - 适用于顺序执行
pub fn set_scan_progress(file: &str, processed: usize, total: usize) {
    let progress = get_scan_progress();
    *progress.current_file.lock().unwrap() = file.to_string();
    progress.files_processed.store(processed, Ordering::Relaxed);
    progress.files_total.store(total, Ordering::Relaxed);
}

/// 设置总文件数 - 只需设置一次
pub fn set_total_files(total: usize) {
    let progress = get_scan_progress();
    progress.files_total.store(total, Ordering::Relaxed);
}

/// 原子递增已处理文件数 - 适用于并行执行
/// 返回递增后的值
pub fn increment_processed() -> usize {
    let progress = get_scan_progress();
    progress.files_processed.fetch_add(1, Ordering::Relaxed) + 1
}

/// 设置当前正在处理的文件名
pub fn set_current_file(file: &str) {
    let progress = get_scan_progress();
    *progress.current_file.lock().unwrap() = file.to_string();
}

/// 重置扫描进度
pub fn reset_scan_progress() {
    let progress = get_scan_progress();
    *progress.current_file.lock().unwrap() = String::new();
    progress.files_processed.store(0, Ordering::Relaxed);
    progress.files_total.store(0, Ordering::Relaxed);
    progress.scan_done.store(false, Ordering::Relaxed);
}

/// 标记扫描完成
pub fn mark_scan_done() {
    get_scan_progress().scan_done.store(true, Ordering::SeqCst);
}