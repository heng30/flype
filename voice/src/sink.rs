//! 上屏后端：把识别文本送进当前焦点输入框。

use std::io::Write;

/// 上屏后端：插入文本和退格，边说边上的增量由 `apply` 拆成两步
pub trait TextSink: Send {
    /// 在光标处插入文本
    fn insert(&mut self, text: &str);
    /// 退格指定字符数
    fn backspace(&mut self, count: usize);
    /// 应用边说边上屏的增量
    fn apply(&mut self, update: crate::typer::Update) {
        if update.backspaces > 0 {
            self.backspace(update.backspaces);
        }
        if !update.insert.is_empty() {
            self.insert(&update.insert);
        }
    }
}

/// 打印到 stdout，用于测试和调试
#[derive(Default)]
pub struct StdoutSink;

impl TextSink for StdoutSink {
    /// 直接写到 stdout，退格不可逆所以只打文本
    fn insert(&mut self, text: &str) {
        print!("{}", text);
        let _ = std::io::stdout().flush();
    }
    /// 用 `<BS>` 标记代表退格，方便在终端里看增量
    fn backspace(&mut self, count: usize) {
        for _ in 0..count {
            print!("<BS>");
        }
        let _ = std::io::stdout().flush();
    }
}

/// 通过命令行工具注入按键的通用后端。
///
/// Wayland 上用 wtype（wlroots 系）或 ydotool（uinput，需要守护进程）。
#[derive(Debug)]
pub struct CommandSink {
    /// 打字的命令行（后面追加文字）
    pub type_cmd: Vec<String>,
    /// 按退格的命令行
    pub backspace_cmd: Vec<String>,
}

impl CommandSink {
    /// wtype：Wayland (wlroots)
    pub fn wtype() -> Self {
        Self {
            type_cmd: vec!["wtype".into()],
            backspace_cmd: vec!["wtype".into(), "-k".into(), "BackSpace".into()],
        }
    }

    /// ydotool：需要 ydotool 守护进程 + uinput 权限
    pub fn ydotool() -> Self {
        Self {
            type_cmd: vec!["ydotool".into(), "type".into()],
            backspace_cmd: vec!["ydotool".into(), "key".into(), "14:1".into(), "14:0".into()],
        }
    }

    /// 执行一条命令：`extra` 非空时作为最后一个参数追加
    fn run(mut cmd: Vec<String>, extra: &str) {
        if let Some((bin, args)) = cmd.split_first_mut() {
            let mut command = std::process::Command::new(bin);
            command.args(args.iter());
            // 空串不能作为参数传进去，否则 ydotool key 14:1 14:0 "" 会报错
            if !extra.is_empty() {
                command.arg(extra);
            }
            let _ = command.status();
        }
    }
}

impl TextSink for CommandSink {
    /// 把文本作为参数交给 type 命令，不经过 stdin
    fn insert(&mut self, text: &str) {
        Self::run(self.type_cmd.clone(), text);
    }
    /// 每退一格跑一次 backspace 命令，不带额外参数
    fn backspace(&mut self, count: usize) {
        for _ in 0..count {
            Self::run(self.backspace_cmd.clone(), "");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证剪贴板后端会把文本写进 copy 命令的 stdin，再触发粘贴
    #[test]
    fn clipboard_sink_feeds_copy_command() {
        let path = std::env::temp_dir().join(format!("flype_clip_{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut sink = ClipboardSink {
            copy_cmd: vec![
                "sh".into(),
                "-c".into(),
                format!("cat > {}", path.display()),
            ],
            paste_cmd: vec!["true".into()],
            backspace_cmd: vec!["true".into()],
        };
        sink.insert("你好");
        let written = std::fs::read_to_string(&path).expect("应当写入文件");
        assert_eq!(written, "你好");
        let _ = std::fs::remove_file(&path);
    }

    /// 验证 apply 会先退格再插入，stdout 后端不报错
    #[test]
    fn stdout_sink_applies_update() {
        let mut s = StdoutSink;
        s.apply(crate::typer::Update {
            backspaces: 1,
            insert: "x".into(),
        });
    }
}

/// 剪贴板后端：写入剪贴板后模拟粘贴。
///
/// 中文必须走这条：ydotool 的 type 是通过键码模拟按键的，没有对应的
/// keysym 就发不出汉字；wtype 虽然支持 Unicode，但在非 wlroots 的
/// Wayland 合成器上不能用。剪贴板 + Ctrl+V 是最通用的方案。
#[derive(Debug)]
pub struct ClipboardSink {
    /// 把文字写进剪贴板的命令行
    pub copy_cmd: Vec<String>,
    /// 模拟粘贴键的命令行
    pub paste_cmd: Vec<String>,
    /// 按退格的命令行
    pub backspace_cmd: Vec<String>,
}

impl ClipboardSink {
    /// Wayland：wl-copy + wtype 粘贴
    pub fn wl() -> Self {
        Self {
            copy_cmd: vec!["wl-copy".into()],
            paste_cmd: vec![
                "wtype".into(),
                "-M".into(),
                "ctrl".into(),
                "-k".into(),
                "v".into(),
                "-m".into(),
                "ctrl".into(),
            ],
            backspace_cmd: vec!["wtype".into(), "-k".into(), "BackSpace".into()],
        }
    }

    /// 执行命令并忽略退出码，上屏失败不打断录音流程
    fn run(cmd: &[String]) {
        if let Some((bin, args)) = cmd.split_first() {
            let _ = std::process::Command::new(bin).args(args).status();
        }
    }
}

impl TextSink for ClipboardSink {
    /// 把文本喂给 copy 命令的 stdin，等它退出后再模拟粘贴
    fn insert(&mut self, text: &str) {
        if let Some((bin, args)) = self.copy_cmd.split_first() {
            let child = std::process::Command::new(bin)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .spawn();
            if let Ok(mut child) = child {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = std::io::Write::write_all(stdin, text.as_bytes());
                }
                let _ = child.wait();
            }
        }
        Self::run(&self.paste_cmd);
    }

    /// 逐格模拟退格键
    fn backspace(&mut self, count: usize) {
        for _ in 0..count {
            Self::run(&self.backspace_cmd);
        }
    }
}
