# cloudot

macOS 配置同步器。通过 git 在多台 Mac 之间同步 dotfiles，所有状态都在 `~/.cloudot` 下。

当前纳管 **ghostty · fish · karabiner · gitpic**，只支持 **git** 后端。后续按需扩展。

## 快速开始

```bash
cargo install --path crates/cloudot-cli    # 装到 ~/.cargo/bin

# 第一台机器
cloudot init --remote git@github.com:<you>/dotfiles.git
cloudot add ghostty        # 备份 → 移进 store → 建软链
cloudot sync               # 提交 + 推送

# 第二台机器
cloudot init --remote git@github.com:<you>/dotfiles.git   # 自动 clone
cloudot apply              # 落地到本机
```

日常：改完配置跑 `cloudot sync`；怀疑有问题跑 `cloudot doctor`；想退出纳管跑 `cloudot unadopt ghostty`。

| 命令 | 用途 |
|---|---|
| `init [--remote URL] [--device NAME]` | 初始化，可重复执行 |
| `add <app>... [--force] [--allow-secrets]` | 纳管 |
| `apply [--force]` | 把 store 落地到本机 |
| `status [--json]` | 纳管与 git 状态 |
| `sync [-m MSG]` | 提交 → 拉取 → 推送 → 落地 |
| `unadopt <app>` | 退出纳管，还原成实体文件 |
| `doctor [--json] [--net]` | 体检；有 error 时以非零码退出 |
| `apps [--json]` | 已知应用定义及检测/纳管状态 |
| `backups [--json]`<br>`backups prune [--keep N] [--older-than D]` | 盘点 / 清理备份 |

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

**store 工作树就是用户的实时配置，所以它永远不能停在中间状态。** 软链目标直接指向工作树，一旦 `git pull --rebase` 冲突残留 `UU` 状态，App 立刻就会读到塞满 `<<<<<<< HEAD` 的配置文件。所以拉取冲突时会自动 `rebase --abort`，保住本地那份继续生效，然后把远端/本地版本的查看与取舍命令列给用户。

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
└─ ops.rs                init / add / apply / sync / unadopt

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

错误带**机器可读的分类**（`locked` / `needs_force` / `secrets_detected` / `pull_conflict` …）。内部一律用 `anyhow`，分类是塞进错误链里再在边界上 downcast 取回的——不用把整个代码库改成自定义错误类型，也不用让界面去 grep 中文错误信息。

schema 带版本号，加字段兼容，改语义要升版本。注意 `doctor` 在有 error 级检查项时会**以非零码退出但输出合法的成功信封**，所以消费方要先解信封再看退出码，不能反过来。

## GUI

纯菜单栏工具（不占 Dock、不进 Cmd-Tab）+ 一个主窗口，SwiftUI 写的，在 [apps/Cloudot/](apps/Cloudot/)。需要 macOS 15+。

```bash
cd apps/Cloudot
./make-app.sh          # 构建 GUI + CLI，组装成 build/Cloudot.app
open build/Cloudot.app
./test.sh              # Swift 契约与界面测试（需要 Xcode 提供 XCTest）
```

主窗口从菜单栏面板的「打开主窗口」进入，分**概览 / 应用 / 体检 / 备份**四个页面。

### 菜单栏图标

菜单栏使用代码绘制的圆角小机器人。所有帧都是 **template image**：源图用白色绘制，
深色菜单栏显示为白色，浅色菜单栏则由 AppKit 自动反转，避免固定白色在浅色背景上消失。

| 状态 | 机器人动作 |
|---|---|
| 正常 | 安静待机，约四秒自然眨一次眼 |
| 待同步 | 只有天线发出一圈短促提示，不持续跳动 |
| 刷新中 | 双眼左右扫描，天线随扫描方向摆动 |
| 同步中 | 两只机械臂交替工作，机身轻微起伏 |
| 同步成功 / 失败 | 短促跳跃笑脸 / 叉眼摇头，播完回到当前状态 |
| 有损坏 | 叉眼、皱眉、歪头的静态姿态 |
| 找不到 CLI | 天线垂下并闭眼，整体降低不透明度 |

动画会遵守 macOS 的“减少动态效果”辅助功能设置；开启后只显示对应静态姿态。

应用图标见 [Icon/README.md](apps/Cloudot/Icon/README.md)（含一条待处理的授权说明）。
应用图标不支持深浅色自适应——`.icns` 无变体概念，appiconset 的 dark 图会被 `actool`
静默丢弃，macOS 26 的自适应要靠只有 GUI 的 Icon Composer。菜单栏图标不受影响，
它是单色 template 渲染，本来就跟随系统外观。

### 其它设计决定

**GUI 只是 CLI 的 JSON 消费者。** 起进程跑 `cloudot --json`，不复用 Rust 的 core：进程隔离，界面崩了动不到你的配置，而且这套 JSON 本来就是为三端共用设计的。任何一次改动操作之后一定重新拉 `status`，界面绝不自己推测新状态。

**`--force` 和 `--allow-secrets` 刻意不进 GUI。** 这两个开关真能丢数据或把凭据推进 git，误点的代价比在终端里敲错命令高得多。GUI 遇到这类错误时，会按错误分类把该敲的命令直接显示出来让你复制。其余破坏性操作（纳管、退出纳管、清理备份）走确认对话框。

**自动只读、手动写。** 同步永远要你点。

状态刷新靠 **FSEvents**：配置文件真的变了才刷，没变化零开销。原来是每 20 秒轮询一次，每小时白起 720 个进程——而轮询其实发现不了远端改动（`status` 读的是本地缓存的 `@{upstream}` ref，**不 fetch**），唯一能新发现的就是「本机改了配置」，而那件事文件系统会主动告诉我们。另留一天一次的兜底轮询，防 FSEvents 漏事件（休眠期间的变化、监视目录被整个移走）。

**必须同时监视 `store/files` 和每个纳管文件所在的目录**，这是实测结论：通过软链改配置只在 **store** 侧报事件（`~/.config` 一侧一个都没有）；软链被替换写入顶掉只在 **配置目录** 侧报。少看一处就漏掉一整类变化。另外改动类操作后有 3 秒抑制窗口，否则 `sync` 自己写的文件会让 watcher 再触发一轮。

**快捷键挂在按钮上而不是 `.commands`。** `LSUIElement` 应用没有自己的菜单栏，菜单项形式的快捷键不会生效；`Button` 上的 `.keyboardShortcut` 在窗口或面板获得焦点时就能用。⌘S 同步、⌘R 刷新、⌘Q 退出。

**不用 .xcodeproj，用 SwiftPM + 组装脚本。** `swift build` 在纯 CommandLineTools 下就能跑，整条链子在终端里可复现；想改界面时 `open Package.swift`，Xcode 直接当完整 IDE 用（Preview 也能用）。代价只是 Info.plist 和 bundle 结构要自己拼，就是 `make-app.sh` 那几十行。`cloudot` 二进制会打进 `.app/Contents/Resources/`，所以用户不必先装 CLI。

## 测试

```bash
cargo test              # 66 个 Rust 单元测试
./e2e.sh                # 40 项端到端断言
apps/Cloudot/test.sh    # Swift 测试（契约 + 菜单栏机器人不变量）
```

`e2e.sh` 在 `/tmp/cloudot-e2e` 下用假 `HOME` 模拟两台机器 + 一个 bare remote，完全不碰真实的 `~/.config` 和 `~/.cloudot`。覆盖：

- 首次纳管、跨机器落地、双向同步、unadopt 还原
- 软链被实体文件顶掉后的拒绝覆盖与 `--force` 恢复
- **回归**：跨机器 unadopt 后的悬空软链自愈；修不好时必须报警而不是装作没事
- **回归**：rebase 冲突自动回滚，实时配置不被冲突标记污染
- `add` 中途失败整体回滚，manifest 与 links.toml 均不被写脏
- 凭据门禁拦下、报告不回显凭据值、`--allow-secrets` 放行、`doctor` 持续报错
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

按优先级：其余应用的 adopter（zsh / zed 等）· secret 扫描与 age 加密 · daemon（后台自动同步）· agent skill 与 MCP · 公证 DMG 与 homebrew tap。
