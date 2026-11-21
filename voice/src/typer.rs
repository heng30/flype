//! 边说边上屏：记录已打出的文本，按公共前缀退格后补打差异。
//!
//! 这样识别结果不断修正时，不会重复插入或依赖剪贴板。

/// 边说边上屏的指针：记着已打出的字符，按公共前缀算退格与补打
#[derive(Debug, Default)]
pub struct Typer {
    /// 本会话已经上屏的字符
    typed: Vec<char>,
    /// 是否已经结束（结束后不再更新）
    done: bool,
}

/// 一次更新需要的动作
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Update {
    /// 要先按几次退格
    pub backspaces: usize,
    /// 退格后要插入的文本
    pub insert: String,
}

impl Update {
    /// 没有任何动作（退格数和插入文本都为空）
    pub fn is_empty(&self) -> bool {
        self.backspaces == 0 && self.insert.is_empty()
    }
}

impl Typer {
    /// 从空屏幕开始的新会话
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前屏幕上已打出的文本
    pub fn typed(&self) -> String {
        self.typed.iter().collect()
    }

    /// 锁定最终文本：之后所有 update 都不再上屏
    pub fn finish(&mut self) {
        self.done = true;
    }

    /// 把屏幕上的文本修正为 next，返回需要执行的退格/插入
    pub fn update(&mut self, next: &str) -> Update {
        if self.done {
            return Update::default();
        }
        let nxt: Vec<char> = next.chars().collect();
        let common = self
            .typed
            .iter()
            .zip(nxt.iter())
            .take_while(|(a, b)| a == b)
            .count();
        let update = Update {
            backspaces: self.typed.len() - common,
            insert: nxt[common..].iter().collect(),
        };
        self.typed = nxt;
        update
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证只在末尾追加时退格数为 0，只补打新增部分
    #[test]
    fn append_only() {
        let mut t = Typer::new();
        assert_eq!(
            t.update("你好"),
            Update {
                backspaces: 0,
                insert: "你好".into()
            }
        );
        assert_eq!(
            t.update("你好世界"),
            Update {
                backspaces: 0,
                insert: "世界".into()
            }
        );
    }

    /// 验证识别结果变短时退掉末尾多出来的字
    #[test]
    fn revise_tail() {
        let mut t = Typer::new();
        t.update("你好世界");
        assert_eq!(
            t.update("你好"),
            Update {
                backspaces: 2,
                insert: String::new()
            }
        );
        assert_eq!(t.typed(), "你好");
    }

    /// 验证前缀完全不同时先退光再重打
    #[test]
    fn replace_all() {
        let mut t = Typer::new();
        t.update("今天天气");
        let u = t.update("明天");
        assert_eq!(u.backspaces, 4);
        assert_eq!(u.insert, "明天");
    }

    /// 验证 finish 之后不再产生任何上屏动作
    #[test]
    fn finish_freezes() {
        let mut t = Typer::new();
        t.update("abc");
        t.finish();
        assert!(t.update("abcd").is_empty());
    }
}
