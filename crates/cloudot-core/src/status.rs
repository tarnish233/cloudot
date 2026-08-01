use crate::git::Git;
use crate::link::{self, LinkState};
use crate::links::{self, Orphan};
use crate::{Config, Layout, LinkRecords, Manifest, adopter};
use anyhow::Result;
use serde::Serialize;

pub const SCHEMA: &str = "cloudot.status/v1";

/// `cloudot status --json` 的输出契约。
///
/// 这是 CLI、（将来的）GUI 和 Agent 三端共用的唯一状态描述。加字段可以，
/// 改语义要同时改 `schema` 版本号。
#[derive(Debug, Serialize)]
pub struct Status {
    pub device: String,
    pub root: String,
    /// 一眼判断要不要人工介入：store 是 git 仓库、所有纳管文件链接正常、没有孤儿。
    /// GUI 和 Agent 直接读这个，不用自己遍历下面的嵌套结构。
    pub healthy: bool,
    pub git: GitInfo,
    /// 已纳管的应用。
    pub apps: Vec<AppStatus>,
    /// 本机检测到、但还没纳管的应用。
    pub available: Vec<AvailableApp>,
    /// 孤儿软链：manifest 里已经没有、但本机软链还指向 store 的条目。
    /// 非空说明有配置正处于（或即将处于）读不到的状态，跑 `cloudot apply` 修。
    pub orphans: Vec<Orphan>,
}

#[derive(Debug, Serialize)]
pub struct GitInfo {
    pub repo: bool,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub remote: Option<String>,
    /// `git status --porcelain` 的行，非空说明 store 有未提交改动。
    pub dirty: Vec<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct AppStatus {
    pub id: String,
    pub name: String,
    pub adopted_by: String,
    /// 该应用下所有文件都 `linked` 才是 ok。
    pub ok: bool,
    pub files: Vec<FileStatus>,
}

#[derive(Debug, Serialize)]
pub struct FileStatus {
    pub target: String,
    pub store: String,
    pub state: LinkState,
}

#[derive(Debug, Serialize)]
pub struct AvailableApp {
    pub id: String,
    pub name: String,
}

pub fn build(layout: &Layout) -> Result<Status> {
    let config = Config::load(layout)?;
    let manifest = Manifest::load(layout)?;
    let git = Git::new(layout.store());

    let git_info = if git.is_repo() {
        let (ahead, behind) = match git.ahead_behind() {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        };
        GitInfo {
            repo: true,
            branch: git.branch(),
            head: git.head_short(),
            remote: git.remote(),
            dirty: git.dirty_files().unwrap_or_default(),
            ahead,
            behind,
        }
    } else {
        GitInfo {
            repo: false,
            branch: None,
            head: None,
            remote: config.remote.clone(),
            dirty: Vec::new(),
            ahead: None,
            behind: None,
        }
    };

    let mut apps = Vec::new();
    for app in &manifest.apps {
        let files: Vec<FileStatus> = app
            .files
            .iter()
            .map(|f| {
                let target = layout.expand(&f.target);
                let store = layout.store_path(&f.store);
                FileStatus {
                    target: f.target.clone(),
                    store: f.store.clone(),
                    state: link::inspect(&target, &store),
                }
            })
            .collect();
        apps.push(AppStatus {
            id: app.id.clone(),
            name: app.name.clone(),
            adopted_by: app.adopted_by.clone(),
            ok: files.iter().all(|f| f.state.is_ok()),
            files,
        });
    }

    let available = adopter::load_all(layout)?
        .into_iter()
        .filter(|ad| manifest.app(&ad.id).is_none() && ad.detected(layout))
        .map(|ad| AvailableApp {
            id: ad.id,
            name: ad.name,
        })
        .collect();

    let records = LinkRecords::load(layout)?;
    let orphans = links::find_orphans(layout, &manifest, &records);
    let healthy = git_info.repo && orphans.is_empty() && apps.iter().all(|a| a.ok);

    Ok(Status {
        device: config.device,
        root: layout.root().display().to_string(),
        healthy,
        git: git_info,
        apps,
        available,
        orphans,
    })
}
