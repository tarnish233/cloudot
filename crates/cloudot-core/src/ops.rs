use crate::config::{default_device_name, timestamp};
use crate::git::{Git, PullOutcome};
use crate::link::{self, AdoptAction, AdoptReport, LinkState};
use crate::links::{Orphan, OrphanKind};
use crate::manifest::{ManagedApp, ManagedFile};
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
}

/// 纳管一个应用：备份本地配置 → 移进 store → 建软链 → 记 manifest → 提交。
///
/// 要么全部路径都成功，要么整体回滚。半纳管状态（文件已经被链进 store 但 manifest
/// 和 links.toml 都没记）是最难收拾的：`status` 看不见它，`unadopt` 也撤不掉。
pub fn add(layout: &Layout, app_id: &str, force: bool, allow_secrets: bool) -> Result<AddOutcome> {
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
        let step = layout
            .store_rel_for(&target)
            .and_then(|store_rel| {
                let store_abs = layout.store_path(&store_rel);
                link::adopt_file(layout, &target, &store_abs, &stamp, force)
                    .map(|report| (store_rel, store_abs, report))
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
    manifest.save(layout)?;
    records.save(layout)?;

    let commit = Git::new(layout.store())
        .commit_all(&format!("cloudot: adopt {} on {}", ad.id, config.device))?;

    Ok(AddOutcome {
        id: ad.id,
        name: ad.name,
        files,
        commit,
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
}

/// 把 store 里的内容落地到本机 —— 新机器上的主命令。
///
/// 默认不覆盖本地实体文件：那份内容可能比 store 新。要覆盖得显式 `--force`，
/// 且一定先备份。
pub fn apply(layout: &Layout, force: bool) -> Result<ApplyOutcome> {
    let _lock = Lock::acquire(layout)?;
    apply_inner(layout, force)
}

/// 不加锁的实现。`sync` 已经持有锁，不能再走公开的 [`apply`]
/// —— flock 按打开的文件描述生效，同进程二次加锁一样会冲突。
fn apply_inner(layout: &Layout, force: bool) -> Result<ApplyOutcome> {
    let manifest = Manifest::load(layout)?;
    let mut records = LinkRecords::load(layout)?;
    let stamp = timestamp();

    // 先处理孤儿：manifest 里已经没有它们了，留着就是悬空软链，App 读不到配置。
    let orphans = links::find_orphans(layout, &manifest, &records);
    let healed = heal_orphans(layout, &orphans, &mut records)?;

    let mut items = Vec::new();
    for app in &manifest.apps {
        for file in &app.files {
            let target = layout.expand(&file.target);
            let store = layout.store_path(&file.store);
            let before = link::inspect(&target, &store);

            let item = match before {
                LinkState::Linked => ApplyItem {
                    target: file.target.clone(),
                    before,
                    action: ApplyAction::AlreadyLinked,
                    backup: None,
                    note: None,
                },
                LinkState::Missing => {
                    let report = link::adopt_file(layout, &target, &store, &stamp, force)?;
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
                        let report = link::adopt_file(layout, &target, &store, &stamp, true)?;
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
    records.save(layout)?;
    Ok(ApplyOutcome { items, healed })
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
fn heal_orphans(
    layout: &Layout,
    orphans: &[Orphan],
    records: &mut LinkRecords,
) -> Result<Vec<HealItem>> {
    let git = Git::new(layout.store());
    let mut out = Vec::new();

    for orphan in orphans {
        let target = layout.expand(&orphan.target);
        let store = layout.store_path(&orphan.store);

        let (source, note) = match orphan.kind {
            OrphanKind::Unmanaged => match fs::read(&store)
                .with_context(|| format!("读取 {} 失败", store.display()))
                .and_then(|bytes| write_real_file(&target, &bytes))
            {
                Ok(()) => (HealSource::Store, None),
                Err(e) => (HealSource::Failed, Some(format!("{e:#}"))),
            },
            OrphanKind::Dangling => {
                if let Some(bytes) = git.content_before_deletion(&orphan.store) {
                    match write_real_file(&target, &bytes) {
                        Ok(()) => (HealSource::GitHistory, None),
                        Err(e) => (HealSource::Failed, Some(format!("{e:#}"))),
                    }
                } else if let Some(backup) = newest_backup(layout, &target) {
                    match fs::read(&backup)
                        .with_context(|| format!("读取备份 {} 失败", backup.display()))
                        .and_then(|bytes| write_real_file(&target, &bytes))
                    {
                        Ok(()) => (
                            HealSource::Backup,
                            Some(format!("取自备份 {}", backup.display())),
                        ),
                        Err(e) => (HealSource::Failed, Some(format!("{e:#}"))),
                    }
                } else {
                    (
                        HealSource::Failed,
                        Some(
                            "git 历史和备份里都找不到内容，软链保持原样以免丢掉线索".to_owned(),
                        ),
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
        fs::remove_file(target)
            .with_context(|| format!("移除软链 {} 失败", target.display()))?;
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
}

/// 提交 → 拉取 → 推送 → 重新落地。
///
/// 最后那步 apply 是「同步」真正成立的关键：别的机器新增的应用需要在本机建链，
/// 别的机器移除的应用需要在本机把软链还原成实体文件。
pub fn sync(layout: &Layout, message: Option<&str>) -> Result<SyncOutcome> {
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
    let commit = git.commit_all(&msg)?;
    let pull = git.pull_rebase()?;
    let pushed = git.push()?;
    let applied = apply_inner(layout, false)?;

    Ok(SyncOutcome {
        commit,
        pull,
        pushed,
        remote: git.remote(),
        applied,
    })
}

// ---------------------------------------------------------------- unadopt

#[derive(Debug, serde::Serialize)]
pub struct UnadoptOutcome {
    pub id: String,
    pub name: String,
    pub restored: Vec<String>,
    pub commit: Option<String>,
}

/// 退出纳管：软链换回实体文件，从 store 和 manifest 里移除。
///
/// 这是产品可信度的逃生门 —— 用户随时能把配置拿回来。
pub fn unadopt(layout: &Layout, app_id: &str) -> Result<UnadoptOutcome> {
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
        link::unadopt_file(layout, &target, &store)?;
        records.remove_target(&file.target);
        restored.push(file.target.clone());
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
