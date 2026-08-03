//! 从 `assets/assets-manifest.json` + `assets/corpus/` 装载测试语料。
//!
//! 文件名约定：`<组>_<说话人>_<角色>_<风格>_<序号>.wav`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    groups: Vec<ManifestGroup>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestGroup {
    id: String,
    slug: String,
    phrase: String,
    confusable: String,
    confusable_tier: String,
    target_voice: String,
}

/// 一条素材在测试里的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// 注册阶段的输入
    Enroll,
    /// 本人说唤醒词——应当被接受
    Positive,
    /// 别人说唤醒词——SpeakerVerify 应当拒绝
    Impostor,
    /// 本人说对立词——应当被拒绝
    Confusable,
}

impl Role {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "enroll" => Some(Self::Enroll),
            "pos" => Some(Self::Positive),
            "imp" => Some(Self::Impostor),
            "cfz" => Some(Self::Confusable),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enroll => "enroll",
            Self::Positive => "pos",
            Self::Impostor => "imp",
            Self::Confusable => "cfz",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Utterance {
    pub path: PathBuf,
    pub voice: String,
    pub role: Role,
    pub style: String,
}

#[derive(Debug, Clone)]
pub struct WakeGroup {
    pub id: String,
    pub phrase: String,
    pub confusable: String,
    pub tier: String,
    pub target_voice: String,
    pub utterances: Vec<Utterance>,
}

impl WakeGroup {
    pub fn by_role(&self, role: Role) -> Vec<&Utterance> {
        let mut items = self
            .utterances
            .iter()
            .filter(|item| item.role == role)
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.path.cmp(&right.path));
        items
    }
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("runtime crate always sits one level below the repo root")
        .to_path_buf()
}

pub fn assets_dir() -> PathBuf {
    repo_root().join("assets")
}

/// 装载全部分组。语料缺失时返回空表，由调用方决定是 skip 还是失败。
pub fn load_groups() -> Result<Vec<WakeGroup>, String> {
    let assets = assets_dir();
    let manifest_path = assets.join("assets-manifest.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read '{}': {error}", manifest_path.display()))?;
    let manifest: ManifestFile = serde_json::from_str(&raw)
        .map_err(|error| format!("failed to parse '{}': {error}", manifest_path.display()))?;

    let filter = std::env::var("DM_E2E_GROUPS").ok().map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });

    let mut groups = Vec::new();
    for entry in manifest.groups {
        if let Some(filter) = &filter {
            if !filter.iter().any(|item| item == &entry.id) {
                continue;
            }
        }
        let dir = assets
            .join("corpus")
            .join(format!("{}-{}", entry.id, entry.slug));
        if !dir.is_dir() {
            continue;
        }
        let mut utterances = Vec::new();
        for file in std::fs::read_dir(&dir)
            .map_err(|error| format!("failed to list '{}': {error}", dir.display()))?
        {
            let path = file
                .map_err(|error| format!("failed to read '{}': {error}", dir.display()))?
                .path();
            if path.extension().is_none_or(|ext| ext != "wav") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let parts = stem.split('_').collect::<Vec<_>>();
            if parts.len() != 5 {
                return Err(format!(
                    "corpus file '{}' does not follow <group>_<voice>_<role>_<style>_<idx>.wav",
                    path.display()
                ));
            }
            let Some(role) = Role::parse(parts[2]) else {
                return Err(format!(
                    "corpus file '{}' has unknown role '{}'",
                    path.display(),
                    parts[2]
                ));
            };
            utterances.push(Utterance {
                voice: parts[1].to_owned(),
                role,
                style: parts[3].to_owned(),
                path,
            });
        }
        utterances.sort_by(|left, right| left.path.cmp(&right.path));
        if utterances.is_empty() {
            continue;
        }
        groups.push(WakeGroup {
            id: entry.id,
            phrase: entry.phrase,
            confusable: entry.confusable,
            tier: entry.confusable_tier,
            target_voice: entry.target_voice,
            utterances,
        });
    }
    Ok(groups)
}

/// 噪声素材按文件名前缀归类（`crowd-babble-train-1.wav` -> `crowd`）。
pub fn load_noise_by_category() -> BTreeMap<String, Vec<PathBuf>> {
    let mut result: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let dir = assets_dir().join("noise");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return result;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "wav") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let category = name.split('-').next().unwrap_or("other").to_owned();
        result.entry(category).or_default().push(path);
    }
    for paths in result.values_mut() {
        paths.sort();
    }
    result
}

pub fn load_free_speech() -> Vec<PathBuf> {
    let dir = assets_dir().join("freespeech");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}
