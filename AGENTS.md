# AGENTS.md

本文件会被 prux 在启动时自动加载（项目级上下文）。动手前至少读「本地环境」与「硬约定」两节——
前者违反会打断用户正在用的输入法，后者违反照样能编译能过测试，只有人眼会翻车。

## 本地环境：fcitx5 是用户正在用的东西

- **禁止 `killall fcitx5` / `pkill fcitx5` / `fcitx5 -r` 重启用户的实例。** 那会打断用户正在打的字、
  丢掉没上屏的 preedit，而且用户很可能正在别的窗口靠它输入中文。
- 插件行为测试只走 `scripts/addon-test.sh`：它起一个私有、无界面、无会话总线的 fcitx5 实例，
  `HOME` / `XDG_*` 全在 `mktemp` 里，词库也走环境变量。不要自己 `fcitx5 -d` 起实例来测。
- 用户配置在 `~/.config/fcitx5/conf/flype.conf` 与 `classicui.conf`（主题、候选字号在后者）。
  要试配置就设 `FLYPE_CONFIG` 指到临时文件，**别直接改用户的配置**验证。
- 装插件、主题、设置界面走 `sudo PREFIX=/usr scripts/install-addon.sh`，不要手工往 `/usr` 里 cp 调试产物。
- 本机 fcitx5 是 5.1.19（`pkg-config --modversion Fcitx5Core`）。

## 仓库地图

Rust workspace，四个成员：`core`、`voice`、`settings`、`fcitx5-addon`。
`default-members` 不含 `fcitx5-addon`——编它需要 fcitx5 开发包，所以 `cargo build` 默认不碰 C++。

| 位置 | 内容 | 注意 |
|---|---|---|
| `core/src/session.rs` | 输入会话：编辑区、选词栈、按键规则、候选面板、联想 | **输入行为的唯一真源**，见「硬约定」 |
| `core/src/session_ffi.rs` | 会话的 C 接口（`flype_session_*`）、`FlypeSessionConfig` | 结构体靠首字段 `size` 做版本兼容 |
| `core/src/ffi.rs` | 引擎的 C 接口（`flype_engine_*`） | 不透明句柄 + 线程本地错误串 |
| `core/src/{lexicon,lm,wordlm,nnlm,beam,graph,rank,syllable,learn}` | 引擎本体：音节图 → 束搜索 → 打分 → 候选、学习、神经重排 | 纯 Rust，有单测 |
| `core/src/{emoji,calc,datetime,symbols,convert,autospace,clipboard,preedit}` | 候选里的附加功能 | 各有单测 |
| `core/src/bin/*` | 工具：`flype-eval` / `check` / `demo` / `bench` / `show` / `train` / `lmbuild` / `probe` | 需要真词库，见 README |
| `core/include/flype_core.h` | **C ABI 的契约**（版本、生命周期、结构体布局） | 改接口先读这里 |
| `voice/src/` | 语音：热键状态机、cpal 采集、重采样/VAD、ASR 渠道、上屏 | 与平台解耦，可无麦克风单测 |
| `voice/src/wtlocal/` | 微信输入法离线模型的纯 Rust 推理（引入的第三方代码 + 适配） | 不是本仓风格，改前看模块顶部注释 |
| `fcitx5-addon/src/flype_engine.cpp/.h` | fcitx5 外壳（C++）：按键翻译、候选框绘制、配置项、D-Bus 接口 | 引擎还在往 Rust 搬，见 `session.rs` 顶部 |
| `fcitx5-addon/build.rs` | 探测 fcitx5、`cc` 编 C++、生成清单与 `FLYPE_ADDON_VERSION` 宏 | 取代了原来的 CMakeLists |
| `settings/src/` | egui 设置界面，读写 `flype.conf` / `classicui.conf` / `custom.tsv` | 只跑 Wayland |
| `fcitx5-addon/test/key_harness.cpp` | 私有 fcitx5 实例里的按键行为用例（137KB 的大头） | 由 `addon-test.sh` 驱动，不是 `cargo test` |
| `core/data/` `data/` `dist/data/` | demo 词典、词库生成输入、装好的词库 | 真词库不进仓库 |
| `tools/*.py` `scripts/make-data.sh` | 词库/语言模型生成流水线（一次性，产物是数据） | 别手改生成物 |
| `WowType/` | 上游原实现（135MB 的 clone），**未纳入 git** | 只读参考，别写、别提交 |

## 硬约定

- **输入行为只往 `core/src/session.rs` 加。** 目标是外壳（`flype_engine.cpp`）只做「平台按键 → 会话 →
  按结果画界面、上屏」。目前还没搬完：`Session::key` 返回 `handled = false` 时外壳会用自己的旧逻辑接手。
  所以看到同一个键在两处都有逻辑，是过渡状态，不是冗余；改行为前先确认这个键归谁。
- **改 C ABI 只按头文件的规矩来**：同一个主版本只加函数、只在结构体末尾加字段，删函数或改参数含义
  要升主版本（`FLYPE_ABI_VERSION_MAJOR`）。外壳启动时用 `flype_abi_version()` 比主版本。
  `FlypeKey` / `FlypeSessionConfig` 传 `size`，新字段必须在 `Default`、`to_config`/`from_config` 三处一起加，
  并标上「x.y 起」的版本注释。头文件里引用的 `docs/ABI.md`、`docs/PORTING.md` 已经不在仓库里，
  规则以头文件和现有字段注释为准。
- **词库与模型不手改、不自造**：`dist/data/*` 由 `scripts/make-data.sh` 产出，`assets` 类数据没有第二份副本。调试要小词库就用 `core/data/demo_lexicon.tsv` 或 `FLYPE_LEXICON` 指临时文件。
- **运行时数据全走环境变量**：`FLYPE_LEXICON` / `FLYPE_ENGLISH` / `FLYPE_LM` / `FLYPE_NNLM` /
  `FLYPE_USER_DICT` / `FLYPE_CONFIG` / `FLYPE_LOG` / `FLYPE_VOICE_MODEL_DIR`。测试和脚本都靠它们隔离，
  不要在代码里写死路径或读 `$HOME` 绕过。
- **edition 2024**：`extern "C"` 块要写 `unsafe extern "C"`，导出符号要写 `#[unsafe(no_mangle)]`，
  否则 `cargo check` 直接报错。
- **C++ 用 `cc` 编成静态库再由 cdylib 链接**，插件工厂符号被 `build.rs` 改名成 `flype_cpp_factory`，
  再由 `fcitx5-addon/src/lib.rs` 的同名导出转一手；删掉那个转手会导致 `--gc-sections` 把整个 C++ 库丢掉。
- **日志只记启动/载入/出错**，绝不记用户打的字、候选词、语音识别结果（`flype_log.h` 的注释是承诺，
  用户会把 `~/.local/state/flype/flype.log` 贴到 issue 里）。`FLYPE_LOG=0` 关文件日志。

## 构建 / 测试

```sh
cargo build --release --workspace        # core + voice + settings（不含 fcitx5-addon）
cargo test --workspace                  # core/voice/settings；--workspace 会连带编 fcitx5-addon，需要 fcitx5 开发包
cargo test -p flype-core -p flype-voice -p flype-config   # 只测 Rust 部分，不需要 fcitx5 头
cargo build --release -p flype-fcitx5   # 插件 → target/release/libflype.so
scripts/abi-smoke.sh                    # C ABI 冒烟（自己造小词库，不用 dist/data）
scripts/addon-test.sh 词库.tsv english.txt   # 插件按键行为，可用 FLYPE_VOICE_WAV 加跑语音
scripts/build-linux.sh --addon          # 上面这些一次跑完
```

`addon-test.sh` 必须带 `--lib` 编 `flype-fcitx5`（脚本里已经带了）：fcitx5 是从 `target/release` 按
`FCITX_ADDON_DIRS` 载入 `libflype.so` 的，只编 `--bin flype-key-harness` 会留下旧 `.so`，
表现为「Version 是插件的版本」这类用例莫名失败。

## 命名与已知边界

- 设置界面的**包名、二进制、安装名、desktop 的 `Exec` 统一是 `flype-config`**（`settings/` 目录，产物
  `target/release/flype-config`）。别再写 `flype-settings`——旧名字曾让 `install-addon.sh` 找不到产物、静默跳过安装。
- 语音是 `core` 的必选依赖，所以 `core` 硬依赖 ALSA（`libasound2-dev`），纯排序工具（`flype-eval` 等）也会链上它。
