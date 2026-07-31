//! 四种唤醒模式的端到端测试：setup -> detect，用合成语料量出 TPR / FAR。
//!
//! 语料由 `cargo run -p xtask --release -- corpus` 生成，默认不在仓库里，
//! 所以这些用例标了 `#[ignore]`，缺素材时直接跳过而不是失败。
//!
//! 运行：
//! ```text
//! cargo test -p dictatingme-runtime --test evoke_e2e -- --ignored --nocapture
//! ```
//! 环境变量：
//! - `DM_E2E_GROUPS=w01,w05` 只跑指定唤醒词分组
//! - `DM_E2E_SENSITIVITY=0.65` 覆盖敏感度

mod support;

use dictatingme_runtime::evoke_setup::features::read_wav_16k;
use dictatingme_runtime::evoke_setup::EvokeMode;

use support::augment::{apply, Condition, Rng};
use support::corpus::{load_free_speech, load_groups, load_noise_by_category, Role};
use support::harness::{detect, TestEnv};
use support::metrics::ModeMetrics;
use support::report::print_and_persist;

const DEFAULT_SENSITIVITY: f32 = 0.65;
/// 每个分组挑几条自由语音做负样本，控制总时长。
const FREE_SPEECH_PER_GROUP: usize = 3;

fn sensitivity() -> f32 {
    std::env::var("DM_E2E_SENSITIVITY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_SENSITIVITY)
}

/// 正样本的测试条件矩阵：语速 × 噪声 × 音量。
fn conditions() -> Vec<Condition> {
    let noise = load_noise_by_category();
    let pick = |category: &str, index: usize| {
        noise
            .get(category)
            .and_then(|paths| paths.get(index % paths.len().max(1)))
            .cloned()
    };

    let mut list = vec![
        Condition::clean(),
        Condition {
            id: "fast".to_owned(),
            speed: 1.15,
            ..Condition::clean()
        },
        Condition {
            id: "slow".to_owned(),
            speed: 0.87,
            ..Condition::clean()
        },
        Condition {
            id: "quiet".to_owned(),
            gain: 0.5,
            ..Condition::clean()
        },
    ];
    for (category, snr_db) in [("office", 15.0), ("crowd", 10.0), ("traffic", 5.0)] {
        let Some(path) = pick(category, 0) else {
            continue;
        };
        list.push(Condition {
            id: format!("{category}@{snr_db:.0}dB"),
            speed: 1.0,
            noise: Some(path),
            noise_category: category.to_owned(),
            snr_db,
            gain: 1.0,
        });
    }
    list
}

fn run_mode(mode: EvokeMode) {
    let groups = match load_groups() {
        Ok(groups) => groups,
        Err(error) => {
            eprintln!("[e2e] 无法读取语料：{error}");
            return;
        }
    };
    if groups.is_empty() {
        eprintln!("[e2e] assets/corpus 为空，跳过。先跑 `cargo run -p xtask --release -- corpus`。");
        return;
    }

    let env = match TestEnv::prepare() {
        Ok(env) => env,
        Err(error) => panic!("failed to prepare assets: {error}"),
    };
    if mode == EvokeMode::SpeakerVerify && env.speaker_model().is_none() {
        eprintln!("[e2e] 声纹模型不可用，跳过 speakerVerify。");
        return;
    }

    let sensitivity = sensitivity();
    let conditions = conditions();
    let free_speech = load_free_speech();
    let mut metrics = ModeMetrics::default();
    let mut rng = Rng::new(0x5EED_0000_0000_0001);

    for (group_index, group) in groups.iter().enumerate() {
        let enroll = group.by_role(Role::Enroll);
        let needed = mode.required_recordings() as usize;
        if enroll.len() < needed {
            metrics.setup_failures.push((
                group.id.clone(),
                format!("需要 {needed} 条注册素材，只有 {}", enroll.len()),
            ));
            continue;
        }
        let recordings = enroll
            .iter()
            .take(needed)
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();

        let profile = match env.run_setup(mode, &group.phrase, recordings) {
            Ok(profile) => profile,
            Err(error) => {
                metrics.setup_failures.push((group.id.clone(), error));
                continue;
            }
        };
        // 与产品一致：beam 宽度按唤醒模式决定（lib.rs 也是这么传的）。
        let mut spotter = match env.new_spotter_with_beam(&group.phrase, mode.kws_max_active_paths())
        {
            Ok(spotter) => spotter,
            Err(error) => {
                metrics
                    .setup_failures
                    .push((group.id.clone(), format!("KWS 装载失败：{error}")));
                continue;
            }
        };

        let verbose = std::env::var("DM_E2E_VERBOSE").is_ok();
        let mut judge = |label: &str, samples: &[f32]| -> Option<bool> {
            match detect(
                &mut spotter,
                &profile,
                env.speaker_model(),
                sensitivity,
                samples,
            ) {
                Ok(outcome) => {
                    if verbose {
                        eprintln!(
                            "    {label:<26} hits={:<3} woke={:<5} overall={:.3} thr={:.3} phrase={:.3} mode={:.3} vad={:.3}",
                            outcome.keyword_hits,
                            outcome.woke,
                            outcome.best.overall,
                            outcome.best.threshold,
                            outcome.best.phrase_score,
                            outcome.best.mode_score,
                            outcome.best.voice_activity,
                        );
                    }
                    Some(outcome.woke)
                }
                Err(error) => {
                    eprintln!("[e2e] {} detect 失败：{error}", group.id);
                    None
                }
            }
        };

        // 正样本：本人说唤醒词，覆盖全部条件。
        for utterance in group.by_role(Role::Positive) {
            let Ok(base) = read_wav_16k(&utterance.path) else {
                continue;
            };
            for condition in &conditions {
                let Ok(samples) = apply(&base, condition, &mut rng) else {
                    continue;
                };
                let Some(woke) = judge(&format!("pos/{}", condition.id), &samples) else { continue };
                metrics.positive.record(woke);
                metrics
                    .by_condition
                    .entry(condition.id.clone())
                    .or_default()
                    .record(woke);
            }
        }

        // 冒充者：别人说同一个唤醒词。
        for utterance in group.by_role(Role::Impostor) {
            let Ok(samples) = read_wav_16k(&utterance.path) else {
                continue;
            };
            if let Some(woke) = judge("imp", &samples) {
                metrics.impostor.record(woke);
            }
        }

        // 对立词：本人说发音相近但不同的短语。
        for utterance in group.by_role(Role::Confusable) {
            let Ok(samples) = read_wav_16k(&utterance.path) else {
                continue;
            };
            if let Some(woke) = judge("cfz", &samples) {
                metrics
                    .confusable
                    .entry(group.tier.clone())
                    .or_default()
                    .record(woke);
            }
        }

        // 自由语音：与唤醒词完全无关的日常句子。
        if !free_speech.is_empty() {
            for offset in 0..FREE_SPEECH_PER_GROUP {
                let index = (group_index * FREE_SPEECH_PER_GROUP + offset) % free_speech.len();
                let Ok(samples) = read_wav_16k(&free_speech[index]) else {
                    continue;
                };
                if let Some(woke) = judge("free", &samples) {
                    metrics.free_speech.record(woke);
                }
            }
        }

        eprintln!(
            "[e2e] {} 完成 {}/{}",
            mode.as_str(),
            group_index + 1,
            groups.len()
        );
    }

    print_and_persist(mode.as_str(), &metrics);
    assert!(
        metrics.positive.total > 0,
        "没有产生任何正样本判定，语料或夹具有问题"
    );
}

#[test]
#[ignore = "需要合成语料，见文件头"]
fn evoke_e2e_text() {
    run_mode(EvokeMode::Text);
}

#[test]
#[ignore = "需要合成语料，见文件头"]
fn evoke_e2e_voice_match() {
    run_mode(EvokeMode::VoiceMatch);
}

#[test]
#[ignore = "需要合成语料，见文件头"]
fn evoke_e2e_speaker_verify() {
    run_mode(EvokeMode::SpeakerVerify);
}

#[test]
#[ignore = "需要合成语料，见文件头"]
fn evoke_e2e_classifier() {
    run_mode(EvokeMode::Classifier);
}
