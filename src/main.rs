//! VidCompare - 视频质量对比工具
//!
//! 支持 PSNR/SSIM/VMAF 计算，GPU 加速，数据库记录，CSV/JSON 导出

mod config;
mod db;
mod engine;
mod export;
mod gui;
mod runtime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志 (使用 tracing + env_filter)
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_target(false)
        .init();

    tracing::info!("VidCompare 启动中...");

    // 检测 GPU
    let (gpu_available, gpu_name) = crate::runtime::detect_gpu();
    tracing::info!("GPU 状态: available={}, name={}", gpu_available, gpu_name);

    // 初始化线程池
    crate::runtime::init_rayon_pool();

    // 启动 GUI
    crate::gui::runGui(gpu_available, gpu_name)?;

    Ok(())
}