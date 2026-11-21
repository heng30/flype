# flype

一个 Wayland 下的 fcitx5 拼音输入法：长句一次打完、打一半就有词、按错键也能认出来，越用越懂你。

从 [WowType](https://github.com/Gloridust/WowType) 移植并重新组织：只保留 Linux/Wayland 这条线，
去掉翻译、macOS 外壳和发行版打包脚本；设置界面用 Rust + egui 重写。

## 特性

- **整句输入**：`xiangquzijieshangban` → 「想去字节上班」；句首词可以先上屏一段，剩下的接着选。
- **半成品也有词**：`zhongg` 就有「中国」。
- **纠错**：相邻键、顺序颠倒、多按漏按（`zhognguo` → 中国）。
- **学习**：记住选过的词和常连着用的词，30 天不用会淡掉。
- **简拼、省略韵母、中英混打**：`bjdx` → 北京大学，`github` → GitHub。
- **横排候选条 + 展开面板**、`v` + 空格工具栏（剪贴板、常用语、符号、表情）。
- **emoji 搜索**、日期时间（`riqi`、`shijian`、`xingqi`）、计算器（`v1+2*3`）、大写金额。
- **中英文之间自动加空格**（默认关）、16 组模糊音。
- **语音输入**：按住右 Alt 说话，松开上屏（微信输入法的离线模型，本机识别，模型在设置里下载）。
- 全部本地运行，候选在几毫秒内出来。

## 结构

```
core/            Rust 引擎 + 输入会话（音节切分、词库、语言模型、纠错、候选排序、学习）
voice/           语音：模型推理（纯 Rust）+ cpal 采集
fcitx5-addon/    fcitx5 外壳（C++），Wayland 下由它接线
settings/        图形设置界面（Rust + egui），装成 flype-config
scripts/         构建、按键测试、词库生成
data/ tools/     词库生成的输入数据与转换脚本
themes/ icons/   fcitx5 主题、图标
```

输入行为（选词栈、退格撤销、标点、翻页、工具栏、emoji、中英文切换、自动空格）都在
`core/src/session.rs` 的会话里，外壳只负责把平台按键翻成会话的键码、按会话给的结果画界面和上屏。

## 构建

```sh
cargo build --release --workspace            # 核心、语音、设置界面
cargo build --release -p flype-fcitx5        # fcitx5 插件（引擎暂时还是 C++，build.rs 编）
sudo PREFIX=/usr scripts/install-addon.sh    # 装插件、清单、数据和设置界面
```

`scripts/build-linux.sh --addon` 一次跑完构建和测试（见 `fcitx5-addon/README.md` 的依赖表）。
装好以后在 fcitx5 的配置中心里把「Flype」加进输入法组，然后重启 fcitx5。

词库不随仓库分发：`scripts/make-data.sh` 从雾凇拼音生成（会 `git clone`，并从 GitHub Release 下载
`words.lm`、`nnlm.bin`）。没有词库时用仓库里的 demo 词典。

需要的数据放在 `dist/data`，`scripts/install-addon.sh` 时一起装进 `share/flype`。

## 设置

`flype-config`（或 `~/.config/fcitx5/conf/flype.conf`）改候选排列、每页个数、翻页键、输入方案、
模糊音、中英文切换、标点、自动空格、emoji、语音、自定义词；保存后 fcitx5 立刻重新读取。
键名和默认值见 `fcitx5-addon/src/flype_engine.h`。

## 测试

```sh
cargo test --workspace                        # 引擎、会话按键、设置界面
scripts/abi-smoke.sh                          # C 接口冒烟测试
scripts/addon-test.sh 词库.tsv english.txt     # fcitx5 外壳的按键行为（私有实例）
```

## Nixos 安装测试

```
nix flake lock --update-input flype # 更新软件源信息
sudo nixos-rebuild switch --flake /etc/nixos#your-hostname # 安装软件
```

## 数据来源与许可证

- 词库：雾凇拼音（GPL-3.0），由 `scripts/make-data.sh` 生成，许可证随数据分发。
- 语料词频：Tatoeba 中文句子（CC BY 2.0 FR），`data/tatoeba-freq.tsv`。
- 语音识别模型和整句增强模型是腾讯的资源，不随仓库或安装包分发。
