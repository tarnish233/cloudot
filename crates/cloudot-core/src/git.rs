use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// 对 `git` 命令行的薄封装。
///
/// 为什么不用 git2/libgit2：用户的 remote 是 SSH，`git` CLI 直接继承了已经配好的
/// ssh-agent、`~/.ssh/config`、gh 凭据；换成 libgit2 就得自己接管认证。
/// 这一层很薄，将来真需要换实现，接口不用动。
pub struct Git {
    dir: PathBuf,
}

/// `git pull --rebase` 之后是否有实际变化。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullOutcome {
    /// 没有配 remote 或还没有 upstream，跳过。
    Skipped,
    UpToDate,
    Updated,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn is_repo(&self) -> bool {
        self.dir.join(".git").exists()
    }

    fn raw(&self, args: &[&str]) -> Result<Output> {
        Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            // 不要在非交互场景弹认证提示，宁可失败也别挂住
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .with_context(|| format!("执行 git {} 失败（git 装了吗？）", args.join(" ")))
    }

    /// 执行并要求成功，返回 trim 过的 stdout。
    fn run(&self, args: &[&str]) -> Result<String> {
        let out = self.raw(args)?;
        if !out.status.success() {
            bail!(
                "git {} 失败：{}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    /// 执行但允许失败，返回 (成功?, stdout, stderr)。
    pub fn try_run(&self, args: &[&str]) -> Result<(bool, String, String)> {
        let out = self.raw(args)?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim().to_owned(),
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ))
    }

    pub fn init(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        self.run(&["init"])?;
        Ok(())
    }

    pub fn clone_into(url: &str, dest: &Path) -> Result<()> {
        let out = Command::new("git")
            .args(["clone", url])
            .arg(dest)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .context("执行 git clone 失败")?;
        if !out.status.success() {
            bail!(
                "git clone {url} 失败：{}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    pub fn remote(&self) -> Option<String> {
        self.try_run(&["remote", "get-url", "origin"])
            .ok()
            .filter(|(ok, _, _)| *ok)
            .map(|(_, url, _)| url)
            .filter(|u| !u.is_empty())
    }

    pub fn set_remote(&self, url: &str) -> Result<()> {
        if self.remote().is_some() {
            self.run(&["remote", "set-url", "origin", url])?;
        } else {
            self.run(&["remote", "add", "origin", url])?;
        }
        Ok(())
    }

    pub fn branch(&self) -> Option<String> {
        self.try_run(&["rev-parse", "--abbrev-ref", "HEAD"])
            .ok()
            .filter(|(ok, _, _)| *ok)
            .map(|(_, b, _)| b)
            .filter(|b| !b.is_empty() && b != "HEAD")
    }

    pub fn head_short(&self) -> Option<String> {
        self.try_run(&["rev-parse", "--short", "HEAD"])
            .ok()
            .filter(|(ok, _, _)| *ok)
            .map(|(_, h, _)| h)
            .filter(|h| !h.is_empty())
    }

    /// `git status --porcelain` 的原始行。
    pub fn dirty_files(&self) -> Result<Vec<String>> {
        let out = self.run(&["status", "--porcelain"])?;
        Ok(out.lines().map(|l| l.trim_end().to_owned()).collect())
    }

    /// 相对 upstream 的 (ahead, behind)；没有 upstream 时返回 None。
    pub fn ahead_behind(&self) -> Option<(u32, u32)> {
        let (ok, out, _) = self
            .try_run(&["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
            .ok()?;
        if !ok {
            return None;
        }
        let mut parts = out.split_whitespace();
        let ahead = parts.next()?.parse().ok()?;
        let behind = parts.next()?.parse().ok()?;
        Some((ahead, behind))
    }

    pub fn has_upstream(&self) -> bool {
        self.try_run(&["rev-parse", "--abbrev-ref", "@{upstream}"])
            .map(|(ok, _, _)| ok)
            .unwrap_or(false)
    }

    /// 提交所有改动；无改动时返回 `Ok(None)`，否则返回新 commit 的短 hash。
    pub fn commit_all(&self, message: &str) -> Result<Option<String>> {
        self.run(&["add", "-A"])?;
        if self.dirty_files()?.is_empty() {
            return Ok(None);
        }
        let mut args: Vec<String> = Vec::new();
        // 用户没配 git 身份时给个兜底，否则 commit 会直接失败
        if self.identity().is_none() {
            args.extend([
                "-c".into(),
                "user.name=cloudot".into(),
                "-c".into(),
                "user.email=cloudot@localhost".into(),
            ]);
        }
        args.extend(["commit".into(), "-m".into(), message.to_owned()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&refs)?;
        Ok(self.head_short())
    }

    /// 已配置的 `user.name <user.email>`，未配置返回 None。
    pub fn identity(&self) -> Option<String> {
        let name = self
            .try_run(&["config", "user.name"])
            .ok()
            .filter(|(ok, _, _)| *ok)
            .map(|(_, v, _)| v)
            .filter(|v| !v.is_empty())?;
        let email = self
            .try_run(&["config", "user.email"])
            .ok()
            .filter(|(ok, _, _)| *ok)
            .map(|(_, v, _)| v)
            .filter(|v| !v.is_empty())?;
        Some(format!("{name} <{email}>"))
    }

    pub fn pull_rebase(&self) -> Result<PullOutcome> {
        if self.remote().is_none() || !self.has_upstream() {
            return Ok(PullOutcome::Skipped);
        }
        let before = self.head_short();
        let (ok, stdout, stderr) = self.try_run(&["pull", "--rebase"])?;
        if !ok {
            // store 工作树就是用户的实时配置：停在冲突态会让 App 立刻读到
            // 塞满 `<<<<<<< HEAD` 的文件。所以必须回滚，绝不留中间状态。
            let uu = self.conflicted_files();
            let aborted = self.rebase_abort();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            let branch = self.branch().unwrap_or_else(|| "main".to_owned());
            let remote_ref = format!("origin/{branch}");

            if !aborted && !uu.is_empty() {
                return Err(crate::tagged(
                    crate::ErrorKind::PullConflict,
                    format!(
                        "拉取远端改动时冲突，而且自动回滚也失败了。\n\
                         ⚠️  {} 里的文件现在可能含有 git 冲突标记，\
                         而它们是被软链引用的实时配置。\n\
                         请立刻手动处理：git -C {} rebase --abort\n\n\
                         git 原始输出：\n{}",
                        self.dir.display(),
                        self.dir.display(),
                        detail
                    ),
                ));
            }

            // abort 之后工作树干净；用 HEAD vs origin/<branch> 做结构化 diff，
            // 给 GUI 展示并让用户选边（`cloudot resolve --theirs/--ours`）。
            let report = self.conflict_report(&branch, &remote_ref, &uu);
            let list = if report.files.is_empty() {
                String::new()
            } else {
                format!(
                    "\n冲突文件：\n  {}\n",
                    report
                        .files
                        .iter()
                        .map(|f| f.path.as_str())
                        .collect::<Vec<_>>()
                        .join("\n  ")
                )
            };
            let message = format!(
                "远端有冲突改动，已自动回滚 —— store 工作树保持干净，你的配置没被动过。\n\
                 本地这份仍然生效，远端那份还没落地。{list}\n\
                 怎么处理：\n\
                 \x20 想要远端的   cloudot resolve --theirs\n\
                 \x20 想留本地的   cloudot resolve --ours\n\
                 \x20 或在 App 里打开冲突面板逐文件查看 diff 后选边\n\n\
                 git 原始输出：\n{detail}"
            );
            return Err(anyhow::Error::new(crate::PullConflictError {
                report,
                message,
            }));
        }
        Ok(if self.head_short() == before {
            PullOutcome::UpToDate
        } else {
            PullOutcome::Updated
        })
    }

    /// 收集 `HEAD` 与远端 ref 之间有差异的文件及 unified diff。
    ///
    /// `hint_paths` 是 rebase 冲突时记下的 UU 列表（abort 前）；abort 后优先用
    /// `git diff --name-only HEAD...remote`，无关历史等没有 merge-base 时回退到
    /// 双侧 diff，再不行就用 hint。
    pub fn conflict_report(
        &self,
        branch: &str,
        remote_ref: &str,
        hint_paths: &[String],
    ) -> crate::ConflictReport {
        let mut paths = self.diff_name_only(&["HEAD...".to_owned() + remote_ref]);
        if paths.is_empty() {
            paths = self.diff_name_only(&["HEAD".into(), remote_ref.to_owned()]);
        }
        if paths.is_empty() {
            paths = hint_paths.to_vec();
        }
        paths.sort();
        paths.dedup();

        let files = paths
            .into_iter()
            .map(|path| {
                let (diff, truncated) = self.file_diff("HEAD", remote_ref, &path);
                crate::ConflictFile {
                    path,
                    diff,
                    truncated,
                }
            })
            .collect();

        crate::ConflictReport {
            branch: branch.to_owned(),
            remote_ref: remote_ref.to_owned(),
            files,
        }
    }

    fn diff_name_only(&self, rev_args: &[String]) -> Vec<String> {
        let mut args: Vec<&str> = vec!["diff", "--name-only"];
        let owned: Vec<&str> = rev_args.iter().map(String::as_str).collect();
        args.extend(owned);
        self.try_run(&args)
            .map(|(ok, out, _)| {
                if ok {
                    out.lines()
                        .filter(|l| !l.is_empty())
                        .map(str::to_owned)
                        .collect()
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default()
    }

    /// 单文件 unified diff；超过 [`crate::errors::CONFLICT_DIFF_LIMIT`] 截断。
    fn file_diff(&self, local: &str, remote: &str, path: &str) -> (String, bool) {
        let (ok, out, err) = match self.try_run(&["diff", local, remote, "--", path]) {
            Ok(v) => v,
            Err(_) => return (String::new(), false),
        };
        // diff 有差异时退出码 1，也算成功输出
        let text = if out.is_empty() { err } else { out };
        if !ok && text.is_empty() {
            return (String::new(), false);
        }
        let limit = crate::errors::CONFLICT_DIFF_LIMIT;
        if text.len() > limit {
            (truncate_on_char_boundary(&text, limit), true)
        } else {
            (text, false)
        }
    }

    /// `git reset --hard <rev>`。给 `resolve --theirs` 用。
    pub fn reset_hard(&self, rev: &str) -> Result<()> {
        self.run(&["reset", "--hard", rev])?;
        Ok(())
    }

    /// `git push --force-with-lease`。给 `resolve --ours` 用。
    ///
    /// lease 保护：若远端在我们 fetch 之后又有人推了新提交，推送会失败而不是
    /// 默默盖掉别人的工作。
    pub fn push_force_with_lease(&self) -> Result<()> {
        if self.remote().is_none() {
            bail!("还没配 remote");
        }
        self.run(&["push", "--force-with-lease"])?;
        Ok(())
    }

    /// 处于冲突未解决状态的文件（`git diff --diff-filter=U`）。
    pub fn conflicted_files(&self) -> Vec<String> {
        self.try_run(&["diff", "--name-only", "--diff-filter=U"])
            .map(|(ok, out, _)| {
                if ok {
                    out.lines().map(str::to_owned).collect()
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default()
    }

    /// 中止进行中的 rebase，返回是否成功。
    pub fn rebase_abort(&self) -> bool {
        self.try_run(&["rebase", "--abort"])
            .map(|(ok, _, _)| ok)
            .unwrap_or(false)
    }

    /// 从 git 历史里取回某个已被删除文件的最后内容。
    ///
    /// 用于修复「另一台机器 unadopt 后本机留下悬空软链」：内容虽然从 store
    /// 里删了，但还在历史里。
    pub fn content_before_deletion(&self, path: &str) -> Option<Vec<u8>> {
        let (ok, sha, _) = self
            .try_run(&["log", "--diff-filter=D", "--format=%H", "-1", "--", path])
            .ok()?;
        if !ok || sha.is_empty() {
            return None;
        }
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["show", &format!("{sha}^:{path}")])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(out.stdout)
    }

    /// 推送；首次推送自动设置 upstream。返回是否真的推了。
    pub fn push(&self) -> Result<bool> {
        if self.remote().is_none() {
            return Ok(false);
        }
        if self.has_upstream() {
            self.run(&["push"])?;
        } else {
            let branch = self.branch().unwrap_or_else(|| "main".to_owned());
            self.run(&["push", "-u", "origin", &branch])?;
        }
        Ok(true)
    }

    /// remote 是否可达。会走网络，失败不代表配置有错（可能只是断网）。
    pub fn remote_reachable(&self) -> Option<bool> {
        let url = self.remote()?;
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["ls-remote", "--exit-code", "-h", &url])
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=8")
            .output()
            .ok()?;
        // 退出码 2 = 连上了但没有匹配的 ref（空仓库），也算可达
        Some(out.status.success() || out.status.code() == Some(2))
    }
}

/// 在不超过 `limit` 字节的前提下按**字符边界**截断，并附上截断说明。
///
/// `&text[..limit]` 会 panic：`limit` 是字节数，落在多字节字符中间时 Rust 直接
/// 中止（`end byte index N is not a char boundary`）。配置文件里中文注释很常见，
/// 而这条路径专门处理冲突报告 —— 冲突时 panic 掉，用户拿到的就不是「diff 太长被
/// 截断」而是一个崩溃，连带看不到该选哪边。
///
/// 往前退而不是往后进：结果必须 ≤ limit，否则截断就没起到限制大小的作用。
fn truncate_on_char_boundary(text: &str, limit: usize) -> String {
    let mut end = limit.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…（diff 过长，已截断）", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 边界正好落在汉字中间时不能 panic。
    ///
    /// 这是回归测试：原来是 `&text[..limit]`，实测 panic 信息是
    /// `end byte index 65536 is not a char boundary; it is inside '配'`。
    #[test]
    fn truncates_multibyte_text_without_panicking() {
        let limit = crate::errors::CONFLICT_DIFF_LIMIT;
        // 前 limit-1 个 ASCII，再放一个 3 字节汉字 —— 边界就落在它的第 2 个字节上
        let mut text = "a".repeat(limit - 1);
        text.push('配');
        text.push_str("后面还有很多内容");
        assert!(
            !text.is_char_boundary(limit),
            "这个用例得让边界落在字符中间"
        );

        let cut = truncate_on_char_boundary(&text, limit);
        assert!(cut.starts_with(&"a".repeat(limit - 1)), "该保留前面的内容");
        assert!(cut.contains("已截断"), "要告诉用户被截断了");
        // 退一个字节到边界，那个汉字整体被丢掉
        assert!(!cut.contains('配'));
    }

    /// 截出来的内容不能超过上限 —— 否则截断就没意义了。
    #[test]
    fn truncated_content_stays_within_the_limit() {
        for limit in [1, 2, 3, 16, 1024] {
            let text = "配".repeat(limit * 2);
            let cut = truncate_on_char_boundary(&text, limit);
            let body = cut
                .strip_suffix("\n…（diff 过长，已截断）")
                .expect("有后缀");
            assert!(
                body.len() <= limit,
                "limit={limit} 时截出了 {} 字节",
                body.len()
            );
        }
    }

    /// 纯 ASCII 走的是最常见的路径，边界天然对齐。
    #[test]
    fn truncates_ascii_at_the_exact_limit() {
        let cut = truncate_on_char_boundary(&"x".repeat(100), 10);
        assert!(cut.starts_with(&"x".repeat(10)));
        assert!(!cut.starts_with(&"x".repeat(11)));
    }

    /// limit 比文本还长时不该越界。
    #[test]
    fn handles_a_limit_beyond_the_text() {
        let cut = truncate_on_char_boundary("短", 4096);
        assert!(cut.starts_with('短'));
    }
}
