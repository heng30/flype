#pragma once

// 数据文件（词库、英文词表……）在用户目录和系统目录里都有时用哪一份。不依赖 fcitx5，按键测试直接测它。

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <optional>
#include <string>
#include <string_view>
#include <system_error>

namespace flype {

// ---------------------------------------------------------------- 数据版本标记
//
// 和 core/src/datamark.rs、settings/flype-config、tools/datamark.py 同一套规则（docs/DATA.md「数据版本标记」）：
// 数据文件前 4096 字节里有一串 `flype-data: <名字> <版本> [来源]`，以换行或 \0 结束。
// 文本文件是头部一行注释；words.lm（WTLM 版本 1）是词表的 1 号词。版本是点分隔的数字，逐段按数字比。

// 标记的前缀，后面接名字、版本、可选来源
inline constexpr std::string_view kDataMarkerKey = "flype-data:";
// 只在文件开头这么多字节里找标记，再往后不算
inline constexpr std::size_t kDataMarkerScanBytes = 4096;

// 从 `flype-data:` 那行解出来的名字、版本、来源（来源可能为空）
struct DataMarker {
    /// 数据文件名（lexicon.tsv、words.lm 等）
    std::string name;
    /// 点分隔的版本号
    std::string version;
    /// 数据来源（发布包名或生成脚本），可能为空
    std::string source;
};

/// 点分隔的数字，每段至少一位（2026.11.14.5）。
inline bool validDataVersion(std::string_view version) {
    if (version.empty() || version.front() == '.' || version.back() == '.') {
        return false;
    }
    char prev = 0;
    for (const char c : version) {
        if (c == '.' ? prev == '.' : (c < '0' || c > '9')) {
            return false;
        }
        prev = c;
    }
    return true;
}

/// 逐段按数字比，缺的段算 0（2026.11.14 = 2026.11.14.0）。每段去掉前导 0 后先比位数再比字面，不会溢出。
/// 返回负数、0、正数。
inline int compareDataVersions(std::string_view a, std::string_view b) {
    while (!a.empty() || !b.empty()) {
        const auto take = [](std::string_view &v) {
            const std::size_t dot = v.find('.');
            std::string_view part = v.substr(0, dot);
            v = dot == std::string_view::npos ? std::string_view{} : v.substr(dot + 1);
            while (!part.empty() && part.front() == '0') {
                part.remove_prefix(1);
            }
            return part;
        };
        const std::string_view x = take(a);
        const std::string_view y = take(b);
        if (x.size() != y.size()) {
            return x.size() < y.size() ? -1 : 1;
        }
        if (const int c = x.compare(y); c != 0) {
            return c < 0 ? -1 : 1;
        }
    }
    return 0;
}

/// 解析 `... flype-data: 名字 版本 [来源]`（到换行或 \0 为止）。
inline std::optional<DataMarker> parseDataMarker(std::string_view text) {
    const std::size_t at = text.find(kDataMarkerKey);
    if (at == std::string_view::npos) {
        return std::nullopt;
    }
    std::string_view rest = text.substr(at + kDataMarkerKey.size());
    rest = rest.substr(0, std::min(rest.find_first_of(std::string_view("\n\r\0", 3)), rest.size()));
    if (rest.empty() || rest.front() != ' ') {
        return std::nullopt;
    }
    const auto skipSpaces = [](std::string_view v) {
        const std::size_t p = v.find_first_not_of(' ');
        return p == std::string_view::npos ? std::string_view{} : v.substr(p);
    };
    const auto token = [](std::string_view &v) {
        const std::size_t sp = v.find(' ');
        std::string_view t = v.substr(0, sp);
        v = sp == std::string_view::npos ? std::string_view{} : v.substr(sp + 1);
        return t;
    };
    rest = skipSpaces(rest);
    const std::string_view name = token(rest);
    rest = skipSpaces(rest);
    const std::string_view version = token(rest);
    std::string_view source = rest;
    while (!source.empty() && (source.front() == ' ' || source.front() == '\t')) {
        source.remove_prefix(1);
    }
    while (!source.empty() && (source.back() == ' ' || source.back() == '\t')) {
        source.remove_suffix(1);
    }
    if (name.empty() || !validDataVersion(version) ||
        !std::all_of(name.begin(), name.end(), [](char c) { return c > 0x20 && c < 0x7f; })) {
        return std::nullopt;
    }
    return DataMarker{std::string(name), std::string(version), std::string(source)};
}

/// 从文件开头的字节里找标记：WTLM 文件看词表 1 号词，别的在这些字节里找 `flype-data:`（要有结尾）。
inline std::optional<DataMarker> scanDataMarker(std::string_view head) {
    head = head.substr(0, std::min(head.size(), kDataMarkerScanBytes));
    if (head.substr(0, 4) == "WTLM") {
        const auto u32At = [&head](std::size_t at) -> std::optional<std::uint32_t> {
            if (at + 4 > head.size()) {
                return std::nullopt;
            }
            const auto *b = reinterpret_cast<const unsigned char *>(head.data() + at);
            return std::uint32_t(b[0]) | std::uint32_t(b[1]) << 8 | std::uint32_t(b[2]) << 16 |
                   std::uint32_t(b[3]) << 24;
        };
        const auto version = u32At(4);
        const auto words = u32At(8);
        if (!version || *version != 1 || !words || *words < 2 || head.size() <= 16) {
            return std::nullopt;
        }
        const std::size_t at = 17 + static_cast<unsigned char>(head[16]);
        if (at >= head.size()) {
            return std::nullopt;
        }
        const std::size_t len = static_cast<unsigned char>(head[at]);
        if (at + 1 + len > head.size()) {
            return std::nullopt;
        }
        const std::string_view word = head.substr(at + 1, len);
        if (word.substr(0, kDataMarkerKey.size()) != kDataMarkerKey) {
            return std::nullopt;
        }
        return parseDataMarker(word);
    }
    const std::size_t at = head.find(kDataMarkerKey);
    if (at == std::string_view::npos ||
        head.find_first_of(std::string_view("\n\r\0", 3), at) == std::string_view::npos) {
        return std::nullopt;
    }
    return parseDataMarker(head.substr(at));
}

/// 读文件开头找标记；读不了、没有标记返回空。
inline std::optional<DataMarker> readDataMarker(const std::filesystem::path &file) {
    std::ifstream in(file, std::ios::binary);
    if (!in) {
        return std::nullopt;
    }
    std::string head(kDataMarkerScanBytes, '\0');
    in.read(head.data(), static_cast<std::streamsize>(head.size()));
    head.resize(static_cast<std::size_t>(in.gcount()));
    return scanDataMarker(head);
}

// ---------------------------------------------------------------- 没有标记的老数据（启发式）

/// 安装包自带、Flype 自己生成的数据文件的第一行。用户目录里的同名文件第一行是这个，
/// 说明是标准数据的一份拷贝，不是用户自己做的。
inline std::string_view bundledDataMarker(std::string_view fileName) {
    if (fileName == "lexicon.tsv") {
        return "# 由 tools/rime2tsv.py 生成"; // scripts/make-data.sh 生成的雾凇词库
    }
    if (fileName == "english.txt") {
        return "# Rime dictionary"; // 雾凇的英文词表原样拼接
    }
    return {};
}

/// 加版本标记以前发布的官方 words.lm 的字节数，和 kOfficialLmVersions 一一对应。
/// 以后发布的 words.lm 词表里带标记（core/src/wordlm.rs 的 set_data_marker），不用再往这里加；
/// 这张表只为认出老用户目录里拷进来的老模型。
inline constexpr std::uintmax_t kOfficialLmSizes[] = {
    38455852, // lm-2026.11.09
};
// 和 kOfficialLmSizes 逐个对应的版本号
inline constexpr std::string_view kOfficialLmVersions[] = {
    "2026.11.09",
};
static_assert(std::size(kOfficialLmSizes) == std::size(kOfficialLmVersions));

/// 没有标记、字节数是某一版官方发布的 words.lm：那一版的版本，不是返回空。
inline std::string_view officialLmVersion(const std::filesystem::path &file) {
    std::error_code ec;
    const std::uintmax_t size = std::filesystem::file_size(file, ec);
    if (ec) {
        return {};
    }
    for (std::size_t i = 0; i < std::size(kOfficialLmSizes); ++i) {
        if (size == kOfficialLmSizes[i]) {
            return kOfficialLmVersions[i];
        }
    }
    return {};
}

// 是不是某一版官方发布、但没有版本标记的 words.lm（只按字节数认）
inline bool isOfficialLm(const std::filesystem::path &file) { return !officialLmVersion(file).empty(); }

/// 数据文件是哪一版：带标记（名字要和 fileName 的文件名一样）就是标记里的版本；
/// 没有标记的 words.lm 字节数是某一版官方发布的就是那一版；都不是返回空。
inline std::string dataFileVersion(const std::filesystem::path &file, std::string_view fileName) {
    const std::string base = std::filesystem::path(fileName).filename().string();
    if (const auto marker = readDataMarker(file)) {
        return marker->name == base ? marker->version : std::string{};
    }
    if (base == "words.lm") {
        return std::string(officialLmVersion(file));
    }
    return {};
}

/// 用户目录里的这份是不是安装包自带数据的旧副本，该让给系统目录里安装包带的那份。
///
/// 0.3.4 以前词库不在安装包里，README 让大家把 lexicon.tsv、english.txt、words.lm 拷进
/// ~/.local/share/flype/（源码安装的说明一直到 0.4.0 都这样写）；
/// 后来安装包自带了，用户目录那份却照样优先，装了新版也一直用旧词库。
///
/// 按版本标记（见上面）判断，系统目录里也要有一份：
/// - 用户目录这份带标记、名字和文件名一样：版本 ≤ 系统目录那份的版本就是旧副本；比它新（自己下了新数据），
///   或者系统目录那份没有版本（老安装包），用用户目录的。
/// - 带标记但名字不一样（万象词库包这类别的数据）：用户自己放的，用用户目录的。
/// - 没有标记（加标记以前发布的数据、或者用户自己做的）：按老办法认——第一行是 Flype 生成的标准数据
///   （bundledDataMarker），而且不比系统目录的新（系统目录的修改时间早于 2000 年时看不出新旧，也算）；
///   words.lm 看字节数是不是某一版官方发布的（kOfficialLmSizes），不看新旧。
/// 用户自己做的词库、自己训的 words.lm 照旧优先；想指定词库的正路是设置里的 Lexicon 或环境变量 FLYPE_LEXICON。
inline bool isStaleBundledCopy(const std::filesystem::path &user, const std::filesystem::path &system,
                               std::string_view fileName) {
    std::error_code ec;
    if (!std::filesystem::is_regular_file(system, ec) || std::filesystem::equivalent(user, system, ec)) {
        return false;
    }
    if (const auto marker = readDataMarker(user)) {
        if (marker->name != std::filesystem::path(fileName).filename().string()) {
            return false;
        }
        const std::string systemVersion = dataFileVersion(system, fileName);
        return !systemVersion.empty() && compareDataVersions(marker->version, systemVersion) <= 0;
    }
    if (fileName == "words.lm") {
        return isOfficialLm(user);
    }
    const std::string_view marker = bundledDataMarker(fileName);
    if (marker.empty()) {
        return false;
    }
    std::ifstream in(user, std::ios::binary);
    std::string first;
    if (!in || !std::getline(in, first) || first.rfind(marker, 0) != 0) {
        return false;
    }
    std::error_code userEc;
    std::error_code systemEc;
    const auto userTime = std::filesystem::last_write_time(user, userEc);
    const auto systemTime = std::filesystem::last_write_time(system, systemEc);
    if (userEc || systemEc) {
        return true;
    }
    const auto toSystemClock = [](std::filesystem::file_time_type t) {
        return std::chrono::time_point_cast<std::chrono::seconds>(
            t - std::filesystem::file_time_type::clock::now() + std::chrono::system_clock::now());
    };
    // 2000-01-01T00:00:00Z
    const auto sane = std::chrono::system_clock::time_point(std::chrono::seconds(946684800));
    if (toSystemClock(systemTime) < sane) {
        return true;
    }
    return userTime <= systemTime;
}

} // namespace flype
