use crate::Layout;
use crate::manifest::Strategy;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;

/// 内置 adopter 定义。加一个新应用 = 往 `adopters/` 放一个 TOML 并在这里登记一行，
/// 不需要改任何逻辑代码。
const BUILTIN: &[(&str, &str)] = &[
    ("fish", include_str!("../../../adopters/fish.toml")),
    ("ghostty", include_str!("../../../adopters/ghostty.toml")),
    ("gitpic", include_str!("../../../adopters/gitpic.toml")),
    (
        "karabiner",
        include_str!("../../../adopters/karabiner.toml"),
    ),
];

/// 一个应用的适配定义：它的配置在哪、怎么落地、怎么检测装没装。
#[derive(Debug, Clone, Deserialize)]
pub struct Adopter {
    pub id: String,
    pub name: String,
    /// 任一路径存在即认为本机装了这个应用。
    #[serde(default)]
    pub detect: Vec<String>,
    pub paths: Vec<AdopterPath>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdopterPath {
    pub path: String,
    #[serde(default)]
    pub strategy: Strategy,
}

impl Adopter {
    pub fn detected(&self, layout: &Layout) -> bool {
        self.detect.iter().any(|p| {
            let expanded = layout.expand(p);
            fs::symlink_metadata(&expanded).is_ok()
        })
    }
}

/// 内置定义 + `~/.cloudot/adopters/*.toml`，同 id 时用户定义覆盖内置。
pub fn load_all(layout: &Layout) -> Result<Vec<Adopter>> {
    let mut out: Vec<Adopter> = Vec::new();
    for (id, raw) in BUILTIN {
        let ad: Adopter = toml::from_str(raw)
            .with_context(|| format!("内置 adopter {id} 解析失败（这是 cloudot 的 bug）"))?;
        out.push(ad);
    }

    let dir = layout.adopters_dir();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("读取 {} 失败", path.display()))?;
            let ad: Adopter =
                toml::from_str(&raw).with_context(|| format!("{} 解析失败", path.display()))?;
            match out.iter_mut().find(|existing| existing.id == ad.id) {
                Some(slot) => *slot = ad,
                None => out.push(ad),
            }
        }
    }

    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

pub fn get(layout: &Layout, id: &str) -> Result<Adopter> {
    let all = load_all(layout)?;
    let known = all
        .iter()
        .map(|a| a.id.clone())
        .collect::<Vec<_>>()
        .join("、");
    match all.into_iter().find(|a| a.id == id) {
        Some(ad) => Ok(ad),
        None => Err(crate::tagged(
            crate::ErrorKind::UnknownApp,
            format!("没有名为 {id} 的应用定义。目前支持：{known}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempHome;

    /// 每个内置定义都必须能解析，且 id 与登记的 key 一致。
    ///
    /// `load_all` 里解析失败只会 `with_context` 报「这是 cloudot 的 bug」——
    /// 那时候用户已经装上了。TOML 写错一个字段名在编译期完全看不出来，所以在这里挡住。
    #[test]
    fn every_builtin_parses_and_matches_its_key() {
        for (key, raw) in BUILTIN {
            let ad: Adopter =
                toml::from_str(raw).unwrap_or_else(|e| panic!("内置 adopter {key} 解析失败：{e}"));
            assert_eq!(&ad.id, key, "{key} 的 id 字段和登记的 key 不一致");
            assert!(!ad.name.is_empty(), "{key} 没有 name");
            assert!(
                !ad.detect.is_empty(),
                "{key} 没有 detect，界面会永远显示未安装"
            );
            assert!(!ad.paths.is_empty(), "{key} 没有 paths，纳管了也什么都不做");
        }
    }

    /// 所有内置路径都要能算出 store 内位置。
    ///
    /// `store_rel_for` 会拒绝家目录之外的路径、`..`、以及非 UTF-8 —— 这些在
    /// TOML 里都写得出来，但要到用户真的 `add` 时才会炸。
    #[test]
    fn every_builtin_path_is_adoptable() {
        let home = TempHome::new("adopter-paths");
        let layout = home.layout();
        for (key, raw) in BUILTIN {
            let ad: Adopter = toml::from_str(raw).expect("解析");
            for p in &ad.paths {
                let expanded = layout.expand(&p.path);
                layout
                    .store_rel_for(&expanded)
                    .unwrap_or_else(|e| panic!("{key} 的路径 {} 不可纳管：{e}", p.path));
            }
        }
    }

    /// 定义里不能出现明显该由运行时维护、或本身就是凭据的路径。
    ///
    /// 前者（fish_variables、automatic_backups）同步过去会让两台机器互相覆盖运行时
    /// 状态；后者会被凭据门禁拦下，等于把定义写成了一个用不了的东西。
    #[test]
    fn builtins_avoid_runtime_and_secret_paths() {
        const FORBIDDEN: &[&str] = &[
            "fish_variables",
            "automatic_backups",
            "id_rsa",
            "id_ed25519",
            ".netrc",
        ];
        for (key, raw) in BUILTIN {
            let ad: Adopter = toml::from_str(raw).expect("解析");
            for p in &ad.paths {
                for bad in FORBIDDEN {
                    assert!(
                        !p.path.contains(bad),
                        "{key} 纳管了 {}，其中含不该同步的 {bad}",
                        p.path
                    );
                }
            }
        }
    }

    /// `adopters/` 下的每个 TOML 都必须在 `BUILTIN` 里登记。
    ///
    /// 漏登记时症状很误导：TOML 在仓库里、README 里写了支持，但 CLI 和 GUI 都看不见
    /// 那个应用 —— 看起来像前端 bug。`include_str!` 是编译期展开的，忘了加一行
    /// 编译器不会有任何提示。
    #[test]
    fn every_adopter_file_is_registered() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adopters");
        let mut on_disk: Vec<String> = fs::read_dir(&dir)
            .expect("读 adopters/ 目录")
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("toml"))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .and_then(|x| x.to_str())
                    .map(str::to_owned)
            })
            .collect();
        on_disk.sort();

        let mut registered: Vec<String> = BUILTIN.iter().map(|(k, _)| (*k).to_owned()).collect();
        registered.sort();

        assert_eq!(
            on_disk, registered,
            "adopters/ 下的文件和 BUILTIN 登记不一致 —— 新增 TOML 后要在 adopter.rs 补一行"
        );
    }

    /// 用户定义能覆盖内置的同 id 定义。
    #[test]
    fn user_definition_overrides_builtin() {
        let home = TempHome::new("adopter-override");
        let layout = home.layout();
        fs::create_dir_all(layout.adopters_dir()).expect("建目录");
        fs::write(
            layout.adopters_dir().join("ghostty.toml"),
            "id = \"ghostty\"\nname = \"我的 Ghostty\"\ndetect = [\"~/x\"]\n\
             [[paths]]\npath = \"~/.config/ghostty/config\"\n",
        )
        .expect("写文件");

        let all = load_all(&layout).expect("加载");
        let ghostty = all
            .iter()
            .find(|a| a.id == "ghostty")
            .expect("找到 ghostty");
        assert_eq!(ghostty.name, "我的 Ghostty");
        // 覆盖不应该把别的内置定义弄丢
        assert!(all.iter().any(|a| a.id == "fish"));
    }
}
