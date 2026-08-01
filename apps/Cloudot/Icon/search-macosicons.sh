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

for q in "$@"; do
  echo "== $q"
  curl -sS -X POST https://api.macosicons.com/api/search \
    -H "Content-Type: application/json" \
    -H "x-api-key: $MACOSICONS_API_KEY" \
    -d "{\"query\":\"$q\",\"searchOptions\":{\"hitsPerPage\":20,\"page\":1}}" \
    --max-time 25 \
  | python3 -c '
import json,sys
d = json.load(sys.stdin)
print(f"  共 {d.get(\"totalHits\",0)} 个结果")
for h in sorted(d.get("hits",[]), key=lambda x: -(x.get("downloads") or 0))[:10]:
    print(f"  {h.get(\"downloads\",0):>6}  {h[\"appName\"]:<38} @{h.get(\"usersName\")}")
    print(f"          {h.get(\"lowResPngUrl\")}")
'
done
