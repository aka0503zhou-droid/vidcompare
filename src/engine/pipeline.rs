//! 处理流水线模块
//!
//! 并行处理多个视频文件对比

use rayon::prelude::*;
use std::path::Path;
use tracing::info;

use super::{ComparisonRecord, DecoderConfig, FilePair, ProcessingStatus, VideoDecoder};
use crate::config::ComputeConfig;

/// 处理结果
#[derive(Debug, Clone)]
pub struct ProcessResult {
    pub record: ComparisonRecord,
    /// 处理吞吐量 (处理的帧数/秒)，不是视频的播放帧率
    pub throughput_fps: f32,
}

impl ProcessResult {
    /// 获取处理速度描述
    pub fn throughput_string(&self) -> String {
        if self.throughput_fps > 0.0 {
            format!("{:.1} 帧/秒", self.throughput_fps)
        } else {
            "—".to_string()
        }
    }
}

/// 并行处理流水线
pub struct Pipeline {
    config: ComputeConfig,
}

impl Pipeline {
    pub fn new(config: ComputeConfig) -> Self {
        Self { config }
    }

    /// 同步版本 - 处理单个文件配对
    pub fn process_pair(&self, pair: &FilePair, cache_idx: u32) -> ProcessResult {
        let mut record = ComparisonRecord::from_pair(pair);
        let start_time = std::time::Instant::now();

        // 检查是否有压缩文件
        let dist = match &pair.distorted {
            Some(d) => d,
            None => {
                record.status = ProcessingStatus::Skipped;
                return ProcessResult {
                    record,
                    throughput_fps: 0.0,
                };
            }
        };

        record.status = ProcessingStatus::Running;
        let throughput_fps;

        // 如果启用了任何指标，使用 FFmpeg 过滤器计算
        if self.config.compute_psnr || self.config.compute_ssim || self.config.compute_vmaf {
            let ffmpeg_path = self.config.ffmpeg_path.as_deref();
            let vmaf_model = self
                .config
                .vmaf_model_path
                .as_ref()
                .map(|p| Path::new(p.as_str()));

            // 设置全局配对索引，用于进度更新
            // 注意：这个索引会在 process_batch 时被外部设置

            match crate::engine::ffmpeg_filter::calculate_all_metrics_ffmpeg_with_progress(
                &pair.ref_file.path,
                &dist.path,
                ffmpeg_path,
                self.config.compute_psnr,
                self.config.compute_ssim,
                self.config.compute_vmaf,
                vmaf_model,
                cache_idx,
            ) {
                Ok(result) => {
                    record.psnr = result.psnr;
                    record.ssim = result.ssim;
                    record.vmaf = result.vmaf;
                    record.frame_count = Some(result.frame_count);
                    record.processing_time_ms = Some(result.processing_time_ms);
                    record.psnr_per_frame = result.psnr_per_frame;
                    record.ssim_per_frame = result.ssim_per_frame;
                    record.vmaf_per_frame = result.vmaf_per_frame;
                    record.status = ProcessingStatus::Completed;
                    throughput_fps = if result.processing_time_ms > 0 {
                        result.frame_count as f32 / (result.processing_time_ms as f32 / 1000.0)
                    } else {
                        0.0
                    };
                }
                Err(e) => {
                    record.status = ProcessingStatus::Failed;
                    record.error_message = Some(e);
                    throughput_fps = 0.0;
                }
            }
        } else {
            // 没有启用任何指标，只做解码测试
            let ref_config = DecoderConfig {
                use_gpu: self.config.use_gpu,
                gpu_device: self.config.gpu_device,
                max_frames: self.config.max_frames,
                ffmpeg_path: self.config.ffmpeg_path.clone(),
                ..Default::default()
            };
            let dist_config = ref_config.clone();

            let ref_decoder = VideoDecoder::new(ref_config);
            let dist_decoder = VideoDecoder::new(dist_config);

            match (
                ref_decoder.decode(&pair.ref_file.path),
                dist_decoder.decode(&dist.path),
            ) {
                (Ok((ref_frames, _)), Ok((dist_frames, _))) => {
                    let min_len = ref_frames.len().min(dist_frames.len());
                    record.frame_count = Some(min_len as u32);
                    record.status = ProcessingStatus::Completed;
                    throughput_fps = 0.0;
                }
                (Err(e), _) | (_, Err(e)) => {
                    record.status = ProcessingStatus::Failed;
                    record.error_message = Some(e);
                    throughput_fps = 0.0;
                }
            }
        }

        let elapsed = start_time.elapsed().as_secs_f32();
        if record.processing_time_ms.is_none() {
            record.processing_time_ms = Some((elapsed * 1000.0) as u64);
        }

        // 计算压缩比: 原大/压后大
        if record.dist_filesize.unwrap_or(0) > 0 {
            let ratio = (record.ref_filesize as f32) / (record.dist_filesize.unwrap() as f32);
            record.compression_ratio = Some((ratio * 100.0).round() / 100.0);
        }

        ProcessResult {
            record,
            throughput_fps,
        }
    }

    /// 并行处理多个配对 (使用 Rayon)
    pub fn process_batch(&self, pairs: &[FilePair]) -> Vec<ProcessResult> {
        // 过滤有效的配对
        let valid_pairs: Vec<_> = pairs
            .iter()
            .filter(|p| p.selected && p.distorted.is_some())
            .cloned()
            .collect();

        let total = valid_pairs.len();
        info!("开始处理 {} 个文件配对 (Rayon 多线程)", total);

        // 使用 stdin 进行并行处理，带进度跟踪
        use std::sync::atomic::{AtomicUsize, Ordering};

        let processed = AtomicUsize::new(0);
        let start_time = std::time::Instant::now();

        // Rayon 并行处理
        let results: Vec<ProcessResult> = valid_pairs
            .par_iter()
            .enumerate()
            .map(|(_idx, pair)| {
                let idx = processed.fetch_add(1, Ordering::Relaxed);
                let elapsed = start_time.elapsed().as_secs_f32();

                // 设置全局配对索引，用于 FFmpeg 进度更新
                crate::engine::set_pair_index((idx + 1) as u32, total as u32);
                // 设置预期帧数
                let expected = pair.ref_file.frame_count.unwrap_or(500);
                crate::engine::set_expected_frames(expected);

                // 打印进度
                eprintln!(
                    "[{}/{}] {} ( {:.1}s )",
                    idx + 1,
                    total,
                    pair.ref_file.name,
                    elapsed
                );

                let cache_idx = (pair.index - 1) as u32;
                self.process_pair(pair, cache_idx)
            })
            .collect();

        let completed = results
            .iter()
            .filter(|r| r.record.status == ProcessingStatus::Completed)
            .count();
        let failed = results
            .iter()
            .filter(|r| r.record.status == ProcessingStatus::Failed)
            .count();

        info!("处理完成: {} 成功, {} 失败", completed, failed);

        results
    }

    /// 并行处理多个配对 - 带进度回调 (使用 Rayon)

    /// 注意: 由于 par_iter 不保证顺序，完成回调的 current 序号不代表配对顺序，
    /// 但 total 准确，可用于进度条百分比计算
    pub fn process_batch_parallel<F>(
        &self,
        pairs: &[FilePair],
        progress_cb: F,
    ) -> Vec<ProcessResult>
    where
        F: Fn(usize, usize, &str, f32) + Send + Sync,
    {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // 过滤有效的配对，同时记录原始索引
        let valid_pairs: Vec<(usize, FilePair)> = pairs
            .iter()
            .filter(|p| p.selected && p.distorted.is_some())
            .enumerate()
            .map(|(i, p)| (i, p.clone()))
            .collect();

        let total = valid_pairs.len();
        let completed = AtomicUsize::new(0);
        let start_time = std::time::Instant::now();

        info!("开始并行处理 {} 个文件配对 (Rayon)", total);

        // 并行处理，每个 worker 返回 (idx, result)
        let mut indexed_results: Vec<(usize, ProcessResult)> = valid_pairs
            .par_iter()
            .map(|(_orig_idx, pair)| {
                let idx = completed.fetch_add(1, Ordering::SeqCst);
                let elapsed = start_time.elapsed().as_secs_f32();
                progress_cb(idx + 1, total, &pair.ref_file.name, elapsed);
                let cache_idx = (pair.index - 1) as u32;
                (idx, self.process_pair(pair, cache_idx))
            })
            .collect();

        // 按 idx 排序恢复原始顺序
        indexed_results.sort_by_key(|(idx, _)| *idx);
        let results: Vec<ProcessResult> = indexed_results.into_iter().map(|(_, r)| r).collect();

        let ok_count = results
            .iter()
            .filter(|r| r.record.status == ProcessingStatus::Completed)
            .count();
        let fail_count = results
            .iter()
            .filter(|r| r.record.status == ProcessingStatus::Failed)
            .count();

        info!("并行处理完成: {} 成功, {} 失败", ok_count, fail_count);

        results
    }

    /// 顺序处理 - 带进度回调

    /// 回调参数: (current, total, filename, elapsed_secs, frame_count, eta_secs)
    pub fn process_batch_sequential<F>(
        &self,
        pairs: &[FilePair],
        progress_cb: F,
    ) -> Vec<ProcessResult>
    where
        F: Fn(usize, usize, &str, f32, u32, f32) + Send + Sync,
    {
        let valid_pairs: Vec<_> = pairs
            .iter()
            .filter(|p| p.selected && p.distorted.is_some())
            .cloned()
            .collect();

        let valid_total = valid_pairs.len();
        let start_time = std::time::Instant::now();
        let mut results = Vec::with_capacity(valid_total);

        // 设置全局配对索引，用于 FFmpeg 进度更新
        crate::engine::set_pair_index(0, valid_total as u32);

        for (idx, pair) in valid_pairs.iter().enumerate() {
            // 更新全局配对索引
            crate::engine::set_pair_index((idx + 1) as u32, valid_total as u32);
            // 设置预期帧数 (使用扫描时获取的帧数，或默认500)
            let expected = pair.ref_file.frame_count.unwrap_or(500);
            crate::engine::set_expected_frames(expected);

            let elapsed = start_time.elapsed().as_secs_f32();

            // ETA 计算
            let eta = if idx > 0 {
                let per_item = elapsed / idx as f32;
                per_item * (valid_total - idx) as f32
            } else {
                0.0
            };

            // 取第一对结果的帧数作为估计
            let frame_count: u32 = results
                .first()
                .and_then(|r: &ProcessResult| r.record.frame_count)
                .unwrap_or(0);

            progress_cb(
                idx + 1,
                valid_total,
                &pair.ref_file.name,
                elapsed,
                frame_count,
                eta,
            );
            let cache_idx = (pair.index - 1) as u32;
            let result = self.process_pair(pair, cache_idx);
            results.push(result);
        }

        results
    }

    /// 自适应并行处理 - 根据系统资源自动选择最优并行度
    /// 回调参数: (current, total, filename, elapsed_secs, frame_count, eta_secs)
    /// 结果发送器: 可选的 channel，用于增量发送结果
    pub fn process_batch_adaptive_with_sender<F>(
        &self,
        pairs: &[FilePair],
        progress_cb: F,
        result_sender: Option<std::sync::mpsc::Sender<ComparisonRecord>>,
    ) -> Vec<ProcessResult>
    where
        F: Fn(usize, usize, &str, f32, u32, f32) + Send + Sync,
    {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let valid_pairs: Vec<_> = pairs
            .iter()
            .filter(|p| p.selected && p.distorted.is_some())
            .cloned()
            .collect();

        let valid_total = valid_pairs.len();
        if valid_total == 0 {
            return Vec::new();
        }

        // 检测系统资源，决定并行度
        let cpu_cores = num_cpus::get();
        let gpu_info = crate::engine::decoder::detect_gpu();

        // GPU 模式下并行度过高会导致显存不足，限制并发
        // CPU 模式下可以更好地利用多核
        let max_parallelism = if gpu_info.available && self.config.use_gpu {
            // GPU 模式：最多同时处理 2 个视频对，避免显存不足
            // 但如果视频数量少或帧数少，可以适当增加
            (cpu_cores / 4).max(2).min(3)
        } else {
            // CPU 模式：可以更激进地并行
            cpu_cores.min(valid_total)
        };

        info!(
            "自适应并行: CPU {} 核心, GPU {} 可用, 并行度 {}",
            cpu_cores, gpu_info.available, max_parallelism
        );

        // 清空并初始化配对进度映射
        crate::engine::clear_pair_progress_map();

        let completed = AtomicUsize::new(0);
        let start_time = std::time::Instant::now();

        // 使用 Rayon 的动态调度实现自适应并行
        let results: Vec<ProcessResult> = valid_pairs
            .par_iter()
            .with_max_len(1) // 每次只分配1个任务，实现动态调度
            .enumerate()
            .map(|(_idx, pair)| {
                let batch_idx = completed.fetch_add(1, Ordering::Relaxed);
                let elapsed = start_time.elapsed().as_secs_f32();

                // 使用 batch_idx 作为缓存键，确保与 get_all_pair_progress() 返回的索引一致
                let cache_idx = batch_idx as u32;

                // 注册配对进度
                let expected = pair.ref_file.frame_count.unwrap_or(500);
                crate::engine::register_pair_progress(cache_idx, expected);
                crate::engine::update_pair_status(
                    cache_idx,
                    crate::engine::PairProcessingStatus::Running,
                );
                crate::engine::set_cache_idx(cache_idx);

                // 更新全局配对索引
                crate::engine::set_pair_index((batch_idx + 1) as u32, valid_total as u32);
                crate::engine::set_expected_frames(expected);

                // 回调进度
                let frame_count = pair.ref_file.frame_count.unwrap_or(0);
                let eta = if batch_idx > 0 {
                    let per_item = elapsed / batch_idx as f32;
                    per_item * (valid_total - batch_idx) as f32
                } else {
                    0.0
                };
                progress_cb(
                    batch_idx + 1,
                    valid_total,
                    &pair.ref_file.name,
                    elapsed,
                    frame_count,
                    eta,
                );

                // 处理配对
                let result = self.process_pair(pair, cache_idx);

                // 更新配对状态
                let status = if result.record.status == ProcessingStatus::Completed {
                    crate::engine::PairProcessingStatus::Completed
                } else {
                    crate::engine::PairProcessingStatus::Failed
                };
                crate::engine::update_pair_status(cache_idx, status);

                // 增量发送结果到 GUI
                if let Some(ref sender) = result_sender {
                    let _ = sender.send(result.record.clone());
                }

                result
            })
            .collect();

        let ok_count = results
            .iter()
            .filter(|r| r.record.status == ProcessingStatus::Completed)
            .count();
        let fail_count = results
            .iter()
            .filter(|r| r.record.status == ProcessingStatus::Failed)
            .count();
        info!("自适应并行处理完成: {} 成功, {} 失败", ok_count, fail_count);

        results
    }
}

/// 进度信息
#[derive(Debug, Clone)]
pub struct Progress {
    pub current: usize,
    pub total: usize,
    pub current_file: String,
    pub status: ProcessingStatus,
    pub throughput_fps: f32,
    pub elapsed_secs: f32,
    pub eta_secs: Option<f32>,
}

impl Progress {
    pub fn percentage(&self) -> f32 {
        if self.total > 0 {
            (self.current as f32 / self.total as f32) * 100.0
        } else {
            0.0
        }
    }

    pub fn eta_string(&self) -> String {
        if let Some(eta) = self.eta_secs {
            if eta < 60.0 {
                format!("{:.0}s", eta)
            } else if eta < 3600.0 {
                format!("{:.1}m", eta / 60.0)
            } else {
                format!("{:.1}h", eta / 3600.0)
            }
        } else {
            "计算中...".to_string()
        }
    }

    /// 获取处理速度描述
    pub fn throughput_string(&self) -> String {
        if self.throughput_fps > 0.0 {
            format!("{:.1} 帧/秒", self.throughput_fps)
        } else {
            "—".to_string()
        }
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            current: 0,
            total: 0,
            current_file: String::new(),
            status: ProcessingStatus::Pending,
            throughput_fps: 0.0,
            elapsed_secs: 0.0,
            eta_secs: None,
        }
    }
}
