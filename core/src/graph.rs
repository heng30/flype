//! 音节图：把输入串切成所有合法音节序列（DAG）。
//!
//! 每条边消耗一个音节（全拼）或一个首字母（简拼）。隔音符产生的强制边界
//! 会禁止跨过该位置的边；儿化边会消耗掉尾部的 r，并把音节标记为儿化。

use crate::syllable::{
    is_jianpin_letter, jianpin_digraph, parse, ParsedInput, Syllable, SyllableQuery,
    MAX_SYLLABLE_LEN,
};

/// 建图模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphMode {
    /// 只允许完整音节。
    Full,
    /// 只允许每个音节取首字母。
    Jianpin,
    /// 同时允许两种边（会产生较多路径，默认不用于 Engine）。
    Mixed,
}

/// 建图选项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphOptions {
    /// 允许哪几种边：全拼、纯简拼或两者都有。
    pub mode: GraphMode,
    /// 是否认儿化边（尾部 r 加隔音符后的 r）。
    pub allow_erhua: bool,
    /// 是否加纠错边（按错相邻键、两键按反、多按/漏按一个键）。只对全拼生效。
    pub typo: bool,
    /// 是否给尾部没打完的音节加补全边（nih -> ni + hao）。只对全拼生效。
    pub completion: bool,
}

impl Default for GraphOptions {
    /// 默认全拼、开儿化，不带纠错和补全
    fn default() -> Self {
        Self {
            mode: GraphMode::Full,
            allow_erhua: true,
            typo: false,
            completion: false,
        }
    }
}

/// 一条边。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    /// 起点字母下标。
    pub start: usize,
    /// 终点字母下标（不含）。
    pub end: usize,
    /// 这条边消耗哪个音节。
    pub kind: EdgeKind,
    /// 这条边是纠错得来的（与实际按键差一处）。
    pub typo: bool,
    /// 纠错是「两键按反」（ie / ei、uo / ou），比按错一个键少罚，见 BeamConfig::swap_discount。
    pub swap: bool,
    /// 补全边：音节比实际输入多出的字母数（0 表示不是补全）。
    pub completion: u8,
}

/// 边的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// 全拼边：消耗一个完整音节，erhua 表示这个音节带儿化。
    Full {
        /// 消耗掉的完整音节。
        syllable: Syllable,
        /// 这个音节带不带儿化。
        erhua: bool,
    },
    /// 简拼边：只消耗一个声母，letter 的写法见 [`crate::syllable::JIANPIN_ZH`]。
    Jianpin {
        /// 声母的简拼写法，取值见 JIANPIN_ZH。
        letter: u8,
    },
    /// 跨音节的两键按反（nhiao -> ni hao）：一条边消耗两个音节。
    Pair {
        /// 跨过去的前一个音节。
        first: Syllable,
        /// 跨过去的后一个音节。
        second: Syllable,
    },
}

impl Edge {
    /// 第一个音节的查询（Pair 边还有第二个，见 push_queries）。
    pub fn query(&self) -> SyllableQuery {
        match self.kind {
            EdgeKind::Full { syllable, erhua } => SyllableQuery::Full {
                id: syllable.id,
                erhua,
            },
            EdgeKind::Jianpin { letter } => SyllableQuery::Jianpin { letter },
            EdgeKind::Pair { first, .. } => SyllableQuery::Full {
                id: first.id,
                erhua: false,
            },
        }
    }

    /// 把这条边的所有音节查询追加到 out。
    pub fn push_queries(&self, out: &mut Vec<SyllableQuery>) {
        out.push(self.query());
        if let EdgeKind::Pair { second, .. } = self.kind {
            out.push(SyllableQuery::Full {
                id: second.id,
                erhua: false,
            });
        }
    }

    /// 是不是简拼边
    pub fn is_jianpin(&self) -> bool {
        matches!(self.kind, EdgeKind::Jianpin { .. })
    }

    /// 供日志/未知词兜底使用的文本。
    pub fn display(&self) -> String {
        let mut queries = Vec::with_capacity(2);
        self.push_queries(&mut queries);
        queries
            .into_iter()
            .map(SyllableQuery::display)
            .collect::<Vec<_>>()
            .join("")
    }

    /// 这条边消耗几个音节（Pair 边是两个）
    pub fn syllable_count(&self) -> usize {
        match self.kind {
            EdgeKind::Pair { .. } => 2,
            _ => 1,
        }
    }
}

/// 音节图。
#[derive(Clone, Debug)]
pub struct SyllableGraph {
    /// 建图用的输入（字母串 + 强制边界 + 儿化位置）。
    input: ParsedInput,
    /// edges[pos] 是从 pos 出发的边；节点数是 len + 1。
    edges: Vec<Vec<Edge>>,
    /// 建图模式，束搜索靠它决定要不要罚省略的韵母（见 [`crate::beam::BeamConfig`]）。
    mode: GraphMode,
}

impl SyllableGraph {
    /// 从输入串直接建图。
    pub fn from_str(
        input: &str,
        options: GraphOptions,
    ) -> Result<Self, crate::syllable::ParseError> {
        let parsed = parse(input)?;
        Ok(Self::build(&parsed, options))
    }

    /// 用已解析的输入建图。
    pub fn build(input: &ParsedInput, options: GraphOptions) -> Self {
        let n = input.len();
        let mut edges = vec![Vec::new(); n + 1];

        if matches!(options.mode, GraphMode::Full | GraphMode::Mixed) {
            build_full_edges(input, options.allow_erhua, &mut edges);
        }
        if matches!(options.mode, GraphMode::Jianpin | GraphMode::Mixed) {
            build_jianpin_edges(input, &mut edges);
        }
        if options.typo && matches!(options.mode, GraphMode::Full | GraphMode::Mixed) {
            build_typo_edges(input, &mut edges);
        }
        if options.completion && matches!(options.mode, GraphMode::Full | GraphMode::Mixed) {
            build_completion_edges(input, &mut edges);
        }

        Self {
            input: input.clone(),
            edges,
            mode: options.mode,
        }
    }

    /// 输入长度（节点数 = len + 1）。
    pub fn len(&self) -> usize {
        self.input.len()
    }

    /// 输入是否为空
    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    /// 建图用的模式
    pub fn mode(&self) -> GraphMode {
        self.mode
    }

    /// 建图用的已解析输入
    pub fn input(&self) -> &ParsedInput {
        &self.input
    }

    /// 从 pos 出发的全部边；越界返回空
    pub fn edges_from(&self, pos: usize) -> &[Edge] {
        self.edges.get(pos).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 是否存在从 0 到 n 的完整路径。
    pub fn has_complete_path(&self) -> bool {
        self.has_path(|_| true)
    }

    /// 不靠纠错边能否走完（补全可以）。
    pub fn has_path_without_typo(&self) -> bool {
        self.has_path(|e| !e.typo)
    }

    /// 不靠纠错边、补全边能否走完。
    pub fn has_exact_path(&self) -> bool {
        self.has_path(|e| !e.typo && e.completion == 0)
    }

    /// 从终点反着标可达性：只看 allow 放行的边能否从 0 走到 n
    fn has_path(&self, allow: impl Fn(&Edge) -> bool) -> bool {
        let n = self.input.len();
        if n == 0 {
            return true;
        }
        let mut reachable = vec![false; n + 1];
        reachable[n] = true;
        for pos in (0..n).rev() {
            reachable[pos] = self.edges[pos].iter().any(|e| allow(e) && reachable[e.end]);
        }
        reachable[0]
    }

    /// 枚举全部完整切分（最多 limit 条，避免歧义输入爆炸）。
    pub fn segmentations(&self, limit: usize) -> Vec<Vec<Edge>> {
        let mut out = Vec::new();
        if limit == 0 || !self.has_complete_path() {
            return out;
        }
        let mut path = Vec::new();
        self.dfs(0, &mut path, &mut out, limit);
        out
    }

    /// 深搜枚举从 pos 出发的完整切分，凑到 limit 条就提前收手
    fn dfs(&self, pos: usize, path: &mut Vec<Edge>, out: &mut Vec<Vec<Edge>>, limit: usize) {
        if out.len() >= limit {
            return;
        }
        if pos == self.input.len() {
            out.push(path.clone());
            return;
        }
        for edge in self.edges_from(pos) {
            path.push(*edge);
            self.dfs(edge.end, path, out, limit);
            path.pop();
            if out.len() >= limit {
                return;
            }
        }
    }
}

/// 往起点桶里加边，同一段同一个音节只留第一条（重复时只把对调标记并进去）
fn add_edge(edges: &mut [Vec<Edge>], edge: Edge) {
    let bucket = &mut edges[edge.start];
    // 同一段、同一个音节只留一条：精确边先加，纠错边遇到同样的音节就不再加
    if let Some(e) = bucket
        .iter_mut()
        .find(|e| e.end == edge.end && e.kind == edge.kind)
    {
        if e.typo && edge.typo && edge.swap {
            e.swap = true;
        }
    } else {
        bucket.push(edge);
    }
}

/// 造一条普通全拼边：不纠错、不补全
fn full_edge(start: usize, end: usize, syllable: Syllable, erhua: bool) -> Edge {
    Edge {
        start,
        end,
        kind: EdgeKind::Full { syllable, erhua },
        typo: false,
        swap: false,
        completion: 0,
    }
}

/// 补全边：尾部 [start, n) 是某些音节的真前缀时，连一条到终点的边。
///
/// 一次搜索就覆盖所有补全，不必像以前那样对每个补全串各跑一遍。
fn build_completion_edges(input: &ParsedInput, edges: &mut [Vec<Edge>]) {
    let n = input.len();
    for start in n.saturating_sub(MAX_SYLLABLE_LEN - 1)..n {
        if input.forced_between(start, n) {
            continue;
        }
        let tail = &input.letters[start..n];
        for text in crate::syllable::SYLLABLES {
            let bytes = text.as_bytes();
            if bytes.len() > tail.len() && bytes.starts_with(tail) {
                if let Some(syllable) = Syllable::from_text(text) {
                    let mut edge = full_edge(start, n, syllable, false);
                    edge.completion = (bytes.len() - tail.len()) as u8;
                    add_edge(edges, edge);
                }
            }
        }
    }
}

/// QWERTY 键盘上的相邻键（同排左右 + 上下排斜邻）。
pub fn keyboard_neighbors(letter: u8) -> &'static [u8] {
    match letter {
        b'q' => b"wa",
        b'w' => b"qesa",
        b'e' => b"wrds",
        b'r' => b"etfd",
        b't' => b"rygf",
        b'y' => b"tuhg",
        b'u' => b"yijh",
        b'i' => b"uokj",
        b'o' => b"iplk",
        b'p' => b"ol",
        b'a' => b"sqwz",
        b's' => b"adwezx",
        b'd' => b"sferxc",
        b'f' => b"dgrtcv",
        b'g' => b"fhtyvb",
        b'h' => b"gjyubn",
        b'j' => b"hkuinm",
        b'k' => b"jliom",
        b'l' => b"kop",
        b'z' => b"xas",
        b'x' => b"zcsd",
        b'c' => b"xvdf",
        b'v' => b"cbfg",
        b'b' => b"vngh",
        b'n' => b"bmhj",
        b'm' => b"njk",
        _ => b"",
    }
}

/// 两个键在 QWERTY 键盘上是否相邻
fn is_neighbor(a: u8, b: u8) -> bool {
    keyboard_neighbors(a).contains(&b)
}

/// 字节串转成音节；不是 UTF-8 或不是合法音节返回 None
fn syllable_of(bytes: &[u8]) -> Option<Syllable> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(Syllable::from_text)
}

/// 纠错边：每条边只纠一处。
///
/// - 替换：某个字母换成键盘上的相邻键（wrnti -> wenti）
/// - 对调：音节内相邻两键按反（zhnog -> zhong）
/// - 多按：多出来的那个键和前后某个键相同或相邻（haoo / haio -> hao）
/// - 漏按：补一个字母（zhon -> zhong 由补全负责，这里管中间漏的：zhng -> zhong）
/// - 跨音节对调：nhiao -> ni hao（Pair 边）
fn build_typo_edges(input: &ParsedInput, edges: &mut [Vec<Edge>]) {
    let n = input.len();
    let letters = &input.letters;
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_SYLLABLE_LEN + 1);
    let add = |edges: &mut [Vec<Edge>], start: usize, end: usize, syllable: Syllable| {
        let mut edge = full_edge(start, end, syllable, false);
        edge.typo = true;
        add_edge(edges, edge);
    };

    for start in 0..n {
        let max_end = (start + MAX_SYLLABLE_LEN + 1).min(n);
        for end in (start + 1)..=max_end {
            if input.forced_between(start, end) {
                continue;
            }
            let text = &letters[start..end];
            let len = text.len();

            if len <= MAX_SYLLABLE_LEN {
                // 替换
                for i in 0..len {
                    for &c in keyboard_neighbors(text[i]) {
                        buf.clear();
                        buf.extend_from_slice(text);
                        buf[i] = c;
                        if let Some(s) = syllable_of(&buf) {
                            add(edges, start, end, s);
                        }
                    }
                }
                // 音节内对调
                for i in 0..len.saturating_sub(1) {
                    if text[i] == text[i + 1] {
                        continue;
                    }
                    buf.clear();
                    buf.extend_from_slice(text);
                    buf.swap(i, i + 1);
                    if let Some(s) = syllable_of(&buf) {
                        let mut edge = full_edge(start, end, s, false);
                        edge.typo = true;
                        edge.swap = true;
                        add_edge(edges, edge);
                    }
                }
            }

            // 多按一个键：只删和左右邻居相同或相邻的键，避免把正常字母当成多余
            if len >= 2 {
                for i in 0..len {
                    let c = text[i];
                    let left = (i > 0).then(|| text[i - 1]);
                    let right = (i + 1 < len).then(|| text[i + 1]);
                    let stray = [left, right]
                        .into_iter()
                        .flatten()
                        .any(|o| o == c || is_neighbor(o, c));
                    if !stray {
                        continue;
                    }
                    buf.clear();
                    buf.extend_from_slice(&text[..i]);
                    buf.extend_from_slice(&text[i + 1..]);
                    if let Some(s) = syllable_of(&buf) {
                        add(edges, start, end, s);
                    }
                }
            }

            // 漏按一个键：只补在音节中间（末尾漏的交给补全，开头漏的歧义太大）
            if (2..MAX_SYLLABLE_LEN).contains(&len) {
                for i in 1..len {
                    for c in b'a'..=b'z' {
                        buf.clear();
                        buf.extend_from_slice(&text[..i]);
                        buf.push(c);
                        buf.extend_from_slice(&text[i..]);
                        if let Some(s) = syllable_of(&buf) {
                            add(edges, start, end, s);
                        }
                    }
                }
            }
        }
    }

    // 跨音节对调：交换 k 与 k+1 后，在两者之间切开得到两个音节
    for k in 0..n.saturating_sub(1) {
        if letters[k] == letters[k + 1] || input.forced_between(k, k + 2) {
            continue;
        }
        let mut swapped = letters.clone();
        swapped.swap(k, k + 1);
        let a_min = (k + 1).saturating_sub(MAX_SYLLABLE_LEN);
        for a in a_min..=k {
            if input.forced_between(a, k + 1) {
                continue;
            }
            let Some(first) = syllable_of(&swapped[a..=k]) else {
                continue;
            };
            let b_max = (k + 1 + MAX_SYLLABLE_LEN).min(n);
            for b in (k + 2)..=b_max {
                if input.forced_between(k + 1, b) {
                    continue;
                }
                if let Some(second) = syllable_of(&swapped[k + 1..b]) {
                    add_edge(
                        edges,
                        Edge {
                            start: a,
                            end: b,
                            kind: EdgeKind::Pair { first, second },
                            typo: true,
                            swap: true,
                            completion: 0,
                        },
                    );
                }
            }
        }
    }
}

/// 全拼边：枚举每个子串，是音节就连边；儿化来自隔音符标记或尾部 r
fn build_full_edges(input: &ParsedInput, allow_erhua: bool, edges: &mut [Vec<Edge>]) {
    let n = input.len();
    for start in 0..n {
        let max_end = (start + MAX_SYLLABLE_LEN).min(n);
        for end in (start + 1)..=max_end {
            if input.forced_between(start, end) {
                continue;
            }
            // parse 保证是 ASCII，这里可以安全地做字节切片。
            let text = std::str::from_utf8(&input.letters[start..end]).expect("ASCII 输入");
            if let Some(syllable) = Syllable::from_text(text) {
                add_edge(edges, full_edge(start, end, syllable, false));
                if input.has_erhua(end) {
                    add_edge(edges, full_edge(start, end, syllable, true));
                }
            }

            // 尾部 r 儿化：huar -> hua + 儿化。er 本身是独立音节，不当作 e 的儿化。
            if allow_erhua && text.len() > 1 && text.ends_with('r') && text != "er" {
                let stem = &text[..text.len() - 1];
                if let Some(syllable) = Syllable::from_text(stem) {
                    add_edge(edges, full_edge(start, end, syllable, true));
                }
            }
        }
    }
}

/// 简拼边：每个合法首字母一条单字母边，zh/ch/sh 再加一条两字母边
fn build_jianpin_edges(input: &ParsedInput, edges: &mut [Vec<Edge>]) {
    for (pos, &letter) in input.letters.iter().enumerate() {
        // zh/ch/sh 也可以只打声母：zhch -> 支持（z h 拆开的那条路照样保留）
        if let Some(&next) = input.letters.get(pos + 1) {
            if let Some(digraph) = jianpin_digraph(letter, next) {
                if !input.forced_between(pos, pos + 2) {
                    add_edge(
                        edges,
                        Edge {
                            start: pos,
                            end: pos + 2,
                            kind: EdgeKind::Jianpin { letter: digraph },
                            typo: false,
                            swap: false,
                            completion: 0,
                        },
                    );
                }
            }
        }
        if is_jianpin_letter(letter) {
            add_edge(
                edges,
                Edge {
                    start: pos,
                    end: pos + 1,
                    kind: EdgeKind::Jianpin { letter },
                    typo: false,
                    swap: false,
                    completion: 0,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一张不纠错、不补全的图，供各测试用
    fn graph(text: &str, mode: GraphMode) -> SyllableGraph {
        let parsed = parse(text).unwrap();
        SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode,
                allow_erhua: true,
                typo: false,
                completion: false,
            },
        )
    }

    /// 把一条切分翻成各边的展示文本，方便和期望值比
    fn texts(path: &[Edge]) -> Vec<String> {
        path.iter().map(Edge::display).collect()
    }

    /// 验证 xian 有两种切分：整音节 xian 与 xi+an
    #[test]
    fn xian_has_two_segmentations() {
        let g = graph("xian", GraphMode::Full);
        let segs = g.segmentations(16);
        assert_eq!(
            segs.len(),
            2,
            "xian 应有 xian 与 xi+an 两种切分: {:?}",
            segs
        );
        assert!(segs.iter().any(|p| texts(p) == vec!["xian"]));
        assert!(segs.iter().any(|p| texts(p) == vec!["xi", "an"]));
    }

    /// 验证隔音符掐断跨过它的边，xi'an 只剩 xi+an 一种切分
    #[test]
    fn apostrophe_forces_xi_an() {
        let g = graph("xi'an", GraphMode::Full);
        let segs = g.segmentations(16);
        assert_eq!(segs.len(), 1);
        assert_eq!(texts(&segs[0]), vec!["xi", "an"]);
    }

    /// 验证简拼 zg 切成 z、g 两条单字母边
    #[test]
    fn jianpin_zg() {
        let g = graph("zg", GraphMode::Jianpin);
        assert!(g.has_complete_path());
        let segs = g.segmentations(8);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].len(), 2);
        assert_eq!(segs[0][0].query(), SyllableQuery::Jianpin { letter: b'z' });
        assert_eq!(segs[0][1].query(), SyllableQuery::Jianpin { letter: b'g' });
    }

    /// 验证 zh/ch/sh 既能合成一条两字母边也能拆成两条，隔音符隔开就不能合，混拼也算
    #[test]
    fn jianpin_digraph_initials() {
        // zh 既可以是一个声母，也可以是 z、h 两个
        let g = graph("zhch", GraphMode::Jianpin);
        let segs = g.segmentations(16);
        let zh_ch = vec![
            SyllableQuery::Jianpin { letter: b'Z' },
            SyllableQuery::Jianpin { letter: b'C' },
        ];
        assert!(segs
            .iter()
            .any(|p| p.iter().map(|e| e.query()).collect::<Vec<_>>() == zh_ch));
        assert!(segs.iter().any(|p| p.len() == 4));
        // 隔音符隔开就不算 zh
        let g = graph("z'h", GraphMode::Jianpin);
        assert!(g.segmentations(8).iter().all(|p| p.len() == 2));
        // 混着全拼：zh + chi
        let g = graph("zhchi", GraphMode::Mixed);
        assert!(g
            .segmentations(32)
            .iter()
            .any(|p| p.len() == 2 && p[0].query() == SyllableQuery::Jianpin { letter: b'Z' }));
    }

    /// 验证 a/o/e 这类不能单独作声母的字母不产生简拼边，za 走不完整
    #[test]
    fn jianpin_rejects_non_initial() {
        let g = graph("za", GraphMode::Jianpin);
        assert!(!g.has_complete_path(), "a 不能作简拼首字母");
    }

    /// 验证尾部 r 被当成儿化：huar 出一条消耗 hua 的儿化边
    #[test]
    fn trailing_r_marks_erhua() {
        let g = graph("huar", GraphMode::Full);
        let segs = g.segmentations(16);
        assert!(
            segs.iter()
                .any(|p| p.len() == 1 && texts(p) == vec!["huar"]),
            "huar 应产生 hua 的儿化边: {:?}",
            segs
        );
    }

    /// 验证隔音符后的 r 也标儿化：hua'r 和 huar 出的边一样
    #[test]
    fn apostrophe_r_marks_erhua() {
        let g = graph("hua'r", GraphMode::Full);
        let segs = g.segmentations(16);
        assert!(
            segs.iter()
                .any(|p| p.len() == 1 && texts(p) == vec!["huar"]),
            "hua'r 应产生 hua 的儿化边: {:?}",
            segs
        );
    }

    /// 建一张开了纠错边的图
    fn typo_graph(text: &str) -> SyllableGraph {
        SyllableGraph::build(
            &parse(text).unwrap(),
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: true,
                typo: true,
                completion: false,
            },
        )
    }

    /// 这条输入能否只靠纠错凑出 want 这串音节
    fn has_typo_path(text: &str, want: &[&str]) -> bool {
        let g = typo_graph(text);
        g.segmentations(4096).iter().any(|p| {
            let mut q = Vec::new();
            for e in p {
                e.push_queries(&mut q);
            }
            q.iter().map(|x| x.display()).collect::<Vec<_>>() == want
        })
    }

    /// 验证五类纠错边都能命中：相邻键替换、音节内对调、跨音节对调、多按、漏按
    #[test]
    fn typo_edges_cover_common_mistakes() {
        assert!(has_typo_path("nihso", &["ni", "hao"]), "相邻键替换");
        assert!(has_typo_path("zhnog", &["zhong"]), "音节内对调");
        assert!(has_typo_path("nhiao", &["ni", "hao"]), "跨音节对调");
        assert!(has_typo_path("haoo", &["hao"]), "多按");
        assert!(has_typo_path("zhng", &["zhong"]), "漏按");
    }

    /// 验证纠错边默认不开，开了以后算纠错路径、不算精确路径
    #[test]
    fn typo_edges_are_off_by_default_and_marked() {
        assert!(!graph("nihso", GraphMode::Full).has_complete_path());
        let g = typo_graph("nihso");
        assert!(g.has_complete_path());
        assert!(!g.has_exact_path());
        assert!(typo_graph("nihao").has_exact_path());
    }
}
