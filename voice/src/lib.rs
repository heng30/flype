//! 语音输入核心（与平台解耦）。
//!
//! 链路：热键状态机 -> 采集 -> 重采样/分帧 -> VAD -> ASR 渠道 -> 上屏
//!
//! 平台相关部分（全局热键、麦克风、按键注入）放在独立模块并由上层组装，
//! 这样这部分逻辑可以在任何系统上单测。

pub mod asr;
pub mod audio;
pub mod capture;
pub mod hotkey;
pub mod sink;
pub mod typer;
pub mod vad;
pub mod wav;
pub mod wtlocal;

pub use asr::{AsrChannel, MultiChannel, Transcript};
pub use audio::{CaptureFormat, Framer, Resampler, FRAME_SAMPLES, TARGET_RATE};
pub use hotkey::{Action, Mode, TriggerConfig, VoiceTrigger};
pub use sink::{ClipboardSink, CommandSink, StdoutSink, TextSink};
pub use typer::Typer;
pub use vad::Vad;
pub use wav::{read_wav, write_wav};

/// 一次语音会话的编排器：把热键动作、音频、识别、上屏串起来。
pub struct VoiceSession {
    /// 热键状态机：收按键、语音活动和时间，给出开始/继续/结束
    trigger: VoiceTrigger,
    /// 把采集到的任意长度采样切成一帧帧
    framer: Framer,
    /// 判断还在不在说话，驱动点按模式的静音自动结束
    vad: Vad,
    /// 多个识别渠道并跑
    asr: MultiChannel,
    /// 最终文本的落点
    sink: Box<dyn TextSink>,
    /// 边说边上屏的指针
    typer: Typer,
    /// 上一次上屏的中间结果，用它的公共前缀算增量
    last_partial: String,
    /// 屏幕上现在有没有本会话打的字（结束时要先清掉）
    on_screen: bool,
}

impl VoiceSession {
    /// 按配置装好状态机、分帧器、VAD 和上屏通道；`speak_as_you_type` 打开边说边上屏
    pub fn new(
        cfg: TriggerConfig,
        asr: MultiChannel,
        sink: Box<dyn TextSink>,
        speak_as_you_type: bool,
    ) -> Self {
        Self {
            trigger: VoiceTrigger::new(cfg),
            framer: Framer::default(),
            // 100ms 起始确认、480ms 挂起；只决定点按模式何时自动结束，音频照样全送识别
            vad: Vad::default(),
            asr,
            sink,
            typer: Typer::new(),
            last_partial: String::new(),
            on_screen: speak_as_you_type,
        }
    }

    /// 键按下：返回状态机动作
    pub fn on_key_down(&mut self, now: std::time::Instant) -> Vec<Action> {
        self.trigger.on_down(now)
    }

    /// 键松开：返回状态机动作
    pub fn on_key_up(&mut self, now: std::time::Instant) -> Vec<Action> {
        self.trigger.on_up(now)
    }

    /// 周期驱动：点按模式静音超时会返回 Stop
    pub fn tick(&mut self, now: std::time::Instant) -> Option<Action> {
        self.trigger.tick(now)
    }

    /// 当前是否在录音（录音期间才收音频）
    pub fn is_recording(&self) -> bool {
        self.trigger.is_recording()
    }

    /// 推入采集到的 16kHz PCM
    pub fn push_pcm(&mut self, pcm: &[i16], now: std::time::Instant) {
        if !self.trigger.is_recording() {
            return;
        }
        for frame in self.framer.push(pcm) {
            if self.vad.push(&frame) {
                self.trigger.note_voice(now);
            }
            self.asr.feed(&frame);
        }
        if self.on_screen {
            let partial = self
                .asr
                .partials()
                .first()
                .map(|(_, t)| t.to_string())
                .unwrap_or_default();
            if partial != self.last_partial {
                let update = self.typer.update(&partial);
                self.sink.apply(update);
                self.last_partial = partial;
            }
        }
    }

    /// 结束会话并把最终文本上屏
    pub fn finish(&mut self) -> Vec<Transcript> {
        if let Some(tail) = self.framer.flush() {
            self.asr.feed(&tail);
        }
        let results = self.asr.finish();
        if let Some(best) = results.first() {
            let update = self.typer.update(&best.text);
            self.sink.apply(update);
        }
        self.typer.finish();
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// 验证整条链路：按下、推音频、结束，取回 mock 渠道的最终结果
    #[test]
    fn session_runs_end_to_end() {
        let t0 = Instant::now();
        let mut s = VoiceSession::new(
            TriggerConfig::default(),
            MultiChannel::new(vec![Box::new(asr::MockChannel::new("mock"))]),
            Box::new(StdoutSink),
            false,
        );
        let key = |_: ()| ();
        key(());
        s.trigger.on_down(t0);
        s.push_pcm(&vec![0i16; 1600], t0 + Duration::from_millis(200));
        let out = s.finish();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "mock:1600");
    }

    // —— VAD 只影响点按模式「什么时候自动结束」，从不丢音频 ——

    /// 有声帧幅度，远高于 VAD 阈值
    const LOUD: i16 = 8000;
    /// 静音帧幅度
    const QUIET: i16 = 0;
    /// 一帧的时长，与 FRAME_SAMPLES 对应
    const FRAME: Duration = Duration::from_millis(20);

    /// 建一个只用 mock 渠道、不上屏的会话
    fn mock_session() -> VoiceSession {
        VoiceSession::new(
            TriggerConfig::default(),
            MultiChannel::new(vec![Box::new(asr::MockChannel::new("mock"))]),
            Box::new(StdoutSink),
            false,
        )
    }

    /// 从 `t` 起每 20ms 推一帧（`script` 之后一直补静音，最多 10 秒），每帧后 tick 一次。
    /// 返回自动结束时距 `t` 的毫秒数（没结束就是 None），以及送进识别的采样数。
    fn drive(s: &mut VoiceSession, t: Instant, script: &[i16]) -> (Option<u64>, usize) {
        let mut fed = 0;
        for i in 0..500 {
            let now = t + FRAME * i as u32;
            let amp = script.get(i).copied().unwrap_or(QUIET);
            s.push_pcm(&[amp; FRAME_SAMPLES], now);
            fed += FRAME_SAMPLES;
            if s.tick(now) == Some(Action::Stop) {
                return (Some(i as u64 * 20), fed);
            }
        }
        (None, fed)
    }

    /// 把 (帧数, 幅度) 展开成逐帧幅度脚本
    fn frames(spec: &[(usize, i16)]) -> Vec<i16> {
        spec.iter().flat_map(|&(n, amp)| vec![amp; n]).collect()
    }

    /// 结束会话，从 mock 结果里读回送进识别的采样数
    fn samples_heard(s: &mut VoiceSession) -> usize {
        let out = s.finish();
        out[0].text.trim_start_matches("mock:").parse().unwrap()
    }

    /// 点按后先点一下键（tap），返回松开的时刻
    fn tap(s: &mut VoiceSession, t0: Instant) -> Instant {
        assert_eq!(s.on_key_down(t0), vec![Action::Start]);
        let up = t0 + Duration::from_millis(100);
        assert_eq!(s.on_key_up(up), vec![Action::Continue]);
        up
    }

    /// 验证点按后短暂静音再说话能续命，且整段音频都进识别
    #[test]
    fn tap_short_word_keeps_listening() {
        // 松开 1 秒后说一个 160ms 的「好」：必须续命，且整段音频都进识别
        let mut s = mock_session();
        let up = tap(&mut s, Instant::now());
        let script = frames(&[(50, QUIET), (8, LOUD)]);
        let (stop, fed) = drive(&mut s, up, &script);
        // 最后一个有声帧在 1140ms，挂起 480ms 后最后一次续命在 1620ms，再等 1.5s
        assert_eq!(stop, Some(3120));
        assert_eq!(samples_heard(&mut s), fed);
    }

    /// 验证字中塞音的短停顿不会被当成静音，仍算说话
    #[test]
    fn tap_word_with_plosive_gap_keeps_listening() {
        // 「不对」这类字中间有 40ms 塞音停顿：3 有声 + 2 静音 + 4 有声，只有 7 帧有声也要算说话
        let mut s = mock_session();
        let up = tap(&mut s, Instant::now());
        let script = frames(&[(50, QUIET), (3, LOUD), (2, QUIET), (4, LOUD)]);
        let (stop, _) = drive(&mut s, up, &script);
        // 第 9 帧（1160ms）才攒够确认，挂起到 1640ms，再等 1.5s
        assert_eq!(stop, Some(3140));
    }

    /// 验证临近静音超时才开口也不会在说话中途被截断
    #[test]
    fn tap_late_onset_is_not_cut() {
        // 松开 1.38 秒才开口（离 1.5 秒静音结束只剩 120ms），说 600ms：不能在说话中途结束
        let mut s = mock_session();
        let up = tap(&mut s, Instant::now());
        let script = frames(&[(69, QUIET), (30, LOUD)]);
        let (stop, _) = drive(&mut s, up, &script);
        let speech_end = (69 + 30) * 20;
        assert!(stop.unwrap() > speech_end, "说话中途就结束了：{stop:?}");
    }

    /// 验证敲键声攒不够起始确认，不续命但仍照常送进识别
    #[test]
    fn tap_key_click_does_not_extend() {
        // 松开 0.5 秒后一下 40ms 的敲键声：不算说话，仍在松开后 1.5 秒结束
        let mut s = mock_session();
        let up = tap(&mut s, Instant::now());
        let script = frames(&[(25, QUIET), (2, LOUD)]);
        let (stop, fed) = drive(&mut s, up, &script);
        assert_eq!(stop, Some(1500));
        // 敲键声和静音照样送进识别，有没有字由识别器决定
        assert_eq!(samples_heard(&mut s), fed);
    }

    /// 验证点按模式在最后一个有声帧后挂起 + 静音超时才结束
    #[test]
    fn tap_ends_about_2s_after_last_voice() {
        // 连续说 1 秒：最后一个有声帧在 980ms，挂起 480ms + 静音 1.5s => 2960ms 结束
        let mut s = mock_session();
        let up = tap(&mut s, Instant::now());
        let script = frames(&[(50, LOUD)]);
        let (stop, _) = drive(&mut s, up, &script);
        assert_eq!(stop, Some(980 + 480 + 1500));
    }

    /// 验证长按模式不看 VAD，tick 从不停，松手才结束且音频全进识别
    #[test]
    fn hold_short_word_reaches_asr() {
        // 长按模式不看 VAD：一个 160ms 的短词照样整段送进识别，松手才结束
        let t0 = Instant::now();
        let mut s = mock_session();
        s.on_key_down(t0);
        let script = frames(&[(5, QUIET), (8, LOUD), (12, QUIET)]);
        let mut fed = 0;
        for (i, &amp) in script.iter().enumerate() {
            let now = t0 + FRAME * i as u32;
            s.push_pcm(&[amp; FRAME_SAMPLES], now);
            fed += FRAME_SAMPLES;
            assert_eq!(s.tick(now), None);
        }
        assert_eq!(
            s.on_key_up(t0 + Duration::from_millis(500)),
            vec![Action::Stop]
        );
        assert_eq!(samples_heard(&mut s), fed);
    }
}
