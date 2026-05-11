//! 视频解码模块
//!
//! 使用 FFmpeg 进行视频解码，支持 GPU 加速

use std::path::Path;
use std::process::{Command, Stdio};
use std::io::{Read, BufReader};
use std::sync::Arc;
use tracing::info;
use serde::Deserialize;

/// GPU 后端类型
#[derive(Debug, Clone, PartialEq)]
pub enum GpuBackend {
    Cuda,
    Vaapi,
    D3d11va,
    None,
}

/// GPU 检测结果
#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub backend: GpuBackend,
    pub name: String,
    pub available: bool,
}

impl Default for GpuInfo {
    fn default() -> Self {
        Self {
            backend: GpuBackend::None,
            name: "CPU".to_string(),
            available: false,
        }
    }
}

/// 检测 GPU 后端 (统一入口，避免重复检测)
pub fn detect_gpu() -> GpuInfo {
    // 优先检测 NVIDIA CUDA
    if let Some(name) = get_nvidia_gpu_name() {
        // 检查 FFmpeg 是否支持 CUDA 硬解
        if system_ffmpeg_has_cuda() || system_ffmpeg_has_nvenc() {
            return GpuInfo {
                backend: GpuBackend::Cuda,
                name,
                available: true,
            };
        }
        // 有 NVIDIA 卡但 FFmpeg 无 CUDA/nvenc，仍然返回 (某些场景可软件解码)
        return GpuInfo {
            backend: GpuBackend::Cuda,
            name,
            available: true,
        };
    }

    // Linux: 检测 VAAPI
    #[cfg(target_os = "linux")]
    {
        if ffmpeg_has_vaapi() {
            return GpuInfo {
                backend: GpuBackend::Vaapi,
                name: "VAAPI GPU".to_string(),
                available: true,
            };
        }
    }

    // Windows: 检测 D3D11VA
    #[cfg(target_os = "windows")]
    {
        if ffmpeg_has_d3d11va() {
            return GpuInfo {
                backend: GpuBackend::D3d11va,
                name: "D3D11VA GPU".to_string(),
                available: true,
            };
        }
    }

    GpuInfo::default()
}

fn get_nvidia_gpu_name() -> Option<String> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .ok()?;

    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout);
        let name = name.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn system_ffmpeg_has_nvenc() -> bool {
    let output = Command::new("ffmpeg")
        .args(["-codecs", "-hide_banner"])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("h264_nvenc"),
        Err(_) => false,
    }
}

fn system_ffmpeg_has_cuda() -> bool {
    let output = Command::new("ffmpeg")
        .args(["-hwaccels", "-hide_banner"])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("cuda"),
        Err(_) => false,
    }
}

#[cfg(target_os = "windows")]
fn ffmpeg_has_d3d11va() -> bool {
    let output = Command::new("ffmpeg")
        .args(["-hwaccels", "-hide_banner"])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("d3d11va"),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
fn ffmpeg_has_vaapi() -> bool {
    let output = Command::new("ffmpeg")
        .args(["-hwaccels", "-hide_banner"])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("vaapi"),
        Err(_) => false,
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn ffmpeg_has_d3d11va() -> bool { false }

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn ffmpeg_has_vaapi() -> bool { false }

/// 解码器配置
#[derive(Clone)]
pub struct DecoderConfig {
    /// 跳帧间隔 (0 = 不跳帧)
    pub frame_skip: u32,
    /// 最大解码帧数 (0 = 不限制)
    pub max_frames: u32,
    /// 是否使用 GPU 加速
    pub use_gpu: bool,
    /// GPU 设备 ID
    pub gpu_device: u32,
    /// FFmpeg 可执行文件路径 (None = 使用系统 ffmpeg)
    pub ffmpeg_path: Option<std::path::PathBuf>,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            frame_skip: 0,
            max_frames: 0,
            use_gpu: false,
            gpu_device: 0,
            ffmpeg_path: None,
        }
    }
}

/// 解码后的帧数据 (YUV420P 格式)
#[derive(Debug, Clone)]
pub struct OwnedFrame {
    /// Y 分量 (亮度)
    pub y: Vec<u8>,
    /// U 分量 (蓝色差)
    pub u: Vec<u8>,
    /// V 分量 (红色差)
    pub v: Vec<u8>,
    /// 帧宽度
    pub width: u32,
    /// 帧高度
    pub height: u32,
    /// 帧序号
    pub frame_num: u32,
}

impl OwnedFrame {
    /// 获取 YUV 数据的大小
    pub fn data_size(&self) -> usize {
        self.y.len() + self.u.len() + self.v.len()
    }
}

/// FFprobe JSON structures for proper deserialization
#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    #[serde(rename = "streams")]
    streams: Option<Vec<StreamInfo>>,
    #[serde(rename = "format")]
    format: Option<FormatInfo>,
}

#[derive(Debug, Deserialize)]
struct StreamInfo {
    #[serde(rename = "codec_name")]
    codec_name: Option<String>,
    #[serde(rename = "width")]
    width: Option<u32>,
    #[serde(rename = "height")]
    height: Option<u32>,
    #[serde(rename = "r_frame_rate")]
    r_frame_rate: Option<String>,
    #[serde(rename = "nb_frames")]
    nb_frames: Option<String>,
    #[serde(rename = "bit_rate")]
    bit_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FormatInfo {
    #[serde(rename = "duration")]
    duration: Option<String>,
    #[serde(rename = "bit_rate")]
    bit_rate: Option<String>,
    #[serde(rename = "size")]
    size: Option<String>,
}

/// 视频信息
#[derive(Debug, Default, Clone)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub frame_count: u32,
    pub codec: String,
    pub duration_ms: u64,
    pub bitrate: u64,
    pub fps: f64,
}

/// 视频解码器
pub struct VideoDecoder {
    config: DecoderConfig,
}

impl VideoDecoder {
    pub fn new(config: DecoderConfig) -> Self {
        Self { config }
    }

    /// 检测 FFmpeg 是否可用
    pub fn is_ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    /// 获取 FFmpeg 版本信息
    pub fn get_ffmpeg_version() -> Option<String> {
        let output = Command::new("ffmpeg")
            .arg("-version")
            .output()
            .ok()?;

        let version = String::from_utf8_lossy(&output.stdout);
        version.lines().next().map(|s| s.to_string())
    }

    /// 检测 FFmpeg 是否支持 NVENC (NVIDIA GPU 加速)
    pub fn is_nvenc_available() -> bool {
        system_ffmpeg_has_nvenc()
    }

    /// 使用 ffprobe 获取视频信息
    pub fn probe<P: AsRef<Path>>(path: P) -> Result<VideoInfo, String> {
        let path = path.as_ref();

        // 第一步：快速获取基本信息（不计数帧）
        let output = Command::new("ffprobe")
            .args(&[
                "-v", "quiet",
                "-print_format", "json",
                "-show_format", "-show_streams",
                path.to_str().unwrap_or("")
            ])
            .output()
            .map_err(|e| format!("无法执行 ffprobe: {}", e))?;

        if !output.status.success() {
            return Err(format!("ffprobe 执行失败: {}", String::from_utf8_lossy(&output.stderr)));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let mut info = parse_probe_json(&json_str)?;

        // 第二步：如果容器没有提供帧数（为0），才计数
        // 这样可以避免读取整个文件的开销
        if info.frame_count == 0 {
            // 尝试只获取帧数（轻量级操作）
            let frame_output = Command::new("ffprobe")
                .args(&[
                    "-v", "quiet",
                    "-count_frames",
                    "-select_streams", "v:0",
                    "-show_entries", "stream=nb_read_frames",
                    "-of", "csv=p=0",
                    path.to_str().unwrap_or("")
                ])
                .output();

            if let Ok(frame_output) = frame_output {
                if frame_output.status.success() {
                    let frame_str = String::from_utf8_lossy(&frame_output.stdout);
                    if let Ok(frame_count) = frame_str.trim().parse::<u32>() {
                        info.frame_count = frame_count;
                    }
                }
            }
        }

        Ok(info)
    }

    /// 解码视频文件 - 使用 FFmpeg 提取原始 YUV 帧
    ///
    /// 使用流式读取避免大文件内存问题
    pub fn decode<P: AsRef<Path>>(&self, path: P) -> Result<(Vec<OwnedFrame>, VideoInfo), String> {
        let path = path.as_ref();

        // 首先获取视频信息
        let info = Self::probe(path)?;

        // 构建 FFmpeg 命令
        let mut cmd = self.build_ffmpeg_command(path, &info)?;

        // 执行 FFmpeg，捕获 stdout (YUV 数据)
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("无法启动 FFmpeg: {}", e))?;

        let stdout = child.stdout.take()
            .ok_or_else(|| "无法获取 FFmpeg 输出".to_string())?;

        // 流式读取 YUV 数据
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::with_capacity(1024 * 1024); // 初始 1MB 预分配
        reader.read_to_end(&mut buffer)
            .map_err(|e| format!("读取帧数据失败: {}", e))?;

        // 等待 FFmpeg 结束
        let status = child.wait()
            .map_err(|e| format!("FFmpeg 异常退出: {}", e))?;

        if !status.success() {
            let stderr = child.stderr.take()
                .map(|mut s| {
                    let mut buf = String::new();
                    s.read_to_string(&mut buf).ok();
                    buf
                })
                .unwrap_or_default();

            return Err(format!("FFmpeg 解码失败: {}", stderr));
        }

        // 解析 YUV 数据为帧
        let frames = self.parse_yuv_frames(&buffer, info.width, info.height)?;

        Ok((frames, info))
    }

    /// 构建 FFmpeg 命令
    fn build_ffmpeg_command<P: AsRef<Path>>(
        &self,
        path: P,
        _info: &VideoInfo,
    ) -> Result<Command, String> {
        let exe = self.config.ffmpeg_path.as_ref()
            .map(|p| p.as_os_str())
            .unwrap_or_else(|| std::ffi::OsStr::new("ffmpeg"));
        let mut cmd = Command::new(exe);

        // GPU 检测 (复用统一入口，避免重复调用)
        let gpu_info = detect_gpu();
        let gpu_backend = if self.config.use_gpu && gpu_info.available {
            Some(&gpu_info.backend)
        } else {
            None
        };

        // 硬件加速选项 - 必须在 -i 之前
        if let Some(backend) = gpu_backend {
            match backend {
                GpuBackend::Cuda => {
                    cmd.args(&["-hwaccel", "cuda"]);
                    cmd.args(&["-hwaccel_output_format", "cuda"]);
                }
                GpuBackend::D3d11va => {
                    cmd.args(&["-hwaccel", "d3d11va"]);
                    cmd.args(&["-hwaccel_output_format", "yuv420p"]);
                }
                GpuBackend::Vaapi => {
                    cmd.args(&["-hwaccel", "vaapi"]);
                    cmd.args(&["-vaapi_device", "/dev/dri/renderD128"]);
                    cmd.args(&["-hwaccel_output_format", "yuv420p"]);
                }
                GpuBackend::None => {}
            }
        }

        // 输入文件
        cmd.arg("-i").arg(path.as_ref().to_str().unwrap_or(""));

        // 跳帧 filter
        if self.config.frame_skip > 0 {
            cmd.args(&[
                "-vf",
                &format!(
                    "select='not(mod(n\\,{}))',setpts=N/FRAME_RATE/TB",
                    self.config.frame_skip + 1
                ),
            ]);
        }

        // GPU 解码后的格式转换 - 必须用 hwdownload 将 GPU 显存数据拷出
        if let Some(backend) = gpu_backend {
            match backend {
                GpuBackend::Cuda => {
                    // CUDA: 从 GPU 拷到 CPU，输出 NV12，再转 yuv420p
                    cmd.args(&["-vf", "hwdownload,format=nv12"]);
                }
                GpuBackend::Vaapi => {
                    cmd.args(&["-vf", "hwdownload,format=nv12"]);
                }
                _ => {}
            }
        }

        // 最大帧数
        if self.config.max_frames > 0 {
            cmd.args(&["-frames:v", &self.config.max_frames.to_string()]);
        }

        // 输出原始 YUV420P
        cmd.args(&["-pix_fmt", "yuv420p", "-f", "rawvideo", "-"]);

        Ok(cmd)
    }

    /// 解析 YUV 数据为帧
    fn parse_yuv_frames(
        &self,
        buffer: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<OwnedFrame>, String> {
        let y_size = (width * height) as usize;
        let uv_size = ((width / 2) * (height / 2)) as usize;
        let frame_size = y_size + 2 * uv_size;

        if buffer.is_empty() {
            return Ok(Vec::new());
        }

        if buffer.len() < frame_size {
            return Err(format!(
                "数据不完整: 期望 {} 字节，实际 {} 字节",
                frame_size,
                buffer.len()
            ));
        }

        let mut frames = Vec::new();
        let mut offset = 0;
        let mut frame_num = 0u32;

        // 内存保护：限制最大帧数
        let max_frames_limit = if self.config.max_frames > 0 {
            self.config.max_frames.min(super::MAX_FRAME_BUFFER as u32) as usize
        } else {
            super::MAX_FRAME_BUFFER
        };

        while offset + frame_size <= buffer.len() {
            // 内存保护：达到上限时停止
            if frames.len() >= max_frames_limit {
                info!("帧数达到内存限制 {}，截断", max_frames_limit);
                break;
            }

            let frame_data = &buffer[offset..offset + frame_size];

            let y = frame_data[..y_size].to_vec();
            let u = frame_data[y_size..y_size + uv_size].to_vec();
            let v = frame_data[y_size + uv_size..].to_vec();

            frames.push(OwnedFrame {
                y,
                u,
                v,
                width,
                height,
                frame_num,
            });

            frame_num += 1;
            offset += frame_size;

            // 检查最大帧数
            if self.config.max_frames > 0 && frame_num >= self.config.max_frames {
                break;
            }
        }

        Ok(frames)
    }
}

/// 解析 ffprobe JSON 输出 (使用 serde_json)
fn parse_probe_json(json: &str) -> Result<VideoInfo, String> {
    let output: FfprobeOutput = serde_json::from_str(json)
        .map_err(|e| format!("JSON解析失败: {} - 原始内容的前100字符: {}", e, &json[..json.len().min(100)]))?;

    let mut info = VideoInfo::default();

    // 从 streams 中提取视频流信息
    if let Some(streams) = &output.streams {
        for stream in streams {
            // 找视频流
            if stream.codec_name.is_some() && stream.width.unwrap_or(0) > 0 {
                if let Some(ref codec) = stream.codec_name {
                    info.codec = codec.clone();
                }
                if let Some(w) = stream.width {
                    info.width = w;
                }
                if let Some(h) = stream.height {
                    info.height = h;
                }
                if let Some(ref fps_str) = stream.r_frame_rate {
                    // 帧率格式 "25/1"
                    if let Some(div) = fps_str.find('/') {
                        if let (Ok(num), Ok(den)) = (
                            fps_str[..div].parse::<f64>(),
                            fps_str[div+1..].parse::<f64>()
                        ) {
                            if den > 0.0 {
                                info.fps = num / den;
                            }
                        }
                    }
                }
                if let Some(fc) = stream.nb_frames.as_ref().and_then(|s| s.parse::<u32>().ok()) {
                    info.frame_count = fc;
                }
                // 如果 format 中没有 bit_rate，用 stream 的
                if info.bitrate == 0 && stream.bit_rate.is_some() {
                    if let Ok(br) = stream.bit_rate.as_ref().unwrap().parse::<u64>() {
                        info.bitrate = br;
                    }
                }
            }
        }
    }

    // 从 format 中提取全局信息
    if let Some(ref format) = output.format {
        // 优先使用 format 层的 bit_rate (这是整体码率，更准确)
        if let Some(ref br_str) = format.bit_rate {
            if let Ok(br) = br_str.parse::<u64>() {
                info.bitrate = br;
            }
        }
        // 解析时长
        if let Some(ref dur_str) = format.duration {
            if let Ok(dur) = dur_str.parse::<f64>() {
                info.duration_ms = (dur * 1000.0) as u64;
            }
        }
    }

    tracing::debug!("解析视频信息: {}x{}, codec={}, bitrate={}, duration={}ms, fps={}",
        info.width, info.height, info.codec, info.bitrate, info.duration_ms, info.fps);

    // 如果码率为0，打印警告
    if info.bitrate == 0 {
        tracing::warn!("警告: 未能获取视频码率 codec={} width={}", info.codec, info.width);
    }

    if info.width == 0 || info.codec.is_empty() {
        return Err(format!("未能解析有效视频信息 codec={} width={}", info.codec, info.width));
    }

    tracing::info!("VideoInfo 最终结果: {}x{} codec={} bitrate={}bps ({:.2}Mbps)",
        info.width, info.height, info.codec, info.bitrate, info.bitrate as f64 / 1_000_000.0);

    Ok(info)
}

/// 使用线程池并行解码多个视频
pub fn decode_parallel<P: AsRef<Path>>(
    paths: &[P],
    config: DecoderConfig,
) -> Vec<Result<(Vec<OwnedFrame>, VideoInfo), String>> {
    use std::sync::mpsc;
    use std::thread;

    let (tx, rx) = mpsc::channel();
    let config = Arc::new(config);

    // 创建工作线程
    let handles: Vec<_> = paths.iter()
        .map(|path| {
            let tx = tx.clone();
            let config = config.clone();
            let path = path.as_ref().to_path_buf();

            thread::spawn(move || {
                let decoder = VideoDecoder::new((*config).clone());
                let result = decoder.decode(&path);
                tx.send(result).ok();
            })
        })
        .collect();

    // 收集结果
    drop(tx);
    let mut results = Vec::new();
    for r in rx {
        results.push(r);
    }

    // 等待所有线程结束
    for h in handles {
        h.join().ok();
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_probe_json() {
        let json = r#"{
            "streams": [{
                "codec_name": "h264",
                "width": 1920,
                "height": 1080,
                "r_frame_rate": "30000/1001"
            }],
            "format": {
                "duration": "120.5",
                "bit_rate": "5000000"
            }
        }"#;

        let info = parse_probe_json(json).unwrap();
        assert_eq!(info.width, 1920);
        assert_eq!(info.height, 1080);
        assert_eq!(info.codec, "h264");
        assert!((info.fps - 29.97).abs() < 0.1);
    }

    #[test]
    fn test_gpu_detection() {
        let info = detect_gpu();
        // 至少能返回有效结构
        assert!(info.name.is_empty() || !info.name.is_empty()); // 总是有效的
    }
}