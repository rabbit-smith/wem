//! Checksum-addressed package resources (Python: `profiles/resources.py`).

use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::data::DataDir;
use crate::error::ProfileError;

/// Return a safe package-relative POSIX resource path.
///
/// Rejects exactly what Python `normalize_resource_path` rejects: empty
/// text, backslashes, absolute paths, and empty/`.`/`..` segments.
pub fn normalize_resource_path(path: &str) -> Result<PathBuf, ProfileError> {
    let unsafe_path = || ProfileError::UnsafePath {
        path: path.to_string(),
    };
    if path.is_empty()
        || path.contains('\\')
        || path.starts_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(unsafe_path());
    }
    Ok(PathBuf::from(path))
}

/// Immutable package resource identity validated by SHA-256
/// (Python `ResourceRef`). Paths are profiles-directory-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef {
    data: DataDir,
    path: PathBuf,
    sha256: String,
}

impl ResourceRef {
    /// Validate package/identity and normalize; mirrors Python `__post_init__`.
    pub fn new(data: DataDir, path: &str, sha256: &str) -> Result<Self, ProfileError> {
        let path = normalize_resource_path(path)?;
        let digest = sha256.to_lowercase();
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ProfileError::InvalidSha256 {
                value: sha256.to_string(),
            });
        }
        Ok(Self {
            data,
            path,
            sha256: digest,
        })
    }

    /// Profiles-relative POSIX path.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Lowercase SHA-256 hex identity.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The data tree this reference resolves against.
    pub fn data(&self) -> &DataDir {
        &self.data
    }

    /// Read bytes, verifying SHA-256 (Python `read_bytes`).
    pub fn read_bytes(&self) -> Result<Vec<u8>, ProfileError> {
        let full = self.data.profiles_dir().join(&self.path);
        let label = self.path.display().to_string();
        let payload = std::fs::read(&full).map_err(|_| ProfileError::MissingResource {
            path: label.clone(),
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let actual = hex(hasher.finalize());
        if actual != self.sha256 {
            return Err(ProfileError::ShaMismatch {
                path: label,
                expected: self.sha256.clone(),
                actual,
            });
        }
        Ok(payload)
    }

    /// Read text (UTF-8) with verification.
    pub fn read_text(&self) -> Result<String, ProfileError> {
        let payload = self.read_bytes()?;
        String::from_utf8(payload).map_err(|_| ProfileError::InvalidJson {
            path: self.path.display().to_string(),
        })
    }

    /// Read and parse JSON with verification (Python `read_json`).
    pub fn read_json(&self) -> Result<Value, ProfileError> {
        let raw = self.read_text()?;
        serde_json::from_str(&raw).map_err(|_| ProfileError::InvalidJson {
            path: self.path.display().to_string(),
        })
    }

    /// Verify identity and digest without consuming the payload.
    pub fn verify(&self) -> Result<(), ProfileError> {
        self.read_bytes().map(|_| ())
    }
}

/// Lowercase hex encoding (no external `hex` crate by design).
pub fn hex(bytes: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0F) as usize] as char);
    }
    out
}
