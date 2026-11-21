//! 识别渠道抽象。
//!
//! 一个渠道可以是云端流式（WebSocket/HTTP）也可以是本地引擎（sherpa-onnx、
//! whisper.cpp 等）。上层只关心：喂音频、取中间结果、取最终结果。

/// 一次识别结果
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    /// 出这个结果的渠道名
    pub channel: String,
    /// 识别出来的文字（已去掉 [BLANK_AUDIO] 之类的标记）
    pub text: String,
    /// 从喂音频到出结果花了多久（目前还没测，恒为 0）
    pub latency_ms: u64,
    /// 是不是最终结果（中间结果不进候选）
    pub is_final: bool,
}

/// 识别渠道：上层只喂音频、取中间结果和最终结果，不关心是云端还是本地引擎
pub trait AsrChannel: Send {
    /// 渠道名，用于标识候选来源
    fn name(&self) -> &str;
    /// 喂入 16kHz 单声道 PCM
    fn feed(&mut self, pcm: &[i16]);
    /// 当前中间结果（边说边出字）
    fn partial(&self) -> Option<&str> {
        None
    }
    /// 结束并取最终结果
    fn finish(&mut self) -> Option<String>;
}

/// 多渠道并跑：任一渠道出最终结果即可参与候选
pub struct MultiChannel {
    /// 并跑的渠道，按加入顺序出候选
    channels: Vec<Box<dyn AsrChannel>>,
}

impl MultiChannel {
    /// 用一批渠道组装
    pub fn new(channels: Vec<Box<dyn AsrChannel>>) -> Self {
        Self { channels }
    }

    /// 音频同时喂给所有渠道
    pub fn feed(&mut self, pcm: &[i16]) {
        for c in &mut self.channels {
            c.feed(pcm);
        }
    }

    /// 各渠道当前的中间结果（渠道名 + 文本），没出字的渠道跳过
    pub fn partials(&self) -> Vec<(&str, &str)> {
        self.channels
            .iter()
            .filter_map(|c| c.partial().map(|p| (c.name(), p)))
            .collect()
    }

    /// 逐个渠道结束，把非空的最终文本收成候选（latency_ms 目前恒为 0）
    pub fn finish(&mut self) -> Vec<Transcript> {
        let mut out = Vec::new();
        for c in &mut self.channels {
            if let Some(text) = c.finish() {
                if !text.is_empty() {
                    out.push(Transcript {
                        channel: c.name().to_string(),
                        text,
                        latency_ms: 0,
                        is_final: true,
                    });
                }
            }
        }
        out
    }
}

/// 测试/占位用渠道：把收到的音频长度当作「识别结果」
#[derive(Debug, Default)]
pub struct MockChannel {
    /// 结果里带的渠道名
    name: String,
    /// 累计收到的采样数，结果里拿它冒充识别文本
    samples: usize,
}

impl MockChannel {
    /// 建一个指定名字的占位渠道
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            samples: 0,
        }
    }
}

impl AsrChannel for MockChannel {
    /// 返回构造时给的名字
    fn name(&self) -> &str {
        &self.name
    }
    /// 只累加收到的采样数，不做真识别
    fn feed(&mut self, pcm: &[i16]) {
        self.samples += pcm.len();
    }
    /// 把名字和累计采样数拼成结果，用于验证链路
    fn finish(&mut self) -> Option<String> {
        Some(format!("{}:{}", self.name, self.samples))
    }
}

/// 外部命令式识别渠道：把整段音频写成 WAV，交给命令行识别器，读 stdout。
///
/// 这样在 Linux 上不用编译任何原生绑定就能接 whisper.cpp / sherpa-onnx：
///
/// ```text
/// whisper-cli -m ggml-large-v3-turbo.bin -l zh -nt -f
/// sherpa-onnx-offline --tokens=... --paraformer=... --wav=
/// ```
///
/// 命令末尾会自动追加 WAV 路径。
pub struct ProcessChannel {
    /// 结果里带的渠道名
    name: String,
    /// 已拆好的命令行（第一项是程序，后面是参数）
    program: Vec<String>,
    /// 攒着的整段音频，finish 时写成 WAV 交给外部程序
    pcm: Vec<i16>,
}

/// 把命令行拆成参数，支持单/双引号和反斜杠转义。
///
/// 不能直接用 split_whitespace：模型路径里常有空格（如
/// "/home/me/My Models/ggml.bin"，按空格切会把它拆坏。
fn split_command(cmdline: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has_token = false;
    let mut chars = cmdline.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else if ch == '\\' && q == '"' {
                    if let Some(next) = chars.next() {
                        cur.push(next);
                    }
                } else {
                    cur.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    has_token = true;
                }
                c if c.is_whitespace() => {
                    if has_token || !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                        has_token = false;
                    }
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        cur.push(next);
                        has_token = true;
                    }
                }
                c => {
                    cur.push(c);
                    has_token = true;
                }
            },
        }
    }
    if has_token || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 命令行里可以用 \`{wav}\` 指定音频路径的位置；没写的话就追加到末尾。
pub const WAV_PLACEHOLDER: &str = "{wav}";

impl ProcessChannel {
    /// `program` 是已拆好的命令行，未写占位符时 WAV 路径会追加到末尾
    pub fn new(name: impl Into<String>, program: Vec<String>) -> Self {
        Self {
            name: name.into(),
            program,
            pcm: Vec::new(),
        }
    }

    /// 从一整行命令行字符串解析出程序与参数
    pub fn from_command_line(name: impl Into<String>, cmdline: &str) -> Self {
        Self::new(name, split_command(cmdline))
    }

    /// 已缓冲的采样数（命令行跑完就被清空）
    pub fn sample_count(&self) -> usize {
        self.pcm.len()
    }
}

/// 去掉识别器常见的标记，如 [BLANK_AUDIO]、(音乐)、<|nospeech|>。
fn clean_transcript(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0i32;
    for ch in text.chars() {
        match ch {
            '[' | '(' | '<' => depth += 1,
            ']' | ')' | '>' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

impl AsrChannel for ProcessChannel {
    /// 返回构造时给的名字
    fn name(&self) -> &str {
        &self.name
    }

    /// 先把整段音频攒在内存里，结束前不碰外部命令
    fn feed(&mut self, pcm: &[i16]) {
        self.pcm.extend_from_slice(pcm);
    }

    /// 写成临时 WAV、替换 `{wav}` 占位符或追加路径、读 stdout（空则回退 stderr）后删文件
    fn finish(&mut self) -> Option<String> {
        if self.pcm.is_empty() || self.program.is_empty() {
            return None;
        }
        use std::sync::atomic::{AtomicUsize, Ordering};
        /// 临时 WAV 文件的序号，避免同一进程里并发识别撞名
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "flype-asr-{}-{}.wav",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        if crate::wav::write_wav(&path, &self.pcm).is_err() {
            return None;
        }
        let wav_path = path.to_string_lossy().to_string();
        let mut argv = self.program.clone();
        let mut replaced = false;
        for arg in argv.iter_mut() {
            if arg.contains(WAV_PLACEHOLDER) {
                *arg = arg.replace(WAV_PLACEHOLDER, &wav_path);
                replaced = true;
            }
        }
        if !replaced {
            argv.push(wav_path);
        }
        let (program, args) = argv.split_first()?;
        let output = std::process::Command::new(program).args(args).output().ok();
        let _ = std::fs::remove_file(&path);
        self.pcm.clear();
        let output = output?;
        if !output.status.success() {
            return None;
        }
        let mut text = clean_transcript(&String::from_utf8_lossy(&output.stdout));
        if text.is_empty() {
            // 有些识别器把结果打到 stderr
            text = clean_transcript(&String::from_utf8_lossy(&output.stderr));
        }
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证外部命令渠道能把 WAV 路径交给识别器并读回 stdout
    #[test]
    fn process_channel_runs_command() {
        // sh -c 'echo 你好'：WAV 路径会作为 $0 被忽略
        let mut ch =
            ProcessChannel::new("echo", vec!["sh".into(), "-c".into(), "echo 你好".into()]);
        ch.feed(&[1i16; 320]);
        let text = ch.finish().expect("应当有结果");
        assert_eq!(text, "你好");
    }

    /// 验证命令行拆分支持引号包裹带空格的模型路径
    #[test]
    fn split_command_handles_quotes() {
        assert_eq!(
            split_command(r#"sh -c "echo 你好 世界""#),
            vec!["sh", "-c", "echo 你好 世界"]
        );
        assert_eq!(
            split_command("whisper-cli -m '/tmp/My Models/ggml.bin' -l zh"),
            vec!["whisper-cli", "-m", "/tmp/My Models/ggml.bin", "-l", "zh"]
        );
    }

    /// 验证 `{wav}` 占位符会被替换成真实音频路径
    #[test]
    fn wav_placeholder_is_substituted() {
        // 识别器需要 --wav=路径 这种形式时，用占位符指定位置
        let mut ch =
            ProcessChannel::from_command_line("probe", r#"sh -c "test -f {wav} && echo ok""#);
        ch.feed(&[1i16; 160]);
        assert_eq!(ch.finish().as_deref(), Some("ok"));
    }

    /// 验证识别器把结果打到 stderr 时也能取到文本
    #[test]
    fn falls_back_to_stderr() {
        let mut ch = ProcessChannel::new(
            "err",
            vec!["sh".into(), "-c".into(), "echo 你好 1>&2".into()],
        );
        ch.feed(&[1i16; 160]);
        assert_eq!(ch.finish().as_deref(), Some("你好"));
    }

    /// 验证清洗会去掉 [BLANK_AUDIO]、(音乐) 这类标记
    #[test]
    fn clean_strips_tags() {
        assert_eq!(clean_transcript("[BLANK_AUDIO]你好(音乐)"), "你好");
    }

    /// 验证多渠道并跑时每个渠道都拿到全量音频并各出一份结果
    #[test]
    fn multi_channel_collects_results() {
        let mut m = MultiChannel::new(vec![
            Box::new(MockChannel::new("a")),
            Box::new(MockChannel::new("b")),
        ]);
        m.feed(&[0i16; 320]);
        m.feed(&[0i16; 320]);
        let out = m.finish();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "a:640");
        assert_eq!(out[1].text, "b:640");
    }
}
