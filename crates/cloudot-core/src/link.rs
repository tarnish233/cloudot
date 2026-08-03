use crate::Layout;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// 目标路径当前的链接状态。
///
/// symlink 策略下不存在「内容漂移」，所以状态就是这五种链接形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// 一切正常：target 是指向 store 的软链，store 内容在。
    Linked,
    /// store 里没有内容 —— 仓库层面的问题，不要动本地文件。
    StoreMissing,
    /// target 不存在，需要 `cloudot apply` 建链。
    Missing,
    /// target 是实体文件而非软链。可能是新机器上的旧配置，
    /// 也可能是某个 App 用「替换写入」把软链顶掉了。**内容可能比 store 新，不能盲目覆盖。**
    ReplacedByFile,
    /// target 是软链但指向别处（大概率被别的工具管着），cloudot 不碰。
    ForeignSymlink,
}

impl LinkState {
    pub fn is_ok(self) -> bool {
        matches!(self, LinkState::Linked)
    }

    pub fn describe(self) -> &'static str {
        match self {
            LinkState::Linked => "已链接",
            LinkState::StoreMissing => "store 内容缺失",
            LinkState::Missing => "本地缺失，待 apply",
            LinkState::ReplacedByFile => "本地是实体文件，未链接",
            LinkState::ForeignSymlink => "本地软链指向别处",
        }
    }
}

pub fn inspect(target: &Path, store: &Path) -> LinkState {
    if fs::symlink_metadata(store).is_err() {
        return LinkState::StoreMissing;
    }
    match fs::symlink_metadata(target) {
        Err(_) => LinkState::Missing,
        Ok(md) if md.file_type().is_symlink() => match fs::read_link(target) {
            Ok(dest) if same_path(&dest, store) => LinkState::Linked,
            _ => LinkState::ForeignSymlink,
        },
        Ok(_) => LinkState::ReplacedByFile,
    }
}

/// 软链是用绝对路径写的，通常直接相等即可；canonicalize 兜住
/// `/Users` ↔ `/private/Users` 之类的差异。
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptAction {
    /// 本来就已经链好了，什么都没做。
    AlreadyLinked,
    /// store 里已有内容（别的机器纳管过），只补建本地软链。
    LinkedFromStore,
    /// 本地配置被移进 store，然后建软链。
    MovedIntoStore,
}

#[derive(Debug, Clone)]
pub struct AdoptReport {
    pub action: AdoptAction,
    pub backup: Option<PathBuf>,
}

/// 把单个文件纳管：备份 → 移进 store → 建软链。
///
/// 幂等；遇到别的工具管着的软链会直接报错而不是覆盖。
pub fn adopt_file(
    layout: &Layout,
    target: &Path,
    store: &Path,
    stamp: &str,
    force: bool,
) -> Result<AdoptReport> {
    if let Ok(md) = fs::symlink_metadata(target) {
        if md.file_type().is_symlink() {
            let dest = fs::read_link(target).unwrap_or_default();
            if same_path(&dest, store) {
                return Ok(AdoptReport {
                    action: AdoptAction::AlreadyLinked,
                    backup: None,
                });
            }
            return Err(crate::tagged(
                crate::ErrorKind::ForeignSymlink,
                format!(
                    "{} 已经是指向 {} 的软链，看起来由别的工具管理，cloudot 不会动它",
                    layout.contract(target),
                    dest.display()
                ),
            ));
        }
        if md.file_type().is_dir() {
            return Err(crate::tagged(
                crate::ErrorKind::Unsupported,
                format!(
                    "{} 是目录。当前版本只支持单文件纳管（目录整体链接会带上需要排除的运行时文件）",
                    layout.contract(target)
                ),
            ));
        }
    }

    let target_exists = fs::symlink_metadata(target).is_ok();
    let store_exists = fs::symlink_metadata(store).is_ok();

    if store_exists {
        // store 已有内容 —— 别的机器纳管过。本地那份要么不存在，要么会被顶掉。
        let mut backup = None;
        if target_exists {
            if !force {
                return Err(crate::tagged(
                    crate::ErrorKind::NeedsForce,
                    format!(
                        "store 里已有 {} 的内容，而本地也存在一份实体文件。\n\
                         加 --force 会先把本地那份备份到 ~/.cloudot/backups 再用 store 的版本覆盖。",
                        layout.contract(target)
                    ),
                ));
            }
            backup = Some(backup_file(layout, target, stamp)?);
            fs::remove_file(target).with_context(|| format!("删除 {} 失败", target.display()))?;
        }
        make_link(target, store)?;
        return Ok(AdoptReport {
            action: AdoptAction::LinkedFromStore,
            backup,
        });
    }

    if !target_exists {
        bail!(
            "{} 不存在，store 里也没有对应内容，没什么可纳管的",
            layout.contract(target)
        );
    }

    // 常规路径：本地有配置，store 是空的。
    let backup = backup_file(layout, target, stamp)?;
    if let Some(parent) = store.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    move_file(target, store)?;
    make_link(target, store)?;
    Ok(AdoptReport {
        action: AdoptAction::MovedIntoStore,
        backup: Some(backup),
    })
}

/// 预测 [`adopt_file`] 会做什么，但**不动任何文件**。给 `--dry-run` 用。
///
/// 必须和 `adopt_file` 的分支结构一一对应（同样的顺序、同样的判据），否则预演
/// 会骗人 —— 那比没有预演更糟。测试 `plan_matches_real_adopt` 钉住这件事：
/// 每个分支都跑一遍预测和真做，比对结论。
///
/// `Ok(action)` 表示真跑会成功并做出 `action`；`Err` 表示真跑会以同样的分类失败。
pub fn plan_adopt(
    layout: &Layout,
    target: &Path,
    store: &Path,
    force: bool,
) -> Result<AdoptAction> {
    if let Ok(md) = fs::symlink_metadata(target) {
        if md.file_type().is_symlink() {
            let dest = fs::read_link(target).unwrap_or_default();
            if same_path(&dest, store) {
                return Ok(AdoptAction::AlreadyLinked);
            }
            return Err(crate::tagged(
                crate::ErrorKind::ForeignSymlink,
                format!(
                    "{} 已经是指向 {} 的软链，看起来由别的工具管理，cloudot 不会动它",
                    layout.contract(target),
                    dest.display()
                ),
            ));
        }
        if md.file_type().is_dir() {
            return Err(crate::tagged(
                crate::ErrorKind::Unsupported,
                format!(
                    "{} 是目录。当前版本只支持单文件纳管（目录整体链接会带上需要排除的运行时文件）",
                    layout.contract(target)
                ),
            ));
        }
    }

    let target_exists = fs::symlink_metadata(target).is_ok();
    let store_exists = fs::symlink_metadata(store).is_ok();

    if store_exists {
        if target_exists && !force {
            return Err(crate::tagged(
                crate::ErrorKind::NeedsForce,
                format!(
                    "store 里已有 {} 的内容，而本地也存在一份实体文件。\n\
                     加 --force 会先把本地那份备份到 ~/.cloudot/backups 再用 store 的版本覆盖。",
                    layout.contract(target)
                ),
            ));
        }
        return Ok(AdoptAction::LinkedFromStore);
    }

    if !target_exists {
        bail!(
            "{} 不存在，store 里也没有对应内容，没什么可纳管的",
            layout.contract(target)
        );
    }

    Ok(AdoptAction::MovedIntoStore)
}

/// 撤销一次 [`adopt_file`]，尽量恢复到调用前的样子。
///
/// 必须按当时实际做了什么来分别处理 —— 特别是 `LinkedFromStore` 那种情况，
/// store 里的内容是**别的机器**放进去的，回滚时绝对不能删。
pub fn revert_adopt(target: &Path, store: &Path, report: &AdoptReport) -> Result<()> {
    match report.action {
        AdoptAction::AlreadyLinked => Ok(()),
        AdoptAction::MovedIntoStore => {
            remove_if_symlink(target)?;
            move_file(store, target)
        }
        AdoptAction::LinkedFromStore => {
            remove_if_symlink(target)?;
            match &report.backup {
                Some(backup) => fs::copy(backup, target)
                    .map(|_| ())
                    .with_context(|| format!("从备份 {} 还原失败", backup.display())),
                None => Ok(()),
            }
        }
    }
}

fn remove_if_symlink(target: &Path) -> Result<()> {
    if let Ok(md) = fs::symlink_metadata(target)
        && md.file_type().is_symlink()
    {
        return fs::remove_file(target)
            .with_context(|| format!("移除软链 {} 失败", target.display()));
    }
    Ok(())
}

/// 反向操作：把软链换回实体文件，并从 store 里删掉。
pub fn unadopt_file(layout: &Layout, target: &Path, store: &Path) -> Result<()> {
    let store_exists = fs::symlink_metadata(store).is_ok();
    match fs::symlink_metadata(target) {
        Ok(md) if md.file_type().is_symlink() => {
            if !store_exists {
                bail!(
                    "{} 是软链但 store 里没有内容，先手动确认再操作",
                    layout.contract(target)
                );
            }
            fs::remove_file(target)
                .with_context(|| format!("删除软链 {} 失败", target.display()))?;
            fs::copy(store, target).with_context(|| {
                format!("把 {} 复制回 {} 失败", store.display(), target.display())
            })?;
        }
        Ok(_) => {
            // 已经是实体文件了，本地内容优先，不动它。
        }
        Err(_) => {
            if store_exists {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(store, target)?;
            }
        }
    }
    if store_exists {
        fs::remove_file(store)
            .with_context(|| format!("从 store 删除 {} 失败", store.display()))?;
    }
    Ok(())
}

/// 备份到 `~/.cloudot/backups/<时间戳>/<家目录相对路径>`，保留原目录结构。
pub fn backup_file(layout: &Layout, target: &Path, stamp: &str) -> Result<PathBuf> {
    let rel = target
        .strip_prefix(layout.home())
        .unwrap_or_else(|_| Path::new(target.file_name().unwrap_or_default()));
    let dest = layout.backups().join(stamp).join(rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建备份目录 {} 失败", parent.display()))?;
    }
    fs::copy(target, &dest)
        .with_context(|| format!("备份 {} 到 {} 失败", target.display(), dest.display()))?;
    Ok(dest)
}

fn make_link(target: &Path, store: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    std::os::unix::fs::symlink(store, target)
        .with_context(|| format!("建立软链 {} -> {} 失败", target.display(), store.display()))
}

/// 优先 rename（保留权限与时间戳），跨卷时退化为 copy + remove。
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)
        .with_context(|| format!("复制 {} 到 {} 失败", from.display(), to.display()))?;
    fs::remove_file(from).with_context(|| format!("删除 {} 失败", from.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempHome;

    /// 造一个假家目录，返回 (守卫, layout, target, store)。
    fn fixture(tag: &str) -> (TempHome, Layout, PathBuf, PathBuf) {
        let home = TempHome::new(tag);
        let layout = home.layout();
        let target = layout.expand("~/.config/ghostty/config");
        let store = layout.store_path("files/.config/ghostty/config");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir_all(store.parent().unwrap()).unwrap();
        (home, layout, target, store)
    }

    // ── plan_adopt：预演必须和真做说一样的话 ──────────────────────
    //
    // 这是 --dry-run 唯一真正危险的地方：预测和实做走的是两段独立的分支代码，
    // 一旦漂移，预演就会骗人 —— 那比没有预演更糟（用户会照着假报告做决定）。

    /// 逐个场景比对：`plan_adopt` 的结论 == `adopt_file` 真跑的结果。
    #[test]
    fn plan_matches_real_adopt_in_every_branch() {
        // (标签, 布置现场, force, 期望)
        type Setup = fn(&Path, &Path);
        let cases: &[(&str, Setup, bool)] = &[
            // 本地有、store 空 → MovedIntoStore
            (
                "local-only",
                |t, _s| fs::write(t, "local\n").unwrap(),
                false,
            ),
            // store 有、本地无 → LinkedFromStore
            (
                "store-only",
                |_t, s| fs::write(s, "store\n").unwrap(),
                false,
            ),
            // 两边都有 + force → LinkedFromStore
            (
                "both-force",
                |t, s| {
                    fs::write(s, "store\n").unwrap();
                    fs::write(t, "local\n").unwrap();
                },
                true,
            ),
            // 已经链好 → AlreadyLinked
            (
                "already",
                |t, s| {
                    fs::write(s, "store\n").unwrap();
                    std::os::unix::fs::symlink(s, t).unwrap();
                },
                false,
            ),
        ];

        for (tag, setup, force) in cases {
            // 预测和真做各用一套独立的假家目录，避免互相影响
            let (_h1, l1, t1, s1) = fixture(&format!("plan-{tag}"));
            setup(&t1, &s1);
            let planned = plan_adopt(&l1, &t1, &s1, *force).expect(tag);

            let (_h2, l2, t2, s2) = fixture(&format!("real-{tag}"));
            setup(&t2, &s2);
            let real = adopt_file(&l2, &t2, &s2, "stamp", *force).expect(tag);

            assert_eq!(
                planned, real.action,
                "{tag}：预演说 {planned:?}，真做是 {:?}",
                real.action
            );
        }
    }

    /// 会失败的场景，预演也必须以**同样的分类**失败。
    ///
    /// 分类相同很重要：GUI 靠 `kind` 决定给什么引导，预演和真做给出不同引导
    /// 就等于预演没用。
    #[test]
    fn plan_reports_the_same_failures_as_real_adopt() {
        // 两边都有内容但没 --force → NeedsForce
        let (_h1, l1, t1, s1) = fixture("plan-needs-force");
        fs::write(&s1, "store\n").unwrap();
        fs::write(&t1, "local\n").unwrap();
        let planned = plan_adopt(&l1, &t1, &s1, false).expect_err("该拒绝");
        assert_eq!(
            crate::errors::kind_of(&planned),
            crate::ErrorKind::NeedsForce
        );

        let (_h2, l2, t2, s2) = fixture("real-needs-force");
        fs::write(&s2, "store\n").unwrap();
        fs::write(&t2, "local\n").unwrap();
        let real = adopt_file(&l2, &t2, &s2, "stamp", false).expect_err("该拒绝");
        assert_eq!(crate::errors::kind_of(&real), crate::ErrorKind::NeedsForce);

        // 别的工具管着的软链 → ForeignSymlink
        let (_h3, l3, t3, s3) = fixture("plan-foreign");
        fs::write(&s3, "store\n").unwrap();
        let elsewhere = l3.expand("~/elsewhere");
        fs::write(&elsewhere, "other\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, &t3).unwrap();
        assert_eq!(
            crate::errors::kind_of(&plan_adopt(&l3, &t3, &s3, false).expect_err("该拒绝")),
            crate::ErrorKind::ForeignSymlink
        );

        // 目录 → Unsupported
        let (_h4, l4, t4, s4) = fixture("plan-dir");
        fs::write(&s4, "store\n").unwrap();
        fs::remove_file(&t4).ok();
        fs::create_dir_all(&t4).unwrap();
        assert_eq!(
            crate::errors::kind_of(&plan_adopt(&l4, &t4, &s4, false).expect_err("该拒绝")),
            crate::ErrorKind::Unsupported
        );
    }

    /// 预演绝不能碰任何文件 —— 这是它唯一的承诺。
    #[test]
    fn plan_adopt_touches_nothing() {
        let (_h, layout, target, store) = fixture("plan-readonly");
        fs::write(&target, "local\n").unwrap();

        plan_adopt(&layout, &target, &store, false).expect("能预测");

        assert!(
            !fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink(),
            "预演把本地文件换成软链了"
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "local\n");
        assert!(!store.exists(), "预演往 store 里写了东西");
        assert!(
            !layout.backups().exists(),
            "预演建了备份目录 —— 备份也是写盘"
        );
    }

    // ── inspect 状态机：五种形态各来一发 ────────────────────────────

    #[test]
    fn inspect_store_missing() {
        let (_h, _l, target, store) = fixture("st-missing");
        fs::write(&target, "x").unwrap();
        assert_eq!(inspect(&target, &store), LinkState::StoreMissing);
    }

    #[test]
    fn inspect_target_missing() {
        let (_h, _l, target, store) = fixture("tg-missing");
        fs::write(&store, "x").unwrap();
        assert_eq!(inspect(&target, &store), LinkState::Missing);
    }

    #[test]
    fn inspect_linked() {
        let (_h, _l, target, store) = fixture("linked");
        fs::write(&store, "x").unwrap();
        std::os::unix::fs::symlink(&store, &target).unwrap();
        assert_eq!(inspect(&target, &store), LinkState::Linked);
    }

    #[test]
    fn inspect_replaced_by_file() {
        let (_h, _l, target, store) = fixture("replaced");
        fs::write(&store, "old").unwrap();
        fs::write(&target, "new").unwrap();
        assert_eq!(inspect(&target, &store), LinkState::ReplacedByFile);
    }

    #[test]
    fn inspect_foreign_symlink() {
        let (_h, layout, target, store) = fixture("foreign-inspect");
        fs::write(&store, "x").unwrap();
        let other = layout.home().join("managed-by-someone-else");
        fs::write(&other, "y").unwrap();
        std::os::unix::fs::symlink(&other, &target).unwrap();
        assert_eq!(inspect(&target, &store), LinkState::ForeignSymlink);
    }

    /// 软链还在但 store 文件被删 —— 归到 StoreMissing，因为问题出在仓库侧。
    #[test]
    fn inspect_dangling_link_reports_store_missing() {
        let (_h, _l, target, store) = fixture("dangling");
        fs::write(&store, "x").unwrap();
        std::os::unix::fs::symlink(&store, &target).unwrap();
        fs::remove_file(&store).unwrap();
        assert_eq!(inspect(&target, &store), LinkState::StoreMissing);
    }

    // ── adopt_file ───────────────────────────────────────────────

    #[test]
    fn adopt_moves_into_store_and_links_back() {
        let (_h, layout, target, store) = fixture("adopt");
        fs::write(&target, "theme = dark\n").unwrap();

        let report = adopt_file(&layout, &target, &store, "stamp", false).unwrap();

        assert_eq!(report.action, AdoptAction::MovedIntoStore);
        assert_eq!(inspect(&target, &store), LinkState::Linked);
        assert_eq!(fs::read_to_string(&store).unwrap(), "theme = dark\n");
        // 透过软链读到的必须还是原内容
        assert_eq!(fs::read_to_string(&target).unwrap(), "theme = dark\n");
        assert_eq!(
            fs::read_to_string(report.backup.unwrap()).unwrap(),
            "theme = dark\n"
        );
    }

    #[test]
    fn adopt_is_idempotent() {
        let (_h, layout, target, store) = fixture("idempotent");
        fs::write(&target, "x").unwrap();
        adopt_file(&layout, &target, &store, "s1", false).unwrap();

        let again = adopt_file(&layout, &target, &store, "s2", false).unwrap();
        assert_eq!(again.action, AdoptAction::AlreadyLinked);
        assert!(again.backup.is_none(), "重复 adopt 不该再备份");
    }

    #[test]
    fn adopt_links_from_store_when_local_absent() {
        let (_h, layout, target, store) = fixture("from-store");
        fs::write(&store, "from-store\n").unwrap();

        let report = adopt_file(&layout, &target, &store, "s", false).unwrap();
        assert_eq!(report.action, AdoptAction::LinkedFromStore);
        assert!(report.backup.is_none());
        assert_eq!(fs::read_to_string(&target).unwrap(), "from-store\n");
    }

    #[test]
    fn adopt_needs_force_when_both_sides_have_content() {
        let (_h, layout, target, store) = fixture("both-sides");
        fs::write(&store, "from-store\n").unwrap();
        fs::write(&target, "from-local\n").unwrap();

        assert!(
            adopt_file(&layout, &target, &store, "s", false).is_err(),
            "默认不该覆盖本地文件"
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "from-local\n");

        let report = adopt_file(&layout, &target, &store, "s2", true).unwrap();
        assert_eq!(report.action, AdoptAction::LinkedFromStore);
        assert_eq!(fs::read_to_string(&target).unwrap(), "from-store\n");
        assert_eq!(
            fs::read_to_string(report.backup.unwrap()).unwrap(),
            "from-local\n",
            "--force 覆盖前必须留下本地那份"
        );
    }

    #[test]
    fn adopt_refuses_foreign_symlink_and_leaves_it_alone() {
        let (_h, layout, target, store) = fixture("foreign-adopt");
        let other = layout.home().join("managed-by-someone-else");
        fs::write(&other, "y").unwrap();
        std::os::unix::fs::symlink(&other, &target).unwrap();

        assert!(adopt_file(&layout, &target, &store, "s", true).is_err());
        assert_eq!(
            fs::read_link(&target).unwrap(),
            other,
            "别的工具的软链必须原样保留"
        );
    }

    #[test]
    fn adopt_refuses_directories() {
        let (_h, layout, target, store) = fixture("dir");
        fs::create_dir_all(&target).unwrap();
        assert!(adopt_file(&layout, &target, &store, "s", false).is_err());
    }

    #[test]
    fn adopt_errors_when_nothing_exists() {
        let (_h, layout, target, store) = fixture("nothing");
        assert!(adopt_file(&layout, &target, &store, "s", false).is_err());
    }

    #[test]
    fn adopt_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (_h, layout, target, store) = fixture("perms");
        fs::write(&target, "secret\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();

        adopt_file(&layout, &target, &store, "s", false).unwrap();

        let mode = fs::metadata(&store).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "移进 store 后权限位不能变宽");
    }

    // ── unadopt_file ─────────────────────────────────────────────

    #[test]
    fn unadopt_restores_real_file_and_clears_store() {
        let (_h, layout, target, store) = fixture("unadopt");
        fs::write(&target, "content\n").unwrap();
        adopt_file(&layout, &target, &store, "s", false).unwrap();

        unadopt_file(&layout, &target, &store).unwrap();

        let md = fs::symlink_metadata(&target).unwrap();
        assert!(!md.file_type().is_symlink(), "应该变回实体文件");
        assert_eq!(fs::read_to_string(&target).unwrap(), "content\n");
        assert!(fs::symlink_metadata(&store).is_err(), "store 里应已清掉");
    }

    #[test]
    fn unadopt_recreates_target_if_it_was_deleted() {
        let (_h, layout, target, store) = fixture("unadopt-gone");
        fs::write(&target, "content\n").unwrap();
        adopt_file(&layout, &target, &store, "s", false).unwrap();
        fs::remove_file(&target).unwrap();

        unadopt_file(&layout, &target, &store).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "content\n");
    }
}
