#include "flype_engine.h"
#include "clipboard_copy.h"
#include "data_files.h"
#include "flype_log.h"

#include <algorithm>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <ctime>
#include <exception>
#include <filesystem>
#include <fstream>
#include <functional>
#include <memory>
#include <string>
#include <string_view>
#include <system_error>
#include <utility>

#include <fcitx-config/iniparser.h>
#include <fcitx-utils/event.h>
#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx-utils/log.h>
#include <fcitx-utils/misc.h>
#include <fcitx-utils/textformatflags.h>
#include <fcitx-utils/utf8.h>
#include <fcitx/addonfactory.h>
#include <fcitx/addonmanager.h>
#include <fcitx/candidatelist.h>
#include <fcitx/globalconfig.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputcontextmanager.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>
#include <fcitx/text.h>
#include <fcitx/userinterface.h>

// fcitx5 自带的 clipboard 插件对外的函数（和它的 clipboard_public.h
// 一样，自 5.0 起没变过）。 抄在这里免得依赖 fcitx5-modules-dev；没装这个插件时
// addon("clipboard") 是空的，不调用。
FCITX_ADDON_DECLARE_FUNCTION(Clipboard, clipboard,
                             std::string(const fcitx::InputContext *ic));

namespace {

// 系统词库目录：CMake 按安装前缀传进来（Nix 等不装在 /usr 的发行版）
#ifndef FLYPE_SYSTEM_DATA_DIR
#define FLYPE_SYSTEM_DATA_DIR "/usr/share/flype"
#endif
/// 系统词库目录，编译时按安装前缀定
constexpr const char *kSystemDataDir = FLYPE_SYSTEM_DATA_DIR;
/// 展开的候选面板：每行几个、一屏几行（照微信输入法：6 × 6）。
constexpr std::size_t kGridCols = 6;
/// 面板一屏几行
constexpr std::size_t kGridRows = 6;
/// 每上屏这么多次保存一次用户词库（另外失去焦点和退出时也保存）。
constexpr int kSaveEvery = 20;
/// 打字推荐的 emoji 插在第几个候选（从 0 数）：不挤掉前两个词。

// 读环境变量当开关：没设或为空时用 fallback，设了且不是 "0" 就算打开
bool envFlag(const char *name, bool fallback) {
  const char *value = std::getenv(name);
  if (!value || !*value) {
    return fallback;
  }
  return *value != '0';
}

// 插件自己的设置文件，相对 fcitx5 的配置目录
constexpr const char *kConfigFile = "conf/flype.conf";

// 用户数据目录：$XDG_DATA_HOME/flype，没有就用
// ~/.local/share/flype，环境变量都没给时返回空
std::string userDataDir() {
  std::string base;
  if (const char *data = std::getenv("XDG_DATA_HOME"); data && *data) {
    base = data;
  } else if (const char *home = std::getenv("HOME"); home && *home) {
    base = std::string(home) + "/.local/share";
  } else {
    return {};
  }
  return base + "/flype";
}

/// 按顺序找第一个存在的文件：环境变量 > 用户数据目录 > 系统目录。
/// 用户目录里那份是安装包自带数据的旧副本（版本不比系统目录的新，见
/// isStaleBundledCopy）时让给系统目录。
std::string findDataFile(const char *envName, const std::string &fileName) {
  if (const char *env = std::getenv(envName); env && *env) {
    return env;
  }
  std::error_code ec;
  const std::string user = userDataDir();
  const std::string system = std::string(kSystemDataDir) + "/" + fileName;
  if (!user.empty() && std::filesystem::exists(user + "/" + fileName, ec)) {
    const std::string mine = user + "/" + fileName;
    const std::string mineVersion = flype::dataFileVersion(mine, fileName);
    const std::string systemVersion = flype::dataFileVersion(system, fileName);
    const auto versionNote = [](const std::string &version) {
      return version.empty() ? std::string("没有版本标记") : "版本 " + version;
    };
    if (flype::isStaleBundledCopy(mine, system, fileName)) {
      WLOG_I() << "Flype: " << mine << "（" << versionNote(mineVersion)
               << "）是安装包自带数据的旧副本，改用 " << system << "（"
               << versionNote(systemVersion)
               << "；用户目录那份没在用，可以删掉）";
      return system;
    }
    // 用户自己放的（训练的 words.lm、自己做的词库、自己下的新数据）照旧优先
    if (flype::readDataMarker(mine)) {
      WLOG_I() << "Flype: 用 " << mine << "（" << versionNote(mineVersion)
               << "），安装包带的是 " << versionNote(systemVersion);
      return mine;
    }
    // 没有标记的：比安装包带的旧时提一句
    std::error_code userEc;
    std::error_code systemEc;
    const auto userTime = std::filesystem::last_write_time(mine, userEc);
    const auto systemTime = std::filesystem::last_write_time(system, systemEc);
    if (!userEc && !systemEc && systemTime > userTime) {
      WLOG_W() << "Flype: 用的是 " << mine << "，它比 " << system
               << " 旧；不是你自己放的话，删掉它就会用安装包带的新版";
    }
    return mine;
  }
  if (std::filesystem::exists(system, ec)) {
    return system;
  }
  return {};
}

/// 去掉末尾一个 UTF-8 字符。
void popUtf8(std::string &text) {
  while (!text.empty()) {
    const auto byte = static_cast<unsigned char>(text.back());
    text.pop_back();
    if ((byte & 0xC0) != 0x80) {
      break;
    }
  }
}

// 语音轮询间隔：录音音量、识别进度每 50ms 问一次
constexpr uint64_t kVoicePollUsec = 50 * 1000;
// 候选框里画麦克风波形用的竖条个数
constexpr std::size_t kVoiceBars = 6;

/// 音量（0–100）画成一排竖条，不足 kVoiceBars 个时左边补最低的条。
std::string voiceWave(const std::vector<int> &levels) {
  static const char *const kBlocks[] = {"▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"};
  std::string out;
  for (std::size_t i = levels.size(); i < kVoiceBars; ++i) {
    out += kBlocks[0];
  }
  for (const int level : levels) {
    out += kBlocks[std::clamp(level, 0, 100) * 7 / 100];
  }
  return out;
}
// 一般的语音出错提示留多久（1.5 秒，和浮条自己收起的时间一致）
constexpr uint64_t kVoiceErrorUsec = 1500 * 1000;
/// 「没有语音模型，去设置里下载」这句提示显示多久。
constexpr uint64_t kVoiceHintUsec = 4000 * 1000;
// 定时器精度。fcitx5 底下是 sd-event，传 0 等于默认的 250ms，50ms
// 的轮询实际一秒只有 4 次
constexpr uint64_t kTimerAccuracyUsec = 1000;

/// 把编辑区画进输入框。每一段都标上 DontCommit：失去焦点、应用重置输入框时，GTK
/// / Qt 的输入模块 （XIM 这类由 fcitx5 代劳）会把编辑区里没标 DontCommit
/// 的字直接上屏，那样文档里会留下
/// 「ni'hao」这种带分隔符的拼音、选了一半的「你好zhong'guo」。打到一半的是放弃
/// （和 IBus 的默认一样），外壳在 reset / deactivate 里清掉会话。
void setClientPreedit(fcitx::InputPanel &panel, const fcitx::Text &text) {
  fcitx::Text shown;
  for (std::size_t i = 0; i < text.size(); ++i) {
    shown.append(text.stringAt(i),
                 text.formatAt(i) | fcitx::TextFormatFlag::DontCommit);
  }
  shown.setCursor(text.cursor());
  panel.setClientPreedit(shown);
}

// 用户配置目录：$XDG_CONFIG_HOME/flype，没有就用
// ~/.config/flype，环境变量都没给时返回空
std::string userConfigDir() {
  if (const char *config = std::getenv("XDG_CONFIG_HOME"); config && *config) {
    return std::string(config) + "/flype";
  }
  if (const char *home = std::getenv("HOME"); home && *home) {
    return std::string(home) + "/.config/flype";
  }
  return {};
}

/// 用户输入习惯（词的热度、词搭配）和用户词库放在同一个目录：history.tsv。
std::string historyPathFor(const std::string &userDict) {
  if (const char *env = std::getenv("FLYPE_HISTORY"); env && *env) {
    return env;
  }
  if (userDict.empty()) {
    return {};
  }
  return (std::filesystem::path(userDict).parent_path() / "history.tsv")
      .string();
}

/// 默认词库：用户数据目录或系统目录里的 lexicon.tsv（雾凇），都没有就用 demo
/// 词典。
std::string defaultLexicon() {
  std::string lexicon = findDataFile("FLYPE_LEXICON", "lexicon.tsv");
  if (lexicon.empty()) {
    lexicon = std::string(kSystemDataDir) + "/demo_lexicon.tsv";
  }
  return lexicon;
}

/// 实际用哪个词库：环境变量 FLYPE_LEXICON > 配置里的 Lexicon（文件在才用）>
/// 默认。 Lexicon 写 wanxiang 指安装包自带的那份（写绝对路径的话升级后 store
/// 路径就变了）。
std::string resolveLexicon(const std::string &configured) {
  if (const char *env = std::getenv("FLYPE_LEXICON"); env && *env) {
    return env;
  }
  if (!configured.empty()) {
    std::string path = configured;
    if (path == "wanxiang") {
      path = std::string(kSystemDataDir) + "/dicts/wanxiang/lexicon.tsv";
    } else if (path.rfind("~/", 0) == 0) {
      if (const char *home = std::getenv("HOME"); home && *home) {
        path = std::string(home) + path.substr(1);
      }
    }
    std::error_code ec;
    if (std::filesystem::is_regular_file(path, ec)) {
      return path;
    }
    WLOG_W() << "Flype: 配置里的词库 " << path << " 不在，用默认词库";
  }
  return defaultLexicon();
}

/// 文件的 (大小, 修改时间)，拿不到是 (0, 0)。
std::pair<std::uintmax_t, long long> fileStamp(const std::string &path) {
  std::error_code ec;
  const auto size = std::filesystem::file_size(path, ec);
  if (ec) {
    return {0, 0};
  }
  const auto time = std::filesystem::last_write_time(path, ec);
  if (ec) {
    return {0, 0};
  }
  return {size, static_cast<long long>(time.time_since_epoch().count())};
}

/// 整句增强模型（字级神经语言模型）用哪一份：环境变量 FLYPE_NNLM
/// 指定了就只用它；否则先用用户导入的
/// 微信输入法模型（wetype/nlrgpt2_nnlm.bin，腾讯的，不随 Flype 分发），没有再用
/// nnlm.bin（安装包自带的 Flype
/// 自己训的，在系统数据目录；用户目录里放了一份就先用那份）。都没有返回空。
/// what 写上是哪一份，给日志用。
std::string findNnlm(std::string &what) {
  if (const char *env = std::getenv("FLYPE_NNLM"); env && *env) {
    what = "环境变量 FLYPE_NNLM 指定的模型";
    return env;
  }
  if (std::string wetype =
          findDataFile("FLYPE_NNLM", "wetype/nlrgpt2_nnlm.bin");
      !wetype.empty()) {
    what = "导入的微信输入法模型";
    return wetype;
  }
  if (std::string own = findDataFile("FLYPE_NNLM", "nnlm.bin"); !own.empty()) {
    what = own.starts_with(std::string(kSystemDataDir) + "/")
               ? "Flype 自带的整句增强模型"
               : "用户目录里的 nnlm.bin";
    return own;
  }
  what.clear();
  return {};
}

/// 认模型文件换没换：路径 + 大小 + 修改时间；没有模型是空串。
std::string nnlmStamp(const std::string &path) {
  if (path.empty()) {
    return {};
  }
  const auto [size, time] = fileStamp(path);
  return path + ":" + std::to_string(size) + ":" + std::to_string(time);
}

/// 用这个词库建核心；语言模型坏了（版本不对、文件不完整）不该让输入法整个用不了，
/// 不带它再试一次（这时 lm 清空）。
FlypeEngineHandle *openCore(const std::string &lexicon, std::string &lm) {
  FlypeEngineHandle *core =
      flype_engine_new(lexicon.c_str(), lm.empty() ? nullptr : lm.c_str());
  if (!core && !lm.empty()) {
    WLOG_W() << "Flype: 语言模型载入失败，不用它，文件=" << lm << "，错误="
             << (flype_last_error() ? flype_last_error() : "unknown");
    lm.clear();
    core = flype_engine_new(lexicon.c_str(), nullptr);
  }
  return core;
}

/// 载入失败的文件（还在的话）改名成 <原名>.bad-<时间戳>
/// 留着，免得接下来存的把它盖掉：
/// 盖上去的只有这次运行里学到的，原来攒的全没了。返回 true 表示可以往 path 存了
/// （改好名了，或者文件已经不在了）；改不了名返回 false，这次先不存。
bool setAsideUnreadFile(const std::string &path, const char *what) {
  std::error_code ec;
  if (!std::filesystem::exists(std::filesystem::symlink_status(path, ec))) {
    return true;
  }
  char stamp[32] = "0";
  const std::time_t now = std::time(nullptr);
  std::tm local{};
  if (localtime_r(&now, &local)) {
    std::strftime(stamp, sizeof(stamp), "%Y%m%d-%H%M%S", &local);
  }
  std::string backup = path + ".bad-" + stamp;
  for (int i = 2;
       std::filesystem::exists(std::filesystem::symlink_status(backup, ec));
       ++i) {
    backup = path + ".bad-" + stamp + "-" + std::to_string(i);
  }
  std::filesystem::rename(path, backup, ec);
  if (ec) {
    WLOG_W() << "Flype: " << what << " " << path
             << " 启动时没读出来，想改名留着也没改成（" << ec.message()
             << "），这次先不存，免得把它盖掉";
    return false;
  }
  WLOG_W() << "Flype: " << what << " " << path
           << " 启动时没读出来，原文件改名留在 " << backup << "，新学到的另存";
  return true;
}

/// 在后台线程里做的事：载入词库、语言模型、各种开关和用户词。
FlypeLoaded loadCore(const std::string &userDict, std::string lexicon) {
  if (lexicon.ends_with("/demo_lexicon.tsv")) {
    WLOG_W() << "Flype: 没找到 lexicon.tsv，用 demo 词典（只有几十个词）";
  }
  // 词级语言模型：flype-lmbuild 生成的 words.lm；没有就找旧的 bigram.tsv
  std::string lm = findDataFile("FLYPE_LM", "words.lm");
  if (lm.empty()) {
    lm = findDataFile("FLYPE_LM", "bigram.tsv");
  }
  const std::string lmFile = lm;
  const auto started = std::chrono::steady_clock::now();
  FlypeEngineHandle *core = openCore(lexicon, lm);
  if (!core) {
    // 选的词库（比如下载的万象）坏了：退回默认词库，别让输入法用不了
    if (const std::string fallback = defaultLexicon(); fallback != lexicon) {
      WLOG_W() << "Flype: 词库 " << lexicon << " 载入失败（"
               << (flype_last_error() ? flype_last_error() : "unknown")
               << "），改用默认词库 " << fallback;
      lexicon = fallback;
      lm = lmFile;
      core = openCore(lexicon, lm);
    }
  }
  if (!core) {
    WLOG_E() << "Flype: 初始化 Rust 核心失败，词典=" << lexicon << "，错误="
             << (flype_last_error() ? flype_last_error() : "unknown");
    return {};
  }
  FlypeLoaded result;
  result.core = core;

  const std::string english = findDataFile("FLYPE_ENGLISH", "english.txt");
  if (!english.empty()) {
    const int loaded = flype_engine_load_english(core, english.c_str());
    if (loaded > 0) {
      WLOG_I() << "Flype: 载入英文词 " << loaded << " 个";
    }
  }
  // 整句增强模型：先用导入的微信输入法模型，没有再用安装包自带的
  // nnlm.bin（findNnlm）
  std::string nnlmWhat;
  if (const std::string nnlm = findNnlm(nnlmWhat); !nnlm.empty()) {
    const int loaded = flype_engine_load_nnlm(core, nnlm.c_str());
    if (loaded > 0) {
      WLOG_I() << "Flype: 载入整句增强模型：" << nnlmWhat << " " << nnlm << "（"
               << loaded << " 字）";
      result.nnlmStamp = nnlmStamp(nnlm);
    } else {
      WLOG_W() << "Flype: 整句增强模型载入失败（" << nnlmWhat << "）" << nnlm
               << "：" << (flype_last_error() ? flype_last_error() : "unknown");
    }
  } else {
    WLOG_I() << "Flype: 没有整句增强模型（没导入微信输入法的，安装包里也没有 "
                "nnlm.bin）";
  }
  // 用户自己训的领域词级模型（可选），和 words.lm 混着用
  if (const std::string domain =
          findDataFile("FLYPE_DOMAIN_LM", "words.domain.lm");
      !domain.empty()) {
    if (flype_engine_set_domain_lm(core, domain.c_str()) == 0) {
      WLOG_I() << "Flype: 载入领域词级模型 " << domain;
    } else {
      WLOG_W() << "Flype: 领域词级模型载入失败 " << domain << "："
               << (flype_last_error() ? flype_last_error() : "unknown");
    }
  }
  // 文件在、却没读出来（读不了、不是
  // UTF-8……）：记下来，存之前先把原文件改名留着（maybeSaveUserDict），
  // 不然下次存盘会用只有这次运行里学到的内容把它盖掉。文件不在是第一次用，不算
  const auto present = [](const std::string &path) {
    std::error_code ec;
    return std::filesystem::exists(std::filesystem::symlink_status(path, ec));
  };
  if (!userDict.empty() && present(userDict)) {
    const int loaded = flype_engine_load_user_dict(core, userDict.c_str());
    if (loaded > 0) {
      WLOG_I() << "Flype: 载入用户词 " << loaded << " 条";
    } else if (loaded < 0) {
      result.userDictUnread = true;
      WLOG_W() << "Flype: 用户词库载入失败 " << userDict << "："
               << (flype_last_error() ? flype_last_error() : "unknown");
    }
  }
  if (const std::string history = historyPathFor(userDict);
      !history.empty() && present(history)) {
    const int loaded = flype_engine_load_history(core, history.c_str());
    if (loaded >= 0) {
      WLOG_I() << "Flype: 载入输入习惯 " << loaded << " 条";
    } else {
      result.historyUnread = true;
      WLOG_W() << "Flype: 输入习惯载入失败 " << history << "："
               << (flype_last_error() ? flype_last_error() : "unknown");
    }
  }
  const auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                      std::chrono::steady_clock::now() - started)
                      .count();
  WLOG_I() << "Flype: 词典 " << lexicon << " 载入完成，用时 " << ms << " ms"
           << (lm.empty() ? "（没有语言模型）" : "，语言模型 " + lm);
  return result;
}

/// 打字途中会话不管的键里，照常交给应用的那些：修饰键和锁定键本身（Shift / Ctrl
/// / Alt / Super、
/// 大写锁定、数字锁定、AltGr、切换键盘布局的键），还有认不出来的键（没有
/// keysym）。别的都吃掉。
bool passesWhileComposing(fcitx::KeySym sym) {
  const auto value = static_cast<uint32_t>(sym);
  return sym == FcitxKey_None || sym == FcitxKey_VoidSymbol ||
         (value >= FcitxKey_Shift_L && value <= FcitxKey_Hyper_R) ||
         (value >= FcitxKey_ISO_Lock && value <= FcitxKey_ISO_Level5_Lock) ||
         sym == FcitxKey_Mode_switch || sym == FcitxKey_Num_Lock ||
         sym == FcitxKey_Scroll_Lock;
}

/// 候选窗里显示的样子：自定义词里的换行、制表符画成 ⏎、⇥，不把候选窗撑乱。
std::string shownText(const std::string &text) {
  if (text.find_first_of("\n\t") == std::string::npos) {
    return text;
  }
  std::string out;
  for (const char c : text) {
    if (c == '\n') {
      out += "⏎";
    } else if (c == '\t') {
      out += "⇥";
    } else {
      out.push_back(c);
    }
  }
  return out;
}

// 普通候选框里的一个候选：点它就是让会话选这个序号的词
class FlypeCandidateWord final : public fcitx::CandidateWord {
public:
  /// shown 是 shownCandidate 画好的样子。
  FlypeCandidateWord(FlypeEngine *engine, FlypeState *state, std::size_t index,
                     fcitx::Text shown)
      : engine_(engine), state_(state), index_(index) {
    setText(std::move(shown));
  }

  // fcitx5 里用户点了这个候选：上屏、学习都由引擎调会话做
  void select(fcitx::InputContext *inputContext) const override {
    engine_->commitCandidate(inputContext, state_, index_);
  }

private:
  /// 点了这个候选时回调的引擎
  FlypeEngine *engine_;
  /// 这个候选所属输入框的会话状态
  FlypeState *state_;
  /// 候选在会话里的全局下标
  std::size_t index_;
};

/// 展开面板的一行（一行里有好几个词）。
class FlypeGridRow final : public fcitx::CandidateWord {
public:
  // text 是 fillGridPanel 拼好的一整行（全角字符排出来的格子）
  FlypeGridRow(FlypeEngine *engine, FlypeState *state, std::size_t row,
               fcitx::Text text)
      : engine_(engine), state_(state), row_(row) {
    setText(std::move(text));
  }

  // 点了一行：光标就在这一行时选光标处的词，否则选这一行的第一个
  void select(fcitx::InputContext *inputContext) const override {
    engine_->selectGridRow(inputContext, state_, row_);
  }

private:
  /// 点了这一行时回调的引擎
  FlypeEngine *engine_;
  /// 这一行所属输入框的会话状态
  FlypeState *state_;
  /// 这一行在面板里的行号
  std::size_t row_;
};

/// 工具栏列表里的一项（菜单、剪贴板历史、常用语）。
class FlypeToolWord final : public fcitx::CandidateWord {
public:
  // text 是这一项显示的字（菜单项、剪贴板内容、常用语）
  FlypeToolWord(FlypeEngine *engine, FlypeState *state, std::size_t index,
                const std::string &text)
      : engine_(engine), state_(state), index_(index) {
    setText(fcitx::Text(text));
  }

  // 点了这一项：菜单项进下一层，其余的上屏（都是会话做）
  void select(fcitx::InputContext *inputContext) const override {
    engine_->choosePanelItem(inputContext, state_, index_);
  }

private:
  /// 点了这一项时回调的引擎
  FlypeEngine *engine_;
  /// 这一项所属输入框的会话状态
  FlypeState *state_;
  /// 这一项在面板里的下标
  std::size_t index_;
};

/// 显示宽度（半角为 1、其他为 2），用来把展开面板的各列对齐。
/// emoji 序列（变体选择符、肤色、零宽连接、旗帜）按一个宽字符算。
std::size_t displayWidth(const std::string &text) {
  std::size_t width = 0;
  bool joinNext = false;
  bool pendingFlag = false;
  for (std::size_t i = 0; i < text.size();) {
    const unsigned char c = static_cast<unsigned char>(text[i]);
    const std::size_t len = c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
    char32_t cp = len == 1 ? c : c & (0xFF >> (len + 1));
    for (std::size_t k = 1; k < len && i + k < text.size(); ++k) {
      cp = (cp << 6) | (static_cast<unsigned char>(text[i + k]) & 0x3F);
    }
    i += len;
    if (cp == 0x200D) {
      joinNext = true;
      continue;
    }
    const bool zeroWidth = cp == 0xFE0F || cp == 0xFE0E || cp == 0x20E3 ||
                           (cp >= 0x1F3FB && cp <= 0x1F3FF) ||
                           (cp >= 0xE0020 && cp <= 0xE007F);
    if (zeroWidth || joinNext) {
      joinNext = false;
      continue;
    }
    if (cp >= 0x1F1E6 && cp <= 0x1F1FF) { // 两个地区指示符拼成一面旗
      if (pendingFlag) {
        pendingFlag = false;
        continue;
      }
      pendingFlag = true;
      width += 2;
      continue;
    }
    pendingFlag = false;
    width += cp < 0x80 ? 1 : 2;
  }
  return width;
}

/// 用全角空格（宽 2）和半角空格（宽 1）补到 width。
void padTo(std::string &out, std::size_t have, std::size_t width) {
  while (have + 2 <= width) {
    out += "\u3000";
    have += 2;
  }
  if (have < width) {
    out += "\u2002";
  }
}

} // namespace

// 外壳抄下来的一份候选：显示用；上屏时按序号让会话自己去选
struct FlypeCandidate {
  std::string text;
  std::string pinyin;
  /// 用掉了剩余输入的多少字节；小于剩余长度说明是句首词候选。
  std::size_t consumed = 0;
  /// emoji 候选：选中时上屏 commit，记进最近用过的 emoji，不学习选词。
  bool emoji = false;
  std::string commit;
  /// emoji 本身（打字推荐的候选显示的是「开心😊」，这里是「😊」）。
  std::string emojiText;
  /// 日期时间、计算结果这类：选中时原样上屏，不学习选词。
  bool plain = false;
  /// 画在字后面的补充说明（比如候选的编码提示），不上屏。
  std::string comment;
};

namespace {

/// 候选框里一个候选的样子：字（换行画成
/// ⏎），后面空一格是斜体的编码提示，和字分得开。 fcitx5 5.1.9
/// 以前候选没有注释位，经典界面也只认粗体、斜体、下划线、删除线，画不了浅色，
/// 斜体是各个版本都一样的那个。
fcitx::Text shownCandidate(const FlypeCandidate &candidate) {
  fcitx::Text out(shownText(candidate.text));
  if (!candidate.comment.empty()) {
    out.append(" " + candidate.comment, fcitx::TextFormatFlag::Italic);
  }
  return out;
}

} // namespace

/// 每个输入上下文独立保存的缓冲区和候选。
class FlypeState final : public fcitx::InputContextProperty {
public:
  // 每个输入框一份，建一个自己的会话（引擎到第一次按键时才挂上）
  FlypeState() : session_(flype_session_new(nullptr, nullptr)) {}
  // 输入框销毁时连会话一起放掉
  ~FlypeState() override { flype_session_free(session_); }
  // 会话指针不能两个状态共用，拷贝不了
  FlypeState(const FlypeState &) = delete;
  FlypeState &operator=(const FlypeState &) = delete;

  /// 核心的输入会话：编辑区的原始输入和已选词栈在这里（见 docs/PORTING.md）。
  /// 表情面板开着时，原始输入就是搜索词。
  FlypeSession *session_;
  /// 会话候选的本地拷贝（fcitx5 的候选框要整个列表，见 refreshCandidates）
  std::vector<FlypeCandidate> candidates_;
  /// 展开的面板（普通候选下键展开，或表情面板）；gridCursor_
  /// 是光标所在候选的全局下标。
  /// 普通候选的这些（连同下面的页码、光标）是会话的候选框状态的拷贝，见
  /// syncView。
  bool expanded_ = false;
  /// 展开面板里光标所在候选的全局下标
  std::size_t gridCursor_ = 0;
  /// 普通候选列表的页码和光标（全局下标，可以不在这一页上）；工具栏也用
  /// panelCursor_。
  std::size_t panelPage_ = 0;
  /// 普通候选列表的光标（全局下标，可以不在这一页上）
  std::size_t panelCursor_ = 0;
  /// candidates_ 抄的是会话第几次重算的候选（FlypeSessionView.generation）。
  uint64_t candGeneration_ = UINT64_MAX;
  /// 开着的工具栏 / 表情面板（FLYPE_TOOL_*），会话管。
  int panel() const { return flype_session_open_panel(session_); }
  /// 会话开着上屏以后的联想（没在打字）。
  bool associating() const { return flype_session_association(session_) != 0; }
  /// 候选框上画着联想行：会话关掉了联想、键又交给了应用时要收起来。
  bool associationShown_ = false;

  /// 全部原始输入（包括已经选过词的部分）。
  std::string input() const { return flype_session_input(session_); }
  /// 还没选词的那部分输入。
  std::string remaining() const { return flype_session_remaining(session_); }
  // 已经选进编辑区的那部分词（还没上屏）
  std::string selectedText() const {
    return flype_session_selected_text(session_);
  }
  // 编辑区里有没有已经选过的词（有就说明在逐词打）
  bool hasSelections() const { return !selectedText().empty(); }
  // 编辑区里有没有字：选过的词或还没选的拼音
  bool composing() const { return *flype_session_input(session_) != '\0'; }
  /// 交给会话处理一个按下的键（keysym 为 0 时只有动作）。先用
  /// FlypeEngine::sessionKey 同步引擎和设置。返回的 frame
  /// 归会话，下一次调会话前有效。 prev_char：应用里光标前一个字（读不到给
  /// 0）；hints：FLYPE_KEY_HINT_*。
  const FlypeFrame *sessionKey(uint32_t keysym, uint32_t modifiers,
                               uint32_t action, uint32_t prevChar = 0,
                               uint32_t hints = 0, uint64_t timeMs = 0) {
    FlypeKey raw{};
    raw.time_ms = timeMs;
    raw.size = sizeof(raw);
    raw.keysym = keysym;
    raw.modifiers = modifiers;
    raw.action = action;
    raw.prev_char = prevChar;
    raw.hints = hints;
    return flype_session_key(session_, &raw);
  }
  // 清空这个输入框的编辑区和外壳抄的候选，顺便告诉会话 Shift 已经撒手
  void clear() {
    flype_session_reset(session_, 0);
    // 按下了 Shift 还没松开，输入框就被重置、失去焦点（Shift+点鼠标选字、按着
    // Shift 换窗口）：和按住 Shift 时按了别的键一样报给会话，再松开就不算单按
    // Shift、不切中英文
    sessionKey(FcitxKey_VoidSymbol, 0, FLYPE_KEY_ACTION_OBSERVE);
    forgetCandidates();
  }

  /// 只丢掉外壳抄的候选和候选框状态，会话的编辑区不动。
  void forgetCandidates() {
    candidates_.clear();
    candGeneration_ = UINT64_MAX;
    expanded_ = false;
    gridCursor_ = 0;
    panelPage_ = 0;
    panelCursor_ = 0;
  }
};

namespace {

/// 普通候选的列表：翻页、移动光标交给会话，再照会话的页码和光标摆好自己。
/// 键盘上的翻页键由会话直接处理；这里接的是鼠标点翻页按钮、滚轮，和直接调候选列表的。
class FlypeSessionList final : public fcitx::CommonCandidateList {
public:
  // state 是这个列表所属的输入框状态，翻页、移光标都交给它的会话
  explicit FlypeSessionList(FlypeState *state) : state_(state) {}

  // 上一页：交给会话翻（鼠标点翻页按钮、滚轮走到这里）
  void prev() override { act(FLYPE_KEY_ACTION_PREV_PAGE); }
  // 下一页：同上，也是会话说了算
  void next() override { act(FLYPE_KEY_ACTION_NEXT_PAGE); }
  // 光标移到上一个候选：交给会话，它会连带翻页
  void prevCandidate() override { act(FLYPE_KEY_ACTION_PREV_CANDIDATE); }
  // 光标移到下一个候选：同上
  void nextCandidate() override { act(FLYPE_KEY_ACTION_NEXT_CANDIDATE); }
  // 直接跳到第 page 页：一页一页让会话翻过去，翻不动就停
  void setPage(int page) override {
    if (mirroring_) {
      // mirror 里调的 CommonCandidateList::next()
      // 会走到这里：只摆自己，不再翻会话
      CommonCandidateList::setPage(page);
      return;
    }
    for (int guard = 0; guard < 1000 && page != currentPage(); ++guard) {
      const int before = currentPage();
      act(page < before ? FLYPE_KEY_ACTION_PREV_PAGE
                        : FLYPE_KEY_ACTION_NEXT_PAGE);
      if (currentPage() == before) {
        break;
      }
    }
  }

  /// 照会话的页码和光标摆好（页码往后翻一页时用 next()，fcitx5
  /// 记得「用过下一页」）。
  void mirror(std::size_t page, std::size_t cursor) {
    mirroring_ = true;
    const int target = static_cast<int>(page);
    if (target != currentPage() && target >= 0 && target < totalPages()) {
      if (target == currentPage() + 1) {
        CommonCandidateList::next();
      } else {
        CommonCandidateList::setPage(target);
      }
    }
    if (static_cast<int>(cursor) < totalSize()) {
      setGlobalCursorIndex(static_cast<int>(cursor));
    }
    mirroring_ = false;
  }

private:
  // 把动作交给会话，再把会话的页码、光标抄回来摆好自己
  void act(uint32_t action) {
    state_->sessionKey(0, 0, action);
    FlypeSessionView view{};
    view.size = sizeof(view);
    if (flype_session_view(state_->session_, &view) != 0) {
      return;
    }
    state_->panelPage_ = view.page;
    state_->panelCursor_ = view.cursor;
    mirror(view.page, view.cursor);
  }

  /// 对应的会话状态
  FlypeState *state_;
  /// 正在把会话的页码 / 光标抄回去，避免递归刷新
  bool mirroring_ = false;
};

/// 工具栏、表情面板、展开面板的列表：候选框上的翻页按钮、滚轮和直接调列表的，都和键盘上按
/// Page Up / Page
/// Down、上下键一样交给会话，再照会话的光标整个重画（FlypeEngine::panelListKey）。
/// 让 CommonCandidateList
/// 自己翻的话只是换了显示的那一页，会话的光标还在原来那一页，接着按数字、
/// 空格选的是原来那页的。
class FlypePanelList final : public fcitx::CommonCandidateList {
public:
  // 记下所属的引擎和输入框（弱引用），翻页动作都转给引擎去调会话
  FlypePanelList(FlypeEngine *engine, fcitx::InputContext *ic,
                 FlypeState *state)
      : engine_(engine), ic_(ic->watch()), state_(state) {}

  // 上一页：和按 Page Up 一样交给会话
  void prev() override { act(FcitxKey_Page_Up); }
  // 下一页：和按 Page Down 一样交给会话
  void next() override { act(FcitxKey_Page_Down); }
  // 上移光标；第一个（展开面板是第一行）上不再往上
  void prevCandidate() override {
    // 第一个（展开面板是第一行）上不再往上：展开面板里那是收起
    if (globalCursorIndex() > 0) {
      act(FcitxKey_Up);
    }
  }
  // 下移光标：和按上下键一样交给会话
  void nextCandidate() override { act(FcitxKey_Down); }
  // 滚轮、翻页按钮走这里：按屏数折算成 Page Up / Page Down 交给会话
  void setPage(int page) override {
    // 一屏一屏翻会话。重画会换掉输入框的列表，翻完之前自己先留着
    const auto keep = shown();
    if (!keep) {
      return;
    }
    const int steps = page - currentPage();
    for (int i = 0; i < std::min(std::abs(steps), 1000) && ic_.get(); ++i) {
      engine_->panelListKey(ic_.get(), state_,
                            steps < 0 ? FcitxKey_Page_Up : FcitxKey_Page_Down);
    }
  }

  /// 建列表时照会话的光标摆好（只摆自己）：setGlobalCursorIndex
  /// 只设光标、不翻页，先翻到光标那一页， 不然光标走到后面几页时看不到。
  void place(std::size_t page, std::size_t cursor) {
    CommonCandidateList::setPage(static_cast<int>(page));
    setGlobalCursorIndex(static_cast<int>(cursor));
  }

private:
  /// 输入框现在显示的就是这个列表才返回它（不是的话是一个过时的列表，不能拿它去翻会话）。
  std::shared_ptr<fcitx::CandidateList> shown() const {
    auto *ic = ic_.get();
    auto list = ic ? ic->inputPanel().candidateList() : nullptr;
    return list.get() == this ? list : nullptr;
  }

  // 输入框还显示着这个列表才往下走，过时的列表不能拿去翻会话
  void act(fcitx::KeySym sym) {
    if (const auto keep = shown()) {
      engine_->panelListKey(ic_.get(), state_, sym);
    }
  }

  /// 回调的引擎
  FlypeEngine *engine_;
  /// 这个列表所在的输入框
  fcitx::TrackableObjectReference<fcitx::InputContext> ic_;
  /// 对应的会话状态
  FlypeState *state_;
};

} // namespace

// 插件初始化：登记输入框属性、读设置，然后把词库丢后台载入
FlypeEngine::FlypeEngine(fcitx::Instance *instance)
    : instance_(instance), core_(nullptr),
      factory_([](fcitx::InputContext &) { return new FlypeState(); }) {
  instance_->inputContextManager().registerProperty("flype", &factory_);
  WLOG_I() << "Flype " << FLYPE_ADDON_VERSION << " 启动（fcitx5 "
           << fcitx::Instance::version() << "）";
  fcitx::RawConfig raw;
  fcitx::readAsIni(raw, kConfigFile);
  loadFlypeConfig(config_, raw);
  // 全量词库载入要几百毫秒，放后台，别卡住 fcitx 启动
  lexicon_ = wantedLexicon_ = resolveLexicon(*config_.lexicon);
  lexiconStamp_ = fileStamp(lexicon_);
  loading_ = std::async(std::launch::async, loadCore, userDictPath(), lexicon_);
  applyConfig();
  setupControl();
}

namespace {
// 外部命令切模式用的会话总线名字（org.flype.Input）、对象路径和接口
constexpr char kControlService[] = "org.flype.Input";
/// 控制器对象路径
constexpr char kControlPath[] = "/org/flype/Input";
/// 控制器接口名
constexpr char kControlInterface[] = "org.flype.Input1";
} // namespace

// 当前模式：外部命令（比如脚本、状态栏）读这个看中文还是英文
std::string FlypeControl::mode() { return engine_->modeName(); }

// 插件版本号（打包名里带着的那个），给外部查询用
std::string FlypeControl::version() { return FLYPE_ADDON_VERSION; }

// 按名字切模式（zh / en / toggle），不认识的名字回一个 D-Bus 参数错误
std::string FlypeControl::setMode(const std::string &mode) {
  if (!engine_->setModeByName(mode)) {
    throw fcitx::dbus::MethodCallError(
        "org.freedesktop.DBus.Error.InvalidArgs",
        ("不认识的模式：" + mode + "（可以是 zh、en、toggle）").c_str());
  }
  return engine_->modeName();
}

// 在会话总线上登记 org.flype.Input，让 flype-control
// 这类外部命令能切模式；没有总线就算了
void FlypeEngine::setupControl() {
  try {
    auto bus =
        std::make_unique<fcitx::dbus::Bus>(fcitx::dbus::BusType::Session);
    if (!bus->isOpen()) {
      return;
    }
    bus->attachEventLoop(&instance_->eventLoop());
    auto control = std::make_unique<FlypeControl>(this);
    if (!bus->addObjectVTable(kControlPath, kControlInterface, *control) ||
        !bus->requestName(
            kControlService,
            fcitx::Flags<fcitx::dbus::RequestNameFlag>{
                fcitx::dbus::RequestNameFlag::ReplaceExisting,
                fcitx::dbus::RequestNameFlag::AllowReplacement})) {
      WLOG_W() << "Flype: 登记 " << kControlService
               << " 失败，外部命令切不了模式";
      return;
    }
    controlBus_ = std::move(bus);
    control_ = std::move(control);
  } catch (const std::exception &e) {
    WLOG_I() << "Flype: 连不上会话总线，外部命令切不了模式：" << e.what();
  }
}

// 当前模式的名字，给 D-Bus 和上面那套命令用
std::string FlypeEngine::modeName() const { return asciiMode_ ? "en" : "zh"; }

// 按名字（zh / en / toggle）切到中或英，名字不对返回 false
bool FlypeEngine::setModeByName(const std::string &mode) {
  bool ascii = false;
  if (mode == "en") {
    ascii = true;
  } else if (mode == "toggle") {
    ascii = !asciiMode_;
  } else if (mode != "zh") {
    return false;
  }
  if (ascii == asciiMode_) {
    return true;
  }
  // 切的是最近用过的输入框（vim 所在的终端）；它没在用 Flype 时只改模式
  fcitx::InputContext *ic = instance_->mostRecentInputContext();
  FlypeState *state = ic ? ic->propertyFor(&factory_) : nullptr;
  switchMode(ic, state, ascii, false);
  return true;
}

// 设置界面保存时调这里：存盘并马上生效
void FlypeEngine::setConfig(const fcitx::RawConfig &config) {
  config_.load(config, true);
  fcitx::safeSaveAsIni(config_, kConfigFile);
  applyConfig();
}

// 重新读设置文件（设置界面改完让 fcitx5 重载），必要时跟着换词库、换整句模型
void FlypeEngine::reloadConfig() {
  // 文件里没写的项回到默认值（设置界面删掉一项就是想恢复默认）
  config_ = FlypeConfig();
  fcitx::RawConfig raw;
  fcitx::readAsIni(raw, kConfigFile);
  loadFlypeConfig(config_, raw);
  applyConfig();
  wantedLexicon_ = resolveLexicon(*config_.lexicon);
  maybeSwitchLexicon();
  // 设置界面刚导入（或删掉）了微信输入法的模型（导入、删除完会让插件重新读设置）：要用的那份变了就载入，
  // 不用重启 fcitx5。删掉导入的以后换回安装包自带的
  // nnlm.bin；两份都没有时已经载入的留到下次重启。
  // 词库还在后台载入时不用管，载入那边会读到新文件
  if (core_) {
    std::string what;
    const std::string nnlm = findNnlm(what);
    const std::string stamp = nnlmStamp(nnlm);
    if (!stamp.empty() && stamp != nnlmStamp_) {
      if (flype_engine_load_nnlm(core_, nnlm.c_str()) > 0) {
        WLOG_I() << "Flype: 换用整句增强模型：" << what << " " << nnlm;
        nnlmStamp_ = stamp;
      } else {
        WLOG_W() << "Flype: 整句增强模型载入失败（" << what << "）" << nnlm
                 << "："
                 << (flype_last_error() ? flype_last_error() : "unknown");
      }
    }
  }
}

// 要用的词库变了（改了设置或文件本身换）就停掉旧的，后台重新载入一份
void FlypeEngine::maybeSwitchLexicon() {
  if (wantedLexicon_ == lexicon_ && fileStamp(lexicon_) == lexiconStamp_) {
    return;
  }
  if (loading_.valid()) {
    if (loading_.wait_for(std::chrono::seconds(0)) !=
        std::future_status::ready) {
      return; // 还在载入上一个：coreReady() 载完时再来
    }
    takeLoaded();
  }
  if (core_) {
    // 新引擎载入时要读这两个文件：等写完
    maybeSaveUserDict(true, true);
  }
  // 候选、编辑区都是旧词库算出来的，先清掉
  instance_->inputContextManager().foreach ([this](fcitx::InputContext *ic) {
    auto *state = ic->propertyFor(&factory_);
    state->clear();
    // 会话借用引擎：先让它放开，再释放（ABI.md：引擎要比会话活得久）
    flype_session_set_engine(state->session_, nullptr);
    ic->inputPanel().reset();
    ic->updatePreedit();
    ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
    return true;
  });
  if (core_) {
    flype_engine_free(core_);
    core_ = nullptr;
  }
  if (lexicon_ == wantedLexicon_) {
    WLOG_I() << "Flype: 词库文件换了，重新载入 " << lexicon_;
  } else {
    WLOG_I() << "Flype: 换词库 " << lexicon_ << " -> " << wantedLexicon_;
  }
  lexicon_ = wantedLexicon_;
  lexiconStamp_ = fileStamp(lexicon_);
  loading_ = std::async(std::launch::async, loadCore, userDictPath(), lexicon_);
}

// 把设置和环境变量算成引擎的成员（环境变量优先，方便不重装 fcitx5 就能试）
void FlypeEngine::applyConfig() {
  const auto envOr = [](const char *name, bool configured) {
    return envFlag(name, configured);
  };
  horizontal_ =
      !envOr("FLYPE_VERTICAL", *config_.layout == FlypeLayout::Vertical);
  arrowKeys_ = *config_.arrowKeys;
  pageSize_ = static_cast<std::size_t>(std::clamp(*config_.pageSize, 3, 9));
  maxCandidates_ =
      static_cast<std::size_t>(std::clamp(*config_.maxCandidates, 10, 1000));
  if (const char *env = std::getenv("FLYPE_MAX_CANDIDATES"); env && *env) {
    maxCandidates_ = static_cast<std::size_t>(
        std::clamp(std::strtol(env, nullptr, 10), 10L, 1000L));
  }
  pageMinus_ = *config_.pageKeysMinus;
  pageBracket_ = *config_.pageKeysBracket;
  pageComma_ = *config_.pageKeysComma;
  pageTab_ = *config_.pageKeysTab;
  if (const char *env = std::getenv("FLYPE_PAGE_KEYS"); env && *env) {
    // 逗号分隔：minus,bracket,comma,tab
    const std::string keys = std::string(",") + env + ",";
    pageMinus_ = keys.find(",minus,") != std::string::npos;
    pageBracket_ = keys.find(",bracket,") != std::string::npos;
    pageComma_ = keys.find(",comma,") != std::string::npos;
    pageTab_ = keys.find(",tab,") != std::string::npos;
  }
  fuzzy_ = envOr("FLYPE_FUZZY", *config_.fuzzy);
  // 位掩码，和核心的 syllable::fuzzy 一致
  fuzzyGroups_ =
      (*config_.fuzzyZh ? 1u : 0u) | (*config_.fuzzyCh ? 2u : 0u) |
      (*config_.fuzzySh ? 4u : 0u) | (*config_.fuzzyNL ? 8u : 0u) |
      (*config_.fuzzyFH ? 16u : 0u) | (*config_.fuzzyRL ? 32u : 0u) |
      (*config_.fuzzyAn ? 64u : 0u) | (*config_.fuzzyEn ? 128u : 0u) |
      (*config_.fuzzyIn ? 256u : 0u) | (*config_.fuzzyIan ? 512u : 0u) |
      (*config_.fuzzyUan ? 1024u : 0u) | (*config_.fuzzyUn ? 2048u : 0u) |
      (*config_.fuzzyAnAi ? 4096u : 0u) |
      (*config_.fuzzyHuangWang ? 8192u : 0u) |
      (*config_.fuzzyUnIong ? 16384u : 0u) |
      (*config_.fuzzyEngOng ? 32768u : 0u);
  if (const char *env = std::getenv("FLYPE_FUZZY_GROUPS"); env && *env) {
    fuzzyGroups_ = static_cast<unsigned>(std::strtoul(env, nullptr, 0));
  }
  selectKeys_ = *config_.selectKeys;
  if (const char *env = std::getenv("FLYPE_SELECT_KEYS"); env && *env) {
    const std::string name = env;
    selectKeys_ = name == "semicolon" ? FlypeSelectKeys::Semicolon
                  : name == "quote"   ? FlypeSelectKeys::SemicolonQuote
                                      : FlypeSelectKeys::Off;
  }
  pickChar_ = envOr("FLYPE_PICK_CHAR", *config_.pickChar);
  association_ = envOr("FLYPE_ASSOCIATION", *config_.association);
  dateTime_ = envOr("FLYPE_DATETIME", *config_.dateTime);
  calculator_ = envOr("FLYPE_CALC", *config_.calculator);
  toolbar_ = envOr("FLYPE_TOOLBAR", *config_.toolbar);
  clipboardHistory_ =
      envOr("FLYPE_CLIPBOARD_HISTORY", *config_.clipboardHistory);
  if (!clipboardHistory_) {
    flype_clipboard_clear();
  }
  autoSpaceFlags_ = *config_.autoSpace ? (*config_.autoSpaceLetters ? 1 : 0) |
                                             (*config_.autoSpaceDigits ? 2 : 0)
                                       : 0;
  if (const char *env = std::getenv("FLYPE_AUTOSPACE"); env && *env) {
    autoSpaceFlags_ = static_cast<int>(std::strtol(env, nullptr, 0)) & 3;
  }
  typo_ = envOr("FLYPE_TYPO", *config_.typo);
  rareExt_ = envOr("FLYPE_RARE_EXT", *config_.rareExt);
  shiftToggle_ = *config_.shiftToggle;
  if (const char *env = std::getenv("FLYPE_SHIFT_TOGGLE"); env && *env) {
    const std::string name = env;
    shiftToggle_ = name == "left"                 ? FlypeShiftToggle::Left
                   : name == "right"              ? FlypeShiftToggle::Right
                   : name == "off" || name == "0" ? FlypeShiftToggle::Off
                                                  : FlypeShiftToggle::Both;
  }
  toggleKey_ = *config_.toggleKey;
  if (!canToggleMode()) { // 切不回中文了，回到中文
    asciiMode_ = false;
  }
  initialMode_ = *config_.initialMode;
  punctStyle_ = *config_.punctStyle;
  if (const char *env = std::getenv("FLYPE_PUNCT_STYLE"); env && *env) {
    const std::string name = env;
    punctStyle_ = name == "half" || name == "halfwidth"
                      ? FlypePunctStyle::Halfwidth
                      : FlypePunctStyle::Fullwidth;
  }
  emojiSuggest_ = envOr("FLYPE_EMOJI", *config_.emojiSuggest);
  emojiKey_ = *config_.emojiKey;

  // 语音热键：FLYPE_VOICE_KEY（fcitx 键名，如 Alt_R、Control_R、F8）优先，none
  // 关掉
  fcitx::Key key = *config_.voice ? *config_.voiceKey : fcitx::Key();
  if (const char *env = std::getenv("FLYPE_VOICE_KEY"); env && *env) {
    const std::string name = env;
    key = name == "none" || name == "0" ? fcitx::Key() : fcitx::Key(name);
    if (key.sym() == FcitxKey_None && name != "none" && name != "0") {
      WLOG_W() << "Flype: 不认识语音热键 " << name << "，改用 Alt_R";
      key = fcitx::Key(FcitxKey_Alt_R);
    }
  }
  if (key.sym() != voiceKey_.sym() || key.states() != voiceKey_.states()) {
    if (voiceActive()) {
      cancelVoice();
    }
    voiceKeyDown_ = false;
  }
  voiceKey_ = key;
  voiceCopy_ = envOr("FLYPE_VOICE_COPY", *config_.voiceCopy);
  if (voiceKey_.sym() != FcitxKey_None && !voice_) {
    // 只建句柄：不开麦克风、不读模型，第一次按热键时才读。模型从来不在这里下载，
    // 没下载时按热键只提示去「Flype 设置 → 语音」里下载
    const char *dir = std::getenv("FLYPE_VOICE_MODEL_DIR");
    voice_ = flype_voice_new(dir);
  }
  WLOG_I() << "Flype: 设置 " << (horizontal_ ? "横排" : "竖排")
           << (arrowKeys_ == FlypeArrowKeys::Caret ? " 左右键移光标" : "")
           << " 每页" << pageSize_ << " 候选上限" << maxCandidates_
           << " 模糊音=" << fuzzy_ << "(" << fuzzyGroups_ << ")"
           << " 纠错=" << typo_ << " 扩展区生僻字=" << rareExt_
           << " Shift切换=" << static_cast<int>(shiftToggle_)
           << " 语音键=" << voiceKey_.toString()
           << " emoji推荐=" << emojiSuggest_
           << " 表情键=" << emojiKey_.toString()
           << " 中英键=" << toggleKey_.toString()
           << " 选词键=" << static_cast<int>(selectKeys_)
           << " 日期时间=" << dateTime_ << " 计算器=" << calculator_
           << " 工具栏=" << toolbar_ << " 剪贴板历史=" << clipboardHistory_;
  if (core_) {
    applyCoreSettings();
  }
}

// 把当前设置灌给刚载入的核心：候选上限、模糊音、自定义词
void FlypeEngine::applyCoreSettings() {
  // 前两页是排好序的整句、词和句首词，后面把能对上的同音词、句首词、单字全补上，
  // 翻页能一直翻到生僻字（每次按键多花不到 1ms）
  flype_engine_set_max_candidates(core_, static_cast<unsigned>(maxCandidates_));
  flype_engine_set_fuzzy(core_, fuzzy_ ? 1 : 0);
  flype_engine_set_fuzzy_groups(core_, fuzzyGroups_);
  flype_engine_set_typo_correction(core_, typo_ ? 1 : 0);
  flype_engine_set_rare_ext_chars(core_, rareExt_ ? 1 : 0);
  if (const std::string custom = customWordsPath(); !custom.empty()) {
    const int loaded = flype_engine_load_custom_words(core_, custom.c_str());
    if (loaded < 0) {
      WLOG_W() << "Flype: 自定义词载入失败 " << custom << "："
               << (flype_last_error() ? flype_last_error() : "unknown");
    } else if (loaded > 0) {
      WLOG_I() << "Flype: 载入自定义词 " << loaded << " 条";
    }
  }
}

// 自定义词文件：环境变量 FLYPE_CUSTOM 优先，否则用用户数据目录里的 custom.tsv
std::string FlypeEngine::customWordsPath() const {
  if (const char *env = std::getenv("FLYPE_CUSTOM"); env && *env) {
    return env;
  }
  const std::string dir = userDataDir();
  return dir.empty() ? std::string() : dir + "/custom.tsv";
}

// 这个键算不算语音热键（单个修饰键不看状态位，松开时状态位里带着它自己）
bool FlypeEngine::isVoiceKey(const fcitx::Key &key) const {
  if (voiceKey_.sym() == FcitxKey_None || key.sym() != voiceKey_.sym()) {
    return false;
  }
  // 单按修饰键（右 Alt 等）不管状态位；松开时状态位里会带着它自己
  if (voiceKey_.isModifier() || voiceKeyDown_) {
    return true;
  }
  const fcitx::KeyStates mask{fcitx::KeyState::SimpleMask};
  return (key.states() & mask) == (voiceKey_.states() & mask);
}

// 这个键是不是表情面板热键
bool FlypeEngine::isEmojiKey(const fcitx::Key &key) const {
  return emojiKey_.sym() != FcitxKey_None &&
         key.checkKeyList(fcitx::KeyList{emojiKey_});
}

// 这个键是不是另设的中英切换键，是就给会话一个切模式的动作
uint32_t FlypeEngine::modeKeyAction(const fcitx::Key &key) const {
  const bool is = toggleKey_.sym() != FcitxKey_None &&
                  key.checkKeyList(fcitx::KeyList{toggleKey_});
  return is ? FLYPE_KEY_ACTION_TOGGLE_MODE : FLYPE_KEY_ACTION_NONE;
}

// 还能不能切回中文：Shift 切换没关掉，或者另外设了切换键
bool FlypeEngine::canToggleMode() const {
  return shiftToggle_ != FlypeShiftToggle::Off ||
         toggleKey_.sym() != FcitxKey_None;
}

// 卸载插件：先让各个会话放开引擎（引擎要比会话活得久），再存盘、释放核心和语音
FlypeEngine::~FlypeEngine() {
  if (loading_.valid()) {
    takeLoaded();
  }
  // 会话借用引擎：先让它放开，再释放（ABI.md：引擎要比会话活得久）
  instance_->inputContextManager().foreach ([this](fcitx::InputContext *ic) {
    auto *state = ic->propertyFor(&factory_);
    flype_session_set_engine(state->session_, nullptr);
    return true;
  });
  if (core_) {
    // 退出前等后台存盘写完（free 也会等，这里能把失败记进日志）
    maybeSaveUserDict(true, true);
    flype_engine_free(core_);
    core_ = nullptr;
  }
  voiceTimer_.reset();
  if (voice_) {
    flype_voice_free(voice_);
    voice_ = nullptr;
  }
}

// 把后台载入完的结果收进来：核心句柄、两个「没读出来」的标记、整句模型的标记
void FlypeEngine::takeLoaded() {
  const FlypeLoaded loaded = loading_.get();
  core_ = loaded.core;
  userDictUnread_ = loaded.userDictUnread;
  historyUnread_ = loaded.historyUnread;
  nnlmStamp_ = loaded.nnlmStamp;
}

// 后台载入完了就收进来并灌上设置；还在载或没载成时返回
// false（调用方先用旧状态）
bool FlypeEngine::coreReady() {
  if (core_) {
    return true;
  }
  if (loading_.valid() &&
      loading_.wait_for(std::chrono::seconds(0)) == std::future_status::ready) {
    takeLoaded();
    if (wantedLexicon_ != lexicon_ || fileStamp(lexicon_) != lexiconStamp_) {
      // 载入途中配置换了词库：这个不用了，换新的
      maybeSwitchLexicon();
      return false;
    }
    if (core_) {
      applyCoreSettings();
    }
  }
  return core_ != nullptr;
}

// 用户词库的路径：环境变量 FLYPE_USER_DICT 优先，否则用户数据目录里的
// user.tsv（顺手建目录）
std::string FlypeEngine::userDictPath() const {
  if (const char *env = std::getenv("FLYPE_USER_DICT"); env && *env) {
    return env;
  }
  const std::string dir = userDataDir();
  if (dir.empty()) {
    return {};
  }
  std::error_code ec;
  std::filesystem::create_directories(dir, ec);
  return dir + "/user.tsv";
}

// 攒够 kSaveEvery 次（或者
// force）就把用户词、输入习惯交给核心的后台线程写盘；wait 是等写完
void FlypeEngine::maybeSaveUserDict(bool force, bool wait) {
  if (!core_) {
    return;
  }
  // 存盘在核心的后台线程上（ABI 1.11）：这里只抄一份数据，不等两次
  // fsync。上次写失败了现在才知道：
  // 记下来，这次接着存（写的都是整份，存成了就补上了）
  if (flype_engine_save_wait(core_, 0) < 0) {
    WLOG_W() << "Flype: 用户词库或输入习惯保存失败："
             << (flype_last_error() ? flype_last_error() : "unknown");
    unsavedCommits_ = std::max(unsavedCommits_, kSaveEvery);
  }
  const std::string path = userDictPath();
  if (!path.empty() && unsavedCommits_ > 0 &&
      (force || unsavedCommits_ >= kSaveEvery)) {
    // 载入时没读出来的文件，第一次存之前先改名留着（只做一次；改不了名就先不存那个文件，下次再试）
    if (userDictUnread_ && setAsideUnreadFile(path, "用户词库")) {
      userDictUnread_ = false;
    }
    const std::string history = historyPathFor(path);
    if (historyUnread_ &&
        (history.empty() || setAsideUnreadFile(history, "输入习惯"))) {
      historyUnread_ = false;
    }
    const bool dictQueued =
        !userDictUnread_ &&
        flype_engine_save_user_dict_async(core_, path.c_str()) == 0;
    const bool historyQueued =
        !historyUnread_ && !history.empty() &&
        flype_engine_save_history_async(core_, history.c_str()) == 0;
    if (dictQueued && historyQueued) {
      unsavedCommits_ = 0;
    }
  }
  if (wait && flype_engine_save_wait(core_, -1) < 0) {
    WLOG_W() << "Flype: 用户词库或输入习惯保存失败："
             << (flype_last_error() ? flype_last_error() : "unknown");
  }
}

// 会话说这一下学到了词就记一笔（攒够了一起存），返回有没有学到
bool FlypeEngine::countLearned(const FlypeFrame *frame) {
  if (!frame || !frame->learned || !core_) {
    return false;
  }
  ++unsavedCommits_;
  return true;
}

// 把 frame 里要上屏的字交给应用，再丢掉外壳抄的候选、照会话现在的样子重画
void FlypeEngine::finishCommit(fcitx::InputContext *ic, FlypeState *state,
                               const FlypeFrame *frame) {
  // 会话已经清空编辑区、记过选词和上文、补好了空格；这里上屏、攒着存用户词库
  const std::string text = frame && frame->commit ? frame->commit : "";
  const bool learned = countLearned(frame);
  if (ic && !text.empty()) {
    // 会话已经补好了空格
    ic->commitString(text);
  }
  // 会话上屏时已经清好了它上屏的那部分，这里从不 reset 会话（reset
  // 会把还在打的丢掉）：只丢掉抄的候选， 会话还在打字就重新抄
  state->forgetCandidates();
  if (state->composing()) {
    syncView(state);
  }
  if (learned) {
    maybeSaveUserDict(false);
  }
}

// 把编辑区里剩的字原样上屏（切英文、失去焦点时用），reset 的 1
// 是连编辑区一起清掉
void FlypeEngine::commitInput(fcitx::InputContext *ic, FlypeState *state) {
  if (!ic) {
    return;
  }
  finishCommit(ic, state, flype_session_reset(state->session_, 1));
}

// 把外壳的设置整份抄给会话：每页个数、翻页键、选词键、标点样式、联想等等
void FlypeEngine::syncSession(FlypeState *state) {
  flype_session_set_engine(state->session_, core_);
  FlypeSessionConfig config{};
  config.size = sizeof(config);
  config.page_size = static_cast<uint32_t>(pageSize_);
  config.calculator = calculator_ ? 1 : 0;
  config.date_time = dateTime_ ? 1 : 0;
  config.emoji_suggest = emojiSuggest_ ? 1 : 0;
  config.horizontal = horizontal_ ? 1 : 0;
  config.arrow_keys = arrowKeys_ == FlypeArrowKeys::Caret
                          ? FLYPE_ARROW_KEYS_CARET
                          : FLYPE_ARROW_KEYS_CANDIDATES;
  config.page_keys = (pageMinus_ ? FLYPE_PAGE_KEYS_MINUS : 0u) |
                     (pageBracket_ ? FLYPE_PAGE_KEYS_BRACKET : 0u) |
                     (pageComma_ ? FLYPE_PAGE_KEYS_COMMA : 0u) |
                     (pageTab_ ? FLYPE_PAGE_KEYS_TAB : 0u);
  config.select_keys = selectKeys_ == FlypeSelectKeys::Semicolon
                           ? FLYPE_SELECT_KEYS_SEMICOLON
                       : selectKeys_ == FlypeSelectKeys::SemicolonQuote
                           ? FLYPE_SELECT_KEYS_SEMICOLON_QUOTE
                           : FLYPE_SELECT_KEYS_OFF;
  config.pick_char = pickChar_ ? 1 : 0;
  config.punct_style = punctStyle_ == FlypePunctStyle::Halfwidth ? 1 : 0;
  config.auto_space = static_cast<uint32_t>(autoSpaceFlags_);
  config.shift_toggle =
      shiftToggle_ == FlypeShiftToggle::Left    ? FLYPE_SHIFT_TOGGLE_LEFT
      : shiftToggle_ == FlypeShiftToggle::Right ? FLYPE_SHIFT_TOGGLE_RIGHT
      : shiftToggle_ == FlypeShiftToggle::Off   ? FLYPE_SHIFT_TOGGLE_OFF
                                                : FLYPE_SHIFT_TOGGLE_BOTH;
  config.toolbar = toolbar_ ? 1 : 0;
  config.clipboard_history = clipboardHistory_ ? 1 : 0;
  config.association = association_ ? 1 : 0;
  flype_session_configure(state->session_, &config);
}

// 把一个按键交给会话：先把频率/设置和模式对齐，再附上密码框、周围文字这些提示
const FlypeFrame *FlypeEngine::sessionKey(fcitx::InputContext *ic,
                                          FlypeState *state,
                                          const fcitx::Key &key,
                                          uint32_t action) {
  coreReady();
  syncSession(state);
  syncMode(ic, state);
  uint32_t hints = 0;
  // 密码框、网址框、邮箱框不自动补空格
  if (!ic || ic->capabilityFlags().testAny(fcitx::CapabilityFlags{
                 fcitx::CapabilityFlag::PasswordOrSensitive,
                 fcitx::CapabilityFlag::Url, fcitx::CapabilityFlag::Email})) {
    hints |= FLYPE_KEY_HINT_NO_AUTO_SPACE;
  }
  if (ic &&
      (ic->capabilityFlags().test(fcitx::CapabilityFlag::PasswordOrSensitive) ||
       ic->capabilityFlags().test(fcitx::CapabilityFlag::Sensitive))) {
    // fcitx5 的 Password / Sensitive
    // 是这个输入框自己的能力位，确定是密码框：选的词也不学
    hints |= FLYPE_KEY_HINT_NO_LEARN;
  }
  // 读得到周围文字时会话拿光标前的字核对自己记的上文（在应用里删过、改过就作废）
  const auto prev = prevChar(ic);
  if (prev) {
    hints |= FLYPE_KEY_HINT_SURROUNDING;
  }
  return state->sessionKey(static_cast<uint32_t>(key.sym()),
                           static_cast<uint32_t>(key.states()), action,
                           prev.value_or(0), hints, nowMs());
}

// 光标前那一个字（自动补空格、删词靠它核对上文）；应用没报周围文字时返回空
std::optional<uint32_t> FlypeEngine::prevChar(fcitx::InputContext *ic) const {
  // 应用报告了周围文字才知道光标前是什么；选中了文字时看选区前面那个字
  if (!ic ||
      !ic->capabilityFlags().test(fcitx::CapabilityFlag::SurroundingText)) {
    return std::nullopt;
  }
  const auto &surrounding = ic->surroundingText();
  if (!surrounding.isValid()) {
    return std::nullopt;
  }
  const unsigned pos = std::min(surrounding.cursor(), surrounding.anchor());
  const std::string &before = surrounding.text();
  if (pos > fcitx::utf8::length(before)) {
    return std::nullopt;
  }
  if (pos == 0) {
    return 0;
  }
  const auto offset = fcitx::utf8::ncharByteLength(before.begin(), pos - 1);
  const uint32_t prev =
      fcitx::utf8::getChar(before.begin() + offset, before.end());
  return fcitx::utf8::isValidChar(prev) ? std::optional<uint32_t>(prev)
                                        : std::nullopt;
}

// fcitx5 全局设置里的翻页 / 移动候选热键翻成会话的动作；方向键留给会话自己判
uint32_t FlypeEngine::keyAction(const fcitx::Key &key) const {
  const auto sym = key.sym();
  if (sym == FcitxKey_Up || sym == FcitxKey_Down || sym == FcitxKey_Left ||
      sym == FcitxKey_Right) {
    return FLYPE_KEY_ACTION_NONE;
  }
  const auto &global = instance_->globalConfig();
  if (key.checkKeyList(global.defaultPrevPage())) {
    return FLYPE_KEY_ACTION_PREV_PAGE;
  }
  if (key.checkKeyList(global.defaultNextPage())) {
    return FLYPE_KEY_ACTION_NEXT_PAGE;
  }
  if (key.checkKeyList(global.defaultPrevCandidate())) {
    return FLYPE_KEY_ACTION_PREV_CANDIDATE;
  }
  if (key.checkKeyList(global.defaultNextCandidate())) {
    return FLYPE_KEY_ACTION_NEXT_CANDIDATE;
  }
  return FLYPE_KEY_ACTION_NONE;
}

// 按会话回的 frame 画候选框：有字就上屏，没字就重抄候选、原地挪页码或重画编辑区
void FlypeEngine::applySessionFrame(fcitx::InputContext *ic, FlypeState *state,
                                    const FlypeFrame *frame) {
  if (frame &&
      frame->size >= offsetof(FlypeFrame, forgot) + sizeof(frame->forgot) &&
      frame->forgot) {
    // Ctrl+Delete 忘掉了学错的词：马上存，免得 fcitx5 退出前没存上、重启又回来
    ++unsavedCommits_;
    maybeSaveUserDict(true);
  }
  if (frame && frame->commit) {
    // 学没学到词看 frame->learned；选了 emoji 由会话记进最近用过
    finishCommit(ic, state, frame);
    updateInputPanel(ic, state);
    return;
  }
  // 没有要上屏的字也可能学到了：记一次、攒够了存（frame
  // 在下一次调会话前有效，先读出来）
  const bool learned = countLearned(frame);
  if (!state->composing()) {
    state->forgetCandidates();
    updateInputPanel(ic, state);
    if (learned) {
      maybeSaveUserDict(false);
    }
    return;
  }
  const bool wasExpanded = state->expanded_;
  const bool recopied = syncView(state);
  if (learned) {
    maybeSaveUserDict(false);
  }
  if (recopied || wasExpanded || state->expanded_) {
    updateInputPanel(ic, state);
    return;
  }
  // 编辑区里的光标动了（左右键移进拼音，#129）：重画编辑区。只在左右键移光标时看
  if (arrowKeys_ == FlypeArrowKeys::Caret && frame && frame->preedit) {
    const fcitx::Text &shown = inlinePreedit(ic)
                                   ? ic->inputPanel().clientPreedit()
                                   : ic->inputPanel().preedit();
    if (shown.cursor() != frame->preedit_cursor) {
      updateInputPanel(ic, state);
      return;
    }
  }
  // 只是翻页、挪候选光标：原地摆好候选框
  if (auto *list = dynamic_cast<FlypeSessionList *>(
          ic->inputPanel().candidateList().get())) {
    list->mirror(state->panelPage_, state->panelCursor_);
    ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
  } else {
    updateInputPanel(ic, state);
  }
}

// 把会话的候选列表、页码、光标和展开状态抄进 state，返回候选有没有重算过
bool FlypeEngine::syncView(FlypeState *state) {
  FlypeSessionView view{};
  view.size = sizeof(view);
  if (flype_session_view(state->session_, &view) != 0) {
    return false;
  }
  const bool recopy = view.generation != state->candGeneration_;
  if (recopy) {
    refreshCandidates(state);
  }
  state->candGeneration_ = view.generation;
  state->panelPage_ = view.page;
  state->panelCursor_ = view.cursor;
  state->expanded_ = view.expanded != 0 && !state->candidates_.empty();
  state->gridCursor_ = view.grid_cursor;
  return recopy;
}

// 从会话重新抄一整份候选（字、拼音、注释、类型），没在打字也没开面板时就清空
void FlypeEngine::refreshCandidates(FlypeState *state) {
  state->candidates_.clear();
  state->candGeneration_ = UINT64_MAX;
  state->expanded_ = false;
  state->gridCursor_ = 0;
  state->panelPage_ = 0;
  state->panelCursor_ = 0;
  const bool panelOpen = state->panel() != FLYPE_TOOL_NONE;
  if (!panelOpen && !state->associating() &&
      (state->remaining().empty() || !coreReady())) {
    return;
  }
  // 候选（引擎候选 + 计算器、emoji 推荐、数字按键
  // emoji、日期时间，或者开着的工具栏 / 表情面板的条目）由会话算， fcitx5
  // 的候选框要整个列表，抄一份过来；点候选按这份拷贝的序号调会话
  syncSession(state);
  FlypeSessionView view{};
  view.size = sizeof(view);
  if (flype_session_view(state->session_, &view) == 0) {
    state->candGeneration_ = view.generation;
    state->panelPage_ = view.page;
    state->panelCursor_ = view.cursor;
    state->gridCursor_ = view.grid_cursor;
  }
  const std::size_t total = flype_session_candidate_total(state->session_);
  state->candidates_.reserve(total);
  for (std::size_t i = 0; i < total; ++i) {
    FlypeCandidateInfo info{};
    info.size = sizeof(info);
    if (flype_session_candidate_info(state->session_, i, &info) != 0) {
      continue;
    }
    FlypeCandidate candidate{info.text, info.pinyin, info.consumed};
    candidate.comment = info.comment;
    candidate.plain = info.kind == FLYPE_CAND_PLAIN;
    if (info.kind == FLYPE_CAND_EMOJI) {
      candidate.emoji = true;
      candidate.commit = info.commit;
      candidate.emojiText = info.emoji;
    }
    state->candidates_.push_back(std::move(candidate));
  }
}

// 最近用过的 emoji 存哪儿：用户数据目录里的 emoji-recent.txt
std::string FlypeEngine::recentEmojiPath() const {
  const std::string dir = userDataDir();
  return dir.empty() ? std::string() : dir + "/emoji-recent.txt";
}

// 从 fcitx5 的 clipboard 插件读一次当前剪贴板，记进历史；密码框里不记
void FlypeEngine::sampleClipboard(fcitx::InputContext *ic) {
  if (!clipboardHistory_ || !ic) {
    return;
  }
  // 密码框里不记（clipboard 插件自己也不记标成密码的内容）
  if (ic->capabilityFlags().testAny(
          fcitx::CapabilityFlag::PasswordOrSensitive)) {
    return;
  }
  auto *addon = instance_->addonManager().addon("clipboard");
  if (!addon) {
    return;
  }
  const std::string text = addon->call<fcitx::IClipboard::clipboard>(ic);
  if (!text.empty()) {
    flype_clipboard_remember(text.c_str());
  }
}

// 工具栏：把抄下来的候选项摆成竖排一列，一页 9 个，数字键选这一页
void FlypeEngine::fillToolPanel(fcitx::InputContext *ic, FlypeState *state) {
  // 工具栏（菜单、剪贴板、常用语）：竖排一列，一页 9 个，数字键选这一页的
  constexpr std::size_t kToolPageSize = 9;
  const std::size_t n = state->candidates_.size();
  if (n == 0) {
    return;
  }
  const std::size_t cursor = std::min(state->panelCursor_, n - 1);
  // 翻到哪一页由会话的光标定（鼠标翻页也交给会话），屏幕上的第 N 项就是按 N
  // 选的那一项
  auto list = std::make_unique<FlypePanelList>(this, ic, state);
  list->setLayoutHint(fcitx::CandidateLayoutHint::Vertical);
  list->setPageSize(static_cast<int>(kToolPageSize));
  list->setSelectionKey(fcitx::Key::keyListFromString("1 2 3 4 5 6 7 8 9"));
  for (std::size_t i = 0; i < n; ++i) {
    list->append<FlypeToolWord>(this, state, i, state->candidates_[i].text);
  }
  list->place(cursor / kToolPageSize, cursor);
  ic->inputPanel().setCandidateList(std::move(list));
}

// 候选框上的滚轮、翻页按钮按下的键：面板还开着就交给会话，再照会话的样子重画
void FlypeEngine::panelListKey(fcitx::InputContext *ic, FlypeState *state,
                               fcitx::KeySym sym) {
  if (!ic || !state) {
    return;
  }
  // 面板还开着才交给会话（没开着时这个键在会话里是别的意思）；不管怎样都照会话现在的样子重画
  FlypeSessionView view{};
  view.size = sizeof(view);
  const bool expanded = state->composing() &&
                        flype_session_view(state->session_, &view) == 0 &&
                        view.expanded != 0;
  if (state->panel() != FLYPE_TOOL_NONE || expanded) {
    if (const FlypeFrame *frame = sessionKey(ic, state, fcitx::Key(sym));
        frame && frame->handled) {
      applySessionFrame(ic, state, frame);
      return;
    }
  }
  updateInputPanel(ic, state);
}

// 点了工具栏 / 表情面板的第 index
// 项：进下一层菜单还是上屏由会话定（表情会话会记下）
void FlypeEngine::choosePanelItem(fcitx::InputContext *ic, FlypeState *state,
                                  std::size_t index) {
  // 点了工具栏 /
  // 表情面板的一项：菜单进下一层，别的上屏（会话做，表情记进最近用过）
  if (!ic) {
    return;
  }
  if (state->panel() == FLYPE_TOOL_MENU) {
    sampleClipboard(ic);
  }
  const FlypeFrame *frame = flype_session_choose(state->session_, index);
  if (frame && frame->handled) {
    applySessionFrame(ic, state, frame);
  }
}

/// 还没选词的拼音，按第一个用完它的候选分好音节（hengxiang -> heng'xiang）。
/// 显示的还是用户打的字母；英文等切不了的原样显示。
std::string FlypeEngine::segmentedRemaining(FlypeState *state) {
  // 会话的编辑区 = 已选的词 + 分好音节的剩余部分，这里只要后半截
  syncSession(state);
  const FlypeFrame *frame = flype_session_frame(state->session_);
  const std::string selected = state->selectedText();
  const std::string shown = frame && frame->preedit ? frame->preedit : "";
  if (shown.compare(0, selected.size(), selected) == 0) {
    return shown.substr(selected.size());
  }
  return state->remaining();
}

// 编辑区是不是画在应用的输入框里（不是的话就画在候选窗上面）
bool FlypeEngine::inlinePreedit(fcitx::InputContext *ic) const {
  return instance_->globalConfig().preeditEnabledByDefault() &&
         ic->capabilityFlags().test(fcitx::CapabilityFlag::Preedit);
}

// 按 inlinePreedit 把编辑区交给输入框（标 DontCommit）或交给候选窗
void FlypeEngine::sendPreedit(fcitx::InputContext *ic,
                              const fcitx::Text &preedit) {
  auto &panel = ic->inputPanel();
  if (inlinePreedit(ic)) {
    setClientPreedit(panel, preedit);
  } else {
    panel.setPreedit(preedit);
  }
}

// 照会话现在的状态把整个候选框重画一遍：面板、编辑区、候选列表、联想行
void FlypeEngine::updateInputPanel(fcitx::InputContext *ic, FlypeState *state) {
  auto &panel = ic->inputPanel();
  panel.reset();
  state->associationShown_ = false;

  if (const int open = state->panel(); open != FLYPE_TOOL_NONE) {
    // 工具栏 / 表情面板：条目、光标、提示、搜索词都是会话给的
    syncView(state);
    const FlypeFrame *frame = flype_session_frame(state->session_);
    fcitx::Text preedit;
    preedit.append(frame && frame->preedit ? frame->preedit : "",
                   fcitx::TextFormatFlag::Underline);
    preedit.setCursor(static_cast<int>(preedit.textLength()));
    sendPreedit(ic, preedit);
    if (frame && frame->aux) {
      panel.setAuxUp(fcitx::Text(frame->aux));
    }
    if (!state->candidates_.empty()) {
      if (open == FLYPE_TOOL_EMOJI || open == FLYPE_TOOL_SYMBOLS) {
        fillGridPanel(ic, state);
      } else {
        fillToolPanel(ic, state);
      }
    }
    ic->updatePreedit();
    ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
    return;
  }

  if (!state->composing()) {
    if (state->associating()) {
      fillAssociation(panel, state);
      state->associationShown_ = true;
    }
    ic->updatePreedit();
    ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
    return;
  }

  {
    // 编辑区：已选的词 +
    // 还没选的拼音（带下划线）；光标位置会话给（左右键能移到拼音中间，#129）
    fcitx::Text preedit;
    preedit.append(state->selectedText());
    preedit.append(segmentedRemaining(state), fcitx::TextFormatFlag::Underline);
    const FlypeFrame *frame = flype_session_frame(state->session_);
    const std::size_t cursor =
        frame && frame->preedit ? frame->preedit_cursor : preedit.textLength();
    preedit.setCursor(static_cast<int>(std::min(cursor, preedit.textLength())));
    sendPreedit(ic, preedit);
    if (frame && frame->aux) {
      // 会话给的提示行（可以没有候选，只有这一行）
      panel.setAuxUp(fcitx::Text(frame->aux));
    }
  }

  if (state->expanded_ && !state->candidates_.empty()) {
    fillGridPanel(ic, state);
  } else if (!state->candidates_.empty()) {
    auto candidates = std::make_unique<FlypeSessionList>(state);
    candidates->setLayoutHint(horizontal_
                                  ? fcitx::CandidateLayoutHint::Horizontal
                                  : fcitx::CandidateLayoutHint::Vertical);
    candidates->setPageSize(static_cast<int>(pageSize_));
    candidates->setSelectionKey(fcitx::KeyList{
        fcitx::Key(FcitxKey_1), fcitx::Key(FcitxKey_2), fcitx::Key(FcitxKey_3),
        fcitx::Key(FcitxKey_4), fcitx::Key(FcitxKey_5), fcitx::Key(FcitxKey_6),
        fcitx::Key(FcitxKey_7), fcitx::Key(FcitxKey_8),
        fcitx::Key(FcitxKey_9)});
    candidates->setCursorPositionAfterPaging(
        fcitx::CursorPositionAfterPaging::ResetToFirst);
    for (std::size_t i = 0; i < state->candidates_.size(); ++i) {
      candidates->append<FlypeCandidateWord>(
          this, state, i, shownCandidate(state->candidates_[i]));
    }
    // 页码和光标照会话的（展开面板里走到后面再收起时，会话翻到光标那一页）
    candidates->mirror(
        state->panelPage_,
        std::min(state->panelCursor_, state->candidates_.size() - 1));
    panel.setCandidateList(std::move(candidates));
  }

  ic->updatePreedit();
  ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}

/// 上屏以后的联想：会话给的一页，照普通候选框画，上方是会话给的提示。没按 Tab
/// 进联想行时不画光标、 不画序号（数字不选联想，照常交给应用）；进了以后光标和
/// 1–9 照常。鼠标点了和普通候选一样走 commitCandidate。
void FlypeEngine::fillAssociation(fcitx::InputPanel &panel, FlypeState *state) {
  syncView(state);
  const FlypeFrame *frame = flype_session_frame(state->session_);
  if (frame && frame->aux) {
    panel.setAuxUp(fcitx::Text(frame->aux));
  }
  const int cursor = frame ? frame->cand_cursor : -1;
  const std::size_t n = state->candidates_.size();
  if (n == 0) {
    return;
  }
  auto list = std::make_unique<fcitx::CommonCandidateList>();
  list->setLayoutHint(horizontal_ ? fcitx::CandidateLayoutHint::Horizontal
                                  : fcitx::CandidateLayoutHint::Vertical);
  list->setPageSize(static_cast<int>(n));
  if (cursor >= 0) {
    list->setSelectionKey(fcitx::Key::keyListFromString("1 2 3 4 5 6 7 8 9"));
  } else {
    list->setSelectionKey(fcitx::KeyList{});
  }
  for (std::size_t i = 0; i < n; ++i) {
    list->append<FlypeCandidateWord>(this, state, i,
                                     shownCandidate(state->candidates_[i]));
  }
  if (cursor >= 0 && static_cast<std::size_t>(cursor) < n) {
    list->setGlobalCursorIndex(cursor);
  }
  panel.setCandidateList(std::move(list));
}

/// 展开的候选面板：竖排列表的每一项是一行（kGridCols 个词），一屏 kGridRows
/// 行。
///
/// 照微信输入法的样子：光标所在行标 １–６，光标处的词用〔〕框起来；同一屏里各列
/// 按最宽的词补齐，这样上下键在同一列里走。classic UI 和 kimpanel
/// 都只会画一行或 一列，画不了真正的网格，所以用全角字符拼出来。
void FlypeEngine::fillGridPanel(fcitx::InputContext *ic, FlypeState *state) {
  static const char *kLabels[] = {"\uFF11", "\uFF12", "\uFF13",
                                  "\uFF14", "\uFF15", "\uFF16"};
  static_assert(sizeof(kLabels) / sizeof(kLabels[0]) == kGridCols);
  const auto &all = state->candidates_;
  const std::size_t n = all.size();
  state->gridCursor_ = std::min(state->gridCursor_, n - 1);
  const std::size_t rows = (n + kGridCols - 1) / kGridCols;
  const std::size_t cursorRow = state->gridCursor_ / kGridCols;

  // 一屏一屏翻也交给会话（FlypePanelList）：光标跟着到新的一屏，数字、空格选的是看到的这一屏
  auto list = std::make_unique<FlypePanelList>(this, ic, state);
  list->setLayoutHint(fcitx::CandidateLayoutHint::Vertical);
  list->setPageSize(static_cast<int>(kGridRows));
  list->setSelectionKey(fcitx::KeyList{}); // 不要行号：编号画在行里
  for (std::size_t pageStart = 0; pageStart < rows; pageStart += kGridRows) {
    const std::size_t pageEnd = std::min(rows, pageStart + kGridRows);
    std::size_t widths[kGridCols] = {};
    for (std::size_t r = pageStart; r < pageEnd; ++r) {
      for (std::size_t c = 0; c < kGridCols && r * kGridCols + c < n; ++c) {
        widths[c] = std::max(
            widths[c],
            displayWidth(shownCandidate(all[r * kGridCols + c]).toString()));
      }
    }
    for (std::size_t r = pageStart; r < pageEnd; ++r) {
      fcitx::Text line;
      for (std::size_t c = 0; c < kGridCols && r * kGridCols + c < n; ++c) {
        const std::size_t index = r * kGridCols + c;
        const bool here = index == state->gridCursor_;
        const fcitx::Text shown = shownCandidate(all[index]);
        std::string after = here ? "\u3015" : "";
        padTo(after, displayWidth(shown.toString()), widths[c]);
        line.append(std::string(r == cursorRow ? kLabels[c] : "\u3000") +
                    (here ? "\u3014" : "\u3000"));
        for (std::size_t i = 0; i < shown.size(); ++i) {
          line.append(shown.stringAt(i), shown.formatAt(i));
        }
        line.append(after + (here ? "" : "\u3000"));
      }
      list->append<FlypeGridRow>(this, state, r, std::move(line));
    }
  }
  // 先翻到光标所在的那一屏，下键走到第 7 行以后才看得到
  list->place(cursorRow / kGridRows, cursorRow);
  ic->inputPanel().setCandidateList(std::move(list));
}

// 展开面板里点了一行：光标就在这一行就选光标处的词，否则选这一行第一个
void FlypeEngine::selectGridRow(fcitx::InputContext *ic, FlypeState *state,
                                std::size_t row) {
  if (!state || state->candidates_.empty()) {
    return;
  }
  const std::size_t index = state->gridCursor_ / kGridCols == row
                                ? state->gridCursor_
                                : row * kGridCols;
  commitCandidate(ic, state, std::min(index, state->candidates_.size() - 1));
}

namespace {

/// 会话第 index 个候选还是不是外壳显示的那个。
bool sameCandidate(FlypeState *state, std::size_t index,
                   const FlypeCandidate &shown) {
  FlypeCandidateInfo info{};
  info.size = sizeof(info);
  if (flype_session_candidate_info(state->session_, index, &info) != 0) {
    return false;
  }
  return shown.text == info.text && shown.pinyin == info.pinyin &&
         shown.comment == info.comment && shown.consumed == info.consumed &&
         shown.plain == (info.kind == FLYPE_CAND_PLAIN) &&
         shown.emoji == (info.kind == FLYPE_CAND_EMOJI);
}

} // namespace

// 选了第 index 个候选：交给会话上屏、学习；显示的那份早就对不上时先刷新
void FlypeEngine::commitCandidate(fcitx::InputContext *ic, FlypeState *state,
                                  std::size_t index) {
  if (!ic || !state || index >= state->candidates_.size()) {
    return;
  }
  if (state->panel() != FLYPE_TOOL_NONE) {
    choosePanelItem(ic, state, index);
    return;
  }
  // 选词、上屏、学习由会话做；外壳的候选是会话候选的一份拷贝，序号一样
  const FlypeCandidate chosen = state->candidates_[index];
  if (!sameCandidate(state, index, chosen)) {
    // 会话的候选在显示以后变了（上文、设置、引擎变了又没刷新）：选同一个词，找不到就只刷新显示
    index = SIZE_MAX;
    const std::size_t total = flype_session_candidate_total(state->session_);
    for (std::size_t i = 0; i < total; ++i) {
      if (sameCandidate(state, i, chosen)) {
        index = i;
        break;
      }
    }
    if (index == SIZE_MAX) {
      refreshCandidates(state);
      updateInputPanel(ic, state);
      return;
    }
  }
  const FlypeFrame *frame = flype_session_choose(state->session_, index);
  if (!frame || !frame->handled) {
    return;
  }
  if (frame->commit) {
    finishCommit(ic, state, frame);
  } else {
    // 句首词进了已选栈，剩下的拼音继续选
    const bool learned = countLearned(frame);
    refreshCandidates(state);
    if (learned) {
      maybeSaveUserDict(false);
    }
  }
  updateInputPanel(ic, state);
}

// 和按空格一样：上屏光标处的候选（没有候选时上屏原始输入）
void FlypeEngine::commitCurrent(fcitx::InputContext *ic, FlypeState *state) {
  // 和按空格一样：上屏光标处的候选，没有候选时上屏原始输入
  const FlypeFrame *frame = sessionKey(ic, state, fcitx::Key(FcitxKey_space));
  if (frame && frame->handled) {
    applySessionFrame(ic, state, frame);
  }
}

// fcitx5
// 的按键入口：语音键、Shift、表情键、工具栏、英文模式、打字中、没在打字，挑一条路走
void FlypeEngine::keyEvent(const fcitx::InputMethodEntry & /*entry*/,
                           fcitx::KeyEvent &event) {
  auto *ic = event.inputContext();
  if (!ic) {
    return;
  }
  auto *state = ic->propertyFor(&factory_);
  const fcitx::Key key = event.key();
  const auto sym = key.sym();
  // 联想开着时按了别的键：会话关掉了联想、键交给了应用（没有 frame
  // 要画）时，候选框上的联想也收起来。 放在最后（各条路都处理完、frame
  // 用完以后）
  struct AfterKey {
    std::function<void()> done;
    ~AfterKey() { done(); }
  } afterKey{[this, ic, state] {
    if (state->associationShown_ && !state->associating()) {
      updateInputPanel(ic, state);
    }
  }};

  if (voice_ && handleVoiceKey(ic, state, event)) {
    return;
  }

  // 单按 Shift 切中英文（Flype 自己的英文模式），由会话判断。fcitx
  // 的「临时切换输入法」热键 （默认左 Shift）生效时 fcitx 会先把 Shift
  // 吃掉，这里就收不到，两者不会重复切换。
  if (handleShiftKey(ic, state, event)) {
    return;
  }
  if (event.isRelease()) {
    return;
  }

  if (isEmojiKey(key)) {
    // 表情面板快捷键：开着就关上；打了一半的拼音当搜索词（会话做）
    coreReady();
    syncSession(state);
    applySessionFrame(ic, state,
                      flype_session_toggle_emoji_panel(state->session_));
    event.filterAndAccept();
    return;
  }
  if (const uint32_t action = modeKeyAction(key);
      action && state->panel() == FLYPE_TOOL_NONE) {
    // 另设的切换键（中 ↔ 英）：会话切
    const FlypeFrame *frame = sessionKey(ic, state, key, action);
    if (frame && frame->handled) {
      followSessionMode(ic, state, frame);
      event.filterAndAccept();
    }
    return;
  }
  if (state->panel() != FLYPE_TOOL_NONE) {
    // 工具栏 / 表情面板开着：按键都归它（会话管）；带 Ctrl / Alt / Super
    // 的收起面板、交给应用
    if (state->panel() == FLYPE_TOOL_MENU) {
      sampleClipboard(ic); // 下一步可能进「剪贴板」页
    }
    const FlypeFrame *frame = sessionKey(ic, state, key);
    if (frame && frame->handled) {
      applySessionFrame(ic, state, frame);
      event.filterAndAccept();
    } else {
      updateInputPanel(ic, state);
    }
    return;
  }

  // 英文模式：按键原样交给应用（字母、半角标点都直接上屏）；汉字后面打字母、数字要补空格时
  // 会话连空格一起上屏
  if (asciiMode_ && !state->composing()) {
    if (const FlypeFrame *frame = sessionKey(ic, state, key);
        frame && frame->handled) {
      applySessionFrame(ic, state, frame);
      event.filterAndAccept();
    }
    return;
  }

  // 带 Ctrl/Alt/Super 的组合键先问会话：打字途中
  // Ctrl+Delete、Ctrl+退格忘掉光标处学错的词； 别的交给应用或 fcitx
  // 的其他处理器（没在打字时会话借这一下知道应用里的字可能变了）
  if (key.states().testAny(fcitx::KeyStates{fcitx::KeyState::Ctrl,
                                            fcitx::KeyState::Alt,
                                            fcitx::KeyState::Super})) {
    if (const FlypeFrame *frame = sessionKey(ic, state, key);
        frame && frame->handled) {
      applySessionFrame(ic, state, frame);
      event.filterAndAccept();
    }
    return;
  }

  if (state->composing()) {
    // 只打了一个 v 再按空格打开工具栏：会话做（剪贴板页要的历史先读一次）
    if (sym == FcitxKey_space && state->input() == "v") {
      sampleClipboard(ic);
    }
    // 字母、隔音符、退格、Esc、空格、回车、数字、分号引号选词、V 模式算式、
    // 翻页、移动候选、展开面板都由会话处理。fcitx5
    // 全局设置里的翻页、移动候选热键 （[Hotkey/PrevPage] 等，比如 , .
    // 翻页）翻成动作一起交给会话：按了就算翻不动也吃掉， 不让它变成标点把字上屏
    const bool wasExpanded = state->expanded_;
    const FlypeFrame *frame = sessionKey(ic, state, key, keyAction(key));
    if (frame && frame->handled) {
      applySessionFrame(ic, state, frame);
      event.filterAndAccept();
      return;
    }
    if (wasExpanded) {
      // 展开面板里按了会话不管的键（标点、Home……）：面板已经收起，重画再照常处理
      syncView(state);
      updateInputPanel(ic, state);
    }

    // 标点、符号（先上屏候选再上屏标点）也由会话处理了。会话不管的键（Home /
    // End / Delete / Tab、 小键盘的 + - * / .、F1–F12、Insert、不是 ASCII
    // 的键……）都吃掉：编辑区里的字还没上屏，
    // 这时交给应用，应用里的光标会乱跑，小键盘的 + 排到「你好」前面，F5
    // 把网页刷新掉。 修饰键、锁定键本身照常交给应用（按住 Ctrl
    // 再按别的键、大写锁定），不然应用里对不上按下和松开
    if (!passesWhileComposing(sym)) {
      event.filterAndAccept();
    }
    return;
  }

  // 没在打字时也交给会话：字母（大写原样保留，Hello、iPhone
  // 要用到）开始编辑区； 标点换成中文标点（半角样式、数字后面的、没有中文写法的
  // / + 交给应用）； 汉字后面打数字要补空格时会话自己上屏（第5章）
  if (const FlypeFrame *frame = sessionKey(ic, state, key);
      frame && frame->handled) {
    applySessionFrame(ic, state, frame);
    event.filterAndAccept();
    return;
  }
}

// 输入框拿到焦点 / 从别的输入法切回来：清掉旧会话，按需要定一下中英
void FlypeEngine::activate(const fcitx::InputMethodEntry & /*entry*/,
                           fcitx::InputContextEvent &event) {
  auto *ic = event.inputContext();
  if (!ic) {
    return;
  }
  auto *state = ic->propertyFor(&factory_);
  state->clear();
  coreReady();
  // 从别的输入法切回来（不是换窗口获得焦点）、启动后第一次用：按「切到 Flype
  // 时」定中 / 英
  // （#158：切走时是英文，切回来还是英文，打不了中文）。换窗口不动，模式照旧全局保持；
  // 进出密码框时 fcitx5
  // 自己换成键盘布局再换回来（CapabilityChanged），也不算用户切回来。
  bool switchedBack = false;
  if (event.type() == fcitx::EventType::InputContextSwitchInputMethod) {
    const auto &switched =
        static_cast<fcitx::InputContextSwitchInputMethodEvent &>(event);
    switchedBack = switched.reason() !=
                   fcitx::InputMethodSwitchedReason::CapabilityChanged;
  }
  if (switchedBack || !initialModeApplied_) {
    initialModeApplied_ = true;
    applyInitialMode(ic, state);
  }
  // 最近用过的 emoji 在核心里（所有输入框共用），存在
  // ~/.local/share/flype/emoji-recent.txt
  flype_recent_emoji_path(recentEmojiPath().c_str());
  sampleClipboard(ic);
}

// 失去焦点或被切走：切走时把打的字上屏，失焦时丢掉；顺便存一次用户词库
void FlypeEngine::deactivate(const fcitx::InputMethodEntry &entry,
                             fcitx::InputContextEvent &event) {
  // 打到一半切换输入法（比如按 Shift 切英文）：已选的词 + 原始字母原样上屏，
  // 不要把用户打的东西吞掉。失去焦点时放弃（reset）：这时再上屏，Qt
  // 会送进新拿到焦点的控件， XIM 的应用收不到；编辑区标了
  // DontCommit，工具包自己也不上屏。
  if (auto *ic = event.inputContext();
      ic && event.type() == fcitx::EventType::InputContextSwitchInputMethod) {
    auto *state = ic->propertyFor(&factory_);
    finishCommit(ic, state, flype_session_reset(state->session_, 1));
  }
  if (voiceActive() && voiceIc_.get() == event.inputContext()) {
    cancelVoice();
  }
  // 按住语音键时焦点切走（或者换了输入法）：松开的那一下到不了这里。不清掉的话，回来后第一次按语音键
  // 会被当成还按着时的自动重复吃掉，没有反应
  voiceKeyDown_ = false;
  reset(entry, event);
  maybeSaveUserDict(true);
}

// 应用重置输入框（点了别处、失去焦点）：丢掉拼音和会话记的上文，清空候选框
void FlypeEngine::reset(const fcitx::InputMethodEntry & /*entry*/,
                        fcitx::InputContextEvent &event) {
  auto *ic = event.inputContext();
  if (!ic) {
    return;
  }
  // 拼音只在输入法里：应用重置输入框（点了别处、失去焦点）就丢掉，会话记的上文也作废
  // （回来时光标可能在别处）。编辑区标了 DontCommit，工具包不会自己上屏。
  auto *state = ic->propertyFor(&factory_);
  state->clear();
  ic->inputPanel().reset();
  ic->updatePreedit();
  ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}

// 当前模式的完整名字，给提示框、托盘用
const char *FlypeEngine::modeTitle() const {
  return asciiMode_ ? "英文" : "中文";
}

// 当前模式的一个字（中 / 英），状态栏和托盘上显示这个
const char *FlypeEngine::modeLabel() const { return asciiMode_ ? "英" : "中"; }

// fcitx5 状态区向当前模式名
std::string FlypeEngine::subMode(const fcitx::InputMethodEntry & /*entry*/,
                                 fcitx::InputContext & /*ic*/) {
  return modeTitle();
}

// 状态区上的短名字，用的是 modeLabel
std::string
FlypeEngine::subModeLabelImpl(const fcitx::InputMethodEntry & /*entry*/,
                              fcitx::InputContext & /*ic*/) {
  // 中文也要给：给空的话切回中文时提示框、托盘写的是输入法的名字「Flype」，和「英」对不上
  return modeLabel();
}

/// 按下 Shift 记下来，松开时中间没按别的键（也没按太久）就切中英文。
bool FlypeEngine::handleShiftKey(fcitx::InputContext *ic, FlypeState *state,
                                 fcitx::KeyEvent &event) {
  coreReady();
  syncSession(state);
  syncMode(ic, state);
  const fcitx::Key key = event.key();
  const auto sym = key.sym();
  const bool isShift = sym == FcitxKey_Shift_L || sym == FcitxKey_Shift_R;
  FlypeKey raw{};
  raw.size = sizeof(raw);
  raw.keysym = static_cast<uint32_t>(sym);
  raw.modifiers = static_cast<uint32_t>(key.states());
  raw.release = event.isRelease() ? 1 : 0;
  // 别的键只报给会话（按住 Shift 时按了它，这一下就不算单按），不让会话处理
  raw.action = isShift ? FLYPE_KEY_ACTION_NONE : FLYPE_KEY_ACTION_OBSERVE;
  // 松开时按键码认：xkb 选项（比如 grp:shifts_toggle）或 fcitx5
  // 按输入法布局重新翻译时， 松开那一下 Shift 可能翻成别的符号（ISO_Next_Group
  // 等）
  raw.code = static_cast<uint32_t>(event.origKey().code());
  // 按住不放的自动重复不重新计时（key() 规整时去掉了 Repeat，要看 rawKey）
  raw.repeat = event.rawKey().states().test(fcitx::KeyState::Repeat) ? 1 : 0;
  raw.time_ms = static_cast<uint64_t>(fcitx::now(CLOCK_MONOTONIC) / 1000);
  const bool wasExpanded = state->expanded_;
  const FlypeFrame *frame = flype_session_key(state->session_, &raw);
  if (!frame || !frame->mode_changed) {
    if (isShift) {
      // Shift
      // 只走这一条路（再交给会话一次会把按下的时刻、键码冲掉）。展开面板里按下
      // Shift： 会话已经把面板收起来了，重画；Shift 本身交给应用
      if (wasExpanded) {
        syncView(state);
        updateInputPanel(ic, state);
      }
      return true;
    }
    return false;
  }
  followSessionMode(ic, state, frame);
  event.filterAndAccept();
  return true;
}

// 会话切了中英（Shift 或切换键）：先把 frame 里的字上屏，再改外壳的模式并重画
void FlypeEngine::followSessionMode(fcitx::InputContext *ic, FlypeState *state,
                                    const FlypeFrame *frame) {
  // 打到一半切英文：已选的词 + 打的字母原样上屏（会话已经放进 frame）
  const int mode = frame->mode;
  const bool committed = frame->commit != nullptr;
  if (committed) {
    finishCommit(ic, state, frame);
  } else {
    countLearned(frame);
  }
  switchMode(ic, state, mode == FLYPE_MODE_ENGLISH, true);
  if (committed || state->composing()) {
    // 换好模式再画：留着的拼音照普通的编辑区画
    updateInputPanel(ic, state);
  }
}

// 按设置决定切到这个输入法时是中文还是英文（「上一个」就不管）
void FlypeEngine::applyInitialMode(fcitx::InputContext *ic, FlypeState *state) {
  if (initialMode_ == FlypeInitialMode::Last) {
    return;
  }
  // 切不回中文时不进英文
  const bool english =
      initialMode_ == FlypeInitialMode::English && canToggleMode();
  if (english == asciiMode_) {
    return;
  }
  switchMode(ic, state, english, false);
}

// 把外壳的中 / 英灌进会话（模式是全局的，每个输入框的会话各对齐一次）
void FlypeEngine::syncMode(fcitx::InputContext *ic, FlypeState *state) {
  // 中 /
  // 英是全局的（在一个应用里切了，别的应用里也是）；会话每个输入框一份，按键前对齐
  const int want = asciiMode_ ? FLYPE_MODE_ENGLISH : FLYPE_MODE_CHINESE;
  if (flype_session_mode(state->session_) == want) {
    return;
  }
  const FlypeFrame *frame = flype_session_set_mode(state->session_, want);
  if (frame && frame->commit && ic) {
    finishCommit(ic, state, frame);
    updateInputPanel(ic, state);
  } else {
    countLearned(frame);
  }
}

// 真正切模式：打到一半要切英文先把字上屏，再记住模式、同步会话、通知界面
void FlypeEngine::switchMode(fcitx::InputContext *ic, FlypeState *state,
                             bool ascii, bool notify) {
  if (ascii && !asciiMode_ && ic && state && state->composing()) {
    // 打到一半切英文：已选的词 + 打的字母原样上屏（和微信输入法一样）
    commitInput(ic, state);
    updateInputPanel(ic, state);
  }
  asciiMode_ = ascii;
  if (state) {
    syncMode(ic, state);
  }
  WLOG_I() << "Flype: 切到" << modeTitle();
  if (!ic) {
    return;
  }
  if (notify) {
    instance_->showCustomInputMethodInformation(ic, modeLabel());
  }
  ic->updateUserInterface(fcitx::UserInterfaceComponent::StatusArea);
}

// 语音热键：按下开始录音并起一个轮询定时器，松开就结束；说话途中 Esc
// 取消，按别的键放弃
bool FlypeEngine::handleVoiceKey(fcitx::InputContext *ic, FlypeState *state,
                                 fcitx::KeyEvent &event) {
  const fcitx::Key key = event.key();
  if (isVoiceKey(key)) {
    if (event.isRelease()) {
      if (!voiceKeyDown_) {
        return false;
      }
      voiceKeyDown_ = false;
      event.filterAndAccept();
      if (voiceActive()) {
        flype_voice_key_up(voice_);
        pollVoice();
      }
      return true;
    }
    if (voiceKeyDown_) {
      // 按住时的自动重复
      event.filterAndAccept();
      return true;
    }
    // 正在打拼音时不抢这个键
    if (state->composing() && !voiceActive()) {
      return false;
    }
    voiceKeyDown_ = true;
    event.filterAndAccept();
    if (!voiceActive()) {
      voiceIc_ = ic->watch();
      voiceShowing_ = true;
      voiceErrorUntil_ = 0;
    }
    flype_voice_key_down(voice_);
    pollVoice();
    if (!voiceTimer_) {
      voiceTimer_ = instance_->eventLoop().addTimeEvent(
          CLOCK_MONOTONIC, fcitx::now(CLOCK_MONOTONIC) + kVoicePollUsec,
          kTimerAccuracyUsec, [this](fcitx::EventSourceTime *source, uint64_t) {
            if (pollVoice()) {
              source->setNextInterval(kVoicePollUsec);
              source->setOneShot();
            }
            return true;
          });
    } else {
      voiceTimer_->setNextInterval(kVoicePollUsec);
      voiceTimer_->setOneShot();
    }
    return true;
  }

  if (!voiceActive() || event.isRelease()) {
    return false;
  }
  if (key.sym() == FcitxKey_Escape) {
    cancelVoice();
    event.filterAndAccept();
    return true;
  }
  // 修饰键单独按下（比如按住热键时碰到 Shift）不打断
  if (key.isModifier()) {
    return false;
  }
  // 说话途中打别的键：不要这次的语音，键照常处理
  cancelVoice();
  return false;
}

// 轮询语音会话的状态：画音量 /
// 进度，出结果就上屏，出错就提示；返回还要不要接着轮询
bool FlypeEngine::pollVoice() {
  if (!voice_ || !voiceShowing_) {
    return false;
  }
  auto *ic = voiceIc_.get();
  if (!ic) {
    // 输入框没了
    cancelVoice();
    return false;
  }
  const int phase = flype_voice_poll(voice_);
  const std::string text = flype_voice_text(voice_);
  switch (phase) {
  case FLYPE_VOICE_LISTENING: {
    const int level = flype_voice_level(voice_);
    // 不写字，只画麦克风 + 跟着音量滚动的波形
    voiceLevels_.push_back(level);
    if (voiceLevels_.size() > kVoiceBars) {
      voiceLevels_.erase(voiceLevels_.begin());
    }
    showVoice(ic, "🎙 " + voiceWave(voiceLevels_) + " ", text);
    return true;
  }
  case FLYPE_VOICE_RECOGNIZING: {
    static const char *const kDots[] = {"·    ", "··   ", "···  ", "···· ",
                                        "·····"};
    showVoice(ic, std::string("🎙 ") + kDots[(voiceFrame_++ / 3) % 5] + " ",
              text);
    return true;
  }
  case FLYPE_VOICE_RESULT: {
    clearVoice();
    // 和工具栏、表情面板上屏的一样交给会话：补空格、记上文
    auto *state = ic->propertyFor(&factory_);
    coreReady();
    syncSession(state);
    finishCommit(ic, state,
                 flype_session_commit_text(state->session_, text.c_str(),
                                           prevChar(ic).value_or(0)));
    // 上屏以后再复制，复制不成也不影响上屏；密码框里说的话不进剪贴板
    if (voiceCopy_ &&
        !ic->capabilityFlags().testAny(
            fcitx::CapabilityFlag::PasswordOrSensitive) &&
        !flype::copyToClipboard(text)) {
      WLOG_I() << "Flype: 没能复制到剪贴板（要装 wl-clipboard）";
    }
    return false;
  }
  case FLYPE_VOICE_ERROR:
    WLOG_I() << "Flype: 语音 " << text;
    voiceErrorText_ = text;
    showVoice(ic, "🎙 " + text, {});
#ifdef FLYPE_CONFIG_TOOL
    // 没下载模型：提示只有几秒、点不了，直接把设置界面打开（语音那一节有下载按钮）。
    // 一次运行里只开一次，不想用语音的人误按语音键不会被反复打扰；测试里不开
    if (static bool opened = false;
        !opened &&
        flype_voice_model_state(voice_) == FLYPE_VOICE_MODEL_MISSING &&
        !std::getenv("FLYPE_NO_OPEN_SETTINGS")) {
      opened = true;
      std::error_code ec;
      if (std::filesystem::exists(FLYPE_CONFIG_TOOL, ec)) {
        fcitx::startProcess({FLYPE_CONFIG_TOOL});
      }
    }
#endif
    // 没下载模型的提示要去设置里操作，多留一会儿让人看清
    voiceErrorUntil_ =
        fcitx::now(CLOCK_MONOTONIC) +
        (flype_voice_model_state(voice_) == FLYPE_VOICE_MODEL_MISSING
             ? kVoiceHintUsec
             : kVoiceErrorUsec);
    return true;
  default:
    if (voiceErrorUntil_ && fcitx::now(CLOCK_MONOTONIC) < voiceErrorUntil_) {
      return true;
    }
    clearVoice();
    return false;
  }
}

// 语音的候选框：上面一行状态，下面画识别到一半的字
void FlypeEngine::showVoice(fcitx::InputContext *ic, const std::string &status,
                            const std::string &partial) {
  auto &panel = ic->inputPanel();
  panel.reset();
  if (!partial.empty()) {
    fcitx::Text preedit;
    preedit.append(partial, fcitx::TextFormatFlag::Underline);
    preedit.setCursor(static_cast<int>(preedit.textLength()));
    sendPreedit(ic, preedit);
  }
  if (!status.empty()) {
    panel.setAuxUp(fcitx::Text(status));
  }
  ic->updatePreedit();
  ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}

// 收起语音提示，把候选框退回说话前的样子
void FlypeEngine::clearVoice() {
  voiceShowing_ = false;
  voiceLevels_.clear();
  voiceFrame_ = 0;
  voiceErrorUntil_ = 0;
  voiceErrorText_.clear();
  if (auto *ic = voiceIc_.get()) {
    // 回到说话前的样子
    updateInputPanel(ic, ic->propertyFor(&factory_));
  }
  voiceIc_.unwatch();
}

// 放弃这次语音：叫停录音，再把候选框里的提示收掉
void FlypeEngine::cancelVoice() {
  if (voice_) {
    flype_voice_cancel(voice_);
  }
  clearVoice();
}

// 单调时钟的毫秒数，给会话判按键的时间用
uint64_t FlypeEngine::nowMs() {
  return static_cast<uint64_t>(fcitx::now(CLOCK_MONOTONIC) / 1000);
}

// fcitx5 用它造插件实例（addon 配置里认这个名字）
class FlypeFactory final : public fcitx::AddonFactory {
public:
  // 造一个 FlypeEngine（没有 addon manager 时返回空）
  fcitx::AddonInstance *create(fcitx::AddonManager *manager) override {
    if (!manager) {
      return nullptr;
    }
    return new FlypeEngine(manager->instance());
  }
};

FCITX_ADDON_FACTORY_V2(flype, FlypeFactory)
