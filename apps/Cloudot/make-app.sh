#!/usr/bin/env bash
# 把 SwiftPM 产物组装成一个真正的 .app。
#
# 为什么不用 .xcodeproj：SwiftPM 能在纯 CommandLineTools 下构建，整条链子可以在
# 终端里复现；想用 Xcode 改界面时 `open Package.swift` 就有完整 IDE 和 Preview。
# 代价只是 Info.plist 和 bundle 结构要自己拼 —— 就是这个脚本。
set -euo pipefail

CONFIG="${1:-release}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
APP="$HERE/build/Cloudot.app"

BUNDLE_ID="com.tarnish233.cloudot"
VERSION="0.2.0"

echo "==> 构建 GUI（${CONFIG}）"
swift build --package-path "$HERE" -c "$CONFIG"
BIN="$(swift build --package-path "$HERE" -c "$CONFIG" --show-bin-path)/Cloudot"

echo "==> 构建 cloudot CLI（会打进 .app，用户不必先装 CLI）"
cargo build --release --manifest-path "$REPO/Cargo.toml" --quiet

echo "==> 组装 $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/Cloudot"
cp "$REPO/target/release/cloudot" "$APP/Contents/Resources/cloudot"

# 应用图标：有 Icon/AppIcon.png 就生成 .icns 装进去。
# LSUIElement 应用没有 Dock 图标，但 Finder、Spotlight 和 DMG 里仍然要看它。
ICON_SRC="$HERE/Icon/AppIcon.png"
ICON_KEY=""
if [ -f "$ICON_SRC" ]; then
  echo "==> 生成 AppIcon.icns"
  ICONSET="$(mktemp -d)/AppIcon.iconset"
  mkdir -p "$ICONSET"
  for spec in "16 16x16" "32 16x16@2x" "32 32x32" "64 32x32@2x" \
              "128 128x128" "256 128x128@2x" "256 256x256" "512 256x256@2x" \
              "512 512x512" "1024 512x512@2x"; do
    set -- $spec
    sips -z "$1" "$1" "$ICON_SRC" --out "$ICONSET/icon_$2.png" >/dev/null 2>&1
  done
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
  ICON_KEY='  <key>CFBundleIconFile</key>          <string>AppIcon</string>'
else
  echo "==> 跳过图标（放一张 1024px 的 Icon/AppIcon.png 就会自动装上）"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>              <string>cloudot</string>
  <key>CFBundleDisplayName</key>       <string>cloudot</string>
  <key>CFBundleExecutable</key>        <string>Cloudot</string>
  <key>CFBundleIdentifier</key>        <string>$BUNDLE_ID</string>
  <key>CFBundlePackageType</key>       <string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key>           <string>$VERSION</string>
$ICON_KEY
  <key>LSMinimumSystemVersion</key>    <string>15.0</string>
  <key>NSHumanReadableCopyright</key>  <string>cloudot</string>
  <!-- 纯菜单栏工具：不占 Dock、不进 Cmd-Tab。
       代价是没有 App 自己的菜单栏，所以快捷键挂在按钮上而不是 .commands。
       主窗口从菜单栏面板的「设置」进入。 -->
  <key>LSUIElement</key>               <true/>
  <key>NSSupportsAutomaticTermination</key> <false/>
  <key>NSSupportsSuddenTermination</key>    <false/>
</dict>
</plist>
PLIST

# 没有开发者证书也能本地跑：adhoc 签名就够，分发时再换成正式证书 + 公证
echo "==> adhoc 签名"
codesign --force --sign - --timestamp=none "$APP" >/dev/null 2>&1 \
  || echo "   （签名跳过，本地运行不受影响）"

# 自检：确认打进去的 CLI 真的认识所有内置应用。
#
# GUI 优先用 .app 里自带的这份 CLI（见 CloudotCLI.locate），所以只改 Rust 侧、
# 忘了重新打包时，界面会一直显示旧行为 —— 症状是「明明加了 adopter，界面里却
# 没有那个应用」，看起来像前端 bug，很难查。这里直接把不一致说出来。
BUNDLED_APPS="$("$APP/Contents/Resources/cloudot" --json apps 2>/dev/null \
  | grep -o '"id"[^,]*' | wc -l | tr -d ' ')"
SOURCE_APPS="$(ls "$REPO/adopters"/*.toml 2>/dev/null | wc -l | tr -d ' ')"
if [ "$BUNDLED_APPS" != "$SOURCE_APPS" ]; then
  echo
  echo "⚠️  打包的 CLI 认识 ${BUNDLED_APPS} 个应用，但 adopters/ 下有 ${SOURCE_APPS} 个定义。"
  echo "   新增 adopter 之后要在 adopter.rs 的 BUILTIN 里登记一行才会生效。"
  exit 1
fi
echo "==> 自检通过：打包的 CLI 认识 ${BUNDLED_APPS} 个应用"

# 顺带提醒 PATH 上那份是否也该更新 —— 它是终端里用的，和 .app 内那份互相独立
INSTALLED="$(command -v cloudot || true)"
if [ -n "$INSTALLED" ] && ! cmp -s "$INSTALLED" "$APP/Contents/Resources/cloudot"; then
  echo "   提示：$INSTALLED 与刚打包的版本不同，终端里要用新版就跑"
  echo "        cargo install --force --path crates/cloudot-cli"
fi

echo
echo "完成：$APP"
echo "运行：open '$APP'"
