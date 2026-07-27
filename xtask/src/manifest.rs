use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub seed: u64,
    pub sample_rate: u32,
    pub recording_duration_ms: u32,
    pub tts_models: Vec<TtsModel>,
    pub voices: Vec<Voice>,
    pub styles: Vec<Style>,
    pub roles: Vec<Role>,
    pub groups: Vec<Group>,
    pub free_speech: FreeSpeech,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsModel {
    pub id: String,
    pub url: String,
    pub root: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub id: String,
    pub model: String,
    pub sid: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    pub id: String,
    pub speed: f32,
    pub gain: f32,
    pub farfield: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    pub id: String,
    /// `phrase` 或 `confusable`
    pub text: String,
    /// `target` 或 `impostor`
    pub source: String,
    pub styles: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub slug: String,
    pub phrase: String,
    pub confusable: String,
    pub confusable_tier: String,
    pub target_voice: String,
    pub impostor_voices: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeSpeech {
    pub dir: String,
    pub voices: Vec<String>,
    pub utterances: Vec<String>,
}

impl Manifest {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read '{}': {error}", path.display()))?;
        let manifest: Manifest = serde_json::from_str(&raw)
            .map_err(|error| format!("failed to parse '{}': {error}", path.display()))?;
        if manifest.schema_version != 1 {
            return Err(format!(
                "unsupported assets-manifest schemaVersion {}",
                manifest.schema_version
            ));
        }
        Ok(manifest)
    }

    pub fn voice(&self, id: &str) -> Result<&Voice, String> {
        self.voices
            .iter()
            .find(|voice| voice.id == id)
            .ok_or_else(|| format!("unknown voice id '{id}'"))
    }

    pub fn style(&self, id: &str) -> Result<&Style, String> {
        self.styles
            .iter()
            .find(|style| style.id == id)
            .ok_or_else(|| format!("unknown style id '{id}'"))
    }
}
