//! cloudot 的领域逻辑。CLI / 未来的 daemon 与 GUI 都只是它的消费者。
//!
//! 设计要点：
//! - 所有状态都在 `~/.cloudot` 下（见 [`Layout`]）。
//! - `~/.cloudot/store` 既是 git 工作树，也是所有软链的目标；它永远在本地。
//! - 落地策略默认 symlink，因此不存在「本地内容与 store 内容漂移」这回事，
//!   状态检查只需判断链接健康度 + git 状态。

pub mod adopter;
pub mod backups;
pub mod config;
pub mod doctor;
pub mod errors;
pub mod git;
pub mod link;
pub mod links;
pub mod lock;
pub mod manifest;
pub mod ops;
pub mod paths;
pub mod secrets;
pub mod status;

pub use config::Config;
pub use errors::{ErrorKind, ErrorReport, tagged};
pub use links::LinkRecords;
pub use lock::Lock;
pub use manifest::Manifest;
pub use paths::Layout;

#[cfg(test)]
pub(crate) mod testutil {
    use crate::Layout;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    /// 一个隔离的假家目录，Drop 时自动清掉。
    ///
    /// 不走 `CLOUDOT_HOME` 环境变量：那是进程全局的，并行测试会互相干扰。
    pub struct TempHome {
        pub path: PathBuf,
    }

    impl TempHome {
        pub fn new(tag: &str) -> Self {
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("cloudot-test-{}-{tag}-{seq}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("建临时家目录");
            Self { path }
        }

        pub fn layout(&self) -> Layout {
            Layout::with_home(&self.path)
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
