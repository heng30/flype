#!/usr/bin/env python3
"""从词库 TSV 生成排序评测用例。

思路：同一个拼音往往对应多个同音词，取频次最高的那个作为期望答案，
这样用例的答案就是「这个词库里最该出现的词」。

用法：
    python3 tools/make_eval.py lexicon.tsv --count 500 > cases.tsv

默认只取最常用的 5000 个词。常用词几乎不会出错，「打全了的低频词被高频补全抢走」
（zhongdi 出「重点」不出「种地」）这类问题要用 --skip 换到中低频词才测得出来。

--truncate 1（少打最后一个字母）时，截完的拼音可能正好是另一个词（zhonggu 是「中古」，不是没打完的
「中国」）。引擎的既定规则是「打全了的词排第一」（docs/ENGINE.md「半成品输入」），所以用 --exact 分开：
    --exact skip   只留截完不是任何词的（期望是补全出来的原词）
    --exact only   只留截完正好是词频 >= 1000 的词的（期望是这些精确词，补全排后面）
截完是词、但词频都不到 1000 的两边都不要（不受「精确优先」保护，哪个排第一都说得过去）。

--homophones 2（或 3）生成同音词排序用例：两（三）个音节的输入，期望是词库里所有用完整个输入的
多字词（`|` 分隔），第四列 before-single：这些词都要排在第一个单字候选前面（flype-eval 判断）。

输出只取决于参数和词库（同频的按词库里的顺序），同一份词库生成的文件逐字节一样。
"""

import argparse
import hashlib
import os
import re
import sys
from collections import defaultdict


# 和 core/src/lib.rs 的 EXACT_PROTECT_MIN_FREQ 一样：词频低于这个的精确词不保证排第一
EXACT_PROTECT_MIN_FREQ = 1000.0
CJK = re.compile(r"^[\u4e00-\u9fff]+$")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def homophones(rows, args, out):
    """同音词用例：按最常用的那个词的词频排，从前 --top 个输入里均匀抽 --count 个。"""
    words = defaultdict(list)  # 输入 -> [词]（按词频从高到低，同频按词库顺序）
    order = []  # 输入，按最常用的那个词的词频从高到低
    # 同一串字母还能按别的音节数切的（xian 是一个音节，也是 xi an「西安」）不要：
    # 那时单字也是打全了的精确结果，「同音词都在单字前」不成立
    counts = defaultdict(set)
    for freq, pinyin, word in rows:
        counts[pinyin.replace(" ", "")].add(len(pinyin.split()))
    for freq, pinyin, word in rows:
        syllables = pinyin.split()
        if len(syllables) != args.homophones or len(word) != len(syllables) or not CJK.match(word):
            continue
        joined = "".join(syllables)
        if len(counts[joined]) > 1:
            continue
        if joined not in words:
            order.append(joined)
        if word not in words[joined]:
            words[joined].append(word)
    order = order[: args.top]
    step = max(1, len(order) // args.count)
    sampled = order[::step][: args.count]
    out.write("# 输入\t词库里全部同音词（都要排在第一个单字前）\t上文\t属性\n")
    for joined in sampled:
        out.write("%s\t%s\t\tbefore-single\n" % (joined, "|".join(words[joined])))
    print("生成 %d 条同音词用例" % len(sampled), file=sys.stderr)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("lexicon")
    ap.add_argument("--count", type=int, default=500)
    ap.add_argument("--min-len", type=int, default=2, help="拼音总长度下限")
    ap.add_argument("--max-len", type=int, default=12, help="拼音总长度上限")
    ap.add_argument("--max-word", type=int, default=4, help="词的字数上限")
    ap.add_argument("--top", type=int, default=5000, help="只从频次前 N 的词里取")
    ap.add_argument(
        "--skip",
        type=int,
        default=0,
        help="跳过频次前 N 的词（只测中低频词：--skip 30000 --top 300000）。"
        "同音词排第几仍按全部词算",
    )
    ap.add_argument(
        "--jianpin",
        action="store_true",
        help="把输入换成简拼（只取每音节首字母）",
    )
    ap.add_argument(
        "--truncate",
        type=int,
        default=0,
        help="截掉末尾 N 个字母，模拟半成品输入（测补全能力）",
    )
    ap.add_argument(
        "--exact",
        choices=["any", "skip", "only"],
        default="any",
        help="--truncate 时截完正好是词的怎么办（见上面）",
    )
    ap.add_argument(
        "--homophones",
        type=int,
        default=0,
        help="生成同音词排序用例：几个音节（2 或 3）",
    )
    ap.add_argument(
        "--rank",
        type=int,
        default=1,
        help="取同音词里第几高频的作为期望（2/3 用来测候选广度）",
    )
    args = ap.parse_args()

    per_pinyin = defaultdict(list)  # pinyin -> [(freq, word)]
    rows = []
    with open(args.lexicon, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 3:
                continue
            pinyin, word, freq = parts[0], parts[1], parts[2]
            try:
                freq = float(freq)
            except ValueError:
                continue
            rows.append((freq, pinyin, word))

    rows.sort(key=lambda r: -r[0])

    out = sys.stdout
    out.write("# 由 tools/make_eval.py 生成：%s\n" % " ".join([os.path.basename(args.lexicon)] + sys.argv[2:]))
    out.write("# 词库 %s 的 sha256：%s\n" % (os.path.basename(args.lexicon), sha256(args.lexicon)))
    if args.homophones:
        homophones(rows, args, out)
        return

    # 截完的拼音正好是哪些词（全部词，不只前 top 个）
    exact = defaultdict(list)  # 拼音（不带空格）-> [(freq, word)]
    if args.truncate > 0 and args.exact != "any":
        for freq, pinyin, word in rows:
            exact[pinyin.replace(" ", "")].append((freq, word))
    rows = rows[: args.top]

    for index, (freq, pinyin, word) in enumerate(rows):
        syllables = pinyin.split()
        joined = "".join(s[0] for s in syllables) if args.jianpin else pinyin.replace(" ", "")
        if len(word) > args.max_word:
            continue
        if args.truncate > 0:
            if len(joined) <= args.truncate + 1:
                continue
            joined = joined[: -args.truncate]
            if args.exact != "any":
                hits = exact.get(joined, [])
                solid = [w for f, w in hits if f >= EXACT_PROTECT_MIN_FREQ]
                if args.exact == "skip" and hits:
                    continue
                if args.exact == "only":
                    if not solid:
                        continue
                    word = "|".join(solid)
        if not (args.min_len <= len(joined) <= args.max_len):
            continue
        bucket = per_pinyin[joined]
        if len(bucket) < args.rank:
            bucket.append((freq, word, index))

    # 第 rank 高频的词（同音词里第 2/3 个）
    items = [
        (joined, (bucket[args.rank - 1][1], bucket[args.rank - 1][0]))
        for joined, bucket in per_pinyin.items()
        if len(bucket) >= args.rank and bucket[args.rank - 1][2] >= args.skip
    ]
    # 均匀抽样，避免全是超高频词
    step = max(1, len(items) // args.count)
    sampled = items[::step][: args.count]

    out.write("# 输入\t期望\n")
    for joined, (word, _freq) in sampled:
        out.write("%s\t%s\n" % (joined, word))
    print("生成 %d 条用例" % len(sampled), file=sys.stderr)


if __name__ == "__main__":
    main()
