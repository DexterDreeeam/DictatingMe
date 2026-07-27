//! 定位 KWS 硬失败词的根因，并量化「注册更短的子词」这条路的收益与代价。
//!
//! 前一轮参数扫描显示：score/threshold 怎么调，w01「小迪小迪」「语音输入」都是 0%。
//! 单点探针又显示：同一段音频，`x iǎo d í x iǎo d í`（整词）0/4，
//! 而 `x iǎo d í`（半词）4/4。所以断点不在声学、不在参数，在**关键词长度**。
//!
//! 这里把结论推广到全部 20 组：对每组的全词与各个连续子词，
//! 用产品自己的拼音派生（`EvokeModelEngine::keyword_syntax`）注册，
//! 在生产参数下同时测正样本命中与负样本误触，避免只报喜不报忧。
//!
//! ```text
//! cargo test --release -p dictatingme-runtime --test kws_variant -- --ignored --nocapture
//! ```

mod support;

use std::path::{Path, PathBuf};

use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};

use support::corpus::{load_free_speech, load_groups, Role};
use support::harness::TestEnv;

/// 生产当前等效参数：`keywords_score` 从未设过（crate 默认 1.0），
/// `keywords_threshold` = `sensitivity_to_threshold(0.65)` = 0.308。
const SCORE: f32 = 1.0;
const THRESHOLD: f32 = 0.308;

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
    config.keywords_buf = Some("n ǐ h ǎo @占位\n".to_owned());
    KeywordSpotter::create(&config).ok_or_else(|| "failed to create keyword spotter".to_owned())
}

/// 剥出 `keyword_syntax()` 里 `@alias` 之前的 token 串。
fn tokens_of(syntax: &str) -> String {
    syntax
        .split_whitespace()
        .take_while(|part| !part.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ")
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

/// 全词，以及所有长度 >= 2 且短于全词的连续子词。
fn sub_phrases(phrase: &str) -> Vec<String> {
    let chars: Vec<char> = phrase.chars().collect();
    let total = chars.len();
    let mut out = vec![phrase.to_owned()];
    for len in (2..total).rev() {
        for start in 0..=(total - len) {
            out.push(chars[start..start + len].iter().collect());
        }
    }
    out
}

#[derive(Default, Clone, Copy)]
struct Counts {
    positive: u32,
    impostor: u32,
    confusable: u32,
    free: u32,
}

#[test]
#[ignore = "需要 assets/corpus 语料，手动运行"]
fn probes_sub_phrase_keywords() {
    let env = TestEnv::prepare().expect("test environment");
    let groups = load_groups().expect("corpus manifest");
    let files = resolve_files(env.kws_model_dir()).expect("kws model files");
    let spotter = build_spotter(&files).expect("spotter");
    let free_speech = load_free_speech();

    println!("\n=== 子词注册探针（生产参数 :{SCORE:.1} #{THRESHOLD:.3}）===");
    println!("只有「正」列越高越好；「对立」「自由」是误触，越低越好。");
    println!("「冒充」是他人念同一唤醒词——text 模式下本就该唤醒，仅作参考。\n");

    let mut full_totals = Counts::default();
    let mut best_totals = Counts::default();
    let mut denom = Counts::default();
    let mut picks: Vec<(String, String, u32, u32)> = Vec::new();

    for (group_index, group) in groups.iter().enumerate() {
        let mut candidates: Vec<(String, String)> = Vec::new();
        for text in sub_phrases(&group.phrase) {
            let Ok(engine) = env.new_spotter(&text) else {
                continue;
            };
            let tokens = tokens_of(engine.keyword_syntax());
            drop(engine);
            if tokens.is_empty() {
                continue;
            }
            candidates.push((text, tokens));
        }
        if candidates.is_empty() {
            continue;
        }

        let mut jobs: Vec<(usize, PathBuf)> = Vec::new();
        for (slot, role) in [Role::Positive, Role::Impostor, Role::Confusable]
            .into_iter()
            .enumerate()
        {
            for item in group.by_role(role) {
                jobs.push((slot, item.path.clone()));
            }
        }
        for offset in 0..3 {
            if free_speech.is_empty() {
                break;
            }
            let index = (group_index * 3 + offset) % free_speech.len();
            jobs.push((3, free_speech[index].clone()));
        }

        let mut counts = vec![Counts::default(); candidates.len()];
        let mut local_denom = Counts::default();
        for (slot, path) in &jobs {
            let samples = dictatingme_runtime::evoke_setup::features::read_wav_16k(path)
                .unwrap_or_else(|error| panic!("failed to read '{}': {error}", path.display()));
            let streams = candidates
                .iter()
                .map(|(_, tokens)| {
                    let syntax = format!("{tokens} @probe :{SCORE:.1} #{THRESHOLD:.3}\n");
                    spotter.create_stream_with_keywords(&syntax)
                })
                .collect::<Vec<_>>();
            match slot {
                0 => local_denom.positive += 1,
                1 => local_denom.impostor += 1,
                2 => local_denom.confusable += 1,
                _ => local_denom.free += 1,
            }
            for (index, hit) in decode_all(&spotter, &streams, &samples).into_iter().enumerate() {
                if !hit {
                    continue;
                }
                match slot {
                    0 => counts[index].positive += 1,
                    1 => counts[index].impostor += 1,
                    2 => counts[index].confusable += 1,
                    _ => counts[index].free += 1,
                }
            }
        }

        println!("[{} {}]", group.id, group.phrase);
        for (index, (text, tokens)) in candidates.iter().enumerate() {
            let c = counts[index];
            let mark = if index == 0 { "  <= 当前注册" } else { "" };
            println!(
                "  {:<6} 正 {}/{}  冒充 {}/{}  对立 {}/{}  自由 {}/{}   [{}]{}",
                text,
                c.positive,
                local_denom.positive,
                c.impostor,
                local_denom.impostor,
                c.confusable,
                local_denom.confusable,
                c.free,
                local_denom.free,
                tokens,
                mark
            );
        }

        // 挑一个「误触不比全词差、正样本最多」的子词，用来估算整体上限。
        let full = counts[0];
        let mut best_index = 0;
        for index in 1..candidates.len() {
            let c = counts[index];
            if c.confusable > full.confusable || c.free > full.free {
                continue;
            }
            if c.positive > counts[best_index].positive {
                best_index = index;
            }
        }
        if best_index != 0 {
            picks.push((
                group.phrase.clone(),
                candidates[best_index].0.clone(),
                full.positive,
                counts[best_index].positive,
            ));
        }

        let best = counts[best_index];
        full_totals.positive += full.positive;
        full_totals.impostor += full.impostor;
        full_totals.confusable += full.confusable;
        full_totals.free += full.free;
        best_totals.positive += best.positive;
        best_totals.impostor += best.impostor;
        best_totals.confusable += best.confusable;
        best_totals.free += best.free;
        denom.positive += local_denom.positive;
        denom.impostor += local_denom.impostor;
        denom.confusable += local_denom.confusable;
        denom.free += local_denom.free;
        println!();
    }

    let pct = |hit: u32, total: u32| {
        if total == 0 {
            0.0
        } else {
            hit as f32 * 100.0 / total as f32
        }
    };
    println!("=== 汇总 ===");
    println!(
        "{:<22} {:>8} {:>9} {:>9} {:>9}",
        "策略", "TPR%", "冒充%", "对立%", "自由%"
    );
    println!(
        "{:<22} {:>8.1} {:>9.1} {:>9.1} {:>9.1}",
        "全词（当前）",
        pct(full_totals.positive, denom.positive),
        pct(full_totals.impostor, denom.impostor),
        pct(full_totals.confusable, denom.confusable),
        pct(full_totals.free, denom.free)
    );
    println!(
        "{:<22} {:>8.1} {:>9.1} {:>9.1} {:>9.1}",
        "每组最优子词",
        pct(best_totals.positive, denom.positive),
        pct(best_totals.impostor, denom.impostor),
        pct(best_totals.confusable, denom.confusable),
        pct(best_totals.free, denom.free)
    );

    if picks.is_empty() {
        println!("\n没有任何一组能靠换子词在不增加误触的前提下提升检出。");
    } else {
        println!("\n可换子词的组（误触未变差）：");
        for (phrase, sub, before, after) in &picks {
            println!("  {phrase} -> {sub}   正样本 {before} -> {after}");
        }
    }
}
