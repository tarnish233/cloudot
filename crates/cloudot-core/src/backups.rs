//! `~/.cloudot/backups` 的盘点与清理。
//!
//! 备份不只是心理安慰：它是孤儿软链自愈的兜底数据源（git 历史取不到时用它），
//! 所以清理必须保守 —— 默认只按「保留最近 N 份」删，而且从不自动执行。

use crate::Layout;
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// 默认保留份数。
pub const DEFAULT_KEEP: usize = 20;

#[derive(Debug, Clone, Serialize)]
pub struct BackupEntry {
    /// 目录名，即 `%Y%m%d-%H%M%S` 形式的时间戳
    pub stamp: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupSet {
    /// 新的在前
    pub entries: Vec<BackupEntry>,
    pub total_files: usize,
    pub total_bytes: u64,
}

pub const SCHEMA: &str = "cloudot.backups/v1";

/// 盘点所有备份，新的在前。
pub fn list(layout: &Layout) -> Result<BackupSet> {
    let root = layout.backups();
    let mut entries: Vec<BackupEntry> = match fs::read_dir(&root) {
        Ok(dir) => dir
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| {
                let (files, bytes) = measure(&e.path());
                BackupEntry {
                    stamp: e.file_name().to_string_lossy().into_owned(),
                    files,
                    bytes,
                }
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("读取 {} 失败", root.display())),
    };
    // 时间戳格式固定宽度，字典序倒排即时间倒排
    entries.sort_by(|a, b| b.stamp.cmp(&a.stamp));

    let total_files = entries.iter().map(|e| e.files).sum();
    let total_bytes = entries.iter().map(|e| e.bytes).sum();
    Ok(BackupSet {
        entries,
        total_files,
        total_bytes,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct PruneOutcome {
    pub removed: Vec<BackupEntry>,
    pub kept: usize,
    pub freed_bytes: u64,
}

/// 删除多余的备份。
///
/// 一份备份只有在**同时**满足「不在最近 `keep` 份之内」和「早于 `older_than_days`
/// （若指定）」时才会被删。两个条件取交集而不是并集，是为了让加了天数限制之后
/// 只会删得更少、不会更多。
pub fn prune(layout: &Layout, keep: usize, older_than_days: Option<u64>) -> Result<PruneOutcome> {
    let set = list(layout)?;
    let cutoff = older_than_days.map(|days| {
        chrono::Local::now() - chrono::Duration::days(days as i64)
    });

    let mut removed = Vec::new();
    let mut freed_bytes = 0;
    for (idx, entry) in set.entries.iter().enumerate() {
        if idx < keep {
            continue;
        }
        if let Some(cutoff) = cutoff {
            match parse_stamp(&entry.stamp) {
                // 够老，可以删
                Some(taken) if taken < cutoff => {}
                // 还不够老，或者时间戳看不懂 —— 两种情况都保守留着
                _ => continue,
            }
        }
        let dir = layout.backups().join(&entry.stamp);
        fs::remove_dir_all(&dir)
            .with_context(|| format!("删除备份 {} 失败", dir.display()))?;
        freed_bytes += entry.bytes;
        removed.push(entry.clone());
    }

    Ok(PruneOutcome {
        kept: set.entries.len() - removed.len(),
        freed_bytes,
        removed,
    })
}

/// 解析 `%Y%m%d-%H%M%S`；解析不了就当它不满足「够老」，宁可留着。
fn parse_stamp(stamp: &str) -> Option<chrono::DateTime<chrono::Local>> {
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").ok()?;
    chrono::Local.from_local_datetime(&naive).single()
}

/// 递归统计 (文件数, 字节数)。
fn measure(dir: &Path) -> (usize, u64) {
    let mut files = 0;
    let mut bytes = 0;
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(read) = fs::read_dir(&current) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(ft) if ft.is_file() => {
                    files += 1;
                    bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
                _ => {}
            }
        }
    }
    (files, bytes)
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempHome;

    fn make_backup(layout: &Layout, stamp: &str, body: &str) {
        let dir = layout.backups().join(stamp).join(".config/ghostty");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config"), body).unwrap();
    }

    #[test]
    fn list_is_empty_when_dir_absent() {
        let home = TempHome::new("bk-absent");
        let set = list(&home.layout()).unwrap();
        assert!(set.entries.is_empty());
        assert_eq!(set.total_bytes, 0);
    }

    #[test]
    fn list_sorts_newest_first_and_measures() {
        let home = TempHome::new("bk-list");
        let layout = home.layout();
        make_backup(&layout, "20260101-000000", "old");
        make_backup(&layout, "20260301-000000", "newer");

        let set = list(&layout).unwrap();
        assert_eq!(set.entries[0].stamp, "20260301-000000");
        assert_eq!(set.total_files, 2);
        assert_eq!(set.total_bytes, 8); // "old" + "newer"
    }

    #[test]
    fn prune_keeps_newest_n() {
        let home = TempHome::new("bk-keep");
        let layout = home.layout();
        for stamp in ["20260101-000000", "20260201-000000", "20260301-000000"] {
            make_backup(&layout, stamp, "x");
        }

        let out = prune(&layout, 1, None).unwrap();
        assert_eq!(out.removed.len(), 2);
        assert_eq!(out.kept, 1);
        assert_eq!(list(&layout).unwrap().entries[0].stamp, "20260301-000000");
    }

    #[test]
    fn prune_with_keep_covering_everything_removes_nothing() {
        let home = TempHome::new("bk-noop");
        let layout = home.layout();
        make_backup(&layout, "20260101-000000", "x");
        let out = prune(&layout, DEFAULT_KEEP, None).unwrap();
        assert!(out.removed.is_empty());
    }

    /// 加了天数限制之后只会删得更少 —— 近期的备份即使超出 keep 也要留。
    #[test]
    fn age_filter_only_narrows_what_gets_removed() {
        let home = TempHome::new("bk-age");
        let layout = home.layout();
        let recent = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        make_backup(&layout, "20200101-000000", "old");
        make_backup(&layout, &recent, "new");

        // keep=0 但要求「早于 365 天」：只有那份 2020 的该走
        let out = prune(&layout, 0, Some(365)).unwrap();
        assert_eq!(out.removed.len(), 1);
        assert_eq!(out.removed[0].stamp, "20200101-000000");
        assert_eq!(list(&layout).unwrap().entries.len(), 1);
    }

    #[test]
    fn unparseable_stamp_is_kept_when_age_filter_is_on() {
        let home = TempHome::new("bk-badstamp");
        let layout = home.layout();
        make_backup(&layout, "手动备份", "x");
        let out = prune(&layout, 0, Some(1)).unwrap();
        assert!(out.removed.is_empty(), "看不懂时间戳就该保守留着");
    }

    #[test]
    fn human_bytes_reads_sensibly() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
