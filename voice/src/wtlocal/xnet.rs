//! 官方 .xnet 模型文件：只读算子（权重/常量）记录表位于文件尾部，按名称索引
use anyhow::{bail, Context, Result};
use std::collections::HashMap;

/// f32 权重的 dtype 编号
pub const F32: u32 = 11;
/// int8 量化权重的 dtype 编号
pub const I8: u32 = 6;

/// 一条算子/常量记录：dtype、可选量化 scale、形状、载荷在文件内的偏移与字节长
pub struct Rec {
    /// dtype 编号（F32 或 I8）
    pub dtype: u32,
    /// int8 权重的量化 scale；f32 权重没有
    pub scale: Option<f32>,
    /// 形状（各维长度）
    pub dims: Vec<usize>,
    /// 载荷在文件里的字节偏移
    pub off: usize,
    /// 载荷字节长
    pub len: usize,
}

/// 游标：在字节切片上顺序读取，越界即返 None
struct Cur<'a> {
    /// 要读的字节
    d: &'a [u8],
    /// 当前读到的位置
    p: usize,
}

impl Cur<'_> {
    /// 读 4 字节小端 u32
    fn u32(&mut self) -> Option<u32> {
        let b = self.d.get(self.p..self.p + 4)?;
        self.p += 4;
        Some(u32::from_le_bytes(b.try_into().unwrap()))
    }
    /// 读 n 字节，长度溢出或不就丢掉则返 None
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let b = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(b)
    }
}

/// dtype 编号到单元素字节数的映射，不认识的返 None
fn elem(dtype: u32) -> Option<usize> {
    match dtype {
        I8 | 2 | 3 => Some(1),
        4 | 5 => Some(2),
        1 | 7 | 8 | F32 => Some(4),
        9 | 10 | 12 => Some(8),
        _ => None,
    }
}

/// 从 p 起逐条解析，必须恰好解析到文件尾（零长度名 + 全零填充）
fn chain(d: &[u8], p: usize) -> Option<HashMap<String, Rec>> {
    let mut c = Cur { d, p };
    let mut out = HashMap::new();
    loop {
        let n = c.u32()? as usize;
        if n == 0 {
            let rest = &d[c.p..];
            return (rest.len() <= 16 && rest.iter().all(|&b| b == 0) && out.len() > 100)
                .then_some(out);
        }
        if n > 256 {
            return None;
        }
        let name = std::str::from_utf8(c.take(n)?).ok()?.to_string();
        let dtype = c.u32()?;
        let es = elem(dtype)?;
        let quant = c.u32()?;
        for _ in 0..3 {
            c.u32()?;
        }
        let ns = c.u32()? as usize;
        c.u32()?;
        if ns > 4096 {
            return None;
        }
        let sc = c.take(ns * 4)?;
        let scale = (quant != 0 && ns > 0).then(|| f32::from_le_bytes(sc[..4].try_into().unwrap()));
        let nz = c.u32()? as usize;
        c.take(nz)?;
        let nd = c.u32()? as usize;
        if nd > 8 {
            return None;
        }
        let dims = (0..nd)
            .map(|_| c.u32().map(|v| v as usize))
            .collect::<Option<Vec<_>>>()?;
        let len = c.u32()? as usize;
        if len != dims.iter().product::<usize>() * es {
            return None;
        }
        let off = c.p;
        c.take(len)?;
        out.insert(
            name,
            Rec {
                dtype,
                scale,
                dims,
                off,
                len,
            },
        );
    }
}

/// 校验文件头后搜索权重表起始位置，能完整解析到文件尾才算命中
pub fn parse(d: &[u8]) -> Result<HashMap<String, Rec>> {
    if d.get(..4) != Some(b"XNET") {
        bail!("不是 xnet 模型文件");
    }
    // 只读记录表的首条名称以 "/" 或字母开头，前缀为 u32 长度；逐个候选尝试完整解析
    let mut p = 8;
    while let Some(i) = d[p..]
        .windows(4)
        .position(|w| w[1..] == [0, 0, 0] && (1..=128).contains(&w[0]))
    {
        let at = p + i;
        let n = d[at] as usize;
        if let Some(name) = d.get(at + 4..at + 4 + n) {
            if name.iter().all(|b| b.is_ascii_graphic()) {
                if let Some(m) = chain(d, at) {
                    return Ok(m);
                }
            }
        }
        p = at + 1;
    }
    None.context("找不到 xnet 权重表")
}
