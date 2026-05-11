//! VidCompare - 视频质量对比工具
//! 
//! CLI 界面

use std::path::PathBuf;
use tracing::{info, error};

use crate::config::ComputeConfig;
use crate::db::Database;
use crate::engine::{FilePair, ComparisonRecord, FfmpegManager, Pipeline, Matcher, scan_directory};
use crate::export::{export_records, ExportFormat, ExportOptions};

/// 应用状态
pub struct App {
    db: Option<Database>,
    pairs: Vec<FilePair>,
    results: Vec<ComparisonRecord>,
    config: ComputeConfig,
    gpu_available: bool,
    gpu_name: String,
}

impl App {
    pub fn new(db: Database, gpu_available: bool, gpu_name: String) -> Self {
        Self {
            db: Some(db),
            pairs: Vec::new(),
            results: Vec::new(),
            config: ComputeConfig::default(),
            gpu_available,
            gpu_name,
        }
    }

    /// 扫描并匹对文件
    pub fn scan_and_match(&mut self, ref_dir: &str, dist_dir: &str) -> Result<Vec<FilePair>, String> {
        let ref_path = PathBuf::from(ref_dir);
        let dist_path = PathBuf::from(dist_dir);
        
        if !ref_path.exists() {
            return Err(format!("目录不存在: {}", ref_dir));
        }
        
        if !dist_path.exists() {
            return Err(format!("目录不存在: {}", dist_dir));
        }
        
        info!("扫描目录: {} vs {}", ref_dir, dist_dir);
        
        let ref_files = scan_directory(&ref_path)
            .map_err(|e| format!("扫描原文件目录失败: {}", e))?;
        
        let dist_files = scan_directory(&dist_path)
            .map_err(|e| format!("扫描压缩文件目录失败: {}", e))?;
        
        info!("扫描完成: 原文件 {} 个, 压缩文件 {} 个", ref_files.len(), dist_files.len());
        
        let matcher = Matcher::new();
        let pairs = matcher.match_files(&ref_files, &dist_files);
        
        self.pairs = pairs.clone();
        self.results = Vec::new();
        
        Ok(pairs)
    }

    /// 开始对比
    /// progress_cb: (current, total, filename, elapsed_secs, eta_secs)
    pub fn start_comparison<F>(&mut self, progress_cb: F) -> Result<Vec<ComparisonRecord>, String>
    where
        F: Fn(usize, usize, &str, f32, Option<f32>) + Send + Sync + 'static,
    {
        let selected_pairs: Vec<_> = self.pairs.iter()
            .filter(|p| p.selected)
            .cloned()
            .collect();
        
        if selected_pairs.is_empty() {
            return Err("没有选中的文件配对".to_string());
        }
        
        info!("开始对比 {} 个配对", selected_pairs.len());
        
        let pipeline = Pipeline::new(self.config.clone());
        let results = pipeline.process_batch_sequential(&selected_pairs, progress_cb);
        
        let records: Vec<ComparisonRecord> = results.into_iter().map(|r| r.record).collect();
        
        // 写入数据库并获取带 id 的记录
        if let Some(ref db) = self.db {
            match db.insert_records(&records) {
                Ok(records_with_id) => {
                    self.results = records_with_id.clone();
                    return Ok(records_with_id);
                }
                Err(e) => {
                    error!("数据库写入失败: {}", e);
                    self.results = records.clone();
                }
            }
        } else {
            self.results = records.clone();
        }
        Ok(self.results.clone())
    }

    /// 更新配置
    pub fn update_config(&mut self, gpu_enabled: bool, compute_vmaf: bool, compute_ssim: bool, compute_psnr: bool) {
        self.config = ComputeConfig::from_ui(gpu_enabled, compute_vmaf, compute_ssim, compute_psnr);

        // GPU 模式：自动下载/使用 GPU 版 FFmpeg
        if gpu_enabled {
            let mut mgr = FfmpegManager::new();
            match mgr.ensure_downloaded() {
                Ok(()) => {
                    if let Some(path) = mgr.get_executable_path() {
                        let ffmpeg_bin = path.join("ffmpeg");
                        if ffmpeg_bin.exists() {
                            self.config.ffmpeg_path = Some(ffmpeg_bin);
                            info!("GPU FFmpeg 已设置: {:?}", self.config.ffmpeg_path);
                        }
                    }
                }
                Err(e) => {
                    error!("GPU FFmpeg 下载失败: {}", e);
                }
            }
        } else {
            self.config.ffmpeg_path = None;
        }
    }

    /// 获取结果列表
    pub fn get_results(&self) -> &[ComparisonRecord] {
        &self.results
    }

    /// 导出结果
    pub fn export(&self, format: ExportFormat, output_path: &str) -> Result<usize, String> {
        let options = ExportOptions {
            format,
            output_path: output_path.to_string(),
            include_skipped: true,
            title: "视频质量对比报告".to_string(),
        };
        
        export_records(&self.results, &options)
            .map_err(|e| format!("导出失败: {}", e))
    }

    /// 获取数据库统计
    pub fn get_stats(&self) -> Result<crate::db::Stats, String> {
        self.db
            .as_ref()
            .ok_or_else(|| "数据库未初始化".to_string())?
            .get_stats()
            .map_err(|e| format!("查询失败: {}", e))
    }

    /// 从数据库加载历史记录
    pub fn load_history(&mut self) -> Result<Vec<ComparisonRecord>, String> {
        self.db
            .as_ref()
            .ok_or_else(|| "数据库未初始化".to_string())?
            .get_all_records()
            .map_err(|e| format!("加载失败: {}", e))
    }
}

/// CLI 模式运行
pub fn run_cli(state: crate::AppState) -> Result<(), Box<dyn std::error::Error>> {
    println!("===========================================");
    println!("  VidCompare - 视频质量对比工具 v0.1.0");
    println!("===========================================");
    println!();
    
    println!("GPU 状态: {}", state.gpu_name);
    println!();
    
    // 检查 FFmpeg
    let ffmpeg_available = crate::engine::VideoDecoder::is_ffmpeg_available();
    if ffmpeg_available {
        if let Some(version) = crate::engine::VideoDecoder::get_ffmpeg_version() {
            println!("FFmpeg: {}", version.lines().next().unwrap_or("未知版本"));
        }
    } else {
        println!("警告: FFmpeg 未安装，部分功能可能受限");
        println!("  请从 https://ffmpeg.org 下载并安装 FFmpeg");
    }
    println!();
    
    // 示例：扫描目录
    println!("用法提示:");
    println!("  1. 使用 --ref <目录> 指定原文件目录");
    println!("  2. 使用 --dist <目录> 指定压缩文件目录");  
    println!("  3. 使用 --start 开始对比");
    println!("  4. 使用 --export-csv <文件> 导出 CSV");
    println!("  5. 使用 --export-json <文件> 导出 JSON");
    println!();
    println!("完整 CLI 和 GUI 功能开发中...");
    println!();
    
    Ok(())
}

/// 运行 GUI (当前实现为文本界面)
pub fn run(state: crate::AppState) -> Result<(), Box<dyn std::error::Error>> {
    run_cli(state)
}
