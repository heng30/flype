//! 微信输入法离线识别：加载官方模型包（`.xnet`），纯 Rust 推理，不依赖官方程序。
//!
//! 来源：J3n5en/voicekey 的 `crates/wtlocal`，经作者同意后引入并适配（见仓库 README 的引用）。
//! 适配点：
//! - 去掉 rayon / memmap2 / zip / tokio，改为标准库并行、整文件读入、系统 `unzip`
//! - 引擎接口改为本项目的 [`AsrChannel`]，同步喂 PCM、同步取结果

mod fbank;
mod kernels;
mod model;
pub mod pack;
mod parallel;
mod stream;
mod xnet;

pub mod download;

pub use model::Model;
pub use stream::{FinalMode, Stabilizer, Stream};

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::asr::AsrChannel;

/// 中间结果的最小间隔（特征帧，10ms/帧）；同时要求距上次计算耗时的 3 倍，
/// 慢机器上自动降低刷新频率，避免解码把录音线程拖死。
const PARTIAL_EVERY: usize = 30;

/// 最近一次加载的模型（目录 + 模型），换目录时重新解析
static CACHE: Mutex<Option<(PathBuf, Arc<Model>)>> = Mutex::new(None);

/// 加载（并缓存）模型。同一目录重复调用只解析一次。
pub fn load(dir: &Path) -> Result<Arc<Model>> {
    let mut cache = CACHE.lock().unwrap();
    if let Some((cached, model)) = cache.as_ref() {
        if cached == dir {
            return Ok(model.clone());
        }
    }
    let model = Arc::new(Model::load(dir)?);
    *cache = Some((dir.to_path_buf(), model.clone()));
    Ok(model)
}

/// 离线识别渠道：喂 16kHz 单声道 PCM，结束时取文本。
pub struct WtLocalChannel {
    /// 流式解码器
    stream: Stream,
    /// 中间结果的去抖器
    stabilizer: Stabilizer,
    /// 上一次的识别文本，用来判断有没有变化
    last: String,
    /// 上一次解码完成的时刻
    computed: Instant,
    /// 上一次解码花了多久
    cost: Duration,
}

impl WtLocalChannel {
    /// 加载（或复用缓存的）模型，流和去抖器从空状态开始
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self> {
        let model = load(&dir.into())?;
        Ok(Self {
            stream: Stream::new(model),
            stabilizer: Stabilizer::default(),
            last: String::new(),
            computed: Instant::now(),
            cost: Duration::ZERO,
        })
    }

    /// 只收音频、不算中间结果（录音线程积压时先把音频收完，再用 [`Self::refresh`] 算一次）。
    pub fn push(&mut self, pcm: &[i16]) {
        self.stream.push(pcm);
    }

    /// 新音频够 [`PARTIAL_EVERY`] 帧就重算中间结果；返回中间结果是否变了。
    ///
    /// 和 `feed` 不同，这里不按上次耗时限频：调用方只在音频收完、没有积压时调，
    /// 每次都用最新的音频，中间结果不会越拖越晚。
    pub fn refresh(&mut self) -> bool {
        if self.stream.fresh() < PARTIAL_EVERY {
            return false;
        }
        let text = self.stabilizer.update(&self.stream.partial());
        if !text.is_empty() && text != self.last {
            self.last = text;
            return true;
        }
        false
    }
}

impl AsrChannel for WtLocalChannel {
    /// 渠道名，配置里用这个名字选到本引擎
    fn name(&self) -> &str {
        "wtlocal"
    }

    /// 喂 PCM 并抽空算中间结果：新帧够数且距上次耗时 3 倍以上才算
    fn feed(&mut self, pcm: &[i16]) {
        self.stream.push(pcm);
        if self.stream.fresh() >= PARTIAL_EVERY && self.computed.elapsed() >= self.cost * 3 {
            let start = Instant::now();
            let text = self.stabilizer.update(&self.stream.partial());
            self.cost = start.elapsed();
            self.computed = Instant::now();
            if !text.is_empty() && text != self.last {
                self.last = text;
            }
        }
    }

    /// 当前中间结果，还没出过字时返回 None
    fn partial(&self) -> Option<&str> {
        if self.last.is_empty() {
            None
        } else {
            Some(self.last.as_str())
        }
    }

    /// 结束录音取定稿文本，空结果返回 None
    fn finish(&mut self) -> Option<String> {
        let text = self.stream.finish();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}
