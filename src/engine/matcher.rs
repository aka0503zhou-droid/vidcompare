//! 文件匹对模块
//! 
//! 高效的字符串匹配算法，支持中英文后缀

use super::{FilePair, VideoFile};
use crate::config::COMPRESSION_SUFFIXES;
use std::collections::HashSet;

/// 文件匹对器
pub struct Matcher {
    suffixes: Vec<String>,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Matcher {
    pub fn new() -> Self {
        let mut suffixes: Vec<String> =
            COMPRESSION_SUFFIXES.iter().map(|s| s.to_string()).collect();
        
        suffixes.sort_by(|a, b| b.len().cmp(&a.len()));
        suffixes.dedup();
        
        Self { suffixes }
    }

    /// 匹对两个目录的文件
    pub fn match_files(
        &self,
        reference_files: &[VideoFile],
        distorted_files: &[VideoFile],
    ) -> Vec<FilePair> {
        let mut pairs = Vec::with_capacity(reference_files.len());
        
        // 构建 dist 文件名到 index 的映射
        let dist_map: HashSet<String> = distorted_files.iter()
            .map(|d| d.name.clone())
            .collect();
        
        for (idx, ref_file) in reference_files.iter().enumerate() {
            let ref_name = ref_file
                .name
                .trim_end_matches(".mp4")
                .trim_end_matches(".MP4")
                .trim_end_matches(".mkv")
                .trim_end_matches(".MKV")
                .trim_end_matches(".avi")
                .trim_end_matches(".AVI")
                .trim_end_matches(".mov")
                .trim_end_matches(".MOV")
                .trim_end_matches(".webm")
                .trim_end_matches(".WEBM");
            
            // 1. 尝试直接匹配 (同名文件)
            let dist_file = if dist_map.contains(&ref_file.name) {
                distorted_files.iter().find(|d| d.name == ref_file.name).cloned()
            } else {
                // 2. 尝试后缀匹配
                self.find_matching_dist(ref_name, &dist_map, distorted_files)
            };
            
            let selected = dist_file.is_some();
            let dist_for_pair = dist_file.clone();
            pairs.push(FilePair {
                index: (idx + 1) as u32,
                reference: Some(ref_file.clone()),
                distorted: dist_for_pair,
                selected,
                ref_file: ref_file.clone(),
                dist_file: dist_file.unwrap_or_else(|| ref_file.clone()),
            });
        }
        
        pairs
    }

    /// 查找匹配的压缩文件
    #[inline]
    fn find_matching_dist(
        &self,
        ref_name: &str,
        dist_set: &HashSet<String>,
        distorted_files: &[VideoFile],
    ) -> Option<VideoFile> {
        for suffix in &self.suffixes {
            let candidate = format!("{}{}", ref_name, suffix);
            
            if dist_set.contains(&candidate) {
                return distorted_files.iter().find(|d| d.name == candidate).cloned();
            }
            
            // 也检查带扩展名的情况
            for ext in &["mp4", "mkv", "avi", "mov", "webm"] {
                let candidate_with_ext = format!("{}.{}", candidate, ext);
                if dist_set.contains(&candidate_with_ext) {
                    return distorted_files.iter().find(|d| d.name == candidate_with_ext).cloned();
                }
            }
        }
        
        None
    }

    /// 获取未匹配的压缩文件
    pub fn find_unmatched_distorted(
        &self,
        reference_files: &[VideoFile],
        distorted_files: &[VideoFile],
    ) -> Vec<VideoFile> {
        let pairs = self.match_files(reference_files, distorted_files);
        
        let matched_dist_names: HashSet<_> = pairs
            .iter()
            .filter_map(|p| p.distorted.as_ref())
            .map(|d| d.name.clone())
            .collect();
        
        distorted_files
            .iter()
            .filter(|dist| !matched_dist_names.contains(&dist.name))
            .cloned()
            .collect()
    }
}

/// 快速提取文件名 (不含扩展名)
#[inline]
pub fn strip_extension(filename: &str) -> &str {
    filename
        .strip_suffix(".mp4")
        .or_else(|| filename.strip_suffix(".MP4"))
        .or_else(|| filename.strip_suffix(".mkv"))
        .or_else(|| filename.strip_suffix(".MKV"))
        .or_else(|| filename.strip_suffix(".avi"))
        .or_else(|| filename.strip_suffix(".AVI"))
        .or_else(|| filename.strip_suffix(".mov"))
        .or_else(|| filename.strip_suffix(".MOV"))
        .or_else(|| filename.strip_suffix(".webm"))
        .or_else(|| filename.strip_suffix(".WEBM"))
        .unwrap_or(filename)
}

/// 检测文件名是否包含压缩相关后缀
#[inline]
pub fn has_compression_suffix(filename: &str) -> bool {
    let name = strip_extension(filename);
    COMPRESSION_SUFFIXES
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// 获取文件的基础名 (去掉所有后缀)
pub fn get_base_name(filename: &str) -> String {
    let name = strip_extension(filename);
    
    let mut base = name.to_string();
    let mut changed = true;
    while changed {
        changed = false;
        for suffix in COMPRESSION_SUFFIXES {
            if base.ends_with(suffix) {
                base = base[..base.len() - suffix.len()].to_string();
                changed = true;
            }
        }
    }
    
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_strip_extension() {
        assert_eq!(strip_extension("video.mp4"), "video");
        assert_eq!(strip_extension("video.MP4"), "video");
        assert_eq!(strip_extension("video.mkv"), "video");
        assert_eq!(strip_extension("video.tar.gz"), "video.tar");
    }
    
    #[test]
    fn test_has_compression_suffix() {
        assert!(has_compression_suffix("video_hc.mp4"));
        assert!(has_compression_suffix("video_高压缩.mkv"));
        assert!(!has_compression_suffix("video.mp4"));
    }
    
    #[test]
    fn test_get_base_name() {
        assert_eq!(get_base_name("video_hc.mp4"), "video");
        assert_eq!(get_base_name("video_高压缩.mp4"), "video");
    }
}
