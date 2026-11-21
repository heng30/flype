#!/usr/bin/env python3
"""给词库里只有占位词频的词换上语料里数出来的词频。

    python3 tools/placeholder-freq.py lexicon.tsv counts.tsv out.tsv [--min-count 5] [--floor 1]

rime-ice 的 ext / tencent 词表（三十多万个词，占整个词库三分之一以上）所有词的频次都是同一个占位值 100，
加载时靠 estimate_placeholder_freqs（core/src/lexicon.rs）按组成部分估。tools/blend-freq.py 融合语料词频时
特意跳过这些词（占位值不是真实频次，按比例融合没有意义）。这里专门处理它们：

- 在语料里出现了至少 --min-count 次的占位词：频次 = 次数 * 换算系数（两边都有的非占位词上估出来，
  见 lexdata.shared_scale），不低于 --floor * 占位值；
- 出现得更少的保持占位值，照旧在加载时估。

counts.tsv 每行「词<TAB>次数」（flype-lmbuild --save-counts 存的计数可以转出来，见 docs/DATA.md）。
"""

import argparse

import lexdata


def main():
    ap = argparse.ArgumentParser(description="占位词频换成语料词频")
    ap.add_argument("lexicon")
    ap.add_argument("counts")
    ap.add_argument("output")
    ap.add_argument("--min-count", type=float, default=5)
    ap.add_argument("--floor", type=float, default=1.0, help="新频次不低于 floor * 占位值")
    ap.add_argument("--boost", type=float, default=1.0,
                    help="换算系数再乘这个数：占位词在切分时很少被选中（频次低，常被拆成更常用的几段），数出来的次数偏少")
    args = ap.parse_args()

    lex = lexdata.Lexicon(args.lexicon)
    counts = lexdata.load_counts(args.counts)
    placeholder = lex.placeholder()
    if placeholder is None:
        raise SystemExit("词库里没有占位频次，不用处理")
    scale = lexdata.shared_scale(lex, counts, placeholder)
    if scale <= 0:
        raise SystemExit("算不出换算系数")
    changed = 0
    for word, rows in lex.readings_by_word().items():
        holders = [r for r in rows if r[3] == placeholder]
        count = counts.get(word, 0)
        if not holders or count < args.min_count:
            continue
        value = max(count * scale * args.boost, args.floor * placeholder)
        for row in holders:
            lex.set_freq(row, value / len(holders))
        changed += 1
    lex.write(args.output)
    lexdata.log(f"占位词频：{changed} 个词换成语料词频（占位值 {placeholder:g}，换算系数 {scale:.4g} × {args.boost:g}，"
                f"至少 {args.min_count:g} 次）")


if __name__ == "__main__":
    main()
