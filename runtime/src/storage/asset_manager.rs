use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bzip2::read::BzDecoder;
use futures_util::{stream::FuturesUnordered, StreamExt};
use reqwest::header::RANGE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{AppPaths, StorageError};

const EMBEDDED_CATALOG: &str = include_str!("../../../assets/sha.json");
const SOURCE_PROBE_BYTES: usize = 256 * 1024;
const SOURCE_PROBE_MIN_BYTES: usize = 64 * 1024;
const SOURCE_PROBE_TIMEOUT: Duration = Duration::from_secs(12);
const SOURCE_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const EMBEDDED_PRESET_FILES: &[(&str, &[u8])] = &[
    (
        "encoder-epoch-12-avg-2-chunk-16-left-64.onnx",
        include_bytes!(
            "../../../assets/preset/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01/encoder-epoch-12-avg-2-chunk-16-left-64.onnx"
        ),
    ),
    (
        "decoder-epoch-12-avg-2-chunk-16-left-64.onnx",
        include_bytes!(
            "../../../assets/preset/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01/decoder-epoch-12-avg-2-chunk-16-left-64.onnx"
        ),
    ),
    (
        "joiner-epoch-12-avg-2-chunk-16-left-64.onnx",
        include_bytes!(
            "../../../assets/preset/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01/joiner-epoch-12-avg-2-chunk-16-left-64.onnx"
        ),
    ),
    (
        "tokens.txt",
        include_bytes!(
            "../../../assets/preset/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01/tokens.txt"
        ),
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetKind {
    PresetEvoke,
    DictationModel,
    SpeakerEmbedding,
    ClassifierResource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetGroup {
    SpeakerRecognition,
    ClassifierRecognition,
    SpeechModels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetFormat {
    Directory,
    File,
    TarBz2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFileDescriptor {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDescriptor {
    pub id: String,
    pub kind: AssetKind,
    #[serde(default)]
    pub display_name: String,
    pub version: String,
    pub bundled: bool,
    pub format: AssetFormat,
    pub install_path: String,
    #[serde(default)]
    pub archive_root: Option<String>,
    #[serde(default)]
    pub output_file: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    pub files: Vec<AssetFileDescriptor>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalizedAssetManifest {
    schema_version: u32,
    locale: String,
    speaker_recognition: LocalizedAssetSection,
    classifier_recognition: LocalizedAssetSection,
    speech_models: LocalizedModelSection,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalizedAssetSection {
    assets: Vec<LocalizedAssetEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalizedModelSection {
    models: Vec<LocalizedAssetEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalizedAssetEntry {
    id: String,
    name: String,
    #[serde(default)]
    primary: bool,
    sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetCatalog {
    pub schema_version: u32,
    pub assets: Vec<AssetDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetPhase {
    Missing,
    Checking,
    Connecting,
    Downloading,
    Verifying,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSummary {
    pub id: String,
    pub kind: AssetKind,
    pub asset_group: Option<AssetGroup>,
    pub display_name: String,
    pub version: String,
    pub asset_path: String,
    pub sources: Vec<String>,
    pub phase: AssetPhase,
    pub progress: Option<f32>,
    pub error: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInstallRequest {
    pub asset_link_list: Vec<String>,
    pub asset_path: String,
}

#[derive(Debug, Clone)]
pub struct AssetProgress {
    pub phase: AssetPhase,
    pub progress: Option<f32>,
    pub message: Option<String>,
}

pub type ProgressCallback = Arc<dyn Fn(AssetProgress) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    path: String,
    size: u64,
    modified: std::time::SystemTime,
}

#[derive(Debug, Clone)]
struct SourceProbe {
    source: String,
    bytes: usize,
    elapsed: Duration,
}

impl SourceProbe {
    fn bytes_per_second(&self) -> f64 {
        self.bytes as f64 / self.elapsed.as_secs_f64().max(0.001)
    }
}

struct StageCleanup {
    path: PathBuf,
}

impl StageCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for StageCleanup {
    fn drop(&mut self) {
        if self.path.exists() {
            if let Err(error) = std::fs::remove_dir_all(&self.path) {
                tracing::warn!(
                    path = %self.path.display(),
                    %error,
                    "failed to immediately clean asset staging directory"
                );
            }
        }
    }
}

#[derive(Clone)]
pub struct AssetManager {
    paths: AppPaths,
    catalog: Arc<AssetCatalog>,
    manifest: Arc<LocalizedAssetManifest>,
    client: reqwest::Client,
    verified: Arc<Mutex<std::collections::HashMap<String, Vec<FileStamp>>>>,
}

impl AssetManager {
    pub fn load(paths: AppPaths, catalog_path: &Path) -> Result<Self, StorageError> {
        let text = std::fs::read_to_string(catalog_path).map_err(|error| {
            StorageError(format!(
                "failed to read asset catalog '{}': {error}",
                catalog_path.display()
            ))
        })?;
        let manifest_path = catalog_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("manifest-cn.json");
        let manifest = std::fs::read_to_string(&manifest_path).map_err(|error| {
            StorageError(format!(
                "failed to read localized asset manifest '{}': {error}",
                manifest_path.display()
            ))
        })?;
        Self::from_catalog_json(paths, &text, &manifest)
    }

    pub(crate) fn load_manifest(
        paths: AppPaths,
        manifest_path: &Path,
    ) -> Result<Self, StorageError> {
        let manifest = std::fs::read_to_string(manifest_path).map_err(|error| {
            StorageError(format!(
                "failed to read localized asset manifest '{}': {error}",
                manifest_path.display()
            ))
        })?;
        Self::from_catalog_json(paths, EMBEDDED_CATALOG, &manifest)
    }

    fn from_catalog_json(
        paths: AppPaths,
        text: &str,
        manifest_text: &str,
    ) -> Result<Self, StorageError> {
        let mut catalog: AssetCatalog = serde_json::from_str(text)
            .map_err(|error| StorageError(format!("invalid asset catalog: {error}")))?;
        if catalog.schema_version != 1 {
            return Err(StorageError(format!(
                "unsupported asset catalog schema version {}",
                catalog.schema_version
            )));
        }
        let manifest: LocalizedAssetManifest = serde_json::from_str(manifest_text)
            .map_err(|error| StorageError(format!("invalid localized asset manifest: {error}")))?;
        validate_catalog(&catalog)?;
        apply_localized_manifest(&mut catalog, &manifest)?;
        tracing::info!(
            locale = %manifest.locale,
            speaker_assets = manifest.speaker_recognition.assets.len(),
            classifier_assets = manifest.classifier_recognition.assets.len(),
            speech_models = manifest.speech_models.models.len(),
            "localized asset manifest loaded"
        );
        let client = reqwest::Client::builder()
            .user_agent(format!("DictatingMe/{}", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::limited(8))
            .build()
            .map_err(|error| StorageError(format!("failed to create HTTP client: {error}")))?;
        Ok(Self {
            paths,
            catalog: Arc::new(catalog),
            manifest: Arc::new(manifest),
            client,
            verified: Arc::new(Mutex::new(std::collections::HashMap::new())),
        })
    }

    pub fn catalog(&self) -> &AssetCatalog {
        &self.catalog
    }

    pub fn descriptor(&self, id: &str) -> Result<&AssetDescriptor, StorageError> {
        self.catalog
            .assets
            .iter()
            .find(|asset| asset.id == id)
            .ok_or_else(|| StorageError(format!("unknown asset id: {id}")))
    }

    pub fn descriptors_for_group(
        &self,
        group: AssetGroup,
    ) -> Result<Vec<&AssetDescriptor>, StorageError> {
        self.manifest_entries(group)
            .iter()
            .map(|entry| self.descriptor(&entry.id))
            .collect()
    }

    pub fn primary_descriptor(&self, group: AssetGroup) -> Result<&AssetDescriptor, StorageError> {
        let entries = self.manifest_entries(group);
        let entry = entries
            .iter()
            .find(|entry| entry.primary)
            .or_else(|| entries.first())
            .ok_or_else(|| StorageError(format!("asset group {group:?} is empty")))?;
        self.descriptor(&entry.id)
    }

    pub fn group_for_asset(&self, id: &str) -> Option<AssetGroup> {
        [
            AssetGroup::SpeakerRecognition,
            AssetGroup::ClassifierRecognition,
            AssetGroup::SpeechModels,
        ]
        .into_iter()
        .find(|group| {
            self.manifest_entries(*group)
                .iter()
                .any(|entry| entry.id == id)
        })
    }

    pub fn first_descriptor_of_kind(
        &self,
        kind: AssetKind,
    ) -> Result<&AssetDescriptor, StorageError> {
        self.catalog
            .assets
            .iter()
            .find(|asset| asset.kind == kind)
            .ok_or_else(|| StorageError(format!("no asset is registered for kind {kind:?}")))
    }

    fn manifest_entries(&self, group: AssetGroup) -> &[LocalizedAssetEntry] {
        match group {
            AssetGroup::SpeakerRecognition => &self.manifest.speaker_recognition.assets,
            AssetGroup::ClassifierRecognition => &self.manifest.classifier_recognition.assets,
            AssetGroup::SpeechModels => &self.manifest.speech_models.models,
        }
    }

    pub fn descriptor_for_path(&self, asset_path: &str) -> Result<&AssetDescriptor, StorageError> {
        let requested = normalize_path(Path::new(asset_path));
        self.catalog
            .assets
            .iter()
            .find(|asset| {
                self.paths
                    .resolve_asset_path(&asset.install_path)
                    .map(|path| normalize_path(&path) == requested)
                    .unwrap_or(false)
            })
            .ok_or_else(|| {
                StorageError(format!(
                    "asset path is not registered in the trusted catalog: {asset_path}"
                ))
            })
    }

    pub fn asset_path(&self, descriptor: &AssetDescriptor) -> Result<PathBuf, StorageError> {
        self.paths.resolve_asset_path(&descriptor.install_path)
    }

    pub fn summaries(&self, selected_id: Option<&str>) -> Vec<AssetSummary> {
        self.catalog
            .assets
            .iter()
            .map(|descriptor| {
                let path = self.asset_path(descriptor).unwrap_or_default();
                let status = self.verify_cached(descriptor, &path);
                AssetSummary {
                    id: descriptor.id.clone(),
                    kind: descriptor.kind,
                    asset_group: self.group_for_asset(&descriptor.id),
                    display_name: descriptor.display_name.clone(),
                    version: descriptor.version.clone(),
                    asset_path: path.display().to_string(),
                    sources: descriptor.sources.clone(),
                    phase: if status.is_ok() {
                        AssetPhase::Ready
                    } else {
                        AssetPhase::Missing
                    },
                    progress: None,
                    error: None,
                    selected: selected_id == Some(descriptor.id.as_str()),
                }
            })
            .collect()
    }

    pub fn inspect(&self, asset_path: &str, selected_id: Option<&str>) -> AssetSummary {
        match self.descriptor_for_path(asset_path) {
            Ok(descriptor) => {
                let path = self.asset_path(descriptor).unwrap_or_default();
                match verify_asset_directory(&path, descriptor) {
                    Ok(()) => AssetSummary {
                        id: {
                            self.mark_verified(descriptor, &path);
                            descriptor.id.clone()
                        },
                        kind: descriptor.kind,
                        asset_group: self.group_for_asset(&descriptor.id),
                        display_name: descriptor.display_name.clone(),
                        version: descriptor.version.clone(),
                        asset_path: path.display().to_string(),
                        sources: descriptor.sources.clone(),
                        phase: AssetPhase::Ready,
                        progress: Some(1.0),
                        error: None,
                        selected: selected_id == Some(descriptor.id.as_str()),
                    },
                    Err(error) => AssetSummary {
                        id: descriptor.id.clone(),
                        kind: descriptor.kind,
                        asset_group: self.group_for_asset(&descriptor.id),
                        display_name: descriptor.display_name.clone(),
                        version: descriptor.version.clone(),
                        asset_path: path.display().to_string(),
                        sources: descriptor.sources.clone(),
                        phase: AssetPhase::Missing,
                        progress: None,
                        error: Some(error.0),
                        selected: selected_id == Some(descriptor.id.as_str()),
                    },
                }
            }
            Err(error) => AssetSummary {
                id: String::new(),
                kind: AssetKind::ClassifierResource,
                asset_group: None,
                display_name: "未知资源".to_owned(),
                version: String::new(),
                asset_path: asset_path.to_owned(),
                sources: Vec::new(),
                phase: AssetPhase::Failed,
                progress: None,
                error: Some(error.0),
                selected: false,
            },
        }
    }

    pub fn bootstrap_bundled(&self, resource_assets: &Path) -> Result<(), StorageError> {
        for descriptor in self.catalog.assets.iter().filter(|asset| asset.bundled) {
            let destination = self.asset_path(descriptor)?;
            if verify_asset_directory(&destination, descriptor).is_ok() {
                self.mark_verified(descriptor, &destination);
                continue;
            }
            let source = resource_assets.join(&descriptor.install_path);
            verify_asset_directory(&source, descriptor).map_err(|error| {
                StorageError(format!(
                    "bundled asset '{}' is invalid at '{}': {}",
                    descriptor.id,
                    source.display(),
                    error.0
                ))
            })?;
            if destination.exists() {
                std::fs::remove_dir_all(&destination).map_err(|error| {
                    StorageError(format!(
                        "failed to replace invalid bundled asset '{}': {error}",
                        destination.display()
                    ))
                })?;
            }
            copy_directory(&source, &destination)?;
            verify_asset_directory(&destination, descriptor)?;
            self.mark_verified(descriptor, &destination);
        }
        Ok(())
    }

    pub fn bootstrap_embedded(&self) -> Result<(), StorageError> {
        let descriptor = self.first_descriptor_of_kind(AssetKind::PresetEvoke)?;
        let destination = self.asset_path(descriptor)?;
        if verify_asset_directory(&destination, descriptor).is_ok() {
            self.mark_verified(descriptor, &destination);
            return Ok(());
        }
        if destination.exists() {
            std::fs::remove_dir_all(&destination).map_err(|error| {
                StorageError(format!(
                    "failed to replace invalid embedded preset '{}': {error}",
                    destination.display()
                ))
            })?;
        }
        std::fs::create_dir_all(&destination).map_err(|error| {
            StorageError(format!(
                "failed to create embedded preset directory '{}': {error}",
                destination.display()
            ))
        })?;
        for expected in &descriptor.files {
            let (_, bytes) = EMBEDDED_PRESET_FILES
                .iter()
                .find(|(path, _)| *path == expected.path)
                .ok_or_else(|| {
                    StorageError(format!(
                        "embedded preset bytes are missing for '{}'",
                        expected.path
                    ))
                })?;
            std::fs::write(destination.join(&expected.path), bytes).map_err(|error| {
                StorageError(format!(
                    "failed to write embedded preset '{}': {error}",
                    expected.path
                ))
            })?;
        }
        verify_asset_directory(&destination, descriptor)?;
        self.mark_verified(descriptor, &destination);
        Ok(())
    }

    pub fn cleanup_transient(&self) -> Result<(), StorageError> {
        self.recover_asset_backups()?;
        cleanup_directory_older_than(&self.paths.staging, std::time::Duration::ZERO)?;
        cleanup_directory_older_than(&self.paths.trash, std::time::Duration::from_secs(60 * 60))?;
        Ok(())
    }

    fn recover_asset_backups(&self) -> Result<(), StorageError> {
        for descriptor in &self.catalog.assets {
            let destination = self.asset_path(descriptor)?;
            let Some(parent) = destination.parent() else {
                continue;
            };
            if !parent.is_dir() {
                continue;
            }
            let file_name = destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("asset");
            let prefix = format!(".{file_name}.backup-");
            let mut backups = std::fs::read_dir(parent)
                .map_err(|error| {
                    StorageError(format!(
                        "failed to scan asset backups in '{}': {error}",
                        parent.display()
                    ))
                })?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(&prefix))
                })
                .collect::<Vec<_>>();
            backups.sort();
            if !destination.exists() {
                if let Some(backup) = backups.pop() {
                    std::fs::rename(&backup, &destination).map_err(|error| {
                        StorageError(format!(
                            "failed to restore interrupted asset backup '{}' to '{}': {error}",
                            backup.display(),
                            destination.display()
                        ))
                    })?;
                }
            }
            for backup in backups {
                let trash = self
                    .paths
                    .trash
                    .join(format!("recovered-backup-{}", Uuid::new_v4()));
                let _ = std::fs::rename(backup, trash);
            }
        }
        Ok(())
    }

    pub async fn install(
        &self,
        request: AssetInstallRequest,
        progress: ProgressCallback,
    ) -> Result<AssetSummary, StorageError> {
        let descriptor = self.descriptor_for_path(&request.asset_path)?.clone();
        if descriptor.bundled {
            return Err(StorageError(format!(
                "bundled asset '{}' cannot be downloaded",
                descriptor.id
            )));
        }
        let allowed_sources = request
            .asset_link_list
            .iter()
            .filter(|source| descriptor.sources.contains(source))
            .cloned()
            .collect::<Vec<_>>();
        if allowed_sources.is_empty() {
            return Err(StorageError(format!(
                "no trusted download source was provided for asset '{}'",
                descriptor.id
            )));
        }

        progress(AssetProgress {
            phase: AssetPhase::Connecting,
            progress: None,
            message: Some("正在并发测试下载源速度".to_owned()),
        });
        let candidates = self
            .benchmark_sources(&descriptor, &allowed_sources)
            .await?;
        let operation_id = Uuid::new_v4().to_string();
        let stage = self.paths.staging.join(&operation_id);
        tokio::fs::create_dir_all(&stage).await.map_err(|error| {
            StorageError(format!(
                "failed to create asset staging directory '{}': {error}",
                stage.display()
            ))
        })?;
        let _stage_cleanup = StageCleanup::new(stage.clone());
        let prepared_root = self
            .download_ranked_sources(&descriptor, &stage, candidates, Arc::clone(&progress))
            .await?;

        let destination = self.asset_path(&descriptor)?;
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                StorageError(format!(
                    "failed to create asset destination parent '{}': {error}",
                    parent.display()
                ))
            })?;
        }
        let backup = destination.parent().map(|parent| {
            parent.join(format!(
                ".{}.backup-{operation_id}",
                destination
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("asset")
            ))
        });
        if destination.exists() {
            tokio::fs::rename(
                &destination,
                backup.as_ref().expect("destination has parent"),
            )
            .await
            .map_err(|error| {
                StorageError(format!(
                    "failed to create recoverable backup for '{}': {error}",
                    destination.display()
                ))
            })?;
        }
        if let Err(error) = tokio::fs::rename(&prepared_root, &destination).await {
            if let Some(backup) = backup.as_ref().filter(|path| path.exists()) {
                if let Err(restore_error) = tokio::fs::rename(backup, &destination).await {
                    return Err(StorageError(format!(
                        "failed to install asset to '{}': {error}; failed to restore previous asset: {restore_error}",
                        destination.display()
                    )));
                }
            }
            return Err(StorageError(format!(
                "failed to atomically install asset to '{}': {error}",
                destination.display()
            )));
        }
        if let Some(backup) = backup.filter(|path| path.exists()) {
            let trash = self.paths.trash.join(format!(
                "asset-{}-{operation_id}",
                descriptor.id.replace('.', "-")
            ));
            let _ = tokio::fs::rename(backup, trash).await;
        }
        verify_asset_directory(&destination, &descriptor)?;
        self.mark_verified(&descriptor, &destination);

        Ok(AssetSummary {
            id: descriptor.id.clone(),
            kind: descriptor.kind,
            asset_group: self.group_for_asset(&descriptor.id),
            display_name: descriptor.display_name,
            version: descriptor.version,
            asset_path: destination.display().to_string(),
            sources: descriptor.sources,
            phase: AssetPhase::Ready,
            progress: Some(1.0),
            error: None,
            selected: false,
        })
    }

    async fn download_ranked_sources(
        &self,
        descriptor: &AssetDescriptor,
        stage: &Path,
        candidates: Vec<SourceProbe>,
        progress: ProgressCallback,
    ) -> Result<PathBuf, StorageError> {
        let mut errors = Vec::new();
        for candidate in candidates {
            let probe_speed = candidate.bytes_per_second();
            let source = candidate.source;
            let _ = tokio::fs::remove_dir_all(stage).await;
            tokio::fs::create_dir_all(stage).await.map_err(|error| {
                StorageError(format!(
                    "failed to reset asset staging directory '{}': {error}",
                    stage.display()
                ))
            })?;
            progress(AssetProgress {
                phase: AssetPhase::Connecting,
                progress: None,
                message: Some(format!(
                    "已选择下载源（探测速率 {:.1} Mbps）",
                    probe_speed * 8.0 / 1_000_000.0
                )),
            });
            let result = if source.contains("{file}") {
                self.download_file_set(descriptor, stage, &source, Arc::clone(&progress))
                    .await
            } else {
                self.download_archive(descriptor, stage, &source, Arc::clone(&progress))
                    .await
            };
            match result {
                Ok(prepared) => return Ok(prepared),
                Err(error) => {
                    tracing::warn!(%source, %error, "ranked asset source failed");
                    errors.push(format!("{source}: {error}"));
                }
            }
        }
        Err(StorageError(format!(
            "all ranked asset sources failed: {}",
            errors.join("; ")
        )))
    }

    async fn download_archive(
        &self,
        descriptor: &AssetDescriptor,
        stage: &Path,
        source: &str,
        progress: ProgressCallback,
    ) -> Result<PathBuf, StorageError> {
        let response = match tokio::time::timeout(
            SOURCE_CONNECT_TIMEOUT,
            self.client.get(source).send(),
        )
        .await
        {
            Ok(Ok(response)) if response.status().is_success() => response,
            Ok(Ok(response)) => {
                return Err(StorageError(format!(
                    "full download returned HTTP {}",
                    response.status()
                )))
            }
            Ok(Err(error)) => {
                return Err(StorageError(format!(
                    "full download request failed: {error}"
                )))
            }
            Err(_) => {
                return Err(StorageError(
                    "full download connection timed out".to_owned(),
                ))
            }
        };
        let archive_path = stage.join("download.part");
        let total = response.content_length();
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(&archive_path)
            .await
            .map_err(|error| StorageError(format!("failed to create archive stage: {error}")))?;
        let mut downloaded = 0_u64;
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|error| StorageError(format!("asset download failed: {error}")))?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .map_err(|error| StorageError(format!("failed to write asset archive: {error}")))?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            progress(AssetProgress {
                phase: AssetPhase::Downloading,
                progress: total
                    .map(|size| (downloaded as f32 / size.max(1) as f32).clamp(0.0, 1.0)),
                message: Some(format!("正在从 {source} 下载")),
            });
        }
        tokio::io::AsyncWriteExt::flush(&mut file)
            .await
            .map_err(|error| StorageError(format!("failed to flush asset archive: {error}")))?;
        drop(file);
        progress(AssetProgress {
            phase: AssetPhase::Verifying,
            progress: None,
            message: Some("正在解压并验证 SHA-256".to_owned()),
        });
        let descriptor = descriptor.clone();
        let stage = stage.to_owned();
        let archive_for_extract = archive_path.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            prepare_staged_asset(&stage, &archive_for_extract, &descriptor)
        })
        .await
        .map_err(|error| StorageError(format!("asset extraction task failed: {error}")))??;
        tokio::fs::remove_file(&archive_path)
            .await
            .map_err(|error| StorageError(format!("failed to delete asset archive: {error}")))?;
        Ok(prepared)
    }

    async fn download_file_set(
        &self,
        descriptor: &AssetDescriptor,
        stage: &Path,
        source_template: &str,
        progress: ProgressCallback,
    ) -> Result<PathBuf, StorageError> {
        let prepared = stage.join("prepared");
        tokio::fs::create_dir_all(&prepared)
            .await
            .map_err(|error| StorageError(format!("failed to create file-set stage: {error}")))?;
        let expected_total = descriptor
            .files
            .iter()
            .map(|file| file.size_bytes)
            .sum::<u64>()
            .max(1);
        let mut downloaded_total = 0_u64;
        for expected in &descriptor.files {
            let source = source_template.replace("{file}", &expected.path);
            let response =
                match tokio::time::timeout(SOURCE_CONNECT_TIMEOUT, self.client.get(&source).send())
                    .await
                {
                    Ok(Ok(response)) if response.status().is_success() => response,
                    Ok(Ok(response)) => {
                        return Err(StorageError(format!(
                            "file '{}' returned HTTP {}",
                            expected.path,
                            response.status()
                        )))
                    }
                    Ok(Err(error)) => {
                        return Err(StorageError(format!(
                            "file '{}' request failed: {error}",
                            expected.path
                        )))
                    }
                    Err(_) => {
                        return Err(StorageError(format!(
                            "file '{}' connection timed out",
                            expected.path
                        )))
                    }
                };
            let target = prepared.join(&expected.path);
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|error| {
                    StorageError(format!("failed to create file-set directory: {error}"))
                })?;
            }
            let mut output = tokio::fs::File::create(&target)
                .await
                .map_err(|error| StorageError(format!("failed to create model file: {error}")))?;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk =
                    chunk.map_err(|error| StorageError(format!("model file failed: {error}")))?;
                tokio::io::AsyncWriteExt::write_all(&mut output, &chunk)
                    .await
                    .map_err(|error| {
                        StorageError(format!("failed to write model file: {error}"))
                    })?;
                downloaded_total = downloaded_total.saturating_add(chunk.len() as u64);
                progress(AssetProgress {
                    phase: AssetPhase::Downloading,
                    progress: Some(
                        (downloaded_total as f32 / expected_total as f32).clamp(0.0, 1.0),
                    ),
                    message: Some(format!("正在从国内镜像下载 {}", expected.path)),
                });
            }
            tokio::io::AsyncWriteExt::flush(&mut output)
                .await
                .map_err(|error| StorageError(format!("failed to flush model file: {error}")))?;
        }
        progress(AssetProgress {
            phase: AssetPhase::Verifying,
            progress: None,
            message: Some("正在验证模型文件 SHA-256".to_owned()),
        });
        let verify_root = prepared.clone();
        let descriptor = descriptor.clone();
        tokio::task::spawn_blocking(move || verify_asset_directory(&verify_root, &descriptor))
            .await
            .map_err(|error| {
                StorageError(format!("file-set verification task failed: {error}"))
            })??;
        Ok(prepared)
    }

    async fn benchmark_sources(
        &self,
        descriptor: &AssetDescriptor,
        sources: &[String],
    ) -> Result<Vec<SourceProbe>, StorageError> {
        let mut pending = FuturesUnordered::new();
        for source in sources.iter().cloned() {
            let client = self.client.clone();
            let probe_url = source_probe_url(&source, descriptor)?;
            pending.push(async move {
                match tokio::time::timeout(
                    SOURCE_PROBE_TIMEOUT,
                    probe_source(client, source.clone(), probe_url),
                )
                .await
                {
                    Ok(Ok(probe)) => Ok(probe),
                    Ok(Err(error)) => Err(format!("{source}: {error}")),
                    Err(_) => Err(format!("{source}: probe timed out")),
                }
            });
        }

        let mut probes = Vec::new();
        let mut errors = Vec::new();
        while let Some(result) = pending.next().await {
            match result {
                Ok(probe) => probes.push(probe),
                Err(error) => errors.push(error),
            }
        }
        rank_source_probes(&mut probes);
        for (rank, probe) in probes.iter().enumerate() {
            tracing::info!(
                rank = rank + 1,
                source = %probe.source,
                probe_bytes = probe.bytes,
                probe_ms = probe.elapsed.as_millis(),
                probe_mbps = probe.bytes_per_second() * 8.0 / 1_000_000.0,
                "asset source benchmark completed"
            );
        }
        for error in &errors {
            tracing::warn!(%error, "asset source benchmark failed");
        }
        if probes.is_empty() {
            return Err(StorageError(format!(
                "no asset source passed the concurrent speed probe: {}",
                errors.join("; ")
            )));
        }
        Ok(probes)
    }

    fn verify_cached(&self, descriptor: &AssetDescriptor, path: &Path) -> Result<(), StorageError> {
        let current = file_stamps(path, descriptor);
        if current.as_ref().is_ok_and(|stamps| {
            self.verified
                .lock()
                .unwrap_or_else(|lock| lock.into_inner())
                .get(&descriptor.id)
                == Some(stamps)
        }) {
            return Ok(());
        }
        verify_asset_directory(path, descriptor)?;
        self.mark_verified(descriptor, path);
        Ok(())
    }

    fn mark_verified(&self, descriptor: &AssetDescriptor, path: &Path) {
        if let Ok(stamps) = file_stamps(path, descriptor) {
            self.verified
                .lock()
                .unwrap_or_else(|lock| lock.into_inner())
                .insert(descriptor.id.clone(), stamps);
        }
    }
}

async fn probe_source(
    client: reqwest::Client,
    source: String,
    probe_url: String,
) -> Result<SourceProbe, StorageError> {
    let started = Instant::now();
    let response = client
        .get(&probe_url)
        .header(RANGE, format!("bytes=0-{}", SOURCE_PROBE_BYTES - 1))
        .send()
        .await
        .map_err(|error| StorageError(format!("probe request failed: {error}")))?;
    if !response.status().is_success() {
        return Err(StorageError(format!(
            "probe returned HTTP {}",
            response.status()
        )));
    }
    let mut stream = response.bytes_stream();
    let mut received = 0_usize;
    while received < SOURCE_PROBE_BYTES {
        let Some(chunk) = stream.next().await else {
            break;
        };
        let chunk = chunk.map_err(|error| StorageError(format!("probe stream failed: {error}")))?;
        received = received.saturating_add(chunk.len()).min(SOURCE_PROBE_BYTES);
    }
    if received < SOURCE_PROBE_MIN_BYTES {
        return Err(StorageError(format!(
            "probe returned only {received} bytes; expected at least {SOURCE_PROBE_MIN_BYTES}"
        )));
    }
    Ok(SourceProbe {
        source,
        bytes: received,
        elapsed: started.elapsed(),
    })
}

fn source_probe_url(source: &str, descriptor: &AssetDescriptor) -> Result<String, StorageError> {
    if !source.contains("{file}") {
        return Ok(source.to_owned());
    }
    let file = descriptor
        .files
        .iter()
        .max_by_key(|file| file.size_bytes)
        .ok_or_else(|| StorageError(format!("asset '{}' has no probe file", descriptor.id)))?;
    Ok(source.replace("{file}", &file.path))
}

fn rank_source_probes(probes: &mut [SourceProbe]) {
    probes.sort_by(|left, right| {
        right
            .bytes_per_second()
            .partial_cmp(&left.bytes_per_second())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn file_stamps(root: &Path, descriptor: &AssetDescriptor) -> Result<Vec<FileStamp>, StorageError> {
    descriptor
        .files
        .iter()
        .map(|file| {
            let path = root.join(&file.path);
            let metadata = std::fs::metadata(&path).map_err(|error| {
                StorageError(format!(
                    "failed to inspect cached asset file '{}': {error}",
                    path.display()
                ))
            })?;
            Ok(FileStamp {
                path: file.path.clone(),
                size: metadata.len(),
                modified: metadata
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            })
        })
        .collect()
}

fn validate_catalog(catalog: &AssetCatalog) -> Result<(), StorageError> {
    let mut ids = std::collections::HashSet::new();
    let mut paths = std::collections::HashSet::new();
    for asset in &catalog.assets {
        if !ids.insert(asset.id.as_str()) {
            return Err(StorageError(format!("duplicate asset id: {}", asset.id)));
        }
        if !paths.insert(asset.install_path.as_str()) {
            return Err(StorageError(format!(
                "duplicate asset install path: {}",
                asset.install_path
            )));
        }
        if asset.files.is_empty() {
            return Err(StorageError(format!("asset '{}' has no files", asset.id)));
        }
        for file in &asset.files {
            if file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(StorageError(format!(
                    "asset '{}' has an invalid SHA-256 for '{}'",
                    asset.id, file.path
                )));
            }
        }
    }
    Ok(())
}

fn apply_localized_manifest(
    catalog: &mut AssetCatalog,
    manifest: &LocalizedAssetManifest,
) -> Result<(), StorageError> {
    if manifest.schema_version != 1 {
        return Err(StorageError(format!(
            "unsupported localized asset manifest schema version {}",
            manifest.schema_version
        )));
    }
    if manifest.locale != "zh-CN" {
        return Err(StorageError(format!(
            "localized asset manifest locale must be zh-CN, found '{}'",
            manifest.locale
        )));
    }

    let mut configured_ids = std::collections::HashSet::new();
    apply_manifest_section(
        catalog,
        AssetGroup::SpeakerRecognition,
        AssetKind::SpeakerEmbedding,
        &manifest.speaker_recognition.assets,
        &mut configured_ids,
    )?;
    apply_manifest_section(
        catalog,
        AssetGroup::ClassifierRecognition,
        AssetKind::ClassifierResource,
        &manifest.classifier_recognition.assets,
        &mut configured_ids,
    )?;
    apply_manifest_section(
        catalog,
        AssetGroup::SpeechModels,
        AssetKind::DictationModel,
        &manifest.speech_models.models,
        &mut configured_ids,
    )?;

    for descriptor in &mut catalog.assets {
        if descriptor.bundled {
            descriptor.display_name.clone_from(&descriptor.id);
        } else if !configured_ids.contains(descriptor.id.as_str()) {
            return Err(StorageError(format!(
                "downloadable asset '{}' is missing from manifest-cn.json",
                descriptor.id
            )));
        }
    }
    Ok(())
}

fn apply_manifest_section(
    catalog: &mut AssetCatalog,
    group: AssetGroup,
    expected_kind: AssetKind,
    entries: &[LocalizedAssetEntry],
    configured_ids: &mut std::collections::HashSet<String>,
) -> Result<(), StorageError> {
    if entries.is_empty() {
        return Err(StorageError(format!(
            "localized asset group {group:?} cannot be empty"
        )));
    }
    if entries.iter().filter(|entry| entry.primary).count() > 1 {
        return Err(StorageError(format!(
            "localized asset group {group:?} has multiple primary entries"
        )));
    }
    for entry in entries {
        if entry.name.trim().is_empty() {
            return Err(StorageError(format!(
                "localized asset '{}' has an empty name",
                entry.id
            )));
        }
        if entry.sources.is_empty()
            || entry
                .sources
                .iter()
                .any(|source| !source.starts_with("https://"))
        {
            return Err(StorageError(format!(
                "localized asset '{}' must provide HTTPS download sources",
                entry.id
            )));
        }
        if !configured_ids.insert(entry.id.clone()) {
            return Err(StorageError(format!(
                "localized asset '{}' appears in multiple manifest groups",
                entry.id
            )));
        }
        let descriptor = catalog
            .assets
            .iter_mut()
            .find(|descriptor| descriptor.id == entry.id)
            .ok_or_else(|| {
                StorageError(format!(
                    "localized asset '{}' is missing from sha.json",
                    entry.id
                ))
            })?;
        if descriptor.bundled || descriptor.kind != expected_kind {
            return Err(StorageError(format!(
                "localized asset '{}' does not match group {group:?}",
                entry.id
            )));
        }
        descriptor.display_name = entry.name.trim().to_owned();
        descriptor.sources.clone_from(&entry.sources);
    }
    Ok(())
}

fn prepare_staged_asset(
    stage: &Path,
    download: &Path,
    descriptor: &AssetDescriptor,
) -> Result<PathBuf, StorageError> {
    let prepared = stage.join("prepared");
    std::fs::create_dir_all(&prepared).map_err(|error| {
        StorageError(format!(
            "failed to create prepared asset directory '{}': {error}",
            prepared.display()
        ))
    })?;
    match descriptor.format {
        AssetFormat::File => {
            let output = descriptor.output_file.as_deref().ok_or_else(|| {
                StorageError(format!("file asset '{}' has no outputFile", descriptor.id))
            })?;
            std::fs::copy(download, prepared.join(output)).map_err(|error| {
                StorageError(format!("failed to stage downloaded asset file: {error}"))
            })?;
        }
        AssetFormat::TarBz2 => {
            let file = File::open(download)
                .map_err(|error| StorageError(format!("failed to open asset archive: {error}")))?;
            let decoder = BzDecoder::new(file);
            let mut archive = tar::Archive::new(decoder);
            archive.set_preserve_permissions(false);
            archive.set_preserve_mtime(false);
            archive.unpack(&prepared).map_err(|error| {
                StorageError(format!("failed to unpack asset archive: {error}"))
            })?;
            let root = descriptor.archive_root.as_deref().ok_or_else(|| {
                StorageError(format!(
                    "archive asset '{}' has no archiveRoot",
                    descriptor.id
                ))
            })?;
            let extracted = prepared.join(root);
            verify_asset_directory(&extracted, descriptor)?;
            return Ok(extracted);
        }
        AssetFormat::Directory => {
            return Err(StorageError(format!(
                "directory asset '{}' cannot be downloaded",
                descriptor.id
            )));
        }
    }
    verify_asset_directory(&prepared, descriptor)?;
    Ok(prepared)
}

pub fn verify_asset_directory(
    root: &Path,
    descriptor: &AssetDescriptor,
) -> Result<(), StorageError> {
    if !root.is_dir() {
        return Err(StorageError(format!(
            "asset directory is missing: {}",
            root.display()
        )));
    }
    for expected in &descriptor.files {
        let path = root.join(&expected.path);
        let metadata = std::fs::metadata(&path).map_err(|error| {
            StorageError(format!(
                "asset file '{}' is missing: {error}",
                path.display()
            ))
        })?;
        if metadata.len() != expected.size_bytes {
            return Err(StorageError(format!(
                "asset file '{}' size mismatch: expected {}, found {}",
                path.display(),
                expected.size_bytes,
                metadata.len()
            )));
        }
        let actual = sha256_file(&path)?;
        if actual != expected.sha256 {
            return Err(StorageError(format!(
                "asset file '{}' SHA-256 mismatch",
                path.display()
            )));
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, StorageError> {
    let mut file = File::open(path).map_err(|error| {
        StorageError(format!(
            "failed to open '{}' for SHA-256: {error}",
            path.display()
        ))
    })?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            StorageError(format!("failed to hash '{}': {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), StorageError> {
    std::fs::create_dir_all(destination).map_err(|error| {
        StorageError(format!(
            "failed to create copied asset directory '{}': {error}",
            destination.display()
        ))
    })?;
    for entry in std::fs::read_dir(source).map_err(|error| {
        StorageError(format!(
            "failed to enumerate bundled asset '{}': {error}",
            source.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            StorageError(format!("failed to read bundled asset entry: {error}"))
        })?;
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| StorageError(format!("failed to inspect asset entry: {error}")))?
            .is_dir()
        {
            copy_directory(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|error| {
                StorageError(format!(
                    "failed to copy bundled asset '{}' to '{}': {error}",
                    entry.path().display(),
                    target.display()
                ))
            })?;
        }
    }
    Ok(())
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn cleanup_directory_older_than(
    root: &Path,
    max_age: std::time::Duration,
) -> Result<(), StorageError> {
    if !root.is_dir() {
        return Ok(());
    }
    let now = std::time::SystemTime::now();
    for entry in std::fs::read_dir(root).map_err(|error| {
        StorageError(format!(
            "failed to enumerate cleanup directory '{}': {error}",
            root.display()
        ))
    })? {
        let entry = entry
            .map_err(|error| StorageError(format!("failed to read cleanup entry: {error}")))?;
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() < max_age {
            continue;
        }
        let path = entry.path();
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(error) = result {
            tracing::warn!(path = %path.display(), %error, "failed to clean transient asset content");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_download_sources_by_measured_throughput() {
        let mut probes = vec![
            SourceProbe {
                source: "slow".to_owned(),
                bytes: SOURCE_PROBE_BYTES,
                elapsed: Duration::from_secs(4),
            },
            SourceProbe {
                source: "fast".to_owned(),
                bytes: SOURCE_PROBE_BYTES,
                elapsed: Duration::from_secs(1),
            },
            SourceProbe {
                source: "medium".to_owned(),
                bytes: SOURCE_PROBE_BYTES,
                elapsed: Duration::from_secs(2),
            },
        ];

        rank_source_probes(&mut probes);

        assert_eq!(
            probes
                .iter()
                .map(|probe| probe.source.as_str())
                .collect::<Vec<_>>(),
            ["fast", "medium", "slow"]
        );
    }

    #[test]
    fn localized_manifest_drives_groups_names_and_sources() {
        let workspace_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("assets");
        let paths = AppPaths::new(PathBuf::from("manifest-test"));
        let manager = AssetManager::load(paths, &workspace_assets.join("sha.json")).unwrap();

        let speaker = manager
            .primary_descriptor(AssetGroup::SpeakerRecognition)
            .unwrap();
        let classifier = manager
            .descriptors_for_group(AssetGroup::ClassifierRecognition)
            .unwrap();
        let speech_models = manager
            .descriptors_for_group(AssetGroup::SpeechModels)
            .unwrap();

        assert_eq!(speaker.display_name, "sherpa-campplus 中文声纹模型");
        assert_eq!(speaker.kind, AssetKind::SpeakerEmbedding);
        assert!(!speaker.sources.is_empty());
        assert_eq!(classifier.len(), 1);
        assert_eq!(classifier[0].kind, AssetKind::ClassifierResource);
        assert_eq!(speech_models.len(), 1);
        assert_eq!(speech_models[0].kind, AssetKind::DictationModel);
        assert!(speech_models[0]
            .sources
            .iter()
            .all(|source| source.starts_with("https://")));
    }

    #[test]
    fn bootstraps_bundled_preset_into_managed_app_data() {
        let temporary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target")
            .join(format!("asset-bootstrap-test-{}", Uuid::new_v4()));
        let paths = AppPaths::new(temporary.clone());
        paths.ensure().unwrap();
        let workspace_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("assets");
        let manager = AssetManager::load(paths, &workspace_assets.join("sha.json")).unwrap();
        manager.bootstrap_embedded().unwrap();
        let descriptor = manager
            .descriptor("evoke.sherpa-zipformer-wenetspeech")
            .unwrap();
        verify_asset_directory(&manager.asset_path(descriptor).unwrap(), descriptor).unwrap();
        std::fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn staged_download_cleanup_is_immediate() {
        let temporary = std::env::temp_dir().join(format!("dictatingme-stage-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temporary).unwrap();
        std::fs::write(temporary.join("download.part"), b"partial").unwrap();

        drop(StageCleanup::new(temporary.clone()));

        assert!(!temporary.exists());
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn downloads_and_verifies_classifier_resource() {
        let workspace_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("assets");
        let temporary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target")
            .join(format!("asset-network-test-{}", Uuid::new_v4()));
        let paths = AppPaths::new(temporary.clone());
        paths.ensure().unwrap();
        let manager = AssetManager::load(paths, &workspace_assets.join("sha.json")).unwrap();
        let descriptor = manager
            .descriptor("classifier.ms-snsd-babble")
            .unwrap()
            .clone();
        let asset_path = manager.asset_path(&descriptor).unwrap();
        let installed = manager
            .install(
                AssetInstallRequest {
                    asset_link_list: descriptor.sources.clone(),
                    asset_path: asset_path.display().to_string(),
                },
                Arc::new(|_| {}),
            )
            .await
            .unwrap();
        assert_eq!(installed.phase, AssetPhase::Ready);
        verify_asset_directory(&asset_path, &descriptor).unwrap();
        std::fs::remove_dir_all(temporary).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn downloads_and_verifies_modelscope_file_set_sample() {
        let workspace_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("assets");
        let temporary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target")
            .join(format!("modelscope-file-set-test-{}", Uuid::new_v4()));
        let paths = AppPaths::new(temporary.clone());
        paths.ensure().unwrap();
        let manager = AssetManager::load(paths, &workspace_assets.join("sha.json")).unwrap();
        let mut descriptor = manager
            .primary_descriptor(AssetGroup::SpeechModels)
            .unwrap()
            .clone();
        descriptor.files.retain(|file| file.path == "tokens.txt");
        let source = descriptor
            .sources
            .iter()
            .find(|source| source.contains("modelscope.cn") && source.contains("{file}"))
            .unwrap();
        let stage = temporary.join("file-set-stage");
        let progress: ProgressCallback = Arc::new(|_| {});

        let prepared = manager
            .download_file_set(&descriptor, &stage, source, progress)
            .await
            .unwrap();

        verify_asset_directory(&prepared, &descriptor).unwrap();
        std::fs::remove_dir_all(temporary).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn connects_to_model_mirrors() {
        let workspace_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("assets");
        let temporary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target")
            .join(format!("asset-connect-test-{}", Uuid::new_v4()));
        let paths = AppPaths::new(temporary.clone());
        paths.ensure().unwrap();
        let manager = AssetManager::load(paths, &workspace_assets.join("sha.json")).unwrap();
        for asset_id in ["dictation.sherpa-zipformer-zh-en", "speaker.campplus-zh"] {
            let descriptor = manager.descriptor(asset_id).unwrap();
            let probes = manager
                .benchmark_sources(descriptor, &descriptor.sources)
                .await
                .unwrap();
            assert!(!probes.is_empty());
            assert!(probes[0].source.starts_with("https://"));
            assert!(probes[0].bytes_per_second() > 0.0);
        }
        std::fs::remove_dir_all(temporary).unwrap();
    }
}
