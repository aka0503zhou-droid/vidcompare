//! 导出器模块 - 支持 CSV、JSON、HTML 格式

use std::fs::File;
use std::io::{BufWriter, Write};
use tracing::info;

use crate::engine::ComparisonRecord;

/// 导出格式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportFormat {
    Csv,
    Json,
    Html,
}

impl ExportFormat {
    pub fn extension(&self) -> &str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            ExportFormat::Html => "html",
        }
    }
}

/// 导出选项
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub output_path: String,
    pub include_skipped: bool,
    pub title: String,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Csv,
            output_path: String::new(),
            include_skipped: true,
            title: "视频质量对比报告".to_string(),
        }
    }
}

/// 导出器 trait
pub trait Exporter {
    fn export(&self, records: &[ComparisonRecord], options: &ExportOptions) -> Result<usize, Box<dyn std::error::Error>>;
}

/// CSV 导出器
pub struct CsvExporter;

impl Exporter for CsvExporter {
    fn export(&self, records: &[ComparisonRecord], options: &ExportOptions) -> Result<usize, Box<dyn std::error::Error>> {
        let file = File::create(&options.output_path)?;
        let mut writer = BufWriter::new(file);
        
        // 写入 BOM (UTF-8)
        writer.write_all(&[0xEF, 0xBB, 0xBF])?;
        
        // CSV 表头
        writeln!(writer, "序号,原文件名,分辨率,原文件大小,压缩文件名,压缩文件大小,原文件码率,压缩文件码率,压缩比,PSNR,SSIM,VMAF")?;
        
        let mut count = 0;
        for record in records {
            // 跳过未处理的记录
            if !options.include_skipped && record.status.to_string() == "skipped" {
                continue;
            }
            
            let compression = record.compression_ratio
                .map(|v| format!("{:.1}", v))
                .unwrap_or_default();
            
            let dist_filename = record.dist_filename.clone().unwrap_or_default();
            let dist_filesize = record.dist_filesize.map(|s| format_size(s)).unwrap_or_default();
            let dist_bitrate = record.dist_bitrate.map(|b| format_bitrate(b)).unwrap_or_default();

            // 分辨率
            let resolution = match (record.ref_width, record.ref_height) {
                (Some(w), Some(h)) => format!("{}x{}", w, h),
                _ => "—".to_string(),
            };

            // 处理 NaN/无穷大
            let psnr_str = record.psnr.map(|p| {
                if p.is_nan() {
                    "N/A".to_string()
                } else if p.is_infinite() {
                    if p.is_sign_positive() { "∞".to_string() } else { "-∞".to_string() }
                } else {
                    format!("{:.2}", p)
                }
            }).unwrap_or_else(|| "N/A".to_string());

            let ssim_str = record.ssim.map(|s| {
                if s.is_nan() {
                    "N/A".to_string()
                } else {
                    format!("{:.4}", s)
                }
            }).unwrap_or_else(|| "N/A".to_string());

            let vmaf_str = record.vmaf.map(|v| {
                if v.is_nan() {
                    "N/A".to_string()
                } else {
                    format!("{:.1}", v)
                }
            }).unwrap_or_else(|| "N/A".to_string());

            writeln!(
                writer,
                "{},{},{},{},{},{},{},{},{},{},{},{}",
                count + 1,
                escape_csv(&record.ref_filename),
                resolution,
                format_size(record.ref_filesize),
                escape_csv(&dist_filename),
                dist_filesize,
                format_bitrate(record.ref_bitrate),
                dist_bitrate,
                compression,
                psnr_str,
                ssim_str,
                vmaf_str,
            )?;
            count += 1;
        }
        
        writer.flush()?;
        info!("CSV 导出完成: {} 条记录 -> {}", count, options.output_path);
        Ok(count)
    }
}

/// JSON 导出器
pub struct JsonExporter;

impl Exporter for JsonExporter {
    fn export(&self, records: &[ComparisonRecord], options: &ExportOptions) -> Result<usize, Box<dyn std::error::Error>> {
        let filtered: Vec<_> = if options.include_skipped {
            records.to_vec()
        } else {
            records.iter().filter(|r| r.status.to_string() != "skipped").cloned().collect()
        };
        
        let json = serde_json::to_string_pretty(&filtered)?;
        
        let mut file = File::create(&options.output_path)?;
        file.write_all(json.as_bytes())?;
        
        info!("JSON 导出完成: {} 条记录 -> {}", filtered.len(), options.output_path);
        Ok(filtered.len())
    }
}

/// HTML 导出器
pub struct HtmlExporter;

impl Exporter for HtmlExporter {
    fn export(&self, records: &[ComparisonRecord], options: &ExportOptions) -> Result<usize, Box<dyn std::error::Error>> {
        let file = File::create(&options.output_path)?;
        let mut writer = BufWriter::new(file);
        
        let filtered: Vec<_> = if options.include_skipped {
            records.to_vec()
        } else {
            records.iter().filter(|r| r.status.to_string() != "skipped").cloned().collect()
        };
        
        // 写入 HTML
        writeln!(writer, "<!DOCTYPE html>")?;
        writeln!(writer, "<html lang=\"zh-CN\">")?;
        writeln!(writer, "<head>")?;
        writeln!(writer, "  <meta charset=\"UTF-8\">")?;
        writeln!(writer, "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">")?;
        writeln!(writer, "  <title>{}</title>", escape_html(&options.title))?;
        writeln!(writer, "  <style>")?;
        writeln!(writer, "    * {{ box-sizing: border-box; margin: 0; padding: 0; }}")?;
        writeln!(writer, "    body {{ font-family: 'Segoe UI', system-ui, sans-serif; background: #f8fafc; color: #1e293b; padding: 20px; }}")?;
        writeln!(writer, "    .container {{ max-width: 1400px; margin: 0 auto; }}")?;
        writeln!(writer, "    h1 {{ color: #2563eb; margin-bottom: 20px; font-size: 24px; }}")?;
        writeln!(writer, "    .summary {{ background: white; border-radius: 8px; padding: 20px; margin-bottom: 20px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }}")?;
        writeln!(writer, "    .summary-item {{ display: inline-block; margin-right: 30px; }}")?;
        writeln!(writer, "    .summary-label {{ color: #64748b; font-size: 12px; }}")?;
        writeln!(writer, "    .summary-value {{ font-size: 24px; font-weight: 600; color: #2563eb; }}")?;
        writeln!(writer, "    table {{ width: 100%; border-collapse: collapse; background: white; border-radius: 8px; overflow: hidden; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }}")?;
        writeln!(writer, "    th {{ background: #1e293b; color: white; padding: 12px 16px; text-align: left; font-weight: 500; }}")?;
        writeln!(writer, "    td {{ padding: 12px 16px; border-bottom: 1px solid #e2e8f0; }}")?;
        writeln!(writer, "    tr:hover {{ background: #f8fafc; }}")?;
        writeln!(writer, "    .status-completed {{ color: #10b981; }}")?;
        writeln!(writer, "    .status-failed {{ color: #ef4444; }}")?;
        writeln!(writer, "    .status-running {{ color: #f59e0b; }}")?;
        writeln!(writer, "    .status-pending {{ color: #64748b; }}")?;
        writeln!(writer, "    .status-skipped {{ color: #94a3b8; }}")?;
        writeln!(writer, "    .metric {{ font-family: 'Consolas', monospace; }}")?;
        writeln!(writer, "    .footer {{ text-align: center; color: #64748b; font-size: 12px; margin-top: 20px; }}")?;
        writeln!(writer, "  </style>")?;
        writeln!(writer, "</head>")?;
        writeln!(writer, "<body>")?;
        writeln!(writer, "  <div class=\"container\">")?;
        writeln!(writer, "    <h1>🎬 {}</h1>", escape_html(&options.title))?;
        
        // 统计信息
        let completed = filtered.iter().filter(|r| r.status.to_string() == "completed").count();
        let avg_psnr = filtered.iter().filter_map(|r| r.psnr).sum::<f32>() / completed.max(1) as f32;
        let avg_ssim = filtered.iter().filter_map(|r| r.ssim).sum::<f32>() / completed.max(1) as f32;
        let avg_vmaf = filtered.iter().filter_map(|r| r.vmaf).sum::<f32>() / completed.max(1) as f32;
        
        writeln!(writer, "    <div class=\"summary\">")?;
        writeln!(writer, "      <div class=\"summary-item\"><div class=\"summary-label\">总记录数</div><div class=\"summary-value\">{}</div></div>", filtered.len())?;
        writeln!(writer, "      <div class=\"summary-item\"><div class=\"summary-label\">已完成</div><div class=\"summary-value\">{}</div></div>", completed)?;
        writeln!(writer, "      <div class=\"summary-item\"><div class=\"summary-label\">平均 PSNR</div><div class=\"summary-value\">{:.2} dB</div></div>", avg_psnr)?;
        writeln!(writer, "      <div class=\"summary-item\"><div class=\"summary-label\">平均 SSIM</div><div class=\"summary-value\">{:.4}</div></div>", avg_ssim)?;
        writeln!(writer, "      <div class=\"summary-item\"><div class=\"summary-label\">平均 VMAF</div><div class=\"summary-value\">{:.1}</div></div>", avg_vmaf)?;
        writeln!(writer, "    </div>")?;
        
        // 表格
        writeln!(writer, "    <table>")?;
        writeln!(writer, "      <thead><tr>")?;
        writeln!(writer, "        <th>#</th>")?;
        writeln!(writer, "        <th>原文件名</th>")?;
        writeln!(writer, "        <th>压缩文件名</th>")?;
        writeln!(writer, "        <th>压缩比</th>")?;
        writeln!(writer, "        <th>PSNR</th>")?;
        writeln!(writer, "        <th>SSIM</th>")?;
        writeln!(writer, "        <th>VMAF</th>")?;
        writeln!(writer, "        <th>状态</th>")?;
        writeln!(writer, "      </tr></thead>")?;
        writeln!(writer, "      <tbody>")?;

        let mut count = 0;
        for record in &filtered {
            let compression = record.compression_ratio
                .map(|v| format!("{:.1}%", v))
                .unwrap_or_default();

            let status_class = match record.status.to_string().as_str() {
                "completed" => "status-completed",
                "failed" => "status-failed",
                "running" => "status-running",
                "skipped" => "status-skipped",
                _ => "status-pending",
            };

            writeln!(writer, "      <tr>")?;
            writeln!(writer, "        <td>{}</td>", count + 1)?;
            writeln!(writer, "        <td>{}</td>", escape_html(&record.ref_filename))?;
            writeln!(writer, "        <td>{}</td>", escape_html(record.dist_filename.as_deref().unwrap_or("-")))?;
            writeln!(writer, "        <td class=\"metric\">{}</td>", compression)?;
            writeln!(writer, "        <td class=\"metric\">{:.2}</td>", record.psnr.unwrap_or(0.0))?;
            writeln!(writer, "        <td class=\"metric\">{:.4}</td>", record.ssim.unwrap_or(0.0))?;
            writeln!(writer, "        <td class=\"metric\">{:.1}</td>", record.vmaf.unwrap_or(0.0))?;
            writeln!(writer, "        <td class=\"{}\">{}</td>", status_class, record.status)?;
            writeln!(writer, "      </tr>")?;
            count += 1;
        }
        
        writeln!(writer, "      </tbody>")?;
        writeln!(writer, "    </table>")?;
        writeln!(writer, "    <div class=\"footer\">生成时间: {}</div>", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))?;
        writeln!(writer, "  </div>")?;
        writeln!(writer, "</body>")?;
        writeln!(writer, "</html>")?;
        
        writer.flush()?;
        info!("HTML 导出完成: {} 条记录 -> {}", filtered.len(), options.output_path);
        Ok(filtered.len())
    }
}

/// 导出记录
pub fn export_records(records: &[ComparisonRecord], options: &ExportOptions) -> Result<usize, Box<dyn std::error::Error>> {
    match options.format {
        ExportFormat::Csv => CsvExporter.export(records, options),
        ExportFormat::Json => JsonExporter.export(records, options),
        ExportFormat::Html => HtmlExporter.export(records, options),
    }
}

// ============ 辅助函数 ============

fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn format_bitrate(bps: u64) -> String {
    const KBPS: u64 = 1000;
    const MBPS: u64 = KBPS * 1000;
    
    if bps >= MBPS {
        format!("{:.2} Mbps", bps as f64 / MBPS as f64)
    } else if bps >= KBPS {
        format!("{:.2} Kbps", bps as f64 / KBPS as f64)
    } else {
        format!("{} bps", bps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ProcessingStatus;
    
    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1048576), "1.00 MB");
        assert_eq!(format_size(1073741824), "1.00 GB");
    }
    
    #[test]
    fn test_format_bitrate() {
        assert_eq!(format_bitrate(500), "500 bps");
        assert_eq!(format_bitrate(1500000), "1.50 Mbps");
    }
    
    #[test]
    fn test_escape_csv() {
        assert_eq!(escape_csv("hello"), "hello");
        assert_eq!(escape_csv("hello,world"), "\"hello,world\"");
        assert_eq!(escape_csv("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
