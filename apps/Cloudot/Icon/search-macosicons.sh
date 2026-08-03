#!/usr/bin/env bash
# 用 macosicons 的 API 找图标候选。
#
# key 从环境变量读，不要写进这个文件 —— 它会进 git 历史。
#   export MACOSICONS_API_KEY=...
#   ./search-macosicons.sh cloud sync link
#
# 注意：macosicons 的高下载量结果几乎全是现有品牌的同人重绘
# （iCloud / Adobe / Apple Shortcuts / GitHub …），不能拿来当自己应用的图标。
set -euo pipefail
: "${MACOSICONS_API_KEY:?请先 export MACOSICONS_API_KEY}"

# 过滤脚本写成独立文件而不是 `python3 -c '...'`：内联的话 shell 引号和 Python
# 引号会打架（fish 尤其会改写内层引号），实测报 SyntaxError。
FILTER="$(mktemp -t macosicons-filter).py"
trap 'rm -f "$FILTER"' EXIT
cat > "$FILTER" <<'PY'
import json, sys
data = json.load(sys.stdin)
print("  共 %d 个结果" % data.get("totalHits", 0))
hits = sorted(data.get("hits", []), key=lambda h: -(h.get("downloads") or 0))
for hit in hits[:10]:
    print("  %6d  %-38s @%s" % (
        hit.get("downloads") or 0, hit.get("appName", "")[:38], hit.get("usersName")))
    print("          %s" % hit.get("lowResPngUrl"))
PY

for query in "$@"; do
  echo "== ${query}"
  # 端点是 /api/v1/search —— 不带 v1 的旧路径现在 404。
  curl -sS -X POST https://api.macosicons.com/api/v1/search \
    -H "Content-Type: application/json" \
    -H "x-api-key: $MACOSICONS_API_KEY" \
    -d "{\"query\":\"${query}\",\"searchOptions\":{\"hitsPerPage\":20,\"page\":1}}" \
    --max-time 25 \
  | python3 "$FILTER"
done
