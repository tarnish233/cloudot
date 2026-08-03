# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目

cloudot：macOS 配置同步器，用 git 在多台 Mac 之间同步 dotfiles。Rust 核心 + CLI，SwiftUI 菜单栏 GUI。当前纳管 ghostty / fish / karabiner / gitpic。

**[README.md](README.md) 记录了完整的设计决策与其理由**，动手改之前先读它 —— 下面很多约束在那里有更详细的论证。

## 常用命令

```bash
# Rust
cargo build --release
cargo test                                    # 66 个单元测试
cargo test --package cloudot-core secrets     # 按模块名过滤
cargo test link::tests::adopt_links_from_store_when_local_absent   # 跑单个测试
cargo install --path crates/cloudot-cli       # 装到 ~/.cargo/bin

./e2e.sh                                      # 40 项端到端断言（假 HOME，不碰真实配置）

# Swift GUI（在 apps/Cloudot/ 下）
./make-app.sh                                 # 构建 GUI + CLI，组装 build/Cloudot.app
MAKE_DMG=1 ./make-app.sh                      # 顺带打发布用的 DMG + .sha256
./test.sh                                     # 64 个测试（含 5 个默认跳过：2 截图 + 3 更新 e2e）
./test.sh --filter ContractTests              # 跑单个 test class
swift build                                   # 只编译，不组装 bundle
./demo-states.sh                              # 沙盒里走遍菜单栏各状态
```

`./test.sh` 借 `DEVELOPER_DIR` 指向 Xcode.app —— XCTest 只在 Xcode 里，而 `swift build` / `make-app.sh` 用纯 CommandLineTools 就能跑。

**改完 Rust 侧要重新 `cargo install --force --path crates/cloudot-cli`**，否则 GUI 仍在调用 `~/.cargo/bin` 里的旧二进制。CLI 与 GUI 的契约不匹配时，界面只会显示「找不到 cloudot / 输出异常」，很难一眼看出是版本问题。

## 架构

### 三端共用一套契约

```
crates/cloudot-core/    领域逻辑，唯一的真相来源
crates/cloudot-cli/     core 的第一个消费者；每个命令都有 --json
apps/Cloudot/           SwiftUI GUI，起进程跑 `cloudot --json` 消费 JSON
```

GUI **不链接** Rust core，而是 spawn CLI 进程。进程隔离，界面崩了动不到用户配置。新能力一律先进 core + CLI，不能出现「只有 GUI 能做的事」。

JSON 统一信封，消费方只需要一条解码路径：

```json
{ "schema": "cloudot.sync/v1",  "ok": true,  "result": { … } }
{ "schema": "cloudot.error/v1", "ok": false, "result": { "kind": "locked", "summary": …, "message": … } }
```

- schema 常量在各模块里（`ops.rs` 的 `SYNC_SCHEMA` 等）。加字段兼容，改语义要升版本。
- 错误带机器可读的 `kind`（见 `errors.rs` 的 `ErrorKind`）。内部一律 `anyhow`，分类塞进错误链、在边界 `downcast` 取回 —— 别为了分类把代码库改成自定义错误类型，也别让界面去 grep 中文错误信息。
- **`status` 未 init 时返回成功信封 + `initialized: false`**，不要抛 `NotInitialized`——那是写路径的事。GUI 靠这个字段画 Setup 引导，而不是红 banner。
- **`pull_conflict` 的错误信封带 `conflict` 字段**（`ConflictReport`：文件列表 + 每文件 diff）。GUI 弹选边面板；终端用 `cloudot resolve --theirs|--ours`。冲突时仍先 `rebase --abort`。
- **`doctor` 有 error 级检查项时以非零码退出，但输出合法的成功信封。** 消费方必须先解信封再看退出码。`CloudotCLI.swift` 就是这么做的，改那里要留住这个顺序。

### 安全关键路径

`link.rs` / `links.rs` 改动要格外小心，它们直接决定用户配置会不会丢：

- **绝不静默覆盖。** 任何破坏本地文件的操作先备份到 `~/.cloudot/backups/<时间戳>/`。`apply` 遇到本地实体文件默认拒绝（那份可能比 store 新），要覆盖必须显式 `--force`。
- **修不好时不删任何东西。** 悬空软链自愈失败时，宁可留个坏链让 `doctor` 继续报警，也不能抹掉用户唯一的线索。
- **`add` 要么整体成功要么整体回滚。** 回滚必须按当时实际做了什么分别处理 —— `LinkedFromStore` 那种情况下 store 里的内容是**别的机器**放的，撤销时绝对不能删。
- **manifest 是共享状态（进 git），`links.toml` 是本机状态（不进 git），两者会分叉。** 别的机器 unadopt 之后本机会留下悬空软链，只看 manifest 发现不了，所以要靠 `links.toml` 对账。
- **凭据扫描结果只报路径、行号和原因，绝不包含命中的值** —— 否则报告自己就成了泄漏渠道。

### 几个容易踩的约束

- **`~/.cloudot/store` 是 git 工作树，同时是所有软链的目标，所以它永远不能停在中间状态。** `git pull --rebase` 冲突残留 `UU` 时，App 立刻会读到塞满 `<<<<<<< HEAD` 的配置。所以冲突时自动 `rebase --abort`。
- **store 必须在本地**，不能放 iCloud 之类按需下载的存储 —— 文件被驱逐成占位符时软链会读到空内容。
- **flock 按「打开的文件描述」生效，同进程再 open 一次也会冲突。** 所以 `sync` 内部走不加锁的 `apply_inner`，`lock.rs` 有测试钉住这个约束。
- **单元测试用 `Layout::with_home()`，不要用 `CLOUDOT_HOME` 环境变量** —— 环境变量是进程全局的，并行测试会互相干扰。`CLOUDOT_HOME` / `CLOUDOT_ROOT` 只给 e2e 和 demo 脚本做隔离用。
- **manifest 里路径一律存 `~/` 形式**，换机器换用户名都能直接 apply。
- **加应用 = 往 `adopters/` 放一个 TOML + 在 `adopter.rs` 的 `BUILTIN` 登记一行**（`include_str!` 编译进二进制）。逻辑代码不用改。用户放在 `~/.cloudot/adopters/` 的同 id 定义会覆盖内置。

## SwiftUI GUI

`LSUIElement` 纯菜单栏应用（不占 Dock、不进 Cmd-Tab），macOS 15+。**一个类型一个文件。**

### 界面之外的实现约束

这些都是踩过的坑，改之前先看懂：

- **不用 `MenuBarExtra`，自己持有 `NSStatusItem`**（`MenuBarController.swift`）。**注意原来的理由已经作废** —— 那时是「动画到不了状态栏，必须逐帧换图」，而图标现在是静态 SF Symbol，别再拿这条当依据。仍然自建是因为还有三件事非拿到 status item 不可：结果反馈那 0.7 秒的临时换图要能精确控制何时换回；tooltip 与辅助功能标签跟着 `headline` 走；点击时顺手拉一次 status（`MenuBarExtra` 没有点击回调）。迁移过去收益为零，不动。
- **破坏性操作的确认框只由 `MenuBarController` 用 AppKit `NSAlert` 呈现一次**（`PendingActionPresenter`）。不要在 `MenuBarPanel` 和 `MainWindow` 上各挂一份 SwiftUI confirm——主窗 `isReleasedWhenClosed = false`，关窗后仍能再弹一个，锚不到 status item 就落到屏幕左下角。冲突 sheet 只挂主窗口；菜单栏 sync 撞车时由 Controller 在冲突**新出现**时拉起主窗口。
- **那个 0.7 秒的回退 `Timer` 必须 `RunLoop.main.add(t, forMode: .common)`。** 默认模式下面板/菜单打开时 run loop 进入 tracking 模式，timer 直接停摆 —— 成功/失败图标会**永久**卡在菜单栏上。
- **菜单栏图标是静态 SF Symbol，状态靠形状区分**（`IconState+Symbol.swift`）。菜单栏是单色 template 渲染，颜色不生效。**常态用的就是 App 图标那个 `arrow.triangle.2.circlepath`** —— 同一个符号，改一边要同时改 `Icon/make-icon.swift`，有测试钉住。`healthy` / `refreshing` / `syncing` **刻意共用**它：静态图区分不了「在跑」，而那件事由面板里的 `ProgressView` 和 tooltip 负责说。真正会变形的只有「要你动手」（pending）和「出事了」（broken / unavailable）。
- **菜单栏项用 `squareLength`，符号字号在 `CloudotTheme.menuBarSymbolPointSize`。** 调大之前先看 `icloud.slash` —— 它是这批里最宽的，会第一个被裁。`MenuBarIconTests` 有一条专门量这个。
- **图标必须是 template image**（`isTemplate = true`）。`NSImage(systemSymbolName:)` 默认就是 template，但 **`withSymbolConfiguration` 返回的是另一个实例**，这个属性不保证跟过来 —— `MenuBarIcon` 里显式写死，别指望默认值。固定色图在浅色菜单栏上会隐身。
- **侧栏行必须显式写 `.tag(item)`，且不能挂 `.badge()`。** 漏了 tag 或加了 badge 都会让 `.tag()` 落不到 SwiftUI 用来匹配选中项的那一层，`NSTableView.selectedRow` 一直是 -1 —— 表现是整个侧栏点了没反应，而且照样编译通过、界面看着正常。
- **`NavigationSplitView` 必须是根视图**，套进 `VStack` 会丢掉平台对侧栏的处理（侧栏退化成浮空小卡且点不动）。
- **快捷键挂在 `Button` 的 `.keyboardShortcut` 上，不用 `.commands`** —— `LSUIElement` 应用没有自己的菜单栏，菜单项形式的快捷键不生效。⌘S 同步、⌘R 刷新、⌘Q 退出。
- **主窗口是自建 `NSWindow` + `NSHostingController`，所以 `.toolbar` 不生效**（不会桥接到 NSWindow 的工具栏），工具条要自己排。同理 `@Environment(\.openWindow)` 在 `NSHostingController` 里是空的 —— 打开主窗口的回调由 `MenuBarController` 注入。
- **配色取系统语义色**：强调色读 `NSColor.controlAccentColor`（跟随「系统设置 → 外观 → 强调色」），尺寸常量集中在 `CloudotTheme`。别自己搭圆角灰底卡片 —— 系统的 `Form` / `GroupBox` / `LabeledContent` / `ContentUnavailableView` 自带正确的行高、分隔线缩进，还跟随动态字体。
- **App 里现在没有任何动画，所以也没有「减少动态效果」判断。** 结果反馈那 0.7 秒的换图是离散的状态指示 —— 没有位移、没有插值、没有缓动，`accessibilityDisplayShouldReduceMotion` 管不到它（而且原来开着该开关时走的就是这条路径）。**以后要是加回任何动画，这个判断必须一起加回来**，`MenuBarController.showPulse` 上留了提醒。
- **`--force` 和 `--allow-secrets` 刻意不进 GUI。** 这两个开关真能丢数据或把凭据推进 git。GUI 按错误分类把该敲的命令显示出来让用户复制。其余破坏性操作（纳管/退出纳管/清理备份/安装更新）走确认对话框。
- **FSEvents 必须同时监视 `store/files` 和每个纳管文件所在的目录。** 实测：通过软链改配置只在 **store** 侧报事件（`~/.config` 一侧一个都没有）；软链被替换写入顶掉只在 **配置目录** 侧报。少监视一处就会漏掉一整类变化。见 `ConfigWatcherTests`。
- **改动类操作后有 3 秒抑制窗口**（`selfWriteGrace`）：`sync` / `apply` / `add` 会真写 store，FSEvents 的回声会再触发一轮刷新，而那些操作本来就会自己刷。`status` 是纯只读，实测不产生事件。
- **任何改动操作之后一定重新拉 `status`**，界面绝不自己推测新状态。
- **自更新走下载 DMG 替换，不用 Sparkle。** 版本发现靠 `releases/latest` 的 302（**不用 GitHub API**，匿名限额按 IP 共享会直接不可用）。替换顺序不能反：先 `ditto` 到 `.Cloudot.app.new` 并校验，再 `mv` 旧的让位 —— 反过来中途失败就没 app 了。装完**不**自动重启，界面问。**目标路径取 `Bundle.main.bundleURL`，不硬编码 `/Applications`**；但 `swift test` 环境下这个值指向 toolchain，路径相关逻辑只能靠 `CLOUDOT_UPDATE_E2E=1` 的真安装测试或手点验收。`isCheckingForUpdate` **刻意不进 `isBusy`**，否则后台查更新会把同步/刷新一起变灰。详情与本地假源验收步骤见 [dist/README.md](apps/Cloudot/dist/README.md)。

### 应用图标

不支持深浅色自适应：`.icns` 无变体概念，appiconset 的 dark 图会被 `actool` 静默丢弃，macOS 26 的自适应要靠只有 GUI 的 Icon Composer。菜单栏图标不受影响（单色 template）。见 [Icon/README.md](apps/Cloudot/Icon/README.md)。

### 分发

Homebrew Cask + GitHub Release 的 DMG（不再发 zip），一条 `brew install --cask` 同时装 GUI 和 CLI —— `binary` 直接指向 .app 内那份，不另打包。**目前是 adhoc 签名**，用户装完必须 `xattr -dr com.apple.quarantine`，否则 GUI 打不开、`cloudot` 命令也会被 SIGKILL。发版步骤、Cask 的几个坑、GUI 自更新机制见 [dist/README.md](apps/Cloudot/dist/README.md)。

## 测试约定

- **Swift fixture 是从真实 CLI 抓下来的输出，不是手写的** —— 手写只能验证「我以为的格式」。重抓：`cloudot --json status > apps/Cloudot/Tests/CloudotTests/Fixtures/status.json`。
- 契约测试里有两条**方向相反**的断言，都是刻意的：未知的**错误分类**降级成 `other`（新分类只影响引导文案），未知的**链接状态**则整体解码失败（新状态可能意味着新的损坏形态，宁可报「输出异常」也不要静默降级成一个看起来正常的值）。
- `e2e.sh` 在 `/tmp/cloudot-e2e` 下用假 HOME 模拟两台机器 + 一个 bare remote。里面有几条是**回归测试**（跨机器 unadopt 后的悬空软链自愈、rebase 冲突自动回滚 + `resolve` 选边、未 init 的 status 信封、`add` 中途失败整体回滚），改相关逻辑时要保证它们仍然通过。

## 注意

- **界面改动很难自动验证**，布局错了照样编译通过、测试全绿。`UIShotTests` 就是干这个的：把视图挂进屏幕外的真实 `NSWindow`、跑一会儿 run loop 再 `cacheDisplay` 成 PNG，默认跳过，要显式开：

  ```bash
  CLOUDOT_UI_SHOT=1 CLOUDOT_HOME=/tmp/某个沙盒 ./test.sh --filter UIShotTests   # 出图到 /tmp/uishot/
  ```

  截图**必须给个装好数据的沙盒 HOME**，空态下看不出真实排版。已经靠它抓到过 `LabeledContent` 在 `GroupBox` 里居中、`chart.bar` 在小字号糊成一团。但它只能查排版，**点击是否真的有反应仍然要人工确认**。
- **自更新的完整安装路径默认也跳过**，要本地假源：

  ```bash
  # 假源怎么搭见 apps/Cloudot/dist/README.md「本地验收自更新」
  CLOUDOT_UPDATE_E2E=1 CLOUDOT_UPDATE_FEED=http://127.0.0.1:8765 \
    ./test.sh --filter UpdaterTests
  ```

  覆盖下载、sha256 校验、挂载替换、资产 404、校验和错误。semver / URL 拼装 / 幂等检查是常规单测，不依赖假源。
- shell 脚本里变量紧跟中文全角标点时**必须写 `${VAR}`**。系统自带的 bash 3.2 会把 `）` 的首字节当成变量名的一部分，`"$CONFIG）"` 会报 `unbound variable`。
