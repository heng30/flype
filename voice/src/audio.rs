//! 音频格式、重采样、分帧。
//!
//! 引擎侧（与主流语音识别一致）统一使用 16kHz 单声道 16bit，20ms 一帧。

/// 引擎统一的目标采样率 16kHz
pub const TARGET_RATE: u32 = 16_000;
/// 一帧的时长（ms），与 VAD / 分帧器一致
pub const FRAME_MS: u32 = 20;
/// 一帧的采样数（16kHz 下 20ms = 320）
pub const FRAME_SAMPLES: usize = (TARGET_RATE as usize * FRAME_MS as usize) / 1000;

/// 采集参数
#[derive(Clone, Copy, Debug)]
pub struct CaptureFormat {
    /// 设备原生采样率（内部会重采样到 TARGET_RATE）
    pub sample_rate: u32,
    /// 设备原生声道数（多声道先混成单声道）
    pub channels: u16,
}

impl Default for CaptureFormat {
    /// 默认按 48kHz 单声道处理，常见的声卡原生格式
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 1,
        }
    }
}

/// 重采样：降采样时先过一个低通滤波（加窗 sinc），再线性插值；无外部依赖。
///
/// 不滤波直接插值的话，44.1k/48k 录音里 8kHz 以上的成分（齿音、风扇声）会折叠进
/// 语音频段，识别会变差。
#[derive(Debug)]
pub struct Resampler {
    /// 输入采样率
    from: u32,
    /// 输出采样率
    to: u32,
    /// 上一个输入样本，用于跨界插值
    prev: f32,
    /// 小数相位，范围 [0,1)
    pos: f32,
    /// 是否已经收到第一个样本（第一个只记下来，不输出）
    started: bool,
    /// 降采样时的低通；升采样时不用滤波
    lowpass: Option<Lowpass>,
}

impl Resampler {
    /// 建重采样器；降采样时顺带配一个截止在目标奈奎斯特 90% 的低通
    pub fn new(from: u32, to: u32) -> Self {
        assert!(from > 0 && to > 0);
        // 截止频率取目标奈奎斯特频率的 90%
        let lowpass = (from > to).then(|| Lowpass::new(0.45 * to as f32 / from as f32, 63));
        Self {
            from,
            to,
            prev: 0.0,
            pos: 0.0,
            started: false,
            lowpass,
        }
    }

    /// 输入输出采样率相同，可以直通不滤波
    pub fn is_identity(&self) -> bool {
        self.from == self.to
    }

    /// 输入 f32 采样，输出重采样后的 f32；长度约为 len*to/from
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.is_identity() {
            return input.to_vec();
        }
        let filtered;
        let input = match self.lowpass.as_mut() {
            Some(lp) => {
                filtered = lp.process(input);
                &filtered[..]
            }
            None => input,
        };
        let step = self.to as f32 / self.from as f32;
        let mut out = Vec::with_capacity((input.len() as f32 * step) as usize + 2);
        for &s in input {
            if !self.started {
                self.prev = s;
                self.started = true;
                continue;
            }
            // 在当前区间 [prev, s] 内按相位取样
            while self.pos < 1.0 {
                let v = self.prev + (s - self.prev) * self.pos;
                out.push(v);
                self.pos += 1.0 / step;
            }
            self.pos -= 1.0;
            self.prev = s;
        }
        out
    }
}

/// FIR 低通（Blackman 窗 sinc），跨调用保留历史样本。
#[derive(Debug)]
struct Lowpass {
    /// FIR 系数（已归一化）
    taps: Vec<f32>,
    /// 上一次剩下的尾部样本，跨块卷积用
    history: Vec<f32>,
}

impl Lowpass {
    /// cutoff：截止频率 / 采样率（0–0.5）。
    fn new(cutoff: f32, len: usize) -> Self {
        let m = (len - 1) as f32;
        let mut taps: Vec<f32> = (0..len)
            .map(|i| {
                let x = i as f32 - m / 2.0;
                let sinc = if x == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
                };
                let t = 2.0 * std::f32::consts::PI * i as f32 / m;
                sinc * (0.42 - 0.5 * t.cos() + 0.08 * (2.0 * t).cos())
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Self {
            history: vec![0.0; len - 1],
            taps,
        }
    }

    /// 卷积一个块的输出，历史样本跨调用保留，所以分块调用结果一致
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let n = self.taps.len();
        self.history.extend_from_slice(input);
        let out = self
            .history
            .windows(n)
            .map(|w| w.iter().zip(&self.taps).map(|(a, b)| a * b).sum())
            .collect();
        let keep = self.history.len() - (n - 1);
        self.history.drain(..keep);
        out
    }
}

/// 把 f32 [-1,1] 转成 i16
pub fn to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect()
}

/// 多声道取平均转单声道
pub fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let ch = channels as usize;
    interleaved
        .chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// 固定帧长切分器
#[derive(Debug, Default)]
pub struct Framer {
    /// 还没凑够一帧的采样
    buf: Vec<i16>,
}

impl Framer {
    /// 推入任意长度采样，每凑够 FRAME_SAMPLES 就切一帧返回，剩下的留到下次
    pub fn push(&mut self, samples: &[i16]) -> Vec<Vec<i16>> {
        self.buf.extend_from_slice(samples);
        let mut frames = Vec::new();
        while self.buf.len() >= FRAME_SAMPLES {
            let rest = self.buf.split_off(FRAME_SAMPLES);
            frames.push(std::mem::replace(&mut self.buf, rest));
        }
        frames
    }

    /// 取出不足一帧的尾巴；已经空了就返回 None
    pub fn flush(&mut self) -> Option<Vec<i16>> {
        if self.buf.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.buf))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证采样率相同时直通，内容原样不改
    #[test]
    fn identity_resample() {
        let mut r = Resampler::new(16000, 16000);
        let out = r.process(&[0.0, 0.5, -0.5]);
        assert_eq!(out, vec![0.0, 0.5, -0.5]);
    }

    /// 验证 48k → 16k 降采样后的样本数成比例
    #[test]
    fn down_sample_48k_to_16k_length() {
        let mut r = Resampler::new(48000, 16000);
        let input: Vec<f32> = (0..4800).map(|i| (i as f32 / 4800.0).sin()).collect();
        let out = r.process(&input);
        assert!((out.len() as i64 - 1600).abs() < 40, "got {}", out.len());
    }

    /// 验证降采样前的低通能压掉折叠的 12kHz 而保留 1kHz，且分块处理与整块一致
    #[test]
    fn downsampling_filters_out_high_frequencies() {
        // 12kHz 正弦在 16k 采样下会折叠成 4kHz；滤波后应几乎没有
        let rate = 48000.0;
        let tone = |hz: f32| -> Vec<f32> {
            (0..48000)
                .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate).sin())
                .collect()
        };
        let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
        let high = Resampler::new(48000, 16000).process(&tone(12000.0));
        let low = Resampler::new(48000, 16000).process(&tone(1000.0));
        assert!(rms(&high[100..]) < 0.01, "12kHz 残留 {}", rms(&high[100..]));
        assert!(
            (rms(&low[100..]) - 0.707).abs() < 0.05,
            "1kHz {}",
            rms(&low[100..])
        );
        // 分块处理和一次处理结果一样
        let input = tone(3000.0);
        let whole = Resampler::new(44100, 16000).process(&input);
        let mut r = Resampler::new(44100, 16000);
        let mut chunked = Vec::new();
        for chunk in input.chunks(441) {
            chunked.extend(r.process(chunk));
        }
        assert_eq!(whole.len(), chunked.len());
        assert!(whole
            .iter()
            .zip(&chunked)
            .all(|(a, b)| (a - b).abs() < 1e-5));
    }

    /// 验证多声道按帧取平均转单声道
    #[test]
    fn downmix_stereo() {
        let out = downmix(&[1.0, 0.0, 1.0, 1.0], 2);
        assert_eq!(out, vec![0.5, 1.0]);
    }

    /// 验证分帧器按 320 采样切帧、尾巴在 flush 取出且只能取一次
    #[test]
    fn framer_splits_320() {
        let mut f = Framer::default();
        let frames = f.push(&vec![0i16; 700]);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len(), FRAME_SAMPLES);
        assert_eq!(f.flush().unwrap().len(), 60);
        assert!(f.flush().is_none());
    }
}
