#!/usr/bin/env bash
# cloudot 端到端测试：在隔离的假 HOME 下模拟两台机器 + 一个 bare remote。
# 完全不碰真实的 ~/.config 和 ~/.cloudot。
set -u

ROOT="$(cd "$(dirname "$0")" && pwd)"
cargo build -q --manifest-path "$ROOT/Cargo.toml" || { echo "构建失败"; exit 1; }
CLOUDOT="$ROOT/target/debug/cloudot"
BASE=/tmp/cloudot-e2e
A="$BASE/machine-a"
B="$BASE/machine-b"
REMOTE="$BASE/remote.git"

section() { printf '\n\033[1m══ %s\033[0m\n' "$1"; }
fail() { printf '\033[31mFAIL: %s\033[0m\n' "$1"; FAILED=1; }
pass() { printf '\033[32m  ok\033[0m %s\n' "$1"; }
FAILED=0

# 重建一对已纳管 ghostty 的机器 + 空 remote，供各回归场景使用。
reset_pair() {
  rm -rf "$BASE"; mkdir -p "$A" "$B"; git init --bare -q -b main "$REMOTE"
  export CLOUDOT_HOME="$A"
  mkdir -p "$A/.config/ghostty"; printf 'theme = dark\n' > "$A/.config/ghostty/config"
  "$CLOUDOT" init --device machine-a --remote "$REMOTE" >/dev/null
  "$CLOUDOT" add ghostty >/dev/null
  "$CLOUDOT" sync >/dev/null
  export CLOUDOT_HOME="$B"
  "$CLOUDOT" init --device machine-b --remote "$REMOTE" >/dev/null
  "$CLOUDOT" apply >/dev/null
}

rm -rf "$BASE"
mkdir -p "$A" "$B"
git init --bare -q -b main "$REMOTE"

# ─────────────────────────────────────────────── 机器 A：首次纳管
section "空态：未 init 时 status 成功返回 initialized=false"
export CLOUDOT_HOME="$A"
UNINIT=$("$CLOUDOT" --json status)
echo "$UNINIT" | grep -qE '"initialized"[[:space:]]*:[[:space:]]*false' \
  && pass "未 init 的 status 带 initialized=false" || fail "未 init status 不对"
echo "$UNINIT" | grep -qE '"ok"[[:space:]]*:[[:space:]]*true' \
  && pass "未 init 的 status 是成功信封" || fail "未 init status 不是 ok"
# 写路径仍拒绝
"$CLOUDOT" --json sync >/dev/null 2>&1 \
  && fail "未 init 时 sync 不该成功" \
  || pass "未 init 时 sync 仍失败"

section "机器 A：init + add ghostty"
export CLOUDOT_HOME="$A"
mkdir -p "$A/.config/ghostty"
printf 'theme = dark\nfont-size = 13\n' > "$A/.config/ghostty/config"
ORIGINAL=$(cat "$A/.config/ghostty/config")

"$CLOUDOT" init --device machine-a --remote "$REMOTE" || fail "init"
"$CLOUDOT" apps
"$CLOUDOT" add ghostty || fail "add"

[ -L "$A/.config/ghostty/config" ] && pass "target 变成软链" || fail "target 不是软链"
[ -f "$A/.cloudot/store/files/.config/ghostty/config" ] && pass "内容进了 store" || fail "store 里没有内容"
[ "$(cat "$A/.config/ghostty/config")" = "$ORIGINAL" ] && pass "透过软链读到原内容" || fail "内容变了"
BK=$(find "$A/.cloudot/backups" -type f -name config | head -1)
[ -n "$BK" ] && [ "$(cat "$BK")" = "$ORIGINAL" ] && pass "备份存在且内容正确 ($BK)" || fail "备份缺失"

section "机器 A：status"
"$CLOUDOT" status
section "机器 A：status --json"
"$CLOUDOT" status --json

section "机器 A：sync 推到 remote"
"$CLOUDOT" sync || fail "sync"

# ─────────────────────────────────────────────── 机器 B：新机器落地
section "机器 B：init（应自动 clone）"
export CLOUDOT_HOME="$B"
"$CLOUDOT" init --device machine-b --remote "$REMOTE" || fail "B init"
[ -f "$B/.cloudot/store/manifest.toml" ] && pass "clone 带回了 manifest" || fail "manifest 没同步过来"

section "机器 B：apply 前 status（应显示本地缺失）"
"$CLOUDOT" status

section "机器 B：apply"
"$CLOUDOT" apply || fail "B apply"
[ -L "$B/.config/ghostty/config" ] && pass "B 上建好了软链" || fail "B 没有软链"
[ "$(cat "$B/.config/ghostty/config")" = "$ORIGINAL" ] && pass "B 读到 A 的配置" || fail "B 内容不对"

# ─────────────────────────────────────────────── 双向同步
section "机器 B：改配置 → sync"
printf 'window-padding-x = 8\n' >> "$B/.config/ghostty/config"
"$CLOUDOT" sync || fail "B sync"

section "机器 A：sync 拉回 B 的改动"
export CLOUDOT_HOME="$A"
"$CLOUDOT" sync || fail "A sync"
grep -q 'window-padding-x = 8' "$A/.config/ghostty/config" \
  && pass "A 收到了 B 的改动" || fail "A 没收到 B 的改动"

# ─────────────────────────────────────────────── 关键失效场景
section "机器 A：模拟 App 用「替换写入」顶掉软链"
rm "$A/.config/ghostty/config"
printf 'theme = light\n# 本机新写的，比 store 新\n' > "$A/.config/ghostty/config"

echo "--- doctor 应报 warn ---"
"$CLOUDOT" doctor
echo "--- apply 应拒绝覆盖 ---"
"$CLOUDOT" apply
[ ! -L "$A/.config/ghostty/config" ] && pass "apply 没有静默覆盖本地文件" || fail "apply 覆盖了本地文件！"

echo "--- apply --force 应备份后覆盖 ---"
"$CLOUDOT" apply --force || fail "apply --force"
[ -L "$A/.config/ghostty/config" ] && pass "软链已修复" || fail "软链没修好"
FOUND=$(grep -rl 'theme = light' "$A/.cloudot/backups" 2>/dev/null | head -1)
[ -n "$FOUND" ] && pass "被覆盖的本地内容已备份 ($FOUND)" || fail "覆盖前没备份！"

# ─────────────────────────────────────────────── 逃生门
section "机器 A：unadopt 还原"
"$CLOUDOT" unadopt ghostty || fail "unadopt"
[ ! -L "$A/.config/ghostty/config" ] && [ -f "$A/.config/ghostty/config" ] \
  && pass "配置还原成实体文件" || fail "还原失败"
grep -q 'window-padding-x = 8' "$A/.config/ghostty/config" \
  && pass "还原后内容完整" || fail "还原后内容丢了"
[ ! -f "$A/.cloudot/store/files/.config/ghostty/config" ] \
  && pass "已从 store 移除" || fail "store 里还有残留"

# ─────────────────────────────────────────────── 回归：孤儿软链
# 曾经的 bug：B 上 unadopt 后 A 同步，A 的软链悬空、ghostty 读不到配置，
# 而 status/doctor 还报告「没有致命问题」。
section "回归：另一台机器 unadopt 后，本机不能留下悬空软链"
reset_pair
export CLOUDOT_HOME="$B"; "$CLOUDOT" unadopt ghostty >/dev/null && "$CLOUDOT" sync >/dev/null
export CLOUDOT_HOME="$A"; "$CLOUDOT" sync 2>&1 | grep -q '从 git 历史取回' \
  && pass "sync 自动从 git 历史取回内容" || fail "sync 没有自愈"
[ -f "$A/.config/ghostty/config" ] && [ ! -L "$A/.config/ghostty/config" ] \
  && pass "已还原成实体文件，不是悬空软链" || fail "留下了悬空软链"
[ -s "$A/.config/ghostty/config" ] && pass "内容非空" || fail "内容丢了"

section "回归：修不好时必须报警而不是装作没事"
reset_pair
# 手动制造一个无从恢复的孤儿：清掉 manifest 与 git 历史，只留软链
export CLOUDOT_HOME="$A"
printf 'version = 1\napps = []\n' > "$A/.cloudot/store/manifest.toml"
rm -f "$A/.cloudot/store/files/.config/ghostty/config"
rm -rf "$A/.cloudot/backups"
git -C "$A/.cloudot/store" add -A >/dev/null && git -C "$A/.cloudot/store" commit -qm wipe
"$CLOUDOT" status 2>&1 | grep -q '孤儿软链' && pass "status 报出孤儿软链" || fail "status 没报"
"$CLOUDOT" doctor >/dev/null 2>&1 && fail "doctor 应以非零退出" || pass "doctor 以非零退出"
[ -L "$A/.config/ghostty/config" ] && pass "修不好时不删任何东西" || fail "坏链被悄悄清掉了"

# ─────────────────────────────────────────────── 回归：冲突不能污染实时配置
# 曾经的 bug：rebase 冲突后 store 停在 UU 态，而 store 就是软链目标，
# ghostty 会立刻读到塞满 <<<<<<< 的配置。
section "回归：rebase 冲突必须自动回滚，实时配置不被污染"
reset_pair
export CLOUDOT_HOME="$B"; printf 'theme = light\n' > "$B/.config/ghostty/config"; "$CLOUDOT" sync >/dev/null 2>&1
export CLOUDOT_HOME="$A"; printf 'theme = solarized\n' > "$A/.config/ghostty/config"
"$CLOUDOT" sync 2>&1 | grep -q '已自动回滚' && pass "报冲突并说明已回滚" || fail "冲突提示不对"
[ -z "$(git -C "$A/.cloudot/store" status --porcelain)" ] \
  && pass "store 工作树干净，没有残留 rebase" || fail "store 停在中间状态"
grep -q '<<<<<<<' "$A/.config/ghostty/config" && fail "实时配置被冲突标记污染！" \
  || pass "实时配置没有冲突标记"
[ "$(cat "$A/.config/ghostty/config")" = "theme = solarized" ] \
  && pass "本机版本保持生效" || fail "本机内容被改了"

# JSON 信封必须带结构化 conflict（GUI 靠它画 diff 面板）
export CLOUDOT_HOME="$B"; printf 'theme = nord\n' > "$B/.config/ghostty/config"; "$CLOUDOT" sync >/dev/null 2>&1
export CLOUDOT_HOME="$A"; printf 'theme = gruvbox\n' > "$A/.config/ghostty/config"
CONFLICT_JSON=$("$CLOUDOT" --json sync 2>/dev/null || true)
# CLI 默认 pretty-print，键值之间有空格
echo "$CONFLICT_JSON" | grep -qE '"kind"[[:space:]]*:[[:space:]]*"pull_conflict"' \
  && pass "JSON 错误分类是 pull_conflict" || fail "JSON 没有 pull_conflict"
echo "$CONFLICT_JSON" | grep -qE '"conflict"[[:space:]]*:' \
  && pass "JSON 带 conflict 字段" || fail "JSON 缺 conflict"
echo "$CONFLICT_JSON" | grep -q 'files/.config/ghostty/config' \
  && pass "conflict 列出了冲突文件" || fail "conflict 没列文件"

section "回归：resolve --theirs 对齐远端"
# 上面 A 本地是 gruvbox、远端是 nord；选远端
export CLOUDOT_HOME="$A"
"$CLOUDOT" resolve --theirs >/dev/null \
  && pass "resolve --theirs 成功" || fail "resolve --theirs 失败"
[ "$(cat "$A/.config/ghostty/config")" = "theme = nord" ] \
  && pass "本机内容换成远端版本" || fail "theirs 后内容不对：$(cat "$A/.config/ghostty/config")"
[ -z "$(git -C "$A/.cloudot/store" status --porcelain)" ] \
  && pass "resolve 后工作树干净" || fail "resolve 后工作树不干净"

section "回归：resolve --ours 强推本地"
# 再制造一次冲突，这次选本地
export CLOUDOT_HOME="$B"; printf 'theme = dracula\n' > "$B/.config/ghostty/config"; "$CLOUDOT" sync >/dev/null 2>&1
export CLOUDOT_HOME="$A"; printf 'theme = solarized\n' > "$A/.config/ghostty/config"
"$CLOUDOT" sync >/dev/null 2>&1 || true
"$CLOUDOT" resolve --ours >/dev/null \
  && pass "resolve --ours 成功" || fail "resolve --ours 失败"
[ "$(cat "$A/.config/ghostty/config")" = "theme = solarized" ] \
  && pass "本机内容保持本地版本" || fail "ours 后本地内容变了"
# B 再 sync 应拿到 A 强推上去的 solarized
export CLOUDOT_HOME="$B"
"$CLOUDOT" sync >/dev/null \
  && pass "B 能拉到 ours 强推的提交" || fail "B 拉 ours 结果失败"
[ "$(cat "$B/.config/ghostty/config")" = "theme = solarized" ] \
  && pass "B 收到 A 强推的本地版本" || fail "B 内容不是 solarized：$(cat "$B/.config/ghostty/config")"

# ─────────────────────────────────────────────── add 的事务性
# 曾经的行为：多路径 adopter 中途失败会留下「已建链但 manifest 和 links.toml
# 都没记」的半纳管状态 —— status 看不见、unadopt 也撤不掉。
section "add 必须整体成功或整体回滚"
reset_pair
export CLOUDOT_HOME="$A"
mkdir -p "$A/.config/tp"
printf 'first = 1\n' > "$A/.config/tp/one"
mkdir -p "$A/.config/tp/two"          # 目录 → 第二个路径注定失败
cat > "$A/.cloudot/adopters/twopath.toml" <<'TOML'
id = "twopath"
name = "Two Path"
detect = ["~/.config/tp/one"]
[[paths]]
path = "~/.config/tp/one"
[[paths]]
path = "~/.config/tp/two"
TOML
"$CLOUDOT" add twopath >/dev/null 2>&1 && fail "应该失败" || pass "整体失败"
[ -f "$A/.config/tp/one" ] && [ ! -L "$A/.config/tp/one" ] \
  && pass "第一个路径已回滚成实体文件" || fail "第一个路径没回滚"
[ "$(cat "$A/.config/tp/one")" = "first = 1" ] && pass "回滚后内容完整" || fail "回滚后内容不对"
[ ! -e "$A/.cloudot/store/files/.config/tp/one" ] && pass "store 里没残留" || fail "store 有残留"
grep -q twopath "$A/.cloudot/store/manifest.toml" && fail "manifest 被写脏了" || pass "manifest 干净"
grep -q twopath "$A/.cloudot/links.toml" && fail "links.toml 被写脏了" || pass "links.toml 干净"

# ─────────────────────────────────────────────── 凭据门禁
section "疑似凭据默认拦下，--allow-secrets 才放行"
reset_pair
export CLOUDOT_HOME="$A"
mkdir -p "$A/.config/st"
printf 'api_key = 8f14e45fceea167a5a36dedd4bea2543\n' > "$A/.config/st/conf"
cat > "$A/.cloudot/adopters/secretish.toml" <<'TOML'
id = "secretish"
name = "Secretish"
detect = ["~/.config/st/conf"]
[[paths]]
path = "~/.config/st/conf"
TOML
OUT=$("$CLOUDOT" add secretish 2>&1)
echo "$OUT" | grep -q '像是有凭据' && pass "默认拦下并说明原因" || fail "没拦住"
echo "$OUT" | grep -q '8f14e45fceea167a5a36dedd4bea2543' && fail "报告里回显了凭据值！" \
  || pass "报告不回显凭据值"
[ ! -L "$A/.config/st/conf" ] && pass "被拦时没有动文件" || fail "被拦却已经改了文件"
"$CLOUDOT" add secretish --allow-secrets >/dev/null 2>&1 \
  && pass "--allow-secrets 放行" || fail "--allow-secrets 不生效"
"$CLOUDOT" doctor >/dev/null 2>&1 && fail "doctor 应因明文凭据报错" || pass "doctor 持续报错"

# ─────────────────────────────────────────────── store 卫生与备份清理
section "store 有 .gitignore，macOS 产物不会被提交"
reset_pair
export CLOUDOT_HOME="$A"
[ -f "$A/.cloudot/store/.gitignore" ] && pass ".gitignore 已就位" || fail "缺 .gitignore"
touch "$A/.cloudot/store/.DS_Store"
"$CLOUDOT" sync >/dev/null 2>&1
git -C "$A/.cloudot/store" ls-files | grep -q '.DS_Store' \
  && fail ".DS_Store 被提交了" || pass ".DS_Store 没进仓库"

section "备份清理"
export CLOUDOT_HOME="$A"
for s in 20200101-000000 20200102-000000 20200103-000000; do
  mkdir -p "$A/.cloudot/backups/$s/.config/ghostty"
  printf 'old\n' > "$A/.cloudot/backups/$s/.config/ghostty/config"
done
"$CLOUDOT" backups | grep -q '共 ' && pass "backups 能盘点" || fail "backups 盘点失败"
"$CLOUDOT" backups prune --keep 1 --dry-run >/dev/null \
  && pass "prune --dry-run 执行成功" || fail "prune --dry-run 失败"
[ "$("$CLOUDOT" backups --json | grep -c '"stamp"')" -ge 3 ] \
  && pass "prune --dry-run 一份都没真删" || fail "prune --dry-run 删了东西！"
"$CLOUDOT" backups prune --keep 1 >/dev/null && pass "prune 执行成功" || fail "prune 失败"
[ "$("$CLOUDOT" backups --json | grep -c '"stamp"')" = 1 ] \
  && pass "只留下 1 份" || fail "保留份数不对"

# ─────────────────────────────────────────────── 预演与 show
# --dry-run 唯一的承诺就是「什么都不动」。一旦它偷偷改了东西，用户对它的
# 信任就没了 —— 所以这里逐个写命令验文件系统与 manifest/links.toml 都没变。
section "--dry-run 必须一个字节都不写"
reset_pair
export CLOUDOT_HOME="$A"
# 命令输出存到 BASE 之外：reset_pair 会 rm -rf "$BASE"
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

# add：本机有 fish 配置就拿它试，没有就造一个自定义 adopter
mkdir -p "$A/.config/dryapp"
printf 'k = v\n' > "$A/.config/dryapp/conf"
cat > "$A/.cloudot/adopters/dryapp.toml" <<'TOML'
id = "dryapp"
name = "Dry App"
detect = ["~/.config/dryapp/conf"]
[[paths]]
path = "~/.config/dryapp/conf"
TOML

MANIFEST_BEFORE=$(cat "$A/.cloudot/store/manifest.toml")
LINKS_BEFORE=$(cat "$A/.cloudot/links.toml" 2>/dev/null || echo "")

"$CLOUDOT" add dryapp --dry-run > "$OUT/add-dry.txt" 2>&1 \
  && pass "add --dry-run 成功退出" || fail "add --dry-run 失败：$(cat "${OUT}/add-dry.txt")"
grep -q '移入 store 并建链' "$OUT/add-dry.txt" \
  && pass "add --dry-run 报出了将做的动作" || fail "add --dry-run 没报动作"
grep -q '预演' "$OUT/add-dry.txt" \
  && pass "add --dry-run 明确说了这是预演" || fail "预演提示缺失"
[ ! -L "$A/.config/dryapp/conf" ] && pass "add --dry-run 没把本地文件换成软链" \
  || fail "add --dry-run 建了软链！"
[ ! -e "$A/.cloudot/store/files/.config/dryapp/conf" ] \
  && pass "add --dry-run 没往 store 写东西" || fail "add --dry-run 写了 store！"
[ "$(cat "$A/.cloudot/store/manifest.toml")" = "$MANIFEST_BEFORE" ] \
  && pass "add --dry-run 没动 manifest" || fail "manifest 被改了！"
[ "$(cat "$A/.cloudot/links.toml" 2>/dev/null || echo "")" = "$LINKS_BEFORE" ] \
  && pass "add --dry-run 没动 links.toml" || fail "links.toml 被改了！"

# apply：先把软链弄坏，预演应报「会建链」但不真建
rm "$A/.config/ghostty/config"
"$CLOUDOT" apply --dry-run > "$OUT/apply-dry.txt" 2>&1
grep -q '会建链' "$OUT/apply-dry.txt" \
  && pass "apply --dry-run 报出会建链" || fail "apply --dry-run 没报会建链"
[ ! -e "$A/.config/ghostty/config" ] \
  && pass "apply --dry-run 没真的建链" || fail "apply --dry-run 建了链！"
"$CLOUDOT" apply >/dev/null 2>&1   # 修回来，后面还要用

# sync：改配置后预演，应报「会提交」但工作树仍然脏
printf 'theme = dryrun\n' > "$A/.config/ghostty/config"
"$CLOUDOT" sync --dry-run > "$OUT/sync-dry.txt" 2>&1
grep -q '会提交' "$OUT/sync-dry.txt" \
  && pass "sync --dry-run 报出会提交" || fail "sync --dry-run 没报会提交"
grep -q '不联网' "$OUT/sync-dry.txt" \
  && pass "sync --dry-run 说明了不联网" || fail "缺少不联网说明"
[ -n "$(git -C "$A/.cloudot/store" status --porcelain)" ] \
  && pass "sync --dry-run 没有提交改动" || fail "sync --dry-run 提交了！"

# unadopt：预演不能把软链换回实体文件
"$CLOUDOT" unadopt ghostty --dry-run > "$OUT/unadopt-dry.txt" 2>&1
grep -q '会还原成实体文件' "$OUT/unadopt-dry.txt" \
  && pass "unadopt --dry-run 报出会还原" || fail "unadopt --dry-run 没报还原"
[ -L "$A/.config/ghostty/config" ] \
  && pass "unadopt --dry-run 软链还在" || fail "unadopt --dry-run 真的解链了！"
grep -q ghostty "$A/.cloudot/store/manifest.toml" \
  && pass "unadopt --dry-run 没动 manifest" || fail "manifest 里的条目被删了！"

# 不支持预演的命令要明确拒绝，不能装作成功
"$CLOUDOT" --json init --dry-run 2>&1 | grep -qE '"kind"[[:space:]]*:[[:space:]]*"unsupported"' \
  && pass "init --dry-run 报 unsupported" || fail "init --dry-run 没有明确拒绝"
"$CLOUDOT" --json resolve --theirs --dry-run 2>&1 \
  | grep -qE '"kind"[[:space:]]*:[[:space:]]*"unsupported"' \
  && pass "resolve --dry-run 报 unsupported" || fail "resolve --dry-run 没有明确拒绝"

# 只读命令收到 --dry-run 应静默忽略
"$CLOUDOT" status --dry-run >/dev/null 2>&1 \
  && pass "status 静默忽略 --dry-run" || fail "status 因 --dry-run 失败"

section "show 报出会动哪些文件"
export CLOUDOT_HOME="$A"
"$CLOUDOT" show ghostty > "$OUT/show.txt" 2>&1 \
  && pass "show 成功退出" || fail "show 失败"
grep -q '~/.config/ghostty/config' "$OUT/show.txt" \
  && pass "show 列出了目标路径" || fail "show 没列出目标路径"
grep -q 'files/.config/ghostty/config' "$OUT/show.txt" \
  && pass "show 列出了 store 内位置" || fail "show 没列 store 位置"
"$CLOUDOT" --json show ghostty | grep -qE '"schema"[[:space:]]*:[[:space:]]*"cloudot.show/v1"' \
  && pass "show 的 JSON schema 正确" || fail "show schema 不对"
"$CLOUDOT" --json show ghostty | grep -qE '"state"[[:space:]]*:[[:space:]]*"linked"' \
  && pass "show 带每个路径的链接状态" || fail "show 缺链接状态"
"$CLOUDOT" --json show nope 2>&1 | grep -qE '"kind"[[:space:]]*:[[:space:]]*"unknown_app"' \
  && pass "show 未知应用报 unknown_app" || fail "show 未知应用分类不对"
# 未 init 也要能看定义 —— 用户装完第一件事就是想知道会动什么
FRESH="$OUT/fresh-home"
mkdir -p "$FRESH"
CLOUDOT_HOME="$FRESH" "$CLOUDOT" show ghostty >/dev/null 2>&1 \
  && pass "未 init 也能 show" || fail "未 init 时 show 失败"

section "结果"
if [ "$FAILED" = 0 ]; then printf '\033[32m全部通过\033[0m\n'; else printf '\033[31m有失败项\033[0m\n'; fi
exit "$FAILED"
