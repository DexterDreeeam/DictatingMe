use std::path::{Path, PathBuf};

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig, OfflineTtsVitsModelConfig,
};

pub struct Synth {
    tts: OfflineTts,
    sample_rate: u32,
    pub num_speakers: i32,
    pub label: String,
}

impl Synth {
    /// 从解压后的 sherpa-onnx VITS 模型目录自动装配配置。
    /// 目录结构由上游发布包决定，这里不写死文件名，只做存在性探测。
    pub fn open(root: &Path) -> Result<Self, String> {
        if !root.is_dir() {
            return Err(format!(
                "TTS model directory is missing: {} —— 先跑 assets\\download-assets.cmd 与 `cargo run -p xtask -- fetch-tts`",
                root.display()
            ));
        }
        let onnx = single_onnx(root)?;
        let mut vits = OfflineTtsVitsModelConfig::default();
        vits.model = Some(path_string(&onnx));
        vits.lexicon = optional_file(root, "lexicon.txt");
        vits.tokens = optional_file(root, "tokens.txt");
        if root.join("dict").is_dir() {
            vits.dict_dir = Some(path_string(&root.join("dict")));
        }

        let mut model = OfflineTtsModelConfig::default();
        model.vits = vits;
        model.num_threads = 2;
        model.debug = false;
        model.provider = Some("cpu".to_owned());

        let rule_fsts = ["phone.fst", "date.fst", "number.fst"]
            .into_iter()
            .filter_map(|name| optional_file(root, name))
            .collect::<Vec<_>>();

        let mut config = OfflineTtsConfig::default();
        config.model = model;
        config.max_num_sentences = 1;
        config.silence_scale = 0.2;
        if !rule_fsts.is_empty() {
            config.rule_fsts = Some(rule_fsts.join(","));
        }

        let tts = OfflineTts::create(&config)
            .ok_or_else(|| format!("failed to create TTS from {}", root.display()))?;
        let sample_rate = u32::try_from(tts.sample_rate()).unwrap_or(0);
        if sample_rate == 0 {
            return Err(format!("TTS at {} reported sample rate 0", root.display()));
        }
        let num_speakers = tts.num_speakers();
        let label = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        Ok(Self {
            tts,
            sample_rate,
            num_speakers,
            label,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 合成一段语音并重采样到 16 kHz。
    pub fn say(&self, text: &str, sid: i32, speed: f32) -> Result<Vec<f32>, String> {
        let mut config = GenerationConfig::default();
        config.sid = sid;
        config.speed = speed;
        config.silence_scale = 0.2;
        let audio = self
            .tts
            .generate_with_config(text, &config, None::<fn(&[f32], f32) -> bool>)
            .ok_or_else(|| format!("[{}] TTS returned nothing for '{text}'", self.label))?;
        let samples = audio.samples().to_vec();
        if samples.is_empty() {
            return Err(format!(
                "[{}] TTS produced 0 samples for '{text}' (sid={sid}) —— 词表里可能没有这些字符",
                self.label
            ));
        }
        Ok(crate::audio::resample_linear(
            &samples,
            self.sample_rate,
            16_000,
        ))
    }
}

fn single_onnx(root: &Path) -> Result<PathBuf, String> {
    let mut found = std::fs::read_dir(root)
        .map_err(|error| format!("failed to list '{}': {error}", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "onnx"))
        .collect::<Vec<_>>();
    found.sort();
    match found.len() {
        0 => Err(format!("no .onnx model found in {}", root.display())),
        1 => Ok(found.remove(0)),
        _ => Err(format!(
            "expected exactly one .onnx in {}, found {}",
            root.display(),
            found.len()
        )),
    }
}

fn optional_file(root: &Path, name: &str) -> Option<String> {
    let path = root.join(name);
    path.is_file().then(|| path_string(&path))
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}
