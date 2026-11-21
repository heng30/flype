// Flype 的日志：同时写 fcitx5 自己的日志和 ~/.local/state/flype/flype.log。
//
// 文件给「Flype 设置 → 导出日志」用，出问题时用户导出来贴到 issue 里。
// 只记启动、载入、出错这类事情，**不记用户打的字、候选词和语音识别结果**。
// 超过 512 KB 就把当前文件改名成 flype.log.1（只留这一份旧的）。
// FLYPE_LOG=文件路径 改位置，FLYPE_LOG=0 不写文件。
#pragma once

#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <filesystem>
#include <mutex>
#include <sstream>
#include <string>
#include <system_error>

#include <fcitx-utils/log.h>

namespace flype {

// 日志文件路径：FLYPE_LOG 优先，其次 $XDG_STATE_HOME 或 ~/.local/state 下的 flype/flype.log，都取不到返回空
inline std::string logFilePath() {
    if (const char *path = std::getenv("FLYPE_LOG"); path && *path) {
        return std::string(path) == "0" ? std::string() : std::string(path);
    }
    std::string base;
    if (const char *state = std::getenv("XDG_STATE_HOME"); state && *state) {
        base = state;
    } else if (const char *home = std::getenv("HOME"); home && *home) {
        base = std::string(home) + "/.local/state";
    } else {
        return {};
    }
    return base + "/flype/flype.log";
}

// 往日志文件追加一行“时间.毫秒 级别 内容”：先建目录，超过 512 KB 就把当前文件改名成 .1
inline void appendLogFile(char level, const std::string &message) {
    static std::mutex mutex;
    static const std::string path = logFilePath();
    if (path.empty()) {
        return;
    }
    constexpr std::uintmax_t kMaxBytes = 512 * 1024;
    std::lock_guard<std::mutex> lock(mutex);
    std::error_code ec;
    const std::filesystem::path file(path);
    std::filesystem::create_directories(file.parent_path(), ec);
    if (std::filesystem::file_size(file, ec) > kMaxBytes && !ec) {
        std::filesystem::rename(file, path + ".1", ec);
    }
    std::FILE *out = std::fopen(path.c_str(), "a");
    if (!out) {
        return;
    }
    const auto now = std::chrono::system_clock::now();
    const std::time_t seconds = std::chrono::system_clock::to_time_t(now);
    const auto millis =
        std::chrono::duration_cast<std::chrono::milliseconds>(now.time_since_epoch()).count() % 1000;
    std::tm local{};
    localtime_r(&seconds, &local);
    char stamp[32];
    std::strftime(stamp, sizeof(stamp), "%Y-%m-%d %H:%M:%S", &local);
    std::fprintf(out, "%s.%03d %c %s\n", stamp, static_cast<int>(millis), level, message.c_str());
    std::fclose(out);
}

/// 一行日志：`WLOG(Info) << "载入 " << n << " 条";`，析构时写出去。
class LogLine {
public:
    LogLine(char level, const char *file, int line) : level_(level), file_(file), line_(line) {}
    LogLine(const LogLine &) = delete;
    LogLine &operator=(const LogLine &) = delete;
    // 析构时把攒好的这一行同时交给 fcitx5 的日志（级别决定严重程度）和日志文件
    ~LogLine() {
        const std::string message = stream_.str();
        switch (level_) {
        case 'E':
            FCITX_LOGC(::fcitx::Log::defaultCategory, Error) << message;
            break;
        case 'W':
            FCITX_LOGC(::fcitx::Log::defaultCategory, Warn) << message;
            break;
        default:
            FCITX_LOGC(::fcitx::Log::defaultCategory, Info) << message;
            break;
        }
        appendLogFile(level_, std::string(std::filesystem::path(file_).filename()) + ":" +
                                  std::to_string(line_) + " " + message);
    }
    // 往这一行里接着拼内容，用完即弃
    template <typename T>
    LogLine &operator<<(const T &value) {
        stream_ << value;
        return *this;
    }

private:
    /// 这一行的级别（I / W / E）
    char level_;
    /// 写日志那处的文件名
    const char *file_;
    /// 写日志那处的行号
    int line_;
    /// 攒起来的这一行内容，析构时一次写出
    std::ostringstream stream_;
};

} // namespace flype

#define WLOG_I() ::flype::LogLine('I', __FILE__, __LINE__) // 记一行 Info，带当前文件名和行号
#define WLOG_W() ::flype::LogLine('W', __FILE__, __LINE__) // 记一行 Warn，fcitx5 那边也当警告
#define WLOG_E() ::flype::LogLine('E', __FILE__, __LINE__) // 记一行 Error，fcitx5 那边也当错误
