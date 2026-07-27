//! KWS 参数扫描：找出把 TPR 天花板抬起来的配置，同时盯住 FAR 代价。
//!
//! 生产当前用 `keywords_score` = 1.0（crate 默认，代码从未设过）、
//! `keywords_threshold` = `sensitivity_to_threshold(0.65)` = 0.308。
//! sherpa 自己的默认阈值是 0.25——也就是说生产比上游默认还严。
//!
//! ```text
//! cargo test --release -p dictatingme-runtime --test kws_sweep -- --ignored --nocapture
//! ```

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};

use support::corpus::{load_free_speech, load_groups, Role};
use support::harness::TestEnv;

/// 扫描的一个参数点。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Combo {
    score: f32,
    threshold: f32,
}

impl Combo {
    fn label(self) -> String {
        format!(":{:.1} #{:.3}", self.score, self.threshold)
    }
}

/// 生产当前等效的参数点。
const PRODUCTION: Combo = Combo {
    score: 1.0,
    threshold: 0.308,
};

fn combos() -> Vec<Combo> {
    let mut list = Vec::new();
    for score in [1.0_f32, 1.5, 2.0, 3.0, 4.0] {
        for threshold in [0.308_f32, 0.25, 0.15, 0.05] {
            list.push(Combo { score, threshold });
        }
    }
    list
}

#[derive(Default, Clone, Copy)]
struct Tally {
    positive_hit: u32,
    positive_total: u32,
    impostor_hit: u32,
    impostor_total: u32,
    confusable_hit: u32,
    confusable_total: u32,
    free_hit: u32,
    free_total: u32,
}

impl Tally {
    fn record(&mut self, bucket: Bucket, hit: bool) {
        let (hits, total) = match bucket {
            Bucket::Positive => (&mut self.positive_hit, &mut self.positive_total),
            Bucket::Impostor => (&mut self.impostor_hit, &mut self.impostor_total),
            Bucket::Confusable => (&mut self.confusable_hit, &mut self.confusable_total),
            Bucket::FreeSpeech => (&mut self.free_hit, &mut self.free_total),
        };
        *total += 1;
        if hit {
            *hits += 1;
        }
    }

    fn tpr(&self) -> f32 {
        rate(self.positive_hit, self.positive_total)
    }

    /// 负样本合计误接受率。对立词与自由语音是真误唤醒；
    /// 冒充者对 text 模式是误唤醒，对声纹类模式由后一级拦截，这里一并统计。
    fn far(&self) -> f32 {
        let hits = self.impostor_hit + self.confusable_hit + self.free_hit;
        let total = self.impostor_total + self.confusable_total + self.free_total;
        rate(hits, total)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    Positive,
    Impostor,
    Confusable,
    FreeSpeech,
}

fn rate(hits: u32, total: u32) -> f32 {
    if total == 0 {
        0.0
    } else {
        hits as f32 * 100.0 / total as f32
    }
}

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

fn build_spotter(
    files: &KwsFiles,
    num_trailing_blanks: i32,
    max_active_paths: i32,
) -> Result<KeywordSpotter, String> {
    let mut config = KeywordSpotterConfig::default();
    config.model_config.transducer.encoder = Some(files.encoder.display().to_string());
    config.model_config.transducer.decoder = Some(files.decoder.display().to_string());
    config.model_config.transducer.joiner = Some(files.joiner.display().to_string());
    config.model_config.tokens = Some(files.tokens.display().to_string());
    config.model_config.num_threads = 2;
    config.num_trailing_blanks = num_trailing_blanks;
    config.max_active_paths = max_active_paths;
    // keywords_buf 必须非空，否则 spotter 创建失败；真正生效的是 per-stream 关键词。
    config.keywords_buf = Some("n ǐ h ǎo @占位\n".to_owned());
    KeywordSpotter::create(&config).ok_or_else(|| "failed to create keyword spotter".to_owned())
}

/// 从 `EvokeModelEngine::keyword_syntax()` 里剥出 token 串与 alias，
/// 这样扫描用的是产品自己的拼音派生结果，而不是另写一份。
fn split_syntax(syntax: &str) -> (String, String) {
    let tokens = syntax
        .split_whitespace()
        .take_while(|part| !part.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ");
    let alias = syntax
        .split_whitespace()
        .find_map(|part| part.strip_prefix('@'))
        .unwrap_or("keyword")
        .to_owned();
    (tokens, alias)
}

/// 把一条音频喂给一组并行 stream，返回每个 stream 是否命中。
fn decode_all(spotter: &KeywordSpotter, streams: &[OnlineStream], samples: &[f32]) -> Vec<bool> {
    let mut hit = vec![false; streams.len()];
    let refs = streams.iter().collect::<Vec<_>>();
    let mut feed = |spotter: &KeywordSpotter, hit: &mut Vec<bool>| loop {
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
        feed(spotter, &mut hit);
    }
    // 收尾：补一段静音把最后的关键词冲出解码器。
    let tail = vec![0.0_f32; 4_800];
    for stream in streams {
        stream.accept_waveform(16_000, &tail);
    }
    feed(spotter, &mut hit);
    hit
}

#[test]
#[ignore = "需要 assets/corpus 语料，手动运行"]
fn sweeps_kws_score_and_threshold() {
    let env = TestEnv::prepare().expect("test environment");
    let groups = load_groups().expect("corpus manifest");
    assert!(!groups.is_empty(), "corpus is empty; run `cargo run -p xtask --release -- corpus`");

    let files = resolve_files(env.kws_model_dir()).expect("kws model files");
    let combos = combos();

    let trailing_blanks: i32 = std::env::var("DM_KWS_TRAILING_BLANKS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let active_paths: i32 = std::env::var("DM_KWS_ACTIVE_PATHS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    let spotter = build_spotter(&files, trailing_blanks, active_paths).expect("spotter");

    let free_speech = load_free_speech();
    let mut totals = vec![Tally::default(); combos.len()];
    let mut per_group: BTreeMap<String, Vec<Tally>> = BTreeMap::new();

    for (group_index, group) in groups.iter().enumerate() {
        let engine = env
            .new_spotter(&group.phrase)
            .expect("engine for phrase derives tokens");
        let (tokens, alias) = split_syntax(engine.keyword_syntax());
        drop(engine);

        let mut jobs: Vec<(Bucket, PathBuf)> = Vec::new();
        for role in [Role::Positive, Role::Impostor, Role::Confusable] {
            let bucket = match role {
                Role::Positive => Bucket::Positive,
                Role::Impostor => Bucket::Impostor,
                _ => Bucket::Confusable,
            };
            for item in group.by_role(role) {
                jobs.push((bucket, item.path.clone()));
            }
        }
        for offset in 0..3 {
            if free_speech.is_empty() {
                break;
            }
            let index = (group_index * 3 + offset) % free_speech.len();
            jobs.push((Bucket::FreeSpeech, free_speech[index].clone()));
        }

        let entry = per_group
            .entry(format!("{} {}", group.id, group.phrase))
            .or_insert_with(|| vec![Tally::default(); combos.len()]);

        for (bucket, path) in jobs {
            let samples = dictatingme_runtime::evoke_setup::features::read_wav_16k(&path)
                .unwrap_or_else(|error| panic!("failed to read '{}': {error}", path.display()));
            let streams = combos
                .iter()
                .map(|combo| {
                    let syntax = format!(
                        "{tokens} @{alias} :{:.1} #{:.3}\n",
                        combo.score, combo.threshold
                    );
                    spotter.create_stream_with_keywords(&syntax)
                })
                .collect::<Vec<_>>();
            let hits = decode_all(&spotter, &streams, &samples);
            for (index, hit) in hits.into_iter().enumerate() {
                totals[index].record(bucket, hit);
                entry[index].record(bucket, hit);
            }
        }
        eprintln!("[sweep] {} {} done", group.id, group.phrase);
    }

    let baseline = combos
        .iter()
        .position(|combo| *combo == PRODUCTION)
        .expect("production combo is part of the sweep");

    println!("\n=== KWS 参数扫描（trailing_blanks={trailing_blanks} active_paths={active_paths}）===");
    println!(
        "{:<16} {:>8} {:>9} {:>9} {:>9} {:>9}",
        "combo", "TPR%", "冒充%", "对立%", "自由%", "FAR%"
    );
    for (index, combo) in combos.iter().enumerate() {
        let tally = totals[index];
        let mark = if index == baseline { " <= 生产" } else { "" };
        println!(
            "{:<16} {:>8.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1}{}",
            combo.label(),
            tally.tpr(),
            rate(tally.impostor_hit, tally.impostor_total),
            rate(tally.confusable_hit, tally.confusable_total),
            rate(tally.free_hit, tally.free_total),
            tally.far(),
            mark
        );
    }

    println!("\n=== 分组检出率（TPR%）===");
    let show: Vec<usize> = combos
        .iter()
        .enumerate()
        .filter(|(_, combo)| {
            [(1.0_f32, 0.308_f32), (2.0, 0.25), (3.0, 0.15), (4.0, 0.05)]
                .iter()
                .any(|(score, threshold)| {
                    (combo.score - score).abs() < 1e-3 && (combo.threshold - threshold).abs() < 1e-3
                })
        })
        .map(|(index, _)| index)
        .collect();
    print!("{:<24}", "组");
    for index in &show {
        print!("{:>14}", combos[*index].label());
    }
    println!();
    for (name, tallies) in &per_group {
        print!("{name:<24}");
        for index in &show {
            print!("{:>14.0}", tallies[*index].tpr());
        }
        println!();
    }

    let production = totals[baseline];
    println!(
        "\n生产基线 TPR {:.1}% / FAR {:.1}%（正 {} 负 {}）",
        production.tpr(),
        production.far(),
        production.positive_total,
        production.impostor_total + production.confusable_total + production.free_total
    );
    assert!(production.positive_total > 0);
}
