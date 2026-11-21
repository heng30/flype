#!/usr/bin/env bash
# 构建 fcitx5 插件并跑按键行为测试。
#
#   scripts/addon-test.sh [lexicon.tsv] [english.txt]
#   FLYPE_VOICE_WAV=说的话.wav FLYPE_VOICE_EXPECT=期望的字 scripts/addon-test.sh ...  # 加跑语音
#   FLYPE_HARNESS_DBUS=$DBUS_SESSION_BUS_ADDRESS ...   # 让插件连上指定的会话总线
#
# 起的是一个私有、无界面、无 D-Bus 的 fcitx5 实例（配置目录都在临时目录里），
# 不会影响正在用的输入法。不给词库时用仓库里的 demo 词典，部分用例会因为缺词失败。
set -euo pipefail
cd "$(dirname "$0")/.."

LEXICON=${1:-$PWD/core/data/demo_lexicon.tsv}
ENGLISH=${2:-}
BUILD=$(realpath -m "${FLYPE_ADDON_TEST_BUILD:-target/release}")

cargo build --release -p flype-fcitx5 --features key-harness --lib --bin flype-key-harness

ROOT=$(mktemp -d)
DBUS_PID=
trap '[[ -n "$DBUS_PID" ]] && kill "$DBUS_PID" 2>/dev/null; rm -rf "$ROOT"' EXIT

# 外部命令切模式的用例要会话总线：没给 FLYPE_HARNESS_DBUS 时起一条私有的（没有 dbus-daemon 时跳过）
if [[ -z "${FLYPE_HARNESS_DBUS:-}" ]] && command -v dbus-daemon >/dev/null; then
  FLYPE_HARNESS_DBUS=$(dbus-daemon --session --fork --print-address=1 --print-pid=3 3>"$ROOT/dbus.pid")
  DBUS_PID=$(cat "$ROOT/dbus.pid")
fi

mkdir -p "$ROOT/data/fcitx5/addon" "$ROOT/data/fcitx5/inputmethod" "$ROOT/config/fcitx5"
cp "$BUILD/addon/flype-addon.conf" "$ROOT/data/fcitx5/addon/flype.conf"
cp "$BUILD/addon/flype.conf" "$ROOT/data/fcitx5/inputmethod/flype.conf"
# fcitx5 自带的测试输入法，用来模拟「打到一半切换输入法」
cat > "$ROOT/data/fcitx5/addon/testim.conf" <<'ADDON'
[Addon]
Name=testim
Category=InputMethod
Library=libtestim
Type=SharedLibrary
OnDemand=False
ADDON
# 假的 clipboard 插件（测试程序里注册的静态插件），盖掉系统的那个。
# FLYPE_HARNESS_REAL_CLIPBOARD=1 时用系统装的 clipboard 插件，只确认调用它不出错
if [[ -z "${FLYPE_HARNESS_REAL_CLIPBOARD:-}" ]]; then
cat > "$ROOT/data/fcitx5/addon/clipboard.conf" <<'ADDON'
[Addon]
Name=clipboard
Category=Module
Library=clipboard
Type=StaticLibrary
OnDemand=False
ADDON
fi
cat > "$ROOT/config/fcitx5/profile" <<'PROFILE'
[Groups/0]
Name=Default
Default Layout=us
DefaultIM=flype

[Groups/0/Items/0]
Name=flype
Layout=

[Groups/0/Items/1]
Name=testim
Layout=

[GroupOrder]
0=Default
PROFILE

# 语音用例要真模型：给了 FLYPE_VOICE_WAV（代替麦克风的录音）才跑，模型默认用本机已下载的那份。
# 没给时用一个空目录，跑「没有模型时按语音键只提示、不下载」
VOICE_WAV=${FLYPE_VOICE_WAV:-}
if [[ -n "$VOICE_WAV" ]]; then
  VOICE_MODEL_DIR=${FLYPE_VOICE_MODEL_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/flype/wtlocal}
else
  VOICE_MODEL_DIR="$ROOT/data/flype/wtlocal"
fi

SYSTEM_ADDONS=$(pkg-config --variable=libdir Fcitx5Core 2>/dev/null || echo /usr/lib/x86_64-linux-gnu)/fcitx5
env -i PATH="$PATH" HOME="$ROOT" \
  XDG_DATA_HOME="$ROOT/data" XDG_CONFIG_HOME="$ROOT/config" XDG_RUNTIME_DIR="$ROOT" \
  XDG_DATA_DIRS="/usr/share" \
  FCITX_ADDON_DIRS="$BUILD:$SYSTEM_ADDONS" \
  FLYPE_LEXICON="$LEXICON" FLYPE_ENGLISH="$ENGLISH" FLYPE_LM="${FLYPE_LM:-}" FLYPE_LOG="${FLYPE_LOG:-0}" \
  FLYPE_USER_DICT="$ROOT/user.tsv" \
  FLYPE_VOICE_WAV="$VOICE_WAV" FLYPE_VOICE_MODEL_DIR="$VOICE_MODEL_DIR" \
  DBUS_SESSION_BUS_ADDRESS="${FLYPE_HARNESS_DBUS:-}" \
  FLYPE_HARNESS_REAL_CLIPBOARD="${FLYPE_HARNESS_REAL_CLIPBOARD:-}" \
  FLYPE_HARNESS_SHOW="${FLYPE_HARNESS_SHOW:-}" FLYPE_VOICE_EXPECT="${FLYPE_VOICE_EXPECT:-}" \
  "$BUILD/flype-key-harness"
