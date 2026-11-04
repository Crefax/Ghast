//! On-disk token cache.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

use super::MinecraftSession;
use crate::SecretString;

/// What [`TokenCache`] stores for one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedAccount {
    /// File format version, currently [`CachedAccount::VERSION`].
    pub version: u32,
    /// The OAuth client id the refresh token belongs to. A cache written for another client id
    /// is ignored.
    pub client_id: String,
    /// The Microsoft refresh token.
    pub microsoft_refresh_token: Option<SecretString>,
    /// The last Minecraft session.
    pub minecraft: Option<MinecraftSession>,
}

impl CachedAccount {
    /// The current file format version.
    pub const VERSION: u32 = 1;
}

/// Errors from [`TokenCache`].
#[derive(Debug, Error)]
pub enum CacheError {
    /// Reading or writing the file failed.
    #[error("I/O error on {path}: {source}")]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The error.
        #[source]
        source: io::Error,
    },
    /// The file is not a valid cache (corrupt, or a newer format version).
    #[error("invalid cache file: {0}")]
    Format(String),
}

/// A JSON file holding one account's tokens.
///
/// The application chooses the path (one file per account). Writes are atomic (temporary file
/// in the same directory, then rename). On Unix the file is created with mode `0600` and a
/// missing parent directory with mode `0700`, so other local users cannot read the tokens. On
/// other platforms the file inherits the directory's permissions; choose a private directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenCache {
    path: PathBuf,
}

/// Distinguishes temporary files of concurrent writers in one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl TokenCache {
    /// A cache stored at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the cache. A missing file is `Ok(None)`.
    pub async fn load(&self) -> Result<Option<CachedAccount>, CacheError> {
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(CacheError::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let account: CachedAccount =
            serde_json::from_slice(&bytes).map_err(|e| CacheError::Format(e.to_string()))?;
        if account.version != CachedAccount::VERSION {
            return Err(CacheError::Format(format!(
                "unsupported version {}",
                account.version
            )));
        }
        Ok(Some(account))
    }

    /// Writes the cache atomically with restrictive permissions.
    pub async fn save(&self, account: &CachedAccount) -> Result<(), CacheError> {
        let json =
            serde_json::to_vec_pretty(account).map_err(|e| CacheError::Format(e.to_string()))?;
        let io_err = |path: &Path| {
            let path = path.to_path_buf();
            move |source| CacheError::Io { path, source }
        };

        let dir = match self.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let mut builder = tokio::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(&dir).await.map_err(io_err(&dir))?;

        let file_name = self
            .path
            .file_name()
            .ok_or_else(|| CacheError::Format("cache path has no file name".into()))?;
        let mut tmp_name = file_name.to_os_string();
        tmp_name.push(format!(
            ".tmp-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let tmp = dir.join(tmp_name);

        let result = async {
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&tmp).await?;
            file.write_all(&json).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&tmp, &self.path).await
        }
        .await;
        if let Err(source) = result {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io_err(&self.path)(source));
        }
        Ok(())
    }

    /// Deletes the cache file (log out). A missing file is not an error.
    pub async fn clear(&self) -> Result<(), CacheError> {
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(CacheError::Io {
                path: self.path.clone(),
                source,
            }),
        }
    }
}
