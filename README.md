# cloudot

macOS 配置同步器。通过 git 在多台 Mac 之间同步 dotfiles，所有状态都在 `~/.cloudot` 下。

当前纳管 **ghostty · fish · karabiner · gitpic**，只支持 **git** 后端。后续按需扩展。

## 安装

```bash
brew install --cask tarnish233/tap/cloudot
```

一条命令装齐菜单栏 GUI 和 `cloudot` 命令 —— CLI 就在 .app 里面，Cask 顺手链到 PATH。

装完先跑一次（没有 Apple 开发者签名，不跑的话 GUI 打不开、`cloudot` 命令也会被系统杀掉）：

```bash
xattr -dr com.apple.quarantine /Applications/Cloudot.app
```

也可以从 [Releases](https://github.com/tarnish233/cloudot/releases) 直接下 DMG，
或者只装 CLI：`cargo install --path crates/cloudot-cli`。
分发细节见 [apps/Cloudot/dist/README.md](apps/Cloudot/dist/README.md)。

## 快速开始

```bash
# 第一台机器
cloudot init --remote git@github.com:<you>/dotfiles.git
cloudot add ghostty        # 备份 → 移进 store → 建软链
cloudot sync               # 提交 + 推送

# 第二台机器
cloudot init --remote git@github.com:<you>/dotfiles.git   # 自动 clone
cloudot apply              # 落地到本机
```

日常：改完配置跑 `cloudot sync`；怀疑有问题跑 `cloudot doctor`；想退出纳管跑 `cloudot unadopt ghostty`。

动手之前想先看会发生什么：`cloudot show <app>`（会动哪些文件）或给写命令加 `--dry-run`（预演）。

| 命令 | 用途 |
|---|---|
| `init [--remote URL] [--device NAME]` | 初始化，可重复执行 |
| `resolve --theirs` / `resolve --ours` | 拉取冲突后选边（远端 / 本地强推） |
| `add <app>... [--force] [--allow-secrets]` | 纳管 |
| `apply [--force]` | 把 store 落地到本机 |
| `status [--json]` | 纳管与 git 状态 |
| `sync [-m MSG]` | 提交 → 拉取 → 推送 → 落地 |
| `unadopt <app>` | 退出纳管，还原成实体文件 |
| `doctor [--json] [--net]` | 体检；有 error 时以非零码退出 |
| `apps [--json]` | 已知应用定义及检测/纳管状态 |
| `show <app> [--json]` | 一个应用的定义 + 每个路径的当前状态 |
| `backups [--json]`<br>`backups prune [--keep N] [--older-than D]` | 盘点 / 清理备份 |

全局开关：`--json`（统一信封输出）、`--dry-run`（只报告不动手）。

### 预演（`--dry-run`）

破坏性操作可以先看一遍再决定：

```bash
cloudot add fish --dry-run       # 会移动哪些文件、备份到哪、有没有疑似凭据
cloudot apply --dry-run          # 会建哪些链、哪些会被跳过及原因
cloudot sync --dry-run           # 会提交哪些改动、会不会推送、会落地什么
cloudot unadopt fish --dry-run
cloudot backups prune --keep 5 --dry-run
```

几条刻意的取舍：

- **校验照常跑。** 检测门禁、凭据扫描、逐路径可行性判断在预演里全都执行 —— 预演的主要价值就是让这些先说话，而不是只画一张乐观的清单。
- **`sync --dry-run` 不联网。** 不 fetch、不 commit、不 push。落后远端几个提交读的是**本地缓存**的 `@{upstream}`，所以那个数字可能是旧的（输出里会写明）。要知道远端当下的真实状态，只能跑真的 `sync`。
- **`init` 与 `resolve` 不支持**，会以 `unsupported` 明确拒绝。`init` 建的就是 `~/.cloudot` 本身，没有可预演的既有状态，而且它可重复执行；`resolve` 之前该看的是 diff，冲突时 `sync` 已经把每个文件的 diff 打出来了。
- **只读命令静默忽略它**（`status` / `doctor` / `apps` / `show`）—— 它们本来就不写盘。
- JSON 信封里加的是 `dry_run: true` 字段，**没有改 `action` 的语义**。消费方旧代码读到的 `action` 仍然是「发生了什么」，配合这个字段读成「将会发生什么」。

预演最危险的地方是「预测」和「真做」走两段独立代码，一旦漂移就会骗人 —— 那比没有预演更糟。所以 [link.rs](crates/cloudot-core/src/link.rs) 里的 `plan_adopt` 和 `adopt_file` 有测试逐分支比对结论与错误分类。

## 目录布局

```
~/.cloudot/
├─ config.toml                          # 本机配置（设备名、remote），不进 git
├─ links.toml                           # 本机建过哪些软链，不进 git
├─ lock                                 # 进程互斥用的 flock 目标
├─ store/                               # ← git 工作树，同时是所有软链的目标
│  ├─ .gitignore                        #   挡掉 .DS_Store 之类
│  ├─ manifest.toml                     #   纳管清单，进 git，跨机器共享
│  └─ files/.config/ghostty/config      #   镜像家目录结构
├─ backups/<时间戳>/.config/ghostty/config
└─ adopters/                            # 用户自定义 adopter，同 id 覆盖内置
```

`~/.config/ghostty/config` 是指向 `~/.cloudot/store/files/.config/ghostty/config` 的软链。

## 设计决策

**store 必须在本地。** 这是 symlink 策略的硬约束。如果链接目标在 iCloud 之类的按需下载存储上，文件被驱逐成占位符时软链就会读到空内容——终端配置在登录时读不到会直接影响可用性。

**落地策略默认 symlink。** 好处是改动即时生效，且不存在「本地内容与 store 漂移」这回事，状态检查退化成判断链接健康度。代价是有些 App 用「替换写入」而不是原地写，会把软链顶成实体文件——这是唯一真正的失效模式，`doctor` 专门检查它。ghostty 只读取配置、不回写，所以很安全。

**store 内路径镜像家目录**（`files/.config/ghostty/config`），不按 app 分目录。不需要处理重名，且 `git log files/.config/ghostty/config` 天然可读。

**manifest 里的路径一律存 `~/` 形式**，换机器甚至换用户名都能直接 apply。

**加应用 = 加一个 TOML。** 见 [adopters/ghostty.toml](adopters/ghostty.toml)。逻辑代码不需要改（只需在 `adopter.rs` 的 `BUILTIN` 登记一行）。

**已纳管应用的注意事项。** 每个 adopter TOML 的注释里写了取舍理由，两处需要特别留意：

| 应用 | 纳管什么 | 注意 |
|---|---|---|
| ghostty | `~/.config/ghostty/config` | 只读配置，最安全 |
| fish | `config.fish` · `conf.d/` 与 `functions/` 下的手写文件 | **不含 `fish_variables`**（fish 自己回写的运行时状态）和 `completions/`（工具自动生成） |
| karabiner | `karabiner.json` | ⚠️ GUI 保存时是**替换写入**，会顶掉软链。适合「一台编辑、其余只读」；多机都用 GUI 改键位就别纳管 |
| gitpic | `~/.config/gitpic/config.toml` | ⚠️ 默认含 GitHub token，会被凭据门禁拒绝。**先把 token 挪到 `GITPIC_TOKEN` 环境变量**再纳管 |

gitpic 的正确做法（实测只设 `GITPIC_TOKEN` 就能通过 `gitpic doctor`）：

```bash
# 1. 从 config.toml 删掉 token = 那一行
# 2. 把 token 放到不进 git 的地方，例如 ~/.config/fish/conf.d/secrets.fish（别纳管它）
set -gx GITPIC_TOKEN ghp_xxx
# 3. cloudot add gitpic
```

karabiner 被顶成实体文件后，`status` 会报 `本地是实体文件，未链接`，`doctor` 会给出 diff 与恢复命令。**恢复时先把本地内容拷进 store 再 `apply --force`**，否则本机那次改动会被 store 里的旧版本覆盖（`--force` 会先备份，但多一步麻烦）。


**git 走命令行而非 libgit2。** 直接继承已配好的 ssh-agent、`~/.ssh/config` 和 gh 凭据，不用自己接管认证。封装很薄（[git.rs](crates/cloudot-core/src/git.rs)），要换实现接口不用动。

**store 工作树就是用户的实时配置，所以它永远不能停在中间状态。** 软链目标直接指向工作树，一旦 `git pull --rebase` 冲突残留 `UU` 状态，App 立刻就会读到塞满 `<<<<<<< HEAD` 的配置文件。所以拉取冲突时会自动 `rebase --abort`，保住本地那份继续生效，再给出结构化的冲突报告（文件列表 + 每文件 diff）。用户在 App 冲突面板里选边，或终端跑 `cloudot resolve --theirs`（对齐远端）/ `--ours`（保留本地并 `--force-with-lease` 推上去）。

**manifest 是共享状态，软链是本地状态，两者会分叉。** 另一台机器 `unadopt` 之后，manifest 里没了、store 文件也删了，但本机的软链还在——那就成了悬空软链，App 直接读不到配置。只看 manifest 发现不了这种情况，所以本机额外记一份 `links.toml`（不进 git）用来对账。`apply` 会自动修复：先从 git 历史取回被删掉的内容，取不到就找最近的备份，还原成实体文件。**修不好时不删任何东西**——宁可留个坏链让 `doctor` 继续报警，也不能把用户唯一的线索抹掉。

**绝不静默覆盖。** 任何会破坏本地文件的操作都先备份到 `~/.cloudot/backups/<时间戳>/`。`apply` 遇到本地实体文件默认拒绝——那份内容可能比 store 新——要覆盖必须显式 `--force`。`unadopt` 是逃生门，随时能把配置拿回来。

**`add` 要么整体成功，要么整体回滚。** 多路径 adopter（fish、zsh 都会是）中途失败时，若不回滚就会留下「文件已被链进 store，但 manifest 和 links.toml 都没记」的半纳管状态——`status` 看不见它，`unadopt` 也撤不掉。回滚必须按当时实际做了什么分别处理：`LinkedFromStore` 那种情况下 store 里的内容是**别的机器**放的，撤销时绝对不能删。

**凭据先拦一道再说。** 用户可以往 `~/.cloudot/adopters/` 放自定义定义，所以今天就可能纳管到 `~/.config/gh/hosts.yml` 这类文件。[secrets.rs](crates/cloudot-core/src/secrets.rs) 在动手之前扫路径规则 + 内容模式，命中就整体拒绝，要强推得加 `--allow-secrets`；`doctor` 之后会持续以 error 级别报警，因为内容已经进了 git 历史。**扫描结果只报路径、行号和原因，绝不包含命中的值**，否则报告自己就成了泄漏渠道。这不是完整的密钥管理（那要等 age 加密那一步），只是一道门禁。

**并发用 flock 挡住。** `sync` 是 commit → pull --rebase → push → apply，两个进程同时跑就可能 rebase 套 rebase，而那正是会污染实时配置的中间状态。用 `flock(2)` 而不是 pid 文件：内核在 fd 关闭时自动释放，`kill -9` 也不留死锁。注意 flock 按「打开的文件描述」生效，同进程再 open 一次也会冲突——所以 `sync` 内部走的是不加锁的 `apply_inner`，[lock.rs](crates/cloudot-core/src/lock.rs) 里有测试把这个约束钉住。

**备份只提醒、不自动删。** 备份是孤儿软链自愈的兜底数据源（git 历史取不到时用它），所以清理必须显式：`doctor` 在超过 30 份或 50MB 时提醒，`cloudot backups prune` 才真的删。`--keep` 和 `--older-than` 取交集而不是并集，这样加上天数限制只会删得更少。

## 架构

```
crates/cloudot-core/     领域逻辑，CLI / 未来的 daemon 与 GUI 共用
├─ paths.rs              ~/.cloudot 布局，~ 展开与收缩
├─ config.rs             本机配置
├─ manifest.rs           纳管清单
├─ adopter.rs            应用适配定义（内置 + 用户覆盖）
├─ link.rs               备份 / 软链 / 链接状态判定  ← 安全关键
├─ links.rs              本机软链记录与孤儿检测      ← 安全关键
├─ secrets.rs            凭据检测（纳管门禁 + doctor）
├─ lock.rs               flock 进程互斥
├─ backups.rs            备份盘点与清理
├─ git.rs                git 命令行封装
├─ status.rs             状态模型（JSON 契约）
├─ doctor.rs             体检
└─ ops.rs                init / add / apply / sync / unadopt / resolve

crates/cloudot-cli/      CLI，core 的第一个消费者
apps/Cloudot/            SwiftUI GUI，CLI 的 JSON 消费者
├─ Sources/Cloudot/      Contracts / CloudotCLI / AppModel + 界面
├─ Tests/                契约测试 + 真实 CLI 输出 fixture
└─ make-app.sh           组装 .app（含 cloudot 二进制）
```

**一套命令面，三副面孔。** 所有能力先在 core 实现，CLI 的**每个**命令都提供 `--json`，包括出错。GUI 和 Agent 只消费这套契约，不会出现「只有 GUI 能做的事」。

JSON 用统一信封，所以消费方只需要一条解码路径：

```json
{ "schema": "cloudot.sync/v1",  "ok": true,  "result": { … } }
{ "schema": "cloudot.error/v1", "ok": false, "result": { "kind": "locked", "summary": …, "message": … } }
```

错误带**机器可读的分类**（`locked` / `needs_force` / `secrets_detected` / `pull_conflict` …）。内部一律用 `anyhow`，分类是塞进错误链里再在边界上 downcast 取回的——不用把整个代码库改成自定义错误类型，也不用让界面去 grep 中文错误信息。`pull_conflict` 额外带 `conflict: { branch, remote_ref, files: [{path, diff, truncated}] }`，供 GUI 画选边面板。

schema 带版本号，加字段兼容，改语义要升版本。`status` 带 `initialized`：未跑过 `init` 时返回**成功**信封 + `initialized: false`（写路径仍是 `not_initialized`）。注意 `doctor` 在有 error 级检查项时会**以非零码退出但输出合法的成功信封**，所以消费方要先解信封再看退出码，不能反过来。

## GUI

纯菜单栏工具（不占 Dock、不进 Cmd-Tab）+ 一个主窗口，SwiftUI 写的，在 [apps/Cloudot/](apps/Cloudot/)。需要 macOS 15+。

```bash
cd apps/Cloudot
./make-app.sh          # 构建 GUI + CLI，组装成 build/Cloudot.app
open build/Cloudot.app
./test.sh              # Swift 契约与界面测试（需要 Xcode 提供 XCTest）
```

主窗口从菜单栏面板的「设置」进入，分**概览 / 应用 / 体检 / 备份 / 关于**五个页面。

**空态是产品状态，不是故障。** 刚装好还没 `init` 时，菜单栏显示「开始设置」、图标保持常态环，概览页给一个可选 remote 的初始化表单——不会红 banner，也不会永久转圈。同步按钮在未 init 时禁用。

**拉取冲突走选边面板。** sync 撞车后自动 abort，主窗口弹出文件列表 + monospaced diff，可选「用远端」或「保留本地并推送」。`--force` / `--allow-secrets` 仍不进 GUI。

**确认对话框只由菜单栏控制器呈现一次**（AppKit `NSAlert`），菜单栏面板和主窗口不再各自挂 SwiftUI confirm——否则关窗后仍存活的宿主会再弹一个，锚不到 status item 就掉到屏幕左下角。

「关于」页默认只显示一行 App 版本；**仅当 CLI 与 App 版本不一致**时才标橙显示 CLI 版本和实际路径（bundle 内 / `~/.cargo/bin` / `defaults write` 指定）。提供「检查更新」按钮可强制重查。App 的版本号由 `make-app.sh` 从 workspace 的 `Cargo.toml` 读，不另写一份。

### 菜单栏图标

菜单栏用系统自带的 SF Symbol，静态、无动画。所有图标都是 **template image**：
深色菜单栏显示为白色，浅色菜单栏由 AppKit 自动反转，避免固定白色在浅色背景上消失。

| 状态 | 图标 |
|---|---|
| 正常 / 刷新中 / 同步中 | `arrow.triangle.2.circlepath`（和 App 图标同一个符号） |
| 待同步 | `arrow.up.arrow.down` |
| 有损坏 | `exclamationmark.triangle.fill` |
| 找不到 CLI | `icloud.slash` |
| 同步成功 / 失败 | `checkmark.circle.fill` / `xmark.octagon.fill`，停留 0.7 秒后回到当前状态 |

早先这里是一个代码绘制的小机器人，有眨眼、扫描、机械臂等一整套逐帧动效。换掉是因为
它和系统的视觉语言不搭，而菜单栏是每天都要看的地方 —— 一个安静的系统符号比一个会动
的吉祥物更耐看，也更容易一眼读出状态。

**菜单栏只在需要你做事时才改形状。** 常态就是 App 图标那个双箭头同步环 —— 同一个
SF Symbol，菜单栏和 Finder 里认的是同一个东西。刷新中和同步中也共用它：静态图标本来
就区分不了「在跑」，而那件事由面板里的转圈和跟着状态走的 tooltip 负责说，菜单栏不必
再表一次态。真正需要图标变形的只有「要你动手」（待同步）和「出事了」（损坏 / 找不到 CLI）。

菜单栏没有动画，所以也没有「减少动态效果」的降级处理。成功/失败那 0.7 秒的换图是
离散的状态指示，不是动效 —— 没有位移、没有插值，该辅助功能设置管不到它。

应用图标见 [Icon/README.md](apps/Cloudot/Icon/README.md)，由 CoreGraphics 绘制，不使用第三方素材。
它的 glyph 和菜单栏「同步中」是**同一个** `arrow.triangle.2.circlepath`。
应用图标不支持深浅色自适应——`.icns` 无变体概念，appiconset 的 dark 图会被 `actool`
静默丢弃，macOS 26 的自适应要靠只有 GUI 的 Icon Composer。菜单栏图标不受影响，
它是单色 template 渲染，本来就跟随系统外观。

### 其它设计决定

**GUI 只是 CLI 的 JSON 消费者。** 起进程跑 `cloudot --json`，不复用 Rust 的 core：进程隔离，界面崩了动不到你的配置，而且这套 JSON 本来就是为三端共用设计的。任何一次改动操作之后一定重新拉 `status`，界面绝不自己推测新状态。

**`--force` 和 `--allow-secrets` 刻意不进 GUI。** 这两个开关真能丢数据或把凭据推进 git，误点的代价比在终端里敲错命令高得多。GUI 遇到这类错误时，会按错误分类把该敲的命令直接显示出来让你复制。其余破坏性操作（纳管、退出纳管、清理备份、安装更新）走确认对话框。

**`--dry-run` 也不进 GUI。** 纳管确认框本身就是预演，而且它列的是 `cloudot show` 拿到的**真实路径清单**（不是一句笼统的「会把配置移进 store」）—— 再加一个预演开关是重复。`show` 取不到时清单为空、退回通用文案，不阻塞操作。

**自动只读、手动写。** 同步永远要你点。

**GUI 自更新是下载 DMG 替换自己**，不判断安装来源、不在界面里跑 brew。版本发现走
`releases/latest` 的 302 重定向（不用有速率限制的 GitHub API），装完问你要不要重启，
不静默拉起新进程。Homebrew 用户想用 `brew upgrade --cask cloudot` 升级就自己在终端跑。
不用 Sparkle 的理由和替换顺序见 [dist/README.md](apps/Cloudot/dist/README.md)。

状态刷新靠 **FSEvents**：配置文件真的变了才刷，没变化零开销。原来是每 20 秒轮询一次，每小时白起 720 个进程——而轮询其实发现不了远端改动（`status` 读的是本地缓存的 `@{upstream}` ref，**不 fetch**），唯一能新发现的就是「本机改了配置」，而那件事文件系统会主动告诉我们。另留一天一次的兜底轮询，防 FSEvents 漏事件（休眠期间的变化、监视目录被整个移走）。

**必须同时监视 `store/files` 和每个纳管文件所在的目录**，这是实测结论：通过软链改配置只在 **store** 侧报事件（`~/.config` 一侧一个都没有）；软链被替换写入顶掉只在 **配置目录** 侧报。少看一处就漏掉一整类变化。另外改动类操作后有 3 秒抑制窗口，否则 `sync` 自己写的文件会让 watcher 再触发一轮。

**快捷键挂在按钮上而不是 `.commands`。** `LSUIElement` 应用没有自己的菜单栏，菜单项形式的快捷键不会生效；`Button` 上的 `.keyboardShortcut` 在窗口或面板获得焦点时就能用。⌘S 同步、⌘R 刷新、⌘Q 退出。

**不用 .xcodeproj，用 SwiftPM + 组装脚本。** `swift build` 在纯 CommandLineTools 下就能跑，整条链子在终端里可复现；想改界面时 `open Package.swift`，Xcode 直接当完整 IDE 用（Preview 也能用）。代价只是 Info.plist 和 bundle 结构要自己拼，就是 `make-app.sh` 那几十行。`cloudot` 二进制会打进 `.app/Contents/Resources/`，所以用户不必先装 CLI。

## 测试

```bash
cargo test              # 73 个 Rust 单元测试
./e2e.sh                # 80 项端到端断言
apps/Cloudot/test.sh    # Swift 测试（契约 + 菜单栏图标 + 自更新；72 个，5 个默认跳过）
```

`e2e.sh` 在 `/tmp/cloudot-e2e` 下用假 `HOME` 模拟两台机器 + 一个 bare remote，完全不碰真实的 `~/.config` 和 `~/.cloudot`。覆盖：

- 首次纳管、跨机器落地、双向同步、unadopt 还原
- 软链被实体文件顶掉后的拒绝覆盖与 `--force` 恢复
- **回归**：跨机器 unadopt 后的悬空软链自愈；修不好时必须报警而不是装作没事
- **回归**：rebase 冲突自动回滚，实时配置不被冲突标记污染；`resolve --theirs/--ours`；未 init 的 status 成功返回
- **回归**：`--dry-run` 一个字节都不写（逐命令验文件系统、manifest、links.toml 均未变），不支持的命令明确报 `unsupported`
- `add` 中途失败整体回滚，manifest 与 links.toml 均不被写脏
- 凭据门禁拦下、报告不回显凭据值、`--allow-secrets` 放行、`doctor` 持续报错
- `show` 列出目标与 store 位置、带链接状态、未 init 也能用
- store 的 `.gitignore` 生效、备份盘点与 prune

`CLOUDOT_HOME` 覆盖 `$HOME`，`CLOUDOT_ROOT` 单独覆盖 `~/.cloudot`——都只为测试隔离，正常使用不需要设置。单元测试走 `Layout::with_home()` 而不是环境变量，因为环境变量是进程全局的，并行测试会互相干扰。

Swift 侧的 fixture 是从**真实 CLI** 抓下来的输出，不是手写的——手写 fixture 只能验证「我以为的格式」。重抓：

```bash
cloudot --json status  > apps/Cloudot/Tests/CloudotTests/Fixtures/status.json
cloudot --json doctor  > apps/Cloudot/Tests/CloudotTests/Fixtures/doctor.json
# apps / backups / sync / apply 同理
```

契约测试里有两条**方向相反**的断言，都是刻意的：未知的**错误分类**降级成 `other`（新分类只影响引导文案，不该让错误显示整体失效），未知的**链接状态**则整体解码失败（新状态可能意味着新的损坏形态，宁可报「输出异常」也不要静默降级成一个看起来正常的值）。


## 还没做

按优先级：其余应用的 adopter（zsh / zed 等）· secret 扫描与 age 加密 · daemon（后台自动同步）· agent skill 与 MCP · 正式签名与公证（现在是 adhoc，用户得手动去掉隔离属性）。
