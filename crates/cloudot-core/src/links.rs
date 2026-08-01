use crate::Layout;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

pub const LINKS_VERSION: u32 = 1;

/// `~/.cloudot/links.toml` —— 本机建过哪些软链的记录，**不进 git**。
///
/// 存在的理由：manifest 是共享状态，本机建过的链是本地状态，两者会分叉。
/// 典型场景是另一台机器 `unadopt` 后同步过来 —— manifest 里没了、store 文件也删了，
/// 但本机的软链还在，就成了悬空软链（App 直接读不到配置）。
/// 只靠 manifest 无法发现这种情况，必须有本地记录才能对账。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkRecords {
    pub version: u32,
    #[serde(default)]
    pub links: Vec<LinkRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkRecord {
    pub app: String,
    /// `~/` 形式的目标路径
    pub target: String,
    /// store 内相对路径
    pub store: String,
}

impl LinkRecord {
    /// target 现在是否还是指向本记录 store 路径的软链。
    pub fn still_ours(&self, layout: &Layout) -> bool {
        points_at(&layout.expand(&self.target), &layout.store_path(&self.store))
    }
}

impl Default for LinkRecords {
    fn default() -> Self {
        Self {
            version: LINKS_VERSION,
            links: Vec::new(),
        }
    }
}

impl LinkRecords {
    pub fn load(layout: &Layout) -> Result<Self> {
        let path = layout.links_file();
        match fs::read_to_string(&path) {
            Ok(raw) => {
                toml::from_str(&raw).with_context(|| format!("{} 解析失败", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("读取 {} 失败", path.display())),
        }
    }

    pub fn save(&self, layout: &Layout) -> Result<()> {
        fs::create_dir_all(layout.root())?;
        let body = format!(
            "# cloudot 在本机建过的软链记录（本机状态，不会同步）\n{}",
            toml::to_string_pretty(self)?
        );
        fs::write(layout.links_file(), body)
            .with_context(|| format!("写入 {} 失败", layout.links_file().display()))
    }

    pub fn upsert(&mut self, app: &str, target: &str, store: &str) {
        let record = LinkRecord {
            app: app.to_owned(),
            target: target.to_owned(),
            store: store.to_owned(),
        };
        match self.links.iter_mut().find(|l| l.target == target) {
            Some(slot) => *slot = record,
            None => self.links.push(record),
        }
        self.links.sort_by(|a, b| a.target.cmp(&b.target));
    }

    pub fn remove_target(&mut self, target: &str) {
        self.links.retain(|l| l.target != target);
    }

    pub fn remove_app(&mut self, app: &str) {
        self.links.retain(|l| l.app != app);
    }
}

/// 本机软链记录与 manifest 对不上的条目。
#[derive(Debug, Clone, Serialize)]
pub struct Orphan {
    pub app: String,
    pub target: String,
    pub store: String,
    pub kind: OrphanKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanKind {
    /// 软链还在，但 store 文件已经没了 —— App 会直接读不到配置，最严重。
    Dangling,
    /// manifest 里已无此条目，但软链仍指向 store，且 store 文件还在（暂时还能读）。
    Unmanaged,
}

impl OrphanKind {
    pub fn describe(self) -> &'static str {
        match self {
            OrphanKind::Dangling => "悬空软链：store 里的文件已不存在，配置读不到了",
            OrphanKind::Unmanaged => "已不在 manifest 中，但软链仍指向 store",
        }
    }
}

/// 找出所有孤儿软链。
///
/// 只认「本机记录里有、manifest 里没有、且 target 确实还是指向 store 的软链」这种情况；
/// 记录过时但 target 早已不是我们的软链的，不算问题（由 `apply` 顺手清掉记录）。
pub fn find_orphans(
    layout: &Layout,
    manifest: &crate::Manifest,
    records: &LinkRecords,
) -> Vec<Orphan> {
    let mut out = Vec::new();
    for record in &records.links {
        let still_managed = manifest
            .app(&record.app)
            .is_some_and(|app| app.files.iter().any(|f| f.target == record.target));
        if still_managed {
            continue;
        }

        if !record.still_ours(layout) {
            continue; // 记录过时，target 已经不是我们的软链了
        }
        let store = layout.store_path(&record.store);
        out.push(Orphan {
            app: record.app.clone(),
            target: record.target.clone(),
            store: record.store.clone(),
            kind: if fs::symlink_metadata(&store).is_ok() {
                OrphanKind::Unmanaged
            } else {
                OrphanKind::Dangling
            },
        });
    }
    out
}

/// target 是否是指向 store 的软链（不解析目标是否存在）。
fn points_at(target: &std::path::Path, store: &std::path::Path) -> bool {
    match fs::symlink_metadata(target) {
        Ok(md) if md.file_type().is_symlink() => {
            fs::read_link(target).map(|d| d == store).unwrap_or(false)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ManagedApp, ManagedFile, Strategy};
    use crate::testutil::TempHome;

    const TARGET: &str = "~/.config/ghostty/config";
    const STORE: &str = "files/.config/ghostty/config";

    fn manifest_with_ghostty() -> crate::Manifest {
        let mut m = crate::Manifest::default();
        m.upsert(ManagedApp {
            id: "ghostty".into(),
            name: "Ghostty".into(),
            adopted_by: "a".into(),
            files: vec![ManagedFile {
                target: TARGET.into(),
                store: STORE.into(),
                strategy: Strategy::Symlink,
            }],
        });
        m
    }

    /// 建好软链，并返回对应的本机记录。
    fn linked_fixture(tag: &str, keep_store_file: bool) -> (TempHome, Layout, LinkRecords) {
        let home = TempHome::new(tag);
        let layout = home.layout();
        let target = layout.expand(TARGET);
        let store = layout.store_path(STORE);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir_all(store.parent().unwrap()).unwrap();
        fs::write(&store, "theme = dark\n").unwrap();
        std::os::unix::fs::symlink(&store, &target).unwrap();
        if !keep_store_file {
            fs::remove_file(&store).unwrap();
        }
        let mut records = LinkRecords::default();
        records.upsert("ghostty", TARGET, STORE);
        (home, layout, records)
    }

    #[test]
    fn managed_links_are_not_orphans() {
        let (_h, layout, records) = linked_fixture("managed", true);
        let orphans = find_orphans(&layout, &manifest_with_ghostty(), &records);
        assert!(orphans.is_empty());
    }

    /// 另一台机器 unadopt 后同步过来：manifest 没了、store 文件也删了。
    /// 这时本机软链已经悬空，App 直接读不到配置 —— 必须报出来。
    #[test]
    fn dropped_from_manifest_with_store_gone_is_dangling() {
        let (_h, layout, records) = linked_fixture("dangling", false);
        let orphans = find_orphans(&layout, &crate::Manifest::default(), &records);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].kind, OrphanKind::Dangling);
        assert_eq!(orphans[0].app, "ghostty");
    }

    #[test]
    fn dropped_from_manifest_with_store_present_is_unmanaged() {
        let (_h, layout, records) = linked_fixture("unmanaged", true);
        let orphans = find_orphans(&layout, &crate::Manifest::default(), &records);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].kind, OrphanKind::Unmanaged);
    }

    /// 记录里有，但 target 早已不是我们的软链 —— 不是问题，只是记录过时。
    #[test]
    fn stale_record_is_not_reported() {
        let home = TempHome::new("stale");
        let layout = home.layout();
        let target = layout.expand(TARGET);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "本地实体文件").unwrap();
        let mut records = LinkRecords::default();
        records.upsert("ghostty", TARGET, STORE);

        let orphans = find_orphans(&layout, &crate::Manifest::default(), &records);
        assert!(orphans.is_empty());
    }

    #[test]
    fn upsert_replaces_by_target_and_stays_sorted() {
        let mut r = LinkRecords::default();
        r.upsert("zsh", "~/.zshrc", "files/.zshrc");
        r.upsert("ghostty", TARGET, STORE);
        r.upsert("ghostty", TARGET, "files/other");

        assert_eq!(r.links.len(), 2);
        assert_eq!(r.links[0].target, TARGET);
        assert_eq!(r.links[0].store, "files/other");
    }

    #[test]
    fn remove_app_drops_all_its_targets() {
        let mut r = LinkRecords::default();
        r.upsert("ghostty", TARGET, STORE);
        r.upsert("zsh", "~/.zshrc", "files/.zshrc");
        r.remove_app("ghostty");
        assert_eq!(r.links.len(), 1);
        assert_eq!(r.links[0].app, "zsh");
    }

    #[test]
    fn roundtrips_through_disk() {
        let home = TempHome::new("roundtrip");
        let layout = home.layout();
        let mut r = LinkRecords::default();
        r.upsert("ghostty", TARGET, STORE);
        r.save(&layout).unwrap();

        let back = LinkRecords::load(&layout).unwrap();
        assert_eq!(back.links.len(), 1);
        assert_eq!(back.links[0].store, STORE);
    }

    #[test]
    fn load_returns_default_when_absent() {
        let home = TempHome::new("absent");
        let r = LinkRecords::load(&home.layout()).unwrap();
        assert!(r.links.is_empty());
    }
}
