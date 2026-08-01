use crate::Layout;
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;

/// `~/.cloudot/lock` 上的独占文件锁，持有到 Drop。
///
/// 为什么需要：`sync` 会 commit → pull --rebase → push → apply。两个 cloudot 同时跑
/// 就可能 rebase 套 rebase，而 store 工作树是软链目标，那种中间状态会直接把用户的
/// 实时配置搞坏。
///
/// 用 `flock(2)` 而不是「写 pid 文件再判断进程是否存活」：内核会在 fd 关闭时
/// 自动释放，进程被 kill -9 也不会留下需要人工清理的死锁。
pub struct Lock {
    /// 只为持有 fd —— Drop 关闭它时锁自动释放。
    _file: File,
}

impl Lock {
    pub fn acquire(layout: &Layout) -> Result<Self> {
        std::fs::create_dir_all(layout.root())
            .with_context(|| format!("创建 {} 失败", layout.root().display()))?;
        let path = layout.lock_file();
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("打开锁文件 {} 失败", path.display()))?;

        // SAFETY: fd 来自上面刚打开的 File，在本次调用期间一直有效。
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            let busy = matches!(
                err.raw_os_error(),
                Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
            );
            if busy {
                return Err(crate::tagged(
                    crate::ErrorKind::Locked,
                    format!(
                        "另一个 cloudot 进程正在操作 {}，等它结束再试",
                        layout.root().display()
                    ),
                ));
            }
            return Err(err).with_context(|| format!("给 {} 加锁失败", path.display()));
        }
        Ok(Self { _file: file })
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
    #[test]
    fn second_acquire_while_held_conflicts_even_in_same_process() {
        let home = TempHome::new("lock-conflict");
        let layout = home.layout();
        let _guard = Lock::acquire(&layout).unwrap();
        assert!(Lock::acquire(&layout).is_err());
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
