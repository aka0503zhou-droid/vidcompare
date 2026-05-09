//! 目录扫描模块
//!
//! 扫描目录中的视频文件 - 分离快速扫描和信息探测

use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator, IndexedParallelIterator};
use super::{VideoFile, VideoDecoder, set_scan_progress, reset_scan_progress, mark_scan_done, set_total_files, increment_processed, set_current_file};
use crate::config::VIDEO_EXTENSIONS;

/// 快速扫描目录 (只获取文件名和大小，不探测视频信息)
/// 这是快速的瞬间操作
pub fn fast_scan_directory(dir: &Path) -> Result<Vec<VideoFile>, Box<dyn std::error::Error + Send + Sync>> {
    if !dir.exists() {
        return Err(format!("目录不存在: {}", dir.display()).into());
    }

    reset_scan_progress();

    // 只快速遍历文件列表，不做任何探测
    let entries: Vec<_> = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            if e.path().is_dir() {
                return false;
            }
            if let Some(ext) = e.path().extension() {
                let ext_lower = ext.to_string_lossy().to_lowercase();
                return VIDEO_EXTENSIONS.contains(&ext_lower.as_str());
            }
            false
        })
        .collect();

    let total = entries.len();
    set_scan_progress("扫描文件...", 0, total);

    let mut videos = Vec::new();
    for (idx, entry) in entries.iter().enumerate() {
        let path = entry.path();
        let name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        set_scan_progress(&name, idx + 1, total);

        if let Ok(metadata) = entry.metadata() {
            let size = metadata.len();
            // 只设置基本信息，不探测视频信息
            videos.push(VideoFile::new(name, path.to_path_buf(), size));
        }
    }

    videos.sort_by(|a, b| a.name.cmp(&b.name));
    mark_scan_done();
    Ok(videos)
}

/// 探测视频文件信息 (耗时操作，应该在后台执行)
/// 使用给定的视频文件列表，更新它们的详细信息
pub fn probe_videos_in_pairs(pairs: &mut [super::FilePair]) {
    // 收集所有需要探测的文件，使用索引确保顺序
    // 索引映射: (pair_idx, is_ref) -> result_idx
    // result_idx = pair_idx * 2 + (if is_ref then 0 else 1)
    let mut all_entries: Vec<(usize, bool, PathBuf)> = Vec::new();

    for (pair_idx, pair) in pairs.iter().enumerate() {
        all_entries.push((pair_idx, true, pair.ref_file.path.clone()));
        all_entries.push((pair_idx, false, pair.dist_file.path.clone()));
    }

    let total = all_entries.len();
    if total == 0 {
        return;
    }

    // 自适应线程数：使用 CPU 核心数，但不超过文件数量
    let cpu_cores = num_cpus::get();
    let num_threads = cpu_cores.min(total).max(1);

    tracing::info!("开始探测 {} 个视频文件，使用 {} 线程", total, num_threads);

    // 设置总文件数（只设置一次）
    set_total_files(total);

    // 并行探测所有视频文件，保留索引信息
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

    // 结果: (result_idx, VideoFile)
    let mut results: Vec<(usize, VideoFile)> = pool.install(|| {
        all_entries
            .par_iter()
            .enumerate()
            .filter_map(|(_result_idx, (pair_idx, is_ref, path))| {
                let _processed = increment_processed();
                let filename = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                set_current_file(&filename);

                let info = VideoDecoder::probe(path);
                let mut video = VideoFile::new(
                    filename,
                    path.clone(),
                    0,
                );

                if let Ok(info) = info {
                    video.bitrate = Some(info.bitrate);
                    video.duration_ms = Some(info.duration_ms);
                    video.width = Some(info.width);
                    video.height = Some(info.height);
                    video.codec = Some(info.codec);
                    video.frame_count = Some(info.frame_count);
                }

                // result_idx = pair_idx * 2 + (if is_ref then 0 else 1)
                let result_idx = pair_idx * 2 + if *is_ref { 0 } else { 1 };
                Some((result_idx, video))
            })
            .collect()
    });

    // 按 result_idx 排序，确保顺序正确
    results.sort_by_key(|(idx, _)| *idx);

    // 将探测结果填回 pairs
    let mut result_iter = results.into_iter();
    for pair in pairs.iter_mut() {
        // ref 文件 (result_idx 应该是偶数)
        if let Some((_, ref_info)) = result_iter.next() {
            pair.ref_file.bitrate = ref_info.bitrate;
            pair.ref_file.duration_ms = ref_info.duration_ms;
            pair.ref_file.width = ref_info.width;
            pair.ref_file.height = ref_info.height;
            pair.ref_file.codec = ref_info.codec;
            pair.ref_file.frame_count = ref_info.frame_count;
        }

        // dist 文件 (result_idx 应该是奇数)
        if let Some((_, dist_info)) = result_iter.next() {
            pair.dist_file.bitrate = dist_info.bitrate;
            pair.dist_file.duration_ms = dist_info.duration_ms;
            pair.dist_file.width = dist_info.width;
            pair.dist_file.height = dist_info.height;
            pair.dist_file.codec = dist_info.codec;
            pair.dist_file.frame_count = dist_info.frame_count;
        }
    }

    tracing::info!("视频信息探测完成");
    mark_scan_done();
}