//! 复用产品代码的 E2E 夹具：资产准备 -> setup -> detect，与 `runtime.rs` 的真实回路一致。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dictatingme_runtime::audio::AudioFrame;
use dictatingme_runtime::evoke_setup::{processor_for, EvokeMode, EvokeProfile, ProcessInput};
use dictatingme_runtime::models::EvokeModelEngine;
use dictatingme_runtime::scoring::{EvokeScore, ScoringSystem};
use dictatingme_runtime::storage::{
    AppPaths, AssetGroup, AssetInstallRequest, AssetKind, AssetManager,
};

use super::corpus::assets_dir;

/// 每帧 100 ms，与 `AudioCapture` 的真实分帧一致。
const FRAME_SAMPLES: usize = 1_600;
const FRAME_MS: u64 = 100;

pub struct TestEnv {
    pub assets: AssetManager,
    runtime: tokio::runtime::Runtime,
    speaker_model: Option<PathBuf>,
    kws_model_dir: PathBuf,
}

impl TestEnv {
    /// 在 `assets/.e2e-home` 下搭一套真实的 AppPaths。
    /// 目录常驻，跨多次测试复用已下载的资产。
    pub fn prepare() -> Result<Self, String> {
        let home = assets_dir().join(".e2e-home");
        std::fs::create_dir_all(&home)
            .map_err(|error| format!("failed to create '{}': {error}", home.display()))?;
        let paths = AppPaths::new(home);
        paths
            .ensure()
            .map_err(|error| format!("failed to prepare app paths: {error}"))?;

        let catalog = assets_dir().join("sha.json");
        let assets = AssetManager::load(paths, &catalog)
            .map_err(|error| format!("failed to load asset catalog: {error}"))?;
        assets
            .bootstrap_embedded()
            .map_err(|error| format!("failed to bootstrap embedded preset: {error}"))?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("failed to build tokio runtime: {error}"))?;

        let kws_model_dir = {
            let descriptor = assets
                .first_descriptor_of_kind(AssetKind::PresetEvoke)
                .map_err(|error| format!("preset evoke asset is missing: {error}"))?;
            assets
                .asset_path(descriptor)
                .map_err(|error| format!("failed to resolve preset path: {error}"))?
        };

        let mut env = Self {
            assets,
            runtime,
            speaker_model: None,
            kws_model_dir,
        };
        env.ensure_group(AssetGroup::ClassifierRecognition)?;
        env.speaker_model = env.ensure_group(AssetGroup::SpeakerRecognition).ok();
        Ok(env)
    }

    /// 资产不在本地时走产品自己的下载流程装上。
    fn ensure_group(&self, group: AssetGroup) -> Result<PathBuf, String> {
        let descriptor = self
            .assets
            .primary_descriptor(group)
            .map_err(|error| format!("{group:?} descriptor missing: {error}"))?
            .clone();
        let path = self
            .assets
            .asset_path(&descriptor)
            .map_err(|error| format!("{group:?} path unresolved: {error}"))?;
        // 与 processor.rs 的 primary_asset_file 一致：单文件资产要指到文件本身。
        let resolved = match descriptor.output_file.as_ref() {
            Some(file) => path.join(file),
            None => path.clone(),
        };
        if resolved.exists() {
            return Ok(resolved);
        }
        eprintln!("[e2e] downloading {} ...", descriptor.id);
        let request = AssetInstallRequest {
            asset_link_list: descriptor.sources.clone(),
            asset_path: path.to_string_lossy().into_owned(),
        };
        self.runtime
            .block_on(self.assets.install(request, Arc::new(|_| {})))
            .map_err(|error| format!("failed to install {}: {error}", descriptor.id))?;
        Ok(resolved)
    }

    pub fn speaker_model(&self) -> Option<&Path> {
        self.speaker_model.as_deref()
    }

    pub fn kws_model_dir(&self) -> &Path {
        &self.kws_model_dir
    }

    /// setup 阶段：与 `evoke_setup` 命令走同一条 `processor_for(...).process(...)`。
    pub fn run_setup(
        &self,
        mode: EvokeMode,
        phrase: &str,
        recordings: Vec<PathBuf>,
    ) -> Result<EvokeProfile, String> {
        let input = ProcessInput {
            mode,
            phrase: phrase.to_owned(),
            recording_paths: recordings,
        };
        self.runtime
            .block_on(processor_for(mode).process(input, &self.assets))
            .map_err(|error| error.to_string())
    }

    /// 按 sherpa-onnx 默认 beam(4) 建 spotter，与既有探针的口径一致。
    pub fn new_spotter(&self, phrase: &str) -> Result<EvokeModelEngine, String> {
        self.new_spotter_with_beam(phrase, 4)
    }

    /// 指定 beam 宽度建 spotter。voiceMatch 的测试用这个。
    pub fn new_spotter_with_beam(
        &self,
        phrase: &str,
        max_active_paths: i32,
    ) -> Result<EvokeModelEngine, String> {
        let path = self
            .kws_model_dir
            .to_str()
            .ok_or_else(|| "preset path is not valid UTF-8".to_owned())?;
        EvokeModelEngine::new(path, phrase.to_owned(), max_active_paths)
            .map_err(|error| error.0)
    }
}

#[derive(Debug, Clone)]
pub struct DetectOutcome {
    pub woke: bool,
    pub best: EvokeScore,
    pub keyword_hits: u32,
}

/// detect 阶段：逐帧复刻 `runtime.rs` 的 `State::Listening` 分支。
pub fn detect(
    spotter: &mut EvokeModelEngine,
    profile: &EvokeProfile,
    speaker_model: Option<&Path>,
    sensitivity: f32,
    samples: &[f32],
) -> Result<DetectOutcome, String> {
    spotter.set_sensitivity(sensitivity);
    spotter.reset();
    let mut scoring = ScoringSystem::new(profile.clone(), sensitivity, speaker_model)?;

    let mut woke = false;
    let mut keyword_hits = 0u32;
    let mut best: Option<EvokeScore> = None;

    for (index, chunk) in samples.chunks(FRAME_SAMPLES).enumerate() {
        let frame = AudioFrame {
            samples: chunk
                .iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * 32_767.0) as i16)
                .collect(),
            sample_rate: 16_000,
            timestamp_ms: index as u64 * FRAME_MS,
        };
        // 与 runtime.rs 的 State::Listening 分支一致：只在 KWS 命中的那一帧做完整打分。
        let candidate = spotter.process_frame(&frame).is_some();
        scoring.push_frame(&frame, candidate);
        if !candidate {
            continue;
        }
        keyword_hits += 1;
        let score = scoring.evaluate_candidate();
        if score.accepted {
            woke = true;
        }
        if best
            .as_ref()
            .is_none_or(|current| score.overall > current.overall)
        {
            best = Some(score);
        }
    }

    // 整段都没有 KWS 命中时补一次预览分，供诊断用。
    let best = match best {
        Some(score) => score,
        None => scoring.preview(),
    };
    Ok(DetectOutcome {
        woke,
        best,
        keyword_hits,
    })
}
