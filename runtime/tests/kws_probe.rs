//! 诊断工具：只跑 KWS 唤醒词检出，不做打分，用来看哪些合成音色/音量是可用的。
//!
//! ```text
//! cargo test -p dictatingme-runtime --test kws_probe -- --ignored --nocapture
//! ```

mod support;

use std::collections::BTreeMap;

use dictatingme_runtime::audio::AudioFrame;
use dictatingme_runtime::evoke_setup::features::{read_wav_16k, recording_quality};

use support::corpus::{load_groups, Role};
use support::harness::TestEnv;

#[test]
#[ignore = "诊断用，需要合成语料"]
fn kws_probe() {
    let groups = load_groups().expect("failed to load corpus");
    if groups.is_empty() {
        eprintln!("[probe] assets/corpus 为空");
        return;
    }
    let env = TestEnv::prepare().expect("failed to prepare assets");

    let mut by_voice: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut by_group: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut by_style: BTreeMap<String, (u32, u32)> = BTreeMap::new();

    for group in &groups {
        let mut spotter = match env.new_spotter(&group.phrase) {
            Ok(spotter) => spotter,
            Err(error) => {
                println!("{}  KWS 装载失败: {error}", group.id);
                continue;
            }
        };
        println!(
            "\n### {} 「{}」 -> {}",
            group.id,
            group.phrase,
            spotter.keyword_syntax()
        );
        for utterance in &group.utterances {
            // 对立词本来就不该命中，跳过。
            if utterance.role == Role::Confusable {
                continue;
            }
            let Ok(samples) = read_wav_16k(&utterance.path) else {
                continue;
            };
            let quality = recording_quality(&samples);
            spotter.reset();
            let mut hits = 0u32;
            for (index, chunk) in samples.chunks(1_600).enumerate() {
                let frame = AudioFrame {
                    samples: chunk
                        .iter()
                        .map(|sample| (sample.clamp(-1.0, 1.0) * 32_767.0) as i16)
                        .collect(),
                    sample_rate: 16_000,
                    timestamp_ms: index as u64 * 100,
                };
                if spotter.process_frame(&frame).is_some() {
                    hits += 1;
                }
            }
            let hit = u32::from(hits > 0);
            by_voice.entry(utterance.voice.clone()).or_default().0 += 1;
            by_voice.entry(utterance.voice.clone()).or_default().1 += hit;
            by_group.entry(group.id.clone()).or_default().0 += 1;
            by_group.entry(group.id.clone()).or_default().1 += hit;
            by_style.entry(utterance.style.clone()).or_default().0 += 1;
            by_style.entry(utterance.style.clone()).or_default().1 += hit;

            println!(
                "{:<40} hits={:<3} rms={:.4} voiced={:.3} clip={:.4} ok={}",
                utterance
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                hits,
                quality.rms,
                quality.voiced_ratio,
                quality.clipping_ratio,
                quality.accepted,
            );
        }
    }

    let dump = |title: &str, table: &BTreeMap<String, (u32, u32)>| {
        println!("\n-- {title} --");
        for (key, (total, hit)) in table {
            println!(
                "{key:<12} {hit:>3}/{total:<3} {:>6.1}%",
                *hit as f32 / *total as f32 * 100.0
            );
        }
    };
    dump("按音色", &by_voice);
    dump("按风格", &by_style);
    dump("按分组", &by_group);
}
