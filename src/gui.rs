//! VidCompare GUI - 三段式垂直布局
//! 上: 目录选择 + GPU开关
//! 中: 扫描 + 文件列表 + 设置 + 开始
//! 下: 过滤 + 排序表格 + 导出 + 详情

#![allow(non_snake_case)]
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::error;

use crate::config::ComputeConfig;
use crate::db::Database;
use crate::engine::{self, ComparisonRecord, FilePair, Matcher, Pipeline, ProcessingStatus};
use crate::export::{export_records, ExportFormat, ExportOptions};

// ============================================================
// 常量
// ============================================================

const C_BG: egui::Color32 = egui::Color32::from_rgb(15, 15, 25);
const C_SURFACE: egui::Color32 = egui::Color32::from_rgb(28, 28, 48);
const C_SURFACE2: egui::Color32 = egui::Color32::from_rgb(38, 38, 62);
const C_SURFACE_ALT: egui::Color32 = egui::Color32::from_rgb(33, 33, 55); // zebra striping
const C_BORDER: egui::Color32 = egui::Color32::from_rgb(50, 50, 80);
const C_PRIMARY: egui::Color32 = egui::Color32::from_rgb(97, 165, 255);
const C_SECONDARY: egui::Color32 = egui::Color32::from_rgb(139, 92, 246);
const C_SUCCESS: egui::Color32 = egui::Color32::from_rgb(34, 197, 94);
const C_WARNING: egui::Color32 = egui::Color32::from_rgb(234, 179, 8);
const C_ERROR: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);
const C_TEXT: egui::Color32 = egui::Color32::from_rgb(220, 220, 235);
const C_MUTED: egui::Color32 = egui::Color32::from_rgb(110, 120, 150);
const C_HEADER: egui::Color32 = egui::Color32::from_rgb(45, 45, 75);

// ============================================================
// 辅助
// ============================================================

/// 简洁的动态进度条
struct SmoothProgressBar {
    progress: f32,
    time: f32,
    color: egui::Color32,
    desired_width: f32,
}

impl SmoothProgressBar {
    fn new(progress: f32, time: f32) -> Self {
        Self {
            progress,
            time,
            color: C_PRIMARY,
            desired_width: 200.0,
        }
    }

    fn fill(mut self, color: egui::Color32) -> Self {
        self.color = color;
        self
    }

    fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = width;
        self
    }
}

impl egui::Widget for SmoothProgressBar {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let height = 12.0_f32;
        let (rect, response) =
            ui.allocate_at_least(egui::vec2(self.desired_width, height), egui::Sense::hover());

        let painter = ui.painter();

        // 背景
        let bg_color = egui::Color32::from_rgb(50, 50, 70);
        painter.rect_filled(rect, height / 2.0, bg_color);

        if self.progress > 0.0 {
            let fill_width = rect.width() * self.progress.min(1.0).max(0.0);
            let fill_rect = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, rect.min.y),
                egui::pos2(rect.min.x + fill_width, rect.max.y),
            );

            // 渐变色进度条
            let gradient_top = egui::Color32::from_rgb(
                (self.color.r() + 40).min(255),
                (self.color.g() + 40).min(255),
                (self.color.b() + 40).min(255),
            );
            painter.rect_filled(fill_rect, height / 2.0, gradient_top);

            // 简洁的高光线
            let highlight_rect = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, rect.min.y),
                egui::pos2(rect.min.x + fill_width, rect.min.y + 2.0),
            );
            painter.rect_filled(
                highlight_rect,
                1.0,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60),
            );
        }

        response
    }
}

fn fmt_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2}GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1}MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.0}KB", bytes as f64 / 1_000.0)
    } else {
        format!("{}B", bytes)
    }
}

fn fmt_bitrate(bps: u64) -> String {
    if bps == 0 {
        "—".to_string()
    } else if bps >= 1_000_000 {
        format!("{:.2}Mbps", bps as f64 / 1_000_000.0)
    } else {
        format!("{:.0}kbps", bps as f64 / 1_000.0)
    }
}

fn fmt_duration(ms: u64) -> String {
    let secs = ms as f64 / 1000.0;
    if secs >= 3600.0 {
        format!("{:.1}h", secs / 3600.0)
    } else if secs >= 60.0 {
        format!("{:.1}m", secs / 60.0)
    } else {
        format!("{:.1}s", secs)
    }
}

// ============================================================
// 进度共享
// ============================================================

pub struct ProgressShared {
    pub phase: AtomicUsize,
    pub processed: AtomicU32,
    pub total: AtomicU32,
    pub fps: Mutex<f32>,
    pub current_file: Mutex<String>,
    pub error_msg: Mutex<Option<String>>,
    pub running: AtomicBool,
    pub done: AtomicBool,
    pub results: Mutex<Vec<ComparisonRecord>>,
    pub eta_seconds: Mutex<f32>,
    pub started_at: Mutex<u64>,
    pub frame_count: AtomicU32,
    pub pair_current: AtomicU32,
    pub pair_total: AtomicU32,
    pub pair_eta_ms: AtomicU64,
    pub result_sender: Mutex<Option<mpsc::Sender<ComparisonRecord>>>,
}

impl Default for ProgressShared {
    fn default() -> Self {
        Self {
            phase: AtomicUsize::new(0),
            processed: AtomicU32::new(0),
            total: AtomicU32::new(0),
            fps: Mutex::new(0.0),
            current_file: Mutex::new(String::new()),
            error_msg: Mutex::new(None),
            running: AtomicBool::new(false),
            done: AtomicBool::new(false),
            results: Mutex::new(Vec::new()),
            eta_seconds: Mutex::new(0.0),
            started_at: Mutex::new(0u64),
            frame_count: AtomicU32::new(0),
            pair_current: AtomicU32::new(0),
            pair_total: AtomicU32::new(0),
            pair_eta_ms: AtomicU64::new(0),
            result_sender: Mutex::new(None),
        }
    }
}

// ============================================================
// 排序
// ============================================================

#[derive(Clone, Copy, PartialEq)]
enum SortCol {
    Index,
    RefName,
    RefSize,
    DistName,
    DistSize,
    RefBitrate,
    DistBitrate,
    Ratio,
    Psnr,
    Ssim,
    Vmaf,
    Time,
    Status,
}

impl SortCol {
    fn label(&self) -> &str {
        match self {
            Self::Index => "序号",
            Self::RefName => "原文件",
            Self::RefSize => "原大小",
            Self::DistName => "压缩文件",
            Self::DistSize => "压后大小",
            Self::RefBitrate => "原码率",
            Self::DistBitrate => "压后码率",
            Self::Ratio => "压缩比",
            Self::Psnr => "PSNR",
            Self::Ssim => "SSIM",
            Self::Vmaf => "VMAF",
            Self::Time => "耗时",
            Self::Status => "状态",
        }
    }
}

// ============================================================
// 主应用
// ============================================================

pub struct VidCompareApp {
    // 目录
    ref_dir: String,
    dist_dir: String,
    pairs: Vec<FilePair>,

    // GPU
    gpu_available: bool,
    gpu_name: String,
    gpu_enabled: bool,

    // 计算设置
    compute_ssim: bool,
    compute_vmaf: bool,
    compute_psnr: bool,
    max_frames: u32,

    // 处理状态
    processing: bool,
    scanning: bool,
    progress: f32,
    max_progress: f32, // 用于确保进度只增不减
    current_file: String,
    eta_display: f32,
    scan_progress: f32,
    scan_current_file: String,
    error_message: Option<String>,
    ffmpeg_ok: bool,
    ffmpeg_version: String,
    progress_shared: Arc<ProgressShared>,
    db: Arc<Mutex<Option<Database>>>,

    // 结果
    results: Vec<ComparisonRecord>,
    history: Vec<ComparisonRecord>,

    // 过滤 & 排序 & 分页
    filter_text: String,
    sort_col: SortCol,
    sort_asc: bool,
    page: usize,
    page_size: usize,

    // 选中详情
    selected_record: Option<ComparisonRecord>,
    // 批量删除选择（存储 results 中的索引）
    selected_for_delete: HashSet<usize>,
    history_loaded: bool,
    // 动画时间
    pub animation_time: f32,
    // 配对进度缓存 (batch_idx -> (frame, expected, status)) - 用于表格显示
    pair_progress_cache: std::collections::HashMap<u32, (u32, u32, engine::PairProcessingStatus)>,
    last_progress_update: std::time::Instant,
}

impl VidCompareApp {
    fn new(gpu_available: bool, gpu_name: String) -> Self {
        let db = Arc::new(Mutex::new(Database::new().ok()));

        // 启动时清空数据库和内存中的历史记录
        if let Ok(db_guard) = db.lock() {
            if let Some(ref db) = *db_guard {
                if let Err(e) = db.truncate() {
                    tracing::error!("清空数据库失败: {}", e);
                }
            }
        }

        let ffmpeg_ok = engine::VideoDecoder::is_ffmpeg_available();
        let ffmpeg_version = if ffmpeg_ok {
            engine::VideoDecoder::get_ffmpeg_version()
                .map(|v| v.lines().next().unwrap_or("").to_string())
                .unwrap_or_default()
        } else {
            "未安装".to_string()
        };

        Self {
            ref_dir: String::new(),
            dist_dir: String::new(),
            pairs: Vec::new(),
            gpu_available,
            gpu_name,
            gpu_enabled: gpu_available,
            compute_ssim: false,
            compute_vmaf: false,
            compute_psnr: true,
            max_frames: 500,
            processing: false,
            scanning: false,
            progress: 0.0,
            max_progress: 0.0,
            current_file: String::new(),
            eta_display: 0.0,
            scan_progress: 0.0,
            scan_current_file: String::new(),
            error_message: None,
            ffmpeg_ok,
            ffmpeg_version,
            progress_shared: Arc::new(ProgressShared::default()),
            db,
            results: Vec::new(),
            history: Vec::new(),
            filter_text: String::new(),
            sort_col: SortCol::Index,
            sort_asc: true,
            page: 1,
            page_size: 10,
            selected_record: None,
            selected_for_delete: HashSet::new(),
            history_loaded: false,
            animation_time: 0.0,
            pair_progress_cache: std::collections::HashMap::new(),
            last_progress_update: std::time::Instant::now(),
        }
    }

    fn scan_dirs(&mut self) {
        self.error_message = None;
        if self.ref_dir.is_empty() || self.dist_dir.is_empty() {
            self.error_message = Some("请先选择源目录和目标目录".to_string());
            return;
        }
        let ref_path = PathBuf::from(&self.ref_dir);
        let dist_path = PathBuf::from(&self.dist_dir);
        if !ref_path.exists() {
            self.error_message = Some(format!("源目录不存在: {}", self.ref_dir));
            return;
        }
        if !dist_path.exists() {
            self.error_message = Some(format!("目标目录不存在: {}", self.dist_dir));
            return;
        }

        // 如果正在扫描，直接返回
        if self.scanning {
            return;
        }

        self.scanning = true;
        self.error_message = None;

        let ref_dir = self.ref_dir.clone();
        let dist_dir = self.dist_dir.clone();

        std::thread::spawn(move || {
            let ref_path = PathBuf::from(&ref_dir);
            let dist_path = PathBuf::from(&dist_dir);

            // 快速扫描（只获取文件名和大小，不探测视频信息）
            let ref_files = match engine::fast_scan_directory(&ref_path) {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("扫描源目录失败: {}", e);
                    return;
                }
            };

            let dist_files = match engine::fast_scan_directory(&dist_path) {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("扫描目标目录失败: {}", e);
                    return;
                }
            };

            tracing::info!(
                "快速扫描完成: 源 {} 个, 目标 {} 个",
                ref_files.len(),
                dist_files.len()
            );

            // 匹配文件
            let matcher = Matcher::new();
            let pairs = matcher.match_files(&ref_files, &dist_files);
            tracing::info!("匹配 {} 对", pairs.len());

            // 通过共享状态传递结果
            let shared = crate::engine::get_scan_result();
            *shared.pairs.lock().unwrap() = pairs;
            shared.scan_done.store(true, Ordering::SeqCst);
        });
    }

    fn poll_scan_result(&mut self) {
        // 轮询扫描进度 (实时文件级进度)
        let scan_prog = crate::engine::get_scan_progress();
        if !scan_prog.scan_done.load(Ordering::SeqCst) {
            let processed = scan_prog.files_processed.load(Ordering::Relaxed);
            let total = scan_prog.files_total.load(Ordering::Relaxed);
            if total > 0 {
                self.scan_progress = processed as f32 / total as f32;
                self.scan_current_file = scan_prog.current_file.lock().unwrap().clone();
            }
        }

        let shared = crate::engine::get_scan_result();
        if shared.scan_done.load(Ordering::SeqCst) {
            self.scanning = false;
            self.scan_progress = 0.0;
            self.scan_current_file.clear();
            self.pairs = shared.pairs.lock().unwrap().clone();
            shared.scan_done.store(false, Ordering::SeqCst);
            if self.pairs.is_empty() {
                self.error_message = Some("未找到匹配的文件对".to_string());
            }
        }
    }

    fn start_comparison(&mut self) {
        if self.pairs.is_empty() {
            return;
        }
        let selected: Vec<_> = self.pairs.iter().filter(|p| p.selected).cloned().collect();
        if selected.is_empty() {
            self.error_message = Some("请至少选择一个文件对".to_string());
            return;
        }

        self.progress = 0.0;
        self.max_progress = 0.0;
        self.current_file = "准备中...".to_string();
        self.eta_display = 0.0;
        self.error_message = None;
        self.processing = true;
        self.selected_record = None;

        let ps = &self.progress_shared;
        ps.phase.store(0, Ordering::SeqCst);
        ps.processed.store(0, Ordering::SeqCst);
        ps.total.store(selected.len() as u32, Ordering::SeqCst);
        *ps.fps.lock().unwrap() = 0.0;
        *ps.current_file.lock().unwrap() = String::new();
        *ps.error_msg.lock().unwrap() = None;
        ps.running.store(true, Ordering::SeqCst);
        ps.done.store(false, Ordering::SeqCst);

        let (tx, rx) = mpsc::channel();
        *ps.result_sender.lock().unwrap() = Some(tx.clone());

        let progress_shared = self.progress_shared.clone();
        let db = self.db.clone();

        // GPU 模式：尝试获取 GPU 版 FFmpeg 路径
        let mut ffmpeg_path: Option<PathBuf> = None;
        if self.gpu_enabled && self.gpu_available {
            let mut mgr = crate::engine::FfmpegManager::new();
            if mgr.has_gpu() {
                if let Ok(()) = mgr.ensure_downloaded() {
                    // get_executable 返回完整路径字符串
                    let exe_path = mgr.get_executable();
                    if exe_path != "ffmpeg" {
                        let path = PathBuf::from(exe_path);
                        if path.exists() {
                            ffmpeg_path = Some(path);
                            tracing::info!("使用 GPU FFmpeg: {:?}", ffmpeg_path);
                        }
                    }
                }
            }
        }

        let config = ComputeConfig {
            use_gpu: self.gpu_enabled && self.gpu_available,
            gpu_device: 0,
            compute_vmaf: self.compute_vmaf,
            compute_ssim: self.compute_ssim,
            compute_psnr: self.compute_psnr,
            max_frames: self.max_frames,
            ffmpeg_path,
            ..Default::default()
        };

        tracing::info!(
            "开始对比: GPU={}, PSNR={}, SSIM={}, VMAF={}, max_frames={}, 线程数={}",
            config.use_gpu,
            config.compute_psnr,
            config.compute_ssim,
            config.compute_vmaf,
            config.max_frames,
            crate::runtime::get_rayon_threads()
        );

        std::thread::spawn(move || {
            // 先探测所有视频文件的详细信息（耗时操作）
            let mut selected_clone = selected.clone();
            tracing::info!("开始探测视频信息...");
            crate::engine::probe_videos_in_pairs(&mut selected_clone);

            // 更新全局 pairs 中的视频信息
            let shared = crate::engine::get_scan_result();
            let mut pairs = shared.pairs.lock().unwrap();
            for (i, p) in pairs.iter_mut().enumerate() {
                if i < selected_clone.len() {
                    p.ref_file = selected_clone[i].ref_file.clone();
                    p.dist_file = selected_clone[i].dist_file.clone();
                }
            }
            drop(pairs);

            let pipeline = Pipeline::new(config.clone());

            // 使用自适应并行处理，根据系统资源自动选择最优并行度
            // 使用增量模式：每完成一个任务就发送到 GUI
            let progress_shared_for_thread = progress_shared.clone();

            // 在后台线程中收集增量结果，同时写入数据库
            std::thread::spawn(move || {
                while let Ok(rec) = rx.recv() {
                    // 写入数据库，获取带 id 的 record
                    let rec_with_id = if let Ok(db_guard) = db.lock() {
                        if let Some(ref db) = *db_guard {
                            match db.insert_record(rec) {
                                Ok(r) => Some(r),
                                Err(e) => {
                                    tracing::error!("数据库写入失败: {}", e);
                                    None
                                }
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    // 发送到 GUI 显示（使用带 id 的 record）
                    if let Some(rec) = rec_with_id {
                        progress_shared_for_thread
                            .results
                            .lock()
                            .unwrap()
                            .push(rec);
                    }
                }
                tracing::info!("增量结果收集线程结束");
            });

            // 带 sender 的版本，每完成一个任务就增量发送
            // 注意：所有结果都通过 channel 发送，后台线程会写入数据库并发送到 GUI
            // 所以这里不需要再收集 final_results
            let _ = pipeline.process_batch_adaptive_with_sender(
                &selected_clone,
                |current, total, filename, elapsed, frame_count, eta| {
                    let _ = (current, total, filename, elapsed, frame_count, eta);
                },
                Some(tx),
            );

            // 注意：所有结果已经在后台线程中通过 channel 写入数据库
            // 这里只用于标记完成状态，不要重复写入数据库
            progress_shared.done.store(true, Ordering::SeqCst);
            progress_shared.running.store(false, Ordering::SeqCst);

            // 标记 sender 结束
            *progress_shared.result_sender.lock().unwrap() = None;
        });
    }

    fn poll_progress(&mut self) {
        let ps = &self.progress_shared;
        if ps.done.load(Ordering::SeqCst) {
            self.processing = false;
            self.progress = 100.0;
            // 清空进度缓存
            crate::engine::clear_pair_progress_map();
            self.pair_progress_cache.clear();

            // 处理增量结果：合并到现有结果
            let new_results = ps.results.lock().unwrap().clone();
            if !new_results.is_empty() {
                self.results.extend(new_results);
                ps.results.lock().unwrap().clear();
            }

            // 重新加载历史（从数据库）
            if let Ok(db_guard) = self.db.lock() {
                if let Some(ref db) = *db_guard {
                    if let Ok(history) = db.get_all_records() {
                        self.history = history.into_iter().rev().take(200).collect();
                    }
                }
            }
            if let Some(ref e) = *ps.error_msg.lock().unwrap() {
                self.error_message = Some(e.clone());
            }

            // 处理完成时，确保从数据库重新加载所有结果
            if ps.done.load(Ordering::SeqCst) {
                if let Ok(db_guard) = self.db.lock() {
                    if let Some(ref db) = *db_guard {
                        if let Ok(db_records) = db.get_all_records() {
                            if !db_records.is_empty() {
                                self.results = db_records;
                            }
                        }
                    }
                }
            }
            return;
        }
        if !ps.running.load(Ordering::SeqCst) && !ps.done.load(Ordering::SeqCst) {
            return;
        }

        // 实时显示已完成的配对（增量追加模式）
        let mut new_results = ps.results.lock().unwrap().clone();
        if !new_results.is_empty() {
            // 按 index 排序确保显示顺序正确
            new_results.sort_by_key(|r| r.index);
            // 追加新结果到显示列表
            self.results.extend(new_results);
            // 清空共享结果，准备接收下一批
            ps.results.lock().unwrap().clear();
        }

        // 根据缓存中的配对进度计算总体进度
        let mut new_progress = 0.0;
        let total_pairs = self.pairs.len();
        if total_pairs > 0 {
            let mut completed = 0u32;
            let mut running_frame_progress = 0.0f32;

            for (_, (frame, expected, status)) in self.pair_progress_cache.iter() {
                match status {
                    crate::engine::PairProcessingStatus::Completed => {
                        completed += 1;
                    }
                    crate::engine::PairProcessingStatus::Running => {
                        if *expected > 0 {
                            running_frame_progress += (*frame as f32 / *expected as f32).min(1.0);
                        }
                    }
                    _ => {}
                }
            }

            // 总体进度 = (已完成的 + 正在运行的帧进度比例) / 总数
            // 注意：running_frame_progress 已经是 0-1 范围内的比例
            new_progress =
                ((completed as f32 + running_frame_progress) / total_pairs as f32) * 100.0;
        }

        // 只更新更大的进度值，确保进度条只增不减
        if new_progress > self.max_progress {
            self.max_progress = new_progress;
        }
        self.progress = self.max_progress;

        self.eta_display = *ps.eta_seconds.lock().unwrap();
        self.current_file = ps.current_file.lock().unwrap().clone();
        if let Some(ref e) = *ps.error_msg.lock().unwrap() {
            self.error_message = Some(e.clone());
        }

        // 限流更新配对进度缓存（每50ms更新一次，避免性能问题）
        let now = std::time::Instant::now();
        if now.duration_since(self.last_progress_update).as_millis() >= 50 {
            self.last_progress_update = now;
            // 获取所有配对的进度
            let all_progress = crate::engine::get_all_pair_progress();
            for (idx, frame, expected, status) in all_progress {
                self.pair_progress_cache
                    .insert(idx, (frame, expected, status));
            }
        }
    }

    fn filtered_results(&self) -> Vec<ComparisonRecord> {
        let mut r = if self.filter_text.is_empty() {
            self.results.clone()
        } else {
            let q = self.filter_text.to_lowercase();
            self.results
                .iter()
                .filter(|rec| {
                    rec.ref_filename.to_lowercase().contains(&q)
                        || rec
                            .dist_filename
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&q)
                        || rec
                            .error_message
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&q)
                })
                .cloned()
                .collect()
        };

        // 排序
        let col = self.sort_col;
        let asc = self.sort_asc;
        r.sort_by(|a, b| {
            let cmp = match col {
                SortCol::Index => a.id.cmp(&b.id),
                SortCol::RefName => a.ref_filename.cmp(&b.ref_filename),
                SortCol::RefSize => a.ref_filesize.cmp(&b.ref_filesize),
                SortCol::DistName => a.dist_filename.cmp(&b.dist_filename),
                SortCol::DistSize => a.dist_filesize.cmp(&b.dist_filesize),
                SortCol::RefBitrate => a.ref_bitrate.cmp(&b.ref_bitrate),
                SortCol::DistBitrate => a.dist_bitrate.unwrap_or(0).cmp(&b.dist_bitrate.unwrap_or(0)),
                SortCol::Ratio => {
                    let ra = a.compression_ratio.unwrap_or(-1.0);
                    let rb = b.compression_ratio.unwrap_or(-1.0);
                    ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortCol::Psnr => {
                    let pa = a.psnr.unwrap_or(-1.0);
                    let pb = b.psnr.unwrap_or(-1.0);
                    pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortCol::Ssim => {
                    let sa = a.ssim.unwrap_or(-1.0);
                    let sb = b.ssim.unwrap_or(-1.0);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortCol::Vmaf => {
                    let va = a.vmaf.unwrap_or(-1.0);
                    let vb = b.vmaf.unwrap_or(-1.0);
                    va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortCol::Time => a.processing_time_ms.cmp(&b.processing_time_ms),
                SortCol::Status => {
                    let sa = format!("{:?}", a.status);
                    let sb = format!("{:?}", b.status);
                    sa.cmp(&sb)
                }
            };
            if asc {
                cmp
            } else {
                cmp.reverse()
            }
        });
        r
    }

    /// 获取分页后的结果（当前页）
    fn paginated_results(&self) -> (Vec<ComparisonRecord>, Vec<usize>) {
        let filtered = self.filtered_results();
        let total = filtered.len();
        if total == 0 {
            return (Vec::new(), Vec::new());
        }

        let total_pages = ((total + self.page_size - 1) / self.page_size).max(1);
        let page = self.page.min(total_pages).max(1);
        let start = (page - 1) * self.page_size;
        let end = (start + self.page_size).min(total);

        if start >= total {
            return (filtered[..self.page_size.min(total)].to_vec(), vec![0; self.page_size.min(total)]);
        }

        // 返回分页数据和对应的 results 索引
        let page_records = filtered[start..end].to_vec();
        let results_indices: Vec<usize> = (start..end)
            .filter_map(|i| {
                let rec = &filtered[i];
                self.results.iter().position(|r| r.id == rec.id)
            })
            .collect();

        (page_records, results_indices)
    }

    /// 选择所有过滤后的数据
    fn select_all_filtered(&mut self) {
        let filtered = self.filtered_results();
        for rec in &filtered {
            if let Some(idx) = self.results.iter().position(|r| r.id == rec.id) {
                self.selected_for_delete.insert(idx);
            }
        }
    }

    fn do_export(&self, fmt: ExportFormat) {
        if self.results.is_empty() {
            return;
        }
        let ext = match fmt {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            _ => "txt",
        };
        let timestamp = chrono::Local::now().format("%Y-%m-%d-%H-%M").to_string();
        let default_name = format!("vidcompare_report_{}.{}", timestamp, ext);
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(ext, &[ext])
            .set_file_name(&default_name)
            .save_file()
        {
            let opts = ExportOptions {
                format: fmt,
                output_path: path.to_string_lossy().to_string(),
                include_skipped: true,
                title: "Video Quality Comparison".to_string(),
            };
            if let Err(e) = export_records(&self.results, &opts) {
                error!("Export failed: {}", e);
            }
        }
    }

    /// 导出过滤后的全部数据
    fn do_export_full(&self, fmt: ExportFormat) {
        if self.results.is_empty() {
            return;
        }
        let ext = match fmt {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            _ => "txt",
        };
        let timestamp = chrono::Local::now().format("%Y-%m-%d-%H-%M").to_string();
        let default_name = format!("vidcompare_full_report_{}.{}", timestamp, ext);
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(ext, &[ext])
            .set_file_name(&default_name)
            .save_file()
        {
            let filtered = self.filtered_results();
            let opts = ExportOptions {
                format: fmt,
                output_path: path.to_string_lossy().to_string(),
                include_skipped: true,
                title: "Video Quality Comparison (Full)".to_string(),
            };
            if let Err(e) = export_records(&filtered, &opts) {
                error!("Export failed: {}", e);
            }
        }
    }

    fn load_history(&mut self) {
        if self.history_loaded {
            return;
        }
        if let Ok(db_guard) = self.db.lock() {
            if let Some(ref db) = *db_guard {
                match db.get_all_records() {
                    Ok(records) => {
                        self.history = records.into_iter().rev().take(200).collect();
                        self.history_loaded = true;
                    }
                    Err(e) => error!("加载历史失败: {}", e),
                }
            }
        }
    }
}

// ============================================================
// UI 渲染
// ============================================================

impl eframe::App for VidCompareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 轮询扫描结果 (非阻塞)
        self.poll_scan_result();

        if self.processing {
            self.poll_progress();
            ctx.request_repaint_after(Duration::from_millis(50)); // 高频率刷新用于动画
        } else if self.scanning {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.load_history();

        // 更新动画时间
        self.animation_time = ctx.input(|i| i.time) as f32;

        // 字体
        let mut fonts = egui::FontDefinitions::default();
        #[cfg(windows)]
        {
            if let Ok(windir) = std::env::var("WINDIR") {
                for path in [
                    format!("{}/Fonts/msyh.ttc", windir),
                    format!("{}/Fonts/simhei.ttf", windir),
                ] {
                    if std::path::Path::new(&path).exists() {
                        if let Ok(data) = std::fs::read(&path) {
                            fonts
                                .font_data
                                .insert("cjk".into(), egui::FontData::from_owned(data).into());
                            fonts
                                .families
                                .entry(egui::FontFamily::Proportional)
                                .or_default()
                                .insert(0, "cjk".into());
                            fonts
                                .families
                                .entry(egui::FontFamily::Monospace)
                                .or_default()
                                .insert(0, "cjk".into());
                            break;
                        }
                    }
                }
            }
        }
        ctx.set_fonts(fonts);
        ctx.set_visuals(egui::Visuals::dark());

        // 主面板 - 带边框的垂直三段
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(C_BG)
                    .inner_margin(egui::Margin::same(8.0))
                    .stroke(egui::Stroke::new(1.0, C_BORDER)),
            )
            .show(ctx, |ui| {
                // ===== 段1: 目录选择 =====
                self.render_top_section(ui);

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                // ===== 段2: 扫描 & 设置 =====
                self.render_middle_section(ui);

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                // ===== 段3: 结果表格 =====
                self.render_bottom_section(ui);
            });
    }
}

impl VidCompareApp {
    // ============================================================
    // 段1: 目录选择 + GPU + FFmpeg状态
    // ============================================================
    fn render_top_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // 标题
            ui.label(
                egui::RichText::new("VidCompare")
                    .size(20.0)
                    .strong()
                    .color(C_PRIMARY),
            );
            ui.label(
                egui::RichText::new("视频质量分析工具")
                    .size(12.0)
                    .color(C_MUTED),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // FFmpeg状态
                let (fv, fc) = if self.ffmpeg_ok {
                    (
                        self.ffmpeg_version
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .to_string(),
                        C_SUCCESS,
                    )
                } else {
                    ("未安装".to_string(), C_ERROR)
                };
                ui.label(egui::RichText::new(fv).size(11.0).color(fc));
                ui.label(egui::RichText::new("FFmpeg").size(10.0).color(C_MUTED));

                ui.add_space(8.0);

                // GPU
                if self.gpu_available {
                    let gc = if self.gpu_enabled { C_SUCCESS } else { C_MUTED };
                    ui.label(egui::RichText::new(&self.gpu_name).size(10.0).color(gc));
                    ui.checkbox(&mut self.gpu_enabled, "");
                    ui.label(egui::RichText::new("GPU加速").size(10.0).color(C_MUTED));
                } else {
                    ui.label(egui::RichText::new("CPU模式").size(10.0).color(C_MUTED));
                }
            });
        });

        ui.add_space(8.0);

        // 两列: 源目录 | 目标目录
        ui.horizontal(|ui| {
            // 左: 源目录
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("源目录 (原文件)")
                        .size(11.0)
                        .color(C_MUTED)
                        .strong(),
                );
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    let tr = ui.text_edit_singleline(&mut self.ref_dir);
                    if tr.lost_focus() && !self.ref_dir.is_empty() {
                        // trigger scan on dir change if both set
                    }
                    if ui
                        .add_sized(
                            [60.0, 24.0],
                            egui::Button::new("浏览")
                                .fill(C_SURFACE2)
                                .min_size([60.0, 24.0].into()),
                        )
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.ref_dir = p.to_string_lossy().to_string();
                        }
                    }
                });
            });

            ui.add_space(16.0);

            // 右: 目标目录
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("目标目录 (压缩文件)")
                        .size(11.0)
                        .color(C_MUTED)
                        .strong(),
                );
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.dist_dir);
                    if ui
                        .add_sized(
                            [60.0, 24.0],
                            egui::Button::new("浏览")
                                .fill(C_SURFACE2)
                                .min_size([60.0, 24.0].into()),
                        )
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.dist_dir = p.to_string_lossy().to_string();
                        }
                    }
                });
            });
        });

        if let Some(ref e) = self.error_message.take() {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("⚠ {}", e))
                    .size(11.0)
                    .color(C_ERROR),
            );
            self.error_message = Some(e.clone());
        }
    }

    // ============================================================
    // 段2: 扫描 & 设置
    // ============================================================
    fn render_middle_section(&mut self, ui: &mut egui::Ui) {
        // 第一行: 扫描按钮 + 设置 + 开始按钮
        ui.horizontal(|ui| {
            // 扫描
            let scan_disabled = self.ref_dir.is_empty() || self.dist_dir.is_empty();
            let scan_text = if scan_disabled {
                egui::RichText::new("🔍 扫描目录").size(13.0).color(C_TEXT)
            } else {
                egui::RichText::new("🔍 扫描目录").size(13.0).color(C_TEXT)
            };
            if ui
                .add_sized(
                    [100.0, 32.0],
                    egui::Button::new(scan_text)
                        .fill(if scan_disabled { C_SURFACE2 } else { C_PRIMARY })
                        .min_size([100.0, 32.0].into()),
                )
                .clicked()
            {
                self.scan_dirs();
            }

            // 统计
            if !self.pairs.is_empty() {
                ui.add_space(8.0);
                let sel = self.pairs.iter().filter(|p| p.selected).count();
                ui.label(
                    egui::RichText::new(format!("{}对匹配, {}已选", self.pairs.len(), sel))
                        .size(11.0)
                        .color(C_MUTED),
                );

                // 全选/取消
                let all_sel = self.pairs.iter().all(|p| p.selected);
                if ui
                    .add_sized(
                        [60.0, 24.0],
                        egui::Button::new(if all_sel { "取消全选" } else { "全选" })
                            .fill(C_SURFACE2)
                            .min_size([60.0, 24.0].into()),
                    )
                    .clicked()
                {
                    let t = !all_sel;
                    for p in &mut self.pairs {
                        p.selected = t;
                    }
                }
            }

            ui.add_space(16.0);

            // 计算设置
            ui.label(egui::RichText::new("指标:").size(11.0).color(C_MUTED));
            ui.checkbox(&mut self.compute_ssim, "SSIM");
            ui.add_space(4.0);
            ui.checkbox(&mut self.compute_psnr, "PSNR");
            ui.add_space(4.0);
            ui.checkbox(&mut self.compute_vmaf, "VMAF");

            ui.add_space(8.0);
            ui.label(egui::RichText::new("最大帧数:").size(11.0).color(C_MUTED));
            ui.add_sized(
                [120.0, 24.0],
                egui::Slider::new(&mut self.max_frames, 10..=2000),
            );
            ui.label(
                egui::RichText::new(format!("({}帧/视频)", self.max_frames))
                    .size(9.0)
                    .color(C_MUTED),
            );

            // 开始按钮
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_start = !self.pairs.is_empty()
                    && self.pairs.iter().any(|p| p.selected)
                    && self.ffmpeg_ok
                    && !self.processing;

                let btn = egui::Button::new(
                    egui::RichText::new(if self.processing {
                        "处理中..."
                    } else {
                        "▶ 开始对比"
                    })
                    .size(14.0)
                    .color(C_TEXT),
                )
                .fill(if can_start { C_SUCCESS } else { C_SURFACE2 })
                .min_size([120.0, 36.0].into());

                if ui.add_sized([120.0, 36.0], btn).clicked() && can_start {
                    self.start_comparison();
                }
            });
        });

        // 进度条
        if self.processing {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [300.0, 16.0],
                    egui::Label::new(
                        egui::RichText::new(&self.current_file)
                            .size(10.0)
                            .color(C_TEXT),
                    ),
                );
                ui.add_space(8.0);
                ui.add(
                    SmoothProgressBar::new(self.progress / 100.0, self.animation_time)
                        .fill(C_PRIMARY)
                        .desired_width(ui.available_width() - 200.0),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!("{:.1}%", self.progress))
                        .size(12.0)
                        .color(C_PRIMARY)
                        .strong(),
                );
                ui.add_space(4.0);
                if self.eta_display > 0.0 {
                    ui.add_space(4.0);
                    let eta = if self.eta_display >= 3600.0 {
                        format!("ETA{:.0}h", self.eta_display / 3600.0)
                    } else if self.eta_display >= 60.0 {
                        format!("ETA{:.0}m", self.eta_display / 60.0)
                    } else {
                        format!("ETA{:.0}s", self.eta_display)
                    };
                    ui.label(egui::RichText::new(eta).size(10.0).color(C_WARNING));
                }
            });
        }

        // 扫描进度条
        if self.scanning {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [300.0, 16.0],
                    egui::Label::new(
                        egui::RichText::new(format!("正在扫描: {}", self.scan_current_file))
                            .size(10.0)
                            .color(C_TEXT),
                    ),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::ProgressBar::new(self.scan_progress)
                        .fill(C_SECONDARY)
                        .desired_width(ui.available_width() - 200.0),
                );
                ui.add_space(8.0);
                let prog = crate::engine::get_scan_progress();
                let processed = prog.files_processed.load(Ordering::Relaxed);
                let total = prog.files_total.load(Ordering::Relaxed);
                ui.label(
                    egui::RichText::new(format!("{}/{}", processed, total))
                        .size(11.0)
                        .color(C_SECONDARY),
                );
            });
        }

        // 文件对列表 - 使用滚动区域显示所有文件
        if !self.pairs.is_empty() {
            ui.add_space(6.0);
            let pair_count = self.pairs.len();

            // 显示配对文件列表，带滚动，每项显示各自的进度
            egui::ScrollArea::vertical()
                .max_height(140.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::default()
                        .fill(C_SURFACE)
                        .inner_margin(egui::Margin::same(4.0))
                        .show(ui, |ui| {
                            for p in &mut self.pairs {
                                ui.horizontal(|ui| {
                                    let mut cb = p.selected;
                                    let clicked = ui
                                        .add_sized([20.0, 20.0], egui::Checkbox::new(&mut cb, ""))
                                        .clicked();
                                    if clicked {
                                        p.selected = cb;
                                    }

                                    let dn = p
                                        .distorted
                                        .as_ref()
                                        .map(|d| d.name.as_str())
                                        .unwrap_or("—");
                                    let dc = if dn == "—" { C_MUTED } else { C_SECONDARY };

                                    // 文件名
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} → {}",
                                            p.ref_file.name, dn
                                        ))
                                        .size(11.0)
                                        .color(dc),
                                    );

                                    // 如果正在处理，显示各自的帧进度（使用 p.index - 1 作为缓存键）
                                    // 注意：完成后也要显示完成状态，所以 processing 检查只针对进度条
                                    let cache_key = (p.index - 1) as u32;
                                    if let Some((frame, expected, status)) =
                                        self.pair_progress_cache.get(&cache_key)
                                    {
                                        if self.processing
                                            && *expected > 0
                                            && *status
                                                == crate::engine::PairProcessingStatus::Running
                                        {
                                            ui.add_space(8.0);
                                            let progress =
                                                (*frame as f32 / *expected as f32).min(1.0);
                                            ui.add(
                                                egui::ProgressBar::new(progress)
                                                    .fill(C_PRIMARY)
                                                    .desired_width(100.0),
                                            );
                                            ui.add_space(4.0);
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{}/{}",
                                                    frame, expected
                                                ))
                                                .size(9.0)
                                                .color(C_PRIMARY),
                                            );
                                        }
                                        if *status == crate::engine::PairProcessingStatus::Completed
                                        {
                                            ui.add_space(8.0);
                                            ui.label(
                                                egui::RichText::new("ok")
                                                    .size(11.0)
                                                    .color(C_SUCCESS),
                                            );
                                        } else if *status
                                            == crate::engine::PairProcessingStatus::Failed
                                        {
                                            ui.add_space(8.0);
                                            ui.label(
                                                egui::RichText::new("fail")
                                                    .size(11.0)
                                                    .color(C_ERROR),
                                            );
                                        }
                                    }
                                });
                            }
                        });
                });

            // 显示总体进度条
            if self.processing && self.progress > 0.0 {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("总体进度: {:.1}%", self.progress))
                            .size(10.0)
                            .color(C_PRIMARY),
                    );
                    ui.add_space(8.0);
                    ui.add(
                        SmoothProgressBar::new(self.progress / 100.0, self.animation_time)
                            .fill(C_PRIMARY)
                            .desired_width(300.0),
                    );
                    if self.eta_display > 0.0 {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("ETA{:.0}s", self.eta_display))
                                .size(9.0)
                                .color(C_WARNING),
                        );
                    }
                });
            }

            // 显示总数统计
            let sel = self.pairs.iter().filter(|p| p.selected).count();
            ui.label(
                egui::RichText::new(format!("共 {} 对, {} 已选", pair_count, sel))
                    .size(10.0)
                    .color(C_MUTED),
            );
        }
    }

    // ============================================================
    // 段3: 过滤 + 排序表格 + 导出 + 详情
    // ============================================================
    fn render_bottom_section(&mut self, ui: &mut egui::Ui) {
        // 工具栏: 过滤 + 排序 + 导出
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("过滤:")
                    .size(12.0)
                    .color(C_TEXT)
                    .strong(),
            );
            ui.add_sized(
                [200.0, 24.0],
                egui::TextEdit::singleline(&mut self.filter_text)
                    .hint_text("按文件名/错误信息过滤..."),
            );
            if !self.filter_text.is_empty() {
                if ui
                    .add_sized([60.0, 24.0], egui::Button::new("清除").fill(C_SURFACE2))
                    .clicked()
                {
                    self.filter_text.clear();
                }
            }

            ui.add_space(8.0);

            let filtered = self.filtered_results();
            let total_pages = ((filtered.len() + self.page_size - 1) / self.page_size).max(1);
            if self.page > total_pages && total_pages > 0 {
                self.page = total_pages;
            }

            ui.label(
                egui::RichText::new(format!("共 {} 条", filtered.len()))
                    .size(11.0)
                    .color(C_MUTED),
            );

            // 分页大小选择
            ui.add_space(8.0);
            ui.label(egui::RichText::new("每页:").size(10.0).color(C_MUTED));
            let page_sizes = [10, 20, 50, 100];
            let current_page_size = self.page_size;
            let mut page_size_label = format!("{}", current_page_size);
            egui::ComboBox::from_label("")
                .selected_text(&page_size_label)
                .width(60.0)
                .show_ui(ui, |ui| {
                    for &size in &page_sizes {
                        let label = format!("{}", size);
                        ui.selectable_value(&mut page_size_label, label.clone(), label);
                    }
                });
            if let Ok(new_size) = page_size_label.parse::<usize>() {
                if new_size != current_page_size {
                    self.page_size = new_size;
                    self.page = 1;
                }
            }

            // 分页导航
            ui.add_space(16.0);
            if ui.add_sized([28.0, 24.0], egui::Button::new("◀").fill(C_SURFACE2)).clicked() {
                if self.page > 1 {
                    self.page -= 1;
                }
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("第 {}/{} 页", self.page, total_pages)).size(11.0).color(C_TEXT));
            ui.add_space(4.0);
            if ui.add_sized([28.0, 24.0], egui::Button::new("▶").fill(C_SURFACE2)).clicked() {
                if self.page < total_pages {
                    self.page += 1;
                }
            }

            // 跳转到页码
            ui.add_space(8.0);
            let mut jump_to = String::new();
            let response = ui.add_sized([50.0, 24.0], egui::TextEdit::singleline(&mut jump_to).hint_text("页码"));
            if response.lost_focus() && !jump_to.is_empty() {
                if let Ok(page) = jump_to.parse::<usize>() {
                    self.page = page.max(1).min(total_pages);
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.results.is_empty() {
                    let del_count = self.selected_for_delete.len();

                    // 全选/取消全选按钮（选择所有过滤后的数据）
                    let none_selected = self.selected_for_delete.is_empty();

                    if none_selected {
                        if ui
                            .add_sized([80.0, 24.0], egui::Button::new("全选").fill(C_SURFACE2))
                            .clicked()
                        {
                            self.select_all_filtered();
                        }
                    } else {
                        if ui
                            .add_sized([80.0, 24.0], egui::Button::new("取消全选").fill(C_SURFACE2))
                            .clicked()
                        {
                            self.selected_for_delete.clear();
                        }
                    }

                    ui.add_space(4.0);

                    if del_count > 0 {
                        if ui
                            .add_sized(
                                [80.0, 24.0],
                                egui::Button::new(format!("删除({})", del_count)).fill(C_ERROR),
                            )
                            .clicked()
                        {
                            // 收集要删除的 id（用于数据库删除）
                            let to_delete_ids: Vec<i64> = self.selected_for_delete
                                .iter()
                                .filter_map(|&i| self.results.get(i).and_then(|r| r.id))
                                .collect();

                            // 1. 从数据库删除
                            if !to_delete_ids.is_empty() {
                                if let Ok(db_guard) = self.db.lock() {
                                    if let Some(ref db) = *db_guard {
                                        if let Err(e) = db.delete_records(&to_delete_ids) {
                                            tracing::error!("数据库删除失败: {}", e);
                                        }
                                    }
                                }
                            }

                            // 2. 从内存结果删除（倒序，防止索引偏移）
                            let mut indices: Vec<usize> = self.selected_for_delete.iter().cloned().collect();
                            indices.sort_unstable();
                            for i in indices.iter().rev() {
                                self.results.remove(*i);
                            }

                            // 3. 清空选择状态（UI 会重新渲染，用户需要重新选择）
                            self.selected_for_delete.clear();
                            self.selected_record = None;
                        }
                        ui.add_space(4.0);
                    }

                    if ui
                        .add_sized([80.0, 24.0], egui::Button::new("清空全部").fill(C_ERROR))
                        .clicked()
                    {
                        // 清空数据库
                        if let Ok(db_guard) = self.db.lock() {
                            if let Some(ref db) = *db_guard {
                                if let Err(e) = db.truncate() {
                                    tracing::error!("清空数据库失败: {}", e);
                                }
                            }
                        }
                        // 清空内存中的结果
                        self.results.clear();
                        self.selected_for_delete.clear();
                        self.selected_record = None;
                        self.page = 1;
                    }
                    ui.add_space(4.0);

                    // 导出全部（过滤后）
                    if ui
                        .add_sized([90.0, 24.0], egui::Button::new("导出全部 CSV").fill(C_SURFACE2))
                        .clicked()
                    {
                        self.do_export_full(ExportFormat::Csv);
                    }
                    if ui
                        .add_sized(
                            [90.0, 24.0],
                            egui::Button::new("导出全部 JSON").fill(C_SURFACE2),
                        )
                        .clicked()
                    {
                        self.do_export_full(ExportFormat::Json);
                    }
                }

                if !self.history.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("历史: {}", self.history.len()))
                            .size(10.0)
                            .color(C_MUTED),
                    );
                }
            });
        });

        // 表格区域 - 表头和数据
        let available = ui.available_width() * 0.80;

        // 列宽比例: 勾选3%, 序号4%, 原文件15%, 分辨率8%, 原大小9%, 压缩文件15%, 压后大小9%, 原码率8%, 压后码率8%, 压缩比6%, PSNR5%, SSIM5%, VMAF5%, 耗时5%, 状态5% (总和100%)
        let ratios = [
            0.03, 0.04, 0.15, 0.08, 0.09, 0.15, 0.09, 0.08, 0.08, 0.06, 0.05, 0.05, 0.05, 0.05,
            0.05,
        ];
        let col_labels = [
            "",
            "#",
            "原文件",
            "分辨率",
            "原大小",
            "压缩文件",
            "压后大小",
            "原码率",
            "压后码率",
            "压缩比",
            "PSNR",
            "SSIM",
            "VMAF",
            "耗时",
            "状态",
        ];

        // 预计算每列宽度
        let col_widths: Vec<f32> = ratios.iter().map(|r| available * r).collect();

        // 表头
        ui.horizontal(|ui| {
            for (i, (width, label)) in col_widths.iter().zip(col_labels.iter()).enumerate() {
                let sort_col_opt = match i {
                    1 => Some(SortCol::Index),
                    2 => Some(SortCol::RefName),
                    4 => Some(SortCol::RefSize),
                    5 => Some(SortCol::DistName),
                    6 => Some(SortCol::DistSize),
                    7 => Some(SortCol::RefBitrate),
                    8 => Some(SortCol::DistBitrate),
                    9 => Some(SortCol::Ratio),
                    10 => Some(SortCol::Psnr),
                    11 => Some(SortCol::Ssim),
                    12 => Some(SortCol::Vmaf),
                    13 => Some(SortCol::Time),
                    14 => Some(SortCol::Status),
                    _ => None,
                };

                let is_active = sort_col_opt.map(|c| c == self.sort_col).unwrap_or(false);
                let label_text = if is_active {
                    format!("{}{}", if self.sort_asc { "↑" } else { "↓" }, label)
                } else {
                    label.to_string()
                };

                let text_color = if is_active { C_PRIMARY } else { C_MUTED };
                let bg_fill = if is_active { C_SURFACE2 } else { C_HEADER };

                if let Some(col) = sort_col_opt {
                    if ui
                        .add_sized(
                            [*width, 20.0],
                            egui::Button::new(
                                egui::RichText::new(label_text)
                                    .size(10.0)
                                    .color(text_color)
                                    .strong(),
                            )
                            .fill(bg_fill),
                        )
                        .clicked()
                    {
                        if self.sort_col == col {
                            self.sort_asc = !self.sort_asc;
                        } else {
                            self.sort_col = col;
                            self.sort_asc = true;
                        }
                    }
                } else {
                    // 非排序列：绘制与可排序列一致的 C_HEADER 背景
                    let avail = ui.available_rect_before_wrap();
                    let rect = egui::Rect::from_min_max(
                        avail.min,
                        egui::pos2(avail.min.x + *width, avail.min.y + 20.0),
                    );
                    ui.painter().rect_filled(rect, 0.0, C_HEADER);
                    ui.add_sized(
                        [*width, 20.0],
                        egui::Label::new(
                            egui::RichText::new(label_text)
                                .size(10.0)
                                .color(text_color)
                                .strong(),
                        ),
                    );
                }
            }
        });

        // 表格内容 - 使用分页后的数据
        let (page_records, page_results_indices) = self.paginated_results();
        let total_filtered = self.filtered_results().len();

        // 计算表格行高，每行高度 * 行数 = 最小滚动高度
        // 使用一个足够大的固定值确保滚动条始终有效
        let row_height = 28.0;
        // 强制设置一个较大的最小高度，确保滚动条可用
        let min_scroll_height = (row_height * total_filtered.max(20) as f32).max(600.0);

        let scroll_area = egui::ScrollArea::vertical()
            .min_scrolled_height(min_scroll_height);

        scroll_area.show(ui, |ui| {
                egui::Frame::default()
                    .inner_margin(egui::Margin::symmetric(4.0, 2.0))
                    .show(ui, |ui| {
                        if total_filtered == 0 && !self.results.is_empty() {
                            ui.label(
                                egui::RichText::new("没有匹配的结果")
                                    .size(12.0)
                                    .color(C_MUTED),
                            );
                            return;
                        }
                        if total_filtered == 0 && self.results.is_empty() {
                            ui.label(
                                egui::RichText::new("暂无结果 - 请先选择目录并点击开始对比")
                                    .size(12.0)
                                    .color(C_MUTED),
                            );
                            return;
                        }

                        for (row_idx, (rec, results_idx)) in page_records.iter().zip(page_results_indices.iter()).enumerate() {
                            // 计算显示序号 (基于分页和排序后的全局位置)
                            let start_idx = (self.page - 1) * self.page_size;
                            let display_index = start_idx + row_idx + 1;
                            self.render_result_row(ui, rec, display_index as u32, Some(*results_idx), &col_widths, row_idx);
                        }
                    });
            });

        // 详情面板
        if let Some(rec) = self.selected_record.clone() {
            ui.add_space(4.0);
            egui::Frame::default()
                .fill(C_SURFACE)
                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("详情")
                                .size(12.0)
                                .color(C_PRIMARY)
                                .strong(),
                        );
                        if ui
                            .add_sized([60.0, 20.0], egui::Button::new("关闭").fill(C_SURFACE2))
                            .clicked()
                        {
                            self.selected_record = None;
                        }
                        ui.add_space(16.0);
                        self.render_detail_fields(ui, &rec);
                    });
                });
        }
    }

    fn render_result_row(
        &mut self,
        ui: &mut egui::Ui,
        rec: &ComparisonRecord,
        _display_index: u32,
        results_idx: Option<usize>,
        col_widths: &[f32],
        row_idx: usize,
    ) {
        let is_selected = self
            .selected_record
            .as_ref()
            .map(|r| r.id == rec.id)
            .unwrap_or(false);
        let zebra_bg = if row_idx % 2 == 0 {
            C_SURFACE_ALT
        } else {
            C_SURFACE
        };
        let bg = if is_selected { C_SURFACE2 } else { zebra_bg };

        let total_width: f32 = col_widths.iter().sum();

        egui::Frame::default()
            .fill(bg)
            .inner_margin(egui::Margin::symmetric(2.0, 2.0))
            .show(ui, |ui| {
                ui.allocate_ui(egui::vec2(total_width, 28.0), |ui| {
                    ui.horizontal(|ui| {
                        // 删除选择复选框 - 使用 results_idx 作为 key
                        if let Some(idx) = results_idx {
                            let is_checked = self.selected_for_delete.contains(&idx);
                            let mut checked = is_checked;
                            let clicked = ui
                                .add_sized(
                                    [col_widths[0], 20.0],
                                    egui::Checkbox::new(&mut checked, ""),
                                )
                                .clicked();
                            if clicked {
                                if checked {
                                    self.selected_for_delete.insert(idx);
                                } else {
                                    self.selected_for_delete.remove(&idx);
                                }
                            }
                        }

                        // 序号 - 使用数据库自增ID
                        let id_for_display = rec.id.unwrap_or(row_idx as i64 + 1);
                        ui.add_sized(
                            [col_widths[1], 20.0],
                            egui::Label::new(
                                egui::RichText::new(format!("{}", id_for_display))
                                    .size(11.0)
                                    .color(C_MUTED),
                            ),
                        );

                        // 原文件名 - 自动换行
                        ui.add_sized(
                            [col_widths[2], 32.0],
                            egui::Label::new(
                                egui::RichText::new(&rec.ref_filename)
                                    .size(11.0)
                                    .color(C_TEXT),
                            )
                            .wrap(),
                        );

                        // 分辨率
                        let res_str = match (rec.ref_width, rec.ref_height) {
                            (Some(w), Some(h)) => format!("{}x{}", w, h),
                            _ => "—".to_string(),
                        };
                        ui.add_sized(
                            [col_widths[3], 20.0],
                            egui::Label::new(
                                egui::RichText::new(res_str).size(11.0).color(C_MUTED),
                            ),
                        );

                        // 原大小
                        ui.add_sized(
                            [col_widths[4], 20.0],
                            egui::Label::new(
                                egui::RichText::new(fmt_size(rec.ref_filesize))
                                    .size(11.0)
                                    .color(C_MUTED),
                            ),
                        );

                        // 压缩文件名 - 自动换行
                        let dn = rec.dist_filename.as_deref().unwrap_or("—");
                        let dc = if dn == "—" { C_MUTED } else { C_SECONDARY };
                        ui.add_sized(
                            [col_widths[5], 32.0],
                            egui::Label::new(egui::RichText::new(dn).size(11.0).color(dc)).wrap(),
                        );

                        // 压后大小
                        let ds = rec
                            .dist_filesize
                            .map(|s| fmt_size(s))
                            .unwrap_or_else(|| "—".to_string());
                        ui.add_sized(
                            [col_widths[6], 20.0],
                            egui::Label::new(egui::RichText::new(&ds).size(11.0).color(C_MUTED)),
                        );

                        // 原码率
                        ui.add_sized(
                            [col_widths[7], 20.0],
                            egui::Label::new(
                                egui::RichText::new(fmt_bitrate(rec.ref_bitrate))
                                    .size(10.0)
                                    .color(C_MUTED),
                            ),
                        );

                        // 压后码率
                        if let Some(db) = rec.dist_bitrate {
                            ui.add_sized(
                                [col_widths[8], 20.0],
                                egui::Label::new(
                                    egui::RichText::new(fmt_bitrate(db))
                                        .size(10.0)
                                        .color(C_MUTED),
                                ),
                            );
                        } else {
                            ui.add_sized(
                                [col_widths[8], 20.0],
                                egui::Label::new(
                                    egui::RichText::new("—").size(10.0).color(C_MUTED),
                                ),
                            );
                        }

                        // 压缩比
                        let ratio_str = rec
                            .compression_ratio
                            .map(|c| format!("{:.2}", c))
                            .unwrap_or_else(|| "—".to_string());
                        ui.add_sized(
                            [col_widths[9], 20.0],
                            egui::Label::new(
                                egui::RichText::new(ratio_str).size(11.0).color(C_MUTED),
                            ),
                        );

                        // PSNR (处理无穷大和 NaN)
                        let psnr_str = rec
                            .psnr
                            .map(|p| {
                                if p.is_infinite() {
                                    if p.is_sign_positive() {
                                        "∞".to_string()
                                    } else {
                                        "-∞".to_string()
                                    }
                                } else if p.is_nan() {
                                    "—".to_string()
                                } else {
                                    format!("{:.2}", p)
                                }
                            })
                            .unwrap_or_else(|| "—".to_string());
                        let psnr_col = rec
                            .psnr
                            .map(|v| {
                                if v >= 40.0 {
                                    C_SUCCESS
                                } else if v >= 30.0 {
                                    C_WARNING
                                } else {
                                    C_ERROR
                                }
                            })
                            .unwrap_or(C_MUTED);
                        ui.add_sized(
                            [col_widths[10], 20.0],
                            egui::Label::new(
                                egui::RichText::new(psnr_str).size(11.0).color(psnr_col),
                            ),
                        );

                        // SSIM
                        ui.add_sized(
                            [col_widths[11], 20.0],
                            egui::Label::new(
                                egui::RichText::new(
                                    rec.ssim
                                        .map(|s| format!("{:.4}", s))
                                        .unwrap_or_else(|| "—".to_string()),
                                )
                                .size(11.0)
                                .color(if rec.ssim.is_some() { C_TEXT } else { C_MUTED }),
                            ),
                        );

                        // VMAF
                        let vmaf_col = rec
                            .vmaf
                            .map(|v| {
                                if v >= 80.0 {
                                    C_SUCCESS
                                } else if v >= 60.0 {
                                    C_WARNING
                                } else {
                                    C_ERROR
                                }
                            })
                            .unwrap_or(C_MUTED);
                        ui.add_sized(
                            [col_widths[12], 20.0],
                            egui::Label::new(
                                egui::RichText::new(
                                    rec.vmaf
                                        .map(|v| format!("{:.1}", v))
                                        .unwrap_or_else(|| "—".to_string()),
                                )
                                .size(11.0)
                                .color(vmaf_col),
                            ),
                        );

                        // 耗时
                        let time_str = rec
                            .processing_time_ms
                            .map(|t| fmt_duration(t))
                            .unwrap_or_else(|| "—".to_string());
                        ui.add_sized(
                            [col_widths[13], 20.0],
                            egui::Label::new(
                                egui::RichText::new(time_str).size(11.0).color(C_MUTED),
                            ),
                        );

                        // 状态 - 显示进度或状态
                        let row_bg = if is_selected { C_SURFACE2 } else { C_SURFACE };
                        let status_text;
                        let status_color = match rec.status {
                            ProcessingStatus::Completed => {
                                status_text = "OK".to_string();
                                C_SUCCESS
                            }
                            ProcessingStatus::Failed => {
                                status_text = "X".to_string();
                                C_ERROR
                            }
                            ProcessingStatus::Running => {
                                // 尝试从缓存获取帧进度
                                if let Some((frame, expected, _)) =
                                    self.pair_progress_cache.get(&(rec.index - 1))
                                {
                                    if *expected > 0 {
                                        status_text = format!("{}/{}", frame, expected);
                                    } else {
                                        status_text = "...".to_string();
                                    }
                                } else {
                                    status_text = "...".to_string();
                                }
                                C_PRIMARY
                            }
                            ProcessingStatus::Skipped => {
                                status_text = "--".to_string();
                                C_WARNING
                            }
                            ProcessingStatus::Pending => {
                                status_text = "-".to_string();
                                C_MUTED
                            }
                        };
                        if ui
                            .add_sized(
                                [col_widths[14], 20.0],
                                egui::Button::new(
                                    egui::RichText::new(&status_text)
                                        .size(10.0)
                                        .color(status_color),
                                )
                                .fill(row_bg),
                            )
                            .clicked()
                        {
                            self.selected_record = Some(rec.clone());
                        }
                    });
                });
            });
    }

    fn render_detail_fields(&self, ui: &mut egui::Ui, rec: &ComparisonRecord) {
        // 紧凑的详情展示: 左原文件信息，右压缩文件信息，下质量指标
        ui.vertical(|ui| {
            // 文件信息行
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("原文件:").size(10.0).color(C_MUTED));
                ui.label(
                    egui::RichText::new(&rec.ref_filename)
                        .size(10.0)
                        .color(C_TEXT),
                );
                ui.add_space(8.0);
                ui.label(egui::RichText::new("大小:").size(10.0).color(C_MUTED));
                ui.label(
                    egui::RichText::new(fmt_size(rec.ref_filesize))
                        .size(10.0)
                        .color(C_TEXT),
                );
                ui.add_space(8.0);
                ui.label(egui::RichText::new("码率:").size(10.0).color(C_MUTED));
                ui.label(
                    egui::RichText::new(fmt_bitrate(rec.ref_bitrate))
                        .size(10.0)
                        .color(C_TEXT),
                );
                ui.add_space(8.0);
                ui.label(egui::RichText::new("路径:").size(10.0).color(C_MUTED));
                ui.label(egui::RichText::new(&rec.ref_path).size(9.0).color(C_MUTED));
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if let Some(ref dn) = rec.dist_filename {
                    ui.label(egui::RichText::new("压缩:").size(10.0).color(C_MUTED));
                    ui.label(egui::RichText::new(dn).size(10.0).color(C_SECONDARY));
                    ui.add_space(8.0);
                    if let Some(ds) = rec.dist_filesize {
                        ui.label(egui::RichText::new("大小:").size(10.0).color(C_MUTED));
                        ui.label(egui::RichText::new(fmt_size(ds)).size(10.0).color(C_TEXT));
                        ui.add_space(8.0);
                    }
                    if let Some(db) = rec.dist_bitrate {
                        ui.label(egui::RichText::new("码率:").size(10.0).color(C_MUTED));
                        ui.label(
                            egui::RichText::new(fmt_bitrate(db))
                                .size(10.0)
                                .color(C_TEXT),
                        );
                        ui.add_space(8.0);
                    }
                    if let Some(r) = rec.compression_ratio {
                        ui.label(egui::RichText::new("压缩比:").size(10.0).color(C_MUTED));
                        ui.label(
                            egui::RichText::new(format!("{:.2}", r))
                                .size(10.0)
                                .color(C_WARNING),
                        );
                        ui.add_space(8.0);
                    }
                }
                if let Some(fc) = rec.frame_count {
                    ui.label(egui::RichText::new("帧数:").size(10.0).color(C_MUTED));
                    ui.label(
                        egui::RichText::new(format!("{}", fc))
                            .size(10.0)
                            .color(C_TEXT),
                    );
                    ui.add_space(8.0);
                }
                if let Some(fps) = rec.avg_fps {
                    ui.label(egui::RichText::new("FPS:").size(10.0).color(C_MUTED));
                    ui.label(
                        egui::RichText::new(format!("{:.1}", fps))
                            .size(10.0)
                            .color(C_TEXT),
                    );
                }
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("质量指标:").size(10.0).color(C_MUTED));
                if let Some(p) = rec.psnr {
                    ui.label(
                        egui::RichText::new(format!("PSNR {:.2}", p))
                            .size(10.0)
                            .color(if p >= 40.0 {
                                C_SUCCESS
                            } else if p >= 30.0 {
                                C_WARNING
                            } else {
                                C_ERROR
                            }),
                    );
                    ui.add_space(6.0);
                }
                if let Some(s) = rec.ssim {
                    ui.label(
                        egui::RichText::new(format!("SSIM {:.4}", s))
                            .size(10.0)
                            .color(C_TEXT),
                    );
                    ui.add_space(6.0);
                }
                if let Some(v) = rec.vmaf {
                    ui.label(
                        egui::RichText::new(format!("VMAF {:.1}", v))
                            .size(10.0)
                            .color(if v >= 80.0 {
                                C_SUCCESS
                            } else if v >= 60.0 {
                                C_WARNING
                            } else {
                                C_ERROR
                            }),
                    );
                    ui.add_space(6.0);
                }
                if let Some(t) = rec.processing_time_ms {
                    ui.label(
                        egui::RichText::new(format!("耗时 {}", fmt_duration(t)))
                            .size(10.0)
                            .color(C_MUTED),
                    );
                    ui.add_space(6.0);
                }
                if let Some(ref err) = rec.error_message {
                    if !err.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("错误: {}", err))
                                .size(10.0)
                                .color(C_ERROR),
                        );
                    }
                }
            });
        });
    }
}

// ============================================================
// 入口
// ============================================================

pub fn runGui(gpu_available: bool, gpu_name: String) -> Result<(), eframe::Error> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([1100.0, 650.0])
            .with_title("VidCompare - 视频质量分析工具")
            .with_decorations(true)
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "VidCompare",
        opts,
        Box::new(|_cc| Ok(Box::new(VidCompareApp::new(gpu_available, gpu_name)))),
    )
}
