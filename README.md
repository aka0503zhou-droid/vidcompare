# VidCompare - 视频质量对比工具

## 功能特性

- **批量匹对**: 按命名规则自动匹配合视频文件 (支持中英文后缀)
- **画质指标**: PSNR / SSIM / VMAF 逐帧计算
- **GPU 加速**: 自动检测 NVIDIA GPU，支持 CUDA 加速
- **数据持久化**: SQLite 数据库存储所有对比记录
- **多格式导出**: CSV / JSON / HTML 报告

## 文件匹对规则

支持的后缀 (中文优先):

| 原文件 | 压缩文件 | 后缀 |
|--------|----------|------|
| a.mp4 | a_hc.mp4 | _hc |
| b.mp4 | b_高压缩.mp4 | _高压缩 |
| c.mp4 | c_转码.mp4 | _转码 |
| d.mp4 | d_enc.mp4 | _enc |

预置后缀列表: `_高压缩`, `_低码率`, `_转码`, `_压缩`, `_hc`, `_crf`, `_enc`, `_av1`, `_hevc`, `_264`, `_265` 等。

## 数据字段

对比结果包含:
- 序号 (#)
- 原文件名 / 压缩文件名
- 原文件大小 / 压缩文件大小
- 原文件码率 / 压缩文件码率
- 压缩比 (%)
- PSNR (dB)
- SSIM (0-1)
- VMAF (0-100)
- 状态 (pending/running/completed/failed)

## 编译 (Windows)

### 方式一: 原生编译

```batch
# 安装 Rust (如果未安装)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 克隆项目
git clone <repo_url>
cd vidcompare

# 编译 (debug)
cargo build

# 编译 (release, 优化后)
cargo build --release

# 运行
.\target\release\vidcompare.exe
```

### 方式二: 交叉编译 (Linux → Windows)

```bash
# 安装交叉编译工具链
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

## 技术架构

```
vidcompare/
├── src/
│   ├── main.rs           # 入口, GPU 检测
│   ├── app.rs            # 主应用逻辑
│   ├── config.rs        # 配置 (后缀列表等)
│   ├── engine/
│   │   ├── mod.rs        # 核心类型定义
│   │   ├── scanner.rs    # 目录扫描
│   │   ├── matcher.rs    # 文件匹对 (HashMap 优化)
│   │   ├── decoder.rs    # 视频解码 (Symphonia)
│   │   ├── metrics.rs   # PSNR/SSIM 计算 (SIMD 优化)
│   │   └── pipeline.rs   # 并行处理流水线 (Rayon)
│   ├── db/
│   │   ├── mod.rs        # 数据库连接
│   │   ├── schema.rs     # 表结构
│   │   └── repository.rs # CRUD 操作
│   ├── export/
│   │   ├── mod.rs
│   │   └── exporters.rs  # CSV/JSON/HTML 导出
│   ├── ui/               # Slint UI 组件
│   └── ffi/              # FFmpeg 检测
└── Cargo.toml
```

## 性能优化

1. **SIMD 加速**: PSNR/SSIM 计算使用 SIMD 指令
2. **零拷贝**: 帧数据直接引用，避免内存拷贝
3. **Buffer 池化**: 帧内存复用，减少 allocations
4. **Rayon 并行**: 多文件同时处理
5. **HashMap 匹对**: O(1) 文件查找

## 依赖

- **GUI**: Slint 1.8
- **视频解码**: Symphonia (纯 Rust)
- **数据库**: SQLite (rusqlite)
- **并行**: Rayon + Tokio
- **GPU 检测**: winreg (Windows)

## License

MIT
