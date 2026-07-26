//! 模型目录兼容封装，供 EvokeModel / DictationModel 共用。

use std::path::{Path, PathBuf};

/// 模型加载/推理相关错误。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelError(pub String);

/// 验证并持有模型目录路径；实际 sherpa-onnx 管线由各模型引擎持有。
pub struct OnnxSession {
    model_path: String,
}

impl OnnxSession {
    /// 同步加载（EvokeModel 常驻加载路径）。
    pub fn load(model_path: &str) -> Result<Self, ModelError> {
        validate_model_directory(model_path)?;
        Ok(Self {
            model_path: model_path.to_owned(),
        })
    }

    /// 异步加载（DictationModel 在 Loading 态走此路径，避免阻塞主循环）。
    pub async fn load_async(model_path: &str) -> Result<Self, ModelError> {
        let model_path = model_path.to_owned();
        tokio::task::spawn_blocking(move || Self::load(&model_path))
            .await
            .map_err(|error| ModelError(format!("model path validation task failed: {error}")))?
    }

    pub fn model_path(&self) -> &str {
        &self.model_path
    }

    /// 显式卸载，释放底层会话资源（DictationModel 在 Unloading 态调用）。
    pub fn unload(self) {}
}

pub(crate) fn validate_model_directory(model_path: &str) -> Result<PathBuf, ModelError> {
    if model_path.trim().is_empty() {
        return Err(ModelError("model path must not be empty".to_owned()));
    }

    let path = PathBuf::from(model_path);
    let metadata = std::fs::metadata(&path).map_err(|error| {
        ModelError(format!(
            "model directory '{}' is not accessible: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(ModelError(format!(
            "model path '{}' is not a directory",
            path.display()
        )));
    }
    Ok(path)
}

pub(crate) fn require_file(directory: &Path, name: &str) -> Result<PathBuf, ModelError> {
    let path = directory.join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(ModelError(format!(
            "required model file '{}' is missing",
            path.display()
        )))
    }
}

pub(crate) fn resolve_model_file(
    directory: &Path,
    role: &str,
    preferred_names: &[&str],
) -> Result<PathBuf, ModelError> {
    for name in preferred_names {
        let path = directory.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }

    let prefix = format!("{role}-");
    let exact_name = format!("{role}.onnx");
    let mut matches = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|error| {
        ModelError(format!(
            "failed to inspect model directory '{}': {error}",
            directory.display()
        ))
    })? {
        let path = entry
            .map_err(|error| {
                ModelError(format!(
                    "failed to read an entry in model directory '{}': {error}",
                    directory.display()
                ))
            })?
            .path();
        let is_match = path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.ends_with(".onnx") && (name == exact_name || name.starts_with(&prefix))
                });
        if is_match {
            matches.push(path);
        }
    }
    matches.sort();

    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(ModelError(format!(
            "no {role} ONNX file found in '{}'",
            directory.display()
        ))),
        _ => Err(ModelError(format!(
            "multiple {role} ONNX files found in '{}'; the model layout is ambiguous",
            directory.display()
        ))),
    }
}

pub(crate) fn path_string(path: &Path) -> Result<String, ModelError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        ModelError(format!(
            "model path '{}' is not valid Unicode",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::OnnxSession;

    #[test]
    fn validates_model_directory_paths() {
        let session = OnnxSession::load(".").expect("current directory should be valid");
        assert_eq!(session.model_path(), ".");
        assert!(OnnxSession::load("").is_err());
        assert!(OnnxSession::load("Cargo.toml").is_err());
        assert!(OnnxSession::load("__dictatingme_missing_model_directory__").is_err());
    }
}
