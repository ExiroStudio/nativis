//! nativis-asset — Lightweight URI path resolver and reader streams.
//!
//! Handles `file://`, `https://`, and `plugin://` URI schemes without asset manager or database overhead.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("Invalid URI format: {0}")]
    InvalidUri(String),
    #[error("Unsupported URI scheme: {0}")]
    UnsupportedScheme(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Supported URI schemes for media sources.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scheme {
    File,
    Http,
    Https,
    Plugin(String),
    Custom(String),
}

/// Lightweight AssetPath carrying URI scheme, path, and optional extension.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetPath {
    raw_uri: String,
    scheme: Scheme,
    path: String,
    extension: String,
}

impl AssetPath {
    /// Parse a URI string or local file path into an `AssetPath`.
    pub fn parse(uri: &str) -> Result<Self, AssetError> {
        let raw_uri = uri.to_string();

        if let Some((scheme_str, rest)) = uri.split_once("://") {
            let scheme = match scheme_str.to_lowercase().as_str() {
                "file" => Scheme::File,
                "http" => Scheme::Http,
                "https" => Scheme::Https,
                other => Scheme::Plugin(other.to_string()),
            };

            let path_buf = PathBuf::from(rest);
            let extension = path_buf
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase();

            Ok(Self {
                raw_uri,
                scheme,
                path: rest.to_string(),
                extension,
            })
        } else {
            // Local file path default
            let path_buf = PathBuf::from(uri);
            let extension = path_buf
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase();

            Ok(Self {
                raw_uri: format!("file://{}", uri),
                scheme: Scheme::File,
                path: uri.to_string(),
                extension,
            })
        }
    }

    pub fn raw_uri(&self) -> &str {
        &self.raw_uri
    }

    pub fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn extension(&self) -> &str {
        &self.extension
    }

    pub fn is_local_file(&self) -> bool {
        matches!(self.scheme, Scheme::File)
    }

    pub fn to_file_path(&self) -> Option<PathBuf> {
        if self.is_local_file() {
            Some(PathBuf::from(&self.path))
        } else {
            None
        }
    }
}
