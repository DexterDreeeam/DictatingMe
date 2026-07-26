use std::path::{Path, PathBuf};

use super::StorageError;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub manifest: PathBuf,
    pub assets: PathBuf,
    pub profiles: PathBuf,
    pub sessions: PathBuf,
    pub history: PathBuf,
    pub staging: PathBuf,
    pub trash: PathBuf,
}

impl AppPaths {
    pub fn new(root: PathBuf) -> Self {
        let assets = root.join("assets");
        Self {
            database: root.join("index.sqlite3"),
            manifest: root.join("manifest-cn.json"),
            staging: assets.join(".staging"),
            trash: assets.join(".trash"),
            assets,
            profiles: root.join("profiles"),
            sessions: root.join("sessions"),
            history: root.join("history"),
            root,
        }
    }

    pub(crate) fn migrate_from(&self, legacy_root: &Path) -> Result<(), StorageError> {
        if legacy_root == self.root || !legacy_root.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.root).map_err(|error| {
            StorageError(format!(
                "failed to create managed data root '{}': {error}",
                self.root.display()
            ))
        })?;

        for (name, destination) in [
            ("index.sqlite3", &self.database),
            ("assets", &self.assets),
            ("profiles", &self.profiles),
            ("sessions", &self.sessions),
            ("history", &self.history),
        ] {
            move_managed_path(&legacy_root.join(name), destination)?;
        }
        for name in ["staging", "trash"] {
            let transient = legacy_root.join(name);
            if transient.exists() {
                std::fs::remove_dir_all(&transient).map_err(|error| {
                    StorageError(format!(
                        "failed to remove legacy transient directory '{}': {error}",
                        transient.display()
                    ))
                })?;
            }
        }
        Ok(())
    }

    pub fn ensure(&self) -> Result<(), StorageError> {
        for path in [
            &self.root,
            &self.assets,
            &self.profiles,
            &self.sessions,
            &self.history,
            &self.staging,
            &self.trash,
        ] {
            std::fs::create_dir_all(path).map_err(|error| {
                StorageError(format!(
                    "failed to create managed app data directory '{}': {error}",
                    path.display()
                ))
            })?;
        }

        Ok(())
    }

    pub fn resolve_asset_path(&self, relative: &str) -> Result<PathBuf, StorageError> {
        let relative = Path::new(relative);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(StorageError(format!(
                "asset path must be relative to the managed asset root: {relative:?}"
            )));
        }
        Ok(self.assets.join(relative))
    }
}

fn move_managed_path(source: &Path, destination: &Path) -> Result<(), StorageError> {
    if !source.exists() {
        return Ok(());
    }
    if !destination.exists() {
        std::fs::rename(source, destination).map_err(|error| {
            StorageError(format!(
                "failed to migrate '{}' to '{}': {error}",
                source.display(),
                destination.display()
            ))
        })?;
        return Ok(());
    }
    if source.is_dir() && destination.is_dir() {
        for entry in std::fs::read_dir(source).map_err(|error| {
            StorageError(format!(
                "failed to enumerate legacy directory '{}': {error}",
                source.display()
            ))
        })? {
            let entry = entry.map_err(|error| {
                StorageError(format!(
                    "failed to read legacy directory '{}': {error}",
                    source.display()
                ))
            })?;
            move_managed_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
        if std::fs::read_dir(source)
            .map_err(|error| {
                StorageError(format!(
                    "failed to inspect migrated directory '{}': {error}",
                    source.display()
                ))
            })?
            .next()
            .is_none()
        {
            std::fs::remove_dir(source).map_err(|error| {
                StorageError(format!(
                    "failed to remove migrated directory '{}': {error}",
                    source.display()
                ))
            })?;
        }
        return Ok(());
    }

    tracing::warn!(
        source = %source.display(),
        destination = %destination.display(),
        "managed data migration kept a conflicting legacy path"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_asset_paths_outside_managed_root() {
        let paths = AppPaths::new(PathBuf::from("app-data"));
        assert!(paths.resolve_asset_path("../escape").is_err());
        assert!(paths.resolve_asset_path("dictation/model").is_ok());
        assert_eq!(paths.manifest, paths.root.join("manifest-cn.json"));
        assert_eq!(paths.staging, paths.assets.join(".staging"));
        assert_eq!(paths.trash, paths.assets.join(".trash"));
    }

    #[test]
    fn migrates_managed_data_and_removes_legacy_downloads() {
        let base = std::env::temp_dir().join(format!("dictatingme-paths-{}", uuid::Uuid::new_v4()));
        let legacy = base.join("com.dictatingme.app");
        let current = base.join("DictatingMe");
        std::fs::create_dir_all(legacy.join("assets").join("speaker")).unwrap();
        std::fs::create_dir_all(legacy.join("staging").join("operation")).unwrap();
        std::fs::write(
            legacy.join("assets").join("speaker").join("model.onnx"),
            b"model",
        )
        .unwrap();
        std::fs::write(
            legacy
                .join("staging")
                .join("operation")
                .join("download.part"),
            b"archive",
        )
        .unwrap();
        std::fs::write(legacy.join("index.sqlite3"), b"database").unwrap();
        let paths = AppPaths::new(current);

        paths.migrate_from(&legacy).unwrap();

        assert!(paths.assets.join("speaker").join("model.onnx").is_file());
        assert!(paths.database.is_file());
        assert!(!legacy.join("staging").exists());
        std::fs::remove_dir_all(base).unwrap();
    }
}
