use crate::Layout;
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

/// `~/.cloudot/lock` 上的独占文件锁，持有到 Drop。
///
/// 为什么需要：`sync` 会 commit → pull --rebase → push → apply。两个 cloudot 同时跑
/// 就可能 rebase 套 rebase，而 store 工作树是软链目标，那种中间状态会直接把用户的
/// 实时配置搞坏。
///
/// 用 `flock(2)` 而不是「写 pid 文件再判断进程是否存活」：内核会在 fd 关闭时
/// 自动释放，进程被 kill -9 也不会留下需要人工清理的死锁。
#[derive(Debug)]
pub struct Lock {
    /// 只为持有 fd —— Drop 关闭它时锁自动释放。
    _file: File,
}

/// 抢锁失败后的重试次数与间隔。
///
/// 为什么需要重试，而不是一撞就报错：**flock 会被子进程短暂继承**。我们全程在起
/// `git`，而 `fork` 复制 fd 表在前、`exec` 应用 `O_CLOEXEC` 在后 —— 这中间只要
/// 别的线程/进程正好在 fork，锁 fd 就被多持有了一小会儿，`O_CLOEXEC` 挡不住
/// （已实测确认，那是 flock 的固有性质，不是可修的 bug）。
///
/// 表现是偶发的假「另一个进程正在操作」：GUI 连着点两下同步、或 `sync` 内部起完
/// git 紧接着 apply，都可能撞上。窗口只有 exec 那一瞬间，几十毫秒足够躲过；
/// 真有另一个 cloudot 在跑长命令时，重试完照样会如实报错。
const RETRY_ATTEMPTS: u32 = 10;
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(30);

impl Lock {
    pub fn acquire(layout: &Layout) -> Result<Self> {
        std::fs::create_dir_all(layout.root())
            .with_context(|| format!("创建 {} 失败", layout.root().display()))?;
        let path = layout.lock_file();

        let mut last_busy = None;
        for attempt in 0..RETRY_ATTEMPTS {
            // `O_CLOEXEC` 在 open 时就带上（标准库本来也会设，这里写死把约束固定下来）。
            // 它挡不住上面说的 fork/exec 竞态，但能保证子进程 exec 成功后不再持有锁。
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .custom_flags(libc::O_CLOEXEC)
                .open(&path)
                .with_context(|| format!("打开锁文件 {} 失败", path.display()))?;

            // SAFETY: fd 来自上面刚打开的 File，在本次调用期间一直有效。
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                return Ok(Self { _file: file });
            }

            let err = std::io::Error::last_os_error();
            let busy = matches!(
                err.raw_os_error(),
                Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
            );
            if !busy {
                return Err(err).with_context(|| format!("给 {} 加锁失败", path.display()));
            }
            last_busy = Some(err);
            if attempt + 1 < RETRY_ATTEMPTS {
                std::thread::sleep(RETRY_DELAY);
            }
        }

        let _ = last_busy;
        Err(crate::tagged(
            crate::ErrorKind::Locked,
            format!(
                "另一个 cloudot 进程正在操作 {}，等它结束再试",
                layout.root().display()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempHome;

    #[test]
    fn acquires_and_releases_on_drop() {
        let home = TempHome::new("lock");
        let layout = home.layout();
        {
            let _guard = Lock::acquire(&layout).unwrap();
        }
        Lock::acquire(&layout).expect("Drop 之后应该能重新拿到");
    }

    /// flock 是按「打开的文件描述」生效的，同一进程里再 open 一次也会冲突。
    /// 这正是 `ops::sync` 不能在内部再调用会加锁的 `ops::apply` 的原因 ——
    /// 内部复用必须走不加锁的私有路径。
    ///
    /// 真被别人持有时重试完仍要如实报 `Locked`，不能因为加了重试就变成静默等待。
    #[test]
    fn second_acquire_while_held_conflicts_even_in_same_process() {
        let home = TempHome::new("lock-conflict");
        let layout = home.layout();
        let _guard = Lock::acquire(&layout).unwrap();
        let err = Lock::acquire(&layout).expect_err("被持有时该失败");
        assert_eq!(crate::errors::kind_of(&err), crate::ErrorKind::Locked);
    }

    /// 起过 `git` 子进程之后，锁必须还能立刻重新拿到。
    ///
    /// `fork` 复制 fd 表在前、`exec` 应用 `O_CLOEXEC` 在后，子进程会短暂持有锁 fd。
    /// 没有重试的话这会表现成偶发的假「另一个进程正在操作」—— 一个只在并发起
    /// 子进程时才出现、单独跑永远看不到的故障。
    #[test]
    fn lock_survives_spawning_git_subprocesses() {
        let home = TempHome::new("lock-after-spawn");
        let layout = home.layout();

        for _ in 0..5 {
            let guard = Lock::acquire(&layout).expect("该拿到锁");
            // 模拟 ops 里的 git 调用
            let _ = std::process::Command::new("git").arg("--version").output();
            drop(guard);
            // 紧接着再拿一次 —— 子进程刚退出，锁必须已经真的放掉
            Lock::acquire(&layout).expect("起过子进程后该能立刻重新拿到");
        }
    }

    #[test]
    fn creates_root_if_missing() {
        let home = TempHome::new("lock-mkdir");
        let layout = home.layout();
        assert!(!layout.root().exists());
        let _guard = Lock::acquire(&layout).unwrap();
        assert!(layout.lock_file().exists());
    }
}
