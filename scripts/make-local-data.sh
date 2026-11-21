#!/usr/bin/env bash
# 离线生成 data/flype 下的词库（雾凇 + 万象），供 nix/package.nix 打包。
#
#   scripts/make-local-data.sh [rime-ice 目录] [rime-wanxiang 目录]
#
# 不给目录时用 nixpkgs 里钉好的 rime-ice / rime-wanxiang：不联网、不下载模型，
# 产物直接落 data/flype（真词库不进 git）：
#   lexicon.tsv english.txt VERSION LICENSE
#   wanxiang/{lexicon.tsv,LICENSE,SOURCE.txt,VERSION}
# words.lm / nnlm.bin 要联网下载，这里不生成，完整数据包仍用 scripts/make-data.sh。
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=data/flype
RIME_ICE=${1:-"$(nix build --no-link --print-out-paths nixpkgs#rime-ice)/share/rime-data"}
RIME_WX=${2:-"$(nix build --no-link --print-out-paths nixpkgs#rime-wanxiang)/share/rime-data"}
SOURCE="rime-ice $(nix eval --raw nixpkgs#rime-ice.version)"
VERSION=$(nix eval --raw nixpkgs#rime-ice.version).7

mkdir -p "$OUT"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# 雾凇：和 scripts/make-data.sh 的词库部分一致（大字表 → 语料词频 → 占位词频 → 热词）
EXTRA=()
[[ -f "$RIME_ICE/cn_dicts/41448.dict.yaml" ]] && EXTRA=(--extra-chars "$RIME_ICE/cn_dicts/41448.dict.yaml")
python3 tools/rime2tsv.py ${EXTRA[@]+"${EXTRA[@]}"} "$RIME_ICE/rime_ice.dict.yaml" > "$tmp/lexicon.tsv"
python3 tools/blend-freq.py "$tmp/lexicon.tsv" data/tatoeba-freq.tsv "$tmp/corpus.tsv" \
  --weight 0.3 --max-factor 20 --floor 1000 --label 语料
if [[ -f data/placeholder-freq.tsv ]]; then
  python3 tools/placeholder-freq.py "$tmp/corpus.tsv" data/placeholder-freq.tsv "$tmp/ph.tsv" \
    --min-count 5 --boost 1.4
  mv "$tmp/ph.tsv" "$tmp/corpus.tsv"
fi
python3 tools/add_hotwords.py "$tmp/corpus.tsv" data/hotwords.tsv > "$tmp/hot.tsv"
python3 tools/add_hotwords.py "$tmp/hot.tsv" data/rime-ice-freq.tsv \
  --block data/blocklist.tsv > "$OUT/lexicon.tsv"
cat "$RIME_ICE/en_dicts/en.dict.yaml" "$RIME_ICE/en_dicts/en_ext.dict.yaml" > "$OUT/english.txt"
install -m644 "$RIME_ICE/LICENSE" "$OUT/LICENSE"
echo "$VERSION" > "$OUT/VERSION"
python3 tools/datamark.py stamp "$OUT/lexicon.tsv" lexicon.tsv "$VERSION" --source "$SOURCE"
python3 tools/datamark.py stamp "$OUT/english.txt" english.txt "$VERSION" --source "$SOURCE"
echo "雾凇：$(grep -vc '^#' "$OUT/lexicon.tsv") 条 -> $OUT/lexicon.tsv"

# 万象：备选词库，随包装在 share/flype/dicts/wanxiang
bash scripts/make-wanxiang.sh "$RIME_WX" "$OUT/wanxiang"
