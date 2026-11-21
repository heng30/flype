//! 输入会话的 C 接口（`flype_session_*`），见 core/include/flype_core.h 和 docs/ABI.md。
//!
//! 会话返回的指针（`FlypeFrame` 和里面的字符串、过渡函数返回的字符串）归会话所有，
//! 在同一个会话的下一次调用前有效。

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::mem::size_of;
use std::os::raw::{c_char, c_int};
use std::panic;
use std::ptr;

use crate::ffi::{set_last_error, FlypeEngineHandle};
use crate::symbols::PunctStyle;
use crate::session::{
    ArrowKeys, Candidate, CandidateKind, Config, Frame, Key, KeyAction, Mode, Panel, SelectKeys,
    Session, ShiftToggle, Tool,
};

/// 一次按键（C 布局）。`size` 是外壳编译时的 `sizeof(FlypeKey)`：
/// 以后在末尾加字段，旧外壳传的 size 小，没传的字段当 0。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FlypeKey {
    /// 外壳编译时的 `sizeof(FlypeKey)`，没传到的字段当 0。
    pub size: u32,
    /// XKB keysym（字母、数字就是 ASCII 码）。
    pub keysym: u32,
    /// 修饰键，`session::modifier` 的位掩码（Shift、Ctrl、Alt、Super…）。
    pub modifiers: u32,
    /// 1 是松开，0 是按下。
    pub release: u32,
    /// 应用里光标前一个字符的码位，读不到给 0。
    pub prev_char: u32,
    /// `FLYPE_KEY_ACTION_*`：外壳认出来的全局热键、候选框上的翻页按钮。
    pub action: u32,
    /// 1.10 起：`FLYPE_KEY_HINT_*`。
    pub hints: u32,
    /// 1.10 起：按键码，不知道给 0。
    pub code: u32,
    /// 1.10 起：按住不放的自动重复。
    pub repeat: u32,
    /// 1.10 起：按键时刻（毫秒，单调递增），不知道给 0。
    pub time_ms: u64,
}

/// 输入框是密码框/网址框/邮箱框：不自动补空格
pub const FLYPE_KEY_HINT_NO_AUTO_SPACE: u32 = crate::session::key_hints::NO_AUTO_SPACE;
/// 外壳开着自己的面板：这时 Shift 不切中英文
pub const FLYPE_KEY_HINT_PANEL_OPEN: u32 = crate::session::key_hints::PANEL_OPEN;
/// 应用告诉了光标周围的文字：`prev_char` 为 0 是真没字，不是读不到
pub const FLYPE_KEY_HINT_SURROUNDING: u32 = crate::session::key_hints::SURROUNDING;
/// 密码框：选的词也不学
pub const FLYPE_KEY_HINT_NO_LEARN: u32 = crate::session::key_hints::NO_LEARN;

/// 一页候选（横排或竖排）
pub const FLYPE_PANEL_NORMAL: c_int = 0;
/// 横排时按下键展开的网格面板
pub const FLYPE_PANEL_EXPANDED: c_int = 1;
/// 表情面板（编辑区是搜索词）
pub const FLYPE_PANEL_EMOJI: c_int = 2;
/// v + 空格的工具栏（菜单、剪贴板、常用语、符号）
pub const FLYPE_PANEL_TOOL: c_int = 3;
/// 1.11 起：上屏以后的联想（`FlypeSessionConfig.association` 开着时才有）。
pub const FLYPE_PANEL_ASSOCIATION: c_int = 4;

/// 没开面板
pub const FLYPE_TOOL_NONE: c_int = 0;
/// 工具栏的菜单页
pub const FLYPE_TOOL_MENU: c_int = 1;
/// 剪贴板历史页
pub const FLYPE_TOOL_CLIPBOARD: c_int = 2;
/// 常用语（自定义词）页
pub const FLYPE_TOOL_PHRASES: c_int = 3;
/// 符号页（也是表情面板，只是不搜索时列全部符号）
pub const FLYPE_TOOL_SYMBOLS: c_int = 4;
/// 表情页
pub const FLYPE_TOOL_EMOJI: c_int = 5;

/// 中文输入
pub const FLYPE_MODE_CHINESE: c_int = 0;
/// 英文输入（按键原样给应用）
pub const FLYPE_MODE_ENGLISH: c_int = 1;

/// 内部模式转 C 的 `FLYPE_MODE_*`
fn mode_to_c(mode: Mode) -> c_int {
    match mode {
        Mode::Chinese => FLYPE_MODE_CHINESE,
        Mode::English => FLYPE_MODE_ENGLISH,
    }
}

/// C 的 `FLYPE_MODE_*` 转内部模式，认不出的当中文
fn mode_from_c(raw: c_int) -> Mode {
    match raw {
        FLYPE_MODE_ENGLISH => Mode::English,
        _ => Mode::Chinese,
    }
}

/// 左右 Shift 都切换中英文
pub const FLYPE_SHIFT_TOGGLE_BOTH: c_int = 0;
/// 只有左 Shift 切换
pub const FLYPE_SHIFT_TOGGLE_LEFT: c_int = 1;
/// 只有右 Shift 切换
pub const FLYPE_SHIFT_TOGGLE_RIGHT: c_int = 2;
/// 不切（Shift 只当修饰键）
pub const FLYPE_SHIFT_TOGGLE_OFF: c_int = 3;

/// 普通按键，没有全局热键动作
pub const FLYPE_KEY_ACTION_NONE: u32 = 0;
/// 候选框上一页
pub const FLYPE_KEY_ACTION_PREV_PAGE: u32 = 1;
/// 候选框下一页
pub const FLYPE_KEY_ACTION_NEXT_PAGE: u32 = 2;
/// 光标在候选里往前挪一格
pub const FLYPE_KEY_ACTION_PREV_CANDIDATE: u32 = 3;
/// 光标在候选里往后挪一格
pub const FLYPE_KEY_ACTION_NEXT_CANDIDATE: u32 = 4;
/// 只告诉会话有这个键，会话不处理（单按 Shift 要用到）
pub const FLYPE_KEY_ACTION_OBSERVE: u32 = 5;
/// 设置里另设的中英文切换键
pub const FLYPE_KEY_ACTION_TOGGLE_MODE: u32 = 6;

/// C 的 action 编号转内部枚举，认不出的当普通按键
fn key_action(raw: u32) -> KeyAction {
    match raw {
        FLYPE_KEY_ACTION_PREV_PAGE => KeyAction::PrevPage,
        FLYPE_KEY_ACTION_NEXT_PAGE => KeyAction::NextPage,
        FLYPE_KEY_ACTION_PREV_CANDIDATE => KeyAction::PrevCandidate,
        FLYPE_KEY_ACTION_NEXT_CANDIDATE => KeyAction::NextCandidate,
        FLYPE_KEY_ACTION_OBSERVE => KeyAction::Observe,
        FLYPE_KEY_ACTION_TOGGLE_MODE => KeyAction::ToggleMode,
        _ => KeyAction::None,
    }
}

/// 不用标点选词
pub const FLYPE_SELECT_KEYS_OFF: c_int = 0;
/// 分号选第二个候选
pub const FLYPE_SELECT_KEYS_SEMICOLON: c_int = 1;
/// 分号、引号选第二、三个候选
pub const FLYPE_SELECT_KEYS_SEMICOLON_QUOTE: c_int = 2;

/// 方向键都用来挑候选
pub const FLYPE_ARROW_KEYS_CANDIDATES: c_int = 0;
/// 左右键在拼音里移光标，上下键选词
pub const FLYPE_ARROW_KEYS_CARET: c_int = 1;

/// 会话的设置（C 布局）。外壳没填的字段（size 比我们认识的短）用默认值。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FlypeSessionConfig {
    /// 外壳编译时的 `sizeof(FlypeSessionConfig)`，没传到的项用默认值。
    pub size: u32,
    /// 一页几个候选；0 用默认（9）。
    pub page_size: u32,
    /// V 模式计算器，默认 1。
    pub calculator: c_int,
    /// 日期时间候选，默认 1。
    pub date_time: c_int,
    /// 打字时推荐 emoji，默认 1。
    pub emoji_suggest: c_int,
    /// 候选横排（默认 1）。
    pub horizontal: c_int,
    /// 翻页键，`FLYPE_PAGE_KEYS_*` 位掩码（默认 - =）。
    pub page_keys: u32,
    /// `FLYPE_SELECT_KEYS_*`（默认关）。
    pub select_keys: c_int,
    /// 1.8 起：以词定字（`[ ]` 取候选的第一个、最后一个字），默认 1。
    pub pick_char: c_int,
    /// 1.10 起：0 全角、1 半角。
    pub punct_style: c_int,
    /// 1.10 起：自动补空格，`autospace::LETTERS | DIGITS`，0 关。
    pub auto_space: u32,
    /// 1.10 起：`FLYPE_SHIFT_TOGGLE_*`，默认两边都行。
    pub shift_toggle: c_int,
    /// 1.10 起：按住 Shift 多久以内松开算单按（毫秒），0 用默认（500）。
    pub shift_tap_ms: u32,
    /// 1.10 起：只打一个 v 再按空格打开工具栏，默认 1。
    pub toolbar: c_int,
    /// 1.10 起：记剪贴板历史（工具栏「剪贴板」页的提示看它），默认 0。
    pub clipboard_history: c_int,
    /// 1.10 起：打字途中方向键做什么，`FLYPE_ARROW_KEYS_*`（#129），默认挑候选。
    pub arrow_keys: c_int,
    /// 1.11 起：上屏以后的联想，默认 0（旧外壳不认识联想、画不出来；设置里的默认值是开，外壳照设置填）。
    pub association: c_int,
}

/// 默认值全取自 `Config::default()`，size 填本结构体的大小
impl Default for FlypeSessionConfig {
    /// 各项照内部 Config 的默认值填，size 填本结构体的实际大小
    fn default() -> Self {
        let config = Config::default();
        Self {
            size: size_of::<FlypeSessionConfig>() as u32,
            page_size: config.page_size as u32,
            calculator: c_int::from(config.calculator),
            date_time: c_int::from(config.date_time),
            emoji_suggest: c_int::from(config.emoji_suggest),
            horizontal: c_int::from(config.horizontal),
            page_keys: config.page_keys,
            select_keys: match config.select_keys {
                SelectKeys::Off => FLYPE_SELECT_KEYS_OFF,
                SelectKeys::Semicolon => FLYPE_SELECT_KEYS_SEMICOLON,
                SelectKeys::SemicolonQuote => FLYPE_SELECT_KEYS_SEMICOLON_QUOTE,
            },
            pick_char: c_int::from(config.pick_char),
            punct_style: c_int::from(config.punct_style == PunctStyle::Halfwidth),
            auto_space: config.auto_space,
            shift_toggle: match config.shift_toggle {
                ShiftToggle::Both => FLYPE_SHIFT_TOGGLE_BOTH,
                ShiftToggle::Left => FLYPE_SHIFT_TOGGLE_LEFT,
                ShiftToggle::Right => FLYPE_SHIFT_TOGGLE_RIGHT,
                ShiftToggle::Off => FLYPE_SHIFT_TOGGLE_OFF,
            },
            shift_tap_ms: config.shift_tap_ms,
            toolbar: c_int::from(config.toolbar),
            clipboard_history: c_int::from(config.clipboard_history),
            arrow_keys: match config.arrow_keys {
                ArrowKeys::Candidates => FLYPE_ARROW_KEYS_CANDIDATES,
                ArrowKeys::Caret => FLYPE_ARROW_KEYS_CARET,
            },
            association: c_int::from(config.association),
        }
    }
}

impl FlypeSessionConfig {
/// 按 size 读出来的结构转内部 `Config`：0 的项用各自的默认值
    fn to_config(self) -> Config {
        Config {
            page_size: if self.page_size == 0 {
                Config::default().page_size
            } else {
                self.page_size as usize
            },
            calculator: self.calculator != 0,
            date_time: self.date_time != 0,
            emoji_suggest: self.emoji_suggest != 0,
            horizontal: self.horizontal != 0,
            page_keys: self.page_keys,
            select_keys: match self.select_keys {
                FLYPE_SELECT_KEYS_SEMICOLON => SelectKeys::Semicolon,
                FLYPE_SELECT_KEYS_SEMICOLON_QUOTE => SelectKeys::SemicolonQuote,
                _ => SelectKeys::Off,
            },
            pick_char: self.pick_char != 0,
            punct_style: if self.punct_style == 1 {
                PunctStyle::Halfwidth
            } else {
                PunctStyle::Fullwidth
            },
            auto_space: self.auto_space,
            shift_toggle: match self.shift_toggle {
                FLYPE_SHIFT_TOGGLE_LEFT => ShiftToggle::Left,
                FLYPE_SHIFT_TOGGLE_RIGHT => ShiftToggle::Right,
                FLYPE_SHIFT_TOGGLE_OFF => ShiftToggle::Off,
                _ => ShiftToggle::Both,
            },
            shift_tap_ms: if self.shift_tap_ms == 0 {
                Config::default().shift_tap_ms
            } else {
                self.shift_tap_ms
            },
            // 旧外壳给的结构体短、没有这一项时，read_sized 留着 Default 的值（开着）
            toolbar: self.toolbar != 0,
            clipboard_history: self.clipboard_history != 0,
            arrow_keys: match self.arrow_keys {
                FLYPE_ARROW_KEYS_CARET => ArrowKeys::Caret,
                _ => ArrowKeys::Candidates,
            },
            association: self.association != 0,
        }
    }
}

/// 过渡：候选框的状态（C 布局），fcitx5 外壳按它摆自己的候选列表。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FlypeSessionView {
    /// 外壳编译时的 `sizeof(FlypeSessionView)`。
    pub size: u32,
    /// 普通候选框在第几页。
    pub page: usize,
    /// 普通候选框的光标（全部候选里的序号，可以不在这一页上）。
    pub cursor: usize,
    /// 1 是展开的网格面板。
    pub expanded: c_int,
    /// 展开面板的光标（全部候选里的序号）。
    pub grid_cursor: usize,
    /// 候选重算过几次：变了就要重新抄候选。
    pub generation: u64,
}

/// 过渡：一个候选的全部信息（C 布局），字段含义见头文件。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FlypeCandidateInfo {
    /// 外壳编译时的 `sizeof(FlypeCandidateInfo)`。
    pub size: u32,
    /// 候选框里显示的字（会话持有）。
    pub text: *const c_char,
    /// 拼音（空格分音节），计算结果、emoji 这类是空串。
    pub pinyin: *const c_char,
    /// 用掉了还没选词那部分输入的多少字节。
    pub consumed: usize,
    /// `FLYPE_CAND_*`。
    pub kind: c_int,
    /// 选中时上屏的字，emoji 推荐和 text 不一样。
    pub commit: *const c_char,
    /// emoji 本身，不是 emoji 是空串。
    pub emoji: *const c_char,
    /// 1.10 起：候选后面的小字，没有是空串。
    pub comment: *const c_char,
}

/// 候选种类：词库里的词
pub const FLYPE_CAND_WORD: c_int = 0;
/// 原样上屏、不学的（算式、日期、带换行的自定义词）
pub const FLYPE_CAND_PLAIN: c_int = 1;
/// emoji：上屏 `commit`，记进最近用过
pub const FLYPE_CAND_EMOJI: c_int = 2;

/// 一个候选的 C 字符串（随候选重算重建）。
struct CCandidate {
    /// 候选框里显示的字。
    text: CString,
    /// 拼音（空格分音节）。
    pinyin: CString,
    /// 选中时上屏的字。
    commit: CString,
    /// emoji 本身，没有是空串。
    emoji: CString,
    /// 候选后面的小字。
    comment: CString,
    /// 用掉的输入字节数。
    consumed: usize,
    /// 种类，`FLYPE_CAND_*`。
    kind: c_int,
}

impl CCandidate {
/// 填成 C 的候选信息：指针指向本结构里的字符串，`size` 交给 `write_sized` 写
    fn info(&self) -> FlypeCandidateInfo {
        FlypeCandidateInfo {
            size: 0,
            text: self.text.as_ptr(),
            pinyin: self.pinyin.as_ptr(),
            consumed: self.consumed,
            kind: self.kind,
            commit: self.commit.as_ptr(),
            emoji: self.emoji.as_ptr(),
            comment: self.comment.as_ptr(),
        }
    }

/// 从内部候选转一份 C 字符串，kind 换成 `FLYPE_CAND_*`
    fn new(c: &Candidate) -> Self {
        Self {
            text: to_c(&c.text),
            pinyin: to_c(&c.pinyin),
            commit: to_c(&c.commit),
            emoji: to_c(&c.emoji),
            comment: to_c(&c.comment),
            consumed: c.consumed,
            kind: match c.kind {
                CandidateKind::Word => FLYPE_CAND_WORD,
                CandidateKind::Plain => FLYPE_CAND_PLAIN,
                CandidateKind::Emoji => FLYPE_CAND_EMOJI,
            },
        }
    }
}

/// 一次调用之后外壳要做的事（C 布局），字段含义见头文件。
#[repr(C)]
pub struct FlypeFrame {
    /// 外壳编译时的 `sizeof(FlypeFrame)`，写的时候按它截断。
    pub size: u32,
    /// 会话处理了这次调用（1/0）；0 时外壳接着自己处理。
    pub handled: c_int,
    /// 要上屏的 UTF-8 字符串，没有是 NULL。
    pub commit: *const c_char,
    /// 编辑区（已选词 + 剩下的输入），总是有效。
    pub preedit: *const c_char,
    /// 编辑区光标的 UTF-8 字节偏移。
    pub preedit_cursor: c_int,
    /// 候选框上方的提示，没有是 NULL。
    pub aux: *const c_char,
    /// `FLYPE_PANEL_*`：候选框画成什么样。
    pub panel: c_int,
    /// 显示了几个候选（一页，或展开面板的一屏）。
    pub cand_count: usize,
    /// 光标在显示的这些里的位置，不在这一页上是 -1。
    pub cand_cursor: c_int,
    /// 前面还有一页（1/0，展开面板是一屏）。
    pub has_prev: c_int,
    /// 后面还有一页（1/0）。
    pub has_next: c_int,
    /// 大于 0：过这么久（毫秒）再调一次会话（现在还没用，恒为 0）。
    pub wake_after_ms: u32,
    /// 1.7 起：这次调用忘掉了一个学过的词（Ctrl+Delete），外壳该马上存用户词库和输入习惯。
    pub forgot: c_int,
    /// 1.10 起：这次上屏里有引擎学到的选词。
    pub learned: c_int,
    /// 1.10 起：这次调用以后的输入模式，`FLYPE_MODE_*`。
    pub mode: c_int,
    /// 1.10 起：这次调用切换了输入模式。
    pub mode_changed: c_int,
}

/// 全清空的一次调用结果，size 填本结构体的大小
impl Default for FlypeFrame {
    /// 一次空的调用结果：没上屏、没候选、没提示，模式是中文
    fn default() -> Self {
        Self {
            size: size_of::<FlypeFrame>() as u32,
            handled: 0,
            commit: ptr::null(),
            preedit: ptr::null(),
            preedit_cursor: 0,
            aux: ptr::null(),
            panel: 0,
            cand_count: 0,
            cand_cursor: 0,
            has_prev: 0,
            has_next: 0,
            wake_after_ms: 0,
            forgot: 0,
            learned: 0,
            mode: FLYPE_MODE_CHINESE,
            mode_changed: 0,
        }
    }
}

/// 会话句柄。引擎借用的是外壳的 `FlypeEngineHandle`，外壳保证它比会话活得久
/// （或者先 set_engine(NULL)）。
pub struct FlypeSession {
    /// 内部会话：编辑区、候选、学习都在这。
    session: Session<'static>,
    /// 候选的 C 字符串，和 `c_generation` 那次重算的候选对应。
    c_candidates: Vec<CCandidate>,
    /// `c_candidates` 是对着哪一次候选重算抄的（和 `session.generation()` 比）。
    c_generation: u64,
    /// 上一次 `publish` 的结果，指针交给外壳，活到下一次调用。
    frame: FlypeFrame,
    /// `frame.commit` 指向的字符串（要上屏的字），没有是 NULL。
    commit: Option<CString>,
    /// `frame.preedit` 指向的字符串（编辑区）。
    preedit: CString,
    /// frame 里显示的候选在全部候选里的起点（`flype_session_candidate` 用）。
    shown_start: usize,
    /// 上一次上屏是选的哪个候选（以前的 `flype_session_last_choice` 用，现在只给调试看）。
    last_choice: Option<CCandidate>,
    /// 过渡函数返回的字符串。
    text: CString,
    /// 候选框上方的提示（工具栏、表情面板）。
    aux: CString,
}


/// Rust 字符串转 C 字符串，NUL 换成空格免得 C 侧当结尾
fn to_c(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).unwrap_or_default()
}

/// 入参 C 字符串转 String：NULL 当空串，非 UTF-8 的有损替换
unsafe fn arg(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// 按 size 读一个带 size 字段的结构体：外壳给的比我们认识的短时，缺的字段是 `T::default()` 的值。
unsafe fn read_sized<T: Copy + Default>(ptr: *const T, size: u32) -> T {
    let mut value = T::default();
    let len = (size as usize).min(size_of::<T>());
    unsafe {
        ptr::copy_nonoverlapping(ptr as *const u8, &mut value as *mut T as *mut u8, len);
    }
    value
}

/// 按外壳的 out->size 写一个带 size 字段的结构体（size 字段写成实际写了多少字节）。
unsafe fn write_sized<T: Copy>(out: *mut T, mut value: T) {
    let len = (unsafe { *(out as *const u32) } as usize).min(size_of::<T>());
    unsafe {
        *(&mut value as *mut T as *mut u32) = len as u32;
        ptr::copy_nonoverlapping(&value as *const T as *const u8, out as *mut u8, len);
    }
}

impl FlypeSession {
/// 把内部 Frame 抄进 C 的 frame（字符串存在会话里），返回 frame 的指针
    fn publish(&mut self, frame: Frame) -> *const FlypeFrame {
        self.commit = frame.commit.as_deref().map(to_c);
        self.preedit = to_c(&frame.preedit);
        self.shown_start = frame.cand_start;
        self.last_choice = self.session.last_choice().map(CCandidate::new);
        self.frame = FlypeFrame {
            handled: c_int::from(frame.handled),
            commit: self.commit.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            preedit: self.preedit.as_ptr(),
            preedit_cursor: frame.preedit_cursor.try_into().unwrap_or(c_int::MAX),
            panel: match frame.panel {
                Panel::Normal => FLYPE_PANEL_NORMAL,
                Panel::Expanded => FLYPE_PANEL_EXPANDED,
                Panel::Emoji => FLYPE_PANEL_EMOJI,
                Panel::Tool => FLYPE_PANEL_TOOL,
                Panel::Association => FLYPE_PANEL_ASSOCIATION,
            },
            cand_count: frame.cand_count,
            cand_cursor: frame
                .cand_cursor
                .and_then(|c| c_int::try_from(c).ok())
                .unwrap_or(-1),
            has_prev: c_int::from(frame.has_prev),
            has_next: c_int::from(frame.has_next),
            forgot: c_int::from(frame.forgot),
            learned: c_int::from(frame.learned),
            mode: mode_to_c(frame.mode),
            mode_changed: c_int::from(frame.mode_changed),
            wake_after_ms: frame.wake_after_ms,
            ..FlypeFrame::default()
        };
        if !frame.aux.is_empty() {
            self.aux = to_c(&frame.aux);
            self.frame.aux = self.aux.as_ptr();
        }
        &self.frame
    }

    /// 跑一个会改会话的调用，出了 panic 就当没处理。
    fn run(
        &mut self,
        what: &str,
        f: impl FnOnce(&mut Session<'static>) -> Frame,
    ) -> *const FlypeFrame {
        let inner = &mut self.session;
        match panic::catch_unwind(panic::AssertUnwindSafe(|| f(inner))) {
            Ok(frame) => self.publish(frame),
            Err(_) => {
                set_last_error(&format!("session {what} panicked"));
                self.current(false)
            }
        }
    }

/// 换掉会话里的返回串并给出指针（活到下一次调用）
    fn text(&mut self, text: &str) -> *const c_char {
        self.text = to_c(text);
        self.text.as_ptr()
    }

    /// 现在的编辑区；算候选时出了 panic 就当没有候选。
    fn current(&mut self, handled: bool) -> *const FlypeFrame {
        let inner = &mut self.session;
        let frame = match panic::catch_unwind(panic::AssertUnwindSafe(|| inner.frame(handled))) {
            Ok(frame) => frame,
            Err(_) => {
                set_last_error("session panicked");
                let preedit = self.session.selected_text() + self.session.remaining();
                Frame {
                    handled,
                    preedit_cursor: preedit.len(),
                    preedit,
                    ..Frame::default()
                }
            }
        };
        self.publish(frame)
    }

/// 候选的 C 版本；generation 没变就不重建，指针保持稳定
    fn c_candidates(&mut self) -> &[CCandidate] {
        let inner = &mut self.session;
        let computed = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            inner.display_candidates();
        }));
        if computed.is_err() {
            set_last_error("session panicked");
            self.c_candidates.clear();
            return &self.c_candidates;
        }
        if self.session.generation() != self.c_generation {
            self.c_candidates = self
                .session
                .display_candidates()
                .iter()
                .map(CCandidate::new)
                .collect();
            self.c_generation = self.session.generation();
        }
        &self.c_candidates
    }
}

/// 引擎句柄转内部 Engine 借用；NULL 或空句柄时是 None（会话照常能打字，只是没候选）
unsafe fn engine_ref(engine: *mut FlypeEngineHandle) -> Option<&'static crate::Engine> {
    unsafe { engine.as_ref() }.map(|handle| &handle.engine)
}

/// 建会话。engine 可以传 NULL（外壳的词库在后台载入，载入好之前也要能打字，载入好了
/// 用 set_engine 补上）；传了的话要比会话活得久。config 可以传 NULL（全用默认）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_new(
    engine: *mut FlypeEngineHandle,
    config: *const FlypeSessionConfig,
) -> *mut FlypeSession {
    let mut session = Session::new();
    session.set_engine(unsafe { engine_ref(engine) });
    if !config.is_null() {
        session.configure(unsafe { read_sized(config, (*config).size) }.to_config());
    }
    let mut handle = Box::new(FlypeSession {
        session,
        c_candidates: Vec::new(),
        c_generation: 0,
        frame: FlypeFrame::default(),
        commit: None,
        preedit: CString::default(),
        shown_start: 0,
        last_choice: None,
        text: CString::default(),
        aux: CString::default(),
    });
    handle.current(false);
    Box::into_raw(handle)
}

/// 换引擎（词库在后台载入好了），可以传 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_set_engine(
    session: *mut FlypeSession,
    engine: *mut FlypeEngineHandle,
) {
    if let Some(handle) = unsafe { session.as_mut() } {
        handle.session.set_engine(unsafe { engine_ref(engine) });
    }
}

/// 改设置。config 为 NULL 时全用默认。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_configure(
    session: *mut FlypeSession,
    config: *const FlypeSessionConfig,
) {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return;
    };
    let config = if config.is_null() {
        Config::default()
    } else {
        unsafe { read_sized(config, (*config).size) }.to_config()
    };
    handle.session.configure(config);
}

/// 现在的编辑区（不处理按键，handled 为 0）。session 为 NULL 时返回 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_frame(
    session: *mut FlypeSession,
) -> *const FlypeFrame {
    match unsafe { session.as_mut() } {
        Some(handle) => handle.current(false),
        None => ptr::null(),
    }
}

/// 释放会话（引擎是外壳的，不动）；NULL 什么也不做
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_free(session: *mut FlypeSession) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

/// 处理一次按键。session 或 key 为 NULL 时返回 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_key(
    session: *mut FlypeSession,
    key: *const FlypeKey,
) -> *const FlypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    if key.is_null() {
        set_last_error("key is null");
        return ptr::null();
    }
    let raw = unsafe { read_sized(key, (*key).size) };
    let key = Key {
        keysym: raw.keysym,
        modifiers: raw.modifiers,
        release: raw.release != 0,
        prev_char: char::from_u32(raw.prev_char).filter(|&c| c != '\0'),
        action: key_action(raw.action),
        hints: raw.hints,
        code: raw.code,
        repeat: raw.repeat != 0,
        time_ms: raw.time_ms,
    };
    handle.run("key", |session| session.key(&key))
}

/// 选候选框里显示的第 index 个（一页，或展开面板的一屏；鼠标点候选）。序号不对时 handled 为 0。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_select(
    session: *mut FlypeSession,
    index: usize,
) -> *const FlypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("select", |session| session.select(index))
}

/// 上一次返回的 frame 里显示的第 index 个候选（0 起，小于 cand_count）。越界或 NULL 返回 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_candidate(
    session: *mut FlypeSession,
    index: usize,
) -> *const c_char {
    unsafe { shown_candidate(session, index) }.map_or(ptr::null(), |c| c.text.as_ptr())
}

/// 上一次返回的 frame 里显示的第 index 个候选后面的小字（1.10），没有是空串。
/// 越界或 NULL 返回 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_candidate_comment(
    session: *mut FlypeSession,
    index: usize,
) -> *const c_char {
    unsafe { shown_candidate(session, index) }.map_or(ptr::null(), |c| c.comment.as_ptr())
}

/// 上一次的 frame 里显示的第 index 个候选。
unsafe fn shown_candidate<'a>(session: *mut FlypeSession, index: usize) -> Option<&'a CCandidate> {
    let handle = unsafe { session.as_mut() }?;
    if index >= handle.frame.cand_count {
        return None;
    }
    let at = handle.shown_start + index;
    handle.c_candidates().get(at)
}

/// 清空编辑区；commit_pending 非 0 时把已选的词和原始输入放进 frame->commit，为 0 是放弃（上文也作废）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_reset(
    session: *mut FlypeSession,
    commit_pending: c_int,
) -> *const FlypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("reset", |session| session.reset(commit_pending != 0))
}

/// 切换输入模式（`FLYPE_MODE_*`，外部命令、默认模式、外壳全局的模式用）。切到英文时打到一半的放进
/// frame->commit。（1.10 起）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_set_mode(
    session: *mut FlypeSession,
    mode: c_int,
) -> *const FlypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("set_mode", |session| session.set_mode(mode_from_c(mode)))
}

/// 现在的输入模式，`FLYPE_MODE_*`；session 为 NULL 时是中文。（1.10 起）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_mode(session: *const FlypeSession) -> c_int {
    unsafe { session.as_ref() }.map_or(FLYPE_MODE_CHINESE, |handle| mode_to_c(handle.session.mode()))
}

/// 外壳要上屏一段与拼音无关的字（工具栏、表情面板、语音识别结果）：补好空格放进 frame->commit。prev_char 是应用里光标前的字，读不到给 0。（1.10 起）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_commit_text(
    session: *mut FlypeSession,
    text: *const c_char,
    prev_char: u32,
) -> *const FlypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let text = unsafe { arg(text) };
    let prev = char::from_u32(prev_char).filter(|&c| c != '\0');
    handle.run("commit_text", |session| session.commit_text(&text, prev))
}

/// 表情面板快捷键（外壳按自己的设置认出来）：开着就关上；打了一半的拼音拿来当搜索词。（1.10 起）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_toggle_emoji_panel(
    session: *mut FlypeSession,
) -> *const FlypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("toggle_emoji_panel", |session| session.toggle_emoji_panel())
}

/// 打开工具栏的一页或表情面板（`FLYPE_TOOL_*`，外壳的菜单、按钮用；v + 空格会话自己开）。
/// `FLYPE_TOOL_NONE` 关上。（1.10 起）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_open_tool(
    session: *mut FlypeSession,
    tool: c_int,
) -> *const FlypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let tool = match tool {
        FLYPE_TOOL_MENU => Some(Tool::Menu),
        FLYPE_TOOL_CLIPBOARD => Some(Tool::Clipboard),
        FLYPE_TOOL_PHRASES => Some(Tool::Phrases),
        FLYPE_TOOL_SYMBOLS => Some(Tool::Emoji { symbols: true }),
        FLYPE_TOOL_EMOJI => Some(Tool::Emoji { symbols: false }),
        _ => None,
    };
    handle.run("open_tool", |session| match tool {
        Some(tool) => session.open_tool(tool),
        None => session.close_panel(),
    })
}

/// 开着的面板，`FLYPE_TOOL_*`（没开是 NONE）。（1.10 起）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_open_panel(session: *const FlypeSession) -> c_int {
    let Some(handle) = (unsafe { session.as_ref() }) else {
        return FLYPE_TOOL_NONE;
    };
    match handle.session.open_panel() {
        None => FLYPE_TOOL_NONE,
        Some(Tool::Menu) => FLYPE_TOOL_MENU,
        Some(Tool::Clipboard) => FLYPE_TOOL_CLIPBOARD,
        Some(Tool::Phrases) => FLYPE_TOOL_PHRASES,
        Some(Tool::Emoji { symbols: true }) => FLYPE_TOOL_SYMBOLS,
        Some(Tool::Emoji { symbols: false }) => FLYPE_TOOL_EMOJI,
    }
}

/// 联想开着（上屏以后的联想行，`FLYPE_PANEL_ASSOCIATION`）返回 1，没有是 0。不改会话，
/// 之前拿到的 frame 照样有效。（1.11 起）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_association(session: *const FlypeSession) -> c_int {
    unsafe { session.as_ref() }.map_or(0, |handle| c_int::from(handle.session.association_open()))
}

/// 最近用过的 emoji 存在这个文件（一行一个，进程内所有会话共用；读进已有的）。
/// NULL 或空串：只记在内存里，并清空。（1.10 起）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_recent_emoji_path(path: *const c_char) {
    crate::recent_emoji::set_path(&unsafe { arg(path) });
}

/// 最近用过的 emoji，最近的在前；用 flype_candidates_* 读取并释放。（1.10 起）
#[unsafe(no_mangle)]
pub extern "C" fn flype_recent_emoji_list() -> *mut crate::ffi::FlypeCandidates {
    let texts: Vec<CString> = crate::recent_emoji::list().iter().map(|t| to_c(t)).collect();
    let scores = vec![0.0; texts.len()];
    Box::into_raw(Box::new(crate::ffi::FlypeCandidates::plain(texts, scores)))
}

// ---- 读会话状态、整个候选列表（fcitx5 的候选框要整个列表），见 docs/ABI.md「输入会话」----

/// 全部原始输入（含已选词用掉的部分）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_input(session: *mut FlypeSession) -> *const c_char {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let text = handle.session.input().to_owned();
    handle.text(&text)
}

/// 还没选词的那部分原始输入。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_remaining(session: *mut FlypeSession) -> *const c_char {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let text = handle.session.remaining().to_owned();
    handle.text(&text)
}

/// 已选的词连起来。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_selected_text(
    session: *mut FlypeSession,
) -> *const c_char {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let text = handle.session.selected_text();
    handle.text(&text)
}

/// 选第 index 个候选（全部候选里的序号，不分页）：句首词进已选栈，其余上屏并记选词，
/// 见 `Session::choose`。序号不对时 handled 为 0。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_choose(
    session: *mut FlypeSession,
    index: usize,
) -> *const FlypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("choose", |session| session.choose(index))
}

/// 候选框的状态写进 out（按 out->size 写）。返回 0 成功，-1 参数为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_view(
    session: *mut FlypeSession,
    out: *mut FlypeSessionView,
) -> c_int {
    let (Some(handle), false) = (unsafe { session.as_mut() }, out.is_null()) else {
        return -1;
    };
    let inner = &mut handle.session;
    let Ok(view) = panic::catch_unwind(panic::AssertUnwindSafe(|| inner.view())) else {
        set_last_error("session view panicked");
        return -1;
    };
    let value = FlypeSessionView {
        size: 0,
        page: view.page,
        cursor: view.cursor,
        expanded: c_int::from(view.expanded),
        grid_cursor: view.grid_cursor,
        generation: handle.session.generation(),
    };
    unsafe { write_sized(out, value) };
    0
}

/// 全部候选的个数（不分页）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_candidate_total(session: *mut FlypeSession) -> usize {
    unsafe { session.as_mut() }.map_or(0, |handle| handle.c_candidates().len())
}

/// 第 index 个候选（不分页）写进 out（按 out->size 写）。返回 0 成功，-1 越界或参数为 NULL。
/// 字符串在同一会话下一次改了编辑区或设置的调用前有效。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_session_candidate_info(
    session: *mut FlypeSession,
    index: usize,
    out: *mut FlypeCandidateInfo,
) -> c_int {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return -1;
    };
    if out.is_null() {
        return -1;
    }
    let Some(c) = handle.c_candidates().get(index) else {
        return -1;
    };
    unsafe { write_sized(out, c.info()) };
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::keysym;

    /// 拍一个没有修饰键的按下，返回 frame 借用（会话活着时有效）
    fn press(session: *mut FlypeSession, keysym: u32) -> &'static FlypeFrame {
        let key = FlypeKey {
            size: size_of::<FlypeKey>() as u32,
            keysym,
            ..FlypeKey::default()
        };
        unsafe { &*flype_session_key(session, &key) }
    }

    /// C 字符串转 String，顺带断言不是 NULL
    fn c(ptr: *const c_char) -> String {
        assert!(!ptr.is_null());
        unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned()
    }

    /// 没引擎也能敲 nihao，退格改编辑区，复位把原始输入当上屏交回来
    #[test]
    fn roundtrip() {
        let session = unsafe { flype_session_new(ptr::null_mut(), ptr::null()) };
        assert!(!session.is_null());
        for ch in "nihaoma".bytes() {
            assert_eq!(press(session, u32::from(ch)).handled, 1);
        }
        assert_eq!(c(unsafe { flype_session_remaining(session) }), "nihaoma");
        // 没有引擎：没有候选，选不了
        assert_eq!(unsafe { flype_session_candidate_total(session) }, 0);
        assert_eq!(unsafe { &*flype_session_choose(session, 0) }.handled, 0);

        let frame = press(session, keysym::BACKSPACE);
        assert_eq!(frame.handled, 1);
        assert!(frame.commit.is_null());
        assert_eq!(c(frame.preedit), "nihaom");
        assert_eq!(frame.size as usize, size_of::<FlypeFrame>());

        let frame = unsafe { &*flype_session_reset(session, 1) };
        assert_eq!(c(frame.commit), "nihaom");
        assert_eq!(c(frame.preedit), "");
        unsafe { flype_session_free(session) };
    }

    /// 旧外壳的 FlypeKey 短一截：size 之外的多余字段不读，当 0 处理
    #[test]
    fn short_key_struct_is_zero_extended() {
        // 旧外壳的 FlypeKey 只到 keysym：后面的字段当 0（没有修饰键、按下）
        let session = unsafe { flype_session_new(ptr::null_mut(), ptr::null()) };
        let key = FlypeKey {
            size: 8,
            keysym: u32::from(b'a'),
            modifiers: 0xffff,
            release: 1,
            prev_char: 0,
            action: 0,
            hints: 0,
            code: 0,
            repeat: 0,
            time_ms: 0,
        };
        let frame = unsafe { &*flype_session_key(session, &key) };
        assert_eq!(frame.handled, 1);
        assert_eq!(c(frame.preedit), "a");
        unsafe { flype_session_free(session) };
    }

    /// 旧外壳的配置短一截：没传到的项用默认值（toolbar 开着）而不是 0
    #[test]
    fn short_config_struct_keeps_defaults() {
        // 旧外壳的 FlypeSessionConfig 没有 toolbar 这一项：读出来照默认开着，不是 0
        let mut config = FlypeSessionConfig::default();
        config.size = std::mem::offset_of!(FlypeSessionConfig, toolbar) as u32;
        config.toolbar = 0;
        let read = unsafe { read_sized(&config, config.size) }.to_config();
        assert!(read.toolbar);
    }

    /// 入口收到 NULL 句柄都不崩：返回 NULL 或 0/-1
    #[test]
    fn null_handles() {
        assert!(unsafe { flype_session_key(ptr::null_mut(), ptr::null()) }.is_null());
        assert!(unsafe { flype_session_reset(ptr::null_mut(), 0) }.is_null());
        assert_eq!(
            unsafe { flype_session_candidate_total(ptr::null_mut()) },
            0
        );
        assert!(unsafe { flype_session_choose(ptr::null_mut(), 0) }.is_null());
        assert!(unsafe { flype_session_select(ptr::null_mut(), 0) }.is_null());
        assert!(unsafe { flype_session_candidate(ptr::null_mut(), 0) }.is_null());
        assert_eq!(
            unsafe { flype_session_view(ptr::null_mut(), ptr::null_mut()) },
            -1
        );
        unsafe { flype_session_free(ptr::null_mut()) };
    }
}
