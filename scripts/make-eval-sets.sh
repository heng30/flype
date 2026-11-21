#!/usr/bin/env bash
# 重新生成 tools/eval/ 下从词库抽出来的评测集（只用 Python 标准库，结果是确定的）。
#
#   scripts/make-eval-sets.sh <lexicon.tsv> [输出目录，默认 tools/eval]
#
# 词库要用 tools/eval/baseline.tsv 里「数据」那一行记的版本（scripts/make-data.sh 用那个 rime-ice 提交生成），
# 换了词库生成出来的用例会不一样，基线也要跟着重跑（见 tools/eval/README.md）。
# 每个文件开头写了生成参数和词库的 sha256。
#
# 不在这里生成的：context.tsv（make_sentence_eval.py --context，要 jieba + pypinyin，见 tools/eval/README.md）、
# regress.tsv（手写）。
set -euo pipefail
cd "$(dirname "$0")/.."

LEX=${1:?用法：scripts/make-eval-sets.sh <lexicon.tsv> [输出目录]}
OUT=${2:-tools/eval}
mkdir -p "$OUT"

gen() {
  local name=$1
  shift
  python3 tools/make_eval.py "$LEX" "$@" > "$OUT/$name.tsv"
}

# 常用词：词频前 5000 的词
gen common --count 1000
# 中频词：第 3 万到 30 万名
gen mid --count 1000 --skip 30000 --top 300000
# 冷门词：第 30 万名以后
gen rare --count 1000 --skip 300000 --top 1000000
# 简拼：常用词只打每个音节的声母
gen jianpin --count 1000 --jianpin
# 少打最后一个字母：截完不是词的（期望补全出原词）、截完正好是常用词的（期望那个精确词，见 make_eval.py）
gen trunc --count 1000 --top 30000 --truncate 1 --exact skip
gen trunc_exact --count 1000 --top 30000 --truncate 1 --exact only
# 同音词排序：两、三个音节的输入，词库里的同音词都要排在第一个单字前面
gen homophone2 --count 300 --top 20000 --homophones 2
gen homophone3 --count 200 --top 20000 --homophones 3

# 日常整句：手写的句子，拼音按词库切词取读音（不用 pypinyin）
python3 tools/make_sentence_eval.py --text tools/eval/daily_sentences.txt --lexicon "$LEX" > "$OUT/daily.tsv"

wc -l "$OUT"/{common,mid,rare,jianpin,trunc,trunc_exact,homophone2,homophone3,daily}.tsv
