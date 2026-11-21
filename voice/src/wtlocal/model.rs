//! 声学模型：卷积前端 + 40 层 pre-norm Transformer + CTC 输出层；int8 权重直接 mmap 自 xnet
use super::fbank::DIM;
use super::kernels::{dotf, qgemm, quant, QAct};
use super::parallel;
use super::xnet::{self, Rec, F32, I8};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;

/// 隐层维度（每帧特征数与 Transformer 通道数）
const D: usize = 512;
/// 注意力头数
const HEADS: usize = 8;
/// 单个注意力头的通道数（D / HEADS）
const HD: usize = D / HEADS;
/// 前馈层中间维度（D 的 4 倍）
const FF: usize = 2048;
/// Transformer 层数
const LAYERS: usize = 40;
/// 第一层卷积输出通道数
const F1: usize = 32; // 卷积通道
/// 第二层卷积输出通道数
const F2: usize = 64;
/// 第三层卷积输出通道数
const F3: usize = 192;
/// 第二层卷积单个输出的核尺寸（F1 × 7 × 7）
const K2: usize = F1 * 49;
/// 第三层卷积单个输出的核尺寸（F2 × 3 × 3）
const K3: usize = F2 * 9;
/// 第二层卷积后频率维补一列 0（卷积核按这个宽度算）
const W2: usize = DIM + 1;
/// 第三层卷积输出展平后的宽度（F3 × 沿频率下采样到 20 点）
const FLAT: usize = F3 * 20;

/// 一组 int8 量化权重：文件内偏移、输出行数、每行元素数、量化 scale
struct Q {
    /// 权重在模型文件里的字节偏移
    off: usize,
    /// 输出行数
    n: usize,
    /// 每行元素数
    k: usize,
    /// 反量化 scale（int8 乘回 f32 用）
    s: f32,
}

/// 单层 pre-norm Transformer：两个 norm 的参数 + 注意力/前馈的量化权重
struct Layer {
    /// 输入 pre-norm 的 (weight, bias)
    pn: (Vec<f32>, Vec<f32>),
    /// 注意力后、前馈前 pre-norm 的 (weight, bias)
    mn: (Vec<f32>, Vec<f32>),
    /// 注意力 query 投影
    q: Q,
    /// 注意力 key 投影
    k: Q,
    /// 注意力 value 投影
    v: Q,
    /// 注意力输出投影
    o: Q,
    /// 前馈第一层权重
    f1: Q,
    /// 前馈第一层偏置
    f1b: Vec<f32>,
    /// 前馈第二层权重
    f2: Q,
    /// 前馈第二层偏置
    f2b: Vec<f32>,
}

/// 完整声学模型：mmap 的 xnet 字节 + 解析后的卷积核、Transformer 层、输出层与词表
pub struct Model {
    /// 整个模型文件的字节（权重靠偏移直接读）
    map: Vec<u8>,
    /// 第一层卷积核（f32）
    c1: Vec<f32>,
    /// 第二层卷积核（f32）
    c2: Vec<f32>,
    /// 第三层卷积核（f32）
    c3: Vec<f32>,
    /// 卷积输出到隐层的线性投影
    lin: Q,
    /// 投影偏置
    lin_b: Vec<f32>,
    /// 全部 Transformer 层
    layers: Vec<Layer>,
    /// CTC 输出层权重
    out: Q,
    /// CTC 输出层偏置
    out_b: Vec<f32>,
    /// 词表，输出编号就是下标
    vocab: Vec<String>,
}

/// 从 xnet 记录表中按名字查权重并校验形状
struct Loader<'a> {
    /// 模型文件的字节
    d: &'a [u8],
    /// 记录名 → 记录（形状和偏移）
    r: HashMap<String, Rec>,
}

impl Loader<'_> {
    /// 按名字取记录，缺失时报出具体名字
    fn rec(&self, name: &str) -> Result<&Rec> {
        self.r.get(name).with_context(|| format!("模型缺少 {name}"))
    }
    /// 取出 f32 张量并核对 dtype 与形状
    fn f32(&self, name: &str, dims: &[usize]) -> Result<Vec<f32>> {
        let r = self.rec(name)?;
        if r.dtype != F32 || r.dims != dims {
            bail!("{name} 形状不符：{:?}", r.dims);
        }
        Ok(self.d[r.off..r.off + r.len]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect())
    }
    /// 取出 `<name>.weight_quantized_s8` 量化权重，核对 int8 及 [n, k] 形状
    fn q(&self, name: &str, n: usize, k: usize) -> Result<Q> {
        let name = format!("{name}.weight_quantized_s8");
        let r = self.rec(&name)?;
        if r.dtype != I8 || r.dims.len() < 2 || r.dims[..2] != [n, k] || r.len != n * k {
            bail!("{name} 形状不符：{:?}", r.dims);
        }
        Ok(Q {
            off: r.off,
            n,
            k,
            s: r.scale.context("缺少量化参数")?,
        })
    }
    /// 取出一组归一化的 weight/bias（长度均为 D）
    fn norm(&self, name: &str) -> Result<(Vec<f32>, Vec<f32>)> {
        Ok((
            self.f32(&format!("{name}.weight"), &[D])?,
            self.f32(&format!("{name}.bias"), &[D])?,
        ))
    }
}

impl Model {
    /// 在目录里找 .xnet 与词表，逐层加载权重并校验词表大小与输出维度一致
    pub fn load(dir: &Path) -> Result<Model> {
        let xnet = std::fs::read_dir(dir)
            .map_err(|_| anyhow::anyhow!("离线模型未下载"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|e| e == "xnet"))
            .context("模型文件不存在")?;
        let map =
            std::fs::read(&xnet).with_context(|| format!("读取模型失败: {}", xnet.display()))?;
        let l = Loader {
            d: &map,
            r: xnet::parse(&map)?,
        };
        let mut layers = Vec::with_capacity(LAYERS);
        for i in 0..LAYERS {
            let p = |s: &str| format!("layers.{i}.{s}");
            layers.push(Layer {
                pn: l.norm(&p("prior_norm"))?,
                mn: l.norm(&p("middle_norm"))?,
                q: l.q(&p("attention.q_conv"), D, D)?,
                k: l.q(&p("attention.k_conv"), D, D)?,
                v: l.q(&p("attention.v_conv"), D, D)?,
                o: l.q(&p("attention.output_transform_conv"), D, D)?,
                f1: l.q(&p("feed_forward.filter_conv"), FF, D)?,
                f1b: l.f32(&p("feed_forward.filter_conv.bias"), &[FF])?,
                f2: l.q(&p("feed_forward.output_conv"), D, FF)?,
                f2b: l.f32(&p("feed_forward.output_conv.bias"), &[D])?,
            });
        }
        let out = l
            .rec("output_conv.bias")?
            .dims
            .first()
            .copied()
            .unwrap_or(0);
        let vocab: Vec<String> = std::fs::read_to_string(dir.join("dict.decoder.utf8.txt"))
            .context("词表不存在")?
            .lines()
            .map(|s| s.split(' ').nth(1).unwrap_or("").to_string())
            .collect();
        if vocab.len() != out {
            bail!("词表大小 {} 与模型输出 {out} 不符", vocab.len());
        }
        Ok(Model {
            c1: l.f32("frontend.conv2d.weight", &[F1, 1, 5, 5])?,
            c2: l.f32("frontend.conv2d_1.weight", &[F2, F1, 7, 7])?,
            c3: l.f32("frontend.conv2d_2.weight", &[F3, F2, 3, 3])?,
            lin: l.q("frontend.linear", D, FLAT)?,
            lin_b: l.f32("frontend.linear.bias", &[D])?,
            layers,
            out: l.q("output_conv", out, D)?,
            out_b: l.f32("output_conv.bias", &[out])?,
            vocab,
            map,
        })
    }

    /// 用 Q 描述的 int8 权重做一次量化矩阵乘（可选 bias）
    fn mm(&self, x: &QAct, q: &Q, b: Option<&[f32]>) -> Vec<f32> {
        let w = &self.map[q.off..q.off + q.n * q.k];
        let w = unsafe { std::slice::from_raw_parts(w.as_ptr() as *const i8, w.len()) };
        qgemm(x, w, q.n, q.s, b)
    }

    /// 输入归一化特征帧，输出每个 50ms 输出帧的 argmax token id
    pub fn forward(&self, x: &[[f32; DIM]]) -> Vec<u32> {
        self.run(x)
    }

    /// 三层卷积降采样后逐层跑 Transformer，最后一帧 argmax 成 token 序列
    fn run(&self, x: &[[f32; DIM]]) -> Vec<u32> {
        let t = x.len();
        if t < 5 {
            return Vec::new();
        }
        let t1 = (t - 5) / 5 + 1;
        let at = |i: usize, d: isize| -> Option<usize> { i.checked_add_signed(d) };

        // conv 5×5, 1→32, 时频各补 2
        let mut y1 = vec![0f32; t * F1 * DIM];
        parallel::for_each_chunk_mut(&mut y1, F1 * DIM, |ti, y| {
            let mut p = [[0f32; 25]; DIM];
            for (f, pf) in p.iter_mut().enumerate() {
                for i in 0..5 {
                    let Some(tt) = at(ti, i as isize - 2).filter(|&v| v < t) else {
                        continue;
                    };
                    for j in 0..5 {
                        if let Some(ff) = at(f, j as isize - 2).filter(|&v| v < DIM) {
                            pf[i * 5 + j] = x[tt][ff];
                        }
                    }
                }
            }
            for o in 0..F1 {
                let w = &self.c1[o * 25..o * 25 + 25];
                for f in 0..DIM {
                    y[o * DIM + f] = dotf(w, &p[f]).max(0.0);
                }
            }
        });

        // conv 7×7, 32→64, 时间步长 5（时补 1、频补 3）
        let mut y2 = vec![0f32; t1 * F2 * W2];
        parallel::for_each_chunk_mut(&mut y2, F2 * W2, |ti, y| {
            let mut p = vec![0f32; DIM * K2];
            for i in 0..7 {
                let Some(tt) = at(ti * 5 + i, -1).filter(|&v| v < t) else {
                    continue;
                };
                let row = &y1[tt * F1 * DIM..(tt + 1) * F1 * DIM];
                for c in 0..F1 {
                    for f in 0..DIM {
                        for j in 0..7 {
                            if let Some(ff) = at(f + j, -3).filter(|&v| v < DIM) {
                                p[f * K2 + c * 49 + i * 7 + j] = row[c * DIM + ff];
                            }
                        }
                    }
                }
            }
            for o in 0..F2 {
                let w = &self.c2[o * K2..(o + 1) * K2];
                for f in 0..DIM {
                    y[o * W2 + f] = dotf(w, &p[f * K2..(f + 1) * K2]).max(0.0);
                }
            }
        });

        // conv 3×3, 64→192, 频率步长 2（时补 1）→ [t1][192×20]
        let mut y3 = vec![0f32; t1 * FLAT];
        parallel::for_each_chunk_mut(&mut y3, FLAT, |ti, y| {
            let mut p = vec![0f32; 20 * K3];
            for i in 0..3 {
                let Some(tt) = at(ti + i, -1).filter(|&v| v < t1) else {
                    continue;
                };
                let row = &y2[tt * F2 * W2..(tt + 1) * F2 * W2];
                for c in 0..F2 {
                    for f in 0..20 {
                        for j in 0..3 {
                            p[f * K3 + c * 9 + i * 3 + j] = row[c * W2 + 2 * f + j];
                        }
                    }
                }
            }
            for o in 0..F3 {
                let w = &self.c3[o * K3..(o + 1) * K3];
                for f in 0..20 {
                    y[o * 20 + f] = dotf(w, &p[f * K3..(f + 1) * K3]).max(0.0);
                }
            }
        });

        let mut h = self.mm(&quant(&y3, FLAT), &self.lin, Some(&self.lin_b));
        for l in &self.layers {
            let y = quant(&norm(&h, &l.pn), D);
            let (q, k, v) = (
                self.mm(&y, &l.q, None),
                self.mm(&y, &l.k, None),
                self.mm(&y, &l.v, None),
            );
            add(
                &mut h,
                &self.mm(&quant(&attention(&q, &k, &v, t1), D), &l.o, None),
            );
            let mut f = self.mm(&quant(&norm(&h, &l.mn), D), &l.f1, Some(&l.f1b));
            f.iter_mut().for_each(|v| *v = gelu(*v));
            add(&mut h, &self.mm(&quant(&f, FF), &l.f2, Some(&l.f2b)));
        }
        let lo = self.mm(&quant(&h, D), &self.out, Some(&self.out_b));
        lo.chunks(self.out.n)
            .map(|r| {
                r.iter()
                    .enumerate()
                    .fold((0, f32::MIN), |b, (i, &v)| if v > b.1 { (i, v) } else { b })
                    .0 as u32
            })
            .collect()
    }

    /// CTC 折叠：去重、去 blank；英文词首（|xxx）前补空格
    pub fn text(&self, ids: &[u32]) -> String {
        let mut s = String::new();
        let mut prev = u32::MAX;
        for &i in ids {
            if i != prev && i != 0 {
                match self.vocab[i as usize].as_str() {
                    "<UNK>" => {}
                    "<SPACE>" => s.push(' '),
                    t if t.len() > 1 && t.starts_with('|') => {
                        if s.ends_with(|c: char| c.is_ascii_alphanumeric()) {
                            s.push(' ');
                        }
                        s.push_str(&t[1..]);
                    }
                    t => s.push_str(t),
                }
            }
            prev = i;
        }
        s
    }
}

/// 按行（每 D 个元素一帧）做 LayerNorm，再乘 gamma 加 beta
fn norm(x: &[f32], (g, b): &(Vec<f32>, Vec<f32>)) -> Vec<f32> {
    let mut y = x.to_vec();
    parallel::for_each_chunk_mut(&mut y, D, |_, r| {
        let m = r.iter().sum::<f32>() / D as f32;
        let v = r.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / D as f32;
        let inv = 1.0 / (v + 1e-6).sqrt();
        for i in 0..D {
            r[i] = (r[i] - m) * inv * g[i] + b[i];
        }
    });
    y
}

/// 残差相加：h += o
fn add(h: &mut [f32], o: &[f32]) {
    h.iter_mut().zip(o).for_each(|(a, b)| *a += b);
}

/// tanh 近似的 GELU 激活
fn gelu(x: f32) -> f32 {
    0.5 * x * (1.0 + (0.797_884_6 * (x + 0.044715 * x * x * x)).tanh())
}

/// 多头自注意力（全序列、无 mask），缩放因子 1/8 即 1/√HD
fn attention(q: &[f32], k: &[f32], v: &[f32], t: usize) -> Vec<f32> {
    let mut out = vec![0f32; t * D];
    parallel::for_each_chunk_mut(&mut out, D, |i, o| {
        let mut s = vec![0f32; t];
        for h in 0..HEADS {
            let qi = &q[i * D + h * HD..i * D + (h + 1) * HD];
            for (j, sj) in s.iter_mut().enumerate() {
                *sj = dotf(qi, &k[j * D + h * HD..j * D + (h + 1) * HD]) * 0.125;
            }
            let m = s.iter().fold(f32::MIN, |a, &b| a.max(b));
            let mut z = 0.0;
            for sj in s.iter_mut() {
                *sj = (*sj - m).exp();
                z += *sj;
            }
            let oh = &mut o[h * HD..(h + 1) * HD];
            for (j, &a) in s.iter().enumerate() {
                let vj = &v[j * D + h * HD..j * D + (h + 1) * HD];
                for d in 0..HD {
                    oh[d] += a / z * vj[d];
                }
            }
        }
    });
    out
}
