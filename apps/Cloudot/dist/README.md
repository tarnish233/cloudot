# 分发

两条渠道，都从 GitHub Release 的 DMG 出发：

| 渠道 | 命令 | 装到哪 |
|---|---|---|
| Homebrew Cask | `brew install --cask tarnish233/tap/cloudot` | `/Applications/Cloudot.app` + `cloudot` 链到 PATH |
| 直接下载 | 从 [Releases](https://github.com/tarnish233/cloudot/releases) 拿 DMG | 自己拖到「应用程序」 |

**不再发 zip。** DMG 首次安装体验更好（拖拽到 Applications），Cask 也只认一种格式更省事。
已发布的 v0.1.0 / v0.2.0 里的 zip 不动。

## 发布一个新版本

```bash
# 1. 改版本号 —— 只改这一处，make-app.sh 会从这里读
vim Cargo.toml           # [workspace.package] version = "0.3.0"

# 2. 打包
cd apps/Cloudot && MAKE_DMG=1 ./make-app.sh

# 3. 发布（脚本最后会把这行命令连同实际路径打印出来）
gh release create v0.3.0 \
  build/Cloudot-0.3.0.dmg build/Cloudot-0.3.0.dmg.sha256 \
  --title 'Cloudot v0.3.0' --generate-notes
```

`make-app.sh` 会顺带产出 `.dmg.sha256`。**那个文件必须一起传** —— homebrew-tap 的
更新流程是从 release 资产里直接读它，而不是下载整个 DMG 再自己算
（和 tap 里 gitpic 那套 workflow 一致）。

DMG 每次构建的 sha256 都不一样（打包时间戳会进映像），所以校验和只在发布那一刻算一次，
别指望能复现。

## Homebrew Cask

[`dist/cloudot-cask.rb`](dist/cloudot-cask.rb) 是 Cask 定义的**参考副本**，
真正生效的那份在 [tarnish233/homebrew-tap](https://github.com/tarnish233/homebrew-tap)
的 `Casks/cloudot.rb`。改动要同步两边。

几个当时踩过的点：

- **`binary` 指向 .app 内的 CLI**，不单独打包一份。GUI 本来就优先用 bundle 里那份
  （见 `CloudotCLI.locate`），再装一份到 `bin/` 会出现两个版本各自更新、对不上的情况。
- **`depends_on macos: :sequoia`**，不要写 `">= :sequoia"`。字符串比较那种写法已经废弃，
  `brew audit` 不报，但安装时会打警告。
- **caveats 必须提 CLI 也受隔离属性影响。** 链到 PATH 的那个二进制就在 .app 里面，
  不跑 `xattr -dr` 的话 `cloudot --version` 会被系统直接 SIGKILL（退出码 137）——
  只说 GUI 打不开会让人以为命令行能用。

验证 Cask 改动（要放进真实 tap 才能审，`brew audit <路径>` 已被禁用）：

```bash
TAP="$(brew --repository)/Library/Taps/tarnish233/homebrew-tap"
mkdir -p "$TAP/Casks" && cp dist/cloudot-cask.rb "$TAP/Casks/cloudot.rb"
brew audit --cask tarnish233/tap/cloudot     # 零输出就是通过
brew install --cask tarnish233/tap/cloudot   # audit 只查定义，装一次才知道行不行
```

验完记得把 `$TAP/Casks/cloudot.rb` 删掉，别把本地实验留在 tap 里。

## 关于签名

目前是 adhoc 签名，没有 Apple Developer ID，所以用户装完必须跑一次：

```bash
xattr -dr com.apple.quarantine /Applications/Cloudot.app
```

这一行同时解决 GUI 打不开和 CLI 被杀两个问题。有证书之后应该改成正式签名 + 公证
（`codesign --sign "Developer ID Application: ..."` 再 `notarytool submit`），
那样这条 caveats 就能删了。

## GUI 自更新

**不用 Sparkle。** 评估过：它的安全模型靠 EdDSA 签名，没有 Apple 证书也能工作，但要求
`make-app.sh` 自己把 Sparkle.framework 拷进 bundle 并单独签名（Xcode 会代劳，这个项目是
手工组装的），还要维护 appcast。自更新逻辑本身就这么点，自己写反而少一层依赖。

GUI **只有一条路径：下载 DMG 替换自己**。不判断是不是 Homebrew 装的、不在 GUI 里跑 brew。
Homebrew 用户想用 brew 升级就自己在终端跑 —— 那是 Homebrew 渠道自身的事。

### 流程

1. **查版本**：`HEAD https://github.com/tarnish233/cloudot/releases/latest`，不跟随 302，
   从 `Location` 头解析 tag。**不用 GitHub API** —— 匿名限额 60 次/小时按 IP 共享，
   NAT 后面可能一次都用不上。
2. **下载**：URLSession 拿 `Cloudot-<版本>.dmg` 和配套的 `.dmg.sha256`，`shasum -a 256` 比对。
3. **安装**（任何一步失败旧版都还在）：
   1. `hdiutil attach -readonly -nobrowse`
   2. `ditto` 到同目录的 `.Cloudot.app.new`
   3. `hdiutil detach`（`defer` 保证执行）
   4. 校验新 bundle 可执行、Info.plist 版本号对得上
   5. `mv` 当前 → `.Cloudot.app.old`
   6. `mv` 新版 → 当前路径；失败则把 old 挪回来
   7. 删 old
4. **重启**：装完**不**自动重启，界面出现「重启以启用 X」按钮；点了才
   `sh -c "sleep 1; open '<path>'"` 然后 `terminate`。

运行中的 `.app` 被 `mv` 走是安全的（inode 已打开，进程继续跑）。网络下来的 release 资产
**不带** `com.apple.quarantine`，替换后用户不必再跑 `xattr -dr`。

### Homebrew 用户被 GUI 抢先更新的后果

Caskroom 的版本记录会暂时落后，但下次 `brew upgrade --cask cloudot` 只是重装成同样的新版，
**不会退回旧版**（实测过）。确认对话框的文案里提了一句。

### 本地验收（不动线上 release）

```bash
# 1. 打一份当前版的 DMG，改成 9.9.9 放到假源里
cd apps/Cloudot && MAKE_DMG=1 ./make-app.sh
FEED=/tmp/cloudot-update-feed
mkdir -p "$FEED/releases/download/v9.9.9"

STAGE=$(mktemp -d)
cp -R build/Cloudot.app "$STAGE/Cloudot.app"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString 9.9.9" \
  "$STAGE/Cloudot.app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion 9.9.9" \
  "$STAGE/Cloudot.app/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$STAGE/Cloudot.app" >/dev/null

DMG_STAGE=$(mktemp -d)/pkg
mkdir -p "$DMG_STAGE"
cp -R "$STAGE/Cloudot.app" "$DMG_STAGE/"
hdiutil create -volname "cloudot 9.9.9" -srcfolder "$DMG_STAGE" -ov -format UDZO -quiet \
  "$FEED/releases/download/v9.9.9/Cloudot-9.9.9.dmg"
shasum -a 256 "$FEED/releases/download/v9.9.9/Cloudot-9.9.9.dmg" \
  | awk '{print $1 "  Cloudot-9.9.9.dmg"}' \
  > "$FEED/releases/download/v9.9.9/Cloudot-9.9.9.dmg.sha256"

# 2. 起一个会回 302 的假 GitHub（/releases/latest → /releases/tag/v9.9.9）
#    最简单的写法见 UpdaterTests 里的 LocalHTTPServer；或者：
python3 -m http.server 8765 --directory "$FEED"   # 还得自己处理 /releases/latest 的 302

# 3a. 自动化（推荐）—— 下载/校验/挂载/替换全覆盖，含坏校验和与 404
CLOUDOT_UPDATE_E2E=1 CLOUDOT_UPDATE_FEED=http://127.0.0.1:8765 \
  ./test.sh --filter UpdaterTests

# 3b. 真 App 手点一遍
defaults write com.tarnish233.cloudot updateFeedURL http://127.0.0.1:8765
open build/Cloudot.app
# 关于页 / 菜单栏页脚应出现「更新到 9.9.9」→ 确认 → 装完出现重启按钮
defaults delete com.tarnish233.cloudot updateFeedURL   # 验完清掉
```

覆盖开关两层，环境变量优先：`CLOUDOT_UPDATE_FEED`（测试用）>
`defaults write … updateFeedURL`（真 App 用）> GitHub 默认值。

**从 v0.3.0 起 Release 资产是 DMG。** 更早的 v0.1.0 / v0.2.0 只有 zip，
自更新对它们会走到「安装包还没上传」——那是预期行为，不是 bug。
