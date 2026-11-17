use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

use super::dto::ModFileDto;

/// Actions that are written to `mod_file_actions`.
#[derive(Debug, Clone, Copy)]
pub enum ModAction {
    Upload,
    Download,
    Delete,
}

impl ModAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Download => "download",
            Self::Delete => "delete",
        }
    }
}

/// Filenames arrive from the URL, so they are validated as a single path
/// segment before any filesystem call: no separators, no `..`, and no hidden
/// files (which would expose `.env` if `MODS_DIR` ever points at the project).
pub fn validate_file_name(raw: &str) -> Result<String, AppError> {
    let name = raw.trim();
    let length = name.chars().count();
    if length == 0 || length > 200 {
        return Err(AppError::Validation(
            "file name must be between 1 and 200 characters".to_owned(),
        ));
    }
    if name.starts_with('.') {
        return Err(AppError::Validation(
            "file name must not start with '.'".to_owned(),
        ));
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        return Err(AppError::Validation(
            "file name must not contain path separators or control characters".to_owned(),
        ));
    }
    Ok(name.to_owned())
}

/// Joins a validated name onto the canonical `MODS_DIR`. The parent check is
/// redundant while the name is a single segment, but keeps the invariant
/// explicit if this helper is reused later.
pub fn resolve(mods_dir: &Path, file_name: &str) -> Result<PathBuf, AppError> {
    let name = validate_file_name(file_name)?;
    let path = mods_dir.join(&name);

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Validation("invalid file name".to_owned()))?;
    if parent != mods_dir {
        return Err(AppError::Validation(
            "file name must not escape the mods directory".to_owned(),
        ));
    }

    Ok(path)
}

/// Stats a path without following symlinks: a link could point outside
/// `MODS_DIR`, so it is refused rather than resolved.
pub async fn ensure_regular_file(path: &Path) -> Result<std::fs::Metadata, AppError> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::NotFound("file"));
        }
        Err(err) => return Err(AppError::Io(err)),
    };

    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(
            "symbolic links are not served through this API".to_owned(),
        ));
    }
    if !metadata.is_file() {
        return Err(AppError::NotFound("file"));
    }

    Ok(metadata)
}

/// Writes every regular file in `MODS_DIR` into a ZIP archive at `target`.
/// Mods ship as JARs, which are already compressed, so entries are stored
/// as-is instead of being deflated a second time.
pub fn build_archive(mods_dir: &Path, files: &[ModFileDto], target: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::create(target)?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o644);

    for entry in files {
        let path = mods_dir.join(&entry.file_name);
        // Re-check the entry: the directory may change between listing and
        // reading, and a symlink must never end up in the archive.
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        if !metadata.file_type().is_file() {
            continue;
        }

        zip.start_file(&entry.file_name, options)?;
        let mut source = std::fs::File::open(&path)?;
        std::io::copy(&mut source, &mut zip)?;
    }

    zip.finish()?;
    Ok(())
}

/// Reads the directory into memory, skipping directories, symlinks, hidden
/// files and names that are not valid UTF-8 (the API cannot address those).
pub async fn list(mods_dir: &Path) -> Result<Vec<ModFileDto>, AppError> {
    let mut entries = tokio::fs::read_dir(mods_dir).await?;
    let mut files = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        // `file_type` describes the entry itself, so a symlink is not a file.
        if !entry.file_type().await?.is_file() {
            continue;
        }

        let Ok(name) = entry.file_name().into_string() else {
            tracing::warn!("skipping non-UTF-8 file name in MODS_DIR");
            continue;
        };
        if name.starts_with('.') {
            continue;
        }

        let metadata = entry.metadata().await?;
        files.push(ModFileDto {
            file_name: name,
            size_bytes: metadata.len(),
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        });
    }

    files.sort_by(|left, right| {
        left.file_name
            .to_lowercase()
            .cmp(&right.file_name.to_lowercase())
            .then_with(|| left.file_name.cmp(&right.file_name))
    });

    Ok(files)
}

/// Appends one audit row. Every upload, download and delete goes through here.
pub async fn log_action(
    pool: &PgPool,
    user_id: Uuid,
    action: ModAction,
    file_name: &str,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO mod_file_actions (user_id, action, file_name) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(action.as_str())
        .bind(file_name)
        .execute(pool)
        .await?;

    Ok(())
}

/// Builds a `Content-Disposition` value that stays ASCII-only: the plain
/// `filename` is a safe fallback, `filename*` carries the original name.
pub fn content_disposition(file_name: &str) -> String {
    let fallback: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();

    format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{}",
        percent_encode(file_name)
    )
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_names_that_could_escape_the_directory() {
        for name in [
            "..",
            "../secret.jar",
            "sub/dir.jar",
            "sub\\dir.jar",
            ".env",
            "",
            "   ",
            "bad\nname.jar",
        ] {
            assert!(
                validate_file_name(name).is_err(),
                "{name:?} should be rejected"
            );
        }
    }

    #[test]
    fn accepts_regular_mod_file_names() {
        for name in ["sodium-0.6.13.jar", "OptiFine 1.20.1.jar", "mod(1).jar"] {
            assert_eq!(validate_file_name(name).unwrap(), name);
        }
    }

    #[test]
    fn resolve_stays_inside_the_mods_dir() {
        let dir = Path::new("/srv/mods");
        assert_eq!(
            resolve(dir, "sodium.jar").unwrap(),
            PathBuf::from("/srv/mods/sodium.jar")
        );
        assert!(resolve(dir, "../etc/passwd").is_err());
    }

    #[test]
    fn content_disposition_is_ascii_and_keeps_the_original_name() {
        let header = content_disposition("мод.jar");

        assert!(header.is_ascii());
        assert!(header.contains("filename=\"___.jar\""));
        assert!(header.contains("filename*=UTF-8''%D0%BC%D0%BE%D0%B4.jar"));
    }

    #[test]
    fn archive_contains_every_regular_file() {
        let dir = std::env::temp_dir().join(format!("mods-archive-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("alpha.jar"), b"alpha").expect("first file");
        std::fs::write(dir.join("beta.jar"), b"beta").expect("second file");

        let files = vec![
            ModFileDto {
                file_name: "alpha.jar".to_owned(),
                size_bytes: 5,
                modified_at: None,
            },
            ModFileDto {
                file_name: "beta.jar".to_owned(),
                size_bytes: 4,
                modified_at: None,
            },
        ];
        let target = dir.join(".archive.zip");

        build_archive(&dir, &files, &target).expect("archive builds");

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&target).expect("archive file"))
            .expect("valid zip");
        assert_eq!(archive.len(), 2);
        assert_eq!(
            std::io::read_to_string(archive.by_name("alpha.jar").expect("alpha"))
                .expect("read alpha"),
            "alpha"
        );
        assert_eq!(
            std::io::read_to_string(archive.by_name("beta.jar").expect("beta")).expect("read beta"),
            "beta"
        );

        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
