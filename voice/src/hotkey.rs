//! 语音触发状态机：长按说话 / 点按说话。
//!
//! 语义（对齐主流输入法习惯）：
//! - 按下即开始录音
//! - 松开时按住时长 < hold_threshold(默认 300ms) => 判定为「点按」，
//!   继续录音，直到再次点按或静音超时
//! - 松开时按住时长 >= hold_threshold => 判定为「长按」，立即结束
//! - 同一个键可以同时充当长按键和点按键

use std::time::{Duration, Instant};

/// 录音模式：按住说话或点一下说
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Hold,
    Tap,
}

/// 状态机的输出动作：开始、继续或结束录音
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// 开始录音
    Start,
    /// 继续录音（点按已识别，等待静音或再次点按）
    Continue,
    /// 结束录音
    Stop,
}

/// 触发参数：点按/长按的分界时长与点按模式的静音结束时长
#[derive(Clone, Copy, Debug)]
pub struct TriggerConfig {
    /// 按住超过这个时长算长按（松手才结束）
    pub hold_threshold: Duration,
    /// 点按模式下静音多久自动结束
    pub silence_timeout: Duration,
}

impl Default for TriggerConfig {
    /// 按住 300ms 以内算点按，点按后静音 1.5s 自动结束
    fn default() -> Self {
        Self {
            hold_threshold: Duration::from_millis(300),
            silence_timeout: Duration::from_millis(1500),
        }
    }
}

/// 热键状态机：只看按下/松开/语音活动/时间，输出该开始、继续还是结束录音
#[derive(Debug, Default)]
pub struct VoiceTrigger {
    /// 点按/长按的分界和静音超时
    cfg: TriggerConfig,
    /// 现在是否在录音
    recording: bool,
    /// 本次按下会被判成的模式
    mode: Mode,
    /// 按键按下的时刻，用来分点按还是长按
    pressed_at: Option<Instant>,
    /// 最后一次语音活动的时刻，用来判静音超时
    last_voice: Option<Instant>,
}

impl VoiceTrigger {
    /// 用给定参数建一个空闲状态机
    pub fn new(cfg: TriggerConfig) -> Self {
        Self {
            cfg,
            recording: false,
            mode: Mode::Hold,
            pressed_at: None,
            last_voice: None,
        }
    }

    /// 是否在录音中
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// 当前判定的模式
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// 键按下
    pub fn on_down(&mut self, now: Instant) -> Vec<Action> {
        if self.recording {
            // 点按模式下再次按下 = 提前结束
            if self.mode == Mode::Tap {
                self.reset();
                return vec![Action::Stop];
            }
            return Vec::new();
        }
        self.recording = true;
        self.mode = Mode::Hold;
        self.pressed_at = Some(now);
        self.last_voice = Some(now);
        vec![Action::Start]
    }

    /// 键松开
    pub fn on_up(&mut self, now: Instant) -> Vec<Action> {
        let Some(pressed) = self.pressed_at.take() else {
            return Vec::new();
        };
        if !self.recording {
            return Vec::new();
        }
        let held = now.saturating_duration_since(pressed);
        if held < self.cfg.hold_threshold {
            self.mode = Mode::Tap;
            self.last_voice = Some(now);
            vec![Action::Continue]
        } else {
            self.reset();
            vec![Action::Stop]
        }
    }

    /// 有语音活动（VAD 判定为说话）
    pub fn note_voice(&mut self, now: Instant) {
        if self.recording {
            self.last_voice = Some(now);
        }
    }

    /// 周期调用；点按模式下静音超时则返回 Stop
    pub fn tick(&mut self, now: Instant) -> Option<Action> {
        if !self.recording || self.mode != Mode::Tap {
            return None;
        }
        let last = self.last_voice.unwrap_or(now);
        if now.saturating_duration_since(last) >= self.cfg.silence_timeout {
            self.reset();
            Some(Action::Stop)
        } else {
            None
        }
    }

    /// 清回空闲：退出录音并丢掉所有计时
    fn reset(&mut self) {
        self.recording = false;
        self.mode = Mode::Hold;
        self.pressed_at = None;
        self.last_voice = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的默认参数：300ms 长按阈值、1.5s 静音超时
    fn cfg() -> TriggerConfig {
        TriggerConfig {
            hold_threshold: Duration::from_millis(300),
            silence_timeout: Duration::from_millis(1500),
        }
    }

    /// 验证按得够久时松手立即结束录音
    #[test]
    fn hold_stops_on_release() {
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        assert_eq!(tr.on_down(t0), vec![Action::Start]);
        assert!(tr.is_recording());
        assert_eq!(
            tr.on_up(t0 + Duration::from_millis(600)),
            vec![Action::Stop]
        );
        assert!(!tr.is_recording());
    }

    /// 验证快速松手判为点按，继续录音并切到 Tap 模式
    #[test]
    fn tap_keeps_recording() {
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        assert_eq!(tr.on_down(t0), vec![Action::Start]);
        assert_eq!(
            tr.on_up(t0 + Duration::from_millis(100)),
            vec![Action::Continue]
        );
        assert!(tr.is_recording());
        assert_eq!(tr.mode(), Mode::Tap);
    }

    /// 验证点按模式下静音超时才返回 Stop
    #[test]
    fn tap_stops_on_silence() {
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        tr.on_down(t0);
        tr.on_up(t0 + Duration::from_millis(100));
        assert_eq!(tr.tick(t0 + Duration::from_millis(1000)), None);
        assert_eq!(
            tr.tick(t0 + Duration::from_millis(1700)),
            Some(Action::Stop)
        );
        assert!(!tr.is_recording());
    }

    /// 验证检测到语音会推后静音超时，继续录下去
    #[test]
    fn voice_activity_extends_tap() {
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        tr.on_down(t0);
        tr.on_up(t0 + Duration::from_millis(100));
        tr.note_voice(t0 + Duration::from_millis(1500));
        assert_eq!(tr.tick(t0 + Duration::from_millis(2000)), None);
        assert_eq!(
            tr.tick(t0 + Duration::from_millis(3100)),
            Some(Action::Stop)
        );
    }

    /// 验证点按录音中再按一次立即结束
    #[test]
    fn second_tap_stops_immediately() {
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        tr.on_down(t0);
        tr.on_up(t0 + Duration::from_millis(80));
        assert_eq!(
            tr.on_down(t0 + Duration::from_millis(500)),
            vec![Action::Stop]
        );
        assert!(!tr.is_recording());
    }

    /// 验证长按期间重复按下不会重启录音，也不产生动作
    #[test]
    fn repeated_down_while_holding_is_ignored() {
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        tr.on_down(t0);
        assert!(tr.on_down(t0 + Duration::from_millis(50)).is_empty());
        assert!(tr.is_recording());
    }

    /// 验证没按下就松手是空操作，不会误开录音
    #[test]
    fn up_without_down_is_noop() {
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        assert!(tr.on_up(t0).is_empty());
        assert!(!tr.is_recording());
    }

    /// 验证空闲时 tick 不产生任何动作
    #[test]
    fn tick_when_idle_is_none() {
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        assert_eq!(tr.tick(t0 + Duration::from_secs(10)), None);
    }

    /// 验证长按模式不会被静音超时打断，只靠松手结束
    #[test]
    fn hold_mode_tick_never_stops() {
        // 长按模式靠松手结束，不该被静音超时打断
        let t0 = Instant::now();
        let mut tr = VoiceTrigger::new(cfg());
        tr.on_down(t0);
        assert_eq!(tr.tick(t0 + Duration::from_secs(5)), None);
        assert!(tr.is_recording());
    }
}
