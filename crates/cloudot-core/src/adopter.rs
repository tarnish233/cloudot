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
    /// 单个文件，或配合 `include` 时的一个目录。
    pub path: String,
    #[serde(default)]
    pub strategy: Strategy,
    /// 目录展开规则：文件名匹配任一模式才纳管（如 `["*.fish"]`）。
    ///
    /// 为空时 `path` 按单文件处理 —— 这是绝大多数 adopter 的形态。
    #[serde(default)]
    pub include: Vec<String>,
    /// 从 `include` 的结果里再剔掉的模式。
    ///
    /// 挡两类东西：工具自己生成的（`cargo.fish` 那种装 cargo 时写进去的环境注入，
    /// 内容含本机路径）、以及编辑器和用户留下的副本（`*.bak`）。
    /// **只在这里显式列**，core 里不藏一份隐式默认 —— 排除规则要能被审。
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl AdopterPath {
    /// 这条定义是不是「目录 + 规则」形态。
    pub fn is_glob(&self) -> bool {
        !self.include.is_empty()
    }
}

/// 极简 glob：只认 `*`（任意长度，含空）和 `?`（恰好一个字符）。
///
/// 刻意不引入 glob crate，也刻意不支持 `**` —— 我们只拿它匹配**单层目录里的文件名**，
/// 递归展开会把 `completions/`、`automatic_backups/` 那类运行时目录一起带上，
/// 那正是要避开的（见 fish 与 karabiner 的 adopter 注释）。
///
/// 匹配的是文件名，不含路径分隔符，所以不必处理 `/` 的特殊语义。
pub fn glob_match(pattern: &str, name: &str) -> bool {
    // 回溯法。`star` 记住上一个 `*` 的位置，失配时退回去让它多吃一个字符。
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_ni = 0usize;

    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_ni = ni;
            pi += 1;
        } else if let Some(s) = star {
            // 退回最近的 `*`，让它多吞一个字符再试
            pi = s + 1;
            star_ni += 1;
            ni = star_ni;
        } else {
            return false;
        }
    }
    // 结尾允许剩下若干个 `*`
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

impl Adopter {
    pub fn detected(&self, layout: &Layout) -> bool {
        self.detect.iter().any(|p| {
            let expanded = layout.expand(p);
            fs::symlink_metadata(&expanded).is_ok()
        })
    }

    /// 把定义里的路径展开成**具体文件**列表（`~/` 形式）。
    ///
    /// 单文件条目原样返回（哪怕文件此刻不存在 —— `add` 要靠后续步骤给出准确的
    /// 错误，而不是在这里静默跳过）。目录条目按 `include`/`exclude` 扫一层，
    /// 结果按字典序排序，保证 manifest 里的顺序稳定、diff 干净。
    ///
    /// 展开发生在 `add` 时，manifest 里存的仍是确定的文件清单 —— `apply` /
    /// `status` / `doctor` 因此完全不需要知道 glob 的存在。代价是本机新增一个
    /// `conf.d/foo.fish` 之后要重跑 `add` 才会纳管它。
    pub fn expand_paths(&self, layout: &Layout) -> Vec<(String, Strategy)> {
        let mut out = Vec::new();
        for p in &self.paths {
            if !p.is_glob() {
                out.push((p.path.clone(), p.strategy));
                continue;
            }
            let dir = layout.expand(&p.path);
            let Ok(entries) = fs::read_dir(&dir) else {
                // 目录不存在（本机没装、或还没生成）—— 不是错误，就是没有可纳管的
                continue;
            };
            let mut matched = Vec::new();
            for entry in entries.flatten() {
                // 只收文件与软链，不递归进子目录
                if entry.path().is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let included = p.include.iter().any(|pat| glob_match(pat, &name));
                let excluded = p.exclude.iter().any(|pat| glob_match(pat, &name));
                if included && !excluded {
                    matched.push(layout.contract(&entry.path()));
                }
            }
            matched.sort();
            out.extend(matched.into_iter().map(|path| (path, p.strategy)));
        }
        out
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

    // ── glob 匹配器 ──────────────────────────────────────────────
    //
    // 自己写的，所以边界要钉死。回溯实现最容易在「多个 *」和「* 吃空」上出错。

    #[test]
    fn glob_matches_literals_and_stars() {
        assert!(glob_match("*.fish", "aliases.fish"));
        assert!(glob_match("*.fish", ".fish"), "* 该能匹配空串");
        assert!(!glob_match("*.fish", "aliases.fish.bak"));
        assert!(glob_match("*.bak", "config.fish.bak"));

        assert!(glob_match("config.fish", "config.fish"));
        assert!(!glob_match("config.fish", "config.fisher"));
        assert!(!glob_match("config.fish", "myconfig.fish"));

        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""), "* 匹配空文件名");
        assert!(glob_match("**", "x"), "连续的 * 不该改变语义");
    }

    #[test]
    fn glob_handles_question_mark() {
        assert!(glob_match("?.fish", "a.fish"));
        assert!(!glob_match("?.fish", "ab.fish"));
        assert!(!glob_match("?.fish", ".fish"), "? 必须吃掉恰好一个字符");
    }

    /// 多个 `*` 要能正确回溯 —— 这是朴素实现最常见的 bug。
    #[test]
    fn glob_backtracks_across_multiple_stars() {
        assert!(glob_match("*env*", "uv.env.fish"));
        assert!(glob_match("*.*.fish", "uv.env.fish"));
        assert!(!glob_match("*.*.fish", "aliases.fish"));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "axxbyy"));
    }

    /// 非 ASCII 文件名不能因为按字节切分而错判。
    #[test]
    fn glob_works_on_non_ascii_names() {
        assert!(glob_match("*.fish", "别名.fish"));
        assert!(glob_match("别名*", "别名.fish"));
        assert!(!glob_match("?.fish", "别名.fish"), "两个字符不该匹配单个 ?");
    }

    // ── expand_paths ────────────────────────────────────────────

    /// 目录 + include/exclude 该只收匹配的文件，且顺序稳定。
    #[test]
    fn expand_paths_applies_include_and_exclude() {
        let home = TempHome::new("adopter-glob");
        let layout = home.layout();
        let dir = layout.expand("~/.config/fish/conf.d");
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "aliases.fish",
            "theme.fish",
            "cargo.fish",     // 工具生成，该排除
            "theme.fish.bak", // 备份，该排除
            "notes.txt",      // 不匹配 include
        ] {
            fs::write(dir.join(name), "x").unwrap();
        }
        // 子目录不该被递归进去
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("nested/deep.fish"), "x").unwrap();

        let ad: Adopter = toml::from_str(
            "id = \"t\"\nname = \"T\"\ndetect = [\"~/x\"]\n\
             [[paths]]\npath = \"~/.config/fish/conf.d\"\n\
             include = [\"*.fish\"]\nexclude = [\"cargo.fish\", \"*.bak\"]\n",
        )
        .expect("解析");

        let paths: Vec<String> = ad
            .expand_paths(&layout)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(
            paths,
            vec![
                "~/.config/fish/conf.d/aliases.fish",
                "~/.config/fish/conf.d/theme.fish",
            ],
            "只该收匹配 include 又不在 exclude 里的顶层文件"
        );
    }

    /// 单文件条目原样返回，哪怕文件不存在 —— 让 add 去给出准确的错误。
    #[test]
    fn expand_paths_passes_single_files_through() {
        let home = TempHome::new("adopter-single");
        let layout = home.layout();
        let ad: Adopter = toml::from_str(
            "id = \"t\"\nname = \"T\"\ndetect = [\"~/x\"]\n\
             [[paths]]\npath = \"~/.config/t/conf\"\n",
        )
        .expect("解析");

        let paths: Vec<String> = ad
            .expand_paths(&layout)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(paths, vec!["~/.config/t/conf"]);
    }

    /// 目录不存在不是错误：本机没装那个工具，就是没有可纳管的文件。
    #[test]
    fn expand_paths_skips_a_missing_directory() {
        let home = TempHome::new("adopter-nodir");
        let layout = home.layout();
        let ad: Adopter = toml::from_str(
            "id = \"t\"\nname = \"T\"\ndetect = [\"~/x\"]\n\
             [[paths]]\npath = \"~/.config/nope\"\ninclude = [\"*\"]\n",
        )
        .expect("解析");
        assert!(ad.expand_paths(&layout).is_empty());
    }

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
    ///
    /// 目录型（glob）条目验的是目录本身可纳管；里面的文件都在它之下，自然也可以。
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

    /// glob 条目的 include 不能为空以外的形态出错。
    ///
    /// 具体挡两件事：写了 `exclude` 却忘了 `include`（那条 exclude 永远不生效，
    /// 因为没有 include 就按单文件处理，静默失效最难查）；以及 include 里出现
    /// 带路径分隔符的模式（我们只匹配单层文件名，`a/b` 永远匹配不上）。
    #[test]
    fn builtin_glob_rules_are_well_formed() {
        for (key, raw) in BUILTIN {
            let ad: Adopter = toml::from_str(raw).expect("解析");
            for p in &ad.paths {
                assert!(
                    p.exclude.is_empty() || p.is_glob(),
                    "{key} 的 {} 写了 exclude 但没有 include —— 那条 exclude 不会生效",
                    p.path
                );
                for pat in p.include.iter().chain(p.exclude.iter()) {
                    assert!(
                        !pat.contains('/'),
                        "{key} 的模式 {pat} 含路径分隔符，但匹配只针对单层文件名"
                    );
                    assert!(
                        !pat.contains("**"),
                        "{key} 的模式 {pat} 用了 **，我们刻意不支持递归展开"
                    );
                }
            }
        }
    }

    /// glob 定义在**真实布局**下必须真的匹配到东西。
    ///
    /// 光验模式合法不够：`include = ["*.fsh"]` 写错一个字母照样合法，但一个文件
    /// 都收不到 —— 表现是 `add` 报「没有匹配到任何文件」，而定义看着完全正常。
    #[test]
    fn fish_glob_picks_up_handwritten_files_and_skips_generated_ones() {
        let home = TempHome::new("adopter-fish-real");
        let layout = home.layout();
        let conf_d = layout.expand("~/.config/fish/conf.d");
        let functions = layout.expand("~/.config/fish/functions");
        fs::create_dir_all(&conf_d).unwrap();
        fs::create_dir_all(&functions).unwrap();

        // 照本机实际布局摆一遍
        for name in [
            "aliases.fish",               // 手写别名，该收
            "fish_frozen_theme.fish",     // 手写主题，该收
            "cargo.fish",                 // rustup 生成，该排除
            "homebrew.fish",              // brew shellenv，该排除
            "uv.env.fish",                // uv 生成，该排除
            "fish_frozen_theme.fish.bak", // 备份，该排除
        ] {
            fs::write(conf_d.join(name), "x").unwrap();
        }
        for name in ["fish_prompt.fish", "sshreset.fish"] {
            fs::write(functions.join(name), "x").unwrap();
        }
        // 这两个绝不该出现在结果里
        fs::write(layout.expand("~/.config/fish/fish_variables"), "x").unwrap();
        let completions = layout.expand("~/.config/fish/completions");
        fs::create_dir_all(&completions).unwrap();
        fs::write(completions.join("copilot.fish"), "x").unwrap();
        fs::write(layout.expand("~/.config/fish/config.fish"), "x").unwrap();

        let raw = BUILTIN
            .iter()
            .find(|(k, _)| *k == "fish")
            .expect("fish 该在 BUILTIN 里")
            .1;
        let ad: Adopter = toml::from_str(raw).expect("解析");
        let got: Vec<String> = ad
            .expand_paths(&layout)
            .into_iter()
            .map(|(p, _)| p)
            .collect();

        assert_eq!(
            got,
            vec![
                "~/.config/fish/config.fish",
                "~/.config/fish/conf.d/aliases.fish",
                "~/.config/fish/conf.d/fish_frozen_theme.fish",
                "~/.config/fish/functions/fish_prompt.fish",
                "~/.config/fish/functions/sshreset.fish",
            ],
            "手写文件该全收，工具生成的与备份该全排除"
        );
        // 冗余但值得写死：这两条是 fish adopter 最重要的两个「不」
        assert!(!got.iter().any(|p| p.contains("fish_variables")));
        assert!(!got.iter().any(|p| p.contains("completions/")));
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
