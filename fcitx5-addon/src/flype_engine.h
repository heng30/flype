#ifndef FLYPE_FCITX5_ENGINE_H
#define FLYPE_FCITX5_ENGINE_H

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <future>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include <fcitx-config/configuration.h>
#include <fcitx-config/enum.h>
#include <fcitx-config/option.h>
#include <fcitx-config/rawconfig.h>
#include <fcitx-utils/dbus/bus.h>
#include <fcitx-utils/dbus/matchrule.h>
#include <fcitx-utils/dbus/objectvtable.h>
#include <fcitx-utils/event.h>
#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx-utils/trackableobject.h>
#include <fcitx/inputcontextproperty.h>
#include <fcitx/inputmethodengine.h>

#include "flype_core.h"

namespace fcitx {
// fcitx5 的实例：输入法引擎、配置文件、输入框管理器都挂在它上面
class Instance;
// 一个输入框（每个应用的每个输入框一个），会话状态和候选框都从它拿
class InputContext;
// 输入框上的事件（获得焦点、切输入法、重置），activate / deactivate / reset
// 的入参
class InputContextEvent;
// 候选框：候选列表、上下两行提示文字、翻页按钮
class InputPanel;
// 一段带格式的文本（编辑区、提示文字都用它）
class Text;
// 输入法条目（Flype 的名字、语言、图标），引擎回调的入参
class InputMethodEntry;
// 一次按键：按键、状态位、按下还是松开、被没被吃掉
class KeyEvent;
} // namespace fcitx

// 每个输入框的会话状态（外壳这一侧），定义在 flype_engine.cpp
class FlypeState;

// 候选横排还是竖排一列
enum class FlypeLayout { Horizontal, Vertical };
FCITX_CONFIG_ENUM_NAME(FlypeLayout, "Horizontal", "Vertical");

/// 选项里显示中文（存盘仍是英文名）。
struct FlypeLayoutAnnotation {
  // 不让 fcitx5 自动导出选项说明，改由下面的 dumpDescription 写中文名
  bool skipDescription() const { return false; }
  // 不把选项说明写进配置文件（只影响设置界面）
  bool skipSave() const { return false; }
  // 导出候选值的中文名：横排、竖排
  void dumpDescription(fcitx::RawConfig &config) const {
    config.setValueByPath("EnumI18n/0", "横排");
    config.setValueByPath("EnumI18n/1", "竖排");
  }
};

/// 打字途中方向键做什么（#129）。
enum class FlypeArrowKeys { Candidates, Caret };
FCITX_CONFIG_ENUM_NAME(FlypeArrowKeys, "Candidates", "Caret");

// 方向键选项的中文名：都用来选词 / 左右键移动光标
struct FlypeArrowKeysAnnotation {
  // 不让 fcitx5 自动导出选项说明，改由下面的 dumpDescription 写中文名
  bool skipDescription() const { return false; }
  // 不把选项说明写进配置文件（只影响设置界面）
  bool skipSave() const { return false; }
  // 导出候选值的中文名：都用来选词、左右键移动光标
  void dumpDescription(fcitx::RawConfig &config) const {
    config.setValueByPath("EnumI18n/0", "都用来选词");
    config.setValueByPath("EnumI18n/1", "左右键移动光标");
  }
};

/// 分号、引号选第 2、3 个候选。
enum class FlypeSelectKeys { Off, Semicolon, SemicolonQuote };
FCITX_CONFIG_ENUM_NAME(FlypeSelectKeys, "Off", "Semicolon", "SemicolonQuote");

// 分号、引号选词选项的中文名
struct FlypeSelectKeysAnnotation {
  // 不让 fcitx5 自动导出选项说明，改由下面的 dumpDescription 写中文名
  bool skipDescription() const { return false; }
  // 不把选项说明写进配置文件（只影响设置界面）
  bool skipSave() const { return false; }
  // 导出候选值的中文名：不用、分号选第 2 个、引号选第 3 个
  void dumpDescription(fcitx::RawConfig &config) const {
    config.setValueByPath("EnumI18n/0", "不用");
    config.setValueByPath("EnumI18n/1", "分号选第 2 个");
    config.setValueByPath("EnumI18n/2",
                          "分号选第 2 个、引号选第 3 个（引号不再当隔音符）");
  }
};

/// 标点样式：中文标点 / ASCII 标点。
enum class FlypePunctStyle { Fullwidth, Halfwidth };
FCITX_CONFIG_ENUM_NAME(FlypePunctStyle, "Fullwidth", "Halfwidth");

// 标点样式选项的中文名：全角 / 半角
struct FlypePunctStyleAnnotation {
  // 不让 fcitx5 自动导出选项说明，改由下面的 dumpDescription 写中文名
  bool skipDescription() const { return false; }
  // 不把选项说明写进配置文件（只影响设置界面）
  bool skipSave() const { return false; }
  // 导出候选值的中文名：全角（中文标点）、半角（英文标点）
  void dumpDescription(fcitx::RawConfig &config) const {
    config.setValueByPath("EnumI18n/0", "全角（中文标点）");
    config.setValueByPath("EnumI18n/1", "半角（英文标点）");
  }
};

/// 单按哪个 Shift 切换中英文。
enum class FlypeShiftToggle { Both, Left, Right, Off };
FCITX_CONFIG_ENUM_NAME(FlypeShiftToggle, "Both", "Left", "Right", "Off");

// 单按 Shift 切换中英文选项的中文名
struct FlypeShiftAnnotation {
  // 不让 fcitx5 自动导出选项说明，改由下面的 dumpDescription 写中文名
  bool skipDescription() const { return false; }
  // 不把选项说明写进配置文件（只影响设置界面）
  bool skipSave() const { return false; }
  // 导出候选值的中文名：左右 Shift 都行、左 Shift、右 Shift、不用
  void dumpDescription(fcitx::RawConfig &config) const {
    config.setValueByPath("EnumI18n/0", "左右 Shift 都行");
    config.setValueByPath("EnumI18n/1", "左 Shift");
    config.setValueByPath("EnumI18n/2", "右 Shift");
    config.setValueByPath("EnumI18n/3", "不用 Shift 切换");
  }
};

/// 切到 Flype 时用中文还是英文（#158、#121）。
enum class FlypeInitialMode { Chinese, English, Last };
FCITX_CONFIG_ENUM_NAME(FlypeInitialMode, "Chinese", "English", "Last");

// 切到 Flype 时的模式选项的中文名
struct FlypeInitialModeAnnotation {
  // 不让 fcitx5 自动导出选项说明，改由下面的 dumpDescription 写中文名
  bool skipDescription() const { return false; }
  // 不把选项说明写进配置文件（只影响设置界面）
  bool skipSave() const { return false; }
  // 导出候选值的中文名：中文、英文、和上次切走时一样
  void dumpDescription(fcitx::RawConfig &config) const {
    config.setValueByPath("EnumI18n/0", "中文");
    config.setValueByPath("EnumI18n/1", "英文");
    config.setValueByPath("EnumI18n/2", "和上次切走时一样");
  }
};

/// ~/.config/fcitx5/conf/flype.conf。fcitx5 配置工具和 flype-config
/// 都改这个文件； 同名环境变量（FLYPE_FUZZY 等）设了的话优先，方便测试。
FCITX_CONFIGURATION(
    FlypeConfig,
    // 候选横排还是竖排一列
    fcitx::Option<FlypeLayout, fcitx::NoConstrain<FlypeLayout>,
                  fcitx::DefaultMarshaller<FlypeLayout>, FlypeLayoutAnnotation>
        layout{this, "CandidateLayout", "候选排列", FlypeLayout::Horizontal};
    // 方向键用来选词还是移光标
    fcitx::Option<FlypeArrowKeys, fcitx::NoConstrain<FlypeArrowKeys>,
                  fcitx::DefaultMarshaller<FlypeArrowKeys>,
                  FlypeArrowKeysAnnotation>
        arrowKeys{this, "ArrowKeys", "方向键", FlypeArrowKeys::Candidates};
    // 一页显示几个候选（3–9，超出范围夹到边上）
    fcitx::Option<int, fcitx::IntConstrain> pageSize{
        this, "PageSize", "每页候选个数", 9, fcitx::IntConstrain(3, 9)};
    // 候选总数上限（翻页能翻到多少个；越大每键算得越慢）
    fcitx::Option<int, fcitx::IntConstrain> maxCandidates{
        this, "MaxCandidates", "候选总数上限", 100,
        fcitx::IntConstrain(10, 1000)};
    // 用 - 和 = 翻页
    fcitx::Option<bool> pageKeysMinus{this, "PageKeysMinusEqual",
                                      "翻页键：- =", true};
    // 用 [ 和 ] 翻页
    fcitx::Option<bool> pageKeysBracket{this, "PageKeysBracket", "翻页键：[ ]",
                                        false};
    // 用 , 和 . 翻页
    fcitx::Option<bool> pageKeysComma{this, "PageKeysCommaPeriod",
                                      "翻页键：, .", false};
    // 用 Shift+Tab 和 Tab 翻页
    fcitx::Option<bool> pageKeysTab{this, "PageKeysTab",
                                    "翻页键：Shift+Tab Tab", false};
    // 模糊音总开关，下面的分组只在它打开时生效
    fcitx::Option<bool> fuzzy{this, "Fuzzy", "模糊音（下面几组分别开关）",
                              false};
    // 模糊音：z 和 zh 当一个声母
    fcitx::Option<bool> fuzzyZh{this, "FuzzyZh", "模糊音：z = zh", true};
    // 模糊音：c 和 ch 当一个声母
    fcitx::Option<bool> fuzzyCh{this, "FuzzyCh", "模糊音：c = ch", true};
    // 模糊音：s 和 sh 当一个声母
    fcitx::Option<bool> fuzzySh{this, "FuzzySh", "模糊音：s = sh", true};
    // 模糊音：n 和 l 当一个声母
    fcitx::Option<bool> fuzzyNL{this, "FuzzyNL", "模糊音：n = l", false};
    // 模糊音：f 和 h 当一个声母
    fcitx::Option<bool> fuzzyFH{this, "FuzzyFH", "模糊音：f = h", false};
    // 模糊音：r 和 l 当一个声母
    fcitx::Option<bool> fuzzyRL{this, "FuzzyRL", "模糊音：r = l", false};
    // 模糊音：an 和 ang 当一个韵母
    fcitx::Option<bool> fuzzyAn{this, "FuzzyAn", "模糊音：an = ang", true};
    // 模糊音：en 和 eng 当一个韵母
    fcitx::Option<bool> fuzzyEn{this, "FuzzyEn", "模糊音：en = eng", true};
    // 模糊音：in 和 ing 当一个韵母
    fcitx::Option<bool> fuzzyIn{this, "FuzzyIn", "模糊音：in = ing", true};
    // 模糊音：ian 和 iang 当一个韵母
    fcitx::Option<bool> fuzzyIan{this, "FuzzyIan", "模糊音：ian = iang", true};
    // 模糊音：uan 和 uang 当一个韵母
    fcitx::Option<bool> fuzzyUan{this, "FuzzyUan", "模糊音：uan = uang", true};
    // 模糊音：un 和 ong 当一个韵母
    fcitx::Option<bool> fuzzyUn{this, "FuzzyUn", "模糊音：un = ong", false};
    // 模糊音：an 和 ai 当一个韵母
    fcitx::Option<bool> fuzzyAnAi{this, "FuzzyAnAi", "模糊音：an = ai", false};
    // 模糊音：huang 和 wang 当一个音节
    fcitx::Option<bool> fuzzyHuangWang{this, "FuzzyHuangWang",
                                       "模糊音：huang = wang", false};
    // 模糊音：un 和 iong 当一个韵母
    fcitx::Option<bool> fuzzyUnIong{this, "FuzzyUnIong", "模糊音：un = iong",
                                    false};
    // 模糊音：eng 和 ong 当一个韵母
    fcitx::Option<bool> fuzzyEngOng{this, "FuzzyEngOng", "模糊音：eng = ong",
                                    false};
    // 按错相邻键（比如把 ng 打成 mg）也给出候选
    fcitx::Option<bool> typo{this, "TypoCorrection", "按错键自动纠正", true};
    // 扩展 B 区以后的生僻字排在候选末尾，翻页才看得到
    fcitx::Option<bool> rareExt{this, "RareExtChars",
                                "候选末尾列出扩展 B 区以后的生僻字", false};
    // 单按哪个 Shift 切中英文
    fcitx::Option<FlypeShiftToggle, fcitx::NoConstrain<FlypeShiftToggle>,
                  fcitx::DefaultMarshaller<FlypeShiftToggle>,
                  FlypeShiftAnnotation>
        shiftToggle{this, "ShiftToggle", "单按 Shift 切换中英文",
                    FlypeShiftToggle::Both};
    // 切到 Flype 时用中文、英文，还是和上次切走时一样
    fcitx::Option<FlypeInitialMode, fcitx::NoConstrain<FlypeInitialMode>,
                  fcitx::DefaultMarshaller<FlypeInitialMode>,
                  FlypeInitialModeAnnotation>
        initialMode{this, "InitialMode", "切到 Flype 时",
                    FlypeInitialMode::Chinese};
    // 打标点时输出中文标点还是英文标点
    fcitx::Option<FlypePunctStyle, fcitx::NoConstrain<FlypePunctStyle>,
                  fcitx::DefaultMarshaller<FlypePunctStyle>,
                  FlypePunctStyleAnnotation>
        punctStyle{this, "PunctuationStyle", "标点样式",
                   FlypePunctStyle::Fullwidth};
    // 分号、引号选第 2、3 个候选（引号那项不再当隔音符）
    fcitx::Option<FlypeSelectKeys, fcitx::NoConstrain<FlypeSelectKeys>,
                  fcitx::DefaultMarshaller<FlypeSelectKeys>,
                  FlypeSelectKeysAnnotation>
        selectKeys{this, "SelectKeys", "分号、引号选词", FlypeSelectKeys::Off};
    // 用 [ ] 上屏当前候选的第一个、最后一个字（以词定字）
    fcitx::Option<bool> pickChar{
        this, "PickChar", "以词定字：打拼音时 [ ] 上屏候选的第一个、最后一个字",
        true};
    // 上屏后给出常跟在后面的词（Tab 选）
    fcitx::Option<bool> association{
        this, "Association",
        "联想：上屏以后给出常跟在后面的词（Tab 选，数字、空格照常）", false};
    // 打 riqi / shijian / xingqi 给当前日期时间
    fcitx::Option<bool> dateTime{
        this, "DateTime", "打 riqi、shijian、xingqi 时给出当前日期时间", true};
    // v 开头的算式直接算出结果
    fcitx::Option<bool> calculator{this, "Calculator", "v 开头算算式（v1+2*3）",
                                   true};
    // v + 空格打开工具栏（剪贴板、常用语、符号、表情）
    fcitx::Option<bool> toolbar{
        this, "Toolbar", "v + 空格打开工具栏（剪贴板、常用语、符号、表情）",
        true};
    // 默认关：fcitx5 的 clipboard 插件不告诉我们哪条是密码管理器复制的密码
    fcitx::Option<bool> clipboardHistory{
        this, "ClipboardHistory", "记录剪贴板历史（只在内存里，最多 20 条）",
        false};
    // 汉字和英文、数字之间自动补一个空格（下面的字母 / 数字两项的总开关）
    fcitx::Option<bool> autoSpace{this, "AutoSpace",
                                  "汉字和英文、数字之间自动加空格", false};
    // 自动补空格：汉字和英文字母之间
    fcitx::Option<bool> autoSpaceLetters{this, "AutoSpaceLetters",
                                         "汉字和英文字母之间", true};
    // 自动补空格：汉字和数字之间
    fcitx::Option<bool> autoSpaceDigits{this, "AutoSpaceDigits",
                                        "汉字和数字之间", true};
    // 打开语音输入（按住语音键说话）
    fcitx::Option<bool> voice{this, "Voice", "语音输入", true};
    // 语音输入的热键，默认右 Alt
    fcitx::Option<fcitx::Key, fcitx::KeyConstrain> voiceKey{
        this, "VoiceKey", "语音输入键（按住说话，或点一下开始）",
        fcitx::Key(FcitxKey_Alt_R),
        fcitx::KeyConstrain({fcitx::KeyConstrainFlag::AllowModifierOnly,
                             fcitx::KeyConstrainFlag::AllowModifierLess})};
    // 识别结果同时复制一份到剪贴板
    fcitx::Option<bool> voiceCopy{this, "VoiceCopy",
                                  "语音识别的结果同时复制到剪贴板", false};
    // 打拼音或英文时推荐相关的 emoji
    fcitx::Option<bool> emojiSuggest{this, "EmojiSuggest", "打字时推荐 emoji",
                                     true};
    // 打开表情面板的快捷键，默认 Super+句号
    fcitx::Option<fcitx::Key, fcitx::KeyConstrain> emojiKey{
        this, "EmojiKey", "打开表情面板的快捷键", fcitx::Key("Super+period"),
        fcitx::KeyConstrain(fcitx::KeyConstrainFlags{
            fcitx::KeyConstrainFlag::AllowModifierLess})};
    // 另设的切换中 / 英文快捷键（#143、#87），默认不用；单按 Shift 照常。fcitx5
    // 自己的热键（比如 切换输入法的 Ctrl+空格）先被 fcitx5 拿走，Flype
    // 收不到，设置界面会提示
    fcitx::Option<fcitx::Key, fcitx::KeyConstrain> toggleKey{
        this, "ToggleKey", "切换中 / 英文的快捷键", fcitx::Key(),
        fcitx::KeyConstrain(fcitx::KeyConstrainFlags{
            fcitx::KeyConstrainFlag::AllowModifierLess})};
    // 词库文件（拼音<TAB>词<TAB>频次）。Flype 设置里切换「万象」时写成
    // ~/.local/share/flype/dicts/wanxiang/lexicon.tsv；文件不在就用默认的
    fcitx::Option<std::string> lexicon{this, "Lexicon",
                                       "词库文件（留空用自带的雾凇词库）", ""};
    // 「自定义词和更多设置」按钮，点了开 flype-config
    fcitx::ExternalOption settings{this, "Settings", "自定义词和更多设置",
                                   "flype-config"};);

/// 键名、默认值、范围、可选值以 spec/config.json
/// 为准（设置契约），改这里要先改规格； fcitx5-addon/test/config_dump.cpp
/// 把上面的导出来，scripts/test-config-contract.py 对照。

/// 把 flype.conf 读成的 RawConfig 装进 config，文件里没写的项是默认值（和
/// fcitx::readAsIni 一样）。 数字项超出范围时夹到边上（三端一样，见
/// spec/config.json 的 readRules.outOfRange）。 fcitx5 自己的 IntConstrain
/// 是不认、回到默认值；认不出的数字（abc、空的）还是默认值。
inline void clampFlypeInt(fcitx::Option<int, fcitx::IntConstrain> &option,
                          const fcitx::RawConfig &raw, int min, int max) {
  const std::string *text = raw.valueByPath(option.path());
  if (!text || text->empty()) {
    return;
  }
  char *end = nullptr;
  const long long n = std::strtoll(text->c_str(), &end, 10);
  if (end == text->c_str() || *end != '\0') {
    return;
  }
  if (n < min) {
    option.setValue(min);
  } else if (n > max) {
    option.setValue(max);
  }
}

/// 去掉两边的空格和 Tab（fcitx5 读文件时值前面的空格会留着：「Key=  值」读成「
/// 值」）。
inline std::string trimFlypeValue(const std::string &text) {
  const auto begin = text.find_first_not_of(" \t");
  if (begin == std::string::npos) {
    return "";
  }
  return text.substr(begin, text.find_last_not_of(" \t") - begin + 1);
}

/// 最上面一层的设置项，键和值两边的空格去掉（三端一样：「PageSize = 5」读成
/// 5，见 spec/config.json 的 readRules）。 [分组]（[Hotkey]
/// 等）是带子项的，不是设置项，不要。
inline fcitx::RawConfig flypeTopLevel(const fcitx::RawConfig &raw) {
  fcitx::RawConfig out;
  for (const auto &key : raw.subItems()) {
    auto item = raw.get(key);
    if (!item || !item->subItems().empty()) {
      continue;
    }
    out.setValueByPath(trimFlypeValue(key), trimFlypeValue(item->value()));
  }
  return out;
}

// 读 flype.conf：先去掉键值两边的空格，再装进 config，超范围的数字夹到边上
inline void loadFlypeConfig(FlypeConfig &config, const fcitx::RawConfig &file) {
  const fcitx::RawConfig raw = flypeTopLevel(file);
  config.load(raw);
  // 范围和上面 IntConstrain 的一样；不一样时契约测试（PageSize=12 读成
  // 9）会报错
  clampFlypeInt(config.pageSize, raw, 3, 9);
  clampFlypeInt(config.maxCandidates, raw, 10, 1000);
}

// 下面 FlypeControl 的 D-Bus 方法要回调到引擎（定义在后面）
class FlypeEngine;

/// 后台载入的结果：核心（失败是空的），还有哪个用户文件在、却没读出来。
struct FlypeLoaded {
  // 后台载入好的核心句柄，没载入或载入失败时是空
  FlypeEngineHandle *core = nullptr;
  // user.tsv 在、这次却没读出来（存之前先把原文件改名备份）
  bool userDictUnread = false;
  // history.tsv 在、这次却没读出来（同上）
  bool historyUnread = false;
  /// 载入的整句增强模型（路径 + 大小 + 修改时间，没载入是空），reloadConfig
  /// 按它认模型换没换
  std::string nnlmStamp;
};

/// 会话总线上的 org.flype.Input：外部命令切中 / 英（vim
/// 退出插入模式切英文，#98）。
///   busctl --user call org.flype.Input /org/flype/Input org.flype.Input1
///   SetMode s en
/// Version
/// 返回正在跑的插件的版本：升级以后设置界面拿它和装好的版本比，不一样（或者没有这个方法，
/// 是 0.4.0 及以前的插件）就提醒重启输入法。
class FlypeControl : public fcitx::dbus::ObjectVTable<FlypeControl> {
public:
  // 记下要回调的引擎（引擎比它活得久）
  explicit FlypeControl(FlypeEngine *engine) : engine_(engine) {}
  // 现在的中 / 英模式：zh 或 en
  std::string mode();
  /// zh / en / toggle，返回切完以后的模式。
  std::string setMode(const std::string &mode);
  // 插件的版本号，设置界面拿来和装好的版本比
  std::string version();

private:
  /// 回调的引擎，构造时给
  FlypeEngine *engine_;
  FCITX_OBJECT_VTABLE_METHOD(mode, "Mode", "", "s");
  FCITX_OBJECT_VTABLE_METHOD(setMode, "SetMode", "s", "s");
  FCITX_OBJECT_VTABLE_METHOD(version, "Version", "", "s");
};

// Flype 的 fcitx5 输入法引擎：把按键交给会话，再把会话的结果画到候选框
class FlypeEngine final : public fcitx::InputMethodEngineV2 {
public:
  // 读配置文件、在后台载入词库（不卡 fcitx5 启动）
  explicit FlypeEngine(fcitx::Instance *instance);
  // 先让会话放开引擎，再等后台把用户词库存完
  ~FlypeEngine() override;

  // 所有按键的总入口：语音键、Shift、表情键、工具栏、中英文模式、打字各走一条路
  void keyEvent(const fcitx::InputMethodEntry &entry,
                fcitx::KeyEvent &event) override;
  // 输入框拿到焦点（或从别的输入法切回来）：清旧会话，按需要定一下中 / 英
  void activate(const fcitx::InputMethodEntry &entry,
                fcitx::InputContextEvent &event) override;
  // 失去焦点或被切走：切走时把打的字上屏，再存一次用户词库
  void deactivate(const fcitx::InputMethodEntry &entry,
                  fcitx::InputContextEvent &event) override;
  // 应用重置输入框（点了别处、失去焦点）：丢掉拼音和会话记的上文，清候选框
  void reset(const fcitx::InputMethodEntry &entry,
             fcitx::InputContextEvent &event) override;
  /// 状态栏、切换提示：「中文」「英文」，一个字的是「中」「英」。
  std::string subMode(const fcitx::InputMethodEntry &entry,
                      fcitx::InputContext &ic) override;
  // 状态栏上的一个字「中」「英」；中文也要给，不然托盘写的是输入法的名字
  std::string subModeLabelImpl(const fcitx::InputMethodEntry &entry,
                               fcitx::InputContext &ic) override;

  // fcitx5 读设置时用这个，直接指向 config_
  const fcitx::Configuration *getConfig() const override { return &config_; }
  // 设置界面保存：写进 config_ 、存盘，马上生效
  void setConfig(const fcitx::RawConfig &config) override;
  /// 重新读配置文件和自定义词（设置界面改完后经 D-Bus ReloadAddonConfig
  /// 调用）。
  void reloadConfig() override;

  // 照会话现在的状态把整个候选框重画一遍：面板、编辑区、候选列表、联想行
  void updateInputPanel(fcitx::InputContext *ic, FlypeState *state);
  /// 预编辑去向：全局开关（「在程序中显示预编辑文本」）打开且应用申报
  /// Preedit 能力时送进应用输入框（on-the-spot），否则画在候选窗顶部。
  void sendPreedit(fcitx::InputContext *ic, const fcitx::Text &preedit);
  /// 编辑区画在应用的输入框里（应用支持、全局设置也开着）。
  bool inlinePreedit(fcitx::InputContext *ic) const;
  /// 选中第 index 个候选（全局下标）：整句候选直接上屏，
  /// 句首词候选先压进已选栈，剩下的拼音继续输入。
  void commitCandidate(fcitx::InputContext *ic, FlypeState *state,
                       std::size_t index);
  /// 点了工具栏 / 表情面板的第 index 项（会话做：菜单进下一层，别的上屏）。
  void choosePanelItem(fcitx::InputContext *ic, FlypeState *state,
                       std::size_t index);
  /// 展开的候选面板里点了第 row
  /// 行：光标在这行就选光标处的词，否则选这行第一个。
  void selectGridRow(fcitx::InputContext *ic, FlypeState *state,
                     std::size_t row);
  /// 工具栏 / 表情面板 / 展开面板的候选框上点了翻页按钮、滚了滚轮：和键盘上按
  /// sym（Page Up / Page
  /// Down、上下键）一样交给会话，照会话的页和光标重画。面板的页码、光标只在会话里有一份。
  void panelListKey(fcitx::InputContext *ic, FlypeState *state,
                    fcitx::KeySym sym);

private:
  /// 词库在后台线程载入；载入好之前 core_ 为空，按键照常进编辑区。
  bool coreReady();
  /// 后台载入完了（loading_
  /// 已经就绪，或者愿意等）：收下核心和「哪个用户文件没读出来」。
  void takeLoaded();
  /// 配置里换了词库：存好用户词、清掉各输入框的编辑状态，在后台换新词库。
  /// 上一次载入还没完就先记着，载完再换（不在主线程上等）。
  void maybeSwitchLexicon();
  /// 把配置（和环境变量覆盖）算成下面的 effective 值，按需建语音句柄。
  void applyConfig();
  /// 词库载入好以后，把开关和自定义词交给核心。
  void applyCoreSettings();
  // 自定义词文件：FLYPE_CUSTOM 优先，否则用用户数据目录里的 custom.tsv
  std::string customWordsPath() const;
  // 这个键算不算语音热键（单个修饰键不看状态位）
  bool isVoiceKey(const fcitx::Key &key) const;
  // 这个键是不是表情面板快捷键
  bool isEmojiKey(const fcitx::Key &key) const;
  /// 设置里另设的切换键：FLYPE_KEY_ACTION_TOGGLE_MODE，不是为 NONE。
  uint32_t modeKeyAction(const fcitx::Key &key) const;
  /// 能自己切中 / 英：单按 Shift
  /// 没关，或者另设了切换键。都没有时一直是中文（切过去就切不回来）。
  bool canToggleMode() const;
  /// 单按 Shift 切换中英文：处理了（切换了）返回 true。
  /// Shift 键交给会话判断单按（别的键只报一下，打断单按）。返回
  /// true：这个键处理完了 （Shift
  /// 键都是；单按切换了模式时吃掉，否则交给应用）。
  bool handleShiftKey(fcitx::InputContext *ic, FlypeState *state,
                      fcitx::KeyEvent &event);
  /// 会话自己切了模式（单按 Shift、切换键）：上屏它放进 frame
  /// 的字，全局的模式跟上。
  void followSessionMode(fcitx::InputContext *ic, FlypeState *state,
                         const FlypeFrame *frame);
  /// 把全局的中 / 英对齐到这个输入框的会话（切英文时会话上屏打到一半的）。
  void syncMode(fcitx::InputContext *ic, FlypeState *state);
  /// 按「切到 Flype 时」（InitialMode）定中 /
  /// 英：从别的输入法切回来、启动后第一次用时调。
  void applyInitialMode(fcitx::InputContext *ic, FlypeState *state);
  /// 切到指定模式。切到英文上屏打到一半的；
  /// notify：弹出「中 / 英」提示（外部命令切的不弹，免得 vim 每按一次 Esc
  /// 闪一下）。
  void switchMode(fcitx::InputContext *ic, FlypeState *state, bool ascii,
                  bool notify);

  /// 现在的模式：「中文」「英文」；提示框和托盘上的一个字「中」「英」。
  const char *modeTitle() const;
  // 一个字的中 / 英（subModeLabelImpl 用它），和上面的完整名字配着用
  const char *modeLabel() const;
  /// 单调时钟的毫秒数（会话的 time_ms 用）。
  static uint64_t nowMs();

  // 表情面板（快捷键打开，打拼音或英文搜 emoji 和符号）
  // 工具栏（剪贴板、常用语）的条目填进候选框，光标和页码照会话的
  void fillToolPanel(fcitx::InputContext *ic, FlypeState *state);
  /// 剪贴板历史：从 fcitx5 的 clipboard
  /// 插件拿最新一条交给核心（没有这个插件就什么都不做）。
  void sampleClipboard(fcitx::InputContext *ic);

public:
private:
  // 最近用过的 emoji 存哪儿：用户数据目录里的 emoji-recent.txt
  std::string recentEmojiPath() const;
  /// 把引擎、上文、设置同步给会话（引擎是后台载入的，设置可能刚改过）。
  void syncSession(FlypeState *state);
  /// 同步后交给会话处理一个按下的键；action 是 FLYPE_KEY_ACTION_*（fcitx5
  /// 全局的翻页、 移动候选热键）。返回会话的 frame（会话没建好时为 NULL）。
  /// 带上应用里光标前的字（prevChar）和输入框情况（密码框不自动补空格、不学习）。
  const FlypeFrame *sessionKey(fcitx::InputContext *ic, FlypeState *state,
                               const fcitx::Key &key,
                               uint32_t action = FLYPE_KEY_ACTION_NONE);
  /// 应用里光标前一个字（Unicode 码位）：光标前没有字是
  /// 0，应用不报告周围文字时没有值。
  std::optional<uint32_t> prevChar(fcitx::InputContext *ic) const;
  /// 这个键对应的 fcitx5
  /// 全局翻页、移动候选热键（方向键不算，照横排竖排的规则走）。
  uint32_t keyAction(const fcitx::Key &key) const;
  /// 会话处理了一个键以后：上屏、记
  /// emoji、照会话的候选框重画（或只挪页码和光标）。
  void applySessionFrame(fcitx::InputContext *ic, FlypeState *state,
                         const FlypeFrame *frame);
  /// 候选由会话算，外壳抄一份画候选框（fcitx5
  /// 的候选框要整个列表），连同页码、光标。
  void refreshCandidates(FlypeState *state);
  /// 会话的候选重算过才重新抄；返回 true 表示重抄了。页码、光标每次都抄。
  bool syncView(FlypeState *state);
  // 表情 / 符号面板：候选排成 6 列的格子，每行拼成一整行候选
  void fillGridPanel(fcitx::InputContext *ic, FlypeState *state);
  /// 上屏以后的联想行（会话的 FLYPE_PANEL_ASSOCIATION）。
  void fillAssociation(fcitx::InputPanel &panel, FlypeState *state);
  // 还没选词的拼音按候选分好音节（hengxiang -> heng'xiang）
  std::string segmentedRemaining(FlypeState *state);
  /// 会话上屏以后：上屏 frame->commit、清空外壳的状态；frame->learned
  /// 时攒着存用户词库。
  void finishCommit(fcitx::InputContext *ic, FlypeState *state,
                    const FlypeFrame *frame);
  /// 会话这次调用里引擎学到了选词（frame->learned）就记一次，返回记了没有。会话给的每个
  /// frame 都要过 一遍（finishCommit 里也是它）。
  bool countLearned(const FlypeFrame *frame);
  /// 上屏已选的词 +
  /// 还没选的原始输入（切输入法、打到一半切英文），原始输入也记一次选词。
  void commitInput(fcitx::InputContext *ic, FlypeState *state);
  /// 选中光标所在的候选；没有候选时上屏原始输入（交给会话按一次空格）。
  void commitCurrent(fcitx::InputContext *ic, FlypeState *state);
  /// 攒够上屏次数（force
  /// 时不管次数）就在后台存用户词库和输入习惯（按键的线程上不等磁盘）。
  /// wait：等后台写完再返回（析构、换词库前：新引擎要读这些文件）。
  void maybeSaveUserDict(bool force, bool wait = false);
  // 用户词库文件的路径：FLYPE_USER_DICT 优先，否则用户数据目录里的
  // user.tsv（顺手建目录）
  std::string userDictPath() const;

  // 语音输入：按住（或点一下）热键说话，识别结果上屏
  /// 处理了这个键（热键本身，或录音时的 Esc）就返回 true。
  bool handleVoiceKey(fcitx::InputContext *ic, FlypeState *state,
                      fcitx::KeyEvent &event);
  /// 定时调用：刷新提示、上屏结果。还要继续轮询就返回 true。
  bool pollVoice();
  // 语音提示画进候选框：上面一行状态，下面画识别到一半的字
  void showVoice(fcitx::InputContext *ic, const std::string &status,
                 const std::string &partial);
  void clearVoice();
  // 放弃这次语音：叫停录音，再把提示收掉
  void cancelVoice();
  /// 在会话总线上登记 org.flype.Input（外部命令切模式）。
  void setupControl();
  // 语音提示正显示着（录音中、识别中或出错提示）
  bool voiceActive() const { return voiceShowing_; }

  /// fcitx5 的实例，管理器都从它拿
  fcitx::Instance *instance_;
  /// 核心会话句柄，后台载入好之前是空
  FlypeEngineHandle *core_;
  /// 后台载入词库 / 用户词的 future
  std::future<FlypeLoaded> loading_;
  /// 用户词库（user.tsv）/
  /// 输入习惯（history.tsv）文件在、这次载入却没读出来：存之前先把原文件改名成
  /// <原名>.bad-<时间戳>
  /// 留着（maybeSaveUserDict），不然会被只有这次运行里学到的内容盖掉。
  bool userDictUnread_ = false;
  /// history.tsv 在、这次载入却没读出来（存之前同样先改名备份）
  bool historyUnread_ = false;
  /// 正在用的整句增强模型（FlypeLoaded::nnlmStamp）
  std::string nnlmStamp_;
  /// core_ / loading_ 用的词库文件，和配置要的词库文件（不一样就该换了）。
  std::string lexicon_;
  /// 配置现在要的词库文件（和 lexicon_ 不一样就该换）
  std::string wantedLexicon_;
  /// lexicon_ 载入时的 (大小,
  /// 修改时间)：同一个路径的文件被换掉（更新了词库包）也要重载。
  std::pair<std::uintmax_t, long long> lexiconStamp_{0, 0};
  /// 每个输入框的会话状态工厂
  fcitx::FactoryFor<FlypeState> factory_;
  /// 距离上次保存用户词库，引擎学到了几次（上屏、忘掉学错的词）。
  int unsavedCommits_ = 0;

  /// flype.conf 的原始配置（fcitx5 读设置用）
  FlypeConfig config_;
  // 生效中的设置（配置文件 + 环境变量覆盖）
  bool horizontal_ = true;
  /// 方向键用来选词还是移光标
  FlypeArrowKeys arrowKeys_ = FlypeArrowKeys::Candidates;
  /// 每页候选个数
  std::size_t pageSize_ = 9;
  /// 候选总数上限（翻页能翻到多少个）
  std::size_t maxCandidates_ = 100;
  /// 模糊音总开关
  bool fuzzy_ = false;
  /// 打开的模糊音分组（位掩码，见 flype_engine_set_fuzzy_groups）。
  unsigned fuzzyGroups_ = 0;
  /// 按错相邻键自动纠正
  bool typo_ = true;
  /// 候选末尾列出扩展 B 区以后的生僻字
  bool rareExt_ = false;
  /// 分号、引号选第 2、3 个候选的方式
  FlypeSelectKeys selectKeys_ = FlypeSelectKeys::Off;
  /// 以词定字（[ ] 上屏候选的首尾字）
  bool pickChar_ = true;
  /// 上屏后给出联想行
  bool association_ = false;
  /// 打 riqi / shijian / xingqi 给日期时间
  bool dateTime_ = true;
  /// v 开头的算式直接算结果
  bool calculator_ = true;
  /// v + 空格打开工具栏
  bool toolbar_ = true;
  /// 记录剪贴板历史（只在内存里）
  bool clipboardHistory_ = false;
  /// 自动补空格（flype_autospace 的 flags）：1 字母、2 数字，0 关。
  int autoSpaceFlags_ = 0;
  /// 单按哪个 Shift 切中英文
  FlypeShiftToggle shiftToggle_ = FlypeShiftToggle::Both;
  /// 切到 Flype 时用中文还是英文
  FlypeInitialMode initialMode_ = FlypeInitialMode::Chinese;
  /// 启动后还没切到过 Flype：第一次切过来（包括 Flype
  /// 是默认输入法时第一次获得焦点）按 InitialMode 定模式
  bool initialModeApplied_ = false;
  /// 打标点时用中文标点还是 ASCII
  FlypePunctStyle punctStyle_ = FlypePunctStyle::Fullwidth;
  /// 打开的翻页键组（Page Up/Down、方向键、fcitx 全局翻页键一直可用）
  bool pageMinus_ = true;
  /// 用 [ 和 ] 翻页
  bool pageBracket_ = false;
  /// 用 , 和 . 翻页
  bool pageComma_ = false;
  /// 用 Shift+Tab 和 Tab 翻页
  bool pageTab_ = false;
  /// 英文模式：按键原样交给应用（中英文切换全局生效，切到别的窗口也保持）。
  bool asciiMode_ = false;
  /// 打字时推荐相关 emoji
  bool emojiSuggest_ = true;
  /// 表情面板快捷键，FcitxKey_None 表示关掉。
  fcitx::Key emojiKey_;
  /// 另设的切换中 / 英文快捷键，FcitxKey_None 表示不用。
  fcitx::Key toggleKey_;

  /// 语音输入句柄；语音关掉时是空。
  FlypeVoice *voice_ = nullptr;
  /// 语音结果同时复制到剪贴板（默认关）。
  bool voiceCopy_ = false;
  /// 语音热键，默认右 Alt；FcitxKey_None 表示关掉语音。
  fcitx::Key voiceKey_;
  /// 热键按下还没松开（松开那一下也要吃掉，免得应用把单按 Alt 当成打开菜单）。
  bool voiceKeyDown_ = false;
  /// 正在显示语音提示（录音、识别中、出错提示）。
  bool voiceShowing_ = false;
  /// 出错提示显示到这个时刻（单调时钟，微秒）。
  uint64_t voiceErrorUntil_ = 0;
  /// 正在显示的出错提示（浮条要接着发）。
  std::string voiceErrorText_;
  /// 开始说话时的输入框；结果只往这里上屏。
  fcitx::TrackableObjectReference<fcitx::InputContext> voiceIc_;
  /// 语音轮询定时器（录音时定时刷新提示、上屏结果）
  std::unique_ptr<fcitx::EventSourceTime> voiceTimer_;
  /// 最近几次轮询的音量（0–100），画成滚动的波形。
  std::vector<int> voiceLevels_;
  /// 轮询次数，「识别中」的省略号动画用。
  unsigned voiceFrame_ = 0;

  // 外部命令切模式：单独一条会话总线连接。control_ 要先于总线析构，所以排在后面
  std::unique_ptr<fcitx::dbus::Bus> controlBus_;
  /// org.flype.Input 的 D-Bus 对象
  std::unique_ptr<FlypeControl> control_;

public:
  /// 现在的模式：zh / en。
  std::string modeName() const;
  /// 外部命令切模式（FlypeControl 调）；不认识的模式返回 false。
  bool setModeByName(const std::string &mode);
};

#endif /* FLYPE_FCITX5_ENGINE_H */
