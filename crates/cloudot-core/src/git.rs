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
    fn try_run(&self, args: &[&str]) -> Result<(bool, String, String)> {
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
            let conflicts = self.conflicted_files();
            let aborted = self.rebase_abort();
            let detail = if stderr.is_empty() { stdout } else { stderr };

            if !aborted && !conflicts.is_empty() {
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

            let list = if conflicts.is_empty() {
                String::new()
            } else {
                format!("\n冲突文件：\n  {}\n", conflicts.join("\n  "))
            };
            return Err(crate::tagged(
                crate::ErrorKind::PullConflict,
                format!(
                    "远端有冲突改动，已自动回滚 —— store 工作树保持干净，你的配置没被动过。\n\
                     本地这份仍然生效，远端那份还没落地。{list}\n\
                     怎么处理：\n\
                     \x20 看远端版本   git -C {store} show origin/{branch}:<文件路径>\n\
                     \x20 看本地改动   git -C {store} log -p -1\n\
                     \x20 想要远端的   git -C {store} reset --hard origin/{branch} && cloudot apply\n\
                     \x20 想留本地的   git -C {store} push --force-with-lease\n\n\
                     git 原始输出：\n{detail}",
                    store = self.dir.display(),
                    branch = self.branch().unwrap_or_else(|| "main".to_owned()),
                ),
            ));
        }
        Ok(if self.head_short() == before {
            PullOutcome::UpToDate
        } else {
            PullOutcome::Updated
        })
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
