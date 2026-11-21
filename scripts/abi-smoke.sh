#!/usr/bin/env bash
# 构建并运行 C ABI 冒烟测试（模拟 fcitx5 插件的调用序列）。
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p flype-core

LEXICON=${1:-/tmp/flype_abi.tsv}
# 旧版本生成的词库可能缺条目，内容不对就重建
if [[ ! -f "$LEXICON" ]] || ! grep -q 'zhi dao' "$LEXICON" 2>/dev/null; then
  printf '# abi smoke\nni hao\t你好\t100000\ntian\t天\t5000\ntian\t田\t2000\nzhong guo\t中国\t8000\nzhi dao\t知道\t6000\n' > "$LEXICON"
fi

BIGRAM=${2:-/tmp/flype_abi_bigram.tsv}
if [[ ! -f "$BIGRAM" ]]; then
  printf '# prev<TAB>cur<TAB>count\n今天\t天\t8\n今天\t田\t1\n' > "$BIGRAM"
fi

INCLUDE=core/include
LIBDIR=target/release
case "$(uname -s)" in
  Darwin) LIB=libflype_core.dylib; RUNENV="DYLD_LIBRARY_PATH=$LIBDIR" ;;
  *)      LIB=libflype_core.so;    RUNENV="LD_LIBRARY_PATH=$LIBDIR" ;;
esac

# 库要放在源文件后面：Ubuntu 默认 --as-needed，放前面会被丢掉
cc -O2 -I"$INCLUDE" -o /tmp/flype_abi_smoke tools/abi_smoke.c -L"$LIBDIR" -lflype_core
env $RUNENV /tmp/flype_abi_smoke "$LEXICON" "$BIGRAM"
