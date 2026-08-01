#!/usr/bin/env bash
# 跑 Swift 侧的契约测试。
#
# `swift build` 用纯 CommandLineTools 就能跑，但 XCTest 只在 Xcode.app 里，
# 所以测试得借 Xcode 的工具链。用 DEVELOPER_DIR 覆盖，不必 sudo xcode-select。
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
XCODE=/Applications/Xcode.app/Contents/Developer

if [ ! -d "$XCODE" ]; then
  echo "找不到 Xcode.app —— Swift 契约测试需要它提供 XCTest。"
  echo "（GUI 本身用 ./make-app.sh 构建，只需要 CommandLineTools。）"
  exit 1
fi

DEVELOPER_DIR="$XCODE" swift test --package-path "$HERE" "$@"
