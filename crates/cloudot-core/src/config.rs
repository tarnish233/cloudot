use crate::Layout;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

pub const CONFIG_VERSION: u32 = 1;

/// `~/.cloudot/config.toml` —— 本机配置，**不进 git**，因此可以放机器特有的东西。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    /// 设备标识，用于在 manifest 和 commit message 里标注是谁做的改动。
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

impl Config {
    pub fn new(device: String, remote: Option<String>) -> Self {
        Self {
            version: CONFIG_VERSION,
            device,
            remote,
        }
    }

    pub fn exists(layout: &Layout) -> bool {
        layout.config_file().exists()
    }

    pub fn load(layout: &Layout) -> Result<Self> {
        let path = layout.config_file();
        let raw = fs::read_to_string(&path).map_err(|_| {
            crate::tagged(
                crate::ErrorKind::NotInitialized,
                format!("读不到 {}，先跑 `cloudot init`", path.display()),
            )
        })?;
        toml::from_str(&raw).with_context(|| format!("{} 解析失败", path.display()))
    }

    pub fn save(&self, layout: &Layout) -> Result<()> {
        fs::create_dir_all(layout.root())?;
        let body = format!(
            "# cloudot 本机配置（不会被同步到 git）\n{}",
            toml::to_string_pretty(self)?
        );
        fs::write(layout.config_file(), body)
            .with_context(|| format!("写入 {} 失败", layout.config_file().display()))
    }
}

/// 由 `hostname -s` 推导一个 slug 化的设备名。
pub fn default_device_name() -> String {
    let raw = std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "mac".to_owned());
    slugify(&raw)
}

fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "mac".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// 备份目录用的时间戳。
pub fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_normalizes_hostnames() {
        assert_eq!(slugify("coma-White55deMac-mini"), "coma-white55demac-mini");
        assert_eq!(slugify("MacBook Pro"), "macbook-pro");
        assert_eq!(slugify("coma_White55的Mac mini"), "coma-white55-mac-mini");
    }

    #[test]
    fn slugify_never_returns_empty_or_edge_dashes() {
        assert_eq!(slugify(""), "mac");
        assert_eq!(slugify("---"), "mac");
        assert_eq!(slugify("的"), "mac");
        assert_eq!(slugify("-host-"), "host");
    }

    #[test]
    fn config_roundtrips_through_disk() {
        let home = crate::testutil::TempHome::new("config");
        let layout = home.layout();
        Config::new("dev-box".into(), Some("git@example.com:u/d.git".into()))
            .save(&layout)
            .unwrap();

        let back = Config::load(&layout).unwrap();
        assert_eq!(back.device, "dev-box");
        assert_eq!(back.remote.as_deref(), Some("git@example.com:u/d.git"));
        assert_eq!(back.version, CONFIG_VERSION);
    }

    #[test]
    fn config_without_remote_roundtrips() {
        let home = crate::testutil::TempHome::new("config-noremote");
        let layout = home.layout();
        Config::new("dev-box".into(), None).save(&layout).unwrap();
        assert!(Config::load(&layout).unwrap().remote.is_none());
    }
}
