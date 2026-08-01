#!/usr/bin/env bash
# 走遍菜单栏小机器人的全部动效。
#
# 全程在沙盒 HOME（/tmp/cloudot-demo）里操作，**不碰你真实的 ~/.cloudot 和
# ~/.config**。App 会带着 CLOUDOT_HOME 启动，它 spawn 的 cloudot 继承这个变量，
# 所以所有命令都作用在沙盒上。
#
# 用法：./demo-states.sh
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
CLI="$REPO/target/release/cloudot"
APP="$HERE/build/Cloudot.app/Contents/MacOS/Cloudot"
DEMO=/tmp/cloudot-demo
LOG=/tmp/cloudot-demo.log

c()  { printf '\n\033[1;36m══ %s\033[0m\n' "$1"; }
say(){ printf '   %s\n' "$1"; }
look(){ printf '\033[1;33m   👀 看菜单栏：%s\033[0m\n' "$1"; }
wait_key(){ printf '\033[2m   （回车继续）\033[0m'; read -r _; }

sandbox() { CLOUDOT_HOME="$DEMO" "$CLI" "$@"; }

[ -x "$CLI" ] || { echo "先构建 CLI：cargo build --release"; exit 1; }
[ -x "$APP" ] || { echo "先构建 App：./make-app.sh"; exit 1; }

c "准备沙盒"
pkill -f 'Cloudot.app/Contents/MacOS/Cloudot' 2>/dev/null
sleep 1
rm -rf "$DEMO"; mkdir -p "$DEMO/.config/ghostty"
printf 'theme = dark\nfont-size = 13\n' > "$DEMO/.config/ghostty/config"
git init --bare -q -b main "$DEMO/remote.git"
sandbox init --device demo --remote "$DEMO/remote.git" >/dev/null
sandbox add ghostty >/dev/null
sandbox sync >/dev/null
say "沙盒：${DEMO}（真实配置未被触碰）"

# 给 status / sync 加延迟，否则刷新和同步动效一闪而过看不清。
cat > /tmp/cloudot-demo-slow.sh <<SH
#!/bin/bash
for a in "\$@"; do
  case "\$a" in
    status) sleep 2; break ;;
    sync) sleep 6; break ;;
  esac
done
exec "$CLI" "\$@"
SH
chmod +x /tmp/cloudot-demo-slow.sh
defaults write com.tarnish233.cloudot cloudotBinaryPath /tmp/cloudot-demo-slow.sh

CLOUDOT_HOME="$DEMO" CLOUDOT_DIAG=1 "$APP" > "$LOG" 2>&1 &
sleep 3
say "App 已启动（诊断日志：${LOG}）"

c "1/8 · 正常"
look "机器人安静待机，约 4 秒自然眨一次眼"
wait_key

c "2/8 · 待同步"
say "在 store 里造一处未提交改动"
printf 'window-padding-x = 8\n' >> "$DEMO/.config/ghostty/config"
say "等最多 20 秒让 App 自动刷新（或点菜单栏图标里的刷新）"
look "天线发出一圈克制的提示"
wait_key

c "3/8 · 刷新中"
say "点面板里的刷新按钮。status 被加了 2 秒延迟，方便看清。"
look "眼睛左右扫描，天线同步摆动"
wait_key

c "4/8 · 同步中  +  5/8 · 同步成功"
say "点面板里的「立即同步」。sync 被我加了 6 秒延迟，方便看清。"
look "机械臂交替工作 6 秒 → 然后笑着轻跳一次"
wait_key

c "6/8 · 同步失败"
say "把 remote 指到一个不存在的路径，sync 必然失败"
git -C "$DEMO/store" remote set-url origin /tmp/cloudot-demo/nope.git
printf 'cursor-style = block\n' >> "$DEMO/.config/ghostty/config"
say "再点一次「立即同步」"
look "机械臂工作 → 叉眼摇头两次 → 回到当前状态"
wait_key
git -C "$DEMO/store" remote set-url origin "$DEMO/remote.git"

c "7/8 · 有损坏（悬空软链）"
say "模拟另一台机器 unadopt 之后同步过来：清空 manifest + 删掉 store 文件，软链留着"
printf 'version = 1\napps = []\n' > "$DEMO/store/manifest.toml"
rm -f "$DEMO/store/files/.config/ghostty/config"
git -C "$DEMO/store" add -A >/dev/null 2>&1
git -C "$DEMO/store" -c user.email=d@d -c user.name=d commit -qm wipe >/dev/null 2>&1
say "等自动刷新（最多 20 秒）"
look "常驻叉眼 + 皱眉 + 歪头"
wait_key

c "8/8 · 找不到 CLI"
say "把二进制路径指到不存在的文件，然后重启 App（locate 只在启动时跑一次）"
defaults write com.tarnish233.cloudot cloudotBinaryPath /tmp/definitely-not-here
pkill -f 'Cloudot.app/Contents/MacOS/Cloudot' 2>/dev/null; sleep 1
CLOUDOT_HOME="$DEMO" CLOUDOT_DIAG=1 "$APP" > "$LOG" 2>&1 &
sleep 3
look "半透明机器人闭眼，天线垂下"
wait_key

c "收尾"
defaults delete com.tarnish233.cloudot cloudotBinaryPath 2>/dev/null
pkill -f 'Cloudot.app/Contents/MacOS/Cloudot' 2>/dev/null
sleep 1
say "已清掉二进制路径覆盖，沙盒留在 ${DEMO}（rm -rf 即可删）"
say "动效日志：$LOG"
echo
printf '\033[2m   重新以正常方式启动：open %s\033[0m\n' "$HERE/build/Cloudot.app"
