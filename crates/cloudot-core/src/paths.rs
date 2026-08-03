use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

/// `~/.cloudot` 下的目录布局。
///
/// 两个环境变量用于测试隔离，正常使用不需要设置：
/// - `CLOUDOT_HOME` 覆盖 `$HOME`（连被纳管的 `~/.config/...` 一起改指向）
/// - `CLOUDOT_ROOT` 只覆盖 `~/.cloudot` 本身
#[derive(Debug, Clone)]
pub struct Layout {
    home: PathBuf,
    root: PathBuf,
}

impl Layout {
    /// 显式指定家目录。测试和将来的 GUI 用；正常 CLI 走 [`Layout::discover`]。
    pub fn with_home(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        let root = home.join(".cloudot");
        Self { home, root }
    }

    pub fn discover() -> Result<Self> {
        let home = std::env::var_os("CLOUDOT_HOME")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| anyhow!("无法确定家目录：$HOME 未设置"))?;
        let root = std::env::var_os("CLOUDOT_ROOT")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| home.join(".cloudot"));
        Ok(Self { home, root })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    /// 本机建过的软链记录，不进 git。
    pub fn links_file(&self) -> PathBuf {
        self.root.join("links.toml")
    }
    /// 进程间互斥用的锁文件。
    pub fn lock_file(&self) -> PathBuf {
        self.root.join("lock")
    }
    /// 用户自定义 adopter 目录，同 id 覆盖内置定义。
    pub fn adopters_dir(&self) -> PathBuf {
        self.root.join("adopters")
    }
    /// git 工作树，同时是所有软链的目标。
    pub fn store(&self) -> PathBuf {
        self.root.join("store")
    }
    pub fn manifest_file(&self) -> PathBuf {
        self.store().join("manifest.toml")
    }
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }
    /// store 内相对路径 → 绝对路径。
    pub fn store_path(&self, rel: &str) -> PathBuf {
        self.store().join(rel)
    }

    /// `~/.config/x` → `/Users/u/.config/x`
    pub fn expand(&self, p: &str) -> PathBuf {
        if let Some(rest) = p.strip_prefix("~/") {
            self.home.join(rest)
        } else if p == "~" {
            self.home.clone()
        } else {
            PathBuf::from(p)
        }
    }

    /// `/Users/u/.config/x` → `~/.config/x`，用于写进 manifest 以跨机器移植。
    pub fn contract(&self, p: &Path) -> String {
        match p.strip_prefix(&self.home) {
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => p.display().to_string(),
        }
    }

    /// 目标路径 → store 内相对路径，如
    /// `~/.config/ghostty/config` → `files/.config/ghostty/config`
    ///
    /// 直接镜像家目录结构：不需要处理重名，且 `git log files/.config/ghostty/config`
    /// 天然可读。
    pub fn store_rel_for(&self, target: &Path) -> Result<String> {
        let unsupported = |msg: String| crate::tagged(crate::ErrorKind::Unsupported, msg);
        let rel = target.strip_prefix(&self.home).map_err(|_| {
            unsupported(format!(
                "暂只支持家目录下的路径，{} 在家目录之外",
                target.display()
            ))
        })?;
        if rel.as_os_str().is_empty() {
            return Err(unsupported("不能纳管家目录本身".to_owned()));
        }
        if rel.components().any(|c| c.as_os_str() == "..") {
            return Err(unsupported(format!(
                "路径不能包含 ..：{}",
                target.display()
            )));
        }
        // 不用 display()：非 UTF-8 路径会被替换字符悄悄改写成另一个路径
        let rel = rel.to_str().ok_or_else(|| {
            unsupported(format!(
                "路径不是合法 UTF-8，暂不支持：{}",
                target.display()
            ))
        })?;
        Ok(format!("files/{rel}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::with_home("/home/u")
    }

    #[test]
    fn expand_resolves_tilde() {
        let l = layout();
        assert_eq!(l.expand("~/.config/x"), PathBuf::from("/home/u/.config/x"));
        assert_eq!(l.expand("~"), PathBuf::from("/home/u"));
        assert_eq!(l.expand("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    fn contract_inverts_expand() {
        let l = layout();
        for p in ["~/.config/ghostty/config", "~/.zshrc"] {
            assert_eq!(l.contract(&l.expand(p)), p);
        }
    }

    #[test]
    fn contract_leaves_outside_paths_absolute() {
        assert_eq!(layout().contract(Path::new("/etc/hosts")), "/etc/hosts");
    }

    #[test]
    fn store_rel_mirrors_home_layout() {
        assert_eq!(
            layout()
                .store_rel_for(Path::new("/home/u/.config/ghostty/config"))
                .unwrap(),
            "files/.config/ghostty/config"
        );
    }

    #[test]
    fn store_rel_rejects_paths_outside_home() {
        assert!(layout().store_rel_for(Path::new("/etc/hosts")).is_err());
    }

    #[test]
    fn store_rel_rejects_home_itself() {
        assert!(layout().store_rel_for(Path::new("/home/u")).is_err());
    }

    #[test]
    fn store_rel_rejects_dotdot_escape() {
        assert!(
            layout()
                .store_rel_for(Path::new("/home/u/../../etc/passwd"))
                .is_err()
        );
    }
}
