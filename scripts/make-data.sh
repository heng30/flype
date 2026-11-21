#!/usr/bin/env bash
# 从雾凇拼音（rime-ice）生成 Flype 用的词库和英文词表。
#
#   scripts/make-data.sh [rime-ice 目录] [输出目录]
#
# 不给 rime-ice 目录时自动 git clone 一份（--depth 1）。输出：
#   lexicon.tsv   中文词库（条数生成时打出来）
#   english.txt   英文词表
#   LICENSE       rime-ice 的许可证（GPL-3.0），随数据一起分发
#   words.lm      词级语言模型（从 GitHub Release 下载，见下面 LM_URL）
#   nnlm.bin      整句增强模型：Flype 自己的字级神经语言模型（WTNN，从 GitHub Release 下载，见下面 NNLM_URL；
#                 下载不到时只警告，不带它照样出别的文件）
#   SOURCE.txt    数据来源与版本
#   VERSION       词库版本（rime-ice 日期 + 修订号，如 2026.11.14.6）
#   DATA-MANIFEST 数据清单：每个数据文件的名字、版本、sha256、来源（tools/datamark.py manifest），
#                 装进安装包的系统数据目录
#
# 每个数据文件带版本标记（docs/DATA.md「数据版本标记」）：lexicon.tsv、english.txt 在这里写进头部注释，
# 版本就是 VERSION；words.lm 的标记在发布模型时就写好了（加标记以前的 lm-2026.11.09 按字节数认）。
#
# words.lm 是用 flype-lmbuild 从约 5 GB 语料训练的，不在这里重新训练。
# 想用本机的文件（不下载）：FLYPE_LM_FILE=path/to/words.lm scripts/make-data.sh
# nnlm.bin 是 tools/nnlm/train.py 训出来的（docs/DATA.md「整句增强模型」），也不在这里训。
# 用本机的（不下载）：FLYPE_NNLM_FILE=path/to/nnlm.bin；FLYPE_NO_NNLM=1 不带它。
#
# 语料词频（默认开，不联网、不要额外的 Python 包）：data/tatoeba-freq.tsv 是从钉死版本的 Tatoeba 中文句子
# （CC BY 2.0 FR）挖出的词频，按权重 0.3 融进已有词（只改词频，不加词），署名写进 SOURCE.txt。
#   FLYPE_NO_CORPUS_FREQ=1             不融这张表（删掉 data/tatoeba-freq.tsv 也一样）
#   FLYPE_CORPUS="a.txt b.txt.bz2"     换成本机语料现挖（要 jieba，见下），必须用 FLYPE_CORPUS_LICENSE 写清来源和许可证；
#                                        带 TAB 的用 FLYPE_CORPUS_COLUMN 指定列，繁体加 FLYPE_CORPUS_T2S=1
#   这张表怎么重新生成、怎么核对：scripts/make-tatoeba-freq.sh [--check]
# 本机语料要的 Python 包（版本钉死）：python3 -m pip install --require-hashes -r tools/requirements-data.txt
# 评测数字见 docs/DATA.md §3.3。
set -euo pipefail
cd "$(dirname "$0")/.."

RIME=${1:-}
OUT=${2:-dist/data}
mkdir -p "$OUT"

# 不用 sha256sum -c：macOS 的 sha256sum 是 BSD 版，不认 -c --quiet；python3 两边都有
sha256() {
  python3 -c 'import hashlib, sys
h = hashlib.sha256()
with open(sys.argv[1], "rb") as f:
    for block in iter(lambda: f.read(1 << 20), b""):
        h.update(block)
print(h.hexdigest())' "$1"
}

# require_py <模块> <版本>：本机语料要的 Python 包必须是钉死的版本
require_py() {
  local have
  have=$(python3 -c 'import importlib.metadata as m, sys; print(m.version(sys.argv[1]))' "$1" 2>/dev/null || true)
  if [[ "$have" != "$2" ]]; then
    echo "要 Python 包 ${1}==${2}（现在是 ${have:-没装}）：" >&2
    echo "  python3 -m pip install --require-hashes -r tools/requirements-data.txt" >&2
    exit 1
  fi
}

if [[ -z "$RIME" ]]; then
  RIME=$(mktemp -d)/rime-ice
  git clone --depth 1 https://github.com/iDvel/rime-ice.git "$RIME"
fi

# 大字表 41448 只补缺的单字（频次小于 1：不参与组句，只排在翻页的最后）：一个音节的字要列全
EXTRA=()
[[ -f "$RIME/cn_dicts/41448.dict.yaml" ]] && EXTRA=(--extra-chars "$RIME/cn_dicts/41448.dict.yaml")
python3 tools/rime2tsv.py ${EXTRA[@]+"${EXTRA[@]}"} "$RIME/rime_ice.dict.yaml" > "$OUT/lexicon.rime.tsv"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# 语料词频放在热词表之前：data/ 下手工定的词频（只提不降）和黑名单最后套，不会被冲掉。
# 语料词频：每个词最多抬到 20 * max(原频次, 1000)，免得小语料里出现得多的词（Tatoeba 的「汤姆」）抬得离谱；
# 只数两个字以上的词（单字频次决定 d/l/b、shi 这些短输入的首页）。参数在留出评测上挑的，见 docs/DATA.md §3.3。
CORPUS_NOTE=""
corpus_counts=""
if [[ "${FLYPE_NO_CORPUS_FREQ:-}" == "1" ]]; then
  :
elif [[ -n "${FLYPE_CORPUS:-}" ]]; then
  if [[ -z "${FLYPE_CORPUS_LICENSE:-}" ]]; then
    echo "用本机语料（FLYPE_CORPUS）时要用 FLYPE_CORPUS_LICENSE 写清来源和许可证" >&2
    exit 1
  fi
  require_py jieba 0.42.1
  read -r -a corpus_files <<< "$FLYPE_CORPUS"
  corpus_args=(--column "${FLYPE_CORPUS_COLUMN:-0}")
  if [[ "${FLYPE_CORPUS_T2S:-}" == "1" ]]; then
    require_py opencc-python-reimplemented 0.1.7
    corpus_args+=(--t2s)
  fi
  python3 tools/mine_corpus_freq.py --min-chars 2 "${corpus_args[@]}" "${corpus_files[@]}" > "$tmp/corpus.counts"
  corpus_counts=$tmp/corpus.counts
  corpus_sums=""
  for f in "${corpus_files[@]}"; do
    corpus_sums+="
  $(basename "$f")  sha256 $(sha256 "$f")"
  done
  CORPUS_NOTE="
语料词频：用 jieba 0.42.1 给下列本机语料分词，数两个字以上的词，融进已有词的词频（不加新词）：${corpus_sums}
来源与许可证：${FLYPE_CORPUS_LICENSE}
"
elif [[ -f data/tatoeba-freq.tsv ]]; then
  corpus_counts=data/tatoeba-freq.tsv
  CORPUS_NOTE="
语料词频：lexicon.tsv 里已有词的词频融进了 data/tatoeba-freq.tsv（scripts/make-tatoeba-freq.sh 生成），
那是从 Tatoeba（https://tatoeba.org）的中文句子统计出的词频（2026-11-05 的导出，只数两个字以上的词），
不含句子本身。Tatoeba 的句子许可证是 CC BY 2.0 FR（https://creativecommons.org/licenses/by/2.0/fr/），
作者是 Tatoeba contributors，各句的作者见 tatoeba.org 上各句的页面。
"
fi
if [[ -n "$corpus_counts" ]]; then
  python3 tools/blend-freq.py "$OUT/lexicon.rime.tsv" "$corpus_counts" "$tmp/lexicon.corpus.tsv" \
    --weight "${FLYPE_CORPUS_WEIGHT:-0.3}" --max-factor 20 --floor 1000 --label 语料
  mv "$tmp/lexicon.corpus.tsv" "$OUT/lexicon.rime.tsv"
fi

# 只有占位词频（100）的词（rime-ice 的 ext / tencent 词表，三十多万个）换上语料里数出来的词频：
# data/placeholder-freq.tsv 是 LCCC + 维基（约 10 亿字）里这些词的次数，至少 5 次的才换，其余照旧在加载时估。
# 占位词频低，切分时常被拆成几段，数出来的次数偏少，换算系数再乘 1.4（扫过 1 / 1.4 / 2 / 3 / 5：1 时没有收益，
# 3 起简拼掉 1 个点以上，5 时回归集挂一条）。评测（scripts/eval.sh）：日常整句 +1.4、上文 +0.2，
# 冷门词 -0.4、简拼 -0.2，其余持平。FLYPE_NO_PLACEHOLDER_FREQ=1 不换。
PLACEHOLDER_NOTE=""
if [[ "${FLYPE_NO_PLACEHOLDER_FREQ:-}" != "1" && -f data/placeholder-freq.tsv ]]; then
  python3 tools/placeholder-freq.py "$OUT/lexicon.rime.tsv" data/placeholder-freq.tsv "$tmp/lexicon.ph.tsv" \
    --min-count 5 --boost 1.4
  mv "$tmp/lexicon.ph.tsv" "$OUT/lexicon.rime.tsv"
  PLACEHOLDER_NOTE="
占位词频：rime-ice 里只有占位频次 100 的词，在 data/placeholder-freq.tsv 里出现至少 5 次的，换成了语料词频
（tools/placeholder-freq.py）。那张表是 LCCC（thu-coai，MIT）和中文维基百科（CC BY-SA 4.0）里这些词的次数，
不含语料本身。
"
fi

# 合进 Flype 自己整理的热词（AI、工作、网络用语），见 data/hotwords.tsv；
# 再按 data/rime-ice-freq.tsv 修正几个常用词的词频，去掉 data/blocklist.tsv 里的错条目（去次）
python3 tools/add_hotwords.py "$OUT/lexicon.rime.tsv" data/hotwords.tsv > "$OUT/lexicon.hot.tsv"
python3 tools/add_hotwords.py "$OUT/lexicon.hot.tsv" data/rime-ice-freq.tsv --block data/blocklist.tsv > "$OUT/lexicon.tsv"
rm "$OUT/lexicon.rime.tsv" "$OUT/lexicon.hot.tsv"
cat "$RIME/en_dicts/en.dict.yaml" "$RIME/en_dicts/en_ext.dict.yaml" > "$OUT/english.txt"
cp "$RIME/LICENSE" "$OUT/LICENSE"

# 词级语言模型，训练语料与命令见 Release 说明。换模型时：
#   1. 训练出的 words.lm 先加版本标记再上传 Release（lm-<日期>），版本比上一版大：
#        python3 tools/datamark.py stamp words.lm words.lm <日期>.0 --source "lm-<日期>"
#      （或者在 flype-lmbuild 里调 wordlm::set_data_marker，两种写出来逐字节一样）；
#   2. 同时改下面的 URL 和 sha256（sha256 是加了标记以后的文件的）。
# 插件和设置界面按标记里的版本认用户目录里的旧副本，不用再往 kOfficialLmSizes（data_files.h）、
# OFFICIAL_LM_SIZES（flype-config）里加字节数：那两张表只为认出加标记以前发布的 lm-2026.11.09
# （它不会重新生成），以后不动。没有标记的新模型会在下面写清单时报错。
LM_URL=https://github.com/Gloridust/Flype/releases/download/lm-2026.11.09/words.lm.xz
LM_SHA256=42cb7c4a7cededa29b1a9ff105530085529fefff96535cc051341ebd51605dbe
if [[ -n "${FLYPE_LM_FILE:-}" ]]; then
  cp "$FLYPE_LM_FILE" "$OUT/words.lm"
else
  # macOS 默认没有 xz，用 python3 自带的 lzma 解
  if command -v xz >/dev/null; then
    curl -fsSL --retry 3 "$LM_URL" | xz -d > "$OUT/words.lm"
  else
    curl -fsSL --retry 3 "$LM_URL" |
      python3 -c 'import lzma, shutil, sys; shutil.copyfileobj(lzma.open(sys.stdin.buffer), sys.stdout.buffer)' \
      > "$OUT/words.lm"
  fi
fi
actual=$(sha256 "$OUT/words.lm")
if [[ "$actual" != "$LM_SHA256" ]]; then
  echo "words.lm 的 sha256 对不上（应为 ${LM_SHA256}，实际 ${actual}）" >&2
  exit 1
fi

# 整句增强模型（nnlm.bin，WTNN）：tools/nnlm/prep_corpus.py 整理语料、tools/nnlm/train.py 训练，
# xz 压缩后上传 Release（nnlm-<日期>）。换模型时改下面的 URL 和 sha256（sha256 是解压后的 nnlm.bin 的；
# 版本标记 train.py 导出时就写在文件头里了）。下载不到（比如 Release 还没发）时只警告、不带它，
# 别的文件照常生成；下载到了但 sha256 对不上就报错退出。
NNLM_URL=https://github.com/Gloridust/Flype/releases/download/nnlm-2026.11.17/nnlm.bin.xz
NNLM_SHA256=d50289dfd10c8a4a0da49fc412bdc61a37c6f63ef1a1cbec4bdb74bf5bb7b52b
rm -f "$OUT/nnlm.bin"
if [[ "${FLYPE_NO_NNLM:-}" == "1" ]]; then
  echo "不带整句增强模型 nnlm.bin（FLYPE_NO_NNLM=1）" >&2
elif [[ -n "${FLYPE_NNLM_FILE:-}" ]]; then
  cp "$FLYPE_NNLM_FILE" "$OUT/nnlm.bin"
else
  if ! curl -fsSL --retry 3 -o "$tmp/nnlm.bin.xz" "$NNLM_URL"; then
    echo "警告：下载不到整句增强模型（${NNLM_URL}），这次不带 nnlm.bin" >&2
    rm -f "$tmp/nnlm.bin.xz"
  elif command -v xz >/dev/null; then
    xz -dc "$tmp/nnlm.bin.xz" > "$OUT/nnlm.bin"
  else
    python3 -c 'import lzma, shutil, sys; shutil.copyfileobj(lzma.open(sys.argv[1]), sys.stdout.buffer)' \
      "$tmp/nnlm.bin.xz" > "$OUT/nnlm.bin"
  fi
fi
NNLM_NOTE=""
if [[ -f "$OUT/nnlm.bin" ]]; then
  actual=$(sha256 "$OUT/nnlm.bin")
  if [[ "$actual" != "$NNLM_SHA256" ]]; then
    echo "nnlm.bin 的 sha256 对不上（应为 ${NNLM_SHA256}，实际 ${actual}）" >&2
    rm -f "$OUT/nnlm.bin"
    exit 1
  fi
  NNLM_NOTE="
nnlm.bin 是 Flype 的整句增强模型（字级神经语言模型，WTNN 格式，3 层 GPT-2、宽 256），
由 Flype 的 tools/nnlm/train.py 从下列语料训练，许可证 CC BY-SA 4.0：
  LCCC（https://huggingface.co/datasets/thu-coai/lccc，清华大学 CoAI 组，MIT）
  中文维基百科（Hugging Face wikimedia/wikipedia 20231101.zh，https://zh.wikipedia.org/，CC BY-SA 4.0）
许可证全文：https://creativecommons.org/licenses/by-sa/4.0/
下载地址：$NNLM_URL
sha256（解压后）：$NNLM_SHA256
"
fi

COMMIT=$(git -C "$RIME" rev-parse HEAD 2>/dev/null || echo unknown)
DATE=$(git -C "$RIME" log -1 --format=%cs 2>/dev/null || date +%F)
# 词库版本 = rime-ice 日期 + Flype 自己的修订号；只改了热词表等、rime-ice 没变时加 REV，
# 否则同版本号的包 apt 不会更新、插件也分不出新旧。rime-ice 换新日期时 REV 归 0。
# 5：默认融进 Tatoeba 语料词频（data/tatoeba-freq.tsv）；数据压缩包里加上五笔码表。
# 6：数据文件带版本标记（flype-data），多了 DATA-MANIFEST；VERSION 全用点分隔（2026.11.14.6）。
# 7：占位词频换成语料词频（data/placeholder-freq.tsv）。
REV=7
VERSION="${DATE//-/.}.$REV"
echo "$VERSION" > "$OUT/VERSION"
# 版本标记：插在第一行注释后面（插件和设置界面按它认用户目录里的旧副本，见 docs/DATA.md）
python3 tools/datamark.py stamp "$OUT/lexicon.tsv" lexicon.tsv "$VERSION" --source "rime-ice ${COMMIT:0:12} (${DATE})"
python3 tools/datamark.py stamp "$OUT/english.txt" english.txt "$VERSION" --source "rime-ice ${COMMIT:0:12} (${DATE})"

cat > "$OUT/SOURCE.txt" <<EOF
数据来源：雾凇拼音 rime-ice（https://github.com/iDvel/rime-ice），许可证 GPL-3.0，见 LICENSE。
版本：${COMMIT}（${DATE}）
lexicon.tsv 由 Flype 的 tools/rime2tsv.py 转换自 rime_ice.dict.yaml 及其 import_tables，
另从 cn_dicts/41448.dict.yaml（大字表）补了词库里没有的单字读音（频次记 0.7~0.9），
再用 tools/add_hotwords.py 合进 Flype 的热词表 data/hotwords.tsv、按 data/rime-ice-freq.tsv 修正几个常用词的词频、
去掉 data/blocklist.tsv 里的错条目；
english.txt 是 en_dicts/en.dict.yaml 与 en_dicts/en_ext.dict.yaml 原样拼接。
${CORPUS_NOTE}${PLACEHOLDER_NOTE}
words.lm 是 Flype 的词级语言模型，由 flype-lmbuild 从下列语料统计生成，许可证 CC BY-SA 4.0：
  LCCC（https://huggingface.co/datasets/thu-coai/lccc，清华大学 CoAI 组，MIT）
  中文维基百科（https://zh.wikipedia.org/，CC BY-SA 4.0）
下载地址：$LM_URL
sha256：$LM_SHA256
${NNLM_NOTE}
各数据文件的版本（文件头的 flype-data 标记）和 sha256 见同一目录（安装包里是 /usr/share/flype）的 DATA-MANIFEST。
EOF
# 数据清单
python3 tools/datamark.py manifest "$OUT"

LINES=$(grep -vc '^#' "$OUT/lexicon.tsv" || true)
echo "词库 $LINES 条 -> $OUT/lexicon.tsv（rime-ice ${DATE}，修订 ${REV}）"
if (( LINES < 100000 )); then
  echo "词库条数太少，rime-ice 的目录结构可能变了" >&2
  exit 1
fi
