//! 加宽 beam 的代价：常驻监听时 KWS 解码的实时率（RTF）。
//!
//! 准确率上 `max_active_paths` 4 -> 8 -> 16 有明显收益，但唤醒词检测是
//! 开机常驻、每一帧都在跑的东西，CPU 涨多少必须先量出来再谈要不要上。
//!
//! RTF = 解码墙钟时间 / 音频时长。RTF 0.05 表示处理 1 秒音频花 50 ms。
//!
//! ```text
//! cargo test --release -p dictatingme-runtime --test kws_cost -- --ignored --nocapture
//! ```

mod support;

use std::path::{Path, PathBuf};
use std::time::Instant;

use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig};

use support::corpus::{load_groups, Role};
use support::harness::TestEnv;

fn resolve(dir: &Path, role: &str, candidates: &[&str]) -> PathBuf {
    candidates
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("missing {role} under '{}'", dir.display()))
}

fn build(dir: &Path, max_active_paths: i32, keywords: &str) -> KeywordSpotter {
    let mut config = KeywordSpotterConfig::default();
    config.model_config.transducer.encoder = Some(
        resolve(
            dir,
            "encoder",
            &["encoder-epoch-12-avg-2-chunk-16-left-64.onnx", "encoder.onnx"],
        )
        .display()
        .to_string(),
    );
    config.model_config.transducer.decoder = Some(
        resolve(
            dir,
            "decoder",
            &["decoder-epoch-12-avg-2-chunk-16-left-64.onnx", "decoder.onnx"],
        )
        .display()
        .to_string(),
    );
    config.model_config.transducer.joiner = Some(
        resolve(
            dir,
            "joiner",
            &["joiner-epoch-12-avg-2-chunk-16-left-64.onnx", "joiner.onnx"],
        )
        .display()
        .to_string(),
    );
    config.model_config.tokens = Some(resolve(dir, "tokens", &["tokens.txt"]).display().to_string());
    config.model_config.num_threads = 2;
    config.max_active_paths = max_active_paths;
    config.keywords_buf = Some(keywords.to_owned());
    KeywordSpotter::create(&config).expect("spotter")
}

#[test]
#[ignore = "需要 assets/corpus 语料，手动运行"]
fn measures_beam_width_cost() {
    let env = TestEnv::prepare().expect("test environment");
    let groups = load_groups().expect("corpus manifest");
    let dir = env.kws_model_dir().to_path_buf();

    // 复刻生产：单个 spotter、单条 stream、单个关键词。
    let group = groups.first().expect("at least one group");
    let engine = env.new_spotter(&group.phrase).expect("engine");
    let keywords = format!("{}\n", engine.keyword_syntax().trim());
    drop(engine);

    let clips: Vec<Vec<f32>> = groups
        .iter()
        .flat_map(|group| group.by_role(Role::Positive).into_iter().take(2))
        .map(|item| {
            dictatingme_runtime::evoke_setup::features::read_wav_16k(&item.path).expect("wav")
        })
        .collect();
    let audio_seconds: f64 = clips.iter().map(|clip| clip.len() as f64 / 16_000.0).sum();

    println!("\n=== beam 宽度的解码代价（单 stream，{:.1} 秒音频）===", audio_seconds);
    println!("{:<20}{:>12}{:>12}{:>16}", "max_active_paths", "墙钟(s)", "RTF", "相对 paths=4");

    let mut baseline = 0.0_f64;
    for paths in [4_i32, 8, 16] {
        let spotter = build(&dir, paths, &keywords);
        // 预热一遍，避开首次推理的一次性开销。
        {
            let stream = spotter.create_stream();
            stream.accept_waveform(16_000, &clips[0]);
            while spotter.is_ready(&stream) {
                spotter.decode(&stream);
            }
        }
        let start = Instant::now();
        for clip in &clips {
            let stream = spotter.create_stream();
            for chunk in clip.chunks(1_600) {
                stream.accept_waveform(16_000, chunk);
                while spotter.is_ready(&stream) {
                    spotter.decode(&stream);
                }
            }
        }
        let elapsed = start.elapsed().as_secs_f64();
        if paths == 4 {
            baseline = elapsed;
        }
        println!(
            "{:<20}{:>12.2}{:>12.4}{:>15.2}x",
            paths,
            elapsed,
            elapsed / audio_seconds,
            elapsed / baseline.max(1e-9)
        );
    }
    println!("RTF 0.05 = 处理 1 秒音频花 50 ms，也就是常驻占用约 5% 的一个核。");
}
