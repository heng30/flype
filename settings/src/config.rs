//! 读写 fcitx5 的 flype.conf 和自定义词表 custom.tsv。
//!
//! 键名、默认值、可选值和插件（fcitx5-addon/src/flype_engine.h 的 FlypeConfig）一致。

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

/// 绿色主题用的两套主题名，和 themes/fcitx5/ 下的目录名一样。
const THEME_LIGHT: &str = "flype";
const THEME_DARK: &str = "flype-dark";

/// 候选字号的范围和 fcitx5 经典界面的默认值。
pub const FONT_SIZE_RANGE: (i32, i32) = (8, 32);
const FONT_SIZE_DEFAULT: i32 = 10;

/// 设置项和默认值（顺序就是界面上的顺序）。整数项的范围见 [`IntRange`]。
pub const DEFAULTS: &[(&str, &str)] = &[
    ("CandidateLayout", "Horizontal"),
    ("PageSize", "9"),
    ("MaxCandidates", "100"),
    ("PageKeysMinusEqual", "True"),
    ("PageKeysBracket", "False"),
    ("PageKeysCommaPeriod", "False"),
    ("PageKeysTab", "False"),
    ("ArrowKeys", "Candidates"),
    ("SelectKeys", "Off"),
    ("PickChar", "True"),
    ("Association", "False"),
    ("TypoCorrection", "True"),
    ("RareExtChars", "False"),
    ("Fuzzy", "False"),
    ("FuzzyZh", "True"),
    ("FuzzyCh", "True"),
    ("FuzzySh", "True"),
    ("FuzzyNL", "False"),
    ("FuzzyFH", "False"),
    ("FuzzyRL", "False"),
    ("FuzzyAn", "True"),
    ("FuzzyEn", "True"),
    ("FuzzyIn", "True"),
    ("FuzzyIan", "True"),
    ("FuzzyUan", "True"),
    ("FuzzyUn", "False"),
    ("FuzzyAnAi", "False"),
    ("FuzzyHuangWang", "False"),
    ("FuzzyUnIong", "False"),
    ("FuzzyEngOng", "False"),
    ("ShiftToggle", "Both"),
    ("InitialMode", "Chinese"),
    ("ToggleKey", ""),
    ("PunctuationStyle", "Fullwidth"),
    ("AutoSpace", "False"),
    ("AutoSpaceLetters", "True"),
    ("AutoSpaceDigits", "True"),
    ("EmojiSuggest", "True"),
    ("EmojiKey", "Super+period"),
    ("DateTime", "True"),
    ("Calculator", "True"),
    ("Toolbar", "True"),
    ("ClipboardHistory", "False"),
    ("Voice", "True"),
    ("VoiceKey", "Alt_R"),
    ("VoiceCopy", "False"),
    ("Lexicon", ""),
];

/// 整数项的范围（和插件里的 IntConstrain 一样，读进来超出就夹到边上）。
pub fn int_range(key: &str) -> (i32, i32) {
    match key {
        "PageSize" => (3, 9),
        "MaxCandidates" => (10, 1000),
        _ => (0, i32::MAX),
    }
}

/// 设置项的默认值；不认识的项是空串。
pub fn default_of(key: &str) -> &'static str {
    DEFAULTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, v)| *v)
}

/// 配置文件里的内容：认识的和默认值不一样的项，加上不认识的行（原样留着，下次保存还写回去）。
#[derive(Clone, Default)]
pub struct Config {
    /// 认识的项里和默认值不一样的那些（等于默认值的已经删掉）。
    values: BTreeMap<String, String>,
    /// 不认识的行，原样留着，保存时照抄回去。
    extra: Vec<String>,
}

impl Config {
    /// 读配置文件：不认识的键原样留进 extra，认不出的值换成默认值并记一条提示；文件不存在就是空的。
    pub fn load(path: &Path) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let Ok(text) = fs::read_to_string(path) else {
            return (Self::default(), warnings);
        };
        let mut config = Self::default();
        for line in text.lines() {
            let line = line.trim_end();
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
                config.extra.push(line.to_owned());
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                config.extra.push(line.to_owned());
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if DEFAULTS.iter().any(|(k, _)| *k == key) {
                config.set(key, value);
            } else {
                config.extra.push(line.to_owned());
            }
        }
        for (key, _) in DEFAULTS {
            if let Some(value) = config.values.get(*key) {
                if let Some(fixed) = normalize(key, value) {
                    warnings.push(format!("{key}={value} 认不出来，用默认值 {fixed}"));
                    config.set(key, &fixed);
                }
            }
        }
        (config, warnings)
    }

    /// 只写和默认值不一样的项：没写的项插件读成默认值（以后改默认值时老配置也跟着走）。
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut lines = Vec::new();
        for (key, default) in DEFAULTS {
            if let Some(value) = self.values.get(*key) {
                if value != default {
                    lines.push(format!("{key}={value}"));
                }
            }
        }
        lines.extend(self.extra.iter().cloned());
        let text = if lines.is_empty() {
            String::new()
        } else {
            lines.join("\n") + "\n"
        };
        write_atomic(path, text.as_bytes())
    }

    /// 取设置项的值；没写过的项是默认值（插件也是这么读的）。
    pub fn text(&self, key: &str) -> &str {
        self.values
            .get(key)
            .map_or_else(|| default_of(key), String::as_str)
    }

    /// 布尔项：值正好是 True 才算开（插件也这么认，别的写法当关）。
    pub fn flag(&self, key: &str) -> bool {
        self.text(key) == "True"
    }

    /// 整数项：读不出来用默认值，再按 [`int_range`] 夹到边上。
    pub fn int(&self, key: &str) -> i32 {
        let (min, max) = int_range(key);
        let fallback = default_of(key).parse::<i32>().unwrap_or(min);
        self.text(key)
            .parse::<i32>()
            .unwrap_or(fallback)
            .clamp(min, max)
    }

    /// 和默认值一样就从文件里删掉（顺手保持「只写改过的项」）。
    pub fn set(&mut self, key: &str, value: &str) {
        let value = value.trim();
        if value == default_of(key) {
            self.values.remove(key);
        } else {
            self.values.insert(key.to_owned(), value.to_owned());
        }
    }

    /// 按布尔写一项（True / False），值等于默认值时 set 顺手删掉。
    pub fn set_flag(&mut self, key: &str, on: bool) {
        self.set(key, if on { "True" } else { "False" });
    }
}

/// 认不出来的值换成默认值，返回要提示的话。
fn normalize(key: &str, value: &str) -> Option<&'static str> {
    let default = default_of(key);
    // 布尔项不检查：插件自己按「是不是 True」认，和这里看到的一样
    if key == "PageSize" || key == "MaxCandidates" {
        let (min, max) = int_range(key);
        return match value.parse::<i32>() {
            Ok(n) if (min..=max).contains(&n) => None,
            _ => Some(default),
        };
    }
    match key {
        "CandidateLayout" => known(value, &["Horizontal", "Vertical"], default),
        "ArrowKeys" => known(value, &["Candidates", "Caret"], default),
        "SelectKeys" => known(value, &["Off", "Semicolon", "SemicolonQuote"], default),
        "ShiftToggle" => known(value, &["Both", "Left", "Right", "Off"], default),
        "InitialMode" => known(value, &["Chinese", "English", "Last"], default),
        "PunctuationStyle" => known(value, &["Fullwidth", "Halfwidth"], default),
        _ => None,
    }
}

/// 值不在 choices 里就返回要提示的默认值，认得返回 None。
fn known<'a>(value: &str, choices: &[&str], default: &'a str) -> Option<&'a str> {
    (!choices.contains(&value)).then_some(default)
}

/// 先写同目录的 .tmp 再改名，写到一半崩了也不会留下半截配置。
fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)
}

/// custom.tsv：一行 `编码<TAB>词[<TAB>频次]`，词里的换行、制表符用 `\n`、`\t` 转义。
#[derive(Clone, Default)]
pub struct CustomWords {
    /// 每行是（编码，词），转义已经拆开，保存时再转回去。
    pub rows: Vec<(String, String)>,
}

impl CustomWords {
    /// 读 custom.tsv：跳过空行和 # 开头的行，只取编码和词两列（频次列丢掉），词里的转义拆开。
    pub fn load(path: &Path) -> Self {
        let mut rows = Vec::new();
        if let Ok(text) = fs::read_to_string(path) {
            for line in text.lines() {
                let line = line.trim_end_matches('\r');
                if line.trim().is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut cols = line.split('\t');
                let (Some(code), Some(word)) = (cols.next(), cols.next()) else {
                    continue;
                };
                if word.trim().is_empty() {
                    continue;
                }
                rows.push((code.trim().to_owned(), unescape(word)));
            }
        }
        Self { rows }
    }

    /// 写回 custom.tsv：编码或词空着的行跳过，词的换行、制表符转义（先建好目录）。
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut lines = Vec::new();
        for (code, word) in &self.rows {
            let (code, word) = (code.trim(), word.trim());
            if code.is_empty() || word.is_empty() {
                continue;
            }
            lines.push(format!("{code}\t{}", escape(word)));
        }
        let text = if lines.is_empty() {
            String::new()
        } else {
            lines.join("\n") + "\n"
        };
        write_atomic(path, text.as_bytes())
    }
}

/// 反斜杠、换行、制表符写成 \\、\n、\t，别的字符原样。
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// 拆开 [`escape`] 写的转义；不认识的转义去掉反斜杠、留着字符本身。
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// 外观：候选框有没有用 Flype 绿色主题、候选字号多少磅。
///
/// 只动 Theme、DarkTheme、UseDarkTheme、Font 这几行，别的项原样写回。
#[derive(Clone, Default, PartialEq)]
pub struct ClassicUi {
    /// 文件里原来的行。
    lines: Vec<String>,
    /// 绿色主题开着没有。
    pub theme: bool,
    /// 绿色主题用深色版（否则亮色版）。
    pub dark: bool,
    /// Font 那行原来的内容（带不带引号都行）。
    font: String,
    /// 候选字号（磅）。
    pub font_size: i32,
}

impl ClassicUi {
    /// 读 classicui.conf；文件不在就当空的（fcitx5 自己会补默认值）。
    pub fn load(path: &Path) -> Self {
        let lines: Vec<String> = fs::read_to_string(path)
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        let font = value_of(&lines, "Font").unwrap_or_else(|| format!("Sans {FONT_SIZE_DEFAULT}"));
        let font_size = font_size_of(&font)
            .unwrap_or(FONT_SIZE_DEFAULT)
            .clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1);
        Self {
            theme: value_of(&lines, "Theme").is_some_and(|name| is_flype_theme(&name)),
            dark: value_of(&lines, "Theme").as_deref() == Some(THEME_DARK),
            font,
            font_size,
            lines,
        }
    }

    /// 写回 classicui.conf：主题两行跟着开关走，字号换掉 Font 里的数字。
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut lines = self.lines.clone();
        // 关着的时候只在原来是 flype 主题时才退回默认，用户自己的第三方主题不动
        let was = value_of(&lines, "Theme").is_some_and(|name| is_flype_theme(&name));
        if self.theme {
            let name = if self.dark { THEME_DARK } else { THEME_LIGHT };
            set_key(&mut lines, "Theme", name);
            set_key(&mut lines, "DarkTheme", name);
            set_key(&mut lines, "UseDarkTheme", "False");
        } else if was {
            set_key(&mut lines, "Theme", "default");
            set_key(&mut lines, "DarkTheme", "default-dark");
        }
        let font = font_with_size(&self.font, self.font_size);
        set_key(&mut lines, "Font", &format!("\"{font}\""));
        write_atomic(path, (lines.join("\n") + "\n").as_bytes())
    }
}

/// 是不是 Flype 的两套主题之一。
fn is_flype_theme(name: &str) -> bool {
    name == THEME_LIGHT || name == THEME_DARK
}

/// `key=值` 那行的值，没有就是 None。
fn value_of(lines: &[String], key: &str) -> Option<String> {
    lines.iter().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| unquote(v.trim()))
    })
}

/// 去掉两边的引号。
fn unquote(text: &str) -> String {
    text.trim_matches('"').to_owned()
}

/// 「Sans Serif 18」里的字号；末尾没写数字就是 None。
fn font_size_of(font: &str) -> Option<i32> {
    font.split_whitespace()
        .last()?
        .parse::<f32>()
        .ok()
        .map(|size| size.round() as i32)
}

/// 换掉字号，字体名和粗细不动：「Sans Serif 18」+ 22 →「Sans Serif 22」。
fn font_with_size(font: &str, size: i32) -> String {
    let mut parts: Vec<&str> = font.split_whitespace().collect();
    if parts.last().is_some_and(|last| last.parse::<f32>().is_ok()) {
        parts.pop();
    }
    if parts.is_empty() {
        parts.push("Sans");
    }
    format!("{} {size}", parts.join(" "))
}

/// 把 `key=值` 那行换成新的；原来没有就加在最后。
fn set_key(lines: &mut Vec<String>, key: &str, value: &str) {
    let line = format!("{key}={value}");
    let found = lines
        .iter_mut()
        .find(|old| old.split_once('=').is_some_and(|(k, _)| k.trim() == key));
    match found {
        Some(old) => *old = line,
        None => lines.push(line),
    }
}

/// 设置界面自己的状态放这里，不和 fcitx5 的配置混在一起。默认在 ~/.config/flype/ 下
pub fn ui_path() -> PathBuf {
    std::env::var_os("FLYPE_UI_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| config_dir().join("flype/ui.conf"))
}

/// 让 fcitx5 重新读 flype 和经典界面的配置（改完马上生效）。没在跑时返回 false。
pub fn reload_fcitx5() -> bool {
    // 外观（主题、字号）是 classicui 那个插件管的，单 reload flype 不会生效
    let app = reload_addon("flype");
    let ui = reload_addon("classicui");
    app && ui
}

/// 让 fcitx5 重读某一个插件的配置。
fn reload_addon(addon: &str) -> bool {
    let calls: &[(&str, &[&str])] = &[
        (
            "gdbus",
            &[
                "call",
                "--session",
                "--dest",
                "org.fcitx.Fcitx5",
                "--object-path",
                "/controller",
                "--method",
                "org.fcitx.Fcitx.Controller1.ReloadAddonConfig",
                addon,
            ],
        ),
        (
            "busctl",
            &[
                "--user",
                "call",
                "org.fcitx.Fcitx5",
                "/controller",
                "org.fcitx.Fcitx.Controller1",
                "ReloadAddonConfig",
                "s",
                "flype",
            ],
        ),
    ];

    for (program, args) in calls {
        let Ok(output) = std::process::Command::new(program).args(*args).output() else {
            continue;
        };
        return output.status.success();
    }
    false
}

/// 配置目录：$XDG_CONFIG_HOME，没有就是 ~/.config，再没有就是当前目录。
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 数据目录：$XDG_DATA_HOME，没有就是 ~/.local/share，再没有就是当前目录。
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// flype.conf 的路径：环境变量 FLYPE_CONFIG 可以改，默认在 ~/.config/fcitx5/conf 下。
pub fn config_path() -> PathBuf {
    std::env::var_os("FLYPE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| config_dir().join("fcitx5/conf/flype.conf"))
}

/// fcitx5 经典界面的配置：候选框主题和候选字号在这儿，和 flype.conf 分开。
pub fn classicui_path() -> PathBuf {
    config_dir().join("fcitx5/conf/classicui.conf")
}

/// custom.tsv 的路径：环境变量 FLYPE_CUSTOM 可以改，默认在数据目录的 ~/.local/share/flype/ 下。
pub fn custom_path() -> PathBuf {
    std::env::var_os("FLYPE_CUSTOM")
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir().join("flype/custom.tsv"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 外观：主题开关和字号写回去，别的行原样留着。
    #[test]
    fn classic_ui_round_trip() {
        let dir = std::env::temp_dir().join(format!("flype-classicui-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("classicui.conf");
        fs::write(
            &path,
            "# 注释\nMenuFont=\"Sans Serif 16\"\nFont=\"Sans Serif 18\"\nTheme=wowtype\n",
        )
        .unwrap();

        let mut ui = ClassicUi::load(&path);
        assert!(!ui.theme);
        assert_eq!(ui.font_size, 18);
        ui.theme = true;
        ui.font_size = 22;
        ui.save(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("MenuFont=\"Sans Serif 16\""),
            "别的项要原样留着：{text}"
        );
        assert!(text.contains("Theme=flype\n"));
        assert!(text.contains("DarkTheme=flype\n"));
        assert!(text.contains("UseDarkTheme=False"));
        assert!(text.contains("Font=\"Sans Serif 22\""));

        let back = ClassicUi::load(&path);
        assert!(back.theme);
        assert_eq!(back.font_size, 22);

        // 自己在用 flype-dark 时保存，不能被改回默认主题
        fs::write(
            &path,
            "Font=\"Sans Serif 18\"\nTheme=flype-dark\nDarkTheme=flype-dark\n",
        )
        .unwrap();
        let mut ui = ClassicUi::load(&path);
        assert!(ui.theme);
        assert!(ui.dark);
        ui.font_size = 20;
        ui.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Theme=flype-dark"), "主题被改写了：{text}");
        assert!(
            text.contains("DarkTheme=flype-dark"),
            "深色主题被改写了：{text}"
        );

        // 切到亮色：两行都换成 flype
        ui.dark = false;
        ui.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Theme=flype\n"), "{text}");
        assert!(text.contains("DarkTheme=flype\n"), "{text}");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 不是 Flype 主题时，不动主题那几行。
    #[test]
    fn foreign_theme_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("flype-theme-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("classicui.conf");
        fs::write(&path, "Theme=wowtype\nDarkTheme=wowtype-dark\n").unwrap();
        let ui = ClassicUi::load(&path);
        assert!(!ui.theme);
        ui.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "Theme=wowtype\nDarkTheme=wowtype-dark\nFont=\"Sans 10\"\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 只写改过的项和认不出的行，读回来一样。
    #[test]
    fn only_changed_values_are_written() {
        let dir = std::env::temp_dir().join(format!("flype-config-test-{}", std::process::id()));
        let path = dir.join("flype.conf");
        let mut config = Config::default();
        assert_eq!(config.text("PageSize"), "9");
        config.set("PageSize", "8");
        config.set_flag("EmojiSuggest", false);
        config.set_flag("Fuzzy", false);
        config.extra.push("SomethingElse=1".to_owned());
        config.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text, "PageSize=8\nEmojiSuggest=False\nSomethingElse=1\n");
        let (loaded, warnings) = Config::load(&path);
        assert!(warnings.is_empty());
        assert_eq!(loaded.int("PageSize"), 8);
        assert!(!loaded.flag("EmojiSuggest"));
        assert!(!loaded.flag("Fuzzy"));
        assert_eq!(loaded.extra, ["SomethingElse=1"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 超出范围、不认识的值回默认值并出提示，布尔项原样留着（插件自己按 True 认）。
    #[test]
    fn bad_values_fall_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("flype-config-bad-{}", std::process::id()));
        let path = dir.join("flype.conf");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            "PageSize=12\nCandidateLayout=Sideways\nEmojiSuggest=yes\n",
        )
        .unwrap();
        let (config, warnings) = Config::load(&path);
        assert_eq!(config.int("PageSize"), 9, "超出范围夹回默认值");
        assert_eq!(config.text("CandidateLayout"), "Horizontal");
        assert_eq!(
            config.text("EmojiSuggest"),
            "yes",
            "布尔项插件自己按 True 认"
        );
        assert_eq!(warnings.len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 词里的换行转义后存盘，读回来还是原来的词。
    #[test]
    fn custom_words_round_trip() {
        let dir = std::env::temp_dir().join(format!("flype-config-words-{}", std::process::id()));
        let path = dir.join("custom.tsv");
        let words = CustomWords {
            rows: vec![
                ("nihao".to_owned(), "你好\n世界".to_owned()),
                ("mail".into(), "a@b.c".into()),
            ],
        };
        words.save(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "nihao\t你好\\n世界\nmail\ta@b.c\n"
        );
        assert_eq!(CustomWords::load(&path).rows, words.rows);
        fs::remove_dir_all(&dir).unwrap();
    }
}
