# VidCompare 项目文档

## 一、项目概述

**VidCompare** 是一款基于 Rust 开发的专业视频质量对比工具，支持 PSNR、SSIM、VMAF 等主流视频质量指标的批量计算与对比分析。

### 1.1 核心功能

- **多指标计算**: PSNR、SSIM、VMAF 视频质量指标
- **GPU 加速**: 支持 NVIDIA CUDA 硬件加速解码
- **批量处理**: 支持多对视频文件并行对比
- **智能匹配**: 自动识别原文件与压缩文件的配对关系
- **数据持久化**: SQLite 本地数据库存储对比结果
- **多格式导出**: 支持 CSV、JSON、HTML 格式导出

### 1.2 目标用户

- 视频压缩算法研究人员
- 视频编码工程师
- 质量控制工程师
- 内容创作者评估压缩效果

---

## 二、需求分析

### 2.1 功能需求

| 需求编号 | 功能描述 | 优先级 | 状态 |
|----------|----------|--------|------|
| F001 | 目录选择：选择原文件和压缩文件目录 | P0 | ✅ 已实现 |
| F002 | 文件扫描：自动扫描目录下所有视频文件 | P0 | ✅ 已实现 |
| F003 | 智能匹配：根据文件名后缀自动匹配配对 | P0 | ✅ 已实现 |
| F004 | 批量对比：批量处理多个文件配对 | P0 | ✅ 已实现 |
| F005 | 指标计算：PSNR/SSIM/VMAF 计算 | P0 | ✅ 已实现 |
| F006 | GPU 加速：支持 CUDA 加速 | P1 | ✅ 已实现 |
| F007 | 实时进度：显示处理进度和 ETA | P1 | ✅ 已实现 |
| F008 | 结果存储：SQLite 数据库持久化 | P1 | ✅ 已实现 |
| F009 | 结果导出：CSV/JSON 导出 | P2 | ✅ 已实现 |
| F010 | 增量显示：任务完成即实时显示 | P1 | ⚠️ 待优化 |
| F011 | 批量删除：支持多选删除结果 | P2 | ⚠️ 待优化 |

### 2.2 非功能需求

| 需求编号 | 需求描述 | 目标 |
|----------|----------|------|
| N001 | 性能：单对视频处理时间 < 总时长的 10% | - |
| N002 | 内存：峰值内存占用 < 2GB | - |
| N003 | 可用性：进度实时更新，无卡顿 | - |
| N004 | 可靠性：数据库操作不丢失数据 | - |

### 2.3 压缩后缀配置

系统支持的可配置压缩后缀：

```
中文后缀: _高压缩, _低码率, _转码, _压缩, _低质量, _编码
英文后缀: _hc, _crf, _enc, _out, _trans, _264, _265, _hevc, _av1, _vp9
```

---

## 三、系统设计

### 3.1 技术选型

| 组件 | 技术选型 | 理由 |
|------|----------|------|
| 编程语言 | Rust | 高性能、内存安全、并发支持 |
| GUI 框架 | eframe/egui | 跨平台、轻量级、无依赖 |
| 并发库 | Rayon | 简洁的数据并行库 |
| 数据库 | SQLite (rusqlite) | 零配置、嵌入式、轻量 |
| 视频处理 | FFmpeg | 行业标准、功能强大 |
| VMAF 计算 | libvmaf-rs | 原生 Rust 实现 |
| 日志 | tracing | 结构化日志、支持异步 |

### 3.2 系统架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                         表示层 (GUI)                            │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │  VidCompareApp                                             │  │
│  │  ├── 上段: 目录选择 + GPU 开关                             │  │
│  │  ├── 中段: 文件列表 + 设置 + 开始对比                       │  │
│  │  └── 下段: 过滤表格 + 导出 + 详情                          │  │
│  └─────────────────────────────────────────────────────────┘  │
└─────────────────────────────┬───────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                         业务逻辑层 (Engine)                      │
│  ┌───────────┬───────────┬───────────┬───────────┬───────────┐  │
│  │ Pipeline  │ Scanner   │ Matcher   │ Decoder   │ Metrics   │  │
│  │ 流水线    │ 目录扫描  │ 文件匹配  │ 视频解码  │ 指标计算  │  │
│  └───────────┴───────────┴───────────┴───────────┴───────────┘  │
└─────────────────────────────┬───────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                         外部依赖层                                │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐           │
│  │   FFmpeg    │  │   SQLite     │  │   GPU/CUDA   │           │
│  │  质量计算    │  │  结果存储    │  │  硬件加速    │           │
│  └──────────────┘  └──────────────┘  └──────────────┘           │
└─────────────────────────────────────────────────────────────────┘
```

### 3.3 数据模型

#### 3.3.1 VideoFile (视频文件)

```rust
pub struct VideoFile {
    pub name: String,           // 文件名
    pub path: PathBuf,           // 文件路径
    pub size: u64,              // 文件大小
    pub bitrate: Option<u64>,   // 视频码率
    pub width: Option<u32>,     // 宽度
    pub height: Option<u32>,    // 高度
    pub duration_ms: Option<u64>,  // 时长
    pub codec: Option<String>,  // 编码格式
    pub frame_count: Option<u32>,  // 帧数
}
```

#### 3.3.2 FilePair (文件配对)

```rust
pub struct FilePair {
    pub index: u32,             // 配对序号
    pub reference: Option<VideoFile>,  // 原文件
    pub distorted: Option<VideoFile>,   // 压缩文件
    pub selected: bool,         // 是否选中
    pub ref_file: VideoFile,     // 探测后的原文件
    pub dist_file: VideoFile,   // 探测后的压缩文件
}
```

#### 3.3.3 ComparisonRecord (对比结果)

```rust
pub struct ComparisonRecord {
    pub id: Option<i64>,        // 数据库 ID
    pub index: u32,             // 显示序号
    pub ref_filename: String,   // 原文件名
    pub dist_filename: Option<String>,  // 压缩文件名
    pub ref_filesize: u64,      // 原文件大小
    pub dist_filesize: Option<u64>,     // 压缩后大小
    pub ref_bitrate: u64,       // 原码率
    pub dist_bitrate: Option<u64>,       // 压后码率
    pub ref_width: Option<u32>, // 原分辨率宽度
    pub ref_height: Option<u32>,// 原分辨率高度
    pub psnr: Option<f32>,      // PSNR 值
    pub ssim: Option<f32>,      // SSIM 值
    pub vmaf: Option<f32>,      // VMAF 值
    pub status: ProcessingStatus,  // 处理状态
    pub compression_ratio: Option<f32>,  // 压缩比
    pub processing_time_ms: Option<u64>, // 处理耗时
}
```

### 3.4 核心流程

#### 3.4.1 目录扫描流程

```
用户选择目录
     │
     ▼
┌─────────────────┐
│  fast_scan_     │  快速扫描：获取文件名、大小
│  directory()    │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  match_files()  │  智能匹配：根据后缀匹配配对
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  probe_videos_  │  探测：获取码率、分辨率、帧数
│  in_pairs()     │
└────────┬────────┘
         │
         ▼
    FilePair 列表
```

#### 3.4.2 批量对比流程

```
开始对比
     │
     ▼
┌─────────────────┐
│  process_batch_ │  自适应并行处理
│  adaptive()     │
│                 │
│  ┌─────────────┐│
│  │ Rayon 并行  ││  每个配对独立处理
│  │ 任务队列    ││
│  └─────────────┘│
└────────┬────────┘
         │
         ├──────────────────────────┐
         ▼                          ▼
┌─────────────────┐    ┌─────────────────┐
│  FFmpeg 计算    │    │  Channel 增量   │
│  PSNR/SSIM/VMAF │───▶│  结果发送       │
└────────┬────────┘    └────────┬────────┘
         │                    │
         ▼                    ▼
┌─────────────────┐    ┌─────────────────┐
│  数据库写入     │◀───│  后台线程      │
│  insert_record() │    │  收集 + 写入    │
└─────────────────┘    └─────────────────┘
```

### 3.5 并发模型

```
┌────────────────────────────────────────────┐
│           GUI 主线程 (egui)                │
│  - 事件循环 + UI 渲染                      │
│  - poll_progress() 50ms 轮询              │
└─────────────────┬──────────────────────────┘
                  │ Arc<ProgressShared>
                  ▼
┌────────────────────────────────────────────┐
│        后台处理线程 (std::thread)          │
│  ┌──────────────────────────────────────┐ │
│  │ ① 视频探测 (probe_videos_in_pairs)    │ │
│  │ ② 并行处理 (process_batch_adaptive)  │ │
│  │ ③ 结果收集 (mpsc::Channel)           │ │
│  │ ④ 数据库写入 (insert_record)          │ │
│  └──────────────────────────────────────┘ │
└────────────────────────────────────────────┘
```

---

## 四、关键设计要点

### 4.1 自适应并行处理

**设计目标**: 根据系统资源自动选择最优并行度

```rust
// GPU 模式：限制并发避免显存不足
let max_parallelism = if gpu_info.available && self.config.use_gpu {
    (cpu_cores / 4).max(2).min(3)  // 最多 3 个并发
} else {
    cpu_cores.min(valid_total)  // CPU 模式可激进并行
};
```

### 4.2 增量结果显示

**设计目标**: 任务完成即显示，不等待全部完成

```
Pipeline ──完成1──▶ Channel ──▶ 后台线程 ──▶ GUI 实时显示
              ──完成2──▶                │
              ──完成3──▶                ▼
                                      数据库写入
```

### 4.3 进度跟踪机制

**设计目标**: 实时准确的进度反馈

```rust
// 全局进度映射 (配对索引 → 进度信息)
static PAIR_PROGRESS_MAP: OnceLock<Mutex<HashMap<u32, PairProgressInfo>>>

// 配对级进度
pub struct PairProgressInfo {
    pub frame: AtomicU32,           // 当前帧
    pub expected_frames: AtomicU32, // 预期总帧数
    pub status: Mutex<ProcessingStatus>,  // 状态
}
```

### 4.4 文件智能匹配

**设计目标**: 自动识别原文件和压缩文件的对应关系

```rust
// 匹配策略优先级
1. 同名文件直接匹配
2. 尝试后缀匹配 (_hc, _压缩, _转码等)
3. 检查带扩展名的情况
```

---

## 五、模块设计

### 5.1 模块依赖图

```
┌──────────────┐
│   main.rs   │  程序入口
└──────┬──────┘
       │
       ▼
┌──────────────┐
│    gui.rs   │  GUI 界面
└──────┬──────┘
       │
       ▼
┌──────────────────────────────────────────────────┐
│                   engine/                          │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐   │
│  │ Pipeline   │──│ Scanner   │──│ Matcher    │   │
│  └─────┬──────┘  └─────┬──────┘  └────────────┘   │
│        │             │                            │
│        └──────┬──────┘                            │
│               ▼                                  │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐   │
│  │ Decoder   │──│ Metrics   │──│ FfmpegFilter│   │
│  └────────────┘  └────────────┘  └────────────┘   │
└──────────────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────────────┐
│                      db/                           │
│  ┌────────────┐  ┌────────────┐                    │
│  │  mod.rs   │──│ Repository │                    │
│  └────────────┘  └────────────┘                    │
└──────────────────────────────────────────────────┘
```

### 5.2 各模块职责

| 模块 | 职责 | 关键 API |
|------|------|----------|
| **gui.rs** | UI 渲染、用户交互 | `runGui()`, `poll_progress()` |
| **pipeline.rs** | 并行任务调度 | `process_batch_adaptive()` |
| **scanner.rs** | 目录扫描、视频探测 | `fast_scan_directory()`, `probe_videos_in_pairs()` |
| **matcher.rs** | 文件配对 | `match_files()` |
| **decoder.rs** | 视频解码、ffprobe | `decode()`, `probe()` |
| **metrics.rs** | PSNR/SSIM/VMAF 实现 | `calculate_psnr()`, `calculate_ssim()` |
| **ffmpeg_filter.rs** | FFmpeg 集成 | `calculate_all_metrics_ffmpeg_with_progress()` |
| **repository.rs** | 数据库 CRUD | `insert_record()`, `delete_records()` |

---

## 六、数据库设计

### 6.1 主表结构

```sql
CREATE TABLE comparison_results (
    id              INTEGER PRIMARY KEY,
    idx             INTEGER,           -- 序号
    ref_filename    TEXT,              -- 原文件名
    dist_filename   TEXT,             -- 压缩文件名
    ref_filesize    INTEGER,          -- 原大小
    dist_filesize   INTEGER,          -- 压后大小
    ref_bitrate     INTEGER,          -- 原码率
    dist_bitrate    INTEGER,          -- 压后码率
    ref_width       INTEGER,          -- 原宽度
    ref_height      INTEGER,          -- 原高度
    psnr            REAL,             -- PSNR 值
    ssim            REAL,             -- SSIM 值
    vmaf            REAL,             -- VMAF 值
    status          TEXT,             -- 状态
    created_at      DATETIME           -- 创建时间
);
```

### 6.2 索引设计

```sql
CREATE INDEX idx_idx ON comparison_results(idx);
CREATE INDEX idx_ref_filename ON comparison_results(ref_filename);
CREATE INDEX idx_status ON comparison_results(status);
CREATE INDEX idx_created_at ON comparison_results(created_at);
```

---

## 七、部署与配置

### 7.1 环境要求

- **操作系统**: Windows 10+, macOS 10.14+, Linux
- **Rust**: 1.70+
- **FFmpeg**: 系统安装或自动下载 GPU 版本
- **GPU**: NVIDIA GPU + CUDA (可选)

### 7.2 构建命令

```bash
# 开发构建
cargo build

# 发布构建
cargo build --release

# 运行
cargo run --release
```

### 7.3 配置项

| 配置项 | 默认值 | 说明 |
|--------|--------|------|
| `max_frames` | 500 | 最大处理帧数 |
| `use_gpu` | false | 是否启用 GPU |
| `compute_psnr` | true | 是否计算 PSNR |
| `compute_ssim` | false | 是否计算 SSIM |
| `compute_vmaf` | false | 是否计算 VMAF |

---

## 八、已知问题与限制

### 8.1 待优化项

| 问题 | 描述 | 影响 |
|------|------|------|
| 重复入库 | 修复前可能存在重复数据 | 数据准确性 |
| Channel 缓冲 | 最后一个结果可能丢失 | 进度显示 |
| db 所有权 | `take()` 后需重建 | 删除功能 |

### 8.2 性能限制

| 限制 | 说明 |
|------|------|
| 显存限制 | GPU 模式最多 3 并发 |
| 内存限制 | 峰值可能达 2GB |
| VMAF 模型 | 首次运行需下载 |

---

## 九、版本历史

| 版本 | 日期 | 更新内容 |
|------|------|----------|
| 0.1.0 | 2026-05 | 初始版本，支持 PSNR/SSIM/VMAF |

---

## 十、联系方式

- 项目维护: VidCompare Team
- 问题反馈: [GitHub Issues]
