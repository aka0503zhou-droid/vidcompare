//! 数据库模块

mod repository;

use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;
use tracing::info;

/// 数据库封装
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// 创建或打开数据库
    pub fn new() -> Result<Self, rusqlite::Error> {
        let db_path = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("vidcompare")
            .join("results.db");

        // 确保目录存在
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        info!("打开数据库: {}", db_path.display());

        let conn = Connection::open(&db_path)?;
        let db = Self { conn: Mutex::new(conn) };
        db.init_schema()?;

        Ok(db)
    }

    /// 从指定路径打开数据库
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        let db = Self { conn: Mutex::new(conn) };
        db.init_schema()?;
        Ok(db)
    }

    /// 初始化数据库表
    fn init_schema(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS comparison_results (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                idx             INTEGER NOT NULL,
                ref_filename    TEXT NOT NULL,
                dist_filename   TEXT,
                ref_filepath    TEXT NOT NULL,
                dist_filepath   TEXT,
                ref_filesize    INTEGER,
                dist_filesize   INTEGER,
                ref_bitrate     INTEGER,
                dist_bitrate    INTEGER,
                ref_width       INTEGER,
                ref_height      INTEGER,
                psnr            REAL,
                ssim            REAL,
                vmaf            REAL,
                compression_ratio REAL,
                avg_fps         REAL,
                processing_time_ms INTEGER,
                frame_count     INTEGER,
                status          TEXT DEFAULT 'pending',
                error_message   TEXT,
                created_at      DATETIME DEFAULT CURRENT_TIMESTAMP,
                started_at      DATETIME,
                completed_at    DATETIME
            );

            CREATE INDEX IF NOT EXISTS idx_idx ON comparison_results(idx);
            CREATE INDEX IF NOT EXISTS idx_ref_filename ON comparison_results(ref_filename);
            CREATE INDEX IF NOT EXISTS idx_dist_filename ON comparison_results(dist_filename);
            CREATE INDEX IF NOT EXISTS idx_status ON comparison_results(status);
            CREATE INDEX IF NOT EXISTS idx_created_at ON comparison_results(created_at);
            "
        )?;

        // 迁移：为已有表添加新列（如果不存在）
        // 使用单独的执行语句，因为 ALTER TABLE ADD COLUMN IF NOT EXISTS 不是所有 SQLite 版本都支持
        let _ = conn.execute(
            "ALTER TABLE comparison_results ADD COLUMN ref_width INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE comparison_results ADD COLUMN ref_height INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE comparison_results ADD COLUMN dist_bitrate INTEGER",
            [],
        );

        conn.execute_batch(
            "
            -- 预聚合表：每日汇总
            CREATE TABLE IF NOT EXISTS daily_summary (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                summary_date         DATE NOT NULL UNIQUE,
                total_comparisons    INTEGER DEFAULT 0,
                successful_comparisons INTEGER DEFAULT 0,
                failed_comparisons   INTEGER DEFAULT 0,
                avg_psnr             REAL,
                min_psnr             REAL,
                max_psnr             REAL,
                avg_ssim             REAL,
                avg_vmaf             REAL,
                total_ref_filesize   INTEGER DEFAULT 0,
                total_dist_filesize  INTEGER DEFAULT 0,
                avg_compression_ratio REAL,
                updated_at           DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            -- 预聚合表：文件统计
            CREATE TABLE IF NOT EXISTS file_stats (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                filename             TEXT NOT NULL UNIQUE,
                file_path            TEXT,
                comparison_count     INTEGER DEFAULT 0,
                first_seen           DATETIME,
                last_seen            DATETIME,
                avg_psnr             REAL,
                min_psnr             REAL,
                max_psnr             REAL,
                avg_ssim             REAL,
                avg_vmaf             REAL,
                avg_compression_ratio REAL,
                total_comparisons    INTEGER DEFAULT 0,
                updated_at           DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            -- 触发器：自动更新每日汇总 (简化版，避免复杂计算)
            CREATE TRIGGER IF NOT EXISTS trg_after_insert_result
            AFTER INSERT ON comparison_results
            WHEN NEW.status = 'completed' AND NEW.psnr IS NOT NULL
            BEGIN
                INSERT INTO daily_summary (summary_date, total_comparisons, successful_comparisons, failed_comparisons, avg_psnr, min_psnr, max_psnr)
                VALUES (DATE(NEW.created_at), 1, 1, 0, NEW.psnr, NEW.psnr, NEW.psnr)
                ON CONFLICT(summary_date) DO UPDATE SET
                    total_comparisons = total_comparisons + 1,
                    successful_comparisons = successful_comparisons + 1,
                    avg_psnr = (avg_psnr * (successful_comparisons - 1) + NEW.psnr) / successful_comparisons,
                    min_psnr = CASE WHEN NEW.psnr < min_psnr THEN NEW.psnr ELSE min_psnr END,
                    max_psnr = CASE WHEN NEW.psnr > max_psnr THEN NEW.psnr ELSE max_psnr END,
                    updated_at = CURRENT_TIMESTAMP;
            END;

            -- 触发器：处理失败记录
            CREATE TRIGGER IF NOT EXISTS trg_after_insert_failed
            AFTER INSERT ON comparison_results
            WHEN NEW.status = 'failed'
            BEGIN
                INSERT INTO daily_summary (summary_date, total_comparisons, successful_comparisons, failed_comparisons)
                VALUES (DATE(NEW.created_at), 1, 0, 1)
                ON CONFLICT(summary_date) DO UPDATE SET
                    total_comparisons = total_comparisons + 1,
                    failed_comparisons = failed_comparisons + 1,
                    updated_at = CURRENT_TIMESTAMP;
            END;

            -- 触发器：自动更新文件统计
            CREATE TRIGGER IF NOT EXISTS trg_after_insert_filestats
            AFTER INSERT ON comparison_results
            WHEN NEW.status = 'completed' AND NEW.psnr IS NOT NULL
            BEGIN
                INSERT INTO file_stats (filename, comparison_count, first_seen, last_seen, avg_psnr, min_psnr, max_psnr)
                VALUES (NEW.ref_filename, 1, NEW.created_at, NEW.created_at, NEW.psnr, NEW.psnr, NEW.psnr)
                ON CONFLICT(filename) DO UPDATE SET
                    comparison_count = comparison_count + 1,
                    last_seen = NEW.created_at,
                    avg_psnr = (avg_psnr * (comparison_count - 1) + NEW.psnr) / comparison_count,
                    min_psnr = CASE WHEN NEW.psnr < min_psnr THEN NEW.psnr ELSE min_psnr END,
                    max_psnr = CASE WHEN NEW.psnr > max_psnr THEN NEW.psnr ELSE max_psnr END,
                    updated_at = CURRENT_TIMESTAMP;
            END;
            "
        )?;

        info!("数据库表初始化完成");
        Ok(())
    }
}