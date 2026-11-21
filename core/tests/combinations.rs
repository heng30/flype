//! 组合场景测试：多个特性同时开启时是否互相干扰。
//!
//! 之前每个特性都单独测过，但它们组合起来没测过。

use std::sync::Arc;

use flype_core::lm::BigramLm;
use flype_core::{Engine, Lexicon, MemoryLexicon};

/// 用 TSV 文本建一个只看词频（默认二元模型）的引擎
fn engine(lexicon_tsv: &str) -> Engine {
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(lexicon_tsv).unwrap());
    Engine::new(lexicon, Arc::new(BigramLm::default()))
}

/// 连续选低频候选后它排到第一位
#[test]
fn repeated_selection_boosts_candidate() {
    // 两个同拼音候选，频次相差不大；连续选低频的那个应当把它顶上来
    let e = engine("shi\t是\t1000\nshi\t事\t900\n");
    assert_eq!(e.input("shi")[0].text, "是");
    for _ in 0..6 {
        e.record_selection("事");
    }
    let list = e.input("shi");
    assert_eq!(
        list[0].text,
        "事",
        "多次选用后应当排到前面: {:?}",
        list.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
}
