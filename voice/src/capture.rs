//! 麦克风采集（cpal），跨平台：Linux 走 ALSA/PulseAudio，macOS 走 CoreAudio。
//!
//! 回调里直接把设备格式转成 16kHz 单声道 i16 再吐出去，上层不用关心采样格式。

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::audio::{downmix, to_i16, Resampler, TARGET_RATE};

/// 采集信息（用于日志和调试）。
#[derive(Clone, Debug)]
pub struct CaptureInfo {
    /// 设备名
    pub device: String,
    /// 设备原生采样率（回调里会重采样到 16kHz）
    pub sample_rate: u32,
    /// 设备原生声道数
    pub channels: u16,
    /// 设备的采样格式（F32 / I16 / U16 …）
    pub format: String,
}

/// 持有 stream 的句柄；丢弃即停止采集。
pub struct Capture {
    /// cpal 的流，丢弃就停止采集
    _stream: cpal::Stream,
    /// 这次采集用的设备和格式，只用于日志
    pub info: CaptureInfo,
}

/// 列出可用的录音设备名。
pub fn list_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    for dev in host.input_devices().context("枚举录音设备失败")? {
        names.push(dev.to_string());
    }
    Ok(names)
}

/// 按名字挑设备，找不到或没指定时回落默认输入设备
fn pick_device(host: &cpal::Host, want: Option<&str>) -> Result<cpal::Device> {
    if let Some(want) = want {
        let mut devices: Vec<cpal::Device> =
            host.input_devices().context("枚举录音设备失败")?.collect();
        if let Some(pos) = devices
            .iter()
            .position(|d| d.to_string() == want)
        {
            return Ok(devices.remove(pos));
        }
        eprintln!("找不到设备 {want:?}，改用默认设备");
    }
    host.default_input_device().context("没有可用的录音设备")
}

/// 开始采集；回调收到的是 16kHz 单声道 i16。
pub fn start<F>(device: Option<&str>, on_pcm: F) -> Result<Capture>
where
    F: FnMut(Vec<i16>) + Send + 'static,
{
    let host = cpal::default_host();
    let dev = pick_device(&host, device)?;
    // 设备能直接给 16kHz 就要 16kHz（PipeWire/PulseAudio 自己重采样，质量比我们好），
    // 声道越少越好；不行再用设备默认格式，自己滤波重采样
    let wanted = TARGET_RATE;
    let preferred = dev.supported_input_configs().ok().and_then(|configs| {
        configs
            .filter(|c| {
                c.min_sample_rate() <= wanted
                    && wanted <= c.max_sample_rate()
                    && matches!(
                        c.sample_format(),
                        cpal::SampleFormat::F32 | cpal::SampleFormat::I16
                    )
            })
            .min_by_key(|c| (c.channels(), c.sample_format() != cpal::SampleFormat::F32))
            .map(|c| c.with_sample_rate(wanted))
    });
    let supported = match preferred {
        Some(config) => config,
        None => dev.default_input_config().context("读取录音配置失败")?,
    };
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels;
    let rate = config.sample_rate;
    let info = CaptureInfo {
        device: dev.to_string(),
        sample_rate: rate,
        channels,
        format: format!("{format:?}"),
    };

    // 三个分支都要用同一个回调，用 Arc<Mutex> 共享；每次回调只会在一个线程里跑。
    let sink = Arc::new(Mutex::new(on_pcm));
    let err_fn = |e| eprintln!("录音流错误: {e}");

    let stream = match format {
        cpal::SampleFormat::F32 => {
            let mut rs = Resampler::new(rate, TARGET_RATE);
            let sink = sink.clone();
            dev.build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mono = downmix(data, channels);
                    let res = rs.process(&mono);
                    if let Ok(mut f) = sink.lock() {
                        f(to_i16(&res));
                    }
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::I16 => {
            let mut rs = Resampler::new(rate, TARGET_RATE);
            let sink = sink.clone();
            dev.build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data.iter().map(|&v| v as f32 / 32768.0).collect();
                    let mono = downmix(&f, channels);
                    let res = rs.process(&mono);
                    if let Ok(mut f) = sink.lock() {
                        f(to_i16(&res));
                    }
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::U16 => {
            let mut rs = Resampler::new(rate, TARGET_RATE);
            let sink = sink.clone();
            dev.build_input_stream(
                config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|&v| (v as f32 - 32768.0) / 32768.0)
                        .collect();
                    let mono = downmix(&f, channels);
                    let res = rs.process(&mono);
                    if let Ok(mut f) = sink.lock() {
                        f(to_i16(&res));
                    }
                },
                err_fn,
                None,
            )?
        }
        other => return Err(anyhow!("不支持的采样格式: {other:?}")),
    };

    stream.play().context("启动录音流失败")?;
    Ok(Capture {
        _stream: stream,
        info,
    })
}
