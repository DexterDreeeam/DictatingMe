//! speakerVerify 隔离探针：排除 KWS，只评估注册策略与 CampPlus 声纹分离能力。
//!
//! ```text
//! cargo test --release -p dictatingme-runtime --test speaker_probe -- --ignored --nocapture
//! ```

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dictatingme_runtime::evoke_setup::features::read_wav_16k;
use dictatingme_runtime::evoke_setup::modes::speaker::{average_embeddings, speaker_similarity};
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

use support::augment::{apply, Condition, Rng};
use support::corpus::{load_free_speech, load_groups, load_noise_by_category, Role};
use support::harness::TestEnv;

const CURRENT_EFFECTIVE_THRESHOLD: f32 = 0.68 - (0.65 - 0.5) * 0.24;

#[derive(Clone)]
struct Sample {
    condition: String,
    embedding: Vec<f32>,
}

#[derive(Default)]
struct Scores {
    positive: Vec<Sample>,
    impostor: Vec<Sample>,
    target_free_speech: Vec<Sample>,
    other_free_speech: Vec<Sample>,
}

struct Strategy {
    name: &'static str,
    enrollment_count: usize,
    centroids: BTreeMap<String, Vec<f32>>,
    enrollment_floors: BTreeMap<String, f32>,
}

#[derive(Clone, Copy)]
struct OperatingPoint {
    threshold: f32,
    tpr: f32,
    far: f32,
    frr: f32,
}

#[test]
#[ignore = "需要完整合成语料，见文件头"]
fn probes_speaker_enrollment_and_thresholds() {
    let groups = load_groups().expect("speaker corpus manifest");
    assert_eq!(groups.len(), 20, "expected the full 20-group corpus");
    let env = TestEnv::prepare().expect("speaker test environment");
    let model = env.speaker_model().expect("speaker model");
    let extractor = create_extractor(model);
    let conditions = conditions();
    let free_speech = load_free_speech();
    assert_eq!(
        free_speech.len(),
        45,
        "expected the full free-speech corpus"
    );

    let mut per_group = BTreeMap::<String, Scores>::new();
    let mut strategy_data = [
        ("first1", 1, BTreeMap::new(), BTreeMap::new()),
        ("first2", 2, BTreeMap::new(), BTreeMap::new()),
        ("first3", 3, BTreeMap::new(), BTreeMap::new()),
        ("first4", 4, BTreeMap::new(), BTreeMap::new()),
        ("balanced3", 3, BTreeMap::new(), BTreeMap::new()),
        ("robust3", 3, BTreeMap::new(), BTreeMap::new()),
        ("all6", 6, BTreeMap::new(), BTreeMap::new()),
    ];
    let mut enrollment_pair_scores = Vec::new();
    let mut rng = Rng::new(0x5350_4541_4B45_5201);

    let free_embeddings = free_speech
        .iter()
        .map(|path| {
            (
                path.clone(),
                embedding_from_path(&extractor, path)
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
            )
        })
        .collect::<Vec<_>>();

    for (group_index, group) in groups.iter().enumerate() {
        let enroll = group.by_role(Role::Enroll);
        assert_eq!(enroll.len(), 6, "{} enrollment fixtures", group.id);
        let enroll_embeddings = enroll
            .iter()
            .map(|item| {
                (
                    item.style.as_str(),
                    embedding_from_path(&extractor, &item.path)
                        .unwrap_or_else(|error| panic!("{}: {error}", item.path.display())),
                )
            })
            .collect::<Vec<_>>();
        enrollment_pair_scores.extend(pairwise_scores(
            &enroll_embeddings
                .iter()
                .map(|(_, embedding)| embedding.clone())
                .collect::<Vec<_>>(),
        ));

        let first3 = enroll_embeddings
            .iter()
            .take(3)
            .map(|(_, embedding)| embedding.clone())
            .collect::<Vec<_>>();
        let balanced3 = ["normal", "louder", "softer"]
            .iter()
            .map(|style| {
                enroll_embeddings
                    .iter()
                    .find(|(candidate, _)| candidate == style)
                    .unwrap_or_else(|| panic!("{} missing {style}", group.id))
                    .1
                    .clone()
            })
            .collect::<Vec<_>>();
        let robust3 = robust_three(
            &enroll_embeddings
                .iter()
                .map(|(_, embedding)| embedding.clone())
                .collect::<Vec<_>>(),
        );
        let all6 = enroll_embeddings
            .iter()
            .map(|(_, embedding)| embedding.clone())
            .collect::<Vec<_>>();
        let first1 = enroll_embeddings[..1]
            .iter()
            .map(|(_, embedding)| embedding.clone())
            .collect::<Vec<_>>();
        let first2 = enroll_embeddings[..2]
            .iter()
            .map(|(_, embedding)| embedding.clone())
            .collect::<Vec<_>>();
        let first4 = enroll_embeddings[..4]
            .iter()
            .map(|(_, embedding)| embedding.clone())
            .collect::<Vec<_>>();
        for ((_, _, centroids, floors), embeddings) in strategy_data
            .iter_mut()
            .zip([first1, first2, first3, first4, balanced3, robust3, all6])
        {
            floors.insert(group.id.clone(), leave_one_out_floor(&embeddings));
            centroids.insert(group.id.clone(), average_embeddings(&embeddings));
        }

        let scores = per_group.entry(group.id.clone()).or_default();
        for utterance in group.by_role(Role::Positive) {
            let base = read_wav_16k(&utterance.path).unwrap();
            for condition in &conditions {
                let samples = apply(&base, condition, &mut rng).unwrap();
                scores.positive.push(Sample {
                    condition: condition.id.clone(),
                    embedding: embedding_from_samples(&extractor, &samples)
                        .unwrap_or_else(|error| panic!("{}: {error}", utterance.path.display())),
                });
            }
        }
        for utterance in group.by_role(Role::Impostor) {
            let base = read_wav_16k(&utterance.path).unwrap();
            for condition in impostor_conditions(&conditions) {
                let samples = apply(&base, condition, &mut rng).unwrap();
                scores.impostor.push(Sample {
                    condition: condition.id.clone(),
                    embedding: embedding_from_samples(&extractor, &samples)
                        .unwrap_or_else(|error| panic!("{}: {error}", utterance.path.display())),
                });
            }
        }
        for (path, embedding) in &free_embeddings {
            let voice = free_speech_voice(path);
            let sample = Sample {
                condition: "free".to_owned(),
                embedding: embedding.clone(),
            };
            if voice.as_deref() == Some(group.target_voice.as_str()) {
                scores.target_free_speech.push(sample);
            } else {
                scores.other_free_speech.push(sample);
            }
        }
        eprintln!("[speaker] embeddings {}/{}", group_index + 1, groups.len());
    }

    let pair_mean = mean(&enrollment_pair_scores);
    let pair_min = enrollment_pair_scores
        .iter()
        .copied()
        .fold(1.0_f32, f32::min);
    println!(
        "\nEnrollment consistency: pairs={} mean={pair_mean:.4} min={pair_min:.4}",
        enrollment_pair_scores.len()
    );

    let strategies = strategy_data
        .into_iter()
        .map(
            |(name, enrollment_count, centroids, enrollment_floors)| Strategy {
                name,
                enrollment_count,
                centroids,
                enrollment_floors,
            },
        )
        .collect::<Vec<_>>();
    for strategy in &strategies {
        report_strategy(strategy, &per_group);
    }
}

fn conditions() -> Vec<Condition> {
    let noise = load_noise_by_category();
    let pick = |category: &str| noise.get(category).and_then(|paths| paths.first()).cloned();
    let mut conditions = vec![
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
        if let Some(path) = pick(category) {
            conditions.push(Condition {
                id: format!("{category}@{snr_db:.0}dB"),
                speed: 1.0,
                noise: Some(path),
                noise_category: category.to_owned(),
                snr_db,
                gain: 1.0,
            });
        }
    }
    conditions
}

fn impostor_conditions(conditions: &[Condition]) -> Vec<&Condition> {
    conditions
        .iter()
        .filter(|condition| {
            matches!(
                condition.id.as_str(),
                "clean" | "fast" | "quiet" | "crowd@10dB"
            )
        })
        .collect()
}

fn create_extractor(model: &Path) -> SpeakerEmbeddingExtractor {
    let mut config = SpeakerEmbeddingExtractorConfig::default();
    config.model = Some(model.display().to_string());
    config.num_threads = 2;
    SpeakerEmbeddingExtractor::create(&config).expect("speaker extractor")
}

fn embedding_from_path(
    extractor: &SpeakerEmbeddingExtractor,
    path: &Path,
) -> Result<Vec<f32>, String> {
    embedding_from_samples(extractor, &read_wav_16k(path)?)
}

fn embedding_from_samples(
    extractor: &SpeakerEmbeddingExtractor,
    samples: &[f32],
) -> Result<Vec<f32>, String> {
    let stream = extractor
        .create_stream()
        .ok_or_else(|| "failed to create speaker stream".to_owned())?;
    stream.accept_waveform(16_000, samples);
    if !extractor.is_ready(&stream) {
        return Err("speaker stream is not ready".to_owned());
    }
    extractor
        .compute(&stream)
        .ok_or_else(|| "failed to compute speaker embedding".to_owned())
}

fn robust_three(embeddings: &[Vec<f32>]) -> Vec<Vec<f32>> {
    assert!(embeddings.len() >= 3);
    let mut best = (f32::NEG_INFINITY, [0, 1, 2]);
    for first in 0..embeddings.len() - 2 {
        for second in first + 1..embeddings.len() - 1 {
            for third in second + 1..embeddings.len() {
                let score = [
                    speaker_similarity(&embeddings[first], &embeddings[second]),
                    speaker_similarity(&embeddings[first], &embeddings[third]),
                    speaker_similarity(&embeddings[second], &embeddings[third]),
                ]
                .into_iter()
                .fold(1.0_f32, f32::min);
                if score > best.0 {
                    best = (score, [first, second, third]);
                }
            }
        }
    }
    best.1
        .into_iter()
        .map(|index| embeddings[index].clone())
        .collect()
}

fn pairwise_scores(embeddings: &[Vec<f32>]) -> Vec<f32> {
    let mut scores = Vec::new();
    for left in 0..embeddings.len() {
        for right in left + 1..embeddings.len() {
            scores.push(speaker_similarity(&embeddings[left], &embeddings[right]));
        }
    }
    scores
}

fn leave_one_out_floor(embeddings: &[Vec<f32>]) -> f32 {
    if embeddings.len() < 2 {
        return 0.0;
    }
    (0..embeddings.len())
        .map(|held_out| {
            let others = embeddings
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != held_out)
                .map(|(_, embedding)| embedding.clone())
                .collect::<Vec<_>>();
            speaker_similarity(&average_embeddings(&others), &embeddings[held_out])
        })
        .fold(1.0_f32, f32::min)
}

fn report_strategy(strategy: &Strategy, groups: &BTreeMap<String, Scores>) {
    let mut positives = Vec::new();
    let mut impostors = Vec::new();
    let mut other_free = Vec::new();
    let mut target_free = Vec::new();
    let mut conditions = BTreeMap::<String, Vec<f32>>::new();
    for (group, samples) in groups {
        let centroid = &strategy.centroids[group];
        for sample in &samples.positive {
            let score = speaker_similarity(centroid, &sample.embedding);
            positives.push(score);
            conditions
                .entry(sample.condition.clone())
                .or_default()
                .push(score);
        }
        for sample in &samples.impostor {
            impostors.push(speaker_similarity(centroid, &sample.embedding));
        }
        for sample in &samples.other_free_speech {
            other_free.push(speaker_similarity(centroid, &sample.embedding));
        }
        for sample in &samples.target_free_speech {
            target_free.push(speaker_similarity(centroid, &sample.embedding));
        }
    }
    let negatives = impostors
        .iter()
        .chain(&other_free)
        .copied()
        .collect::<Vec<_>>();

    let current = operating_point(&positives, &negatives, CURRENT_EFFECTIVE_THRESHOLD);
    let eer = equal_error_point(&positives, &negatives);
    let far1 = best_at_far(&positives, &negatives, 0.01);
    let far5 = best_at_far(&positives, &negatives, 0.05);
    let impostor_far1 = best_at_far(&positives, &impostors, 0.01);
    let impostor_far5 = best_at_far(&positives, &impostors, 0.05);
    println!(
        "\n================ speaker strategy: {} ================",
        strategy.name
    );
    println!(
        "samples positive={} impostor={} other-free={} target-free={} AUC(all)={:.4} AUC(impostor)={:.4}",
        positives.len(),
        impostors.len(),
        other_free.len(),
        target_free.len(),
        auc(&positives, &negatives),
        auc(&positives, &impostors),
    );
    print_point("current", current);
    print_point("EER", eer);
    print_point("FAR<=1%", far1);
    print_point("FAR<=5%", far5);
    print_point("IMP FAR<=1%", impostor_far1);
    print_point("IMP FAR<=5%", impostor_far5);
    println!(
        "score distribution: positive mean={:.4} p05={:.4} | impostor mean={:.4} p95={:.4} | other-free mean={:.4} p95={:.4} | target-free mean={:.4}",
        mean(&positives),
        quantile(&positives, 0.05),
        mean(&impostors),
        quantile(&impostors, 0.95),
        mean(&other_free),
        quantile(&other_free, 0.95),
        mean(&target_free),
    );
    let current_impostor = operating_point(&positives, &impostors, CURRENT_EFFECTIVE_THRESHOLD);
    let current_other_free = operating_point(&positives, &other_free, CURRENT_EFFECTIVE_THRESHOLD);
    println!(
        "current split: impostor FAR={:.1}% | other-free FAR={:.1}%",
        current_impostor.far * 100.0,
        current_other_free.far * 100.0
    );
    if strategy.enrollment_count >= 2 {
        println!("-- per-speaker threshold = leave-one-out enrollment floor - margin --");
        for margin in [0.02, 0.05, 0.08, 0.12] {
            let point = adaptive_point(strategy, groups, margin);
            println!(
                "  margin={margin:.2} threshold mean={:.4} TPR={:>6.1}% impostor FAR={:>6.1}% other-free FAR={:>6.1}%",
                point.0,
                point.1 * 100.0,
                point.2 * 100.0,
                point.3 * 100.0,
            );
        }
    }
    println!("-- positive TPR at current effective threshold --");
    for (condition, scores) in conditions {
        let accepted = scores
            .iter()
            .filter(|score| **score >= CURRENT_EFFECTIVE_THRESHOLD)
            .count();
        println!(
            "  {condition:<16} {:>3}/{:<3} {:>6.1}% mean={:.4} p05={:.4}",
            accepted,
            scores.len(),
            accepted as f32 * 100.0 / scores.len().max(1) as f32,
            mean(&scores),
            quantile(&scores, 0.05),
        );
    }

    fn adaptive_point(
        strategy: &Strategy,
        groups: &BTreeMap<String, Scores>,
        margin: f32,
    ) -> (f32, f32, f32, f32) {
        let mut thresholds = Vec::new();
        let mut positive = (0_usize, 0_usize);
        let mut impostor = (0_usize, 0_usize);
        let mut other_free = (0_usize, 0_usize);
        for (group, samples) in groups {
            let centroid = &strategy.centroids[group];
            let threshold = (strategy.enrollment_floors[group] - margin).clamp(0.0, 1.0);
            thresholds.push(threshold);
            for sample in &samples.positive {
                positive.1 += 1;
                if speaker_similarity(centroid, &sample.embedding) >= threshold {
                    positive.0 += 1;
                }
            }
            for sample in &samples.impostor {
                impostor.1 += 1;
                if speaker_similarity(centroid, &sample.embedding) >= threshold {
                    impostor.0 += 1;
                }
            }
            for sample in &samples.other_free_speech {
                other_free.1 += 1;
                if speaker_similarity(centroid, &sample.embedding) >= threshold {
                    other_free.0 += 1;
                }
            }
        }
        (
            mean(&thresholds),
            positive.0 as f32 / positive.1.max(1) as f32,
            impostor.0 as f32 / impostor.1.max(1) as f32,
            other_free.0 as f32 / other_free.1.max(1) as f32,
        )
    }
}

fn print_point(label: &str, point: OperatingPoint) {
    println!(
        "{label:<10} threshold={:.4} TPR={:>6.1}% FAR={:>6.1}% FRR={:>6.1}%",
        point.threshold,
        point.tpr * 100.0,
        point.far * 100.0,
        point.frr * 100.0,
    );
}

fn operating_point(positive: &[f32], negative: &[f32], threshold: f32) -> OperatingPoint {
    let tpr = positive.iter().filter(|score| **score >= threshold).count() as f32
        / positive.len().max(1) as f32;
    let far = negative.iter().filter(|score| **score >= threshold).count() as f32
        / negative.len().max(1) as f32;
    OperatingPoint {
        threshold,
        tpr,
        far,
        frr: 1.0 - tpr,
    }
}

fn candidate_thresholds(positive: &[f32], negative: &[f32]) -> Vec<f32> {
    let mut values = positive.iter().chain(negative).copied().collect::<Vec<_>>();
    values.extend([0.0, 1.0]);
    values.sort_by(|left, right| right.partial_cmp(left).unwrap());
    values.dedup_by(|left, right| (*left - *right).abs() < 1e-6);
    values
}

fn equal_error_point(positive: &[f32], negative: &[f32]) -> OperatingPoint {
    candidate_thresholds(positive, negative)
        .into_iter()
        .map(|threshold| operating_point(positive, negative, threshold))
        .min_by(|left, right| {
            (left.far - left.frr)
                .abs()
                .partial_cmp(&(right.far - right.frr).abs())
                .unwrap()
        })
        .unwrap()
}

fn best_at_far(positive: &[f32], negative: &[f32], limit: f32) -> OperatingPoint {
    candidate_thresholds(positive, negative)
        .into_iter()
        .map(|threshold| operating_point(positive, negative, threshold))
        .filter(|point| point.far <= limit + 1e-6)
        .max_by(|left, right| left.tpr.partial_cmp(&right.tpr).unwrap())
        .unwrap()
}

fn auc(positive: &[f32], negative: &[f32]) -> f32 {
    let mut wins = 0.0_f64;
    for positive_score in positive {
        for negative_score in negative {
            wins += if positive_score > negative_score {
                1.0
            } else if (positive_score - negative_score).abs() < 1e-6 {
                0.5
            } else {
                0.0
            };
        }
    }
    (wins / (positive.len().max(1) * negative.len().max(1)) as f64) as f32
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

fn quantile(values: &[f32], position: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let index = ((sorted.len() - 1) as f32 * position.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}

fn free_speech_voice(path: &PathBuf) -> Option<String> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split('_').nth(1))
        .map(str::to_owned)
}
