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
}
