#!/usr/bin/env bash
# 安装 fcitx5 插件和它的数据（替代原来的 cmake --install）。
#
#   sudo PREFIX=/usr scripts/install-addon.sh
#
# 先跑一次 cargo build --release -p flype-fcitx5 -p flype-config（或 scripts/build-linux.sh --addon）。
# 可用 PREFIX / DESTDIR / LIBDIR / DATADIR / BINDIR 改安装位置；repo 里没有的语言模型、词库只是跳过。
set -euo pipefail
cd "$(dirname "$0")/.."

PREFIX=${PREFIX:-/usr/local}
DESTDIR=${DESTDIR:-}
LIBDIR=${LIBDIR:-$PREFIX/lib}
DATADIR=${DATADIR:-$PREFIX/share}
BINDIR=${BINDIR:-$PREFIX/bin}
TARGET=${CARGO_TARGET_DIR:-target}/release

if [[ ! -f "$TARGET/libflype.so" ]]; then
  echo "找不到 $TARGET/libflype.so，先执行 cargo build --release -p flype-fcitx5" >&2
  exit 1
fi

put() { install -Dm"$1" "$2" "$DESTDIR$3"; }
put_optional() { [[ -f "$1" ]] && put "$2" "$1" "$3" || true; }

# 插件本体 + 两份清单（fcitx5 按 Library=libflype 找 .so）
put 755 "$TARGET/libflype.so" "$LIBDIR/fcitx5/libflype.so"
put 644 "$TARGET/addon/flype-addon.conf" "$DATADIR/fcitx5/addon/flype.conf"
put 644 "$TARGET/addon/flype.conf" "$DATADIR/fcitx5/inputmethod/flype.conf"
put 644 core/include/flype_core.h "$PREFIX/include/flype/flype_core.h"

# 词库和模型：demo 词典兜底，真实数据由 scripts/make-data.sh 生成在 dist/data
put 644 core/data/demo_lexicon.tsv "$DATADIR/flype/demo_lexicon.tsv"
for name in lexicon.tsv english.txt words.lm nnlm.bin DATA-MANIFEST; do
  put_optional "dist/data/$name" 644 "$DATADIR/flype/$name"
done

# 图标、fcitx5 主题
cp -r icons/hicolor "$DESTDIR$DATADIR/icons/"
cp -r themes/fcitx5 "$DESTDIR$DATADIR/fcitx5/themes/"

# 设置界面（cargo build -p flype-config）
if [[ -f "$TARGET/flype-config" ]]; then
  put 755 "$TARGET/flype-config" "$BINDIR/flype-config"
  put 644 settings/flype-config.desktop "$DATADIR/applications/flype-config.desktop"
else
  echo "没编 flype-config，跳过设置界面（cargo build --release -p flype-config）" >&2
fi

echo "装好了：$DESTDIR$LIBDIR/fcitx5/libflype.so"
echo "让 fcitx5 认一下：fcitx5 -r（或注销重登）"
