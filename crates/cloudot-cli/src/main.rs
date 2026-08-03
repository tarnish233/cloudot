//! cloudot CLI —— core 的第一个消费者。
//!
//! `--json` 是全局开关（`cloudot --json sync` 和 `cloudot sync --json` 都行），
//! **所有**命令包括出错都会走统一信封：
//!
//! ```json
//! { "schema": "cloudot.sync/v1", "ok": true,  "result": { … } }
//! { "schema": "cloudot.error/v1", "ok": false, "result": { "kind": "locked", … } }
//! ```
//!
//! GUI 和 Agent 只需要一条解码路径：看 `ok`，然后按 `schema` 解 `result`。

use anyhow::Result;
use clap::{Parser, Subcommand};
use cloudot_core::errors::ErrorReport;
use cloudot_core::git::PullOutcome;
use cloudot_core::link::AdoptAction;
use cloudot_core::ops::{self, ApplyAction, HealSource};
use cloudot_core::{Layout, backups, doctor, errors, status};
use serde::Serialize;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "cloudot",
    version,
    about = "macOS 配置同步器 —— 通过 git 在多台机器间同步 dotfiles",
    long_about = None,
    // clap 默认短选项是 -V；常见习惯是 -v，关掉内建再自己挂一份
    disable_version_flag = true,
)]
struct Cli {
    /// 打印版本号
    #[arg(short = 'v', visible_short_alias = 'V', long = "version", action = clap::ArgAction::Version)]
    _version: (),

    /// 以 JSON 输出（含错误）；GUI 与 Agent 用这个模式
    #[arg(long, global = true)]
    json: bool,

    /// 只报告将会发生什么，不动任何文件、不碰 git
    ///
    /// 支持 add / apply / sync / unadopt / backups prune。
    /// 只读命令（status / doctor / apps / show）收到它会静默忽略 —— 它们本来就不写盘。
    /// init 与 resolve 不支持：前者是「什么都还没有」，后者用户已经看过 diff 才选边。
    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 初始化 ~/.cloudot（可重复执行）
    Init {
        /// git remote，如 git@github.com:you/dotfiles.git；已有 cloudot 仓库会直接 clone 下来
        #[arg(long)]
        remote: Option<String>,
        /// 设备名，默认取 hostname
        #[arg(long)]
        device: Option<String>,
    },
    /// 纳管一个应用的配置：备份 → 移进 store → 建软链
    Add {
        /// 应用 id，如 ghostty
        apps: Vec<String>,
        /// store 里已有内容时，备份本地那份并用 store 的覆盖
        #[arg(long)]
        force: bool,
        /// 即使扫到疑似凭据也照样纳管（内容会进 git 历史，慎用）
        #[arg(long)]
        allow_secrets: bool,
    },
    /// 把 store 里的配置落地到本机（新机器上的主命令）
    Apply {
        /// 覆盖本地已存在的实体文件（一定会先备份）
        #[arg(long)]
        force: bool,
    },
    /// 查看纳管状态与 git 状态
    Status,
    /// 提交 → 拉取 → 推送 → 重新落地
    Sync {
        /// 自定义 commit message
        #[arg(short, long)]
        message: Option<String>,
    },
    /// 退出纳管：软链换回实体文件，并从 store 移除
    Unadopt { app: String },
    /// 体检：初始化状态、git 状态、逐文件链接健康度、明文凭据
    Doctor {
        /// 额外探测 remote 可达性（走网络）
        #[arg(long)]
        net: bool,
    },
    /// 列出所有已知应用定义及其检测/纳管状态
    Apps,
    /// 看一个应用的定义与当前状态：会动哪些文件、现在链好了没
    Show {
        /// 应用 id，如 ghostty
        app: String,
    },
    /// 盘点或清理 ~/.cloudot/backups
    Backups {
        #[command(subcommand)]
        action: Option<BackupAction>,
    },
    /// 拉取冲突后选边：用远端或保留本地
    Resolve {
        /// 用远端覆盖本地（`reset --hard origin/<branch>` + apply）
        #[arg(long, group = "side")]
        theirs: bool,
        /// 保留本地并推送（`push --force-with-lease`）
        #[arg(long, group = "side")]
        ours: bool,
    },
}

#[derive(Subcommand)]
enum BackupAction {
    /// 删除多余的备份
    Prune {
        /// 保留最近几份
        #[arg(long, default_value_t = backups::DEFAULT_KEEP)]
        keep: usize,
        /// 只删早于多少天的（与 --keep 取交集，只会删得更少）
        #[arg(long)]
        older_than: Option<u64>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            if cli.json {
                let report = ErrorReport::from(&e);
                // 出错时也要输出合法 JSON，否则 GUI 只能去猜 stderr
                match envelope(errors::SCHEMA, false, &report) {
                    Ok(text) => println!("{text}"),
                    Err(_) => println!(
                        r#"{{"schema":"{}","ok":false,"result":{{"kind":"other","summary":"序列化失败","message":"序列化失败"}}}}"#,
                        errors::SCHEMA
                    ),
                }
            } else {
                eprintln!("错误：{e:#}");
            }
            ExitCode::FAILURE
        }
    }
}

fn envelope<T: Serialize>(schema: &str, ok: bool, result: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "schema": schema,
        "ok": ok,
        "result": result,
    }))?)
}

fn emit<T: Serialize>(schema: &str, result: &T) -> Result<()> {
    println!("{}", envelope(schema, true, result)?);
    Ok(())
}

fn run(cli: &Cli) -> Result<ExitCode> {
    let layout = Layout::discover()?;
    let json = cli.json;
    let dry_run = cli.dry_run;

    // 不支持预演的写命令要明确说清，别让用户以为「什么都没发生 = 预演成功了」。
    if dry_run {
        match &cli.command {
            Command::Init { .. } => {
                return Err(cloudot_core::tagged(
                    cloudot_core::ErrorKind::Unsupported,
                    "init 不支持 --dry-run：它建的就是 ~/.cloudot 本身，没有可预演的既有状态。\n\
                     它可以重复执行，直接跑即可。",
                ));
            }
            Command::Resolve { .. } => {
                return Err(cloudot_core::tagged(
                    cloudot_core::ErrorKind::Unsupported,
                    "resolve 不支持 --dry-run：选边之前该看的是 diff，冲突时 `cloudot sync` \
                     已经把每个文件的 diff 打出来了（GUI 里是冲突面板）。",
                ));
            }
            // 只读命令：静默忽略，它们本来就不写盘
            _ => {}
        }
    }

    match &cli.command {
        Command::Init { remote, device } => {
            let out = ops::init(&layout, remote.as_deref(), device.as_deref())?;
            if json {
                emit(ops::INIT_SCHEMA, &out)?;
            } else {
                println!(
                    "{} {}",
                    if out.already {
                        "已更新"
                    } else {
                        "已初始化"
                    },
                    out.root.display()
                );
                println!("  设备    {}", out.device);
                println!("  store   {}", layout.store().display());
                match &out.remote {
                    Some(r) => println!("  remote  {r}"),
                    None => println!("  remote  （未配置，只在本地留 git 历史）"),
                }
                if out.cloned && out.apps_in_store > 0 {
                    println!(
                        "\n已从 remote clone 下来（{} 个纳管条目），跑 `cloudot apply` 落地到本机。",
                        out.apps_in_store
                    );
                } else if out.cloned {
                    println!("\n远端目前是空仓库。下一步：`cloudot add ghostty`");
                } else {
                    println!("\n下一步：`cloudot add ghostty`");
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Add {
            apps,
            force,
            allow_secrets,
        } => {
            if apps.is_empty() {
                anyhow::bail!("要纳管哪个应用？`cloudot apps` 看可选项");
            }
            let mut outcomes = Vec::new();
            for app in apps {
                let out = ops::add(&layout, app, *force, *allow_secrets, dry_run)?;
                if !json {
                    print_add(&out);
                }
                outcomes.push(out);
            }
            if json {
                emit(ops::ADD_SCHEMA, &outcomes)?;
            } else if dry_run {
                print_dry_run_footer();
            } else if cloudot_core::Config::load(&layout)?.remote.is_some() {
                println!("\n跑 `cloudot sync` 推到 remote。");
            } else {
                println!(
                    "\n还没配 remote，只在本地留了 git 历史。\
                     配上之后就能跨机器同步：`cloudot init --remote <git-url>`"
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Apply { force } => {
            let out = ops::apply(&layout, *force, dry_run)?;
            if json {
                emit(ops::APPLY_SCHEMA, &out)?;
            } else if out.items.is_empty() && out.healed.is_empty() {
                println!("manifest 里没有任何纳管条目。");
            } else {
                print_heal(&out.healed);
                print_apply(&out.items, dry_run);
                if dry_run {
                    print_dry_run_footer();
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Status => {
            let st = status::build(&layout)?;
            if json {
                emit(status::SCHEMA, &st)?;
            } else {
                print_status(&st);
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Sync { message } => {
            let out = ops::sync(&layout, message.as_deref(), dry_run)?;
            if json {
                emit(ops::SYNC_SCHEMA, &out)?;
            } else if dry_run {
                print_sync_dry_run(&out);
            } else {
                match &out.commit {
                    Some(c) => println!("  ✓ 已提交 {c}"),
                    None => println!("  · 无本地改动可提交"),
                }
                match out.pull {
                    PullOutcome::Skipped => {
                        if out.remote.is_some() {
                            println!("  · 跳过拉取：还没有 upstream（本次推送后就有了）");
                        } else {
                            println!("  · 跳过拉取：没有配 remote");
                        }
                    }
                    PullOutcome::UpToDate => println!("  · 已是最新"),
                    PullOutcome::Updated => println!("  ✓ 已拉取远端改动"),
                }
                if out.pushed {
                    println!("  ✓ 已推送到 {}", out.remote.clone().unwrap_or_default());
                } else {
                    println!("  ! 未推送：还没配 remote（`cloudot init --remote <url>`）");
                }
                let changed = out
                    .applied
                    .items
                    .iter()
                    .any(|i| i.action != ApplyAction::AlreadyLinked);
                if !out.applied.healed.is_empty() || changed {
                    println!("\n落地：");
                    print_heal(&out.applied.healed);
                    print_apply(&out.applied.items, false);
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Unadopt { app } => {
            let out = ops::unadopt(&layout, app, dry_run)?;
            if json {
                emit(ops::UNADOPT_SCHEMA, &out)?;
            } else if dry_run {
                println!("{} 将退出纳管", out.name);
                for t in &out.restored {
                    println!("  → {t} 会还原成实体文件");
                }
                print_dry_run_footer();
            } else {
                println!("{} 已退出纳管", out.name);
                for t in &out.restored {
                    println!("  ✓ {t} 已还原成实体文件");
                }
                if let Some(c) = &out.commit {
                    println!("  提交 {c}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Doctor { net } => {
            let report = doctor::run(&layout, *net)?;
            if json {
                emit(doctor::SCHEMA, &report)?;
            } else {
                for c in &report.checks {
                    let mark = match c.level {
                        doctor::Level::Ok => "✓",
                        doctor::Level::Warn => "!",
                        doctor::Level::Error => "✗",
                    };
                    println!("{mark} {:<28} {}", c.name, c.message);
                    if let Some(h) = &c.hint {
                        println!("    → {h}");
                    }
                }
                println!(
                    "\n{}",
                    if report.ok {
                        "没有致命问题。"
                    } else {
                        "存在需要处理的问题。"
                    }
                );
            }
            // 有 error 级别的检查项时以非零码退出，方便脚本和 CI 用
            Ok(if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }

        Command::Apps => {
            let list = ops::apps(&layout)?;
            if json {
                emit(ops::APPS_SCHEMA, &list)?;
            } else {
                for a in &list {
                    let state = match (a.managed, a.detected) {
                        (true, _) => "已纳管",
                        (false, true) => "已装，未纳管",
                        (false, false) => "未检测到",
                    };
                    println!("  {:<12} {:<12} {}", a.id, state, a.name);
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Show { app } => {
            let out = ops::show(&layout, app)?;
            if json {
                emit(ops::SHOW_SCHEMA, &out)?;
            } else {
                print_show(&out);
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Resolve { theirs, ours } => {
            let side = match (*theirs, *ours) {
                (true, false) => ops::ResolveSide::Theirs,
                (false, true) => ops::ResolveSide::Ours,
                _ => anyhow::bail!("指定 --theirs（用远端）或 --ours（保留本地）之一"),
            };
            let out = ops::resolve(&layout, side)?;
            if json {
                emit(ops::RESOLVE_SCHEMA, &out)?;
            } else {
                match out.side {
                    ops::ResolveSide::Theirs => {
                        println!(
                            "已对齐远端 {}（HEAD {}）",
                            out.target,
                            out.head.as_deref().unwrap_or("?")
                        );
                        if let Some(applied) = &out.applied {
                            print_heal(&applied.healed);
                            print_apply(&applied.items, false);
                        }
                    }
                    ops::ResolveSide::Ours => {
                        println!(
                            "已把本地推到 {}（--force-with-lease，HEAD {}）",
                            out.target,
                            out.head.as_deref().unwrap_or("?")
                        );
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Backups { action } => {
            match action {
                None => {
                    let set = backups::list(&layout)?;
                    if json {
                        emit(backups::SCHEMA, &set)?;
                    } else if set.entries.is_empty() {
                        println!("还没有备份。");
                    } else {
                        for e in &set.entries {
                            println!(
                                "  {:<18} {:>3} 个文件  {}",
                                e.stamp,
                                e.files,
                                backups::human_bytes(e.bytes)
                            );
                        }
                        println!(
                            "\n共 {} 份 · {} 个文件 · {}",
                            set.entries.len(),
                            set.total_files,
                            backups::human_bytes(set.total_bytes)
                        );
                    }
                }
                Some(BackupAction::Prune { keep, older_than }) => {
                    let out = backups::prune(&layout, *keep, *older_than, dry_run)?;
                    if json {
                        emit(backups::SCHEMA, &out)?;
                    } else if out.removed.is_empty() {
                        println!("没有需要清理的备份（保留 {keep} 份）。");
                    } else if dry_run {
                        for e in &out.removed {
                            println!(
                                "  → 会删除 {:<18} {}",
                                e.stamp,
                                backups::human_bytes(e.bytes)
                            );
                        }
                        println!(
                            "\n会清理 {} 份，释放 {}，保留 {} 份。",
                            out.removed.len(),
                            backups::human_bytes(out.freed_bytes),
                            out.kept
                        );
                        print_dry_run_footer();
                    } else {
                        for e in &out.removed {
                            println!("  已删除 {:<18} {}", e.stamp, backups::human_bytes(e.bytes));
                        }
                        println!(
                            "\n清理 {} 份，释放 {}，保留 {} 份。",
                            out.removed.len(),
                            backups::human_bytes(out.freed_bytes),
                            out.kept
                        );
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ────────────────────────────────────────────── 人类可读输出

/// 预演的收尾提示。每条预演输出都要有 —— 不然「什么都没变」看起来像操作失败了。
fn print_dry_run_footer() {
    println!("\n以上是预演（--dry-run），没有动任何文件。去掉这个开关即真正执行。");
}

fn print_add(out: &ops::AddOutcome) {
    println!("{} ({})", out.name, out.id);
    for f in &out.files {
        let verb = match f.action {
            AdoptAction::AlreadyLinked => "已链接，未改动",
            AdoptAction::LinkedFromStore => "从 store 建链",
            AdoptAction::MovedIntoStore => "移入 store 并建链",
        };
        // 预演里 ✓ 会让人误以为已经做了
        let mark = if out.dry_run { "→" } else { "✓" };
        println!("  {mark} {} → {}  [{}]", f.target, f.store, verb);
        if let Some(b) = &f.backup {
            println!("    备份 {}", b.display());
        }
    }
    if out.dry_run {
        return;
    }
    match &out.commit {
        Some(c) => println!("  提交 {c}"),
        None => println!("  （无需提交）"),
    }
}

/// `sync --dry-run` 的报告。**刻意不联网**，所以只说三件本地能确定的事。
fn print_sync_dry_run(out: &ops::SyncOutcome) {
    match out.would_commit.as_deref() {
        Some([]) | None => println!("  · 无本地改动可提交"),
        Some(files) => {
            println!("  → 会提交 {} 处改动：", files.len());
            for f in files {
                println!("      {f}");
            }
        }
    }

    match out.behind {
        Some(0) => println!("  · 本地缓存显示没有落后远端"),
        Some(n) => println!("  → 本地缓存显示落后远端 {n} 个提交，会拉取"),
        None => println!("  · 还没有 upstream，无从比较"),
    }
    // 说清这个数字的来源，否则用户会以为它是当下的真实差异
    println!("    （预演不联网，以上落后数取自上次同步时缓存的远端状态）");

    match &out.remote {
        Some(r) => println!("  → 会推送到 {r}"),
        None => println!("  · 未配 remote，不会推送"),
    }

    let changed = out
        .applied
        .items
        .iter()
        .any(|i| i.action != ApplyAction::AlreadyLinked);
    if !out.applied.healed.is_empty() || changed {
        println!("\n落地：");
        print_heal(&out.applied.healed);
        print_apply(&out.applied.items, true);
    }
    print_dry_run_footer();
}

/// `show <app>` 的报告：定义 + 当前状态。
fn print_show(out: &ops::ShowOutcome) {
    println!("{} ({})", out.name, out.id);
    println!(
        "  本机     {}",
        if out.detected {
            "已检测到"
        } else {
            "未检测到"
        }
    );
    match &out.adopted_by {
        Some(device) => println!("  纳管     是（由 {device} 纳管）"),
        None => println!("  纳管     否"),
    }

    println!("\n检测路径（任一存在即视为已装）：");
    for d in &out.detect {
        println!("  {d}");
    }

    println!("\n会同步这些文件：");
    for p in &out.paths {
        let mark = if p.state.is_ok() { "✓" } else { "·" };
        println!("  {mark} {}", p.target);
        match &p.store {
            Some(store) => println!("      store    {store}"),
            None => println!("      store    （算不出位置，这个路径纳管不了）"),
        }
        // 未纳管时 store 里当然没有内容，直接抄 LinkState 的措辞会像在报故障
        let state = if !out.managed {
            match p.exists {
                true => "本机有这份文件，纳管后会移进 store 并建软链",
                false => "本机还没有这份文件，纳管后会从 store 建链（若远端有）",
            }
        } else {
            p.state.describe()
        };
        println!("      当前     {state}");
    }
    if !out.managed {
        println!("\n还没纳管。`cloudot add {}` 会做上面这些事；", out.id);
        println!("加 --dry-run 可以先看一遍具体动作（含备份路径与凭据检查）。");
    }
}

fn print_status(st: &status::Status) {
    println!("cloudot · {}", st.device);
    println!("  root    {}", st.root);
    if st.git.repo {
        let branch = st.git.branch.as_deref().unwrap_or("无分支");
        let head = st.git.head.as_deref().unwrap_or("无提交");
        println!("  git     {branch} @ {head}");
        match &st.git.remote {
            Some(r) => println!("  remote  {r}"),
            None => println!("  remote  （未配置）"),
        }
        if let (Some(a), Some(b)) = (st.git.ahead, st.git.behind)
            && (a > 0 || b > 0)
        {
            println!("          ↑{a} ↓{b}");
        }
        if !st.git.dirty.is_empty() {
            println!("          {} 处未提交改动", st.git.dirty.len());
        }
    } else {
        println!("  git     store 还不是 git 仓库");
    }

    if st.apps.is_empty() {
        println!("\n还没纳管任何应用。");
    } else {
        println!("\n已纳管");
        for app in &st.apps {
            println!("  {} ({})", app.name, app.id);
            for f in &app.files {
                let mark = if f.state.is_ok() { "✓" } else { "!" };
                println!("    {mark} {:<34} {}", f.target, f.state.describe());
            }
        }
    }

    if !st.orphans.is_empty() {
        println!("\n⚠️  孤儿软链（manifest 里已无此条目，但本机软链还在）");
        for o in &st.orphans {
            println!("  ✗ {:<34} {}", o.target, o.kind.describe());
        }
        println!("  跑 `cloudot apply` 修复：会从 git 历史或备份取回内容，还原成实体文件。");
    }

    if !st.available.is_empty() {
        println!("\n检测到但未纳管");
        for a in &st.available {
            println!("  · {:<12} `cloudot add {}`", a.name, a.id);
        }
    }
}

/// 落地结果。`dry_run` 时把「已做」的文案换成「将做」——
/// 同一份数据两种读法，靠这里区分，而不是给 `ApplyAction` 加变体。
fn print_apply(items: &[ops::ApplyItem], dry_run: bool) {
    for i in items {
        let (mark, verb) = match (i.action, dry_run) {
            (ApplyAction::AlreadyLinked, _) => ("·", "已链接，无需改动"),
            (ApplyAction::Linked, false) => ("✓", "已建链"),
            (ApplyAction::Linked, true) => ("→", "会建链"),
            (ApplyAction::Replaced, false) => ("✓", "已用 store 版本覆盖"),
            (ApplyAction::Replaced, true) => ("→", "会备份本地那份，再用 store 版本覆盖"),
            (ApplyAction::Skipped, _) => ("!", "跳过"),
        };
        println!("  {mark} {:<34} {verb}", i.target);
        if let Some(b) = &i.backup {
            println!("      备份 {}", b.display());
        }
        if let Some(n) = &i.note {
            println!("      {n}");
        }
    }
}

fn print_heal(healed: &[ops::HealItem]) {
    for h in healed {
        let (mark, verb) = match h.source {
            HealSource::GitHistory => ("✓", "已从 git 历史取回内容，还原成实体文件"),
            HealSource::Store => ("✓", "已还原成实体文件"),
            HealSource::Backup => ("✓", "已从备份取回内容，还原成实体文件"),
            HealSource::Failed => ("✗", "修复失败，软链保持原样"),
        };
        println!("  {mark} {:<34} {verb}", h.target);
        println!(
            "      （{} 已不在 manifest 中：{}）",
            h.app,
            h.kind.describe()
        );
        if let Some(n) = &h.note {
            println!("      {n}");
        }
    }
}
