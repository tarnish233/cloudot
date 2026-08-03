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

glyph 用的是 SF Symbol `arrow.triangle.2.circlepath` —— **和菜单栏「同步中」是同一个符号**，
不是「风格相似」而是同一份矢量数据。改任何一边之前先看
[`IconState+Symbol.swift`](../Sources/Cloudot/IconState%2BSymbol.swift)，两边要一起改。

底面是渐变 squircle，材质靠六层叠出来（外投影、三段底色渐变、球面高光、底部冷色反射、
内侧倒角、符号自身的投影与柔光）。几条踩过的坑都写在脚本注释里了，动参数前先读：

- 投影与底色**必须分两层画**，同层会在抗锯齿边缘漏出填充色，像描歪的边框。
- 投影色必须是**中性黑**，用深蓝会在外沿留一圈可见的蓝晕。
- 符号柔光**不能用 `.plusLighter`**，加法混合在蓝底上会把通道推到饱和，
  出来是霓虹描边而不是光晕。

16 / 32px 下都实测过。换图前务必缩到 16px 看一眼再定。

早先有两版已经弃用：一版派生自 macosicons 上 @Luca K 的「Smart Backup」，
`design-icons.swift` 是它的渲染器，**构图取自那件作品，不能用于分发**，留着仅作记录；
另一版是「两个节点 + 缺口环」的软链意象，因为和菜单栏图标不同源而换掉了。

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

真正每天看的是菜单栏图标；它是单色 template 渲染，本来就跟随系统外观。

## 脚本

- `make-icon.swift` —— **当前在用的渲染器**，输出 `AppIcon.png`。见上面的「授权」。

- `design-icons.swift` —— 更早一版的渲染器，输出到 `/tmp/cloudot-icon/`。
  它的构图派生自第三方素材，**不能用于分发**，留着仅作记录。
  外形都是**超椭圆**而不是圆弧圆角（macOS 的图标形状是 squircle，小尺寸下看得出差别）。

- `search-macosicons.sh` —— 用 macosicons API 搜候选，key 从 `MACOSICONS_API_KEY` 环境变量读。

  ⚠️ 那个库里下载量最高的几乎全是现有品牌的同人重绘（iCloud、Adobe Creative Cloud、
  Apple Shortcuts、系统设置齿轮、GitHub、LinkedIn、希捷），不能拿来当自己应用的图标。
  剩下的多是**文件夹**造型，应用用文件夹图标会在 Finder 里被误认成文件夹。

  该站现在还有一组 `/api/v1/editor/*` 端点，其中 `POST /editor/generate` 能用 AI
  出图，`/editor/icns`、`/editor/iconset`、`/editor/pack` 能直接产出 macOS 图标格式。
  评估过，**没有采用** —— 服务条款是「仅限非商业用途 + 必须署名 macOSicons.com」，
  为了一张图标把这层约束装回来不划算，自绘完全够用。真要用的话注意配额很紧：
  免费档 50 次/月，而生成一张图记 25 credits。
