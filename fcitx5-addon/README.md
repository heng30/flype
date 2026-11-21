# flype-fcitx5 插件

fcitx5 的输入法外壳：把 fcitx5 的按键翻成会话的键码（`flype_session_key`），按会话给的
`FlypeFrame` 画编辑区和候选框、上屏。所有输入行为（选词栈、标点、翻页、工具栏、表情面板、中英文）
都在 `core/` 的会话里，这里只管 fcitx5 的接线。

Wayland 下编辑区走 text-input 协议，剪贴板复制走 `wl-copy`。

## 构建

```sh
cargo build --release -p flype-fcitx5        # 出 target/release/libflype.so
sudo PREFIX=/usr scripts/install-addon.sh    # 装插件、清单、数据和设置界面
```

引擎还是 C++（`src/flype_engine.cpp`），`build.rs` 用 cc 编它、pkg-config 或库文件
反推 fcitx5 的头和库，所以不需要 CMake。依赖：`fcitx5`（≥ 5.1.11，要开发头文件）、
C++20 编译器、`libasound2`（语音）。
词库、语言模型由 `scripts/make-data.sh` 生成到 `dist/data`，安装时一起装进
`$PREFIX/share/flype`；没有它们时退到仓库里的 demo 词典。

## 按键测试

```sh
scripts/addon-test.sh 词库.tsv english.txt
```

起一个私有的、无界面 fcitx5 实例跑 `test/key_harness.cpp`，不影响正在用的输入法。
脚本自己会 `cargo build --release -p flype-fcitx5 --features key-harness`。

## 环境变量

| 变量 | 作用 |
|---|---|
| `FLYPE_LEXICON` / `FLYPE_ENGLISH` / `FLYPE_LM` | 换词库、英文词表、词级语言模型 |
| `FLYPE_USER_DICT` / `FLYPE_CUSTOM` | 用户词库、自定义词文件 |
| `FLYPE_VOICE_MODEL_DIR` | 语音模型目录（默认 `~/.local/share/flype/wtlocal`） |
| `FLYPE_VOICE_KEY` / `FLYPE_EMOJI_KEY` / `FLYPE_SHIFT_TOGGLE` | 临时覆盖设置里的键 |
| `FLYPE_LOG` | 1 开详细日志 |
| `FLYPE_NO_OPEN_SETTINGS` | 设了就不在缺语音模型时自动打开设置界面 |

## 设置

`~/.config/fcitx5/conf/flype.conf`，键名和默认值见 `src/flype_engine.h` 的 `FlypeConfig`。
改完让 fcitx5 重新读：

```sh
gdbus call --session --dest org.fcitx.Fcitx5 --object-path /controller \
  --method org.fcitx.Fcitx.Controller1.ReloadAddonConfig flype
```

图形设置界面是 `settings/`（Rust + egui），装成 `flype-config`。
