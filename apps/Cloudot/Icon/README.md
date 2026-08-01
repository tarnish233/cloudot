# 图标

## 应用图标

`AppIcon.png`（1024×1024）。`make-app.sh` 会用 `sips` + `iconutil` 生成完整尺寸集的
`.icns` 装进 bundle。LSUIElement 应用没有 Dock 图标，但 Finder、Spotlight 和 DMG 里
仍然要看它。

### 授权

**原创设计，无第三方素材。** 全部由 [make-icon.swift](make-icon.swift) 用 CoreGraphics
绘制，重新生成：

```bash
cd apps/Cloudot/Icon && swift make-icon.swift
```

构图取的是 cloudot 自己的核心意象 —— **软链**：两个节点（本机 / 共享 store）由一条
留缺口的环形链路相连，缺口处用箭头收尾表示单向落地。整体轮廓恰好像个「c」。
32px 下验证过仍可辨认。

早先的版本派生自 macosicons 上 @Luca K 的「Smart Backup」，那站没有明确许可，
已在开源前替换掉。`design-icons.swift` 是那一版的渲染器，**它的构图同样取自那件作品，
所以也不能用于分发** —— 留着仅作记录。

### 关于深浅色自适应

做不到，实测确认过：

- `.icns` 是单一外观格式，没有变体概念。
- 资源目录（`AppIcon.appiconset`）加
  `"appearances":[{"appearance":"luminosity","value":"dark"}]` 之后，`actool`
  **编译通过但会警告 `has 10 unassigned children`，把 dark 那批图全丢掉** ——
  macOS 的 appiconset 不支持深浅色变体（iOS 的支持）。
- macOS 26 确实支持（Liquid Glass 的 light/dark/clear/tinted），但要靠
  **Icon Composer** 产出 `.icon` 文件，而它**只有 GUI**：`iconutil` 只认 icns/iconset，
  `xcrun -f icontool` 找不到，actool 二进制里也搜不到 `.icon` 相关字串。

真正每天看的是菜单栏机器人；它是单色 template 渲染，本来就跟随系统外观。

## 脚本

- `design-icons.swift` —— 原创图标渲染器（不依赖任何第三方素材），输出到 `/tmp/cloudot-icon/`。
  留着是为了应对上面那条授权问题：需要换图时直接用。
  glyph 仍使用 SF Symbol `arrow.triangle.2.circlepath`；菜单栏机器人是独立的 18pt 矢量设计，
  外形是**超椭圆**而不是圆弧圆角（macOS 的图标形状是 squircle，小尺寸下看得出差别）。

- `search-macosicons.sh` —— 用 macosicons API 搜候选，key 从 `MACOSICONS_API_KEY` 环境变量读。

  ⚠️ 那个库里下载量最高的几乎全是现有品牌的同人重绘（iCloud、Adobe Creative Cloud、
  Apple Shortcuts、系统设置齿轮、GitHub、LinkedIn、希捷），不能拿来当自己应用的图标。
  剩下的多是**文件夹**造型，应用用文件夹图标会在 Finder 里被误认成文件夹。
