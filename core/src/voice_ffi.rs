//! 语音输入的 C 接口：按住（或点一下）热键说话，松开后把识别结果交给插件上屏。
//!
//! 识别用微信输入法的离线模型（`flype_voice::wtlocal`）。模型不随软件分发，
//! 用户在设置界面（Flype 设置 → 语音）里下载；这里只读，不下载。
//! 没有模型时按热键只给一句提示（VOICE_ERROR + MODEL_MISSING），什么也不做。
//!
//! 线程：
//! - 插件线程只调 `flype_voice_*`，这些函数都不阻塞（不开设备、不解码）；
//! - 每次说话起一个工作线程：开麦克风、分帧、VAD、喂识别、出中间结果，结束时整段定稿；
//! - 模型的加载在单独的线程里，只做一次。说话时模型还没读完，音频先攒着，好了再补喂。
//!
//! 插件定时调 `flype_voice_poll`（几十毫秒一次）取状态与文本。
//!
//! 测试：设了环境变量 `FLYPE_VOICE_WAV` 时不开麦克风，改读这个 WAV 文件当作说的话。

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use flype_voice::asr::AsrChannel;
use flype_voice::audio::{downmix, to_i16, Framer, Resampler, TARGET_RATE};
use flype_voice::hotkey::{Action, Mode, TriggerConfig, VoiceTrigger};
use flype_voice::vad::Vad;
use flype_voice::wtlocal::{self, download, WtLocalChannel};

/// 没在录音。
pub const VOICE_IDLE: c_int = 0;
/// 正在听；文本是中间结果（可能为空）。
pub const VOICE_LISTENING: c_int = 1;
/// 录完了，正在出最终结果。
pub const VOICE_RECOGNIZING: c_int = 2;
/// 最终结果；文本即要上屏的字。只返回一次，之后回到 IDLE。
pub const VOICE_RESULT: c_int = 3;
/// 出错或没听清；文本是给用户看的原因。只返回一次。
pub const VOICE_ERROR: c_int = 4;

/// 模型状态。
pub const MODEL_NONE: c_int = 0;
/// 不再用：模型改在设置界面里下载，这里不会返回它（留着编号免得插件和核心对不上）。
#[allow(dead_code)]
pub const MODEL_DOWNLOADING: c_int = 1;
/// 模型正在读进内存
pub const MODEL_LOADING: c_int = 2;
/// 模型能用了
pub const MODEL_READY: c_int = 3;
/// 载入失败（具体原因不往编号里带）
pub const MODEL_FAILED: c_int = 4;
/// 按了热键，但模型还没下载。
pub const MODEL_MISSING: c_int = 5;

/// 没有模型时按热键显示的提示。
const MISSING_HINT: &str = "没有语音模型，请在「Flype 设置 → 语音」里下载";

/// 单次最长录音。和 SenseVoice 那套一样，误触不会一直录下去。
const MAX_RECORDING: Duration = Duration::from_secs(120);
/// 等模型加载的上限（已下载好、只是还在读进内存）。
const LOAD_WAIT: Duration = Duration::from_secs(60);

/// 没有句柄时 `flype_voice_text` 返回的空串，保证不会给插件 NULL
static EMPTY: [u8; 1] = [0];

/// 模型载入状态，插件看到的是上面的 `MODEL_*` 编号
enum ModelState {
    /// 还没看过磁盘。
    None,
    /// 磁盘上没有模型（没在设置里下载过）。
    Missing,
    /// 载入线程正在读进内存。
    Loading,
    /// 能用了。
    Ready,
    /// 载入失败（原因给用户看）。
    Failed(String),
}

/// 模型状态 + 载入线程的通知：说话前问它模型在不在，没在读就起线程读
struct ModelSlot {
    /// 现在的状态，载入线程写、插件线程读。
    state: Mutex<ModelState>,
    /// 载入线程改完状态通知等的人（wait_ready）。
    changed: Condvar,
}

impl ModelSlot {
/// 状态转成插件认的 `MODEL_*` 编号
    fn code(&self) -> c_int {
        match &*self.state.lock().unwrap() {
            ModelState::None => MODEL_NONE,
            ModelState::Missing => MODEL_MISSING,
            ModelState::Loading => MODEL_LOADING,
            ModelState::Ready => MODEL_READY,
            ModelState::Failed(_) => MODEL_FAILED,
        }
    }

    /// 还没载入（或上次失败）就起线程载入。模型不在磁盘上返回 false，不下载；
    /// 载入的线程起不来也返回 false（状态记成失败，下次按热键再试）。
    /// 返回 false 时给用户看的话见 [`ModelSlot::unavailable_hint`]。
    fn prepare(self: &Arc<Self>, dir: &Path) -> bool {
        {
            let mut state = self.state.lock().unwrap();
            if !matches!(*state, ModelState::None | ModelState::Missing | ModelState::Failed(_)) {
                return true;
            }
            // 每次都看一眼磁盘：设置界面可能刚下载好，也可能刚删掉
            if !download::is_ready(dir) {
                *state = ModelState::Missing;
                return false;
            }
            *state = ModelState::Loading;
        }
        let slot = self.clone();
        let dir = dir.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("flype-voice-model".into())
            .spawn(move || {
                let result = wtlocal::load(&dir).map(|_| ());
                *slot.state.lock().unwrap() = match result {
                    Ok(()) => ModelState::Ready,
                    Err(e) => {
                        eprintln!("flype: 语音模型不可用：{e:#}");
                        ModelState::Failed(format!("{e}"))
                    }
                };
                slot.changed.notify_all();
            });
        if let Err(e) = spawned {
            // 线程数到了上限、内存不够：这里是输入法（C++ / Swift）调进来的，不能 panic——
            // panic 出不了 FFI 边界，整个输入法进程跟着没了
            eprintln!("flype: 开不了载入语音模型的线程：{e}");
            *self.state.lock().unwrap() = ModelState::Failed(format!("开不了载入线程（{e}）"));
            self.changed.notify_all();
            return false;
        }
        true
    }

    /// [`ModelSlot::prepare`] 返回 false 时给用户看的原因：没下载，或者这次没载入成。
    fn unavailable_hint(&self) -> String {
        match &*self.state.lock().unwrap() {
            ModelState::Failed(e) => format!("语音模型不可用：{e}"),
            _ => MISSING_HINT.to_string(),
        }
    }

    /// 等模型读进内存。
    fn wait_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + LOAD_WAIT;
        let mut state = self.state.lock().unwrap();
        loop {
            match &*state {
                ModelState::Ready => return Ok(()),
                ModelState::Failed(e) => return Err(format!("语音模型不可用：{e}")),
                ModelState::None => return Err("语音模型没有准备".into()),
                ModelState::Missing => return Err(MISSING_HINT.into()),
                ModelState::Loading => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err("语音模型加载超时".into());
                    }
                    state = self.changed.wait_timeout(state, left).unwrap().0;
                }
            }
        }
    }

/// 载入好了没有：工作线程每 50ms 看一次，好了才开识别
    fn is_ready(&self) -> bool {
        matches!(*self.state.lock().unwrap(), ModelState::Ready)
    }
}

/// 工作线程与插件线程共享的状态。
struct Shared {
    /// 当前这次说话的编号；旧会话的结果对不上号就丢掉。
    session: u64,
    /// 现在的 VOICE_* 状态。
    phase: c_int,
    /// 中间结果、最终结果，或者给用户看的错误。
    text: String,
    /// 最近一次检测到说话的时间（点按模式靠它判断说完了）。
    voice_at: Option<Instant>,
    /// 最近一块音频的音量，0–100（给插件画波形）。
    level: u8,
}

/// 一块 PCM 的音量：-60 dB（安静房间）到 -10 dB（大声说话）映射到 0–100。
fn level_of(pcm: &[i16]) -> u8 {
    if pcm.is_empty() {
        return 0;
    }
    let energy = pcm
        .iter()
        .map(|&v| (v as f64 / 32768.0).powi(2))
        .sum::<f64>()
        / pcm.len() as f64;
    let db = 10.0 * energy.max(1e-10).log10();
    (((db + 60.0) / 50.0).clamp(0.0, 1.0) * 100.0).round() as u8
}

/// 插件线程发给工作线程的消息
enum Msg {
    /// 一块 16 kHz 单声道音频。
    Pcm(Vec<i16>),
    /// 热键松开了：剩下的算完再定稿。
    Stop,
    /// 这次不要了：丢掉，不定稿。
    Cancel,
}

/// 语音句柄。只在插件线程用。
pub struct FlypeVoice {
    /// 模型目录（默认 ~/.local/share/flype/wtlocal）。
    dir: PathBuf,
    /// 测试用：设了 FLYPE_VOICE_WAV 时读这个文件代替麦克风。
    wav: Option<PathBuf>,
    /// 模型状态和载入线程的通知。
    model: Arc<ModelSlot>,
    /// 和工作线程共享的状态（状态、文本、音量）。
    shared: Arc<Mutex<Shared>>,
    /// 热键状态机（长按 / 点按）。
    trigger: VoiceTrigger,
    /// 这次说话的编号，加一就让旧线程的结果对不上号。
    session: u64,
    /// 录音线程的消息口；没在说话是 None。
    tx: Option<Sender<Msg>>,
    /// 上一次 poll 的文本（flype_voice_text 返回的指针）。
    text: CString,
}

impl FlypeVoice {
/// 建句柄：只记下模型目录（测试时还有 WAV），不碰麦克风、不读模型
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            wav: std::env::var_os("FLYPE_VOICE_WAV").map(PathBuf::from),
            model: Arc::new(ModelSlot {
                state: Mutex::new(ModelState::None),
                changed: Condvar::new(),
            }),
            shared: Arc::new(Mutex::new(Shared {
                session: 0,
                phase: VOICE_IDLE,
                text: String::new(),
                voice_at: None,
                level: 0,
            })),
            trigger: VoiceTrigger::new(TriggerConfig::default()),
            session: 0,
            tx: None,
            text: CString::default(),
        }
    }

/// 把热键状态机给的动作用顺序做掉
    fn apply(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Start => self.start(),
                Action::Stop => self.stop(),
                Action::Continue => {}
            }
        }
    }

/// 开一次说话：先取消上一次；模型不在就只提示不开麦克风，否则起工作线程录音
    fn start(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Cancel);
        }
        if !self.model.prepare(&self.dir) {
            // 没有模型：只提示去设置里下载，不开麦克风、不下载。
            // （载入的线程起不来也走这里，提示的是那个原因）
            self.session += 1;
            {
                let mut shared = self.shared.lock().unwrap();
                shared.session = self.session;
                shared.voice_at = None;
                shared.level = 0;
            }
            let hint = self.model.unavailable_hint();
            self.fail(&hint);
            return;
        }
        self.session += 1;
        {
            let mut shared = self.shared.lock().unwrap();
            shared.session = self.session;
            shared.phase = VOICE_LISTENING;
            shared.text.clear();
            shared.voice_at = Some(Instant::now());
            shared.level = 0;
        }
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            session: self.session,
            dir: self.dir.clone(),
            wav: self.wav.clone(),
            model: self.model.clone(),
            shared: self.shared.clone(),
            tx: tx.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("flype-voice".into())
            .spawn(move || worker.run(rx));
        if spawned.is_err() {
            self.fail("开不了录音线程");
            return;
        }
        self.tx = Some(tx);
    }

/// 让工作线程收尾（松开热键、说完了）：剩下的音频算完再定稿
    fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Stop);
        }
    }

/// 丢掉这次结果：线程收到 Cancel，热键状态机复位，编号加一让旧结果对不上号
    fn cancel(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Cancel);
        }
        self.trigger = VoiceTrigger::new(TriggerConfig::default());
        self.session += 1;
        let mut shared = self.shared.lock().unwrap();
        shared.session = self.session;
        shared.phase = VOICE_IDLE;
        shared.text.clear();
    }

/// 把要告诉用户的错误写进共享状态，等插件 poll 到 VOICE_ERROR
    fn fail(&mut self, message: &str) {
        let mut shared = self.shared.lock().unwrap();
        shared.phase = VOICE_ERROR;
        shared.text = message.to_string();
    }

/// 推一次热键状态机并取状态：结果和错误只报一次，报完就落回 IDLE
    fn poll(&mut self) -> c_int {
        let now = Instant::now();
        let (phase, text, voice_at) = {
            let shared = self.shared.lock().unwrap();
            (shared.phase, shared.text.clone(), shared.voice_at)
        };
        if let Some(at) = voice_at {
            self.trigger.note_voice(at);
        }
        if let Some(action) = self.trigger.tick(now) {
            self.apply(vec![action]);
        }
        // 工作线程自己停了（到了时长上限、开不了麦克风）：热键状态机跟着复位
        if phase != VOICE_LISTENING && self.trigger.is_recording() {
            self.trigger = VoiceTrigger::new(TriggerConfig::default());
            self.tx = None;
        }
        if phase == VOICE_RESULT || phase == VOICE_ERROR {
            let mut shared = self.shared.lock().unwrap();
            if shared.phase == phase {
                shared.phase = VOICE_IDLE;
            }
        }
        self.text = CString::new(text.replace('\0', "")).unwrap_or_default();
        phase
    }
}

/// 一次说话的录音线程：收音频、喂 VAD 和识别、出中间结果，停了以后整段定稿
struct Worker {
    /// 属于哪一次说话（对不上号就不写共享状态）。
    session: u64,
    /// 模型目录。
    dir: PathBuf,
    /// 测试用的 WAV，没有就开麦克风。
    wav: Option<PathBuf>,
    /// 模型状态（每 50 ms 看一眼读好了没有）。
    model: Arc<ModelSlot>,
    /// 和插件线程共享的状态。
    shared: Arc<Mutex<Shared>>,
    /// 往自己的 channel 送音频（麦克风回调、WAV 都走它）。
    tx: Sender<Msg>,
}

/// 麦克风（或测试用的 WAV）。丢弃即停。
enum Source {
    /// 麦克风（丢弃即停）。
    #[allow(dead_code)]
    Mic(flype_voice::capture::Capture),
    /// 测试用的 WAV，已经一次性全送进 channel 了。
    Wav,
}

impl Worker {
    /// 改共享状态；这次说话已经被新的一次顶掉（或取消）就什么也不做。
    fn update(&self, f: impl FnOnce(&mut Shared)) {
        let mut shared = self.shared.lock().unwrap();
        if shared.session == self.session {
            f(&mut shared);
        }
    }

/// 开音源：测试 WAV 一次性全送进来，否则开麦克风把音频转发到 channel
    fn open(&self) -> anyhow::Result<Source> {
        if let Some(path) = &self.wav {
            let (rate, channels, pcm) = flype_voice::read_wav(path)?;
            let mono: Vec<f32> = downmix(
                &pcm.iter().map(|&v| v as f32 / 32768.0).collect::<Vec<_>>(),
                channels,
            );
            let pcm = to_i16(&Resampler::new(rate, TARGET_RATE).process(&mono));
            for chunk in pcm.chunks(1600) {
                let _ = self.tx.send(Msg::Pcm(chunk.to_vec()));
            }
            return Ok(Source::Wav);
        }
        let tx = self.tx.clone();
        let capture = flype_voice::capture::start(None, move |pcm| {
            let _ = tx.send(Msg::Pcm(pcm));
        })?;
        Ok(Source::Mic(capture))
    }

/// 收到停为止：模型好了就建识别通道，每批音频报一次音量、刷中间结果，最后定稿
    fn run(self, rx: Receiver<Msg>) {
        let source = match self.open() {
            Ok(source) => source,
            Err(e) => {
                eprintln!("flype: 开麦克风失败：{e:#}");
                self.update(|s| {
                    s.phase = VOICE_ERROR;
                    s.text = "开不了麦克风".into();
                });
                return;
            }
        };
        let started = Instant::now();
        let mut framer = Framer::default();
        let mut vad = Vad::default();
        let mut pending: Vec<i16> = Vec::new();
        let mut asr: Option<WtLocalChannel> = None;
        let mut shown = String::new();

        let stopped = loop {
            if started.elapsed() >= MAX_RECORDING {
                break true;
            }
            let msg = rx.recv_timeout(Duration::from_millis(50));
            if asr.is_none() && self.model.is_ready() {
                match WtLocalChannel::new(&self.dir) {
                    Ok(mut ch) => {
                        ch.feed(&pending);
                        pending = Vec::new();
                        asr = Some(ch);
                    }
                    Err(e) => {
                        self.update(|s| {
                            s.phase = VOICE_ERROR;
                            s.text = format!("语音模型不可用：{e}");
                        });
                        return;
                    }
                }
            }
            let mut batch: Vec<Vec<i16>> = Vec::new();
            let mut end: Option<bool> = None;
            match msg {
                Ok(Msg::Pcm(pcm)) => batch.push(pcm),
                Ok(Msg::Stop) => end = Some(true),
                Ok(Msg::Cancel) | Err(RecvTimeoutError::Disconnected) => end = Some(false),
                Err(RecvTimeoutError::Timeout) => {}
            }
            // 算中间结果的时候麦克风还在送音频：先把积压的都收下，再用最新的音频算一次，
            // 中间结果就不会越来越落后
            while end.is_none() {
                match rx.try_recv() {
                    Ok(Msg::Pcm(pcm)) => batch.push(pcm),
                    Ok(Msg::Stop) => end = Some(true),
                    Ok(Msg::Cancel) | Err(TryRecvError::Disconnected) => end = Some(false),
                    Err(TryRecvError::Empty) => break,
                }
            }
            for pcm in &batch {
                let level = level_of(pcm);
                self.update(|s| s.level = level);
                let mut voiced = false;
                for frame in framer.push(pcm) {
                    voiced |= vad.push(&frame);
                }
                if voiced {
                    self.update(|s| s.voice_at = Some(Instant::now()));
                }
                match asr.as_mut() {
                    Some(ch) => ch.push(pcm),
                    None => pending.extend_from_slice(pcm),
                }
            }
            if let Some(stopped) = end {
                break stopped;
            }
            if let Some(ch) = asr.as_mut() {
                if ch.refresh() {
                    if let Some(text) = ch.partial() {
                        if text != shown {
                            shown = text.to_string();
                            self.update(|s| s.text = shown.clone());
                        }
                    }
                }
            }
        };
        drop(source);
        if !stopped {
            return;
        }
        // 停麦克风之前已经送出来的音频也要算上
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Pcm(pcm) => match asr.as_mut() {
                    Some(ch) => ch.push(&pcm),
                    None => pending.extend_from_slice(&pcm),
                },
                Msg::Cancel => return,
                Msg::Stop => {}
            }
        }
        self.update(|s| s.phase = VOICE_RECOGNIZING);

        let mut asr = match asr {
            Some(ch) => ch,
            None => {
                let ready = self
                    .model
                    .wait_ready()
                    .and_then(|()| WtLocalChannel::new(&self.dir).map_err(|e| e.to_string()));
                match ready {
                    Ok(mut ch) => {
                        ch.feed(&pending);
                        ch
                    }
                    Err(e) => {
                        self.update(|s| {
                            s.phase = VOICE_ERROR;
                            s.text = e;
                        });
                        return;
                    }
                }
            }
        };
        let text = asr.finish().unwrap_or_default();
        self.update(|s| {
            if text.trim().is_empty() {
                s.phase = VOICE_ERROR;
                s.text = "没听清".into();
            } else {
                s.phase = VOICE_RESULT;
                s.text = text;
            }
        });
    }
}

/// 创建语音句柄；model_dir 为 NULL/空串时用默认目录（~/.local/share/flype/wtlocal）。
/// 不碰麦克风、不读模型，第一次按热键时才读；从来不下载（模型在设置界面里下载）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_new(model_dir: *const c_char) -> *mut FlypeVoice {
    let dir = if model_dir.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(model_dir) }
            .to_str()
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    };
    let dir = dir.unwrap_or_else(download::default_dir);
    Box::into_raw(Box::new(FlypeVoice::new(dir)))
}

/// 释放句柄；在说话的话先取消（工作线程自己收）+ 什么也不返回
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_free(voice: *mut FlypeVoice) {
    if !voice.is_null() {
        let mut voice = unsafe { Box::from_raw(voice) };
        voice.cancel();
    }
}

/// 热键按下。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_key_down(voice: *mut FlypeVoice) {
    if let Some(voice) = unsafe { voice.as_mut() } {
        let actions = voice.trigger.on_down(Instant::now());
        voice.apply(actions);
    }
}

/// 热键松开。按住超过 300ms 算长按，松开即结束；更短算点按，继续听到说完（静音 1.5 秒）或再按一次。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_key_up(voice: *mut FlypeVoice) {
    if let Some(voice) = unsafe { voice.as_mut() } {
        let actions = voice.trigger.on_up(Instant::now());
        voice.apply(actions);
    }
}

/// 不要这次的结果（按了 Esc、切走了输入框）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_cancel(voice: *mut FlypeVoice) {
    if let Some(voice) = unsafe { voice.as_mut() } {
        voice.cancel();
    }
}

/// 定时调用：推进状态机（点按模式的静音结束），返回 VOICE_* 状态。
/// 文本用 flype_voice_text 取。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_poll(voice: *mut FlypeVoice) -> c_int {
    match unsafe { voice.as_mut() } {
        Some(voice) => voice.poll(),
        None => VOICE_IDLE,
    }
}

/// 上一次 poll 对应的文本；下一次 poll 或释放前有效。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_text(voice: *const FlypeVoice) -> *const c_char {
    match unsafe { voice.as_ref() } {
        Some(voice) => voice.text.as_ptr(),
        None => EMPTY.as_ptr() as *const c_char,
    }
}

/// 正在听时最近的音量，0–100。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_level(voice: *const FlypeVoice) -> c_int {
    match unsafe { voice.as_ref() } {
        Some(voice) => c_int::from(voice.shared.lock().unwrap().level),
        None => 0,
    }
}

/// 模型状态，MODEL_*。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_model_state(voice: *const FlypeVoice) -> c_int {
    match unsafe { voice.as_ref() } {
        Some(voice) => voice.model.code(),
        None => MODEL_NONE,
    }
}

/// 是否点按模式（松开热键后还在听）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flype_voice_is_tap(voice: *const FlypeVoice) -> c_int {
    match unsafe { voice.as_ref() } {
        Some(voice) => (voice.trigger.is_recording() && voice.trigger.mode() == Mode::Tap) as c_int,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没按热键一直是 IDLE（模型也没准备），cancel 后仍 IDLE、状态机复位
    #[test]
    fn idle_until_key_and_cancel_resets() {
        let dir = std::env::temp_dir().join(format!("flype-voice-none-{}", std::process::id()));
        let mut voice = FlypeVoice::new(dir);
        assert_eq!(voice.poll(), VOICE_IDLE);
        assert_eq!(voice.model.code(), MODEL_NONE);
        voice.cancel();
        assert_eq!(voice.poll(), VOICE_IDLE);
        assert!(!voice.trigger.is_recording());
    }

    /// 没模型按热键只给一句去设置的提示，不开录音线程，也不往磁盘写任何东西
    #[test]
    fn key_without_model_hints_and_downloads_nothing() {
        let root = std::env::temp_dir().join(format!("flype-nomodel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.join("wtlocal");
        let mut voice = FlypeVoice::new(dir.clone());
        let actions = voice.trigger.on_down(Instant::now());
        voice.apply(actions);
        assert!(voice.tx.is_none(), "没有模型不该开录音线程");
        assert_eq!(voice.poll(), VOICE_ERROR);
        assert!(voice.text.to_str().unwrap().contains("设置"));
        assert_eq!(voice.model.code(), MODEL_MISSING);
        assert!(!voice.trigger.is_recording());
        let actions = voice.trigger.on_up(Instant::now());
        voice.apply(actions);
        assert_eq!(voice.poll(), VOICE_IDLE);
        // 什么也没下载：目录里没有模型、没有 .apk / .part
        assert!(!dir.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 静音是 0，大声说话 70 以上，小声音落在 0–40
    #[test]
    fn level_maps_silence_and_speech() {
        assert_eq!(level_of(&[0; 320]), 0);
        assert!(level_of(&[8000, -8000].repeat(160)) > 70);
        let quiet = level_of(&[60, -60].repeat(160));
        assert!(quiet > 0 && quiet < 40, "{quiet}");
    }

    /// 没下载和载入线程起不来分开提示，后者不能再叫用户去下载
    #[test]
    fn unavailable_hint_tells_missing_from_failed() {
        let dir = std::env::temp_dir().join(format!("flype-voice-hint-{}", std::process::id()));
        let voice = FlypeVoice::new(dir);
        *voice.model.state.lock().unwrap() = ModelState::Missing;
        assert_eq!(voice.model.unavailable_hint(), MISSING_HINT);
        // 载入的线程起不来时 prepare 记下的状态：提示原因，不是「去下载」
        *voice.model.state.lock().unwrap() = ModelState::Failed("开不了载入线程（EAGAIN）".into());
        assert_eq!(voice.model.code(), MODEL_FAILED);
        let hint = voice.model.unavailable_hint();
        assert!(hint.contains("开不了载入线程") && !hint.contains("下载"), "{hint}");
    }
}
