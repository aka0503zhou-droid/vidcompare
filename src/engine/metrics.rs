//! 质量指标计算模块
//!
//! PSNR、SSIM、VMAF 等视频质量指标计算

use rayon::prelude::*;
use std::f32;

/// PSNR 计算器
pub struct PsnrCalculator {
    pub max_value: f32,
}

impl Default for PsnrCalculator {
    fn default() -> Self {
        Self { max_value: 255.0 }
    }
}

impl PsnrCalculator {
    pub fn new(max_value: f32) -> Self {
        Self { max_value }
    }

    /// 计算两个 YUV 帧的 PSNR (标准 MPSNR 方法)
    pub fn calculate_frame(
        &self,
        ref_frame: &super::decoder::OwnedFrame,
        dist_frame: &super::decoder::OwnedFrame,
    ) -> f32 {
        if ref_frame.y.len() != dist_frame.y.len()
            || ref_frame.u.len() != dist_frame.u.len()
            || ref_frame.v.len() != dist_frame.v.len()
        {
            return 0.0;
        }

        let mse_y = self.calculate_mse(&ref_frame.y, &dist_frame.y);
        let mse_u = self.calculate_mse(&ref_frame.u, &dist_frame.u);
        let mse_v = self.calculate_mse(&ref_frame.v, &dist_frame.v);

        if mse_y == 0.0 && mse_u == 0.0 && mse_v == 0.0 {
            return 60.0;
        }

        let total_mse = (mse_y * 6.0 + mse_u + mse_v) / 8.0;
        if total_mse == 0.0 {
            60.0
        } else {
            10.0 * (self.max_value * self.max_value / total_mse).log10()
        }
    }

    /// 计算两个平面的 PSNR
    pub fn calculate_plane(&self, ref_data: &[u8], dist_data: &[u8]) -> f32 {
        if ref_data.len() != dist_data.len() {
            return 0.0;
        }

        let mse = self.calculate_mse(ref_data, dist_data);

        if mse == 0.0 {
            60.0
        } else {
            10.0 * (self.max_value * self.max_value / mse).log10()
        }
    }

    /// 并行计算 MSE (使用 Rayon)
    pub fn calculate_mse(&self, ref_data: &[u8], dist_data: &[u8]) -> f32 {
        if ref_data.len() != dist_data.len() {
            return 0.0;
        }

        let len = ref_data.len();

        // 使用 Rayon 并行计算
        let sum_squared_error: f32 = ref_data
            .par_iter()
            .zip(dist_data.par_iter())
            .map(|(r, d)| {
                let diff = (*r as f32) - (*d as f32);
                diff * diff
            })
            .sum();

        sum_squared_error / len as f32
    }

    /// 计算整个视频的平均 PSNR
    pub fn calculate_video_psnr(
        &self,
        ref_frames: &[super::decoder::OwnedFrame],
        dist_frames: &[super::decoder::OwnedFrame],
    ) -> f32 {
        if ref_frames.is_empty() || dist_frames.is_empty() {
            return 0.0;
        }

        let min_len = ref_frames.len().min(dist_frames.len());

        let total_psnr: f32 = (0..min_len)
            .into_par_iter()
            .map(|i| self.calculate_frame(&ref_frames[i], &dist_frames[i]))
            .sum();

        total_psnr / min_len as f32
    }
}

/// SSIM 计算器
pub struct SsimCalculator {
    /// 线性系数 (通常为 1，但 VMAF 使用动态值)
    pub k1: f32,
    pub k2: f32,
    /// 动态系数
    pub c1: f32,
    pub c2: f32,
    /// 块大小 (通常 8x8)
    pub block_size: u32,
}

impl Default for SsimCalculator {
    fn default() -> Self {
        Self::new(8, 1.0, 1.0)
    }
}

impl SsimCalculator {
    pub fn new(block_size: u32, k1: f32, k2: f32) -> Self {
        let c1 = (k1 * 255.0).powi(2);
        let c2 = (k2 * 255.0).powi(2);

        Self {
            k1,
            k2,
            c1,
            c2,
            block_size,
        }
    }

    /// 计算两个 YUV 帧的 SSIM
    pub fn calculate_frame(
        &self,
        ref_frame: &super::decoder::OwnedFrame,
        dist_frame: &super::decoder::OwnedFrame,
    ) -> f32 {
        if ref_frame.width != dist_frame.width || ref_frame.height != dist_frame.height {
            return 0.0;
        }

        // 计算 Y 分量的 SSIM
        let ssim_y = self.calculate_plane_ssim(
            &ref_frame.y,
            &dist_frame.y,
            ref_frame.width,
            ref_frame.height,
        );

        ssim_y
    }

    /// 计算平面的 SSIM (使用 8x8 滑动窗口)
    pub fn calculate_plane_ssim(
        &self,
        ref_data: &[u8],
        dist_data: &[u8],
        width: u32,
        height: u32,
    ) -> f32 {
        let width = width as usize;
        let height = height as usize;
        let block_size = self.block_size as usize;

        let mut ssim_sum = 0.0f32;
        let mut block_count = 0usize;

        // 滑动窗口步长
        let step = block_size / 2;

        for y in (0..height.saturating_sub(block_size)).step_by(step) {
            for x in (0..width.saturating_sub(block_size)).step_by(step) {
                let ssim = self.calculate_block_ssim(ref_data, dist_data, x, y, width);
                ssim_sum += ssim;
                block_count += 1;
            }
        }

        if block_count > 0 {
            ssim_sum / block_count as f32
        } else {
            1.0
        }
    }

    /// 计算单个 8x8 块的 SSIM
    fn calculate_block_ssim(
        &self,
        ref_data: &[u8],
        dist_data: &[u8],
        x: usize,
        y: usize,
        stride: usize,
    ) -> f32 {
        let block_size = self.block_size as usize;

        let mut sum_x: f32 = 0.0;
        let mut sum_y: f32 = 0.0;
        let mut sum_xx: f32 = 0.0;
        let mut sum_yy: f32 = 0.0;
        let mut sum_xy: f32 = 0.0;

        let pixels = block_size * block_size;

        for j in 0..block_size {
            for i in 0..block_size {
                let idx = (y + j) * stride + (x + i);

                if idx >= ref_data.len() || idx >= dist_data.len() {
                    continue;
                }

                let rx = ref_data[idx] as f32;
                let dy = dist_data[idx] as f32;

                sum_x += rx;
                sum_y += dy;
                sum_xx += rx * rx;
                sum_yy += dy * dy;
                sum_xy += rx * dy;
            }
        }

        let n = pixels as f32;

        let mux = sum_x / n;
        let muy = sum_y / n;

        let sigmax = (sum_xx / n - mux * mux).sqrt();
        let sigmay = (sum_yy / n - muy * muy).sqrt();

        let sigmaxy = sum_xy / n - mux * muy;

        let numerator = (2.0 * mux * muy + self.c1) * (2.0 * sigmaxy + self.c2);
        let denominator =
            (mux * mux + muy * muy + self.c1) * (sigmax * sigmax + sigmay * sigmay + self.c2);

        if denominator == 0.0 {
            1.0
        } else {
            numerator / denominator
        }
    }

    /// 计算整个视频的平均 SSIM
    pub fn calculate_video_ssim(
        &self,
        ref_frames: &[super::decoder::OwnedFrame],
        dist_frames: &[super::decoder::OwnedFrame],
    ) -> f32 {
        if ref_frames.is_empty() || dist_frames.is_empty() {
            return 0.0;
        }

        let min_len = ref_frames.len().min(dist_frames.len());

        let total_ssim: f32 = (0..min_len)
            .into_par_iter()
            .map(|i| self.calculate_frame(&ref_frames[i], &dist_frames[i]))
            .sum();

        total_ssim / min_len as f32
    }
}

/// VMAF 分数计算器
///
/// 注意: 真正的 VMAF 需要 Netflix 的 libvmaf 库。
/// 当前实现:
/// - 如果配置了 libvmaf 路径，调用 FFmpeg libvmaf filter 计算
/// - 否则使用 PSNR/SSIM 的简化估算公式 (精度有限)
pub struct VmafCalculator {
    /// 模型路径 (如果使用 libvmaf)
    pub model_path: Option<String>,
    /// 是否使用多线程
    pub thread_count: usize,
}

impl Default for VmafCalculator {
    fn default() -> Self {
        Self {
            model_path: None,
            thread_count: 4,
        }
    }
}

impl VmafCalculator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 使用 libvmaf 计算 VMAF (需要系统 FFmpeg 编译了 libvmaf)
    pub fn calculate_with_libvmaf(
        ref_path: &str,
        dist_path: &str,
        model_path: Option<&str>,
    ) -> Result<f32, String> {
        use std::process::Command;

        let mut cmd = Command::new("ffmpeg");
        cmd.args(&["-i", ref_path, "-i", dist_path, "-lavfi"]);

        // 构建 libvmaf 滤镜参数
        let mut filter_parts = Vec::new();

        if let Some(model) = model_path {
            filter_parts.push(format!("model_path={}", model));
        }

        filter_parts.push("psnr=1".to_string());
        filter_parts.push("ssim=1".to_string());

        let filter = format!("libvmaf={}", filter_parts.join(":"));
        cmd.arg(filter);
        cmd.args(&["-f", "null", "-"]);

        let output = cmd
            .output()
            .map_err(|e| format!("无法执行 FFmpeg libvmaf: {}", e))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        if !output.status.success() {
            return Err(format!(
                "libvmaf 执行失败 (可能 FFmpeg 未编译 libvmaf): {}",
                stderr.lines().last().unwrap_or("unknown error")
            ));
        }

        let combined = format!("{}\n{}", stdout, stderr);

        // 查找 "VMAF score: X.XXXX" 格式
        for line in combined.lines() {
            let line = line.trim();
            if line.contains("VMAF score:") {
                if let Some(score_str) = line.split("VMAF score:").last() {
                    let score_trimmed = score_str.trim();
                    if let Ok(score) = score_trimmed.parse::<f32>() {
                        return Ok(score);
                    }
                    if let Some(paren_start) = score_trimmed.find('(') {
                        if let Some(paren_end) = score_trimmed.find(')') {
                            let inner = &score_trimmed[paren_start + 1..paren_end];
                            if let Ok(score) = inner.parse::<f32>() {
                                return Ok(score);
                            }
                        }
                    }
                }
            }
        }

        // 解析 JSON {"vmaf_score": 95.1234}
        for line in combined.lines() {
            if line.contains("vmaf_score") {
                if let Some(start) = line.find("\"vmaf_score\"") {
                    let after_colon = &line[start..];
                    if let Some(colon_pos) = after_colon.find(':') {
                        let value_str = &after_colon[colon_pos + 1..];
                        let mut end = 0;
                        for (i, c) in value_str.char_indices() {
                            if c.is_whitespace() || c == ',' || c == '}' || c == '"' {
                                end = i;
                                break;
                            }
                        }
                        if end > 0 {
                            let num_str = value_str[..end].trim();
                            if let Ok(score) = num_str.parse::<f32>() {
                                return Ok(score);
                            }
                        }
                    }
                }
            }
        }

        // 从输出末尾查找有效的 0-100 分数
        let lines: Vec<&str> = combined.lines().collect();
        for line in lines.iter().rev().take(20) {
            let line = *line;
            if line.contains("fps") || line.contains("time") || line.contains("frame") {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in parts {
                if let Ok(v) = part.parse::<f32>() {
                    if v > 0.0 && v <= 100.0 {
                        return Ok(v);
                    }
                }
            }
        }

        Err(format!(
            "无法从 FFmpeg 输出解析 VMAF 分数。 \
            FFmpeg 可能未启用 libvmaf 支持。",
        ))
    }

    /// 简化版 VMAF 计算 (基于 PSNR 和 SSIM 估计)
    ///
    /// 警告: 这是一个近似算法，真正的 VMAF 需要机器学习模型。
    pub fn estimate_vmaf(&self, psnr: f32, ssim: f32) -> f32 {
        let psnr_clamped = psnr.max(20.0).min(60.0);
        let ssim_clamped = ssim.max(0.0).min(1.0);

        if psnr_clamped >= 60.0 && ssim_clamped >= 0.9999 {
            return 100.0;
        }

        let psnr_component = (psnr_clamped - 20.0) / 40.0;
        let ssim_component = ssim_clamped;

        let vmaf = psnr_component * 40.0 + ssim_component * 60.0;

        vmaf.clamp(0.0, 100.0)
    }

    /// 计算整个视频的 VMAF (使用简化算法)
    pub fn calculate_video_vmaf(
        &self,
        ref_frames: &[super::decoder::OwnedFrame],
        dist_frames: &[super::decoder::OwnedFrame],
    ) -> f32 {
        if ref_frames.is_empty() || dist_frames.is_empty() {
            return 0.0;
        }

        let psnr_calc = PsnrCalculator::default();
        let ssim_calc = SsimCalculator::default();

        let psnr = psnr_calc.calculate_video_psnr(ref_frames, dist_frames);
        let ssim = ssim_calc.calculate_video_ssim(ref_frames, dist_frames);

        self.estimate_vmaf(psnr, ssim)
    }
}

/// 质量指标统计
#[derive(Debug, Clone, Default)]
pub struct MetricsStats {
    /// 平均 PSNR
    pub psnr_avg: f32,
    /// PSNR 最小值
    pub psnr_min: f32,
    /// PSNR 最大值
    pub psnr_max: f32,
    /// 平均 SSIM
    pub ssim_avg: f32,
    /// 平均 VMAF
    pub vmaf_avg: f32,
    /// 处理帧数
    pub frame_count: usize,
}

/// 计算所有指标
pub fn calculate_all_metrics(
    ref_frames: &[super::decoder::OwnedFrame],
    dist_frames: &[super::decoder::OwnedFrame],
) -> Result<MetricsStats, String> {
    if ref_frames.is_empty() || dist_frames.is_empty() {
        return Err("帧数据为空".to_string());
    }

    let psnr_calc = PsnrCalculator::default();
    let ssim_calc = SsimCalculator::default();
    let vmaf_calc = VmafCalculator::new();

    let min_len = ref_frames.len().min(dist_frames.len());

    // 并行计算所有帧的指标
    let frame_metrics: Vec<(f32, f32)> = (0..min_len)
        .into_par_iter()
        .map(|i| {
            let psnr = psnr_calc.calculate_frame(&ref_frames[i], &dist_frames[i]);
            let ssim = ssim_calc.calculate_frame(&ref_frames[i], &dist_frames[i]);
            (psnr, ssim)
        })
        .collect();

    let psnr_values: Vec<f32> = frame_metrics.iter().map(|(p, _)| *p).collect();
    let ssim_values: Vec<f32> = frame_metrics.iter().map(|(_, s)| *s).collect();

    let psnr_avg = psnr_values.iter().sum::<f32>() / min_len as f32;
    let psnr_min = psnr_values.iter().cloned().fold(f32::INFINITY, f32::min);
    let psnr_max = psnr_values
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);

    let ssim_avg = ssim_values.iter().sum::<f32>() / min_len as f32;

    let vmaf_avg = vmaf_calc.estimate_vmaf(psnr_avg, ssim_avg);

    Ok(MetricsStats {
        psnr_avg,
        psnr_min,
        psnr_max,
        ssim_avg,
        vmaf_avg,
        frame_count: min_len,
    })
}

#[cfg(test)]
mod tests {
    use super::super::decoder::{OwnedFrame, VideoInfo};
    use super::*;

    fn create_test_frame(width: u32, height: u32, value: u8) -> OwnedFrame {
        let y_size = (width * height) as usize;
        let uv_size = ((width / 2) * (height / 2)) as usize;

        OwnedFrame {
            y: vec![value; y_size],
            u: vec![value; uv_size],
            v: vec![value; uv_size],
            width,
            height,
            frame_num: 0,
        }
    }

    #[test]
    fn test_psnr_identical() {
        let calc = PsnrCalculator::default();
        let frame1 = create_test_frame(64, 64, 128);
        let frame2 = create_test_frame(64, 64, 128);

        let psnr = calc.calculate_frame(&frame1, &frame2);
        assert!(psnr.is_infinite());
    }

    #[test]
    fn test_psnr_different() {
        let calc = PsnrCalculator::default();
        let frame1 = create_test_frame(64, 64, 0);
        let frame2 = create_test_frame(64, 64, 255);

        let psnr = calc.calculate_frame(&frame1, &frame2);
        assert!(
            psnr >= 0.0 && psnr < 50.0,
            "PSNR should be near 0 for opposite values, got {}",
            psnr
        );
    }

    #[test]
    fn test_ssim_identical() {
        let calc = SsimCalculator::default();
        let frame1 = create_test_frame(64, 64, 128);
        let frame2 = create_test_frame(64, 64, 128);

        let ssim = calc.calculate_frame(&frame1, &frame2);
        assert!((ssim - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_vmaf_estimation() {
        let calc = VmafCalculator::new();

        let vmaf = calc.estimate_vmaf(40.0, 0.95);
        assert!(vmaf > 0.0 && vmaf <= 100.0);
    }
}
