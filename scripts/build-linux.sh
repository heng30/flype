#!/usr/bin/env bash
# 在 Linux 上构建（Wayland 下由 fcitx5 承载输入法）。依赖：cargo、C++20 编译器、
# fcitx5 开发包（fcitx5-devel / fcitx5-dev）、libasound2-dev（语音采集必需）、libxkbcommon-dev。
#   scripts/build-linux.sh          构建并测试 Rust 工作区和 C ABI
#   scripts/build-linux.sh --addon  连 fcitx5 插件一起构建
set -euo pipefail
cd "$(dirname "$0")/.."

case "${1:-}" in
  --help|-h)
    echo "用法：scripts/build-linux.sh [--addon]"
    echo "--addon 只构建，安装需另执行 sudo scripts/install-addon.sh。"
    exit 0 ;;
  ""|--addon) ;;
  *) echo "未知参数：$1；用法：scripts/build-linux.sh [--addon]" >&2; exit 2 ;;
esac
(( $# <= 1 )) || { echo "最多接受一个 --addon 参数" >&2; exit 2; }

echo "依赖和安装步骤见 README.md。"
for name in FLYPE_LEXICON FLYPE_ENGLISH FLYPE_LM; do
  if [[ -n "${!name:-}" && ( ! -f "${!name}" || ! -r "${!name}" || ! -s "${!name}" ) ]]; then
    echo "$name 指定的文件不存在、为空或不可读：${!name}" >&2
    exit 1
  fi
done
lexicon=${FLYPE_LEXICON:-/usr/share/flype/lexicon.tsv}
english=${FLYPE_ENGLISH:-$(dirname "$lexicon")/english.txt}
lm=${FLYPE_LM:-$(dirname "$lexicon")/words.lm}
[[ -f "$lm" && -r "$lm" && -s "$lm" ]] || lm=""

echo "== Rust 工作区（核心、语音、设置界面）=="
cargo build --release --locked --workspace
cargo test --locked --workspace

echo "== C ABI 冒烟测试 =="
./scripts/abi-smoke.sh

echo "== 数据自检 =="
if [[ -f "$lexicon" && -r "$lexicon" && -s "$lexicon" ]]; then
  cargo run --release --locked -q -p flype-core --bin flype-check -- \
    --lexicon "$lexicon" --lm "$lm"
else
  echo "跳过真实词库自检：没有 ${lexicon}；准备好词库后设置 FLYPE_LEXICON 再运行。"
fi

if [[ "${1:-}" == "--addon" ]]; then
  echo "== fcitx5 插件 =="
  cargo build --release -p flype-fcitx5
  echo "== 插件按键测试（私有 fcitx5 实例，不影响正在用的输入法）=="
  if [[ -s "$lexicon" && -s "$english" && -s "$lm" ]]; then
    FLYPE_LM="$lm" ./scripts/addon-test.sh "$lexicon" "$english"
  elif [[ -n "${FLYPE_LEXICON:-}${FLYPE_ENGLISH:-}${FLYPE_LM:-}" ]]; then
    echo "按键测试需要完整词库、英文词表和语言模型；请同时提供 FLYPE_LEXICON、FLYPE_ENGLISH、FLYPE_LM。" >&2
    exit 1
  else
    echo "跳过真实数据按键测试：缺 lexicon.tsv、english.txt 或 words.lm，准备步骤见 README.md 的「构建」。"
  fi
  echo "安装：sudo PREFIX=/usr scripts/install-addon.sh"
fi

echo "完成"
