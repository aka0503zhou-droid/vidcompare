//! 数据仓库模块 - CRUD 操作

use rusqlite::{params, Row};
use tracing::info;

use super::Database;
use crate::engine::ComparisonRecord;

impl Database {
    /// 插入单条记录，返回带 id 的 record
    pub fn insert_record(&self, mut record: ComparisonRecord) -> Result<ComparisonRecord, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            r#"
            INSERT INTO comparison_results (
                idx, ref_filename, dist_filename, ref_filepath, dist_filepath,
                ref_filesize, dist_filesize, ref_bitrate, dist_bitrate,
                ref_width, ref_height,
                compression_ratio, psnr, ssim, vmaf, status, error_message
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            params![
                record.index,
                record.ref_filename,
                record.dist_filename.as_ref().unwrap_or(&"-".to_string()),
                record.ref_path,
                record.dist_path.as_ref().unwrap_or(&"-".to_string()),
                record.ref_filesize as i64,
                record.dist_filesize.map(|s| s as i64),
                record.ref_bitrate as i64,
                record.dist_bitrate.map(|b| b as i64),
                record.ref_width.map(|w| w as i64),
                record.ref_height.map(|h| h as i64),
                record.compression_ratio,
                record.psnr,
                record.ssim,
                record.vmaf,
                record.status.to_string(),
                record.error_message,
            ],
        )?;

        record.id = Some(conn.last_insert_rowid());
        Ok(record)
    }

    /// 批量插入记录，返回带 id 的 records
    pub fn insert_records(&self, records: &[ComparisonRecord]) -> Result<Vec<ComparisonRecord>, rusqlite::Error> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut result = Vec::with_capacity(records.len());
        for mut record in records.iter().cloned() {
            tx.execute(
                r#"
                INSERT INTO comparison_results (
                    idx, ref_filename, dist_filename, ref_filepath, dist_filepath,
                    ref_filesize, dist_filesize, ref_bitrate, dist_bitrate,
                    ref_width, ref_height,
                    compression_ratio, psnr, ssim, vmaf, status, error_message
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
                params![
                    record.index,
                    record.ref_filename,
                    record.dist_filename.as_ref().unwrap_or(&"-".to_string()),
                    record.ref_path,
                    record.dist_path.as_ref().unwrap_or(&"-".to_string()),
                    record.ref_filesize as i64,
                    record.dist_filesize.map(|s| s as i64),
                    record.ref_bitrate as i64,
                    record.dist_bitrate.map(|b| b as i64),
                    record.ref_width.map(|w| w as i64),
                    record.ref_height.map(|h| h as i64),
                    record.compression_ratio,
                    record.psnr,
                    record.ssim,
                    record.vmaf,
                    record.status.to_string(),
                    record.error_message,
                ],
            )?;
            record.id = Some(tx.last_insert_rowid());
            result.push(record);
        }

        tx.commit()?;
        info!("批量插入 {} 条记录", result.len());
        Ok(result)
    }

    /// 查询所有记录
    pub fn get_all_records(&self) -> Result<Vec<ComparisonRecord>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM comparison_results ORDER BY idx"
        )?;
        
        let records = stmt.query_map([], |row| row_to_record(row))?
            .filter_map(|r| r.ok())
            .collect();
        
        Ok(records)
    }

    /// 按状态查询
    pub fn get_records_by_status(&self, status: &str) -> Result<Vec<ComparisonRecord>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM comparison_results WHERE status = ? ORDER BY idx"
        )?;
        
        let records = stmt.query_map([status], |row| row_to_record(row))?
            .filter_map(|r| r.ok())
            .collect();
        
        Ok(records)
    }

    /// 按时间范围查询
    pub fn get_records_by_date_range(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<ComparisonRecord>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM comparison_results WHERE created_at BETWEEN ? AND ? ORDER BY idx"
        )?;
        
        let records = stmt.query_map([from, to], |row| row_to_record(row))?
            .filter_map(|r| r.ok())
            .collect();
        
        Ok(records)
    }

    /// 按文件名搜索
    pub fn search_by_filename(&self, pattern: &str) -> Result<Vec<ComparisonRecord>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM comparison_results WHERE ref_filename LIKE ? OR dist_filename LIKE ? ORDER BY idx"
        )?;
        
        let pattern = format!("%{}%", pattern);
        let records = stmt.query_map([&pattern, &pattern], |row| row_to_record(row))?
            .filter_map(|r| r.ok())
            .collect();
        
        Ok(records)
    }

    /// 更新单条记录
    pub fn update_record(&self, record: &ComparisonRecord) -> Result<bool, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        
        let affected = conn.execute(
            r#"
            UPDATE comparison_results SET
                dist_filename = ?,
                dist_filepath = ?,
                dist_filesize = ?,
                dist_bitrate = ?,
                compression_ratio = ?,
                psnr = ?,
                ssim = ?,
                vmaf = ?,
                status = ?,
                error_message = ?
            WHERE idx = ? AND ref_filename = ?
            "#,
            params![
                record.dist_filename.as_ref().unwrap_or(&"-".to_string()),
                record.dist_path.as_ref().unwrap_or(&"-".to_string()),
                record.dist_filesize.map(|s| s as i64),
                record.dist_bitrate.map(|b| b as i64),
                record.compression_ratio,
                record.psnr,
                record.ssim,
                record.vmaf,
                record.status.to_string(),
                record.error_message,
                record.index,
                record.ref_filename,
            ],
        )?;
        
        Ok(affected > 0)
    }

    /// 删除记录
    pub fn delete_record(&self, id: i64) -> Result<bool, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "DELETE FROM comparison_results WHERE id = ?",
            [id],
        )?;
        Ok(affected > 0)
    }

    /// 批量删除记录
    pub fn delete_records(&self, ids: &[i64]) -> Result<usize, rusqlite::Error> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().unwrap();
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "DELETE FROM comparison_results WHERE id IN ({})",
            placeholders.join(",")
        );
        let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        let affected = conn.execute(&sql, params.as_slice())?;
        Ok(affected)
    }

    /// 清空所有记录并重置自增ID
    pub fn truncate(&self) -> Result<usize, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM comparison_results", [])?;
        conn.execute("DELETE FROM sqlite_sequence WHERE name='comparison_results'", [])?;
        info!("Truncate 完成，ID已重置");
        Ok(1)
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> Result<Stats, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM comparison_results", [], |row| row.get(0)
        )?;
        
        let completed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM comparison_results WHERE status = 'completed'", [], |row| row.get(0)
        )?;
        
        let avg_psnr: Option<f64> = conn.query_row(
            "SELECT AVG(psnr) FROM comparison_results WHERE psnr IS NOT NULL", [], |row| row.get(0)
        ).ok().flatten();
        
        let avg_ssim: Option<f64> = conn.query_row(
            "SELECT AVG(ssim) FROM comparison_results WHERE ssim IS NOT NULL", [], |row| row.get(0)
        ).ok().flatten();
        
        let avg_vmaf: Option<f64> = conn.query_row(
            "SELECT AVG(vmaf) FROM comparison_results WHERE vmaf IS NOT NULL", [], |row| row.get(0)
        ).ok().flatten();
        
        Ok(Stats {
            total: total as usize,
            completed: completed as usize,
            avg_psnr: avg_psnr.map(|v| v as f32),
            avg_ssim: avg_ssim.map(|v| v as f32),
            avg_vmaf: avg_vmaf.map(|v| v as f32),
        })
    }
}

/// 统计数据
#[derive(Debug, Default)]
pub struct Stats {
    pub total: usize,
    pub completed: usize,
    pub avg_psnr: Option<f32>,
    pub avg_ssim: Option<f32>,
    pub avg_vmaf: Option<f32>,
}

/// 从 Row 转换为 ComparisonRecord
fn row_to_record(row: &Row) -> Result<ComparisonRecord, rusqlite::Error> {
    Ok(ComparisonRecord {
        id: row.get("id").ok(),
        index: row.get("idx")?,
        ref_filename: row.get("ref_filename")?,
        dist_filename: Some(row.get("dist_filename")?),
        ref_path: row.get("ref_filepath")?,
        dist_path: Some(row.get("dist_filepath")?),
        ref_filesize: row.get::<_, i64>("ref_filesize")? as u64,
        dist_filesize: row.get::<_, Option<i64>>("dist_filesize")?.map(|s| s as u64),
        ref_bitrate: row.get::<_, i64>("ref_bitrate")? as u64,
        dist_bitrate: row.get::<_, Option<i64>>("dist_bitrate")?.map(|b| b as u64),
        ref_width: row.get::<_, Option<i64>>("ref_width")?.map(|w| w as u32),
        ref_height: row.get::<_, Option<i64>>("ref_height")?.map(|h| h as u32),
        psnr: row.get("psnr")?,
        ssim: row.get("ssim")?,
        vmaf: row.get("vmaf")?,
        avg_fps: None,
        processing_time_ms: None,
        compression_ratio: row.get("compression_ratio")?,
        frame_count: None,
        status: row.get::<_, String>("status")?.as_str().into(),
        error_message: row.get("error_message")?,
        created_at: chrono::Utc::now().to_rfc3339(),
        started_at: None,
        completed_at: None,
        psnr_per_frame: Vec::new(),
        ssim_per_frame: Vec::new(),
        vmaf_per_frame: Vec::new(),
    })
}
