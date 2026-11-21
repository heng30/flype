#!/usr/bin/env bash
# 排序评测：用一份数据目录（lexicon.tsv、words.lm、english.txt，scripts/make-data.sh 的输出）跑 tools/eval/ 下的
# 全部用例。配置和两个外壳默认设置下一样（core/src/bin/common/preset.rs）。CI 的「排序评测」任务跑的就是它。
#
#   scripts/eval.sh <数据目录> [--update-baseline] [flype-eval 的其它参数，比如 --misses]
#
# - regress 组（tools/eval/regress.tsv）必须全过；
# - 其余各组的 top1（同音词组还有「通过」）和 tools/eval/baseline.tsv 比，低了超过 0.5 个百分点就算没过；
# - --update-baseline：照样打印和旧基线的对比（贴进提交说明），然后把这次的数字写进 tools/eval/baseline.tsv，不算没过。
#
# 有 $GITHUB_STEP_SUMMARY 时把汇总表（Markdown）追加进去。FLYPE_EVAL_BIN 指定现成的 flype-eval，不给就现编一个。
# 数据目录不可写时（比如 /usr/share/flype）先复制到临时目录：载入词库会在旁边写 .wcache。
set -euo pipefail
cd "$(dirname "$0")/.."

DATA=${1:?用法：scripts/eval.sh <数据目录> [--update-baseline] [flype-eval 的参数]}
shift
EVAL_DIR=tools/eval
BASELINE=$EVAL_DIR/baseline.tsv
# 回归集放第一个；组名就是文件名
SETS=(regress common mid rare jianpin trunc trunc_exact homophone2 homophone3 daily context)

update=0
extra=()
for arg in "$@"; do
  if [[ "$arg" == "--update-baseline" ]]; then
    update=1
  else
    extra+=("$arg")
  fi
done

for f in lexicon.tsv words.lm english.txt; do
  if [[ ! -s "$DATA/$f" ]]; then
    echo "数据目录 ${DATA} 里没有 ${f}（要 scripts/make-data.sh 的输出）" >&2
    exit 1
  fi
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [[ ! -w "$DATA" ]]; then
  mkdir "$tmp/data"
  for f in lexicon.tsv words.lm english.txt VERSION SOURCE.txt; do
    [[ -f "$DATA/$f" ]] && cp "$DATA/$f" "$tmp/data/"
  done
  DATA=$tmp/data
fi

BIN=${FLYPE_EVAL_BIN:-}
if [[ -z "$BIN" ]]; then
  cargo build --release --locked -p flype-core --bin flype-eval >&2
  BIN=${CARGO_TARGET_DIR:-target}/release/flype-eval
fi

args=(--data "$DATA" --strict regress --markdown "$tmp/summary.md")
for name in "${SETS[@]}"; do
  args+=(--eval "$EVAL_DIR/$name.tsv")
done
[[ -f "$BASELINE" ]] && args+=(--baseline "$BASELINE")
if (( update )); then
  args+=(--write-baseline "$tmp/baseline.tsv")
fi

status=0
"$BIN" "${args[@]}" ${extra[@]+"${extra[@]}"} || status=$?

if [[ -n "${GITHUB_STEP_SUMMARY:-}" && -f "$tmp/summary.md" ]]; then
  cat "$tmp/summary.md" >> "$GITHUB_STEP_SUMMARY"
fi
if (( update )); then
  if [[ ! -f "$tmp/baseline.tsv" ]]; then
    echo "没写出基线（flype-eval 出错了）" >&2
    exit 1
  fi
  if grep -q '没过（必须全过）' "$tmp/summary.md"; then
    echo "回归集没全过，不更新基线：先修好，或者在提交说明里写清为什么改那几条" >&2
    exit 1
  fi
  cp "$tmp/baseline.tsv" "$BASELINE"
  echo "基线已更新：${BASELINE}（提交时把上面的对比表贴进提交说明）" >&2
  exit 0
fi
exit "$status"
