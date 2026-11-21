// 设置契约：把插件的 FlypeConfig（键、类型、默认值、范围、可选值）导出成 JSON，
// scripts/test-config-contract.py 拿去对照 spec/config.json（scripts/addon-test.sh 里跑）。
//
//   flype-config-dump                  各项的说明：{"options": [{"key", "type", "default", "min", "max", "values", "names"}]}
//   flype-config-dump --load K=V ...   把这几行当成 flype.conf 读进来（和插件读文件一样走 loadFlypeConfig），
//                                        导出读到的值：{"values": {"K": "V", ...}}
//   flype-config-dump --load-file 文件    整个文件按 fcitx5 读 flype.conf 的方式读（[分组]、引号转义都照它的规矩），
//                                        再走 loadFlypeConfig，导出读到的值（规格里 readRules.cases 的例子用它）

#include <cstdio>
#include <string>

#include <fcitx-config/iniparser.h>
#include <fcitx-config/rawconfig.h>

#include "flype_engine.h"

namespace {

// 把字符串包成 JSON 字面量：引号、反斜杠、换行制表符和 0x20 以下的字符都转义
std::string jsonString(const std::string &text) {
    std::string out = "\"";
    for (unsigned char c : text) {
        switch (c) {
        case '"': out += "\\\""; break;
        case '\\': out += "\\\\"; break;
        case '\n': out += "\\n"; break;
        case '\r': out += "\\r"; break;
        case '\t': out += "\\t"; break;
        default:
            if (c < 0x20) {
                char buf[8];
                std::snprintf(buf, sizeof(buf), "\\u%04x", c);
                out += buf;
            } else {
                out += static_cast<char>(c);
            }
        }
    }
    return out + "\"";
}

// 可空的值：有值转成 JSON 字符串，没值（比如不存盘的 ExternalOption）输出 null
std::string jsonValue(const std::string *value) { return value ? jsonString(*value) : "null"; }

/// 说明里 path/0、path/1……这样一串（Enum 是存盘名，EnumI18n 是界面上的名字）。
std::string jsonList(const fcitx::RawConfig &item, const std::string &path) {
    std::string out = "[";
    for (int i = 0;; ++i) {
        const std::string *value = item.valueByPath(path + "/" + std::to_string(i));
        if (!value) {
            break;
        }
        if (i > 0) {
            out += ",";
        }
        out += jsonString(*value);
    }
    return out + "]";
}

// 不带参数：把各项说明导出成一条 {"options": [...]}，默认值取插件存盘时的样子
int dumpOptions() {
    FlypeConfig config;
    fcitx::RawConfig description;
    config.dumpDescription(description);
    // 默认值按写进文件的样子取（save 和插件存盘一样）；ExternalOption 不存盘，是 null
    fcitx::RawConfig saved;
    config.save(saved);
    auto root = description.get("FlypeConfig");
    if (!root) {
        std::fprintf(stderr, "配置说明里没有 FlypeConfig\n");
        return 1;
    }
    std::string out = "{\"options\":[";
    bool first = true;
    for (const auto &key : root->subItems()) {
        auto item = root->get(key);
        if (!item) {
            continue;
        }
        if (!first) {
            out += ",";
        }
        first = false;
        out += "{\"key\":" + jsonString(key) + ",\"type\":" + jsonValue(item->valueByPath("Type")) +
               ",\"default\":" + jsonValue(saved.valueByPath(key)) +
               ",\"min\":" + jsonValue(item->valueByPath("IntMin")) +
               ",\"max\":" + jsonValue(item->valueByPath("IntMax")) +
               ",\"values\":" + jsonList(*item, "Enum") + ",\"names\":" + jsonList(*item, "EnumI18n") + "}";
    }
    out += "]}\n";
    std::fputs(out.c_str(), stdout);
    return 0;
}

// 下面两处载入最后都走它，定义在后面
int dumpValues(const fcitx::RawConfig &raw);

// --load：把后面每个 键=值 参数塞进 RawConfig，写错格式返回 2
int dumpLoaded(int argc, char **argv) {
    fcitx::RawConfig raw;
    for (int i = 2; i < argc; ++i) {
        const std::string line = argv[i];
        const auto eq = line.find('=');
        if (eq == std::string::npos) {
            std::fprintf(stderr, "要写成 键=值：%s\n", argv[i]);
            return 2;
        }
        raw.setValueByPath(line.substr(0, eq), line.substr(eq + 1));
    }
    return dumpValues(raw);
}

// --load-file：整个文件按 fcitx5 读 flype.conf 的规矩（分组、引号转义）读进 RawConfig
int dumpLoadedFile(const char *path) {
    FILE *file = std::fopen(path, "r");
    if (!file) {
        std::fprintf(stderr, "打不开 %s\n", path);
        return 2;
    }
    fcitx::RawConfig raw;
    fcitx::readFromIni(raw, file);
    std::fclose(file);
    return dumpValues(raw);
}

// 载入后的值按 JSON 打出来：走插件的 loadFlypeConfig，再存盘一遍取实际生效的值
int dumpValues(const fcitx::RawConfig &raw) {
    FlypeConfig config;
    loadFlypeConfig(config, raw);
    fcitx::RawConfig saved;
    config.save(saved);
    std::string out = "{\"values\":{";
    bool first = true;
    for (const auto &key : saved.subItems()) {
        if (!first) {
            out += ",";
        }
        first = false;
        out += jsonString(key) + ":" + jsonValue(saved.valueByPath(key));
    }
    out += "}}\n";
    std::fputs(out.c_str(), stdout);
    return 0;
}

} // namespace

// 看参数选模式：没有参数导说明，--load / --load-file 导载入结果，其它参数算用错（返回 2）
int main(int argc, char **argv) {
    if (argc > 1 && std::string(argv[1]) == "--load") {
        return dumpLoaded(argc, argv);
    }
    if (argc == 3 && std::string(argv[1]) == "--load-file") {
        return dumpLoadedFile(argv[2]);
    }
    if (argc > 1) {
        std::fprintf(stderr, "用法：%s [--load 键=值 ... | --load-file 文件]\n", argv[0]);
        return 2;
    }
    return dumpOptions();
}
