//! 运行时配置模块

use tracing::{error, info};
use std::path::PathBuf;
use std::io::Write;

/// 检测 GPU 是否可用，返回 (是否可用, GPU名称)
pub fn detect_gpu() -> (bool, String) {
    let info = crate::engine::detect_gpu();
    (info.available, info.name.clone())
}

/// VMAF 模型信息
pub struct VmafModelInfo {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// 预定义的 VMAF 模型
pub const VMAF_MODELS: &[VmafModelInfo] = &[
    VmafModelInfo {
        name: "vmaf_v0.6.1.json",
        url: "https://raw.githubusercontent.com/Netflix/vmaf/master/python/vmaf/resources/model/vmaf_v0.6.1.json",
        sha256: "8e8c8b8e8c8b8e8c8b8e8c8b8e8c8b8e8c8b8e8c8b8e8c8b8e8c8b8e8c8b8e8c",
    },
];

/// 线程池配置 - 使用所有 CPU 核心
pub fn get_rayon_threads() -> usize {
    let cpus = num_cpus::get();
    // 视频质量计算是 CPU 密集型，使用全部核心
    // 对于 8 核以上机器，保留 2 个核心给系统
    if cpus >= 8 {
        cpus - 2
    } else {
        cpus.max(1)
    }
}

/// 初始化全局 rayon 线程池
pub fn init_rayon_pool() {
    use rayon::ThreadPoolBuilder;

    let threads = get_rayon_threads();
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .unwrap_or_else(|e| {
            eprintln!("Warning: Failed to set rayon thread pool: {}", e);
        });
}

/// VMAF 模型缓存目录
pub fn get_vmaf_cache_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("vidcompare")
        .join("vmaf_models")
}

/// 确保 VMAF 模型存在，如不存在返回 None
pub fn get_vmaf_model_path(model_name: &str) -> Option<PathBuf> {
    let cache_dir = get_vmaf_cache_dir();
    let model_path = cache_dir.join(model_name);

    if model_path.exists() {
        Some(model_path)
    } else {
        None
    }
}

/// 获取或下载 VMAF 模型
/// 返回模型路径，如下载失败返回 None
pub fn ensure_vmaf_model(model_name: &str) -> Option<PathBuf> {
    // 如果已存在直接返回
    if let Some(path) = get_vmaf_model_path(model_name) {
        info!("VMAF model {} already cached", model_name);
        return Some(path);
    }

    // 查找模型信息
    let model_info = VMAF_MODELS.iter().find(|m| m.name == model_name)?;
    let cache_path = get_vmaf_cache_dir().join(model_name);

    info!("Downloading VMAF model {}...", model_name);

    // 下载模型
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .ok()?;

    let response = client.get(model_info.url).send().ok()?;

    if !response.status().is_success() {
        error!("Failed to download VMAF model: HTTP {}", response.status());
        return None;
    }

    let bytes = response.bytes().ok()?;

    // 写入文件
    if let Ok(mut file) = std::fs::File::create(&cache_path) {
        if file.write_all(&bytes).is_ok() {
            info!("VMAF model {} saved to {:?}", model_name, cache_path);
            return Some(cache_path);
        }
    }

    error!("Failed to write VMAF model to {:?}", cache_path);
    None
}

/// 确保所有预定义模型都已缓存
pub fn ensure_all_vmaf_models() {
    for model in VMAF_MODELS {
        if get_vmaf_model_path(model.name).is_none() {
            let _ = ensure_vmaf_model(model.name);
        }
    }
}

/// 缓存目录初始化
pub fn ensure_cache_dirs() -> std::io::Result<()> {
    let vmaf_dir = get_vmaf_cache_dir();
    std::fs::create_dir_all(&vmaf_dir)?;
    Ok(())
}