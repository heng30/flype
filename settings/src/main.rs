//! Flype 设置：改 ~/.config/fcitx5/conf/flype.conf 和自定义词，保存后让 fcitx5 重新读。
//!
//! 左边是导航，右边是卡片式的设置项（外观见 [`ui`]）。只跑 Wayland（eframe 的 wayland 后端）。
//! 用法：`flype-config`，fcitx5 的配置工具里「自定义词和更多设置」也打开它。

mod config;
mod ui;

use config::{ClassicUi, Config, CustomWords};
use eframe::egui::{self, Margin, RichText};
use flype_voice::wtlocal::download;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use ui::Icon;

/// 界面字体：思源黑体简体，编译进程序，不依赖系统装了什么字体。
const UI_FONT: &[u8] = include_bytes!("../font/SourceHanSansCN.otf");

/// 左边导航的页面。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    /// 输入法、候选框、帮助。
    General,
    /// 输入方案、中英文、标点空格。
    Input,
    /// 语音输入和识别模型。
    Voice,
    /// emoji 推荐、日期时间、计算器、工具栏、词库。
    Extra,
    /// 自定义词表。
    Words,
}

/// 导航项：页面、图标、文字，顺序就是侧栏里的顺序。
const PAGES: &[(Page, Icon, &str)] = &[
    (Page::General, Icon::General, "常规"),
    (Page::Input, Icon::Input, "输入"),
    (Page::Voice, Icon::Voice, "语音"),
    (Page::Extra, Icon::Emoji, "增强"),
    (Page::Words, Icon::Book, "自定义词"),
];

/// 模糊音开关：设置键和界面上的文字。
const FUZZY_RULES: &[(&str, &str)] = &[
    ("FuzzyZh", "z = zh"),
    ("FuzzyCh", "c = ch"),
    ("FuzzySh", "s = sh"),
    ("FuzzyNL", "n = l"),
    ("FuzzyFH", "f = h"),
    ("FuzzyRL", "r = l"),
    ("FuzzyAn", "an = ang"),
    ("FuzzyEn", "en = eng"),
    ("FuzzyIn", "in = ing"),
    ("FuzzyIan", "ian = iang"),
    ("FuzzyUan", "uan = uang"),
    ("FuzzyUn", "un = ong"),
    ("FuzzyAnAi", "an = ai"),
    ("FuzzyHuangWang", "huang = wang"),
    ("FuzzyUnIong", "un = iong"),
    ("FuzzyEngOng", "eng = ong"),
];

/// 快捷键预设：fcitx5 键名和界面上的叫法。
///
/// 只放系统没占的键：Ctrl+空格 被 fcitx5 拿去切输入法、Super+空格 被桌面拿走，
/// 设了也收不到，所以切换键给的是 Control+Shift+space。
const VOICE_KEY_PRESETS: &[(&str, &str)] = &[
    ("Alt_R", "右 Alt（默认）"),
    ("Control_R", "右 Ctrl"),
    ("F8", "F8"),
    ("F9", "F9"),
    ("F10", "F10"),
];

const EMOJI_KEY_PRESETS: &[(&str, &str)] = &[
    ("Super+period", "Super + 句号（默认）"),
    ("Control+semicolon", "Ctrl + 分号"),
    ("Control+Shift+E", "Ctrl + Shift + E"),
    ("", "不用快捷键"),
];

const TOGGLE_KEY_PRESETS: &[(&str, &str)] = &[
    ("", "不用（单按 Shift 照常）"),
    ("Control+Shift+space", "Ctrl + Shift + 空格"),
];

/// 读配置和自定义词，开 eframe 窗口跑设置界面（只跑 Wayland）。
fn main() -> eframe::Result {
    let config_path = config::config_path();
    let custom_path = config::custom_path();
    let ui_path = config::ui_path();
    let scale = load_scale(&ui_path).unwrap_or(1.0);
    let (config, warnings) = Config::load(&config_path);
    let classic_path = config::classicui_path();
    let classic = ClassicUi::load(&classic_path);
    let words = CustomWords::load(&custom_path);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Flype 设置")
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([880.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "flype-config",
        options,
        Box::new(move |cc| {
            install_font(&cc.egui_ctx);
            ui::apply(&cc.egui_ctx);
            let brand = ui::brand_logo(&cc.egui_ctx);
            cc.egui_ctx
                .options_mut(|options| options.zoom_with_keyboard = true);

            Ok(Box::new(SettingsApp {
                saved: config.clone(),
                config,
                words,
                classic,
                classic_path,
                config_path,
                custom_path,
                warnings,
                status: String::new(),
                model: ModelState::default(),
                ui_path,
                saved_scale: scale,
                scale: 0.0,
                started: false,
                page: Page::General,
                brand,
                page_keys_open: false,
                fuzzy_open: false,
                tips_open: false,
            }))
        }),
    )
}

/// 把内嵌的思源黑体放在字体列表最前面，中文和拉丁字母都用它。
fn install_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "ui".to_owned(),
        Arc::new(egui::FontData::from_static(UI_FONT)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "ui".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// 读记住的缩放级别；文件不在或内容坏了就是 None。
fn load_scale(path: &Path) -> Option<f32> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("scale="))
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|scale| *scale > 0.0)
}

/// 把缩放级别写回文件，写不进去就算了（下次自己重新取）。
fn save_scale(path: &Path, scale: f32) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    _ = std::fs::write(path, format!("scale={scale}\n"));
}

/// 语音模型的下载状态（后台线程跑 flype_voice 的下载、校验、解包）。
#[derive(Default)]
struct ModelState {
    /// 模型文件齐了没有（每次画之前重查一遍）。
    ready: bool,
    /// 要显示的那句话：正在下载、好了、还是失败。
    message: String,
    /// 后台下载线程共用的结果槽，None 表示没在下载。
    job: Option<Arc<Mutex<Option<String>>>>,
}

/// 界面的状态：配置、自定义词、两个文件路径、提示、语音模型下载进度和展开着的几组设置。
struct SettingsApp {
    /// 文件里现在的内容（「保存」时和它不一样才有改过的提示）。
    saved: Config,
    /// 界面上正在改的内容，点保存才写回文件。
    config: Config,
    /// 自定义词表，和配置一起读写。
    words: CustomWords,
    /// 外观（fcitx5 经典界面的主题和候选字号），和 flype.conf 分开存。
    classic: ClassicUi,
    /// classicui.conf 的路径。
    classic_path: PathBuf,
    /// flype.conf 的路径。
    config_path: PathBuf,
    /// custom.tsv 的路径。
    custom_path: PathBuf,
    /// 读配置时认不出来的值，画在页面顶上提醒。
    warnings: Vec<String>,
    /// 底部状态栏那句话，空串就是不显示。
    status: String,
    /// 语音模型的下载状态。
    model: ModelState,
    /// 第一帧已经把缩放设下去了。
    started: bool,
    /// 左边导航选中的页面。
    page: Page,
    /// 侧栏顶上的 logo 贴图。
    brand: egui::TextureHandle,
    /// 「翻页键」那组是否展开。
    page_keys_open: bool,
    /// 「模糊音规则」那组是否展开。
    fuzzy_open: bool,
    /// 「小技巧」那组是否展开。
    tips_open: bool,
    /// ui.conf 的路径，缩放级别存在这儿。
    ui_path: PathBuf,
    /// 上次文件里记下的缩放级别（相对显示器缩放），0 表示没有。
    saved_scale: f32,
    /// 这次实际在用的缩放级别，0 表示还没定。
    scale: f32,
}

impl SettingsApp {
    /// 写配置和自定义词，成功后让 fcitx5 重读；写失败就拼成一句提示放在状态栏。
    fn save(&mut self) {
        let mut problems = Vec::new();
        if let Err(error) = self.config.save(&self.config_path) {
            problems.push(format!("写 {} 失败：{error}", self.config_path.display()));
        }
        if let Err(error) = self.words.save(&self.custom_path) {
            problems.push(format!("写 {} 失败：{error}", self.custom_path.display()));
        }
        if let Err(error) = self.classic.save(&self.classic_path) {
            problems.push(format!("写 {} 失败：{error}", self.classic_path.display()));
        }
        self.status = if problems.is_empty() {
            self.saved = self.config.clone();
            self.classic = ClassicUi::load(&self.classic_path);
            self.warnings.clear();
            if config::reload_fcitx5() {
                "已保存，fcitx5 已重新读取".to_owned()
            } else {
                "已保存。fcitx5 没在跑，下次启动生效".to_owned()
            }
        } else {
            problems.join("；")
        };
    }

    /// 从文件重新读配置和自定义词，丢掉还没保存的改动。
    fn reload(&mut self) {
        let (config, warnings) = Config::load(&self.config_path);
        self.saved = config.clone();
        self.config = config;
        self.words = CustomWords::load(&self.custom_path);
        self.classic = ClassicUi::load(&self.classic_path);
        self.warnings = warnings;
        self.status = "已重新读取".to_owned();
    }

    /// 和文件里的内容比，有没有改过（保存按钮的字跟着变）；自定义词和磁盘上的比。
    fn changed(&self) -> bool {
        self.config.text("PageSize") != self.saved.text("PageSize")
            || config::DEFAULTS
                .iter()
                .any(|(key, _)| self.config.text(key) != self.saved.text(key))
            || self.words.rows != CustomWords::load(&self.custom_path).rows
            || self.classic != ClassicUi::load(&self.classic_path)
    }

    /// 查语音模型在不在；后台下载线程有结果就收下，还在跑就 300ms 后再画。
    fn poll_model(&mut self, ctx: &egui::Context) {
        let dir = download::default_dir();
        self.model.ready = download::is_ready(&dir);
        let Some(job) = self.model.job.clone() else {
            return;
        };

        let done = job.lock().ok().and_then(|slot| slot.clone());
        match done {
            Some(message) => {
                self.model.message = message;
                self.model.job = None;
                self.model.ready = download::is_ready(&dir);
            }
            None => ctx.request_repaint_after(Duration::from_millis(300)),
        }
    }

    /// 开后台线程下载语音模型，结果放进 job 里的槽，界面靠 poll_model 收。
    fn start_model_download(&mut self) {
        let slot = Arc::new(Mutex::new(None));
        let write = slot.clone();

        self.model.message = "正在下载（约 100MB）…".to_owned();
        self.model.job = Some(slot);

        std::thread::spawn(move || {
            let dir = download::default_dir();
            let message = match download::ensure(&dir) {
                Ok(()) => "语音模型好了".to_owned(),
                Err(error) => format!("下载失败：{error}"),
            };
            if let Ok(mut slot) = write.lock() {
                *slot = Some(message);
            }
        });
    }

    /// 下拉选择的一行：改完写回配置。
    fn select_row(&mut self, ui: &mut egui::Ui, key: &str, label: &str, options: &[(&str, &str)]) {
        let mut value = self.config.text(key).to_owned();
        if ui::row(ui, label, None, |ui| ui::select(ui, &mut value, options)) {
            self.config.set(key, &value);
        }
    }

    /// 开关的一行：改完写回配置。
    fn flag_row(&mut self, ui: &mut egui::Ui, key: &str, label: &str, desc: Option<&str>) {
        let mut on = self.config.flag(key);
        if ui::row(ui, label, desc, |ui| ui::switch(ui, &mut on)) {
            self.config.set_flag(key, on);
        }
    }

    /// 整数步进的一行：范围从 [`config::int_range`] 来，改完写回配置。
    fn int_row(&mut self, ui: &mut egui::Ui, key: &str, label: &str) {
        let (min, max) = config::int_range(key);
        let mut value = self.config.int(key);
        // 候选总数上限偏大，一次跳 50 才不至于点一百下
        let step = if key == "MaxCandidates" { 50 } else { 1 };
        if ui::row(ui, label, None, |ui| {
            ui::stepper(ui, &mut value, min, max, step)
        }) {
            self.config.set(key, &value.to_string());
        }
    }

    /// 快捷键的一行：从预设里选（fcitx5 键名），配置里是预设之外的键时补一项「其他」。
    fn key_row(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        label: &str,
        desc: Option<&str>,
        presets: &[(&str, &str)],
    ) {
        let current = self.config.text(key).to_owned();
        let mut options = presets.to_vec();
        let other;
        if !presets.iter().any(|(value, _)| *value == current) {
            other = format!("其他（{current}）");
            options.push((current.as_str(), other.as_str()));
        }
        let mut value = current.clone();
        if ui::row(ui, label, desc, |ui| ui::select(ui, &mut value, &options)) {
            self.config.set(key, &value);
        }
    }

    /// 词库：雾凇（默认）和万象，两个都在安装包里，切换只是写 Lexicon 里的词库名。
    fn lexicon_card(&mut self, ui: &mut egui::Ui) {
        let current = self.config.text("Lexicon").to_owned();
        // 老配置里存的是万象词库的路径（以前是本地加载的），照样显示成选中
        let is_wanxiang = current == "wanxiang" || current.ends_with("/dicts/wanxiang/lexicon.tsv");
        if ui::radio_row(
            ui,
            "雾凇拼音（默认）",
            "约 89 万条，在安装包里",
            !is_wanxiang,
            |_| {},
        ) {
            self.config.set("Lexicon", "");
        }
        ui::divider(ui);
        if ui::radio_row(
            ui,
            "万象拼音",
            "约 212 万条，在安装包里",
            is_wanxiang,
            |_| {},
        ) {
            self.config.set("Lexicon", "wanxiang");
        }
    }

    /// 恢复上次的缩放级别（Ctrl +/- 调的那个）；只有用户再缩放时才存盘。
    ///
    /// eframe 会把上次的 zoom 记在它自己的记忆里，所以每次启动都明确设一次。
    fn apply_scale(&mut self, ctx: &egui::Context) {
        if !self.started {
            self.started = true;
            self.scale = if self.saved_scale > 0.0 {
                self.saved_scale
            } else {
                1.0
            };
            // 缩放因子要下一帧才生效，这帧不比对，否则会把旧值写回去
            ctx.set_zoom_factor(self.scale);
            return;
        }

        // Ctrl +/- 只改 zoom_factor，不动 input.zoom_delta()，所以直接比对当前值
        let now = ctx.zoom_factor();
        if (now - self.scale).abs() > 0.01 {
            self.scale = now;
            save_scale(&self.ui_path, now);
        }
    }

    /// 左边导航：logo 加五个页面。
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui::logo(ui, &self.brand);
        ui.add_space(20.0);
        for (page, icon, label) in PAGES {
            if ui::nav(ui, *icon, label, self.page == *page) {
                self.page = *page;
            }
            ui.add_space(2.0);
        }
    }

    /// 底部：状态文字在左，保存和重新读取在右。
    fn actions(&mut self, ui: &mut egui::Ui) {
        let status = self.status.clone();
        ui.horizontal(|ui| {
            if !status.is_empty() {
                ui.label(RichText::new(status).size(12.5).color(ui::MUTED));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if self.changed() {
                    "保存并生效"
                } else {
                    "保存"
                };

                if ui::primary_button(ui, label) {
                    self.save();
                }

                ui.add_space(8.0);

                if ui::secondary_button(ui, "重新读取") {
                    self.reload();
                }
            });
        });
    }

    /// 当前页面的内容，卡片列最大 660 宽，左边留一点白。
    fn page(&mut self, ui: &mut egui::Ui) {
        if !self.warnings.is_empty() {
            ui::card(ui, |ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "配置文件里有认不出来的值：{}",
                        self.warnings.join("；")
                    ))
                    .size(12.5)
                    .color(egui::Color32::from_rgb(0xC0, 0x6A, 0x1E)),
                );
                ui.add_space(8.0);
            });
        }

        match self.page {
            Page::General => self.general(ui),
            Page::Input => self.input_page(ui),
            Page::Voice => self.voice(ui),
            Page::Extra => self.extra(ui),
            Page::Words => self.words_page(ui),
        }
    }

    /// 常规：输入法提示、候选框、帮助。
    fn general(&mut self, ui: &mut egui::Ui) {
        ui::title(ui, "常规");
        ui::section(ui, "输入法");
        ui::card(ui, |ui| {
            ui::row(
                ui,
                "在 fcitx5 里使用 Flype",
                Some("用 fcitx5 的配置工具把 Flype 加进输入法列表，再用 Ctrl+空格 切换"),
                |_| false,
            );
        });

        ui::section(ui, "候选词");
        ui::card(ui, |ui| {
            self.select_row(
                ui,
                "CandidateLayout",
                "排列方向",
                &[("Horizontal", "横排"), ("Vertical", "竖排")],
            );
            ui::divider(ui);
            self.int_row(ui, "PageSize", "每页个数");
            ui::divider(ui);
            self.int_row(ui, "MaxCandidates", "候选总数上限");
            ui::divider(ui);
            ui::collapsible(ui, "翻页键", &mut self.page_keys_open);
            if self.page_keys_open {
                for (key, label) in [
                    ("PageKeysMinusEqual", "- ="),
                    ("PageKeysBracket", "[ ]"),
                    ("PageKeysCommaPeriod", ", ."),
                    ("PageKeysTab", "Shift+Tab / Tab"),
                ] {
                    self.flag_row(ui, key, label, None);
                }
            }
            ui::divider(ui);
            self.select_row(
                ui,
                "ArrowKeys",
                "打字时方向键",
                &[("Candidates", "用来选词"), ("Caret", "移动光标")],
            );
            ui::divider(ui);
            self.select_row(
                ui,
                "SelectKeys",
                "分号、引号选词",
                &[
                    ("Off", "不用"),
                    ("Semicolon", "分号选第 2 个"),
                    ("SemicolonQuote", "分号第 2 个、引号第 3 个"),
                ],
            );
            ui::divider(ui);
            self.flag_row(
                ui,
                "PickChar",
                "以词定字",
                Some("[ ] 上屏候选的第一个、最后一个字"),
            );
            ui::divider(ui);
            self.flag_row(
                ui,
                "Association",
                "联想",
                Some("上屏以后给出常跟在后面的词"),
            );
        });

        ui::section(ui, "外观");
        ui::card(ui, |ui| {
            let mut theme = self.classic.theme;
            if ui::row(
                ui,
                "Flype 绿色主题",
                Some("圆角候选框，亮色深色两套"),
                |ui| ui::switch(ui, &mut theme),
            ) {
                self.classic.theme = theme;
            }
            if self.classic.theme {
                ui::divider(ui);
                let mut mode = if self.classic.dark { "Dark" } else { "Light" }.to_owned();
                if ui::row(ui, "主题配色", None, |ui| {
                    ui::select(ui, &mut mode, &[("Light", "亮色"), ("Dark", "深色")])
                }) {
                    self.classic.dark = mode == "Dark";
                }
            }
            ui::divider(ui);
            let (min, max) = config::FONT_SIZE_RANGE;
            let mut size = self.classic.font_size;
            if ui::row(ui, "候选字号", None, |ui| {
                ui::stepper(ui, &mut size, min, max, 1)
            }) {
                self.classic.font_size = size;
            }
        });

        ui::section(ui, "帮助");
        ui::card(ui, |ui| {
            ui::collapsible(ui, "小技巧", &mut self.tips_open);
            if self.tips_open {
                ui::row(
                    ui,
                    "选词",
                    Some("空格上屏第一个；Page Up / Down 或方向键翻页"),
                    |_| false,
                );
                ui::row(
                    ui,
                    "临时英文",
                    Some("Shift 在中文里直接打英文，回车或空格上屏"),
                    |_| false,
                );
            }
            ui::divider(ui);
            let config_path = self.config_path.display().to_string();
            ui::row(ui, "配置文件", Some(config_path.as_str()), |_| false);
            ui::divider(ui);
            let custom_path = self.custom_path.display().to_string();
            ui::row(ui, "自定义词", Some(custom_path.as_str()), |_| false);
        });
    }

    /// 输入：拼音的纠错和模糊音、中英文、标点空格。
    fn input_page(&mut self, ui: &mut egui::Ui) {
        ui::title(ui, "输入");
        ui::section(ui, "拼音");
        ui::card(ui, |ui| {
            self.flag_row(
                ui,
                "TypoCorrection",
                "按错键自动纠正",
                Some("相邻键、顺序颠倒、多按漏按"),
            );
            ui::divider(ui);
            self.flag_row(ui, "RareExtChars", "列出扩展 B 区以后的生僻字", None);
            ui::divider(ui);
            self.flag_row(ui, "Fuzzy", "模糊音", None);
            if self.config.flag("Fuzzy") {
                ui::collapsible(ui, "模糊音规则", &mut self.fuzzy_open);
                if self.fuzzy_open {
                    for (key, label) in FUZZY_RULES {
                        self.flag_row(ui, key, label, None);
                    }
                }
            }
        });

        ui::section(ui, "中英文");
        ui::card(ui, |ui| {
            self.select_row(
                ui,
                "ShiftToggle",
                "单按 Shift 切换中英文",
                &[
                    ("Both", "左右都行"),
                    ("Left", "左 Shift"),
                    ("Right", "右 Shift"),
                    ("Off", "不用 Shift"),
                ],
            );
            ui::divider(ui);
            self.select_row(
                ui,
                "InitialMode",
                "切到 Flype 时",
                &[
                    ("Chinese", "中文"),
                    ("English", "英文"),
                    ("Last", "和上次一样"),
                ],
            );
            ui::divider(ui);
            self.key_row(
                ui,
                "ToggleKey",
                "切换中 / 英文的快捷键",
                None,
                TOGGLE_KEY_PRESETS,
            );
        });

        ui::section(ui, "标点、空格");
        ui::card(ui, |ui| {
            self.select_row(
                ui,
                "PunctuationStyle",
                "标点样式",
                &[
                    ("Fullwidth", "全角（中文标点）"),
                    ("Halfwidth", "半角（英文标点）"),
                ],
            );
            ui::divider(ui);
            self.flag_row(ui, "AutoSpace", "汉字和英文、数字之间自动加空格", None);
            ui::divider(ui);
            self.flag_row(ui, "AutoSpaceLetters", "汉字和英文字母之间", None);
            ui::divider(ui);
            self.flag_row(ui, "AutoSpaceDigits", "汉字和数字之间", None);
        });
    }

    /// 语音：开关、语音键、复制结果，以及识别模型的下载。
    fn voice(&mut self, ui: &mut egui::Ui) {
        ui::title(ui, "语音");
        ui::section(ui, "语音输入");
        ui::card(ui, |ui| {
            self.flag_row(ui, "Voice", "语音输入", Some("按住语音键说话，松开上屏"));
            ui::divider(ui);
            self.key_row(ui, "VoiceKey", "语音键", None, VOICE_KEY_PRESETS);
            ui::divider(ui);
            self.flag_row(ui, "VoiceCopy", "识别结果同时复制到剪贴板", None);
        });

        ui::section(ui, "识别模型");
        ui::card(ui, |ui| {
            let ready = self.model.ready;
            let busy = self.model.job.is_some();
            let mut download = false;
            let label = if ready {
                "语音模型已下载"
            } else {
                "语音模型还没下载"
            };
            ui::row(ui, label, None, |ui| {
                if !ready && !busy && ui::secondary_button(ui, "下载") {
                    download = true;
                }
                false
            });
            if download {
                self.start_model_download();
            }
            ui::divider(ui);
            ui::row(
                ui,
                "模型来源",
                Some("微信输入法的离线识别包（腾讯的资源），从腾讯的服务器下载，录音不上传"),
                |_| false,
            );
            let message = self.model.message.clone();
            if !message.is_empty() {
                ui::divider(ui);
                ui::row(ui, message.as_str(), None, |_| false);
            }
        });
    }

    /// 增强：emoji 推荐、日期时间、计算器、工具栏、剪贴板历史、词库文件。
    fn extra(&mut self, ui: &mut egui::Ui) {
        ui::title(ui, "增强");
        ui::section(ui, "打字的附加功能");
        ui::card(ui, |ui| {
            self.flag_row(ui, "EmojiSuggest", "打字时推荐 emoji", None);
            ui::divider(ui);
            self.key_row(
                ui,
                "EmojiKey",
                "打开表情面板的快捷键",
                None,
                EMOJI_KEY_PRESETS,
            );
            ui::divider(ui);
            self.flag_row(
                ui,
                "DateTime",
                "打 riqi、shijian、xingqi 给出日期和时间",
                None,
            );
            ui::divider(ui);
            self.flag_row(ui, "Calculator", "v 开头算算式", Some("v1+2*3"));
            ui::divider(ui);
            self.flag_row(
                ui,
                "Toolbar",
                "v + 空格打开工具栏",
                Some("剪贴板、常用语、符号、表情"),
            );
            ui::divider(ui);
            self.flag_row(
                ui,
                "ClipboardHistory",
                "记录剪贴板历史",
                Some("只在内存里，最多 20 条"),
            );
        });

        ui::section(ui, "词库");
        ui::card(ui, |ui| {
            self.lexicon_card(ui);
        });
    }

    /// 自定义词：一行一个编码和词，能删能加。
    fn words_page(&mut self, ui: &mut egui::Ui) {
        ui::title(ui, "自定义词");
        ui::section(ui, "自定义词");
        ui::card(ui, |ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "编码写拼音（可以合着写 nihao），也可以用 mail 这类字母；加进去的词排第一",
                )
                .size(12.0)
                .color(ui::MUTED),
            );
            ui.add_space(6.0);
            let mut remove = None;
            for (index, (code, word)) in self.words.rows.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui::text_field(ui, code, "拼音编码", 140.0);
                    ui::text_field(ui, word, "词", 330.0);
                    if ui::secondary_button(ui, "删除") {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                self.words.rows.remove(index);
            }
            ui.add_space(6.0);
            if ui::secondary_button(ui, "添加一行") {
                self.words.rows.push((String::new(), String::new()));
            }
            ui.add_space(8.0);
        });
    }
}

impl eframe::App for SettingsApp {
    /// 画整个界面：左边导航、右上内容、底部保存栏。
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.apply_scale(ui.ctx());
        self.poll_model(ui.ctx());
        egui::Panel::left("nav")
            .frame(
                egui::Frame::new()
                    .fill(ui::SIDEBAR)
                    .inner_margin(Margin::symmetric(12, 14)),
            )
            .exact_size(206.0)
            .resizable(false)
            .show(ui, |ui| self.sidebar(ui));
        egui::Panel::bottom("actions")
            .frame(egui::Frame::new().fill(ui::WINDOW).inner_margin(Margin {
                left: 28,
                right: 28,
                top: 8,
                bottom: 10,
            }))
            .show_separator_line(false)
            .show(ui, |ui| self.actions(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui::WINDOW).inner_margin(Margin {
                left: 28,
                right: 28,
                top: 14,
                bottom: 0,
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            let column = 660.0_f32.min(ui.available_width());
                            ui.add_space(((ui.available_width() - column) / 2.0).max(0.0));
                            ui.vertical(|ui| {
                                ui.set_max_width(column);
                                self.page(ui);
                            });
                        });
                    });
            });
    }
}
