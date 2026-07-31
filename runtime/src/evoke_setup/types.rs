use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvokeMode {
    Text,
    VoiceMatch,
    SpeakerVerify,
    Classifier,
}

impl EvokeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::VoiceMatch => "voiceMatch",
            Self::SpeakerVerify => "speakerVerify",
            Self::Classifier => "classifier",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "voiceMatch" => Some(Self::VoiceMatch),
            "speakerVerify" => Some(Self::SpeakerVerify),
            "classifier" => Some(Self::Classifier),
            _ => None,
        }
    }

    pub fn required_recordings(self) -> u8 {
        match self {
            Self::Text => 0,
            Self::VoiceMatch | Self::SpeakerVerify => 3,
            Self::Classifier => 6,
        }
    }

    /// KWS 解码保留的候选路径数（beam 宽度）。
    ///
    /// 放宽 beam 会让 KWS 多放进一些候选，代价按模式而不同：
    /// text 模式 KWS 命中即唤醒，多放进来的日常语音会直接变成误唤醒；
    /// voiceMatch 有 DTW 作第二级，实测无论 beam 取 4/8/16/32，
    /// 日常说话的误触率都被稳定压在 1% 左右，代价只落在近音词上。
    /// 因此 voiceMatch 可以放宽，其余模式保持 sherpa-onnx 的默认值。
    pub fn kws_max_active_paths(self) -> i32 {
        match self {
            Self::VoiceMatch => 16,
            _ => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvokeSetupPhase {
    Draft,
    Capturing,
    ReadyToProcess,
    Processing,
    Committed,
    Failed,
    Cancelled,
}

impl EvokeSetupPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Capturing => "capturing",
            Self::ReadyToProcess => "readyToProcess",
            Self::Processing => "processing",
            Self::Committed => "committed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "capturing" => Some(Self::Capturing),
            "readyToProcess" => Some(Self::ReadyToProcess),
            "processing" => Some(Self::Processing),
            "committed" => Some(Self::Committed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartEvokeSetup {
    pub mode: EvokeMode,
    pub phrase: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPrompt {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentPlan {
    pub mode: EvokeMode,
    pub required_recordings: u8,
    pub recording_duration_ms: u32,
    pub prompts: Vec<RecordingPrompt>,
    pub required_asset_ids: Vec<String>,
}

impl EnrollmentPlan {
    pub fn for_mode(mode: EvokeMode, required_asset_ids: Vec<String>) -> Self {
        let prompts = [
            ("normal", "正常音量说出唤醒词"),
            ("louder", "提高音量再次录制"),
            ("softer", "降低音量再次录制"),
            ("faster", "语速较快地说出唤醒词"),
            ("slower", "语速较慢地说出唤醒词"),
            ("farther", "稍远距离再次录制"),
        ]
        .into_iter()
        .take(mode.required_recordings() as usize)
        .map(|(id, text)| RecordingPrompt {
            id: id.to_owned(),
            text: text.to_owned(),
        })
        .collect();
        Self {
            mode,
            required_recordings: mode.required_recordings(),
            recording_duration_ms: 5_000,
            prompts,
            required_asset_ids,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingQuality {
    pub accepted: bool,
    pub rms: f32,
    pub peak: f32,
    pub clipping_ratio: f32,
    pub voiced_ratio: f32,
    pub rejection: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvokeSetupSession {
    pub id: String,
    pub mode: EvokeMode,
    pub phrase: String,
    pub phase: EvokeSetupPhase,
    pub plan: EnrollmentPlan,
    pub completed_recordings: u8,
    pub operation_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingReceipt {
    pub setup_id: String,
    pub index: u8,
    pub duration_ms: u32,
    pub quality: RecordingQuality,
    pub completed_recordings: u8,
    pub remaining_recordings: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvokeProfile {
    pub id: String,
    pub mode: EvokeMode,
    pub phrase: String,
    pub threshold: f32,
    pub artifact: EvokeArtifact,
    pub required_asset_ids: Vec<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvokeProfileSummary {
    pub id: String,
    pub mode: EvokeMode,
    pub phrase: String,
    pub created_at_ms: u64,
}

impl From<&EvokeProfile> for EvokeProfileSummary {
    fn from(profile: &EvokeProfile) -> Self {
        Self {
            id: profile.id.clone(),
            mode: profile.mode,
            phrase: profile.phrase.clone(),
            created_at_ms: profile.created_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EvokeArtifact {
    Text { keyword_syntax: String },
    VoiceMatch { template: Vec<Vec<f32>> },
    SpeakerVerify { centroid: Vec<f32> },
    Classifier { weights: Vec<f32>, bias: f32 },
}
