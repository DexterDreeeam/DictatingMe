//! 多变体注册：用**先验规则**派生子词，与全词共享同一个 alias，一次注册多行。
//!
//! 为什么要有这个测试：`kws_variant` 证明了「短关键词更容易检出」，
//! 但它报的 88.8% 是在同一批数据上挑「每组最优子词」得来的，存在选择偏差，
//! 不能当可交付数字。这里改成**看结果之前就定死的结构规则**，
//! 规则只看唤醒词的字面形态、不看任何检出结果，因此得到的数字是无偏的。
//!
//! 另一个前提：超集触发（注册「小迪」后用户说「小迪小迪」也唤醒）算符合预期，
//! 不计为误触发。所以真正要盯的代价只有两项——对立词、自由语音。
//!
//! 全部在 `max_active_paths = 8` 下跑，因为 beam 宽度已经单独验证过，
//! 这里要回答的是「在已经放宽 beam 之后，多变体注册还有没有增量收益」。
//!
//! ```text
//! cargo test --release -p dictatingme-runtime --test kws_multi -- --ignored --nocapture
//! ```

mod support;

use std::path::{Path, PathBuf};

use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};

use support::corpus::{load_free_speech, load_groups, Role};
use support::harness::TestEnv;

const SCORE: f32 = 1.0;
const THRESHOLD: f32 = 0.308;
/// 第 2 节已确认的推荐值。多变体注册的增量收益要在这个基础上衡量。
const ACTIVE_PATHS: i32 = 8;

/// 四种注册策略。规则在跑之前就定死，只看唤醒词字面形态。
#[derive(Clone, Copy, PartialEq)]
enum Rule {
    /// 只注册全词 —— 当前生产行为。
    FullOnly,
    /// 全词 + 前半，仅当唤醒词是 AABB 重复型（前半 == 后半）。最保守。
    RepeatHalf,
    /// 全词 + 去掉最后一个字。中间档。
    DropLast,
    /// 全词 + 首二字 + 末二字（长度 >= 4 时）。最激进。
    HeadTail2,
}

impl Rule {
    fn label(self) -> &'static str {
        match self {
            Rule::FullOnly => "全词（当前）",
            Rule::RepeatHalf => "+ 重复词前半",
            Rule::DropLast => "+ 去尾一字",
            Rule::HeadTail2 => "+ 首二字/末二字",
        }
    }

    /// 纯结构派生，不看任何检出数据。
    fn variants(self, phrase: &str) -> Vec<String> {
        let chars: Vec<char> = phrase.chars().collect();
        let n = chars.len();
        let sub = |a: usize, b: usize| -> String { chars[a..b].iter().collect() };
        let mut out = vec![phrase.to_owned()];
        let mut push = |candidate: String| {
            if candidate.chars().count() >= 2 && !out.contains(&candidate) {
                out.push(candidate);
            }
        };
        match self {
            Rule::FullOnly => {}
            Rule::RepeatHalf => {
                if n >= 4 && n % 2 == 0 && sub(0, n / 2) == sub(n / 2, n) {
                    push(sub(0, n / 2));
                }
            }
            Rule::DropLast => {
                if n >= 4 {
                    push(sub(0, n - 1));
                }
            }
            Rule::HeadTail2 => {
                if n >= 4 {
                    push(sub(0, 2));
                    push(sub(n - 2, n));
                }
            }
        }
        out
    }
}

const RULES: [Rule; 4] = [
    Rule::FullOnly,
    Rule::RepeatHalf,
    Rule::DropLast,
    Rule::HeadTail2,
];

struct KwsFiles {
    encoder: PathBuf,
    decoder: PathBuf,
    joiner: PathBuf,
    tokens: PathBuf,
}

fn resolve_files(dir: &Path) -> Result<KwsFiles, String> {
    let pick = |role: &str, candidates: &[&str]| -> Result<PathBuf, String> {
        candidates
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
            .ok_or_else(|| format!("missing {role} under '{}'", dir.display()))
    };
    Ok(KwsFiles {
        encoder: pick(
            "encoder",
            &[
                "encoder-epoch-12-avg-2-chunk-16-left-64.onnx",
                "encoder.onnx",
            ],
        )?,
        decoder: pick(
            "decoder",
            &[
                "decoder-epoch-12-avg-2-chunk-16-left-64.onnx",
                "decoder.onnx",
            ],
        )?,
        joiner: pick(
            "joiner",
            &["joiner-epoch-12-avg-2-chunk-16-left-64.onnx", "joiner.onnx"],
        )?,
        tokens: pick("tokens", &["tokens.txt"])?,
    })
}

fn build_spotter(files: &KwsFiles) -> Result<KeywordSpotter, String> {
    let mut config = KeywordSpotterConfig::default();
    config.model_config.transducer.encoder = Some(files.encoder.display().to_string());
    config.model_config.transducer.decoder = Some(files.decoder.display().to_string());
    config.model_config.transducer.joiner = Some(files.joiner.display().to_string());
    config.model_config.tokens = Some(files.tokens.display().to_string());
    config.model_config.num_threads = 2;
    config.max_active_paths = ACTIVE_PATHS;
    config.keywords_buf = Some("n ǐ h ǎo @占位\n".to_owned());
    KeywordSpotter::create(&config).ok_or_else(|| "failed to create keyword spotter".to_owned())
}

fn tokens_of(syntax: &str) -> String {
    syntax
        .split_whitespace()
        .take_while(|part| !part.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 把一组变体拼成多行 keywords，**全部共享同一个 alias**。
/// `process_frame` 比的是 alias，所以命中任意一行都算命中同一个唤醒词。
fn keywords_buf(env: &TestEnv, variants: &[String]) -> Option<(String, Vec<String>)> {
    let mut syntax = String::new();
    let mut lines = Vec::new();
    for text in variants {
        let engine = env.new_spotter(text).ok()?;
        let tokens = tokens_of(engine.keyword_syntax());
        drop(engine);
        if tokens.is_empty() || lines.iter().any(|(_, t)| t == &tokens) {
            continue;
        }
        lines.push((text.clone(), tokens));
    }
    if lines.is_empty() {
        return None;
    }
    for (_, tokens) in &lines {
        syntax.push_str(&format!("{tokens} @唤醒 :{SCORE:.1} #{THRESHOLD:.3}\n"));
    }
    Some((syntax, lines.into_iter().map(|(text, _)| text).collect()))
}

fn decode_all(spotter: &KeywordSpotter, streams: &[OnlineStream], samples: &[f32]) -> Vec<bool> {
    let mut hit = vec![false; streams.len()];
    let refs = streams.iter().collect::<Vec<_>>();
    let feed = |hit: &mut Vec<bool>| loop {
        let ready = refs
            .iter()
            .copied()
            .filter(|stream| spotter.is_ready(stream))
            .collect::<Vec<_>>();
        if ready.is_empty() {
            break;
        }
        spotter.decode_multiple_streams(&ready);
        for (index, stream) in streams.iter().enumerate() {
            if let Some(result) = spotter.get_result(stream) {
                if !result.keyword.is_empty() {
                    hit[index] = true;
                }
            }
        }
    };
    for chunk in samples.chunks(1_600) {
        for stream in streams {
            stream.accept_waveform(16_000, chunk);
        }
        feed(&mut hit);
    }
    let tail = vec![0.0_f32; 4_800];
    for stream in streams {
        stream.accept_waveform(16_000, &tail);
    }
    feed(&mut hit);
    hit
}

#[derive(Default, Clone, Copy)]
struct Counts {
    positive: u32,
    confusable: u32,
    free: u32,
}

#[test]
#[ignore = "需要 assets/corpus 语料，手动运行"]
fn probes_multi_variant_registration() {
    let env = TestEnv::prepare().expect("test environment");
    let groups = load_groups().expect("corpus manifest");
    let files = resolve_files(env.kws_model_dir()).expect("kws model files");
    let spotter = build_spotter(&files).expect("spotter");
    let free_speech = load_free_speech();

    println!("\n=== 多变体注册（先验规则，max_active_paths = {ACTIVE_PATHS}）===");
    println!("规则在看结果之前定死，只看唤醒词字面形态，因此无选择偏差。");
    println!("超集触发算符合预期，不计入误触；真正的代价只有「对立」和「自由」。\n");

    let mut totals = vec![Counts::default(); RULES.len()];
    let mut denom = Counts::default();
    let mut applied = vec![0_u32; RULES.len()];

    for (group_index, group) in groups.iter().enumerate() {
        let mut buffers = Vec::new();
        for rule in RULES {
            let variants = rule.variants(&group.phrase);
            let Some((syntax, lines)) = keywords_buf(&env, &variants) else {
                panic!("failed to derive keywords for '{}'", group.phrase);
            };
            buffers.push((syntax, lines));
        }

        let mut jobs: Vec<(usize, PathBuf)> = Vec::new();
        for (slot, role) in [Role::Positive, Role::Confusable].into_iter().enumerate() {
            for item in group.by_role(role) {
                jobs.push((slot, item.path.clone()));
            }
        }
        for offset in 0..3 {
            if free_speech.is_empty() {
                break;
            }
            let index = (group_index * 3 + offset) % free_speech.len();
            jobs.push((2, free_speech[index].clone()));
        }

        let mut counts = vec![Counts::default(); RULES.len()];
        let mut local = Counts::default();
        for (slot, path) in &jobs {
            let samples = dictatingme_runtime::evoke_setup::features::read_wav_16k(path)
                .unwrap_or_else(|error| panic!("failed to read '{}': {error}", path.display()));
            let streams = buffers
                .iter()
                .map(|(syntax, _)| spotter.create_stream_with_keywords(syntax))
                .collect::<Vec<_>>();
            match slot {
                0 => local.positive += 1,
                1 => local.confusable += 1,
                _ => local.free += 1,
            }
            for (index, hit) in decode_all(&spotter, &streams, &samples)
                .into_iter()
                .enumerate()
            {
                if !hit {
                    continue;
                }
                match slot {
                    0 => counts[index].positive += 1,
                    1 => counts[index].confusable += 1,
                    _ => counts[index].free += 1,
                }
            }
        }

        println!("[{} {}]", group.id, group.phrase);
        for (index, rule) in RULES.iter().enumerate() {
            let c = counts[index];
            let lines = &buffers[index].1;
            if lines.len() > 1 {
                applied[index] += 1;
            }
            println!(
                "  {:<18} 正 {}/{}  对立 {}/{}  自由 {}/{}   注册: {}",
                rule.label(),
                c.positive,
                local.positive,
                c.confusable,
                local.confusable,
                c.free,
                local.free,
                lines.join(" | ")
            );
            totals[index].positive += c.positive;
            totals[index].confusable += c.confusable;
            totals[index].free += c.free;
        }
        denom.positive += local.positive;
        denom.confusable += local.confusable;
        denom.free += local.free;
        println!();
    }

    let pct = |hit: u32, total: u32| {
        if total == 0 {
            0.0
        } else {
            hit as f32 * 100.0 / total as f32
        }
    };
    println!("=== 汇总（n 正={} 对立={} 自由={}）===", denom.positive, denom.confusable, denom.free);
    println!(
        "{:<18} {:>8} {:>9} {:>9} {:>10}",
        "策略", "TPR%", "对立%", "自由%", "生效组数"
    );
    for (index, rule) in RULES.iter().enumerate() {
        println!(
            "{:<18} {:>8.1} {:>9.1} {:>9.1} {:>10}",
            rule.label(),
            pct(totals[index].positive, denom.positive),
            pct(totals[index].confusable, denom.confusable),
            pct(totals[index].free, denom.free),
            applied[index]
        );
    }
}
