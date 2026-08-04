use crate::Layout;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

pub const MANIFEST_VERSION: u32 = 1;

/// 落地策略。
///
/// 目前只有 symlink。等 `cloudot doctor` 报出某个 App 反复把软链换成实体文件时，
/// 再为它加 `copy` —— 那时才有真实依据决定 copy 的语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    #[default]
    Symlink,
}

/// `~/.cloudot/store/manifest.toml` —— 纳管清单，**进 git**，跨机器共享。
///
/// 路径一律以 `~/` 形式存储，这样换机器（甚至换用户名）都能直接 apply。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub apps: Vec<ManagedApp>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            apps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedApp {
    pub id: String,
    pub name: String,
    /// 最初是哪台机器纳管的，纯信息性字段。
    pub adopted_by: String,
    pub files: Vec<ManagedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedFile {
    /// 家目录下的目标路径，形如 `~/.config/ghostty/config`
    pub target: String,
    /// store 内相对路径，形如 `files/.config/ghostty/config`
    pub store: String,
    #[serde(default)]
    pub strategy: Strategy,
}

impl Manifest {
    /// 文件不存在时返回默认空清单（`init` 之前也能安全调用）。
    pub fn load(layout: &Layout) -> Result<Self> {
        let path = layout.manifest_file();
        match fs::read_to_string(&path) {
            Ok(raw) => toml::from_str(&raw).with_context(|| format!("{} 解析失败", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("读取 {} 失败", path.display())),
        }
    }

    pub fn save(&self, layout: &Layout) -> Result<()> {
        let path = layout.manifest_file();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = format!(
            "# cloudot 纳管清单 —— 由 cloudot 维护，手改前请先 `cloudot status`\n{}",
            toml::to_string_pretty(self)?
        );
        fs::write(&path, body).with_context(|| format!("写入 {} 失败", path.display()))
    }

    /// 逐条校验路径是否安全可用；返回所有问题（不是遇到第一个就停）。
    ///
    /// **manifest 进 git、跨机器共享，所以它是外部输入。** `add` 写进去的路径过了
    /// [`Layout::store_rel_for`] 的门禁，但从远端 clone / pull 回来的那份没有 ——
    /// 一个被改过的仓库或者手滑的编辑就能让 `apply` 往家目录之外建软链（实测不需要
    /// `--force`：目标不存在时 `apply` 会直接建链，还会顺手把中间目录建出来）。
    ///
    /// 校验只在**写路径**（`apply` / `add` / `unadopt`）上强制。`status` / `show`
    /// 照旧能读 —— 它们不动盘，而且清单坏掉的时候恰恰最需要能看见里面是什么；
    /// `doctor` 把问题报成 error 级检查项，这是用户发现它的正常途径。
    pub fn problems(&self, layout: &Layout) -> Vec<String> {
        let mut out = Vec::new();

        if self.version > MANIFEST_VERSION {
            out.push(format!(
                "manifest 版本是 {}，这个 cloudot 只认到 {MANIFEST_VERSION} —— \
                 先升级 cloudot，别用旧版本去改新格式的清单",
                self.version
            ));
        }

        let mut seen: Vec<&str> = Vec::new();
        for app in &self.apps {
            for file in &app.files {
                let target = layout.expand(&file.target);
                // 复用 add 那道门禁：家目录之外、`..`、非 UTF-8 全在这里挡掉。
                // 两条路径共用同一个判据，才不会出现「add 拒绝、apply 放行」。
                match layout.store_rel_for(&target) {
                    Err(e) => out.push(format!("{} 的 {} —— {e}", app.id, file.target)),
                    // store 必须正好是 target 推导出来的位置。这一条同时挡住
                    // `../..` 逃出 store、绝对路径 store、以及 target/store 张冠李戴。
                    Ok(expected) if expected != file.store => out.push(format!(
                        "{} 的 {} 对应的 store 位置该是 {expected}，清单里写的却是 {} —— \
                         软链会指向 store 之外",
                        app.id, file.target, file.store
                    )),
                    Ok(_) => {}
                }

                if seen.contains(&file.target.as_str()) {
                    out.push(format!(
                        "{} 重复出现（同一个路径被纳管两次，撤销时会互相干扰）",
                        file.target
                    ));
                } else {
                    seen.push(&file.target);
                }
            }
        }
        out
    }

    /// [`Manifest::problems`] 的硬门禁版本，给会动文件的命令用。
    ///
    /// 刻意整体拒绝而不是跳过坏条目：清单是机器维护的，它坏了就是异常状态，
    /// 「6 条里默默少做 1 条」比明确报错更难查。
    pub fn ensure_safe(&self, layout: &Layout) -> Result<()> {
        let problems = self.problems(layout);
        if problems.is_empty() {
            return Ok(());
        }
        Err(crate::tagged(
            crate::ErrorKind::Unsupported,
            format!(
                "{} 里有不安全的路径，什么都没做：\n{}\n\n\
                 这份清单是跨机器共享的（进 git），正常情况下由 cloudot 自己维护。\n\
                 如果不是你手改的，先看一遍 `git log -p manifest.toml` 确认来源。",
                layout.manifest_file().display(),
                problems
                    .iter()
                    .map(|p| format!("  {p}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        ))
    }

    pub fn app(&self, id: &str) -> Option<&ManagedApp> {
        self.apps.iter().find(|a| a.id == id)
    }

    /// 按 id 插入或替换。
    pub fn upsert(&mut self, app: ManagedApp) {
        match self.apps.iter_mut().find(|a| a.id == app.id) {
            Some(slot) => *slot = app,
            None => self.apps.push(app),
        }
        self.apps.sort_by(|a, b| a.id.cmp(&b.id));
    }

    /// 移除并返回被移除的条目。
    pub fn remove(&mut self, id: &str) -> Option<ManagedApp> {
        let idx = self.apps.iter().position(|a| a.id == id)?;
        Some(self.apps.remove(idx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> ManagedApp {
        ManagedApp {
            id: id.into(),
            name: id.into(),
            adopted_by: "a".into(),
            files: vec![ManagedFile {
                target: format!("~/.config/{id}/config"),
                store: format!("files/.config/{id}/config"),
                strategy: Strategy::Symlink,
            }],
        }
    }

    #[test]
    fn upsert_replaces_by_id_and_keeps_sorted() {
        let mut m = Manifest::default();
        m.upsert(app("zed"));
        m.upsert(app("ghostty"));
        assert_eq!(
            m.apps.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["ghostty", "zed"]
        );

        let mut updated = app("ghostty");
        updated.name = "Ghostty 改名了".into();
        m.upsert(updated);
        assert_eq!(m.apps.len(), 2);
        assert_eq!(m.app("ghostty").unwrap().name, "Ghostty 改名了");
    }

    #[test]
    fn remove_returns_entry_and_is_none_when_absent() {
        let mut m = Manifest::default();
        m.upsert(app("ghostty"));
        assert_eq!(m.remove("ghostty").unwrap().id, "ghostty");
        assert!(m.apps.is_empty());
        assert!(m.remove("ghostty").is_none());
    }

    #[test]
    fn roundtrips_through_disk() {
        let home = crate::testutil::TempHome::new("manifest");
        let layout = home.layout();
        let mut m = Manifest::default();
        m.upsert(app("ghostty"));
        m.save(&layout).unwrap();

        let back = Manifest::load(&layout).unwrap();
        assert_eq!(back.version, MANIFEST_VERSION);
        assert_eq!(back.apps.len(), 1);
        assert_eq!(back.apps[0].files[0].strategy, Strategy::Symlink);
    }

    #[test]
    fn load_returns_default_when_absent() {
        let home = crate::testutil::TempHome::new("manifest-absent");
        assert!(Manifest::load(&home.layout()).unwrap().apps.is_empty());
    }

    // ── 路径校验 ────────────────────────────────────────────────
    //
    // manifest 进 git、跨机器共享，所以它是**外部输入**。`add` 写进去的路径过了
    // `store_rel_for` 的门禁，从远端 pull 回来的那份没有。这批测试钉住的是
    // 「写命令动手之前必须先拒绝」，实测过不校验时 `apply` 会在家目录之外建软链
    // 且不需要 --force。

    /// 手写一个只含单条 file 的清单，用来逐个构造攻击形态。
    fn manifest_with(target: &str, store: &str) -> Manifest {
        Manifest {
            version: MANIFEST_VERSION,
            apps: vec![ManagedApp {
                id: "evil".into(),
                name: "Evil".into(),
                adopted_by: "attacker".into(),
                files: vec![ManagedFile {
                    target: target.into(),
                    store: store.into(),
                    strategy: Strategy::Symlink,
                }],
            }],
        }
    }

    #[test]
    fn rejects_targets_outside_home() {
        let layout = Layout::with_home("/home/u");
        // 绝对路径：apply 会直接在 /etc 下建软链
        let m = manifest_with("/tmp/victim/planted.txt", "files/evil/payload.txt");
        assert!(!m.problems(&layout).is_empty());
        assert!(m.ensure_safe(&layout).is_err());
    }

    #[test]
    fn rejects_dotdot_escape_in_target() {
        let layout = Layout::with_home("/home/u");
        // `~/../` 绕出家目录 —— expand 之后是 /home/u/../victim
        let m = manifest_with("~/../victim/x.txt", "files/evil/payload.txt");
        assert!(m.ensure_safe(&layout).is_err());
    }

    /// store 字段自己就能逃出 store 目录：软链目标是 store 根 + 这个相对路径。
    #[test]
    fn rejects_store_paths_that_escape_the_store() {
        let layout = Layout::with_home("/home/u");
        let m = manifest_with("~/.config/leaked.txt", "../../../outside.txt");
        let problems = m.problems(&layout);
        assert_eq!(problems.len(), 1, "该只报 store 位置不对：{problems:?}");
        assert!(m.ensure_safe(&layout).is_err());
    }

    /// target 与 store 张冠李戴：内容会串到另一个应用的配置上。
    #[test]
    fn rejects_mismatched_target_and_store() {
        let layout = Layout::with_home("/home/u");
        let m = manifest_with("~/.config/fish/config.fish", "files/.config/ghostty/config");
        assert!(m.ensure_safe(&layout).is_err());
    }

    #[test]
    fn accepts_a_manifest_written_by_add() {
        let layout = Layout::with_home("/home/u");
        let m = manifest_with("~/.config/ghostty/config", "files/.config/ghostty/config");
        assert_eq!(
            m.problems(&layout),
            Vec::<String>::new(),
            "正常清单不该被拦"
        );
        m.ensure_safe(&layout).expect("正常清单该通过");
    }

    /// 未来版本的清单不能被旧 cloudot 拿去改 —— 它读不懂新语义，
    /// 写回去等于把新字段抹掉。
    #[test]
    fn rejects_a_manifest_from_the_future() {
        let layout = Layout::with_home("/home/u");
        let mut m = manifest_with("~/.config/ghostty/config", "files/.config/ghostty/config");
        m.version = MANIFEST_VERSION + 1;
        assert!(m.ensure_safe(&layout).is_err());
    }

    #[test]
    fn rejects_duplicate_targets() {
        let layout = Layout::with_home("/home/u");
        let mut m = manifest_with("~/.config/ghostty/config", "files/.config/ghostty/config");
        let dup = m.apps[0].files[0].clone();
        m.apps[0].files.push(dup);
        assert!(m.ensure_safe(&layout).is_err());
    }

    /// 报告要列出**所有**问题，不是遇到第一个就停 —— 用户得一次看全。
    #[test]
    fn reports_every_problem_at_once() {
        let layout = Layout::with_home("/home/u");
        let mut m = manifest_with("/etc/passwd", "files/evil/a.txt");
        m.apps[0].files.push(ManagedFile {
            target: "~/../escape/b.txt".into(),
            store: "files/evil/b.txt".into(),
            strategy: Strategy::Symlink,
        });
        assert_eq!(m.problems(&layout).len(), 2);
    }

    /// 校验只在写路径强制，`load` 本身必须照旧成功 —— 清单坏掉的时候恰恰
    /// 最需要 `status` / `show` 还能看见里面是什么。
    #[test]
    fn load_still_reads_an_unsafe_manifest() {
        let home = crate::testutil::TempHome::new("manifest-unsafe-load");
        let layout = home.layout();
        manifest_with("/etc/passwd", "files/evil/payload.txt")
            .save(&layout)
            .unwrap();

        let back = Manifest::load(&layout).expect("坏清单也要能读出来");
        assert_eq!(back.apps.len(), 1);
        assert!(!back.problems(&layout).is_empty());
    }
}
