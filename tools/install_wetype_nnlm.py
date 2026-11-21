#!/usr/bin/env python3
"""把微信输入法自带的字级神经语言模型（nlrgpt2_nnlm.bin）放到 Flype 找得到的地方。

模型是腾讯的，Flype 不分发它。你本机有微信输入法的安装包或程序时，可以用这个脚本取出来：

    python3 tools/install_wetype_nnlm.py WeType-3.0.0.zip
    python3 tools/install_wetype_nnlm.py /Applications/WeType.app
    python3 tools/install_wetype_nnlm.py path/to/nlrgpt2_nnlm.bin

默认放到 ~/.local/share/flype/wetype/nlrgpt2_nnlm.bin（--dest 可改）。不看版本号：找得到
flype-check 时用核心的读取代码试解析，解析不了才不装。设置界面「输入 → 整句增强模型」里的「导入…」做的是同一件事。
"""

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import zipfile

NAME = "nlrgpt2_nnlm.bin"


def default_dest():
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(base, "flype", "wetype", NAME)


def read_model(src):
    if os.path.isdir(src):
        for root, _, files in os.walk(src):
            if NAME in files:
                with open(os.path.join(root, NAME), "rb") as f:
                    return f.read()
        sys.exit(f"{src} 里没有 {NAME}")
    if zipfile.is_zipfile(src):
        with zipfile.ZipFile(src) as z:
            hits = [n for n in z.namelist() if n.endswith("/" + NAME) or n == NAME]
            if not hits:
                sys.exit(f"{src} 里没有 {NAME}")
            return z.read(hits[0])
    with open(src, "rb") as f:
        return f.read()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("source", help="微信输入法的安装包（.zip）、程序目录（.app）或模型文件本身")
    ap.add_argument("--dest", default=default_dest())
    ap.add_argument("--force", action="store_true", help="解析不了也装（不保证能用）")
    args = ap.parse_args()
    data = read_model(args.source)
    os.makedirs(os.path.dirname(args.dest), exist_ok=True)
    tmp = args.dest + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    check = shutil.which("flype-check")
    if check and not args.force:
        done = subprocess.run([check, "--nnlm", tmp], capture_output=True, text=True)
        if done.returncode != 0:
            os.remove(tmp)
            sys.exit(f"读不出这个模型：{done.stderr.strip()}；确定要装加 --force")
    os.replace(tmp, args.dest)
    md5 = hashlib.md5(data).hexdigest()
    print(f"已装到 {args.dest}（{len(data) / 1e6:.1f} MB，MD5 {md5}）。在设置里点一次「导入」或重启 fcitx5 生效。")


if __name__ == "__main__":
    main()
