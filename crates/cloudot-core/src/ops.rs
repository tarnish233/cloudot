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

/// 提交阶段之前的状态快照，用于在最后一步失败时把两份清单还原回去。
///
/// 存的是**原始字节**而不是解析后的结构：还原要的是「回到一模一样的样子」，
/// 重新序列化可能因为字段顺序或默认值的变化产生 diff。`None` 表示那时文件还不存在
/// （首次 `add`），还原就是把它删掉。
struct StateSnapshot {
    manifest: Option<Vec<u8>>,
    records: Option<Vec<u8>>,
}

impl StateSnapshot {
    fn take(layout: &Layout) -> Self {
        Self {
            manifest: fs::read(layout.manifest_file()).ok(),
            records: fs::read(layout.links_file()).ok(),
        }
    }

    /// 尽最大努力还原；把没能还原的写成人能看懂的说明返回。
    fn restore(&self, layout: &Layout) -> Vec<String> {
        let mut errors = Vec::new();
        for (path, saved) in [
            (layout.manifest_file(), &self.manifest),
            (layout.links_file(), &self.records),
        ] {
            let done = match saved {
                Some(bytes) => fs::write(&path, bytes),
                // 原本没有这个文件（首次 add），删掉就是还原
                None => match fs::remove_file(&path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    other => other,
                },
            };
            if let Err(e) = done {
                errors.push(format!("  {} —— {e}", path.display()));
            }
        }
        errors
    }
}

/// 把这次事务碰过的 store 路径从索引里撤下来。
///
/// **刻意不用 `git reset`（无路径）**：store 工作树是用户的实时配置，里面很可能有与
/// 本次操作无关的未提交改动（改完配置还没 sync 就很常见）。整体 reset 会把那些一起
/// 撤掉，那是在修一个 bug 的时候制造另一个。
///
/// `manifest.toml` 一定要在列表里：`commit_all` 走的是 `git add -A`，所以清单的新版本
/// 已经进了索引，光把工作树的文件还原回去不够 —— 索引里那份还留着这次的改动，
/// `git status` 会显示 `MM`，`git diff --cached` 能看到本该消失的条目。
/// （`links.toml` 不在 store 里、不进 git，还原文件本身就够了。）
///
/// 已知局限：`git add -A` 顺带把用户原本未暂存的改动也暂存了，这里不会替他们撤回去
/// —— 那些改动没丢，只是从「未暂存」变成「已暂存」，而 cloudot 自己每次提交前都
/// `add -A`，对它的工作流没有影响。
fn unstage(git: &Git, store_paths: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    for rel in store_paths
        .iter()
        .map(String::as_str)
        .chain(["manifest.toml"])
    {
        // `--` 之后是路径，避免和分支名歧义；已经不在索引里的路径 reset 也不报错
        if let Ok((ok, _, err)) = git.try_run(&["reset", "--quiet", "HEAD", "--", rel])
            && !ok
        {
            errors.push(format!("  {rel} —— {}", err.trim()));
        }
    }
    errors
}

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

/// 既有 manifest 条目 ∪ 本次展开结果，既有条目在前、顺序稳定。
///
/// `add` 因此是**增量**的：重跑只会补新文件，不会因为本机暂时看不到某个文件就把它
/// 从清单里摘掉（那等于替用户做了 `unadopt` 的决定，而 store 里的副本还留着，
/// 变成没人管的孤儿 —— 实测这种状态 `doctor` 都发现不了，因为它对账靠的是
/// 「links.toml 有、manifest 没有」，而 `add` 把两边一起清了）。
///
/// 既有条目的 strategy 沿用清单里记的那个：adopter 定义可能改过，但已经落地的
/// 文件该按当初纳管的方式继续对待，换策略是另一件事。
fn merge_with_managed(
    layout: &Layout,
    managed: Option<&ManagedApp>,
    expanded: Vec<(String, Strategy)>,
) -> Vec<(String, Strategy)> {
    let Some(app) = managed else {
        return expanded;
    };
    let mut out: Vec<(String, Strategy)> = Vec::new();
    for file in &app.files {
        // 既有条目也要过门禁：#3 的校验拦的是「拿来动文件」，这里是把它带进新清单，
        // 同样不能放行坏路径，否则「保留既有条目」就成了保留恶意条目。
        if layout
            .store_rel_for(&layout.expand(&file.target))
            .is_ok_and(|rel| rel == file.store)
        {
            out.push((file.target.clone(), file.strategy));
        }
    }
    for (path, strategy) in expanded {
        if !out.iter().any(|(existing, _)| *existing == path) {
            out.push((path, strategy));
        }
    }
    out
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
    // 清单是跨机器共享的外部输入，动文件之前先过一遍门禁。放在展开与凭据扫描
    // 之前：清单本身不可信时，后面那些结论都不值得算。
    manifest.ensure_safe(layout)?;
    let mut records = LinkRecords::load(layout)?;
    let stamp = timestamp();
    // 提交阶段失败时要把两份清单还原回去，所以快照必须在动任何盘之前取。
    let snapshot = StateSnapshot::take(layout);

    // glob 在这里一次性展开成具体文件，后面的探测、凭据扫描、纳管循环都消费
    // 同一份清单 —— 否则三处各扫一次目录，中间有文件增删就会各说各话。
    // manifest 里存的仍是展开后的确定清单（见 `Adopter::expand_paths`）。
    //
    // **和既有条目合并，不是替换。** `expand_paths` 只看得见本机此刻存在的文件，
    // 而 manifest 是跨机器共享的：别的机器纳管过、或者本机这个文件暂时不在
    // （软链被误删、目录还没同步下来）时，重跑 `add` 若直接用展开结果覆盖，
    // 那些条目就静默消失了 —— store 里的文件还在，但没人再管它，`status` 也
    // 照样报 healthy。删除必须走显式的 `unadopt`。
    let targets = merge_with_managed(layout, manifest.app(&ad.id), ad.expand_paths(layout));

    let store_has_any = targets.iter().any(|(path, _)| {
        layout
            .store_rel_for(&layout.expand(path))
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

    // glob 展开后一个文件都不剩：目录在但里面没有匹配的内容，纳管了也什么都不做。
    // 说清楚比返回一个空的成功结果好 —— 后者看起来像成功了。
    if targets.is_empty() {
        return Err(crate::tagged(
            crate::ErrorKind::NotDetected,
            format!(
                "{} 的定义没有匹配到任何文件（检查 adopter 里的 include / exclude）",
                ad.name
            ),
        ));
    }

    // 凭据检查放在动手之前，而且是全部路径一起看：宁可整体拒绝，
    // 也不要纳管了一半才发现有 token 要往 git 里推。
    if !allow_secrets {
        let mut findings = Vec::new();
        for (path, _) in &targets {
            let target = layout.expand(path);
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

    for (path, strategy) in &targets {
        let target = layout.expand(path);
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
                    strategy: *strategy,
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

    let git = Git::new(layout.store());
    let commit = match manifest
        .save(layout)
        .and_then(|()| records.save(layout))
        .and_then(|()| git.commit_all(&format!("cloudot: adopt {} on {}", ad.id, config.device)))
    {
        Ok(commit) => commit,
        // 走到这里文件已经全部移进 store、软链也建好了，但清单没写成 / 提交没成功
        // （pre-commit hook 拒绝、磁盘满、store 权限变了都会到这儿）。
        // 不回滚的话留下的是最难查的一种状态：本地看着已纳管，实际清单里没有它。
        Err(err) => {
            let mut unwind_errors = Vec::new();
            for (target, store, report) in done.iter().rev() {
                if let Err(e) = link::revert_adopt(target, store, report) {
                    unwind_errors.push(format!("  {} —— {e:#}", layout.contract(target)));
                }
            }
            unwind_errors.extend(snapshot.restore(layout));
            unwind_errors.extend(unstage(
                &git,
                &files.iter().map(|f| f.store.clone()).collect::<Vec<_>>(),
            ));
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
    };

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
    // 这是最要紧的一处：`apply` 会照着清单建软链，目标不存在时连中间目录都会建出来
    // 且不需要 `--force`。清单从远端 pull 回来，所以必须在动手之前校验。
    manifest.ensure_safe(layout)?;
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
/// 这是产品可信度的逃生门 —— 用户随时能把配置拿回来。所以它和 [`add`] 一样
/// **要么整体成功、要么整体回滚**：多路径应用（fish 有 3 个）中途失败时，若不回滚
/// 就会留下「一部分已解链、store 副本也删了，但 manifest 里这个应用还完整」的
/// 半退管状态 —— `status` 以为还在纳管，实际那几个文件已经取不回来了。
pub fn unadopt(layout: &Layout, app_id: &str, dry_run: bool) -> Result<UnadoptOutcome> {
    let _lock = Lock::acquire(layout)?;
    let config = Config::load(layout)?;
    let mut manifest = Manifest::load(layout)?;
    // unadopt 会删 store 副本、重建软链，同样是照着清单动文件。
    // 校验放在 `remove` 之前：坏清单要整体拒绝，不能先摘掉一个应用再报错。
    manifest.ensure_safe(layout)?;
    let mut records = LinkRecords::load(layout)?;
    // 和 add 一样：提交阶段失败要还原两份清单，快照得在动盘之前取。
    let snapshot = StateSnapshot::take(layout);
    let app = manifest.remove(app_id).ok_or_else(|| {
        crate::tagged(
            crate::ErrorKind::NotAdopted,
            format!("{app_id} 没有被纳管（`cloudot status` 看当前清单）"),
        )
    })?;

    let mut restored = Vec::new();
    let mut done: Vec<(PathBuf, PathBuf, link::UnadoptReport)> = Vec::new();
    let mut failure: Option<anyhow::Error> = None;

    for file in &app.files {
        let target = layout.expand(&file.target);
        let store = layout.store_path(&file.store);
        if dry_run {
            // 预演也要把「这个文件会失败」报出来，这正是它的用处
            if let Err(e) = link::plan_unadopt(layout, &target, &store) {
                failure = Some(e);
                break;
            }
            restored.push(file.target.clone());
            continue;
        }
        match link::unadopt_file(layout, &target, &store) {
            Ok(report) => {
                records.remove_target(&file.target);
                restored.push(file.target.clone());
                done.push((target, store, report));
            }
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }

    if let Some(err) = failure {
        if dry_run {
            return Err(err.context(format!("预演：退出纳管 {} 会失败", app.name)));
        }
        // 逆序撤销已经做过的那些，把软链和 store 副本都还原回去
        let mut unwind_errors = Vec::new();
        for (target, store, report) in done.iter().rev() {
            if let Err(e) = link::revert_unadopt(target, store, report) {
                unwind_errors.push(format!("  {} —— {e:#}", layout.contract(target)));
            }
        }
        return Err(if unwind_errors.is_empty() {
            err.context(format!("退出纳管 {} 失败，已回滚本次全部改动", app.name))
        } else {
            err.context(format!(
                "退出纳管 {} 失败，而且回滚不完整，需要手工检查：\n{}\n\
                 store 里被删掉的内容在 ~/.cloudot/backups 里还有一份。",
                app.name,
                unwind_errors.join("\n")
            ))
        });
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

    let git = Git::new(layout.store());
    let commit = match manifest
        .save(layout)
        .and_then(|()| records.save(layout))
        .and_then(|()| git.commit_all(&format!("cloudot: unadopt {} on {}", app.id, config.device)))
    {
        Ok(commit) => commit,
        // unadopt 是逃生门，半开状态尤其不能留：软链已经换回实体文件、store 副本也
        // 删了，但清单里这个应用还完整 —— `status` 以为还在纳管，实际早已脱管。
        Err(err) => {
            let mut unwind_errors = Vec::new();
            for (target, store, report) in done.iter().rev() {
                if let Err(e) = link::revert_unadopt(target, store, report) {
                    unwind_errors.push(format!("  {} —— {e:#}", layout.contract(target)));
                }
            }
            unwind_errors.extend(snapshot.restore(layout));
            unwind_errors.extend(unstage(
                &git,
                &app.files
                    .iter()
                    .map(|f| f.store.clone())
                    .collect::<Vec<_>>(),
            ));
            return Err(if unwind_errors.is_empty() {
                err.context(format!("退出纳管 {} 失败，已回滚本次全部改动", app.name))
            } else {
                err.context(format!(
                    "退出纳管 {} 失败，而且回滚不完整，需要手工检查：\n{}\n\
                     store 里被删掉的内容在 ~/.cloudot/backups 里还有一份。",
                    app.name,
                    unwind_errors.join("\n")
                ))
            });
        }
    };

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

    // 和 add 走同一条展开路径 —— show 要报的就是「add 会动哪些文件」，
    // 两边若各自算一遍，glob 的语义一旦有出入，预览就会骗人。
    let paths = ad
        .expand_paths(layout)
        .into_iter()
        .map(|(path, strategy)| {
            let target = layout.expand(&path);
            let store_rel = layout.store_rel_for(&target).ok();
            let state = match &store_rel {
                Some(rel) => link::inspect(&target, &layout.store_path(rel)),
                // 算不出 store 位置的路径根本纳管不了，报 store 缺失最贴近事实
                None => LinkState::StoreMissing,
            };
            ShowPath {
                target: layout.contract(&target),
                store: store_rel,
                strategy,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempHome;

    /// 造一个已 init 的假家目录，并放一个自定义 adopter。
    ///
    /// `paths` 是 adopter 要纳管的家目录相对路径；顺序即 `add`/`unadopt` 的处理顺序，
    /// 事务性测试靠它安排「第一个成功、第二个失败」。
    fn init_home(tag: &str, paths: &[&str]) -> (TempHome, Layout) {
        let home = TempHome::new(tag);
        let layout = home.layout();
        init(&layout, None, Some("test-device")).expect("init");

        let entries = paths
            .iter()
            .map(|p| format!("[[paths]]\npath = \"~/{p}\"\n"))
            .collect::<String>();
        let first = paths.first().expect("至少一个路径");
        fs::write(
            layout.adopters_dir().join("t.toml"),
            format!("id = \"t\"\nname = \"T\"\ndetect = [\"~/{first}\"]\n{entries}",),
        )
        .expect("写 adopter");
        (home, layout)
    }

    /// 在假家目录里写一个文件（自动建父目录）。
    fn write_home_file(layout: &Layout, rel: &str, body: &str) {
        let path = layout.expand(&format!("~/{rel}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
    }

    fn is_symlink(path: &Path) -> bool {
        fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    // ── add 的事务性 ─────────────────────────────────────────────

    #[test]
    fn add_moves_file_into_store_and_links_it() {
        let (_h, layout) = init_home("ops-add", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");

        let out = add(&layout, "t", false, false, false).expect("add");
        assert_eq!(out.files.len(), 1);
        assert_eq!(out.files[0].action, AdoptAction::MovedIntoStore);
        assert!(!out.dry_run);

        let target = layout.expand("~/.config/t/conf");
        assert!(is_symlink(&target), "target 该变成软链");
        assert_eq!(fs::read_to_string(&target).unwrap(), "body\n");
        assert!(Manifest::load(&layout).unwrap().app("t").is_some());
    }

    /// 第二个路径是目录（当前版本不支持），整体必须回滚。
    ///
    /// 曾经的行为会留下「第一个已建链但 manifest 没记」的半纳管状态：
    /// `status` 看不见它，`unadopt` 也撤不掉。
    #[test]
    fn add_rolls_back_everything_when_a_later_path_fails() {
        let (_h, layout) = init_home("ops-add-rollback", &[".config/t/one", ".config/t/two"]);
        write_home_file(&layout, ".config/t/one", "first\n");
        // 目录 → adopt_file 报 Unsupported
        fs::create_dir_all(layout.expand("~/.config/t/two")).unwrap();

        let err = add(&layout, "t", false, false, false).expect_err("该整体失败");
        assert!(format!("{err:#}").contains("已回滚"), "错误里该说明回滚了");

        let one = layout.expand("~/.config/t/one");
        assert!(!is_symlink(&one), "第一个路径该回滚成实体文件");
        assert_eq!(fs::read_to_string(&one).unwrap(), "first\n", "内容要完整");
        assert!(
            !layout.store_path("files/.config/t/one").exists(),
            "store 里不该有残留"
        );
        assert!(
            Manifest::load(&layout).unwrap().app("t").is_none(),
            "manifest 不该被写脏"
        );
        assert!(
            LinkRecords::load(&layout).unwrap().links.is_empty(),
            "links.toml 不该被写脏"
        );
    }

    /// 凭据门禁在动手之前就该拦下，而且是整体拒绝。
    #[test]
    fn add_refuses_before_touching_anything_when_secrets_are_found() {
        let (_h, layout) = init_home("ops-add-secrets", &[".config/t/conf", ".ssh/id_rsa"]);
        write_home_file(&layout, ".config/t/conf", "harmless\n");
        write_home_file(&layout, ".ssh/id_rsa", "x\n");

        let err = add(&layout, "t", false, false, false).expect_err("该被拦下");
        assert_eq!(
            crate::errors::kind_of(&err),
            crate::ErrorKind::SecretsDetected
        );
        // 即使第一个路径无害，也不该已经被移走
        assert!(!is_symlink(&layout.expand("~/.config/t/conf")));
        assert!(!layout.store_path("files/.config/t/conf").exists());
    }

    // ── 提交阶段失败也要整体回滚（回归）────────────────────────
    //
    // 曾经的行为：文件循环全部成功之后，`manifest.save` / `records.save` /
    // `commit_all` 里任何一个 `?` 都会直接把错误抛出去，绕过前面的补偿逻辑。
    // 实测症状（pre-commit hook 返回 1）：退出码 1，但配置已移进 store、本地已成
    // 软链、manifest 已记为纳管、git 还留着 staged 改动 —— 和注释里写的
    // 「要么整体成功要么整体回滚」正好相反。

    /// 装一个必然失败的 pre-commit hook，用来逼出提交阶段的错误。
    ///
    /// 比让 `manifest.save` 失败更贴近真实：hook 失败、磁盘满、store 权限变了
    /// 都会走到同一段代码，而 hook 是唯一能在测试里稳定复现的。
    fn install_failing_hook(layout: &Layout) {
        let hooks = layout.store().join(".git/hooks");
        fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        fs::write(&hook, "#!/bin/sh\necho 'hook 拒绝' >&2\nexit 1\n").unwrap();
        let mut perm = fs::metadata(&hook).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
        fs::set_permissions(&hook, perm).unwrap();
    }

    #[test]
    fn add_rolls_back_when_the_commit_fails() {
        let (_h, layout) = init_home("ops-add-commit-fail", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        install_failing_hook(&layout);

        let err = add(&layout, "t", false, false, false).expect_err("提交失败该整体失败");
        assert!(format!("{err:#}").contains("已回滚"), "错误里该说明回滚了");

        let target = layout.expand("~/.config/t/conf");
        assert!(!is_symlink(&target), "该回滚成实体文件");
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "body\n",
            "内容必须原样回来"
        );
        assert!(
            !layout.store_path("files/.config/t/conf").exists(),
            "store 里不该有残留"
        );
        assert!(
            Manifest::load(&layout).unwrap().app("t").is_none(),
            "manifest 不该留下这次的条目"
        );
        assert!(
            LinkRecords::load(&layout).unwrap().links.is_empty(),
            "links.toml 不该留下这次的记录"
        );
        // 索引里也不能留 —— `commit_all` 走 `git add -A`，只还原工作树不够：
        // 索引里那份仍带着本次的条目，`git status` 会显示 MM
        let staged = Git::new(layout.store())
            .try_run(&["show", ":manifest.toml"])
            .map(|(_, out, _)| out)
            .unwrap_or_default();
        assert!(
            !staged.contains("~/.config/t/conf"),
            "索引里的 manifest 还带着本次改动：{staged}"
        );
    }

    /// 回滚不能牵连本次操作之外的改动。
    ///
    /// store 工作树就是用户的实时配置，「改完配置还没 sync」是最常见的状态。
    /// 用整体 `git reset` 清索引会把那些一起撤掉 —— 修一个 bug 制造另一个。
    #[test]
    fn commit_rollback_leaves_unrelated_staged_changes_alone() {
        let (_h, layout) = init_home("ops-add-commit-unrelated", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");

        let git = Git::new(layout.store());
        fs::write(layout.store().join("unrelated.txt"), "mine\n").unwrap();
        git.try_run(&["add", "unrelated.txt"]).unwrap();

        install_failing_hook(&layout);
        add(&layout, "t", false, false, false).expect_err("该失败");

        let (ok, staged, _) = git.try_run(&["show", ":unrelated.txt"]).unwrap();
        assert!(ok && staged.contains("mine"), "无关的暂存改动被撤掉了");
    }

    #[test]
    fn unadopt_rolls_back_when_the_commit_fails() {
        let (_h, layout) = init_home("ops-unadopt-commit-fail", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        add(&layout, "t", false, false, false).expect("add");
        install_failing_hook(&layout);

        let err = unadopt(&layout, "t", false).expect_err("提交失败该整体失败");
        assert!(format!("{err:#}").contains("已回滚"));

        // 逃生门回滚后应当**回到纳管中**的样子，而不是半退管
        assert!(
            is_symlink(&layout.expand("~/.config/t/conf")),
            "软链该重建回来"
        );
        assert!(
            layout.store_path("files/.config/t/conf").exists(),
            "store 副本该还原回来"
        );
        assert!(
            Manifest::load(&layout).unwrap().app("t").is_some(),
            "manifest 该保持原样（还在纳管）"
        );
        assert_eq!(
            LinkRecords::load(&layout).unwrap().links.len(),
            1,
            "links.toml 该保持原样"
        );
    }

    // ── add 是增量的（回归）──────────────────────────────────
    //
    // 曾经的行为：`expand_paths` 只看得见本机此刻存在的文件，而它的结果被整体
    // 塞进 manifest。实测症状：移走一个已纳管文件后重跑 `add`，那个条目从清单里
    // 静默消失、store 文件变成没人管的孤儿，而 `status` 照样报 healthy ——
    // 别的机器同步后也会跟着停止纳管它。

    /// 本机暂时看不到的已纳管文件，重跑 add 不能把它从清单里摘掉。
    ///
    /// **必须用 glob 定义来测**：`expand_paths` 对单文件条目是原样透传的（存不存在都
    /// 返回），只有目录 + include 那条路径会「本机没有就不出现在结果里」。
    /// 拿单文件条目写这个测试的话，把修复回退掉它照样通过 —— 试过，所以留这行注释。
    #[test]
    fn add_keeps_managed_entries_whose_local_file_is_gone() {
        let home = TempHome::new("ops-add-incremental");
        let layout = home.layout();
        init(&layout, None, Some("test-device")).expect("init");
        let dir = layout.expand("~/.config/t/conf.d");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("one.conf"), "one\n").unwrap();
        fs::write(dir.join("two.conf"), "two\n").unwrap();
        fs::write(
            layout.adopters_dir().join("t.toml"),
            "id = \"t\"\nname = \"T\"\ndetect = [\"~/.config/t/conf.d\"]\n\
             [[paths]]\npath = \"~/.config/t/conf.d\"\ninclude = [\"*.conf\"]\n",
        )
        .unwrap();
        add(&layout, "t", false, false, false).expect("首次 add");

        // 软链被误删 / 目录还没同步下来，都是这个形态
        fs::remove_file(dir.join("two.conf")).unwrap();

        add(&layout, "t", false, false, false).expect("重跑 add");

        let manifest = Manifest::load(&layout).unwrap();
        let targets: Vec<&str> = manifest
            .app("t")
            .expect("t 还该在")
            .files
            .iter()
            .map(|f| f.target.as_str())
            .collect();
        assert!(
            targets.contains(&"~/.config/t/conf.d/two.conf"),
            "已纳管条目被静默删掉了：{targets:?}"
        );
        // store 里的内容还在，所以该被修回来（从 store 建链）而不是留成孤儿
        assert!(is_symlink(&dir.join("two.conf")));
    }

    /// 增量不代表不收新文件 —— glob 目录里新增的仍要纳管进来。
    #[test]
    fn add_still_picks_up_newly_added_files() {
        let home = TempHome::new("ops-add-glob-new");
        let layout = home.layout();
        init(&layout, None, Some("test-device")).expect("init");
        let dir = layout.expand("~/.config/t/conf.d");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.conf"), "a\n").unwrap();
        fs::write(
            layout.adopters_dir().join("t.toml"),
            "id = \"t\"\nname = \"T\"\ndetect = [\"~/.config/t/conf.d\"]\n\
             [[paths]]\npath = \"~/.config/t/conf.d\"\ninclude = [\"*.conf\"]\n",
        )
        .unwrap();
        add(&layout, "t", false, false, false).expect("首次 add");

        fs::write(dir.join("b.conf"), "b\n").unwrap();
        add(&layout, "t", false, false, false).expect("重跑 add");

        let manifest = Manifest::load(&layout).unwrap();
        let targets: Vec<&str> = manifest
            .app("t")
            .unwrap()
            .files
            .iter()
            .map(|f| f.target.as_str())
            .collect();
        assert!(targets.contains(&"~/.config/t/conf.d/a.conf"));
        assert!(
            targets.contains(&"~/.config/t/conf.d/b.conf"),
            "新增文件没被收进来：{targets:?}"
        );
    }

    /// 合并既有条目时也要过路径门禁，否则「保留既有条目」就成了保留恶意条目。
    #[test]
    fn add_drops_unsafe_entries_instead_of_carrying_them_forward() {
        let (_h, layout) = init_home("ops-add-merge-unsafe", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        assert_eq!(
            merge_with_managed(
                &layout,
                Some(&ManagedApp {
                    id: "t".into(),
                    name: "T".into(),
                    adopted_by: "attacker".into(),
                    files: vec![ManagedFile {
                        target: "/etc/passwd".into(),
                        store: "files/evil".into(),
                        strategy: Strategy::Symlink,
                    }],
                }),
                vec![("~/.config/t/conf".to_owned(), Strategy::Symlink)],
            ),
            vec![("~/.config/t/conf".to_owned(), Strategy::Symlink)],
            "家目录之外的既有条目不该被带进新清单"
        );
    }

    // ── unadopt 的事务性（回归：逃生门不能半开）─────────────────
    //
    // 曾经的行为：多路径应用中途失败时，前面的文件已经解链、store 副本也删了，
    // 但 manifest 还没保存 —— `status` 以为还在纳管，实际那几个文件已经取不回来。

    #[test]
    fn unadopt_restores_all_files_and_clears_manifest() {
        let (_h, layout) = init_home("ops-unadopt", &[".config/t/one", ".config/t/two"]);
        write_home_file(&layout, ".config/t/one", "one\n");
        write_home_file(&layout, ".config/t/two", "two\n");
        add(&layout, "t", false, false, false).expect("add");

        let out = unadopt(&layout, "t", false).expect("unadopt");
        assert_eq!(out.restored.len(), 2);

        for (rel, body) in [(".config/t/one", "one\n"), (".config/t/two", "two\n")] {
            let target = layout.expand(&format!("~/{rel}"));
            assert!(!is_symlink(&target), "{rel} 该还原成实体文件");
            assert_eq!(fs::read_to_string(&target).unwrap(), body);
        }
        assert!(Manifest::load(&layout).unwrap().app("t").is_none());
    }

    #[test]
    fn unadopt_rolls_back_when_a_later_file_fails() {
        let (_h, layout) = init_home("ops-unadopt-rollback", &[".config/t/one", ".config/t/two"]);
        write_home_file(&layout, ".config/t/one", "one\n");
        write_home_file(&layout, ".config/t/two", "two\n");
        add(&layout, "t", false, false, false).expect("add");

        // 让第二个文件的 store 内容消失 → 软链悬空，unadopt_file 会拒绝处理它
        fs::remove_file(layout.store_path("files/.config/t/two")).unwrap();

        let err = unadopt(&layout, "t", false).expect_err("该整体失败");
        assert!(
            format!("{err:#}").contains("已回滚"),
            "错误里该说明回滚了：{err:#}"
        );

        // 第一个文件必须回到纳管状态：软链在、store 副本也在
        let one = layout.expand("~/.config/t/one");
        assert!(is_symlink(&one), "第一个文件该重新变回软链");
        assert_eq!(
            fs::read_to_string(&one).unwrap(),
            "one\n",
            "透过软链读到的内容不能变"
        );
        assert!(
            layout.store_path("files/.config/t/one").exists(),
            "store 副本该被还原回来"
        );
        assert!(
            Manifest::load(&layout).unwrap().app("t").is_some(),
            "整体失败时 manifest 该保持原样，不能变成半退管"
        );
    }

    #[test]
    fn unadopt_reports_unknown_app_as_not_adopted() {
        let (_h, layout) = init_home("ops-unadopt-unknown", &[".config/t/conf"]);
        let err = unadopt(&layout, "nope", false).expect_err("该失败");
        assert_eq!(crate::errors::kind_of(&err), crate::ErrorKind::NotAdopted);
    }

    // ── apply ───────────────────────────────────────────────────

    #[test]
    fn apply_relinks_a_missing_target() {
        let (_h, layout) = init_home("ops-apply", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        add(&layout, "t", false, false, false).expect("add");

        let target = layout.expand("~/.config/t/conf");
        fs::remove_file(&target).unwrap();

        let out = apply(&layout, false, false).expect("apply");
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.items[0].before, LinkState::Missing);
        assert_eq!(out.items[0].action, ApplyAction::Linked);
        assert!(is_symlink(&target));
    }

    /// 本地是实体文件时默认拒绝覆盖 —— 那份内容可能比 store 新。
    ///
    /// 注意两次 `apply` 之间要让锁先释放（`Lock` 的守卫在 drop 时才放锁，
    /// 而 flock 按「打开的文件描述」生效，同进程二次 acquire 也会冲突）。
    #[test]
    fn apply_skips_a_real_local_file_without_force() {
        let (_h, layout) = init_home("ops-apply-skip", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "in-store\n");
        add(&layout, "t", false, false, false).expect("add");

        // 模拟 App 用「替换写入」顶掉软链
        let target = layout.expand("~/.config/t/conf");
        fs::remove_file(&target).unwrap();
        fs::write(&target, "newer-local\n").unwrap();

        {
            let out = apply(&layout, false, false).expect("apply");
            assert_eq!(out.items[0].before, LinkState::ReplacedByFile);
            assert_eq!(out.items[0].action, ApplyAction::Skipped);
            assert!(out.items[0].note.is_some(), "跳过要说明原因");
            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                "newer-local\n",
                "绝不能静默覆盖本地内容"
            );
        }

        // --force 才覆盖，且一定先备份
        let forced = apply(&layout, true, false).expect("apply --force");
        assert_eq!(forced.items[0].action, ApplyAction::Replaced);
        assert!(is_symlink(&target));
        let backup = forced.items[0].backup.as_ref().expect("该留备份");
        assert_eq!(fs::read_to_string(backup).unwrap(), "newer-local\n");
    }

    /// 孤儿软链（manifest 里已没有、却还指向 store）要自愈成实体文件。
    #[test]
    fn apply_heals_an_orphan_from_git_history() {
        let (_h, layout) = init_home("ops-apply-heal", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        add(&layout, "t", false, false, false).expect("add");

        // 模拟别的机器 unadopt 后同步过来：manifest 清空、store 文件删掉，软链留着
        let mut manifest = Manifest::load(&layout).unwrap();
        manifest.remove("t");
        manifest.save(&layout).unwrap();
        fs::remove_file(layout.store_path("files/.config/t/conf")).unwrap();
        Git::new(layout.store()).commit_all("wipe").unwrap();

        let out = apply(&layout, false, false).expect("apply");
        assert_eq!(out.healed.len(), 1, "该报出一条自愈");
        assert_eq!(out.healed[0].source, HealSource::GitHistory);

        let target = layout.expand("~/.config/t/conf");
        assert!(!is_symlink(&target), "该还原成实体文件，不是悬空软链");
        assert_eq!(fs::read_to_string(&target).unwrap(), "body\n");
    }

    // ── dry-run：与真跑结论一致，且一个字节都不写 ────────────────

    #[test]
    fn dry_run_add_predicts_the_same_action_without_writing() {
        let (_h, layout) = init_home("ops-dry-add", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        let manifest_before = fs::read_to_string(layout.manifest_file()).unwrap();

        let planned_actions = {
            let planned = add(&layout, "t", false, false, true).expect("dry-run add");
            assert!(planned.dry_run);
            assert!(planned.commit.is_none(), "预演不该提交");

            let target = layout.expand("~/.config/t/conf");
            assert!(!is_symlink(&target), "预演建了软链");
            assert!(!layout.store_path("files/.config/t/conf").exists());
            assert_eq!(
                fs::read_to_string(layout.manifest_file()).unwrap(),
                manifest_before,
                "预演动了 manifest"
            );
            planned.files.iter().map(|f| f.action).collect::<Vec<_>>()
        };

        // 真跑一遍，结论必须一致
        let real = add(&layout, "t", false, false, false).expect("add");
        assert_eq!(
            planned_actions,
            real.files.iter().map(|f| f.action).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn dry_run_unadopt_keeps_the_link_and_the_manifest() {
        let (_h, layout) = init_home("ops-dry-unadopt", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        add(&layout, "t", false, false, false).expect("add");

        let out = unadopt(&layout, "t", true).expect("dry-run unadopt");
        assert!(out.dry_run);
        assert_eq!(out.restored.len(), 1, "该报出会还原哪些文件");

        assert!(is_symlink(&layout.expand("~/.config/t/conf")), "预演解链了");
        assert!(layout.store_path("files/.config/t/conf").exists());
        assert!(
            Manifest::load(&layout).unwrap().app("t").is_some(),
            "预演从 manifest 里删掉了条目"
        );
    }

    /// 预演也要把「会失败」报出来 —— 这正是它最该做的事。
    #[test]
    fn dry_run_unadopt_surfaces_a_failure_that_the_real_run_would_hit() {
        let (_h, layout) = init_home("ops-dry-un-fail", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        add(&layout, "t", false, false, false).expect("add");
        // store 内容消失 → 软链悬空
        fs::remove_file(layout.store_path("files/.config/t/conf")).unwrap();

        let err = unadopt(&layout, "t", true).expect_err("预演该报失败");
        assert!(
            format!("{err:#}").contains("预演"),
            "要说清这是预演的结论：{err:#}"
        );
        // 预演失败也不能动东西
        assert!(is_symlink(&layout.expand("~/.config/t/conf")));
    }

    #[test]
    fn dry_run_apply_predicts_relinking_without_doing_it() {
        let (_h, layout) = init_home("ops-dry-apply", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        add(&layout, "t", false, false, false).expect("add");
        let target = layout.expand("~/.config/t/conf");
        fs::remove_file(&target).unwrap();

        {
            let out = apply(&layout, false, true).expect("dry-run apply");
            assert!(out.dry_run);
            assert_eq!(out.items[0].action, ApplyAction::Linked, "该预测会建链");
            assert!(fs::symlink_metadata(&target).is_err(), "预演真的把链建了");
        }

        // 真跑一遍，结论要对得上
        let real = apply(&layout, false, false).expect("apply");
        assert_eq!(real.items[0].action, ApplyAction::Linked);
        assert!(is_symlink(&target));
    }

    // ── sync ────────────────────────────────────────────────────

    /// 没有 remote 时 sync 仍要能提交本地改动并落地。
    #[test]
    fn sync_commits_locally_without_a_remote() {
        let (_h, layout) = init_home("ops-sync-local", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        add(&layout, "t", false, false, false).expect("add");

        // 透过软链改内容 —— 等于改了 store 工作树
        fs::write(layout.expand("~/.config/t/conf"), "changed\n").unwrap();

        let out = sync(&layout, None, false).expect("sync");
        assert!(out.commit.is_some(), "该提交本地改动");
        assert!(!out.pushed, "没有 remote 就不该报已推送");
        assert_eq!(out.pull, PullOutcome::Skipped);
        assert!(!out.dry_run);
        assert!(
            Git::new(layout.store()).dirty_files().unwrap().is_empty(),
            "提交后工作树该干净"
        );
    }

    /// `sync --dry-run` 报出「将提交哪些」，但不提交。
    #[test]
    fn dry_run_sync_reports_pending_changes_without_committing() {
        let (_h, layout) = init_home("ops-dry-sync", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");
        add(&layout, "t", false, false, false).expect("add");
        fs::write(layout.expand("~/.config/t/conf"), "changed\n").unwrap();

        let out = sync(&layout, None, true).expect("dry-run sync");
        assert!(out.dry_run);
        assert!(out.commit.is_none());
        assert!(!out.pushed);
        let would = out.would_commit.expect("该报出将提交的文件");
        assert!(
            would.iter().any(|f| f.contains("files/.config/t/conf")),
            "将提交的清单里该有那个文件：{would:?}"
        );
        assert!(
            !Git::new(layout.store()).dirty_files().unwrap().is_empty(),
            "预演不该真的提交"
        );
    }

    #[test]
    fn sync_refuses_when_store_is_not_a_repo() {
        let home = TempHome::new("ops-sync-norepo");
        let layout = home.layout();
        init(&layout, None, Some("d")).expect("init");
        fs::remove_dir_all(layout.store().join(".git")).unwrap();

        assert!(sync(&layout, None, false).is_err());
    }

    // ── show ────────────────────────────────────────────────────

    #[test]
    fn show_lists_paths_with_their_current_state() {
        let (_h, layout) = init_home("ops-show", &[".config/t/conf"]);
        write_home_file(&layout, ".config/t/conf", "body\n");

        // 纳管前
        let before = show(&layout, "t").expect("show");
        assert!(!before.managed);
        assert!(before.detected, "detect 路径存在就该算检测到");
        assert_eq!(before.paths.len(), 1);
        assert_eq!(before.paths[0].target, "~/.config/t/conf");
        assert_eq!(
            before.paths[0].store.as_deref(),
            Some("files/.config/t/conf")
        );
        assert!(before.paths[0].exists);
        assert!(before.adopted_by.is_none());

        // 纳管后
        add(&layout, "t", false, false, false).expect("add");
        let after = show(&layout, "t").expect("show");
        assert!(after.managed);
        assert_eq!(after.adopted_by.as_deref(), Some("test-device"));
        assert_eq!(after.paths[0].state, LinkState::Linked);
    }

    /// 未 init 也要能看定义 —— 用户装完第一件事就是想知道会动什么。
    #[test]
    fn show_works_before_init() {
        let home = TempHome::new("ops-show-uninit");
        let layout = home.layout();
        let out = show(&layout, "ghostty").expect("未 init 也该能 show");
        assert!(!out.managed);
        assert!(!out.paths.is_empty());
    }

    #[test]
    fn show_reports_unknown_app() {
        let home = TempHome::new("ops-show-unknown");
        let err = show(&home.layout(), "nope").expect_err("该失败");
        assert_eq!(crate::errors::kind_of(&err), crate::ErrorKind::UnknownApp);
    }

    // ── init ────────────────────────────────────────────────────

    #[test]
    fn init_is_idempotent_and_keeps_the_device_name() {
        let home = TempHome::new("ops-init");
        let layout = home.layout();

        let first = init(&layout, None, Some("my-mac")).expect("init");
        assert!(!first.already);
        assert_eq!(first.device, "my-mac");
        assert!(layout.manifest_file().exists());
        assert!(
            layout.store().join(".gitignore").exists(),
            "该挡掉 .DS_Store"
        );

        // 重复执行不该改设备名
        let second = init(&layout, None, None).expect("再 init");
        assert!(second.already);
        assert_eq!(second.device, "my-mac");
    }
}
