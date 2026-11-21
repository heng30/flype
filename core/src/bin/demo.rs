//! 命令行 demo：读入拼音串，打印候选。
//!
//! 用法：
//!   cargo run --bin flype-demo -- xian
//!   cargo run --bin flype-demo -- --lexicon data/demo_lexicon.tsv nihao
//!   echo "zg" | cargo run --bin flype-demo
//!
//! 不传拼音参数时从 stdin 逐行读取。

use std::env;
use std::io::{self, BufRead};
use std::process;
use std::sync::Arc;

use flype_core::lm::{ArpaLm, BigramLm, LanguageModel};
use flype_core::{Engine, Lexicon, MemoryLexicon};

/// 打印命令行帮助（选项说明和默认词典路径）
fn usage(program: &str) {
    println!("用法: {program} [选项] [拼音...]");
    println!();
    println!("选项:");
    println!("  --lexicon <file>  TSV 词典（拼音<TAB>词<TAB>频次）");
    println!(
        "  --lm <file>       语言模型：words.lm，或 bigram TSV（prev<TAB>cur<TAB>count），可选"
    );
    println!("  --arpa <file>     ARPA 语言模型（与 --lm 二选一）");
    println!("  --beam <n>        beam size，默认 20");
    println!("  --fuzzy           开启模糊音（z/zh、an/ang 等）");
    println!("  --english <file>  英文词表（每行一个词，或 rime 的 en.dict.yaml）");
    println!("  --context <text>  光标前已有文本（上下文排序）");
    println!("  --max <n>         最多显示候选数，默认 10");
    println!("  -h, --help        显示帮助");
    println!();
    println!("默认词典: {}", default_lexicon_path());
}

/// 源码目录里的 data/demo_lexicon.tsv
fn default_lexicon_path() -> String {
    format!("{}/data/demo_lexicon.tsv", env!("CARGO_MANIFEST_DIR"))
}

/// 解析选项逐项配置引擎，再把命令行给的拼音（或 stdin 每行）交给 run_query
fn main() {
    let args: Vec<String> = env::args().collect();
    let program = args.first().map(String::as_str).unwrap_or("flype-demo");
    let mut lexicon_path = default_lexicon_path();
    let mut lm_path: Option<String> = None;
    let mut arpa_path: Option<String> = None;
    let mut beam: Option<usize> = None;
    let mut fuzzy = false;
    let mut english_path: Option<String> = None;
    let mut context_text: Option<String> = None;
    let mut max_candidates = 10usize;
    let mut queries: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                usage(program);
                return;
            }
            "--lexicon" => {
                i += 1;
                match args.get(i) {
                    Some(value) => lexicon_path = value.clone(),
                    None => fail("--lexicon 需要一个文件路径"),
                }
            }
            "--lm" => {
                i += 1;
                match args.get(i) {
                    Some(value) => lm_path = Some(value.clone()),
                    None => fail("--lm 需要一个文件路径"),
                }
            }
            "--arpa" => {
                i += 1;
                match args.get(i) {
                    Some(value) => arpa_path = Some(value.clone()),
                    None => fail("--arpa 需要一个文件路径"),
                }
            }
            "--beam" => {
                i += 1;
                beam = Some(parse_usize(args.get(i), "--beam"));
            }
            "--fuzzy" => fuzzy = true,
            "--context" => {
                i += 1;
                match args.get(i) {
                    Some(value) => context_text = Some(value.clone()),
                    None => fail("--context 需要文本"),
                }
            }
            "--max" => {
                i += 1;
                max_candidates = parse_usize(args.get(i), "--max");
            }
            "--english" => {
                i += 1;
                match args.get(i) {
                    Some(value) => english_path = Some(value.clone()),
                    None => fail("--english 需要一个文件路径"),
                }
            }
            other if other.starts_with('-') => fail(&format!("未知选项: {other}")),
            other => queries.push(other.to_string()),
        }
        i += 1;
    }

    let lexicon = match MemoryLexicon::from_tsv_path_cached(&lexicon_path) {
        Ok(lexicon) => lexicon,
        Err(error) => fail(&format!("{error:#}")),
    };
    let lm: Arc<dyn LanguageModel> = if let Some(path) = &arpa_path {
        match ArpaLm::from_arpa_path(path) {
            Ok(lm) => Arc::new(lm),
            Err(error) => fail(&format!("{error:#}")),
        }
    } else {
        match &lm_path {
            Some(path) => match flype_core::lm::load_lm(path) {
                Ok(lm) => lm,
                Err(error) => fail(&format!("{error:#}")),
            },
            None => Arc::new(BigramLm::default()),
        }
    };
    let lexicon: Arc<dyn Lexicon> = Arc::new(lexicon);
    let mut engine = Engine::new(lexicon, lm);
    if let Some(beam) = beam {
        engine.set_beam_size(beam);
    }
    engine.set_fuzzy(fuzzy);
    if let Some(text) = &context_text {
        engine.set_context(text);
    }
    if let Some(path) = &english_path {
        match engine.load_english(path) {
            Ok(n) => eprintln!("英文词表：{n} 条"),
            Err(error) => fail(&format!("{error:#}")),
        }
    }
    engine.set_max_candidates(max_candidates);

    if queries.is_empty() {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    let query = line.trim();
                    if !query.is_empty() {
                        run_query(&engine, query);
                    }
                }
                Err(error) => fail(&format!("读取 stdin 失败: {error}")),
            }
        }
    } else {
        for query in &queries {
            run_query(&engine, query);
        }
    }
}

/// 把选项后面的值解析成正整数，缺值或解析失败就退出
fn parse_usize(value: Option<&String>, flag: &str) -> usize {
    match value {
        Some(value) => match value.parse() {
            Ok(parsed) => parsed,
            Err(_) => fail(&format!("{flag} 需要正整数，收到 {value}")),
        },
        None => fail(&format!("{flag} 需要一个参数")),
    }
}

/// 打印错误并以退出码 2 结束进程
fn fail(message: &str) -> ! {
    eprintln!("错误: {message}");
    process::exit(2);
}

/// 打印一条输入的候选：序号、词、分数、拼音和分词，输入解析失败就只打印错误
fn run_query(engine: &Engine, query: &str) {
    let candidates = match engine.try_input(query) {
        Ok(candidates) => candidates,
        Err(error) => {
            println!("输入: {query}");
            println!("  错误: {error:#}");
            return;
        }
    };
    println!("输入: {query}");
    if candidates.is_empty() {
        println!("  (无候选)");
        return;
    }
    for (index, candidate) in candidates.iter().enumerate() {
        let segmentation = candidate
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        println!(
            "  {:>2}. {}  score={:.4}  pinyin={}  分词={}",
            index + 1,
            candidate.text,
            candidate.score,
            candidate.pinyin,
            segmentation
        );
    }
}
