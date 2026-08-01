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
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<Tagged>().map(|t| t.kind))
        .unwrap_or(ErrorKind::Other)
}

/// `cloudot --json` 出错时的输出体。
#[derive(Debug, Serialize)]
pub struct ErrorReport {
    pub kind: ErrorKind,
    /// 给人看的完整信息，含处置建议（就是终端里会打印的那段）
    pub message: String,
    /// 一句话摘要，界面上做标题用
    pub summary: &'static str,
}

impl ErrorReport {
    pub fn from(error: &anyhow::Error) -> Self {
        let kind = kind_of(error);
        Self {
            kind,
            message: format!("{error:#}"),
            summary: kind.summary(),
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
