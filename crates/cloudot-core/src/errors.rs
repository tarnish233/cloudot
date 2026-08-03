//! 机器可读的错误分类。
//!
//! 内部一律用 `anyhow`，但 GUI 和 Agent 需要区分错误**种类**才能给出不同的处置
//! 引导：「被别的进程锁住了，等一下」和「扫到凭据，确认后加 --allow-secrets」
//! 在界面上完全是两回事。
//!
//! 做法是把一个带 kind 的错误塞进 anyhow 的错误链，输出 JSON 时再 downcast 取回。
//! 这样既不用把整个代码库改成自定义错误类型，又能在边界上还原分类。

use serde::Serialize;
use std::fmt;

pub const SCHEMA: &str = "cloudot.error/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// 还没跑过 `cloudot init`
    NotInitialized,
    /// 另一个 cloudot 进程正在操作
    Locked,
    /// 扫到疑似凭据，需要 `--allow-secrets`
    SecretsDetected,
    /// 本地已有内容，需要 `--force`
    NeedsForce,
    /// 拉取远端时冲突（已自动回滚）
    PullConflict,
    /// 没有这个应用定义
    UnknownApp,
    /// 本机没检测到这个应用
    NotDetected,
    /// 这个应用没有被纳管
    NotAdopted,
    /// 目标路径被别的工具的软链占着
    ForeignSymlink,
    /// 当前版本不支持的目标（如目录、家目录之外的路径）
    Unsupported,
    /// 没有更具体的分类
    Other,
}

impl ErrorKind {
    /// 给界面用的一句话说明。
    pub fn summary(self) -> &'static str {
        match self {
            ErrorKind::NotInitialized => "还没初始化",
            ErrorKind::Locked => "另一个 cloudot 进程正在操作",
            ErrorKind::SecretsDetected => "扫到疑似凭据",
            ErrorKind::NeedsForce => "需要显式确认覆盖",
            ErrorKind::PullConflict => "远端有冲突改动（已自动回滚）",
            ErrorKind::UnknownApp => "没有这个应用定义",
            ErrorKind::NotDetected => "本机没检测到这个应用",
            ErrorKind::NotAdopted => "这个应用没有被纳管",
            ErrorKind::ForeignSymlink => "路径被别的工具管着",
            ErrorKind::Unsupported => "当前版本不支持",
            ErrorKind::Other => "操作失败",
        }
    }
}

#[derive(Debug)]
struct Tagged {
    kind: ErrorKind,
    message: String,
}

impl fmt::Display for Tagged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Tagged {}

/// 造一个带分类的错误。
pub fn tagged(kind: ErrorKind, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Tagged {
        kind,
        message: message.into(),
    })
}

/// 从错误链里取回分类；没有标记过的算 `Other`。
pub fn kind_of(error: &anyhow::Error) -> ErrorKind {
    for cause in error.chain() {
        if cause.downcast_ref::<PullConflictError>().is_some() {
            return ErrorKind::PullConflict;
        }
        if let Some(t) = cause.downcast_ref::<Tagged>() {
            return t.kind;
        }
    }
    ErrorKind::Other
}

/// 拉取冲突时附带的结构化信息，供 GUI 展示 diff 并让用户选边。
///
/// 冲突发生后 store 已经 `rebase --abort` 干净；这里的 diff 是
/// `HEAD` vs `origin/<branch>` 的对比，不是带 `<<<<<<<` 的工作树。
#[derive(Debug, Clone, Serialize)]
pub struct ConflictReport {
    /// 本地分支名
    pub branch: String,
    /// 对比的远端 ref，如 `origin/main`
    pub remote_ref: String,
    pub files: Vec<ConflictFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictFile {
    /// 相对 store 根的路径
    pub path: String,
    /// unified diff 文本（可能被截断）
    pub diff: String,
    pub truncated: bool,
}

/// 单文件 diff 上限。超出就截断并标 `truncated`，避免把整份大配置灌进 JSON。
pub const CONFLICT_DIFF_LIMIT: usize = 64 * 1024;

/// `cloudot --json` 出错时的输出体。
#[derive(Debug, Serialize)]
pub struct ErrorReport {
    pub kind: ErrorKind,
    /// 给人看的完整信息，含处置建议（就是终端里会打印的那段）
    pub message: String,
    /// 一句话摘要，界面上做标题用
    pub summary: &'static str,
    /// 仅 `pull_conflict` 时有：冲突文件列表 + 每文件 diff
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<ConflictReport>,
}

/// 带结构化冲突报告的错误，塞进 anyhow 链后由 [`ErrorReport::from`] 取回。
#[derive(Debug)]
pub struct PullConflictError {
    pub report: ConflictReport,
    pub message: String,
}

impl fmt::Display for PullConflictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PullConflictError {}

impl ErrorReport {
    pub fn from(error: &anyhow::Error) -> Self {
        let kind = kind_of(error);
        let conflict = error.chain().find_map(|cause| {
            cause
                .downcast_ref::<PullConflictError>()
                .map(|e| e.report.clone())
        });
        Self {
            kind,
            message: format!("{error:#}"),
            summary: kind.summary(),
            conflict,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_survives_context_wrapping() {
        let err = tagged(ErrorKind::Locked, "被锁了")
            .context("外层说明")
            .context("更外层");
        assert_eq!(kind_of(&err), ErrorKind::Locked);
    }

    #[test]
    fn untagged_errors_fall_back_to_other() {
        let err = anyhow::anyhow!("随便一个错");
        assert_eq!(kind_of(&err), ErrorKind::Other);
    }

    #[test]
    fn report_keeps_full_message_chain() {
        let err = tagged(ErrorKind::NeedsForce, "本地有内容").context("纳管 fish 失败");
        let report = ErrorReport::from(&err);
        assert_eq!(report.kind, ErrorKind::NeedsForce);
        assert!(report.message.contains("纳管 fish 失败"));
        assert!(report.message.contains("本地有内容"));
    }
}
