use crate::git::Git;
use crate::link::{self, LinkState};
use crate::links::{self, OrphanKind};
use crate::{Config, Layout, LinkRecords, Manifest, backups, secrets};
use anyhow::Result;
use serde::Serialize;
use std::fs;

pub const SCHEMA: &str = "cloudot.doctor/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub level: Level,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    /// 没有 error 就算通过（warn 不算失败）。
    pub ok: bool,
    pub checks: Vec<Check>,
}

fn check(name: &str, level: Level, message: impl Into<String>) -> Check {
    Check {
        name: name.to_owned(),
        level,
        message: message.into(),
        hint: None,
    }
}

fn with_hint(mut c: Check, hint: impl Into<String>) -> Check {
    c.hint = Some(hint.into());
    c
}

/// 体检。`check_network` 为 true 时会做一次 remote 可达性探测（走网络）。
pub fn run(layout: &Layout, check_network: bool) -> Result<Report> {
    let mut checks = Vec::new();

    // 1. 初始化状态
    //
    // 未初始化是产品空态，不是故障。用 Warn 而不是 Error：GUI 空态页会画引导，
    // doctor 不应再把它渲染成「发现需要处理的问题」主叙事。`ok` 仍为 true（warn
    // 不算失败），和「还没纳管任何应用」那条 warn 一个量级。
    if !Config::exists(layout) {
        checks.push(with_hint(
            check("config", Level::Warn, "还没初始化"),
            "在 App 里完成设置，或跑 `cloudot init [--remote <url>]`",
        ));
        return Ok(Report { ok: true, checks });
    }
    let config = Config::load(layout)?;
    checks.push(check(
        "config",
        Level::Ok,
        format!("设备 {} · root {}", config.device, layout.root().display()),
    ));

    // 2. store 是不是 git 仓库
    let git = Git::new(layout.store());
    if !git.is_repo() {
        checks.push(with_hint(
            check(
                "store",
                Level::Error,
                format!("{} 不是 git 仓库", layout.store().display()),
            ),
            "跑 `cloudot init` 重建",
        ));
    } else {
        checks.push(check(
            "store",
            Level::Ok,
            format!(
                "git 仓库正常（{} @ {}）",
                git.branch().unwrap_or_else(|| "无分支".into()),
                git.head_short().unwrap_or_else(|| "无提交".into())
            ),
        ));

        // 3. git 身份
        match git.identity() {
            Some(id) => checks.push(check("git-identity", Level::Ok, id)),
            None => checks.push(with_hint(
                check("git-identity", Level::Warn, "git 没配 user.name/user.email"),
                "cloudot 会用 cloudot@localhost 兜底；建议 `git config --global user.email ...`",
            )),
        }

        // 4. remote
        match git.remote() {
            None => checks.push(with_hint(
                check("remote", Level::Warn, "没配 remote，只能本地留历史"),
                "跑 `cloudot init --remote git@github.com:<you>/dotfiles.git`",
            )),
            Some(url) => {
                if check_network {
                    match git.remote_reachable() {
                        Some(true) => {
                            checks.push(check("remote", Level::Ok, format!("{url} 可达")))
                        }
                        Some(false) => checks.push(with_hint(
                            check("remote", Level::Warn, format!("{url} 不可达")),
                            "可能只是断网或 SSH key 没加载，`ssh -T git@github.com` 验证一下",
                        )),
                        None => checks.push(check("remote", Level::Ok, url)),
                    }
                } else {
                    checks.push(check("remote", Level::Ok, url));
                }
            }
        }

        // 5. 未推送的改动
        if let Some((ahead, behind)) = git.ahead_behind() {
            if ahead > 0 || behind > 0 {
                checks.push(with_hint(
                    check(
                        "sync-state",
                        Level::Warn,
                        format!("本地领先 {ahead} 个提交、落后 {behind} 个"),
                    ),
                    "跑 `cloudot sync`",
                ));
            } else {
                checks.push(check("sync-state", Level::Ok, "与 remote 一致"));
            }
        }
        let dirty = git.dirty_files().unwrap_or_default();
        if !dirty.is_empty() {
            checks.push(with_hint(
                check(
                    "uncommitted",
                    Level::Warn,
                    format!("store 有 {} 处未提交改动", dirty.len()),
                ),
                "跑 `cloudot sync` 提交并推送",
            ));
        }
    }

    // 6. 孤儿软链 —— manifest 里已经没了、本机链还在，最容易静默损坏
    let manifest = Manifest::load(layout)?;
    let records = LinkRecords::load(layout)?;
    let orphans = links::find_orphans(layout, &manifest, &records);
    for orphan in &orphans {
        let level = match orphan.kind {
            OrphanKind::Dangling => Level::Error,
            OrphanKind::Unmanaged => Level::Warn,
        };
        checks.push(with_hint(
            check(
                &format!("orphan:{}", orphan.target),
                level,
                orphan.kind.describe(),
            ),
            "跑 `cloudot apply` —— 会从 git 历史或备份取回内容，还原成实体文件",
        ));
    }

    // 7. 逐文件链接健康度 —— symlink 策略下唯一真正会出问题的地方
    if manifest.apps.is_empty() && orphans.is_empty() {
        checks.push(with_hint(
            check("links", Level::Warn, "还没纳管任何应用"),
            "跑 `cloudot add ghostty`",
        ));
    }
    for app in &manifest.apps {
        for file in &app.files {
            let target = layout.expand(&file.target);
            let store = layout.store_path(&file.store);
            let state = link::inspect(&target, &store);
            let name = format!("link:{}", file.target);
            match state {
                LinkState::Linked => {
                    checks.push(check(&name, Level::Ok, state.describe()));
                }
                LinkState::Missing => checks.push(with_hint(
                    check(&name, Level::Warn, state.describe()),
                    "跑 `cloudot apply` 建立软链",
                )),
                LinkState::ReplacedByFile => checks.push(with_hint(
                    check(&name, Level::Warn, state.describe()),
                    format!(
                        "{} 可能用「替换写入」顶掉了软链，本地那份也许比 store 新。\
                         先 `diff {} {}` 比一下，再决定是 `cloudot apply --force`（store 覆盖本地）\
                         还是手动把本地内容拷进 store。",
                        app.name,
                        target.display(),
                        store.display()
                    ),
                )),
                LinkState::ForeignSymlink => checks.push(with_hint(
                    check(&name, Level::Error, state.describe()),
                    "该路径被别的工具管着，cloudot 不会动它；确认后手动清理",
                )),
                LinkState::StoreMissing => checks.push(with_hint(
                    check(&name, Level::Error, state.describe()),
                    "store 里内容不见了，检查 `cloudot sync` 是否成功、或从备份恢复",
                )),
            }
        }
    }

    // 8. 明文凭据与备份体积
    secret_checks(layout, &manifest, &mut checks);
    backup_checks(layout, &mut checks);

    let ok = !checks.iter().any(|c| c.level == Level::Error);
    Ok(Report { ok, checks })
}

/// 扫 store 里的明文凭据。
///
/// 报 Error 而不是 Warn：内容已经在 git 仓库里了，而且大概率已经推到远端。
fn secret_checks(layout: &Layout, manifest: &Manifest, checks: &mut Vec<Check>) {
    let mut findings = Vec::new();
    for app in &manifest.apps {
        for file in &app.files {
            let store = layout.store_path(&file.store);
            if fs::symlink_metadata(&store).is_ok() {
                findings.extend(secrets::scan_file(&store, &file.target));
            }
        }
    }

    if findings.is_empty() {
        if !manifest.apps.is_empty() {
            checks.push(check("secrets", Level::Ok, "store 里没扫到明文凭据"));
        }
        return;
    }
    for finding in &findings {
        let where_ = match finding.line {
            Some(n) => format!("{}:{n}", finding.path),
            None => finding.path.clone(),
        };
        checks.push(with_hint(
            check(
                "secret",
                Level::Error,
                format!("{where_} —— {}", finding.reason),
            ),
            "这份内容已经进了 git。先 `cloudot unadopt <app>` 摘出去，\
             并按泄漏处理该凭据 —— git 历史里仍然留有记录",
        ));
    }
}

/// 备份体积检查。备份是孤儿自愈的兜底数据源，所以只提醒、不自动删。
fn backup_checks(layout: &Layout, checks: &mut Vec<Check>) {
    const MANY: usize = 30;
    const BIG: u64 = 50 * 1024 * 1024;

    let Ok(set) = backups::list(layout) else {
        return;
    };
    if set.entries.is_empty() {
        return;
    }
    let summary = format!(
        "{} 份备份，共 {}",
        set.entries.len(),
        backups::human_bytes(set.total_bytes)
    );
    if set.entries.len() > MANY || set.total_bytes > BIG {
        checks.push(with_hint(
            check("backups", Level::Warn, summary),
            "跑 `cloudot backups prune` 清理（默认保留最近 20 份）",
        ));
    } else {
        checks.push(check("backups", Level::Ok, summary));
    }
}
