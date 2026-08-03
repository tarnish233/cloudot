//! 凭据检测。
//!
//! 目的不是做完整的密钥管理（那是后面 age 加密那一步的事），而是**在把文件放进
//! git 仓库之前拦一道**。用户可以往 `~/.cloudot/adopters/` 放自定义定义，所以
//! 今天就可能纳管到 `~/.config/gh/hosts.yml` 这类含 OAuth token 的文件。
//!
//! 刻意不引入 regex 依赖：这里的模式都很固定，手写匹配更可预测，也不会因为
//! 正则回溯在大文件上卡住。
//!
//! **输出里绝不包含命中的值本身** —— 只报路径、行号和命中原因，
//! 否则扫描结果自己就成了泄漏渠道。

use serde::Serialize;
use std::path::Path;

/// 超过这个大小就不扫了：配置文件不会这么大，多半是数据文件。
const MAX_SCAN_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// `~/` 形式的路径
    pub path: String,
    /// 1 起算的行号；按路径规则命中时为 None
    pub line: Option<usize>,
    /// 命中原因，不含命中的值
    pub reason: String,
}

/// 路径本身就说明内容敏感的规则（子串匹配 `~/` 形式路径）。
///
/// 刻意**不**整体拦 `~/.ssh/` —— `config` 和 `known_hosts` 是很合理的同步对象，
/// 而私钥会被下面的后缀规则和 PEM 内容检测抓到，不需要连坐。
/// `.gnupg/` 反而要整体拦：私钥环是二进制，内容扫描会跳过它。
const SECRET_PATH_RULES: &[(&str, &str)] = &[
    ("/.gnupg/", "GnuPG 私钥环"),
    ("/.config/gh/hosts.yml", "GitHub CLI 的 OAuth token"),
    ("/.aws/credentials", "AWS 长期凭据"),
    ("/.netrc", "明文登录凭据"),
    ("/.npmrc", "npm registry token"),
    ("/.pypirc", "PyPI 上传 token"),
    ("/.docker/config.json", "Docker registry 凭据"),
    ("/.kube/config", "Kubernetes 集群凭据"),
    ("/.claude.json", "Claude 凭据与会话数据"),
    ("/.terraformrc", "Terraform Cloud token"),
];

/// 一看后缀就知道是密钥材料。
const SECRET_SUFFIXES: &[(&str, &str)] = &[
    (".pem", "PEM 密钥/证书"),
    (".p12", "PKCS#12 密钥库"),
    (".pfx", "PKCS#12 密钥库"),
    (".keystore", "Java keystore"),
    ("_rsa", "SSH 私钥"),
    ("_dsa", "SSH 私钥"),
    ("_ecdsa", "SSH 私钥"),
    ("_ed25519", "SSH 私钥"),
];

/// 已知服务的 token 前缀：(前缀, 前缀后至少还要有多少个 token 字符, 说明)
const TOKEN_PREFIXES: &[(&str, usize, &str)] = &[
    ("ghp_", 20, "GitHub personal access token"),
    ("gho_", 20, "GitHub OAuth token"),
    ("ghu_", 20, "GitHub user-to-server token"),
    ("ghs_", 20, "GitHub server-to-server token"),
    ("ghr_", 20, "GitHub refresh token"),
    ("github_pat_", 20, "GitHub fine-grained token"),
    ("sk-ant-", 20, "Anthropic API key"),
    ("sk-proj-", 20, "OpenAI project key"),
    ("xoxb-", 10, "Slack bot token"),
    ("xoxp-", 10, "Slack user token"),
    ("xapp-", 10, "Slack app token"),
    ("glpat-", 16, "GitLab personal access token"),
    ("AKIA", 16, "AWS access key ID"),
    ("ASIA", 16, "AWS 临时 access key ID"),
    ("AIza", 30, "Google API key"),
];

/// 键名里出现这些词，就要看它的值像不像凭据。
const SUSPICIOUS_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "accesskey",
    "access_key",
    "privatekey",
    "private_key",
    "credential",
    "client_secret",
    "auth_token",
];

/// 明显是占位符/环境变量引用，不算命中。
fn is_placeholder(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    if value.starts_with('$') || value.starts_with("${") || value.contains("%s") {
        return true; // 环境变量或格式化占位
    }
    if value.starts_with('<') || value.starts_with('{') {
        return true; // <your-token-here> / {{ .Token }}
    }
    let lower = value.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "null" | "none" | "nil" | "true" | "false" | "changeme" | "todo" | "example"
    ) || lower.chars().all(|c| c == 'x' || c == '*' || c == '.')
}

/// token 里可能出现的字符。
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+' | '=' | '~')
}

/// 只按路径判断，不读文件内容。
pub fn scan_path(display_path: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (needle, reason) in SECRET_PATH_RULES {
        if display_path.contains(needle) {
            out.push(Finding {
                path: display_path.to_owned(),
                line: None,
                reason: (*reason).to_owned(),
            });
        }
    }
    // 公钥可以放心同步，别拦
    if !display_path.ends_with(".pub") {
        for (suffix, reason) in SECRET_SUFFIXES {
            if display_path.ends_with(suffix) {
                out.push(Finding {
                    path: display_path.to_owned(),
                    line: None,
                    reason: (*reason).to_owned(),
                });
            }
        }
    }
    out
}

/// 扫一个文件：路径规则 + 内容模式。
///
/// 读不到、太大、或者是二进制的文件都直接跳过（返回路径规则的结果），
/// 因为扫不动不等于有问题。
pub fn scan_file(file: &Path, display_path: &str) -> Vec<Finding> {
    let mut out = scan_path(display_path);

    let too_big = std::fs::metadata(file)
        .map(|m| m.len() > MAX_SCAN_BYTES)
        .unwrap_or(true);
    if too_big {
        return out;
    }
    let Ok(bytes) = std::fs::read(file) else {
        return out;
    };
    if bytes.contains(&0) {
        return out; // 二进制
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return out;
    };

    out.extend(scan_text(&text, display_path));
    out
}

/// 扫文本内容。独立出来便于测试。
pub fn scan_text(text: &str, display_path: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let lineno = idx + 1;
        let mut reasons: Vec<String> = Vec::new();

        // PEM 私钥块
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY") {
            reasons.push("PEM 私钥块".to_owned());
        }

        // 已知服务的 token 前缀
        for (prefix, min_trailing, label) in TOKEN_PREFIXES {
            if let Some(pos) = line.find(prefix) {
                let trailing = line[pos + prefix.len()..]
                    .chars()
                    .take_while(|c| is_token_char(*c))
                    .count();
                if trailing >= *min_trailing {
                    reasons.push((*label).to_owned());
                }
            }
        }

        // `key = value` / `key: value` 形式的可疑赋值
        if let Some(reason) = suspicious_assignment(line) {
            reasons.push(reason);
        }

        for reason in reasons {
            out.push(Finding {
                path: display_path.to_owned(),
                line: Some(lineno),
                reason,
            });
        }
    }
    out
}

fn suspicious_assignment(line: &str) -> Option<String> {
    let sep = line.find('=').or_else(|| line.find(':'))?;
    let (raw_key, raw_value) = line.split_at(sep);
    let key = raw_key
        .trim()
        .trim_start_matches(['#', ';', '-', ' '])
        .trim()
        .to_ascii_lowercase();
    if key.is_empty() || key.len() > 64 {
        return None;
    }
    let matched = SUSPICIOUS_KEYS
        .iter()
        .find(|needle| key.contains(*needle))?;

    let value = raw_value[1..].trim().trim_matches(['"', '\'', ',']).trim();
    if is_placeholder(value) || value.len() < 16 {
        return None;
    }
    if !value.chars().all(is_token_char) {
        return None; // 有空格之类，更像散文而不是凭据
    }
    // 只报键名，绝不带上值
    Some(format!(
        "键名含 “{matched}”，值长度 {} 且形似凭据",
        value.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_known_secret_paths() {
        assert!(!scan_path("~/.ssh/id_ed25519").is_empty());
        assert!(!scan_path("~/.config/gh/hosts.yml").is_empty());
        assert!(!scan_path("~/.aws/credentials").is_empty());
        assert!(!scan_path("~/.gnupg/private-keys-v1.d/x.key").is_empty());
    }

    #[test]
    fn allows_public_keys_and_ordinary_configs() {
        assert!(scan_path("~/.ssh/id_ed25519.pub").is_empty());
        assert!(scan_path("~/.config/ghostty/config").is_empty());
        assert!(scan_path("~/.zshrc").is_empty());
    }

    /// `~/.ssh/config` 和 `known_hosts` 本身不含私钥，是合理的同步对象 ——
    /// 不能因为在 .ssh 目录下就连坐。私钥靠后缀规则与 PEM 内容检测拦。
    #[test]
    fn does_not_blanket_block_the_ssh_directory() {
        assert!(scan_path("~/.ssh/config").is_empty());
        assert!(scan_path("~/.ssh/known_hosts").is_empty());
    }

    #[test]
    fn flags_known_token_prefixes() {
        let hits = scan_text("oauth_token: gho_16C7e42F292c6912E7710c838347Ae178B4a", "p");
        assert!(!hits.is_empty());
        let hits = scan_text("aws_access_key_id = AKIAIOSFODNN7EXAMPLE", "p");
        assert!(!hits.is_empty());
    }

    #[test]
    fn flags_pem_private_key_blocks() {
        assert!(!scan_text("-----BEGIN OPENSSH PRIVATE KEY-----", "p").is_empty());
        // 证书不是私钥，别拦
        assert!(scan_text("-----BEGIN CERTIFICATE-----", "p").is_empty());
    }

    #[test]
    fn flags_suspicious_assignments() {
        assert!(!scan_text("api_key = 8f14e45fceea167a5a36dedd4bea2543", "p").is_empty());
        assert!(!scan_text("password: hunter2hunter2hunter2", "p").is_empty());
    }

    #[test]
    fn ignores_placeholders_and_env_refs() {
        assert!(scan_text("api_key = ${GITHUB_TOKEN}", "p").is_empty());
        assert!(scan_text("api_key = $MY_TOKEN", "p").is_empty());
        assert!(scan_text("api_key = <your-key-here>", "p").is_empty());
        assert!(scan_text("password =", "p").is_empty());
        assert!(scan_text("token = xxxxxxxxxxxxxxxxxx", "p").is_empty());
    }

    #[test]
    fn ignores_short_values_and_prose() {
        assert!(scan_text("password = 123", "p").is_empty());
        assert!(
            scan_text("secret = 这是一段说明文字 不是凭据", "p").is_empty(),
            "带空格的值更像散文"
        );
    }

    /// ghostty 配置里全是颜色、字体、键位，不能有一条误报。
    #[test]
    fn does_not_false_positive_on_ghostty_style_config() {
        let sample = "\
font-family = Maple Mono NF CN
font-size = 13
background = 1e1e2e
foreground = cdd6f4
theme = catppuccin-mocha
keybind = cmd+shift+t=new_tab
window-padding-x = 8
shell-integration = fish
cursor-style = block
palette = 0=#45475a
";
        let hits = scan_text(sample, "~/.config/ghostty/config");
        assert!(hits.is_empty(), "误报：{hits:?}");
    }

    #[test]
    fn findings_never_contain_the_secret_value() {
        let secret = "gho_16C7e42F292c6912E7710c838347Ae178B4a";
        for finding in scan_text(&format!("oauth_token: {secret}"), "p") {
            assert!(
                !finding.reason.contains(secret),
                "扫描结果里不能出现命中的值"
            );
        }
    }
}
