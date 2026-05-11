//! FFmpeg 管理模块
//!
//! 内置 FFmpeg 自动下载，支持 GPU 加速检测和回退
//!
//! GPU FFmpeg 下载源:
//!   - Windows: github.com/BtbN/FFmpeg-Builds (NVENC+CUDA 完整支持)
//!   - Linux: johnvansickle.com (含 vaapi/rtnvenc)
//!   - macOS: evermeet.cx

use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

/// GPU 加速类型
#[derive(Debug, Clone, PartialEq)]
pub enum GpuAccel {
    /// CUDA NVENC + CUDA 硬解
    Cuda,
    /// D3D11VA (Windows 集成显卡)
    D3d11va,
    /// VAAPI (Linux)
    Vaapi,
    /// 无 GPU
    None,
}

impl GpuAccel {
    pub fn is_supported(&self) -> bool {
        !matches!(self, GpuAccel::None)
    }
}

/// FFmpeg 下载源
#[derive(Debug, Clone)]
pub enum FfmpegSource {
    /// 从 github.com/BtbN/FFmpeg-Builds 下载 (GPU enabled)
    BtbN { version: String },
    /// 从 gyan.dev 下载 ( essentials build)
    Gyan,
    /// 从 johnvansickle.com 下载 (static build, Linux)
    JohnVanSickle,
    /// 系统已有 FFmpeg
    System,
    /// 用户指定路径
    Custom(PathBuf),
}

impl FfmpegSource {
    /// 获取下载 URL
    fn get_url(&self) -> Option<String> {
        match self {
            FfmpegSource::BtbN { version } => {
                // BtbN 提供多个变体: win64-gpl, win64-gpl-shared, linux-gpl, linux-gpl-shared
                // gpl 变体包含 nonfree 编码器 (NVENC)
                #[cfg(target_os = "windows")]
                {
                    Some(format!(
                        "https://github.com/BtbN/FFmpeg-Builds/releases/download/{}/ffmpeg-{}-win64-gpl.zip",
                        version, version
                    ))
                }
                #[cfg(target_os = "linux")]
                {
                    Some(format!(
                        "https://github.com/BtbN/FFmpeg-Builds/releases/download/{}/ffmpeg-{}-linux-gpl-x86_64.tar.xz",
                        version, version
                    ))
                }
                #[cfg(target_os = "macos")]
                {
                    Some(format!(
                        "https://github.com/BtbN/FFmpeg-Builds/releases/download/{}/ffmpeg-{}-macos64-gpl.tar.xz",
                        version, version
                    ))
                }
            }
            FfmpegSource::Gyan => {
                #[cfg(target_os = "windows")]
                return Some("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip".to_string());
                #[cfg(target_os = "linux")]
                return Some("https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz".to_string());
                #[cfg(target_os = "macos")]
                return Some("https://evermeet.cx/ffmpeg/getrelease/zip".to_string());
                #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
                None
            }
            _ => None,
        }
    }
}

/// FFmpeg 管理器 - 负责内置 FFmpeg 下载和 GPU 检测
pub struct FfmpegManager {
    /// FFmpeg 来源
    source: FfmpegSource,
    /// GPU 加速类型
    gpu_accel: GpuAccel,
    /// GPU 设备名称
    gpu_name: String,
    /// 缓存的二进制路径
    binary_path: Option<PathBuf>,
}

impl FfmpegManager {
    /// 创建新的 FFmpeg 管理器
    pub fn new() -> Self {
        // 优先检测系统 FFmpeg + GPU
        let (gpu_accel, gpu_name) = Self::detect_gpu();

        // 选择最佳 FFmpeg 来源
        let source = if gpu_accel.is_supported() {
            // GPU 可用，尝试使用系统 FFmpeg 或下载 GPU-enabled 版本
            let system_has_nvenc = Self::system_ffmpeg_has_nvenc();
            let system_has_cuda = Self::system_ffmpeg_has_cuda();

            if system_has_nvenc || system_has_cuda {
                info!(
                    "使用系统 FFmpeg (GPU: {})",
                    gpu_name
                );
                FfmpegSource::System
            } else {
                // 系统 FFmpeg 无 GPU，下载 GPU-enabled 版本
                info!("系统 FFmpeg 无 GPU 支持，下载 GPU-enabled 版本");
                FfmpegSource::BtbN {
                    version: "latest".to_string(),
                }
            }
        } else {
            // 无 GPU，使用 essentials build
            info!("无 GPU，使用 essentials FFmpeg build");
            FfmpegSource::Gyan
        };

        Self {
            source,
            gpu_accel,
            gpu_name,
            binary_path: None,
        }
    }

    /// 检测 GPU 类型和名称
    fn detect_gpu() -> (GpuAccel, String) {
        // 优先检测 NVIDIA CUDA
        if let Some(name) = Self::get_nvidia_name() {
            // 检查 FFmpeg 是否支持 CUDA 硬解
            if Self::system_ffmpeg_has_cuda() {
                return (GpuAccel::Cuda, name);
            }
            // 有 NVIDIA 卡但 FFmpeg 无 CUDA，检查是否有 NVENC
            if Self::system_ffmpeg_has_nvenc() {
                return (GpuAccel::Cuda, name);
            }
        }

        // Windows: 检测 D3D11VA
        #[cfg(target_os = "windows")]
        {
            if Self::ffmpeg_has_d3d11va() {
                return (GpuAccel::D3d11va, "D3D11VA".to_string());
            }
        }

        // Linux: 检测 VAAPI
        #[cfg(target_os = "linux")]
        {
            if Self::ffmpeg_has_vaapi() {
                return (GpuAccel::Vaapi, "VAAPI".to_string());
            }
        }

        (GpuAccel::None, "CPU".to_string())
    }

    /// 获取 NVIDIA GPU 名称
    fn get_nvidia_name() -> Option<String> {
        let output = Command::new("nvidia-smi")
            .args(["--query-gpu=name", "--format=csv,noheader"])
            .output()
            .ok()?;
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
        None
    }

    /// 检测系统 FFmpeg 是否有 NVENC
    fn system_ffmpeg_has_nvenc() -> bool {
        let output = Command::new("ffmpeg")
            .args(["-codecs", "-hide_banner"])
            .output()
            .ok();
        match output {
            Some(o) => String::from_utf8_lossy(&o.stdout).contains("h264_nvenc"),
            None => false,
        }
    }

    /// 检测系统 FFmpeg 是否有 CUDA hwaccel
    fn system_ffmpeg_has_cuda() -> bool {
        let output = Command::new("ffmpeg")
            .args(["-hwaccels", "-hide_banner"])
            .output()
            .ok();
        match output {
            Some(o) => String::from_utf8_lossy(&o.stdout).contains("cuda"),
            None => false,
        }
    }

    /// 检测 FFmpeg 是否有 D3D11VA
    #[cfg(target_os = "windows")]
    fn ffmpeg_has_d3d11va() -> bool {
        let output = Command::new("ffmpeg")
            .args(["-hwaccels", "-hide_banner"])
            .output()
            .ok();
        match output {
            Some(o) => String::from_utf8_lossy(&o.stdout).contains("d3d11va"),
            None => false,
        }
    }

    /// 检测 FFmpeg 是否有 VAAPI
    #[cfg(target_os = "linux")]
    fn ffmpeg_has_vaapi() -> bool {
        let output = Command::new("ffmpeg")
            .args(["-hwaccels", "-hide_banner"])
            .output()
            .ok();
        match output {
            Some(o) => String::from_utf8_lossy(&o.stdout).contains("vaapi"),
            None => false,
        }
    }

    /// 下载内置 FFmpeg
    pub fn ensure_downloaded(&mut self) -> Result<(), String> {
        use std::io::{Read, Write};

        let url = self.source.get_url().ok_or("Unsupported platform for auto-download")?;

        let download_dir = Self::get_ffmpeg_cache_dir();
        std::fs::create_dir_all(&download_dir)
            .map_err(|e| format!("创建缓存目录失败: {}", e))?;

        info!("下载 FFmpeg: {}", url);

        // 确定下载路径
        let archive_path = if url.ends_with(".xz") {
            download_dir.join("ffmpeg.tar.xz")
        } else if url.ends_with(".zip") {
            download_dir.join("ffmpeg.zip")
        } else {
            return Err("不支持的压缩格式".to_string());
        };

        // 使用 reqwest 下载
        let response = reqwest::blocking::Client::new()
            .get(&url)
            .send()
            .map_err(|e| format!("下载 FFmpeg 失败: {}", e))?;

        let total_size = response.content_length().unwrap_or(0);

        {
            let mut file = std::fs::File::create(&archive_path)
                .map_err(|e| format!("创建文件失败: {}", e))?;
            let mut bytes = 0u64;
            let mut stream = response;
            let mut buf = [0u8; 65536];
            loop {
                let n = stream.read(&mut buf).map_err(|e| format!("下载失败: {}", e))?;
                if n == 0 { break; }
                bytes += n as u64;
                file.write_all(&buf[..n]).map_err(|e| format!("写入失败: {}", e))?;
                if total_size > 0 {
                    info!("下载进度: {}/{} MB", bytes / 1024 / 1024, total_size / 1024 / 1024);
                }
            }
        }

        // 解压
        let unpack_dir = download_dir.join("unpacked");
        std::fs::create_dir_all(&unpack_dir)
            .map_err(|e| format!("创建解压目录失败: {}", e))?;

        let is_xz = archive_path.extension().map_or(false, |e| e == "xz");
        Self::unpack_archive(&archive_path, &unpack_dir, is_xz)?;

        // 查找 ffmpeg 二进制路径
        let ffmpeg_bin = Self::find_ffmpeg_binary(&unpack_dir)?;

        info!("FFmpeg 已安装到: {:?}", ffmpeg_bin);
        self.binary_path = Some(ffmpeg_bin);

        Ok(())
    }

    /// 解压归档文件
    fn unpack_archive(archive: &Path, dest: &Path, is_xz: bool) -> Result<(), String> {

        if is_xz {
            // 解压 .tar.xz
            let file = std::fs::File::open(archive).map_err(|e| format!("打开归档失败: {}", e))?;
            let mut decomp = xz2::read::XzDecoder::new(file);
            let mut ar = tar::Archive::new(&mut decomp);
            ar.unpack(dest)
                .map_err(|e| format!("解压 tar.xz 失败: {}", e))?;
        } else {
            // 解压 .zip
            let file = std::fs::File::open(archive).map_err(|e| format!("打开归档失败: {}", e))?;
            let mut zip =
                zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败: {}", e))?;
            zip.extract(dest)
                .map_err(|e| format!("解压 zip 失败: {}", e))?;
        }
        Ok(())
    }

    /// 获取 FFmpeg 缓存目录
    fn get_ffmpeg_cache_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("vidcompare")
            .join("ffmpeg")
    }

    /// 在解压目录中查找 ffmpeg 二进制
    fn find_ffmpeg_binary(dir: &Path) -> Result<PathBuf, String> {
        // 查找 ffmpeg.exe (Windows) 或 ffmpeg (Linux/macOS)
        let exe_name = if cfg!(target_os = "windows") {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };

        // 遍历解压后的目录找 bin/ffmpeg
        for entry in walkdir::WalkDir::new(dir)
            .max_depth(4)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.file_name().map_or(false, |n| n == exe_name) {
                // 验证是文件且可执行
                if path.is_file() {
                    return Ok(path.to_path_buf());
                }
            }
        }

        Err(format!("在 {:?} 中未找到 {}", dir, exe_name))
    }

    /// 获取 FFmpeg 可执行文件路径
    pub fn get_executable(&self) -> &str {
        match &self.binary_path {
            Some(p) => p.to_str().unwrap_or("ffmpeg"),
            None => "ffmpeg", // 降级到系统 ffmpeg
        }
    }

    /// 获取 FFmpeg 目录
    pub fn get_dir(&self) -> Option<&Path> {
        self.binary_path.as_ref().map(|p| p.parent().unwrap_or(p.as_path()))
    }

    /// 获取 GPU 加速类型
    pub fn gpu_accel(&self) -> &GpuAccel {
        &self.gpu_accel
    }

    /// 获取 GPU 名称
    pub fn gpu_name(&self) -> &str {
        &self.gpu_name
    }

    /// GPU 是否可用
    pub fn has_gpu(&self) -> bool {
        self.gpu_accel.is_supported()
    }

    /// 检查 FFmpeg 是否可用
    pub fn is_available(&self) -> bool {
        let exe = self.get_executable();
        Command::new(exe)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }

    /// 构建解码命令 (用于提取 YUV 帧)
    pub fn build_decode_command<P: AsRef<Path>>(
        &self,
        path: P,
        use_gpu: bool,
        max_frames: u32,
    ) -> std::process::Command {
        let exe = self.get_executable();
        let mut cmd = std::process::Command::new(exe);

        let gpu = if use_gpu { &self.gpu_accel } else { &GpuAccel::None };

        // 硬件加速选项 - 必须在 -i 之前
        match gpu {
            GpuAccel::Cuda => {
                cmd.args(["-hwaccel", "cuda"]);
                cmd.args(["-hwaccel_output_format", "cuda"]);
            }
            GpuAccel::D3d11va => {
                cmd.args(["-hwaccel", "d3d11va"]);
                cmd.args(["-hwaccel_output_format", "yuv420p"]);
            }
            GpuAccel::Vaapi => {
                cmd.args(["-hwaccel", "vaapi"]);
                cmd.args(["-vaapi_device", "/dev/dri/renderD128"]);
                cmd.args(["-hwaccel_output_format", "yuv420p"]);
            }
            GpuAccel::None => {}
        }

        cmd.arg("-i").arg(path.as_ref());

        // GPU 解码后的格式转换
        match gpu {
            GpuAccel::Cuda | GpuAccel::Vaapi => {
                cmd.args(["-vf", "hwdownload,format=nv12"]);
            }
            _ => {}
        }

        // 最大帧数
        if max_frames > 0 {
            cmd.args(["-frames:v", &max_frames.to_string()]);
        }

        // 输出 YUV420P
        cmd.args(["-pix_fmt", "yuv420p", "-f", "rawvideo", "-"]);

        cmd
    }

    /// 构建探针命令
    pub fn build_probe_command<P: AsRef<Path>>(&self, path: P) -> std::process::Command {
        let exe = self.get_executable();
        let mut cmd = std::process::Command::new(exe);
        cmd.args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_format", "-show_streams",
            "-count_frames",
        ]);
        cmd.arg(path.as_ref());
        cmd
    }

    /// 获取 ffprobe 路径
    pub fn get_probed_executable(&self) -> &str {
        self.get_executable() // ffmpeg-sidecar 下载的包同时包含 ffmpeg 和 ffprobe
    }
}

impl Default for FfmpegManager {
    fn default() -> Self {
        Self::new()
    }
}
