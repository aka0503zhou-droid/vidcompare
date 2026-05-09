//! FFmpeg 过滤器计算模块
//!
//! 使用 FFmpeg 内置的 psnr/ssim/libvmaf 过滤器进行高性能计算
//! 这些过滤器用 C 编写，比 Rust 实现快很多


use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::{debug, info};

/// 全局进度原子变量 (用于线程间通信)
static PROGRESS_FRAME: AtomicU32 = AtomicU32::new(0);
static PAIR_INDEX: AtomicU32 = AtomicU32::new(0);
static PAIR_TOTAL: AtomicU32 = AtomicU32::new(0);
/// 当前视频的预期总帧数 (用于计算帧级进度百分比)
static EXPECTED_FRAMES: AtomicU32 = AtomicU32::new(0);
/// 当前配对的缓存索引 (pair.index - 1) - 注意：这个值在多线程环境下会被覆盖！
static CURRENT_CACHE_IDX: AtomicU32 = AtomicU32::new(0);

/// 获取当前帧进度 (返回: pair_idx, pair_total, current_frame, expected_frames)
pub fn get_progress() -> (u32, u32, u32, u32) {
    let pair = PAIR_INDEX.load(Ordering::Relaxed);
    let pair_total = PAIR_TOTAL.load(Ordering::Relaxed);
    let frame = PROGRESS_FRAME.load(Ordering::Relaxed);
    let expected = EXPECTED_FRAMES.load(Ordering::Relaxed);
    (pair, pair_total, frame, expected)
}

/// 设置当前配对索引 (batch_idx 1-based, total)
pub fn set_pair_index(idx: u32, total: u32) {
    PAIR_INDEX.store(idx, Ordering::Relaxed);
    PAIR_TOTAL.store(total, Ordering::Relaxed);
    PROGRESS_FRAME.store(0, Ordering::Relaxed);
}

/// 设置当前配对的缓存索引 (用于帧进度更新)
/// 注意：此函数应该在任务开始前调用，在任务结束前不应再调用其他任务
#[allow(dead_code)]
pub fn set_cache_idx(cache_idx: u32) {
    CURRENT_CACHE_IDX.store(cache_idx, Ordering::Relaxed);
}

/// 设置当前视频的预期总帧数
pub fn set_expected_frames(frames: u32) {
    EXPECTED_FRAMES.store(frames, Ordering::Relaxed);
}

/// FFmpeg 过滤器计算结果
#[derive(Debug, Clone)]
pub struct FilterResult {
    pub psnr: Option<f32>,
    pub ssim: Option<f32>,
    pub vmaf: Option<f32>,
    pub frame_count: u32,
    pub processing_time_ms: u64,
    /// 每帧 PSNR 数据 (用于图表)
    pub psnr_per_frame: Vec<f32>,
    /// 每帧 SSIM 数据 (用于图表)
    pub ssim_per_frame: Vec<f32>,
    /// 每帧 VMAF 数据 (用于图表)
    pub vmaf_per_frame: Vec<f32>,
}

/// 同时计算多个指标 (使用 FFmpeg 的 psnr+ssim+vmaf 组合)
///
/// 通过全局原子变量报告进度: pair_index, pair_total, frame
/// cache_idx 用于帧进度更新（传入而非全局变量，避免多线程覆盖）
pub fn calculate_all_metrics_ffmpeg_with_progress(
    ref_path: &Path,
    dist_path: &Path,
    ffmpeg_path: Option<&Path>,
    compute_psnr: bool,
    compute_ssim: bool,
    compute_vmaf: bool,
    _vmaf_model_path: Option<&Path>,
    cache_idx: u32,
) -> Result<FilterResult, String> {
    let start_time = std::time::Instant::now();
    let exe = ffmpeg_path
        .map(|p| p.as_os_str())
        .unwrap_or_else(|| std::ffi::OsStr::new("ffmpeg"));

    // GPU 检测
    let gpu_info = crate::engine::decoder::detect_gpu();
    let use_gpu = gpu_info.available;

    // 构建 filter graph
    // 每个指标使用独立的 split 和输出标签
    // 注意: libvmaf 不使用 log=1，避免写入文件导致权限错误
    let filter_str = match (compute_psnr, compute_ssim, compute_vmaf) {
        (true, true, true) => {
            "[0:v]split=3[ref1][ref2][ref3];[1:v]split=3[dist1][dist2][dist3];[ref1][dist1]psnr[psnr];[ref2][dist2]ssim[ssim];[ref3][dist3]libvmaf[vmaf]"
        }
        (true, true, false) => {
            "[0:v]split=2[ref1][ref2];[1:v]split=2[dist1][dist2];[ref1][dist1]psnr[psnr];[ref2][dist2]ssim[ssim]"
        }
        (true, false, true) => {
            "[0:v]split=2[ref1][ref2];[1:v]split=2[dist1][dist2];[ref1][dist1]psnr[psnr];[ref2][dist2]libvmaf[vmaf]"
        }
        (false, true, true) => {
            "[0:v]split=2[ref1][ref2];[1:v]split=2[dist1][dist2];[ref1][dist1]ssim[ssim];[ref2][dist2]libvmaf[vmaf]"
        }
        (true, false, false) => {
            "[0:v][1:v]psnr"
        }
        (false, true, false) => {
            "[0:v][1:v]ssim"
        }
        (false, false, true) => {
            "[0:v][1:v]libvmaf"
        }
        _ => {
            return Err("No metrics to compute".to_string());
        }
    };

    let mut cmd = Command::new(exe);

    // GPU 加速选项 - 只启用 cuvid/nvenc 解码，不改变帧格式
    if use_gpu {
        match &gpu_info.backend {
            crate::engine::decoder::GpuBackend::Cuda => {
                cmd.args(&["-hwaccel", "cuda"]);
            }
            crate::engine::decoder::GpuBackend::D3d11va => {
                cmd.args(&["-hwaccel", "d3d11va"]);
            }
            crate::engine::decoder::GpuBackend::Vaapi => {
                cmd.args(&["-hwaccel", "vaapi"]);
            }
            crate::engine::decoder::GpuBackend::None => {}
        }
        info!("Using GPU acceleration: {:?}", gpu_info.backend);
    }

    cmd.args(&["-hide_banner"])
        .args(&["-i", ref_path.to_str().unwrap_or("")])
        .args(&["-i", dist_path.to_str().unwrap_or("")])
        .args(&["-filter_complex", &filter_str]);

    // 根据启用的指标添加 map 参数
    match (compute_psnr, compute_ssim, compute_vmaf) {
        (true, true, true) => {
            cmd.args(&["-map", "[psnr]"])
                .args(&["-map", "[ssim]"])
                .args(&["-map", "[vmaf]"]);
        }
        (true, true, false) => {
            cmd.args(&["-map", "[psnr]"]).args(&["-map", "[ssim]"]);
        }
        (true, false, true) => {
            cmd.args(&["-map", "[psnr]"]).args(&["-map", "[vmaf]"]);
        }
        (false, true, true) => {
            cmd.args(&["-map", "[ssim]"]).args(&["-map", "[vmaf]"]);
        }
        (true, false, false) => {
            cmd.args(&["-map", "0:v"]);
        }
        (false, true, false) => {
            cmd.args(&["-map", "0:v"]);
        }
        (false, false, true) => {
            cmd.args(&["-map", "0:v"]);
        }
        _ => {}
    }

    cmd.args(&["-f", "null", "-"])
        .args(&["-progress", "pipe:1"])  // 输出进度到 stdout
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    info!("Executing FFmpeg metrics (GPU: {}): {:?}", use_gpu, cmd);

    // 使用 spawn 启动进程
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn FFmpeg: {}", e))?;

    let stdout = child.stdout.take().expect("Failed to capture stdout");

    // 使用单独的线程读取 stdout 的进度输出 (-progress)
    use std::io::BufRead;
    let stdout_thread = std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            // -progress 输出格式: frame=123
            if let Some(frame_str) = line.strip_prefix("frame=") {
                if let Ok(frame) = frame_str.trim().parse::<u32>() {
                    PROGRESS_FRAME.store(frame, Ordering::Relaxed);
                    crate::engine::update_pair_frame(cache_idx, frame);
                }
            }
        }
    });

    // 存储解析结果
    let mut last_frame = 0u32;
    let mut total_psnr = 0.0;
    let mut psnr_count = 0u32;
    let mut total_ssim = 0.0;
    let mut ssim_count = 0u32;
    let mut total_vmaf = 0.0;
    let mut vmaf_count = 0u32;
    let mut psnr_per_frame: Vec<f32> = Vec::new();
    let mut ssim_per_frame: Vec<f32> = Vec::new();
    let mut vmaf_per_frame: Vec<f32> = Vec::new();

    // 等待进程结束
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait for FFmpeg: {}", e))?;

    // 等待 stdout 进度线程结束
    let _ = stdout_thread.join();

    // 从 output 中再提取一些可能遗漏的数据
    let additional_stderr = String::from_utf8_lossy(&output.stderr);
    for line in additional_stderr.lines() {
        // 解析帧数: frame=123
        if let Some(frame_str) = line.strip_prefix("frame=") {
            if let Ok(frame) = frame_str.trim().parse::<u32>() {
                last_frame = frame;
                // 更新全局进度
                PROGRESS_FRAME.store(frame, Ordering::Relaxed);
            }
        }

        // 解析 PSNR 逐帧输出: [Parsed_psnr_0 @ address] PSNR y:xx u:xx v:xx average:xx
        if line.contains("PSNR") {
            debug!("PSNR line: {}", line);
            for part in line.split_whitespace() {
                if part.starts_with("average:") {
                    if let Ok(val) = part[8..].parse::<f32>() {
                        total_psnr += val;
                        psnr_count += 1;
                        psnr_per_frame.push(val);
                    }
                }
            }
        }

        // 解析 SSIM 逐帧输出: [Parsed_ssim_0 @ address] SSIM Y:xx (xx) U:xx (xx) V:xx (xx) All:xx (xx)
        if line.contains("SSIM") {
            debug!("SSIM line: {}", line);
            for part in line.split_whitespace() {
                // SSIM 输出格式是 All:0.990668 (20.300359) - 需要提取 All: 后面的数字
                if part.starts_with("All:") {
                    let val_str = part.trim_start_matches("All:");
                    // 值可能带有 (xx) 后缀，如 "0.990668 (20.300359)"
                    if let Some(paren_idx) = val_str.find('(') {
                        if let Ok(val) = val_str[..paren_idx].trim().parse::<f32>() {
                            total_ssim += val;
                            ssim_count += 1;
                            ssim_per_frame.push(val);
                        }
                    } else if let Ok(val) = val_str.parse::<f32>() {
                        total_ssim += val;
                        ssim_count += 1;
                        ssim_per_frame.push(val);
                    }
                }
            }
        }

        // 解析 VMAF 逐帧输出: [Parsed_libvmaf_0 @ address] VMAF score: xx.xxxxxx
        if line.contains("VMAF score:") {
            info!("VMAF line detected: {}", line);
            // 提取 "VMAF score:" 后面的数字部分
            if let Some(score_start) = line.find("VMAF score:") {
                let after_label = &line[score_start + 12..]; // 12 = len("VMAF score:")
                let trimmed = after_label.trim();
                // 提取数字部分（可能以空格、制表符、换行结尾）
                let mut end_idx = trimmed.len();
                for (i, c) in trimmed.char_indices() {
                    if c.is_whitespace() || c == '\n' || c == '\r' {
                        end_idx = i;
                        break;
                    }
                }
                let num_str = &trimmed[..end_idx];
                info!("VMAF parsing: num_str='{}'", num_str);
                if let Ok(score) = num_str.parse::<f32>() {
                    total_vmaf += score;
                    vmaf_count += 1;
                    vmaf_per_frame.push(score);
                    info!("VMAF parsed: {} (count={})", score, vmaf_count);
                } else {
                    info!("VMAF parse failed for: {}", num_str);
                }
            }
        }
    }

    let elapsed = start_time.elapsed();

    // 先检查是否已成功收集到数据
    let has_psnr = psnr_count > 0;
    let has_ssim = ssim_count > 0;
    let has_vmaf = vmaf_count > 0;

    // 如果没有收集到任何数据，且进程退出失败，才报错
    // 如果已收集到数据，即使进程退出码非零，也使用收集到的数据
    if !has_psnr && !has_ssim && !has_vmaf && !output.status.success() {
        // 截断过长错误信息，保留前 2000 字符
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        let truncated = if stderr_str.len() > 2000 {
            format!(
                "{}...[truncated {} chars]",
                &stderr_str[..2000],
                stderr_str.len() - 2000
            )
        } else {
            stderr_str.to_string()
        };
        return Err(format!(
            "FFmpeg metrics failed (exit {}): {}",
            output.status, truncated
        ));
    }

    let mut result = FilterResult {
        psnr: None,
        ssim: None,
        vmaf: None,
        frame_count: last_frame,
        processing_time_ms: (elapsed.as_secs_f64() * 1000.0) as u64,
        psnr_per_frame,
        ssim_per_frame,
        vmaf_per_frame,
    };

    // 计算平均值
    debug!("Collected PSNR: count={}, total={}", psnr_count, total_psnr);
    debug!("Collected SSIM: count={}, total={}", ssim_count, total_ssim);
    debug!("Collected VMAF: count={}, total={}", vmaf_count, total_vmaf);

    if has_psnr {
        result.psnr = Some(total_psnr / psnr_count as f32);
    }
    if has_ssim {
        result.ssim = Some(total_ssim / ssim_count as f32);
    }
    if has_vmaf {
        result.vmaf = Some(total_vmaf / vmaf_count as f32);
    }

    debug!(
        "Final result: psnr={:?}, ssim={:?}, vmaf={:?}",
        result.psnr, result.ssim, result.vmaf
    );

    // 清理进度
    PROGRESS_FRAME.store(0, Ordering::Relaxed);

    Ok(result)
}
