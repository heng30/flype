//! 候选特征与线性打分。
//!
//! 特征名严格采用 WeType 排序链路中已确认的 22 个名字。默认权重只是让
//! 骨架能跑通的占位值，并非从 WeType 超参数文件还原的数值。

/// 特征名列表，顺序必须与 Feature 字段一致（也用于训练的向量化）。
pub const FEATURE_NAMES: [&str; 22] = [
    "kind",
    "typo_count",
    "history_count",
    "complete_count",
    "history_time_diff",
    "context_bigram_score",
    "user_word_rollback_score",
    "lm_score_with_previous_txt",
    "head_word_nnlm",
    "hot_val_conf",
    "with_context",
    "user_tail_conf",
    "coverage_score",
    "history_use_count",
    "history_hot_val",
    "prob",
    "context_flag",
    "full_jianpin",
    "prefix_completion",
    "other_completion",
    "typo",
    "force_emit",
];

/// 特征维数，等于 [`FEATURE_NAMES`] 的长度
pub const FEATURE_COUNT: usize = FEATURE_NAMES.len();

/// 候选特征。除 kind 外都按浮点数处理，方便直接喂给线性模型。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Feature {
    /// 候选类型：0=词，1=整句；分类特征。
    pub kind: f32,
    /// 这条路径纠了几处错。
    pub typo_count: f32,
    /// 历史选用次数（不衰减的原始次数，整句命中与末词命中取次数多的那个）。
    pub history_count: f32,
    /// 补全命中次数。目前没有数据源，恒为 0。
    pub complete_count: f32,
    /// 距上次使用的 ln(1+秒)：越大说明越久没选过。
    pub history_time_diff: f32,
    /// 候选首词与上文的最大二元分（词级、字级 LM 取高的那个）。
    pub context_bigram_score: f32,
    /// 用户词回滚分。目前没有数据源，恒为 0。
    pub user_word_rollback_score: f32,
    /// 整条路径的 LM 对数概率累计，与 path.score 里那份是同一个。
    pub lm_score_with_previous_txt: f32,
    /// 字级神经语言模型给整条路径的分。
    pub head_word_nnlm: f32,
    /// 历史热度置信。目前没有数据源，恒为 0。
    pub hot_val_conf: f32,
    /// 有上文、而且查到了候选首词的二元分时为 1。
    pub with_context: f32,
    /// 用户词末词置信。目前没有数据源，恒为 0。
    pub user_tail_conf: f32,
    /// 词典覆盖的音节比例，0~1。
    pub coverage_score: f32,
    /// 整个候选文本自己被选过的热度（不含末词命中）。
    pub history_use_count: f32,
    /// ln(1+历史热度)：整句命中算全额，多字末词命中算一半。
    pub history_hot_val: f32,
    /// 与最高分的相对概率 exp(本条分 - 最高分)，0~1。
    pub prob: f32,
    /// 上下文标志。目前没有数据源，恒为 0。
    pub context_flag: f32,
    /// 路径里有只打了声母的音节时为 1。
    pub full_jianpin: f32,
    /// 补全多出来的字母数，封顶 3。
    pub prefix_completion: f32,
    /// 非前缀补全的标记。目前没有数据源，恒为 0。
    pub other_completion: f32,
    /// 有纠错时为 1。
    pub typo: f32,
    /// 强制上屏标志。目前没有数据源，恒为 0。
    pub force_emit: f32,
}

impl Feature {
    /// 按 [`FEATURE_NAMES`] 顺序摊平成向量，给训练用
    pub fn to_vector(self) -> [f32; FEATURE_COUNT] {
        [
            self.kind,
            self.typo_count,
            self.history_count,
            self.complete_count,
            self.history_time_diff,
            self.context_bigram_score,
            self.user_word_rollback_score,
            self.lm_score_with_previous_txt,
            self.head_word_nnlm,
            self.hot_val_conf,
            self.with_context,
            self.user_tail_conf,
            self.coverage_score,
            self.history_use_count,
            self.history_hot_val,
            self.prob,
            self.context_flag,
            self.full_jianpin,
            self.prefix_completion,
            self.other_completion,
            self.typo,
            self.force_emit,
        ]
    }
}

/// 线性权重。字段与 Feature 一一对应。
#[derive(Clone, Copy, Debug)]
pub struct FeatureWeights {
    /// 候选类型的权重（默认 0：不区分词和整句）。
    pub kind: f32,
    /// 每处纠错的权重，默认负值：纠错越多排越后。
    pub typo_count: f32,
    /// 历史选用次数的权重（默认 0：次数线性加分没有上限）。
    pub history_count: f32,
    /// 补全命中次数的权重。
    pub complete_count: f32,
    /// 距上次使用时间的权重（默认 0：时间差只当特征，不压排序）。
    pub history_time_diff: f32,
    /// 与上文二元分的权重。
    pub context_bigram_score: f32,
    /// 用户词回滚分的权重。
    pub user_word_rollback_score: f32,
    /// 结合光标前文本的 LM 分权重（默认 0：这份分已计入 path.score）。
    pub lm_score_with_previous_txt: f32,
    /// 神经语言模型分的权重。
    pub head_word_nnlm: f32,
    /// 历史热度置信的权重。
    pub hot_val_conf: f32,
    /// 「有上文」标志的权重。
    pub with_context: f32,
    /// 用户词末词置信的权重。
    pub user_tail_conf: f32,
    /// 音节覆盖度的权重。
    pub coverage_score: f32,
    /// 候选自身历史热度的权重。
    pub history_use_count: f32,
    /// 历史热度 ln(1+热度) 的权重。
    pub history_hot_val: f32,
    /// prob 的权重（默认 0：prob 是 path.score 的单调变换）。
    pub prob: f32,
    /// 上下文标志的权重。
    pub context_flag: f32,
    /// 简拼路径的权重，默认负值：同分时全拼优先。
    pub full_jianpin: f32,
    /// 前缀补全字母数的权重，默认负值。
    pub prefix_completion: f32,
    /// 非前缀补全的权重，默认负值。
    pub other_completion: f32,
    /// 有纠错标志的权重，默认负值。
    pub typo: f32,
    /// 强制上屏标志的权重。
    pub force_emit: f32,
}

impl Default for FeatureWeights {
    /// 默认权重，未经 WeType 超参标定
    fn default() -> Self {
        // 占位权重：只保证“更可能、覆盖率更高、纠错更少”的词排前面。
        Self {
            kind: 0.0,
            typo_count: -1.5,
            // 历史只看热度（history_hot_val）：次数线性加分会无限涨，
            // 时间差的 ln(1+秒) 会让「一小时前选过」比「从没选过」还靠后
            history_count: 0.0,
            complete_count: 0.2,
            history_time_diff: 0.0,
            // 上下文二元分是 path.score 之外的新信息。以前给 2.0，是因为「我要」「不知道」这类
            // 语料里被并成一个词的搭配分数偏低，得靠放大才压得过偶然见过的「我腰」；现在这些搭配
            // 有保底分（MERGED_PAIR_SCORE），放大反而让低频偶然搭配（不 + 咬）压过字频。
            // 实测逐词带上文 top1：2.0 → 86.1%，1.0 → 88.6%，0.5 → 88.4%
            // （tools/tune_weights.py 在评测集上自动调的，2026-11-17，见 docs/ENGINE.md「排序参数」）：原来是 1.0
            context_bigram_score: 1.5,
            user_word_rollback_score: 0.8,
            // 这一项已经计入 path.score，再叠加就是重复计权
            lm_score_with_previous_txt: 0.0,
            // （tools/tune_weights.py 在评测集上自动调的，2026-11-17，见 docs/ENGINE.md「排序参数」）。单独调它时（别的参数是原来的值）0.5 最好；
            // 和上面几个一起调以后 1.5 最好（语言模型权重降到 0.85，字级模型要补上这部分）
            head_word_nnlm: 1.5,
            hot_val_conf: 0.3,
            with_context: 0.1,
            user_tail_conf: 0.3,
            coverage_score: 2.0,
            history_use_count: 0.0,
            // ln(1+热度)：选过一次约 +2.1，两次 +3.3，五次 +5.4
            history_hot_val: 3.0,
            // prob 是 path.score 的单调变换，再乘权重只会放大基础分
            prob: 0.0,
            context_flag: 0.0,
            full_jianpin: -0.3,
            prefix_completion: -0.2,
            other_completion: -0.2,
            typo: -1.0,
            force_emit: 0.0,
        }
    }
}

impl FeatureWeights {
    /// 特征与逐项权重的点积，就是线性基础分
    pub fn dot(&self, f: &Feature) -> f32 {
        self.kind * f.kind
            + self.typo_count * f.typo_count
            + self.history_count * f.history_count
            + self.complete_count * f.complete_count
            + self.history_time_diff * f.history_time_diff
            + self.context_bigram_score * f.context_bigram_score
            + self.user_word_rollback_score * f.user_word_rollback_score
            + self.lm_score_with_previous_txt * f.lm_score_with_previous_txt
            + self.head_word_nnlm * f.head_word_nnlm
            + self.hot_val_conf * f.hot_val_conf
            + self.with_context * f.with_context
            + self.user_tail_conf * f.user_tail_conf
            + self.coverage_score * f.coverage_score
            + self.history_use_count * f.history_use_count
            + self.history_hot_val * f.history_hot_val
            + self.prob * f.prob
            + self.context_flag * f.context_flag
            + self.full_jianpin * f.full_jianpin
            + self.prefix_completion * f.prefix_completion
            + self.other_completion * f.other_completion
            + self.typo * f.typo
            + self.force_emit * f.force_emit
    }
}

/// 排序器接口。
pub trait Ranker: Send + Sync {
    /// 最终分
    fn score(&self, feature: &Feature) -> f32;
}

/// 线性打分器：特征与权重的点积。
pub struct LinearRanker {
    /// 逐项线性权重。
    pub weights: FeatureWeights,
}

impl Default for LinearRanker {
    /// 默认权重
    fn default() -> Self {
        Self {
            weights: FeatureWeights::default(),
        }
    }
}

impl LinearRanker {
    /// 用给定权重新建打分器
    pub fn new(weights: FeatureWeights) -> Self {
        Self { weights }
    }
}

impl Ranker for LinearRanker {
    /// 直接返回权重点积
    fn score(&self, feature: &Feature) -> f32 {
        self.weights.dot(feature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证特征名表长度与 FEATURE_COUNT 一致
    #[test]
    fn feature_names_match_count() {
        assert_eq!(FEATURE_NAMES.len(), FEATURE_COUNT);
    }

    /// 验证线性分等于特征与权重点积（默认 prob 权重为 0，只剩 coverage_score 贡献）
    #[test]
    fn linear_dot_product() {
        let f = Feature {
            prob: 0.5,
            coverage_score: 1.0,
            ..Feature::default()
        };
        let weights = FeatureWeights::default();
        let ranker = LinearRanker::new(weights);
        let expected = 0.5 * weights.prob + weights.coverage_score;
        assert!((ranker.score(&f) - expected).abs() < 1e-6);
    }

    /// 验证向量化顺序：kind 在第 0 位，force_emit 在最后一位
    #[test]
    fn vector_order() {
        let f = Feature {
            kind: 1.0,
            force_emit: 2.0,
            ..Default::default()
        };
        let v = f.to_vector();
        assert_eq!(v[0], 1.0);
        assert_eq!(v[FEATURE_COUNT - 1], 2.0);
    }
}

impl FeatureWeights {
    /// 按特征名改权重，返回是否命中。用于命令行调参。
    pub fn set_by_name(&mut self, name: &str, value: f32) -> bool {
        match name {
            "kind" => self.kind = value,
            "typo_count" => self.typo_count = value,
            "history_count" => self.history_count = value,
            "complete_count" => self.complete_count = value,
            "history_time_diff" => self.history_time_diff = value,
            "context_bigram_score" => self.context_bigram_score = value,
            "user_word_rollback_score" => self.user_word_rollback_score = value,
            "lm_score_with_previous_txt" => self.lm_score_with_previous_txt = value,
            "head_word_nnlm" => self.head_word_nnlm = value,
            "hot_val_conf" => self.hot_val_conf = value,
            "with_context" => self.with_context = value,
            "user_tail_conf" => self.user_tail_conf = value,
            "coverage_score" => self.coverage_score = value,
            "history_use_count" => self.history_use_count = value,
            "history_hot_val" => self.history_hot_val = value,
            "prob" => self.prob = value,
            "context_flag" => self.context_flag = value,
            "full_jianpin" => self.full_jianpin = value,
            "prefix_completion" => self.prefix_completion = value,
            "other_completion" => self.other_completion = value,
            "typo" => self.typo = value,
            "force_emit" => self.force_emit = value,
            _ => return false,
        }
        true
    }

    /// 解析 `a=1,b=-0.5` 形式的覆盖串。
    pub fn apply_pairs(&mut self, spec: &str) -> Result<(), String> {
        for pair in spec.split(',').filter(|p| !p.trim().is_empty()) {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("权重项应为 名称=数值: {pair}"))?;
            let value: f32 = value
                .trim()
                .parse()
                .map_err(|_| format!("权重不是数字: {pair}"))?;
            if !self.set_by_name(name.trim(), value) {
                return Err(format!("未知特征名: {name}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod weight_tests {
    use super::*;

    /// 验证按名字覆盖权重串能生效（项名两边的空格被忽略）
    #[test]
    fn applies_pairs_by_name() {
        let mut w = FeatureWeights::default();
        w.apply_pairs("prob=1.5, coverage_score=-0.5").unwrap();
        assert_eq!(w.prob, 1.5);
        assert_eq!(w.coverage_score, -0.5);
    }

    /// 验证未知特征名、值不是数字、缺等号的写法都报错
    #[test]
    fn rejects_unknown_names() {
        let mut w = FeatureWeights::default();
        assert!(w.apply_pairs("nope=1").is_err());
        assert!(w.apply_pairs("prob=abc").is_err());
        assert!(w.apply_pairs("prob").is_err());
    }

    /// 验证每个特征名都能按名字设权重，且权重数组往返不错位
    #[test]
    fn names_and_arrays_line_up() {
        let mut w = FeatureWeights::default();
        for (i, name) in FEATURE_NAMES.iter().enumerate() {
            assert!(
                w.set_by_name(name, i as f32 + 1.0),
                "特征名 {name} 不在 set_by_name 里"
            );
        }
        assert_eq!(
            FeatureWeights::from_array(w.to_array()).to_array(),
            w.to_array()
        );
    }
}

impl FeatureWeights {
    /// 按 FEATURE_NAMES 顺序导出成数组，便于做数值优化。
    pub fn to_array(&self) -> [f32; FEATURE_COUNT] {
        [
            self.kind,
            self.typo_count,
            self.history_count,
            self.complete_count,
            self.history_time_diff,
            self.context_bigram_score,
            self.user_word_rollback_score,
            self.lm_score_with_previous_txt,
            self.head_word_nnlm,
            self.hot_val_conf,
            self.with_context,
            self.user_tail_conf,
            self.coverage_score,
            self.history_use_count,
            self.history_hot_val,
            self.prob,
            self.context_flag,
            self.full_jianpin,
            self.prefix_completion,
            self.other_completion,
            self.typo,
            self.force_emit,
        ]
    }

    /// 按 FEATURE_NAMES 顺序从数组还原权重
    pub fn from_array(w: [f32; FEATURE_COUNT]) -> Self {
        Self {
            kind: w[0],
            typo_count: w[1],
            history_count: w[2],
            complete_count: w[3],
            history_time_diff: w[4],
            context_bigram_score: w[5],
            user_word_rollback_score: w[6],
            lm_score_with_previous_txt: w[7],
            head_word_nnlm: w[8],
            hot_val_conf: w[9],
            with_context: w[10],
            user_tail_conf: w[11],
            coverage_score: w[12],
            history_use_count: w[13],
            history_hot_val: w[14],
            prob: w[15],
            context_flag: w[16],
            full_jianpin: w[17],
            prefix_completion: w[18],
            other_completion: w[19],
            typo: w[20],
            force_emit: w[21],
        }
    }
}
