#!/usr/bin/env python3
"""在固定评测集上自动调排序参数（坐标搜索），代替手工试常数。

    python3 tools/tune_weights.py --data DIR [--nnlm nnlm.bin] [--rounds 2] [--out tuned.txt]

DIR 是 scripts/make-data.sh 的输出（lexicon.tsv、words.lm、english.txt）。

做法：tools/eval/ 的每组用例按行号奇偶分成「调参用」和「检验用」两半。每轮依次试每个参数的几个候选值，
只在调参的那一半上算目标（各组 top1 的平均），回归集（regress.tsv）必须全过才算；
找到更好的就留下。最后在检验的那一半上对比调参前后，检验也变好才值得采用——结果打印出来、写进 --out，
由人决定要不要改 core/src/rank.rs、core/src/beam.rs 里的默认值（改默认值时照常跑 scripts/eval.sh、更新基线）。

不用用户的任何输入记录：评测集是从公开的词库和手写句子生成的（见 tools/eval/README.md）。
"""

import argparse
import os
import subprocess
import tempfile

SETS = ["common", "mid", "rare", "jianpin", "trunc", "trunc_exact", "homophone2", "homophone3", "daily", "context"]

# (参数名, 命令行写法, 默认值, 候选值)。weights=… 是 rank.rs 的 FeatureWeights，其余是 flype-eval 的参数
PARAMS = [
    ("lm_weight", "--lm-weight", 1.0, [0.7, 0.85, 1.15, 1.3]),
    ("typo_cost", "--typo-cost", None, [4.5, 5.0, 7.0, 8.0]),
    ("completion_cost", "--completion-cost", None, [0.8, 1.0, 1.5, 1.8]),
    ("abbrev_cost", "--abbrev-cost", None, None),
    ("context_bigram_score", "weights", 1.0, [0.5, 0.75, 1.5, 2.0]),
    ("head_word_nnlm", "weights", 1.0, [0.5, 0.75, 1.25, 1.5]),
    ("coverage_score", "weights", 2.0, [1.0, 1.5, 2.5, 3.0]),
    ("typo_count", "weights", -1.5, [-0.5, -1.0, -2.0, -2.5]),
    ("prefix_completion", "weights", -0.2, [0.0, -0.1, -0.4, -0.6]),
    ("full_jianpin", "weights", -0.3, [0.0, -0.15, -0.5, -0.8]),
    ("hot_val_conf", "weights", 0.3, [0.0, 0.15, 0.5, 0.8]),
    ("with_context", "weights", 0.1, [0.0, 0.05, 0.2, 0.4]),
]


def split_sets(eval_dir, tmp):
    halves = {"tune": [], "check": []}
    for name in SETS:
        with open(os.path.join(eval_dir, name + ".tsv"), encoding="utf-8") as f:
            lines = f.readlines()
        header = [l for l in lines if l.startswith("#")]
        cases = [l for l in lines if l.strip() and not l.startswith("#")]
        for half, part in (("tune", cases[0::2]), ("check", cases[1::2])):
            path = os.path.join(tmp, f"{name}.{half}.tsv")
            with open(path, "w", encoding="utf-8") as f:
                f.writelines(header + part)
            halves[half].append(path)
    return halves


def run(bin_path, data, nnlm, files, regress, values):
    with tempfile.NamedTemporaryFile("r", suffix=".tsv", delete=False) as out:
        out_path = out.name
    cmd = [bin_path, "--data", data, "--eval", regress, "--strict", "regress", "--write-baseline", out_path]
    for f in files:
        cmd += ["--eval", f]
    if nnlm:
        cmd += ["--nnlm", nnlm]
    weights = []
    for name, flag, _default, _cands in PARAMS:
        if name not in values:
            continue
        if flag == "weights":
            weights.append(f"{name}={values[name]}")
        else:
            cmd += [flag, str(values[name])]
    if weights:
        cmd += ["--weights", ",".join(weights)]
    done = subprocess.run(cmd, capture_output=True, text=True)
    rates = {}
    regress_ok = False
    try:
        with open(out_path, encoding="utf-8") as f:
            for line in f:
                if line.startswith("#"):
                    continue
                cols = line.rstrip("\n").split("\t")
                if len(cols) < 6:
                    continue
                group, n, top1, passed = cols[0].split(".")[0], int(cols[1]), int(cols[2]), int(cols[5])
                if group == "regress":
                    regress_ok = passed == n
                else:
                    rates[group] = (passed if group.startswith("homophone") else top1) / max(n, 1)
    finally:
        os.remove(out_path)
    if not rates:
        raise SystemExit(f"flype-eval 出错：{done.stderr[-2000:]}")
    return regress_ok, sum(rates.values()) / len(rates), rates


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--nnlm")
    ap.add_argument("--bin", default="target/release/flype-eval")
    ap.add_argument("--eval-dir", default="tools/eval")
    ap.add_argument("--rounds", type=int, default=2)
    ap.add_argument("--out")
    args = ap.parse_args()
    regress = os.path.join(args.eval_dir, "regress.tsv")
    with tempfile.TemporaryDirectory() as tmp:
        halves = split_sets(args.eval_dir, tmp)
        best = {}
        ok, score, _ = run(args.bin, args.data, args.nnlm, halves["tune"], regress, best)
        print(f"起点：调参集 {score:.4%}，回归集{'全过' if ok else '没全过'}", flush=True)
        start_score = score
        for r in range(args.rounds):
            improved = False
            for name, flag, default, cands in PARAMS:
                if not cands:
                    continue
                for v in cands:
                    trial = dict(best, **{name: v})
                    ok, s, _ = run(args.bin, args.data, args.nnlm, halves["tune"], regress, trial)
                    mark = ""
                    if ok and s > score + 1e-9:
                        best, score, improved, mark = trial, s, True, "  <- 更好"
                    print(f"  第 {r + 1} 轮 {name}={v}: {s:.4%}{'' if ok else '（回归没过）'}{mark}", flush=True)
            if not improved:
                break
        _, before, before_rates = run(args.bin, args.data, args.nnlm, halves["check"], regress, {})
        ok, after, after_rates = run(args.bin, args.data, args.nnlm, halves["check"], regress, best)
        lines = [f"调参集：{start_score:.4%} -> {score:.4%}",
                 f"检验集：{before:.4%} -> {after:.4%}（回归集{'全过' if ok else '没全过'}）",
                 "参数：" + (", ".join(f"{k}={v}" for k, v in best.items()) or "（默认值最好）")]
        for g in SETS:
            lines.append(f"  {g}: {before_rates.get(g, 0):.2%} -> {after_rates.get(g, 0):.2%}")
        print("\n".join(lines))
        if args.out:
            with open(args.out, "w", encoding="utf-8") as f:
                f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
