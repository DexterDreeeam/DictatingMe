//! DictatingMe 开发期工具。
//!
//! 目前只有一个子命令：
//!
//! ```text
//! cargo run -p xtask --release -- corpus [--force] [--groups w01,w02]
//! ```
//!
//! 它按 `assets/assets-manifest.json` 的配方，用 sherpa-onnx 的中文 VITS 模型
//! 合成唤醒词 E2E 测试语料，落到 `assets/corpus/<组>/` 与 `assets/freespeech/`。
//! 所有产物都被 .gitignore 排除，不进仓库。

mod audio;
mod manifest;
mod tts;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use audio::Rng;
use manifest::Manifest;
use tts::Synth;

const TARGET_PEAK: f32 = 0.62;
const ROOM_TONE: f32 = 0.0016;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let command = args.first().map(String::as_str).unwrap_or("help");
    let result = match command {
        "corpus" => run_corpus(&args[1..]),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command '{other}'; try `xtask help`")),
    };
    if let Err(error) = result {
        eprintln!("xtask: {error}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "xtask commands:\n  \
         corpus [--force] [--groups w01,w05]   合成唤醒词 E2E 语料到 assets/corpus 与 assets/freespeech"
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask crate always sits one level below the repo root")
        .to_path_buf()
}

struct CorpusOptions {
    force: bool,
    groups: Option<Vec<String>>,
}

fn parse_corpus_options(args: &[String]) -> Result<CorpusOptions, String> {
    let mut options = CorpusOptions {
        force: false,
        groups: None,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--force" => options.force = true,
            "--groups" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--groups needs a comma separated list".to_owned())?;
                options.groups = Some(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(str::to_owned)
                        .collect(),
                );
            }
            other => return Err(format!("unknown corpus flag '{other}'")),
        }
        index += 1;
    }
    Ok(options)
}

fn run_corpus(args: &[String]) -> Result<(), String> {
    let options = parse_corpus_options(args)?;
    let root = repo_root();
    let assets = root.join("assets");
    let manifest = Manifest::load(&assets.join("assets-manifest.json"))?;
    let total_samples = (manifest.sample_rate as u64 * u64::from(manifest.recording_duration_ms)
        / 1_000) as usize;

    // 只打开真正会用到的模型；每个模型加载一次，全程复用。
    let mut synths: BTreeMap<String, Synth> = BTreeMap::new();
    for model in &manifest.tts_models {
        let model_root = assets.join(&model.root);
        let synth = Synth::open(&model_root)?;
        println!(
            "[tts ] {:<26} sr={} speakers={}",
            model.id,
            synth.sample_rate(),
            synth.num_speakers
        );
        synths.insert(model.id.clone(), synth);
    }
    for voice in &manifest.voices {
        let synth = synths
            .get(&voice.model)
            .ok_or_else(|| format!("voice '{}' references unknown model '{}'", voice.id, voice.model))?;
        if voice.sid >= synth.num_speakers {
            return Err(format!(
                "voice '{}' uses sid {} but model '{}' only has {} speakers",
                voice.id, voice.sid, voice.model, synth.num_speakers
            ));
        }
    }

    let corpus_dir = assets.join("corpus");
    let mut written = 0_usize;
    let mut skipped = 0_usize;

    for group in &manifest.groups {
        if let Some(filter) = &options.groups {
            if !filter.iter().any(|item| item == &group.id) {
                continue;
            }
        }
        // 中文 VITS 的词表里没有拉丁字母，遇到会静默丢弃——那样唤醒词和对立词
        // 会合成出一模一样的音频，整组测试直接失效。这里提前拦下来。
        for (label, text) in [("phrase", &group.phrase), ("confusable", &group.confusable)] {
            if let Some(bad) = text.chars().find(char::is_ascii_alphanumeric) {
                return Err(format!(
                    "group '{}' {label} 含有 '{bad}'：中文 TTS 词表会丢弃拉丁字母/数字，请换成汉字",
                    group.id
                ));
            }
        }
        let group_dir = corpus_dir.join(format!("{}-{}", group.id, group.slug));
        std::fs::create_dir_all(&group_dir)
            .map_err(|error| format!("failed to create '{}': {error}", group_dir.display()))?;

        // 每个组一条独立的随机流，改动别的组不会影响本组已有素材。
        let mut rng = Rng::new(manifest.seed ^ fnv1a(&group.id));

        for role in &manifest.roles {
            let text = match role.text.as_str() {
                "phrase" => &group.phrase,
                "confusable" => &group.confusable,
                other => return Err(format!("unknown role text source '{other}'")),
            };
            for (index, style_id) in role.styles.iter().enumerate() {
                let style = manifest.style(style_id)?;
                let voice_id = match role.source.as_str() {
                    "target" => group.target_voice.clone(),
                    "impostor" => group
                        .impostor_voices
                        .get(index % group.impostor_voices.len().max(1))
                        .cloned()
                        .ok_or_else(|| format!("group '{}' has no impostor voices", group.id))?,
                    other => return Err(format!("unknown role source '{other}'")),
                };
                let voice = manifest.voice(&voice_id)?;
                let file = group_dir.join(format!(
                    "{}_{}_{}_{}_{:02}.wav",
                    group.id,
                    voice_id,
                    role.id,
                    style_id,
                    index + 1
                ));
                if file.exists() && !options.force {
                    skipped += 1;
                    continue;
                }
                let synth = synths
                    .get(&voice.model)
                    .ok_or_else(|| format!("missing synth for model '{}'", voice.model))?;

                // 同一 style 下再叠一点点语速抖动，避免 6 条 enroll 完全同构。
                let jitter = rng.range(0.96, 1.04);
                let mut speech = synth.say(text, voice.sid, style.speed * jitter)?;
                let trimmed = audio::trim_silence(&speech, 0.012).to_vec();
                speech = if trimmed.len() > 800 { trimmed } else { speech };
                audio::normalize_peak(&mut speech, TARGET_PEAK);
                if style.farfield {
                    speech = audio::far_field(&speech, 16_000);
                    audio::normalize_peak(&mut speech, TARGET_PEAK);
                }
                for sample in speech.iter_mut() {
                    *sample *= style.gain;
                }
                let track = audio::place_in_window(&speech, total_samples, ROOM_TONE, &mut rng);
                audio::write_wav_16k(&file, &track)?;
                written += 1;
            }
        }
        println!(
            "[corp] {} {:<18} {:<8} 对立词 {} ({})",
            group.id, group.slug, group.phrase, group.confusable, group.confusable_tier
        );
    }

    // 自由语音：用于 FA/h（每小时误唤醒次数）测量，内容与任何唤醒词无关。
    if options.groups.is_none() {
        let free_dir = assets.join(&manifest.free_speech.dir);
        std::fs::create_dir_all(&free_dir)
            .map_err(|error| format!("failed to create '{}': {error}", free_dir.display()))?;
        let mut rng = Rng::new(manifest.seed ^ fnv1a("freespeech"));
        for (index, text) in manifest.free_speech.utterances.iter().enumerate() {
            for voice_id in &manifest.free_speech.voices {
                let voice = manifest.voice(voice_id)?;
                let file = free_dir.join(format!("free_{voice_id}_{:02}.wav", index + 1));
                if file.exists() && !options.force {
                    skipped += 1;
                    continue;
                }
                let synth = synths
                    .get(&voice.model)
                    .ok_or_else(|| format!("missing synth for model '{}'", voice.model))?;
                let mut speech = synth.say(text, voice.sid, rng.range(0.92, 1.1))?;
                audio::normalize_peak(&mut speech, TARGET_PEAK);
                let padded = speech.len() + 16_000 / 2;
                let track = audio::place_in_window(&speech, padded, ROOM_TONE, &mut rng);
                audio::write_wav_16k(&file, &track)?;
                written += 1;
            }
        }
        println!("[free] {} 条自由语音", manifest.free_speech.utterances.len() * manifest.free_speech.voices.len());
    }

    println!("[done] 新写入 {written} 条，跳过已存在 {skipped} 条 -> {}", corpus_dir.display());
    Ok(())
}

fn fnv1a(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
