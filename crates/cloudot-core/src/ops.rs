use crate::config::{default_device_name, timestamp};
use crate::git::{Git, PullOutcome};
use crate::link::{self, AdoptAction, AdoptReport, LinkState};
use crate::links::{Orphan, OrphanKind};
use crate::manifest::{ManagedApp, ManagedFile, Strategy};
use crate::{Config, Layout, LinkRecords, Lock, Manifest, adopter, links, secrets};
use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// 各命令 `--json` 输出信封里的 schema 值。加字段兼容，改语义要升版本。
pub const INIT_SCHEMA: &str = "cloudot.init/v1";
pub const ADD_SCHEMA: &str = "cloudot.add/v1";
pub const APPLY_SCHEMA: &str = "cloudot.apply/v1";
pub const SYNC_SCHEMA: &str = "cloudot.sync/v1";
pub const UNADOPT_SCHEMA: &str = "cloudot.unadopt/v1";
pub const APPS_SCHEMA: &str = "cloudot.apps/v1";

// ---------------------------------------------------------------- init

#[derive(Debug, serde::Serialize)]
pub struct InitOutcome {
    pub root: PathBuf,
    pub device: String,
    pub remote: Option<String>,
    pub cloned: bool,
    pub already: bool,
    /// clone 回来的 store 里已有多少个纳管条目（为 0 说明远端还是空的）。
    pub apps_in_store: usize,
}

/// 建好 `~/.cloudot`，把 store 变成 git 仓库。可重复执行。
pub fn init(layout: &Layout, remote: Option<&str>, device: Option<&str>) -> Result<InitOutcome> {
    let _lock = Lock::acquire(layout)?;
    let already = Config::exists(layout);

    fs::create_dir_all(layout.root())
        .with_context(|| format!("创建 {} 失败", layout.root().display()))?;
    fs::create_dir_all(layout.backups())?;
    fs::create_dir_all(layout.adopters_dir())?;

    let store = layout.store();
    let git = Git::new(&store);
    let mut cloned = false;

    if !git.is_repo() {
        let store_empty = match fs::read_dir(&store) {
            Ok(mut it) => it.next().is_none(),
            Err(_) => true,
        };
        match remote {
            Some(url) if store_empty => {
                // clone 要求目标不存在或为空；先把空目录让出来
                let _ = fs::remove_dir(&store);
                Git::clone_into(url, &store)?;
                cloned = true;
            }
            _ => {
                if !store_empty {
                    bail!(
                        "{} 已存在内容但不是 git 仓库，先手动处理再 init",
                        store.display()
                    );
                }
                git.init()?;
            }
        }
    }

    if let Some(url) = remote {
        git.set_remote(url)?;
    }

    // clone 回来的仓库可能已经有 manifest，不要覆盖
    if !layout.manifest_file().exists() {
        Manifest::default().save(layout)?;
    }

    // commit_all 用的是 `git add -A`，而在 Finder 里点一下 store 就会多出 .DS_Store
    let gitignore = store.join(".gitignore");
    if !gitignore.exists() {
        fs::write(
            &gitignore,
            "# cloudot: 别让 macOS 与编辑器的产物混进配置仓库\n.DS_Store\n._*\n*.swp\n*~\n",
        )
        .with_context(|| format!("写入 {} 失败", gitignore.display()))?;
    }

    let device = match device {
        Some(d) => d.to_owned(),
        None if already => Config::load(layout)?.device,
        None => default_device_name(),
    };
    let remote_url = remote.map(str::to_owned).or_else(|| git.remote());
    Config::new(device.clone(), remote_url.clone()).save(layout)?;

    git.commit_all(&format!("cloudot: init on {device}"))?;

    Ok(InitOutcome {
        root: layout.root().to_path_buf(),
        device,
        remote: remote_url,
        cloned,
        already,
        apps_in_store: Manifest::load(layout)?.apps.len(),
    })
}

// ---------------------------------------------------------------- add

#[derive(Debug, serde::Serialize)]
pub struct FileOutcome {
    pub target: String,
    pub store: String,
    pub action: AdoptAction,
    pub backup: Option<PathBuf>,
}

#[derive(Debug, serde::Serialize)]
pub struct AddOutcome {
    pub id: String,
    pub name: String,
    pub files: Vec<FileOutcome>,
    pub commit: Option<String>,
    /// 这次是预演（`--dry-run`），什么都没真的做。
    ///
    /// 加字段而不是改 `action` 的语义：消费方旧代码读到的 `action` 仍然是
    /// 「发生了什么」，只是要配合这个字段读成「将会发生什么」。
    #[serde(default)]
    pub dry_run: bool,
}

/// 纳管一个应用：备份本地配置 → 移进 store → 建软链 → 记 manifest → 提交。
///
/// 要么全部路径都成功，要么整体回滚。半纳管状态（文件已经被链进 store 但 manifest
/// 和 links.toml 都没记）是最难收拾的：`status` 看不见它，`unadopt` 也撤不掉。
///
/// `dry_run` 时**所有校验照常跑**（检测门禁、凭据扫描、逐路径可行性），只是不动
/// 文件、不写 manifest/links.toml、不提交 —— 预演的价值恰恰在于让这些校验先说话。
pub fn add(
    layout: &Layout,
    app_id: &str,
    force: bool,
    allow_secrets: bool,
    dry_run: bool,
) -> Result<AddOutcome> {
    let _lock = Lock::acquire(layout)?;
    let config = Config::load(layout)?;
    let ad = adopter::get(layout, app_id)?;
    let mut manifest = Manifest::load(layout)?;
    let mut records = LinkRecords::load(layout)?;
    let stamp = timestamp();

    let store_has_any = ad.paths.iter().any(|p| {
        layout
            .store_rel_for(&layout.expand(&p.path))
            .map(|rel| layout.store_path(&rel).exists())
            .unwrap_or(false)
    });
    if !ad.detected(layout) && !store_has_any {
        return Err(crate::tagged(
            crate::ErrorKind::NotDetected,
            format!(
                "本机没检测到 {}（找过：{}），store 里也没有它的配置",
                ad.name,
                ad.detect.join("、")
            ),
        ));
    }

    // 凭据检查放在动手之前，而且是全部路径一起看：宁可整体拒绝，
    // 也不要纳管了一半才发现有 token 要往 git 里推。
    if !allow_secrets {
        let mut findings = Vec::new();
        for path in &ad.paths {
            let target = layout.expand(&path.path);
            let display = layout.contract(&target);
            if fs::symlink_metadata(&target).is_ok() {
                findings.extend(secrets::scan_file(&target, &display));
            } else {
                findings.extend(secrets::scan_path(&display));
            }
        }
        if !findings.is_empty() {
            let list = findings
                .iter()
                .map(|f| match f.line {
                    Some(n) => format!("  {}:{n} —— {}", f.path, f.reason),
                    None => format!("  {} —— {}", f.path, f.reason),
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Err(crate::tagged(
                crate::ErrorKind::SecretsDetected,
                format!(
                    "{} 的配置里像是有凭据，没有纳管：\n{list}\n\n\
                     这些内容一旦提交就会进 git 历史，之后很难真正删掉。\n\
                     确认无害的话用 --allow-secrets 强制纳管。",
                    ad.name
                ),
            ));
        }
    }

    let mut files = Vec::new();
    let mut managed = Vec::new();
    let mut done: Vec<(PathBuf, PathBuf, AdoptReport)> = Vec::new();
    let mut failure: Option<anyhow::Error> = None;

    for path in &ad.paths {
        let target = layout.expand(&path.path);
        // 路径计算与写入分开：预演要拿到 store 位置和预测结论，但不能动文件。
        let step = layout.store_rel_for(&target).and_then(|store_rel| {
            let store_abs = layout.store_path(&store_rel);
            if dry_run {
                link::plan_adopt(layout, &target, &store_abs, force).map(|action| {
                    let report = AdoptReport {
                        action,
                        backup: None,
                    };
                    (store_rel, store_abs, report)
                })
            } else {
                link::adopt_file(layout, &target, &store_abs, &stamp, force)
                    .map(|report| (store_rel, store_abs, report))
            }
        });

        match step {
            Ok((store_rel, store_abs, report)) => {
                let target_repr = layout.contract(&target);
                records.upsert(&ad.id, &target_repr, &store_rel);
                files.push(FileOutcome {
                    target: target_repr.clone(),
                    store: store_rel.clone(),
                    action: report.action,
                    backup: report.backup.clone(),
                });
                managed.push(ManagedFile {
                    target: target_repr,
                    store: store_rel,
                    strategy: path.strategy,
                });
                done.push((target, store_abs, report));
            }
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }

    if let Some(err) = failure {
        // 预演没动过东西，没有可回滚的；直接把校验结论抛出去。
        if dry_run {
            return Err(err.context(format!("预演：纳管 {} 会失败", ad.name)));
        }
        let mut unwind_errors = Vec::new();
        for (target, store, report) in done.iter().rev() {
            if let Err(e) = link::revert_adopt(target, store, report) {
                unwind_errors.push(format!("  {} —— {e:#}", layout.contract(target)));
            }
        }
        return Err(if unwind_errors.is_empty() {
            err.context(format!("纳管 {} 失败，已回滚本次全部改动", ad.name))
        } else {
            err.context(format!(
                "纳管 {} 失败，而且回滚不完整，需要手工检查：\n{}",
                ad.name,
                unwind_errors.join("\n")
            ))
        });
    }

    let adopted_by = manifest
        .app(&ad.id)
        .map(|a| a.adopted_by.clone())
        .unwrap_or_else(|| config.device.clone());
    manifest.upsert(ManagedApp {
        id: ad.id.clone(),
        name: ad.name.clone(),
        adopted_by,
        files: managed,
    });

    if dry_run {
        return Ok(AddOutcome {
            id: ad.id,
            name: ad.name,
            files,
            commit: None,
            dry_run: true,
        });
    }

    manifest.save(layout)?;
    records.save(layout)?;

    let commit = Git::new(layout.store())
        .commit_all(&format!("cloudot: adopt {} on {}", ad.id, config.device))?;

    Ok(AddOutcome {
        id: ad.id,
        name: ad.name,
        files,
        commit,
        dry_run: false,
    })
}

// ---------------------------------------------------------------- apply

#[derive(Debug, serde::Serialize)]
pub struct ApplyItem {
    pub target: String,
    pub before: LinkState,
    pub action: ApplyAction,
    pub backup: Option<PathBuf>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyAction {
    /// 本来就好的，没动。
    AlreadyLinked,
    /// 新建了软链。
    Linked,
    /// 备份本地实体文件后用 store 版本覆盖（需要 --force）。
    Replaced,
    /// 有情况但没动，见 note。
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealSource {
    /// 从 git 历史里取回被删掉的内容。
    GitHistory,
    /// store 文件还在，直接落成实体文件。
    Store,
    /// 从 `~/.cloudot/backups` 里最近的一份取回。
    Backup,
    /// 没能修好，软链保持原样（不删任何东西）。
    Failed,
}

#[derive(Debug, serde::Serialize)]
pub struct HealItem {
    pub app: String,
    pub target: String,
    pub kind: OrphanKind,
    pub source: HealSource,
    pub note: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct ApplyOutcome {
    pub items: Vec<ApplyItem>,
    /// 被修复的孤儿软链（manifest 里已经没有、却还指向 store 的链）。
    pub healed: Vec<HealItem>,
    /// 这次是预演，什么都没真的做。见 [`AddOutcome::dry_run`]。
    #[serde(default)]
    pub dry_run: bool,
}

/// 把 store 里的内容落地到本机 —— 新机器上的主命令。
///
/// 默认不覆盖本地实体文件：那份内容可能比 store 新。要覆盖得显式 `--force`，
/// 且一定先备份。
pub fn apply(layout: &Layout, force: bool, dry_run: bool) -> Result<ApplyOutcome> {
    let _lock = Lock::acquire(layout)?;
    apply_inner(layout, force, dry_run)
}

/// 不加锁的实现。`sync` 已经持有锁，不能再走公开的 [`apply`]
/// —— flock 按打开的文件描述生效，同进程二次加锁一样会冲突。
fn apply_inner(layout: &Layout, force: bool, dry_run: bool) -> Result<ApplyOutcome> {
    let manifest = Manifest::load(layout)?;
    let mut records = LinkRecords::load(layout)?;
    let stamp = timestamp();

    // 先处理孤儿：manifest 里已经没有它们了，留着就是悬空软链，App 读不到配置。
    let orphans = links::find_orphans(layout, &manifest, &records);
    let healed = heal_orphans(layout, &orphans, &mut records, dry_run)?;

    // 真跑时 heal 会把孤儿目标写成实体文件，后面的 inspect 因此看到的是修完的样子。
    // 预演没动过盘，要自己把这批目标记下来，免得报出的 before 和真跑不一致。
    let healed_targets: HashSet<&str> = if dry_run {
        healed
            .iter()
            .filter(|h| h.source != HealSource::Failed)
            .map(|h| h.target.as_str())
            .collect()
    } else {
        HashSet::new()
    };

    let mut items = Vec::new();
    for app in &manifest.apps {
        for file in &app.files {
            let target = layout.expand(&file.target);
            let store = layout.store_path(&file.store);
            let before = if healed_targets.contains(file.target.as_str()) {
                // 预演：heal 会把它还原成实体文件，真跑时 inspect 就会这么看到
                LinkState::ReplacedByFile
            } else {
                link::inspect(&target, &store)
            };

            let item = match before {
                LinkState::Linked => ApplyItem {
                    target: file.target.clone(),
                    before,
                    action: ApplyAction::AlreadyLinked,
                    backup: None,
                    note: None,
                },
                LinkState::Missing => {
                    let report = if dry_run {
                        AdoptReport {
                            action: link::plan_adopt(layout, &target, &store, force)?,
                            backup: None,
                        }
                    } else {
                        link::adopt_file(layout, &target, &store, &stamp, force)?
                    };
                    ApplyItem {
                        target: file.target.clone(),
                        before,
                        action: match report.action {
                            AdoptAction::AlreadyLinked => ApplyAction::AlreadyLinked,
                            _ => ApplyAction::Linked,
                        },
                        backup: report.backup,
                        note: None,
                    }
                }
                LinkState::ReplacedByFile => {
                    if force {
                        let report = if dry_run {
                            AdoptReport {
                                action: link::plan_adopt(layout, &target, &store, true)?,
                                backup: None,
                            }
                        } else {
                            link::adopt_file(layout, &target, &store, &stamp, true)?
                        };
                        ApplyItem {
                            target: file.target.clone(),
                            before,
                            action: ApplyAction::Replaced,
                            backup: report.backup,
                            note: None,
                        }
                    } else {
                        ApplyItem {
                            target: file.target.clone(),
                            before,
                            action: ApplyAction::Skipped,
                            backup: None,
                            note: Some(
                                "本地是实体文件，内容可能比 store 新。先 diff 确认，\
                                 再用 --force 覆盖（会自动备份）。"
                                    .to_owned(),
                            ),
                        }
                    }
                }
                LinkState::ForeignSymlink => ApplyItem {
                    target: file.target.clone(),
                    before,
                    action: ApplyAction::Skipped,
                    backup: None,
                    note: Some("软链指向别处，疑似被其他工具管理，cloudot 不动它".to_owned()),
                },
                LinkState::StoreMissing => ApplyItem {
                    target: file.target.clone(),
                    before,
                    action: ApplyAction::Skipped,
                    backup: None,
                    note: Some(format!("store 里没有 {}", file.store)),
                },
            };

            if item.action != ApplyAction::Skipped {
                records.upsert(&app.id, &file.target, &file.store);
            }
            items.push(item);
        }
    }

    prune_records(layout, &manifest, &mut records);
    if !dry_run {
        records.save(layout)?;
    }
    Ok(ApplyOutcome {
        items,
        healed,
        dry_run,
    })
}

/// 清掉既不在 manifest、也不再指向 store 的陈旧记录。
fn prune_records(layout: &Layout, manifest: &Manifest, records: &mut LinkRecords) {
    let managed: HashSet<&str> = manifest
        .apps
        .iter()
        .flat_map(|a| a.files.iter().map(|f| f.target.as_str()))
        .collect();
    records
        .links
        .retain(|l| managed.contains(l.target.as_str()) || l.still_ours(layout));
}

/// 把孤儿软链还原成实体文件，尽最大努力保住内容。
///
/// 修不好时**不删任何东西** —— 宁可留一个坏链让 `doctor` 继续报警，
/// 也不能把用户唯一的线索抹掉。
///
/// `dry_run` 时照常去找内容（git 历史、备份），只是不落盘 —— 「能不能修好」
/// 恰恰是预演最该回答的问题，光报「有个孤儿」没用。
fn heal_orphans(
    layout: &Layout,
    orphans: &[Orphan],
    records: &mut LinkRecords,
    dry_run: bool,
) -> Result<Vec<HealItem>> {
    let git = Git::new(layout.store());
    let mut out = Vec::new();

    for orphan in orphans {
        let target = layout.expand(&orphan.target);
        let store = layout.store_path(&orphan.store);

        let (source, note) = match orphan.kind {
            OrphanKind::Unmanaged => match fs::read(&store)
                .with_context(|| format!("读取 {} 失败", store.display()))
                .and_then(|bytes| {
                    if dry_run {
                        Ok(())
                    } else {
                        write_real_file(&target, &bytes)
                    }
                }) {
                Ok(()) => (HealSource::Store, None),
                Err(e) => (HealSource::Failed, Some(format!("{e:#}"))),
            },
            OrphanKind::Dangling => {
                if let Some(bytes) = git.content_before_deletion(&orphan.store) {
                    let done = if dry_run {
                        Ok(())
                    } else {
                        write_real_file(&target, &bytes)
                    };
                    match done {
                        Ok(()) => (HealSource::GitHistory, None),
                        Err(e) => (HealSource::Failed, Some(format!("{e:#}"))),
                    }
                } else if let Some(backup) = newest_backup(layout, &target) {
                    match fs::read(&backup)
                        .with_context(|| format!("读取备份 {} 失败", backup.display()))
                        .and_then(|bytes| {
                            if dry_run {
                                Ok(())
                            } else {
                                write_real_file(&target, &bytes)
                            }
                        }) {
                        Ok(()) => (
                            HealSource::Backup,
                            Some(format!("取自备份 {}", backup.display())),
                        ),
                        Err(e) => (HealSource::Failed, Some(format!("{e:#}"))),
                    }
                } else {
                    (
                        HealSource::Failed,
                        Some("git 历史和备份里都找不到内容，软链保持原样以免丢掉线索".to_owned()),
                    )
                }
            }
        };

        if source != HealSource::Failed {
            records.remove_target(&orphan.target);
        }
        out.push(HealItem {
            app: orphan.app.clone(),
            target: orphan.target.clone(),
            kind: orphan.kind,
            source,
            note,
        });
    }
    Ok(out)
}

/// 用实体文件替换 target（原来是软链就先摘掉）。
fn write_real_file(target: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(md) = fs::symlink_metadata(target)
        && md.file_type().is_symlink()
    {
        fs::remove_file(target).with_context(|| format!("移除软链 {} 失败", target.display()))?;
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(target, bytes).with_context(|| format!("写入 {} 失败", target.display()))
}

/// 备份目录名是 `%Y%m%d-%H%M%S`，字典序即时间序。
fn newest_backup(layout: &Layout, target: &Path) -> Option<PathBuf> {
    let rel = target.strip_prefix(layout.home()).ok()?;
    let mut stamps: Vec<_> = fs::read_dir(layout.backups())
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name())
        .collect();
    stamps.sort();
    stamps.into_iter().rev().find_map(|stamp| {
        let candidate = layout.backups().join(stamp).join(rel);
        candidate.is_file().then_some(candidate)
    })
}

// ---------------------------------------------------------------- sync

#[derive(Debug, serde::Serialize)]
pub struct SyncOutcome {
    pub commit: Option<String>,
    pub pull: PullOutcome,
    pub pushed: bool,
    pub remote: Option<String>,
    /// pull 之后重新落地的结果（别的机器新纳管的应用会在这里被建链）。
    pub applied: ApplyOutcome,
    /// 这次是预演，什么都没真的做。见 [`AddOutcome::dry_run`]。
    #[serde(default)]
    pub dry_run: bool,
    /// 仅预演：将被提交的文件（store 工作树里的未提交改动）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub would_commit: Option<Vec<String>>,
    /// 仅预演：本地缓存的 `@{upstream}` 显示落后远端多少个提交。
    ///
    /// **不 fetch**，所以这个数字可能是旧的 —— 它只说明「上次见到的远端」。
    /// 真要知道当下差多少，得跑真的 `sync`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<u32>,
}

/// 提交 → 拉取 → 推送 → 重新落地。
///
/// 最后那步 apply 是「同步」真正成立的关键：别的机器新增的应用需要在本机建链，
/// 别的机器移除的应用需要在本机把软链还原成实体文件。
///
/// `dry_run` **刻意不联网**：不 fetch、不 commit、不 push。它回答三个纯本地的
/// 问题 —— 将提交哪些文件、本地缓存里落后远端多少、apply 会动哪些链接。
/// 想知道远端当下的真实状态只能跑真的 sync（那才是它该做的事）。
pub fn sync(layout: &Layout, message: Option<&str>, dry_run: bool) -> Result<SyncOutcome> {
    let _lock = Lock::acquire(layout)?;
    let config = Config::load(layout)?;
    let git = Git::new(layout.store());
    if !git.is_repo() {
        bail!(
            "{} 不是 git 仓库，先跑 `cloudot init`",
            layout.store().display()
        );
    }

    let msg = match message {
        Some(m) => m.to_owned(),
        None => format!("cloudot: sync from {}", config.device),
    };

    if dry_run {
        let dirty = git.dirty_files()?;
        let behind = git.ahead_behind().map(|(_, behind)| behind);
        let applied = apply_inner(layout, false, true)?;
        return Ok(SyncOutcome {
            commit: None,
            // 没 fetch，所以谈不上「拉取结果」
            pull: PullOutcome::Skipped,
            pushed: false,
            remote: git.remote(),
            applied,
            dry_run: true,
            would_commit: Some(dirty),
            behind,
        });
    }

    let commit = git.commit_all(&msg)?;
    let pull = git.pull_rebase()?;
    let pushed = git.push()?;
    let applied = apply_inner(layout, false, false)?;

    Ok(SyncOutcome {
        commit,
        pull,
        pushed,
        remote: git.remote(),
        applied,
        dry_run: false,
        would_commit: None,
        behind: None,
    })
}

// ---------------------------------------------------------------- resolve

pub const RESOLVE_SCHEMA: &str = "cloudot.resolve/v1";

/// 拉取冲突后用户选边。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveSide {
    /// 用远端覆盖本地：`reset --hard origin/<branch>` + apply
    Theirs,
    /// 保留本地并推上去：`push --force-with-lease`
    Ours,
}

#[derive(Debug, serde::Serialize)]
pub struct ResolveOutcome {
    pub side: ResolveSide,
    /// theirs 时是 reset 到的 ref；ours 时是 push 目标 remote
    pub target: String,
    /// theirs 之后落地的结果；ours 时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<ApplyOutcome>,
    pub head: Option<String>,
}

/// 冲突选边。store 在冲突时已经被 abort 干净，这里是用户看过 diff 之后的动作。
///
/// - **theirs**：丢弃本地未推送的提交，对齐远端，再 apply 把软链落到新内容。
/// - **ours**：把本地历史强推上去（`--force-with-lease`，远端若有我们没见过的新提交会失败）。
pub fn resolve(layout: &Layout, side: ResolveSide) -> Result<ResolveOutcome> {
    let _lock = Lock::acquire(layout)?;
    let _config = Config::load(layout)?;
    let git = Git::new(layout.store());
    if !git.is_repo() {
        bail!(
            "{} 不是 git 仓库，先跑 `cloudot init`",
            layout.store().display()
        );
    }
    if git.remote().is_none() {
        bail!("还没配 remote，没有可对齐的远端");
    }

    let branch = git.branch().unwrap_or_else(|| "main".to_owned());
    let remote_ref = format!("origin/{branch}");

    match side {
        ResolveSide::Theirs => {
            // 确保有 origin/<branch> 可 reset
            if !git
                .try_run(&["rev-parse", "--verify", &remote_ref])
                .map(|(ok, _, _)| ok)
                .unwrap_or(false)
            {
                bail!(
                    "找不到 {remote_ref}，先 `git -C {} fetch`",
                    layout.store().display()
                );
            }
            git.reset_hard(&remote_ref)?;
            let applied = apply_inner(layout, false, false)?;
            Ok(ResolveOutcome {
                side,
                target: remote_ref,
                applied: Some(applied),
                head: git.head_short(),
            })
        }
        ResolveSide::Ours => {
            git.push_force_with_lease()?;
            Ok(ResolveOutcome {
                side,
                target: git.remote().unwrap_or_default(),
                applied: None,
                head: git.head_short(),
            })
        }
    }
}

// ---------------------------------------------------------------- unadopt

#[derive(Debug, serde::Serialize)]
pub struct UnadoptOutcome {
    pub id: String,
    pub name: String,
    pub restored: Vec<String>,
    pub commit: Option<String>,
    /// 这次是预演，什么都没真的做。见 [`AddOutcome::dry_run`]。
    #[serde(default)]
    pub dry_run: bool,
}

/// 退出纳管：软链换回实体文件，从 store 和 manifest 里移除。
///
/// 这是产品可信度的逃生门 —— 用户随时能把配置拿回来。
pub fn unadopt(layout: &Layout, app_id: &str, dry_run: bool) -> Result<UnadoptOutcome> {
    let _lock = Lock::acquire(layout)?;
    let config = Config::load(layout)?;
    let mut manifest = Manifest::load(layout)?;
    let mut records = LinkRecords::load(layout)?;
    let app = manifest.remove(app_id).ok_or_else(|| {
        crate::tagged(
            crate::ErrorKind::NotAdopted,
            format!("{app_id} 没有被纳管（`cloudot status` 看当前清单）"),
        )
    })?;

    let mut restored = Vec::new();
    for file in &app.files {
        let target = layout.expand(&file.target);
        let store = layout.store_path(&file.store);
        if !dry_run {
            link::unadopt_file(layout, &target, &store)?;
            records.remove_target(&file.target);
        }
        restored.push(file.target.clone());
    }

    if dry_run {
        return Ok(UnadoptOutcome {
            id: app.id,
            name: app.name,
            restored,
            commit: None,
            dry_run: true,
        });
    }

    manifest.save(layout)?;
    records.save(layout)?;
    let commit = Git::new(layout.store())
        .commit_all(&format!("cloudot: unadopt {} on {}", app.id, config.device))?;

    Ok(UnadoptOutcome {
        id: app.id,
        name: app.name,
        restored,
        commit,
        dry_run: false,
    })
}

// ---------------------------------------------------------------- apps

#[derive(Debug, serde::Serialize)]
pub struct AppListing {
    pub id: String,
    pub name: String,
    pub detected: bool,
    pub managed: bool,
}

pub fn apps(layout: &Layout) -> Result<Vec<AppListing>> {
    let manifest = Manifest::load(layout)?;
    Ok(adopter::load_all(layout)?
        .into_iter()
        .map(|ad| AppListing {
            detected: ad.detected(layout),
            managed: manifest.app(&ad.id).is_some(),
            id: ad.id,
            name: ad.name,
        })
        .collect())
}

// ---------------------------------------------------------------- show

pub const SHOW_SCHEMA: &str = "cloudot.show/v1";

/// 一个应用会动到的单个路径。
#[derive(Debug, serde::Serialize)]
pub struct ShowPath {
    /// 家目录相对形式（`~/.config/…`），和 manifest 里存的一致。
    pub target: String,
    /// 在 store 里的相对位置。算不出来时为 None（家目录之外、含 `..` 之类）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    pub strategy: Strategy,
    /// 当前链接状态。未纳管时多半是 `store_missing` 或 `replaced_by_file`。
    pub state: LinkState,
    /// 本机这个路径当前存在吗（软链也算）。
    pub exists: bool,
}

/// `cloudot show <app>` 的输出：定义 + 当前状态。
///
/// 刻意不直接序列化 [`adopter::Adopter`]：输出结构要能独立演进，
/// 而 adopter 是存储格式（用户会手写 TOML），两者耦合起来以后不好改。
#[derive(Debug, serde::Serialize)]
pub struct ShowOutcome {
    pub id: String,
    pub name: String,
    /// 本机检测到这个应用了吗（`detect` 里任一路径存在即为真）。
    pub detected: bool,
    pub managed: bool,
    /// 纳管它的设备名，未纳管时为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adopted_by: Option<String>,
    /// 用来判断「装没装」的探测路径。
    pub detect: Vec<String>,
    pub paths: Vec<ShowPath>,
}

/// 看一个应用的定义和当前状态 —— 纳管之前先知道会动哪些文件。
///
/// 纯只读，不加锁。未 init 也能用：adopter 定义来自编译进二进制的内置表加
/// `~/.cloudot/adopters/`，不需要 `config.toml`。
pub fn show(layout: &Layout, app_id: &str) -> Result<ShowOutcome> {
    let ad = adopter::get(layout, app_id)?;
    let manifest = Manifest::load(layout)?;
    let managed_app = manifest.app(&ad.id);

    let paths = ad
        .paths
        .iter()
        .map(|p| {
            let target = layout.expand(&p.path);
            let store_rel = layout.store_rel_for(&target).ok();
            let state = match &store_rel {
                Some(rel) => link::inspect(&target, &layout.store_path(rel)),
                // 算不出 store 位置的路径根本纳管不了，报 store 缺失最贴近事实
                None => LinkState::StoreMissing,
            };
            ShowPath {
                target: layout.contract(&target),
                store: store_rel,
                strategy: p.strategy,
                state,
                exists: fs::symlink_metadata(&target).is_ok(),
            }
        })
        .collect();

    Ok(ShowOutcome {
        detected: ad.detected(layout),
        managed: managed_app.is_some(),
        adopted_by: managed_app.map(|a| a.adopted_by.clone()),
        id: ad.id,
        name: ad.name,
        detect: ad.detect,
        paths,
    })
}
